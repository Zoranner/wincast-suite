use std::{
    net::TcpStream,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

use wincast_capture::CapturedBgraFrame;
use wincast_input::{CaptureInputBounds, WindowsInputEventSink};
use wincast_protocol::{
    frame::{FrameError, read_message, write_message},
    ipc::SessionEndReason,
    message::{ControlMessage, ErrorCode},
    raw_frame::{RawBgraFrame, write_raw_bgra_frame},
};

use super::capture::{CaptureRuntime, InputEventSink};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HostSessionEndReason {
    StopSession,
    CaptureInactive,
    ClientDisconnected,
    CaptureFailed,
    InputFailed,
    TransportFailed,
}

impl HostSessionEndReason {
    fn service_reason(self) -> SessionEndReason {
        match self {
            Self::StopSession => SessionEndReason::ServiceRequested,
            Self::CaptureInactive | Self::ClientDisconnected => {
                SessionEndReason::DesktopUnavailable
            }
            Self::CaptureFailed | Self::InputFailed | Self::TransportFailed => {
                SessionEndReason::SessionFailed
            }
        }
    }
}

impl From<HostSessionEndReason> for SessionEndReason {
    fn from(reason: HostSessionEndReason) -> Self {
        reason.service_reason()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HostSessionError {
    pub(super) reason: HostSessionEndReason,
    pub(super) message: String,
}

impl HostSessionError {
    pub(super) fn new(reason: HostSessionEndReason, message: impl Into<String>) -> Self {
        Self {
            reason,
            message: message.into(),
        }
    }
}
pub(super) fn write_control_error(
    writer: &mut impl std::io::Write,
    code: ErrorCode,
    message: String,
) -> Result<(), String> {
    write_message(writer, &ControlMessage::Error { code, message })
        .map_err(|error| format!("写入控制错误消息失败: {error}"))
}

pub(super) fn write_session_ready(
    writer: &mut impl std::io::Write,
    frame: &CapturedBgraFrame,
) -> Result<(), String> {
    write_message(
        writer,
        &ControlMessage::SessionReady {
            width: frame.metadata.frame.width,
            height: frame.metadata.frame.height,
        },
    )
    .map_err(|error| format!("写入会话就绪消息失败: {error}"))
}

pub(super) fn write_raw_bgra_stream(
    writer: &mut impl std::io::Write,
    input_reader: &TcpStream,
    first_frame: &CapturedBgraFrame,
    session: &mut dyn CaptureRuntime,
    input_bounds: CaptureInputBounds,
) -> Result<HostSessionEndReason, HostSessionError> {
    let input_stream = input_reader.try_clone().map_err(|error| {
        HostSessionError::new(
            HostSessionEndReason::TransportFailed,
            format!("克隆客户端输入事件读取端失败: {error}"),
        )
    })?;
    let input_events = spawn_input_event_reader(input_stream, input_bounds);
    write_raw_bgra_stream_with_input_reader(writer, first_frame, session, input_events)
}

pub(super) fn write_raw_bgra_stream_with_input_reader(
    writer: &mut impl std::io::Write,
    first_frame: &CapturedBgraFrame,
    session: &mut dyn CaptureRuntime,
    input_reader: InputEventReader,
) -> Result<HostSessionEndReason, HostSessionError> {
    let result = write_raw_bgra_stream_with_input_events(
        writer,
        first_frame,
        session,
        input_reader.receiver(),
    );
    let cleanup_result = match &result {
        Ok(HostSessionEndReason::StopSession) => join_input_reader(input_reader),
        _ => stop_and_join_input_reader(input_reader),
    };

    result.and_then(|reason| {
        cleanup_result?;
        Ok(reason)
    })
}

pub(super) fn write_raw_bgra_stream_with_input_events(
    writer: &mut impl std::io::Write,
    first_frame: &CapturedBgraFrame,
    session: &mut dyn CaptureRuntime,
    input_events: &mpsc::Receiver<InputReaderEvent>,
) -> Result<HostSessionEndReason, HostSessionError> {
    write_message(writer, &ControlMessage::VideoReady).map_err(|error| {
        HostSessionError::new(
            HostSessionEndReason::TransportFailed,
            format!("写入视频就绪消息失败: {error}"),
        )
    })?;
    write_raw_bgra_frame_from_capture(writer, first_frame)
        .map_err(|message| HostSessionError::new(HostSessionEndReason::TransportFailed, message))?;
    if let Some(reason) = check_input_reader_events(input_events)? {
        write_session_goodbye(writer).map_err(|message| {
            HostSessionError::new(HostSessionEndReason::TransportFailed, message)
        })?;
        return Ok(reason);
    }

    loop {
        let frame = match session.try_next_bgra_frame() {
            Ok(Some(frame)) => frame,
            Ok(None) => {
                if let Some(reason) = check_input_reader_events(input_events)? {
                    write_session_goodbye(writer).map_err(|message| {
                        HostSessionError::new(HostSessionEndReason::TransportFailed, message)
                    })?;
                    return Ok(reason);
                }
                if !session.is_active() {
                    write_session_goodbye(writer).map_err(|message| {
                        HostSessionError::new(HostSessionEndReason::TransportFailed, message)
                    })?;
                    return Ok(HostSessionEndReason::CaptureInactive);
                }
                thread::sleep(Duration::from_millis(16));
                continue;
            }
            Err(error) => {
                let message = format!("读取后续 raw BGRA 捕获帧失败: {error}");
                let _ = write_control_error(writer, ErrorCode::CaptureFailed, message.clone());
                return Err(HostSessionError::new(
                    HostSessionEndReason::CaptureFailed,
                    message,
                ));
            }
        };
        write_raw_bgra_frame_from_capture(writer, &frame).map_err(|message| {
            HostSessionError::new(HostSessionEndReason::TransportFailed, message)
        })?;
        if let Some(reason) = check_input_reader_events(input_events)? {
            write_session_goodbye(writer).map_err(|message| {
                HostSessionError::new(HostSessionEndReason::TransportFailed, message)
            })?;
            return Ok(reason);
        }
    }
}

fn stop_and_join_input_reader(input_reader: InputEventReader) -> Result<(), HostSessionError> {
    input_reader.stop_and_join().map(|_| ()).map_err(|_| {
        HostSessionError::new(
            HostSessionEndReason::InputFailed,
            "输入事件读取线程异常结束".to_owned(),
        )
    })
}

fn join_input_reader(input_reader: InputEventReader) -> Result<(), HostSessionError> {
    input_reader.join().map(|_| ()).map_err(|_| {
        HostSessionError::new(
            HostSessionEndReason::InputFailed,
            "输入事件读取线程异常结束".to_owned(),
        )
    })
}

pub(super) fn write_session_goodbye(writer: &mut impl std::io::Write) -> Result<(), String> {
    match write_message(writer, &ControlMessage::Goodbye) {
        Ok(()) => writer
            .flush()
            .map_err(|error| format!("刷新会话结束消息失败: {error}")),
        Err(error) if is_control_stream_closed(&error) => Ok(()),
        Err(error) => Err(format!("写入会话结束消息失败: {error}")),
    }
}

fn write_raw_bgra_frame_from_capture(
    writer: &mut impl std::io::Write,
    frame: &CapturedBgraFrame,
) -> Result<(), String> {
    let raw_frame = RawBgraFrame {
        width: frame.metadata.frame.width,
        height: frame.metadata.frame.height,
        row_pitch: frame.row_pitch,
        sequence_number: frame.metadata.frame.sequence_number,
        timestamp_ns: frame.metadata.frame.timestamp_ns,
        bytes: frame.bytes.clone(),
    };
    write_raw_bgra_frame(writer, &raw_frame)
        .map_err(|error| format!("写入 raw BGRA 二进制帧失败: {error}"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum InputReaderEvent {
    StopSession,
    Disconnected,
    Failed(String),
}

pub(super) fn spawn_input_event_reader(
    mut input_reader: TcpStream,
    input_bounds: CaptureInputBounds,
) -> InputEventReader {
    let (sender, receiver) = mpsc::channel();
    let stop_requested = Arc::new(AtomicBool::new(false));
    let reader_stop_requested = Arc::clone(&stop_requested);
    let handle = thread::spawn(move || {
        let _ = input_reader.set_read_timeout(Some(Duration::from_millis(100)));
        let mut input_sink = WindowsInputEventSink::new(input_bounds);
        let event = read_input_events_until_stop(
            &mut input_reader,
            &mut input_sink,
            &reader_stop_requested,
        );
        let _ = sender.send(event.clone());
        Some(event)
    });

    InputEventReader {
        receiver,
        handle: Some(handle),
        stop_requested,
    }
}

pub(super) struct InputEventReader {
    receiver: mpsc::Receiver<InputReaderEvent>,
    handle: Option<thread::JoinHandle<Option<InputReaderEvent>>>,
    stop_requested: Arc<AtomicBool>,
}

impl InputEventReader {
    pub(super) fn receiver(&self) -> &mpsc::Receiver<InputReaderEvent> {
        &self.receiver
    }

    pub(super) fn join(mut self) -> thread::Result<Option<InputReaderEvent>> {
        self.handle
            .take()
            .expect("input reader handle should exist")
            .join()
    }

    pub(super) fn stop_and_join(self) -> thread::Result<Option<InputReaderEvent>> {
        self.stop_requested.store(true, Ordering::SeqCst);
        self.join()
    }
}

impl Drop for InputEventReader {
    fn drop(&mut self) {
        if self
            .handle
            .as_ref()
            .is_some_and(|handle| handle.is_finished())
            && let Some(handle) = self.handle.take()
        {
            let _ = handle.join();
        }
    }
}

pub(super) fn read_input_events_until_stop(
    reader: &mut impl std::io::Read,
    sink: &mut impl InputEventSink,
    stop_requested: &AtomicBool,
) -> InputReaderEvent {
    loop {
        if stop_requested.load(Ordering::SeqCst) {
            return InputReaderEvent::Disconnected;
        }

        match read_message(reader) {
            Ok(ControlMessage::InputEvent(event)) => {
                if let Err(error) = sink.handle_input_event(event) {
                    return InputReaderEvent::Failed(format!("处理客户端输入事件失败: {error}"));
                }
            }
            Ok(ControlMessage::StopSession) | Ok(ControlMessage::Goodbye) => {
                return InputReaderEvent::StopSession;
            }
            Ok(message) => {
                return InputReaderEvent::Failed(format!("客户端输入事件消息无效: {message:?}"));
            }
            Err(error) => {
                if is_temporary_read_timeout(&error) {
                    continue;
                }
                if is_control_stream_closed(&error) {
                    return InputReaderEvent::Disconnected;
                }
                return InputReaderEvent::Failed(format!("读取客户端输入事件失败: {error}"));
            }
        }
    }
}

fn is_temporary_read_timeout(error: &FrameError) -> bool {
    matches!(
        error,
        FrameError::Io(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            )
    )
}

fn is_control_stream_closed(error: &FrameError) -> bool {
    match error {
        FrameError::Io(error) => matches!(
            error.kind(),
            std::io::ErrorKind::UnexpectedEof
                | std::io::ErrorKind::ConnectionAborted
                | std::io::ErrorKind::ConnectionReset
                | std::io::ErrorKind::BrokenPipe
        ),
        _ => false,
    }
}

fn check_input_reader_events(
    receiver: &mpsc::Receiver<InputReaderEvent>,
) -> Result<Option<HostSessionEndReason>, HostSessionError> {
    match receiver.try_recv() {
        Ok(InputReaderEvent::StopSession) => Ok(Some(HostSessionEndReason::StopSession)),
        Ok(InputReaderEvent::Disconnected) | Err(mpsc::TryRecvError::Disconnected) => {
            Ok(Some(HostSessionEndReason::ClientDisconnected))
        }
        Ok(InputReaderEvent::Failed(error)) => Err(HostSessionError::new(
            HostSessionEndReason::InputFailed,
            error,
        )),
        Err(mpsc::TryRecvError::Empty) => Ok(None),
    }
}
