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
use wincast_media::{
    MediaConfigError, MediaError, OpenH264Encoder, RawPixelFormat, RawVideoFrame,
    RawVideoFrameError, VideoEncoder, VideoPipelineConfig,
};
use wincast_protocol::{
    frame::{FrameError, read_message, write_message},
    message::{ControlMessage, ErrorCode},
};

use super::capture::{CaptureRuntime, InputEventSink};
use super::session::SessionGate;

const INPUT_READER_READ_TIMEOUT: Duration = Duration::from_millis(100);
const INPUT_READER_TIMEOUT_LIMIT: u32 = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HostSessionEndReason {
    StopSession,
    CaptureInactive,
    ClientDisconnected,
    CaptureFailed,
    InputFailed,
    TransportFailed,
    SessionUnavailable,
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

pub(super) fn write_h264_encoded_stream(
    writer: &mut impl std::io::Write,
    input_reader: &TcpStream,
    first_frame: &CapturedBgraFrame,
    session: &mut dyn CaptureRuntime,
    input_bounds: CaptureInputBounds,
    pipeline_config: VideoPipelineConfig,
    session_gate: &impl SessionGate,
) -> Result<HostSessionEndReason, HostSessionError> {
    let input_stream = input_reader.try_clone().map_err(|error| {
        HostSessionError::new(
            HostSessionEndReason::TransportFailed,
            format!("克隆客户端输入事件读取端失败: {error}"),
        )
    })?;
    let input_events = spawn_input_event_reader(input_stream, input_bounds);
    write_h264_encoded_stream_with_input_reader(
        writer,
        first_frame,
        session,
        input_events,
        pipeline_config,
        session_gate,
    )
}

pub(super) fn write_h264_encoded_stream_with_input_reader(
    writer: &mut impl std::io::Write,
    first_frame: &CapturedBgraFrame,
    session: &mut dyn CaptureRuntime,
    input_reader: InputEventReader,
    pipeline_config: VideoPipelineConfig,
    session_gate: &impl SessionGate,
) -> Result<HostSessionEndReason, HostSessionError> {
    let result = write_h264_encoded_stream_with_input_events(
        writer,
        first_frame,
        session,
        input_reader.receiver(),
        pipeline_config,
        session_gate,
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

pub(super) fn write_h264_encoded_stream_with_input_events(
    writer: &mut impl std::io::Write,
    first_frame: &CapturedBgraFrame,
    session: &mut dyn CaptureRuntime,
    input_events: &mpsc::Receiver<InputReaderEvent>,
    pipeline_config: VideoPipelineConfig,
    session_gate: &impl SessionGate,
) -> Result<HostSessionEndReason, HostSessionError> {
    let mut encoder = OpenH264Encoder::new(pipeline_config).map_err(|error| {
        write_h264_encoding_error(
            writer,
            format!("H.264 编码器初始化失败: {}", media_error_message(&error)),
        )
    })?;
    write_h264_frame_from_capture(writer, &mut encoder, first_frame, "H.264 首帧编码失败")?;
    if let Some(reason) = check_input_reader_events(input_events)? {
        write_session_goodbye(writer).map_err(|message| {
            HostSessionError::new(HostSessionEndReason::TransportFailed, message)
        })?;
        return Ok(reason);
    }
    if let Some(reason) = check_session_gate(writer, session_gate)? {
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
                if let Some(reason) = check_session_gate(writer, session_gate)? {
                    return Ok(reason);
                }
                thread::sleep(Duration::from_millis(16));
                continue;
            }
            Err(error) => {
                let message = format!("读取后续 H.264 捕获帧失败: {error}");
                let _ = write_control_error(writer, ErrorCode::CaptureFailed, message.clone());
                return Err(HostSessionError::new(
                    HostSessionEndReason::CaptureFailed,
                    message,
                ));
            }
        };
        write_h264_frame_from_capture(writer, &mut encoder, &frame, "H.264 后续帧编码失败")?;
        if let Some(reason) = check_input_reader_events(input_events)? {
            write_session_goodbye(writer).map_err(|message| {
                HostSessionError::new(HostSessionEndReason::TransportFailed, message)
            })?;
            return Ok(reason);
        }
        if let Some(reason) = check_session_gate(writer, session_gate)? {
            return Ok(reason);
        }
    }
}

fn check_session_gate(
    writer: &mut impl std::io::Write,
    session_gate: &impl SessionGate,
) -> Result<Option<HostSessionEndReason>, HostSessionError> {
    let Some((code, message)) = session_gate.remote_session_status().to_protocol_error() else {
        return Ok(None);
    };
    write_control_error(writer, code, message.to_owned())
        .map_err(|message| HostSessionError::new(HostSessionEndReason::TransportFailed, message))?;
    write_session_goodbye(writer)
        .map_err(|message| HostSessionError::new(HostSessionEndReason::TransportFailed, message))?;
    Ok(Some(HostSessionEndReason::SessionUnavailable))
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

fn write_h264_frame_from_capture(
    writer: &mut impl std::io::Write,
    encoder: &mut impl VideoEncoder,
    frame: &CapturedBgraFrame,
    failure_prefix: &str,
) -> Result<(), HostSessionError> {
    let encoded = encoder
        .encode(raw_video_frame_from_capture(frame))
        .map_err(|error| {
            write_h264_encoding_error(
                writer,
                format!("{failure_prefix}: {}", media_error_message(&error)),
            )
        })?;
    write_message(writer, &ControlMessage::EncodedVideoFrame(encoded)).map_err(|error| {
        HostSessionError::new(
            HostSessionEndReason::TransportFailed,
            format!("写入 H.264 编码视频帧失败: {error}"),
        )
    })
}

fn raw_video_frame_from_capture(frame: &CapturedBgraFrame) -> RawVideoFrame<'_> {
    RawVideoFrame {
        width: frame.metadata.frame.width,
        height: frame.metadata.frame.height,
        row_pitch: frame.row_pitch,
        format: RawPixelFormat::Bgra8Unorm,
        sequence_number: frame.metadata.frame.sequence_number,
        timestamp_ns: frame.metadata.frame.timestamp_ns,
        bytes: &frame.bytes,
    }
}

fn write_h264_encoding_error(
    writer: &mut impl std::io::Write,
    message: String,
) -> HostSessionError {
    let write_result = write_control_error(writer, ErrorCode::EncodingFailed, message.clone());
    let message = append_error_response_write_failure(message, write_result);
    HostSessionError::new(HostSessionEndReason::CaptureFailed, message)
}

fn media_error_message(error: &MediaError) -> String {
    match error {
        MediaError::Config(error) => media_config_error_message(error),
        MediaError::InvalidRawFrame(error) => raw_video_frame_error_message(error),
        MediaError::UnsupportedEncodedCodec { codec } => {
            format!("编码视频帧只支持 H.264，当前为 {codec:?}")
        }
        MediaError::InvalidEncodedFrame(error) => format!("编码视频帧无效: {error:?}"),
        MediaError::DecodedPayloadTooLarge { actual, max } => {
            format!("fake 解码输出载荷 {actual} 超过上限 {max}")
        }
        MediaError::BackendUnavailable(message) => format!("媒体后端不可用: {message}"),
        MediaError::Backend(message) => format!("媒体后端处理失败: {message}"),
    }
}

fn media_config_error_message(error: &MediaConfigError) -> String {
    match error {
        MediaConfigError::UnsupportedCodec { codec } => {
            format!("媒体链路只支持 H.264，当前配置为 {codec:?}")
        }
        MediaConfigError::InvalidDimensions { width, height } => {
            format!("视频尺寸无效: {width}x{height}")
        }
        MediaConfigError::ResolutionTooLarge {
            width,
            height,
            max_width,
            max_height,
        } => format!("视频尺寸 {width}x{height} 超过上限 {max_width}x{max_height}"),
        MediaConfigError::OddDimensions { width, height } => {
            format!("H.264 I420 编码要求偶数尺寸，当前为 {width}x{height}")
        }
        MediaConfigError::InvalidFps { fps, max_fps } => {
            format!("视频帧率 {fps} 无效，最大支持 {max_fps}")
        }
        MediaConfigError::InvalidMaxBitrate => "视频码率上限必须大于 0".to_owned(),
        MediaConfigError::InvalidBitrate {
            bitrate_kbps,
            max_bitrate_kbps,
        } => format!("视频目标码率 {bitrate_kbps} 无效，上限为 {max_bitrate_kbps}"),
    }
}

fn raw_video_frame_error_message(error: &RawVideoFrameError) -> String {
    match error {
        RawVideoFrameError::InvalidDimensions { width, height } => {
            format!("raw 视频尺寸无效: {width}x{height}")
        }
        RawVideoFrameError::ResolutionTooLarge {
            width,
            height,
            max_width,
            max_height,
        } => format!("视频尺寸 {width}x{height} 超过上限 {max_width}x{max_height}"),
        RawVideoFrameError::ConfigDimensionMismatch {
            frame_width,
            frame_height,
            config_width,
            config_height,
        } => format!(
            "raw 视频尺寸 {frame_width}x{frame_height} 与配置尺寸 {config_width}x{config_height} 不一致"
        ),
        RawVideoFrameError::OddDimensions { width, height } => {
            format!("H.264 I420 编码要求偶数 raw frame 尺寸，当前为 {width}x{height}")
        }
        RawVideoFrameError::UnsupportedPixelFormat { format } => {
            format!("H.264 编码器只支持 BGRA8 raw 帧，当前为 {format:?}")
        }
        RawVideoFrameError::EmptyPayload => "raw 视频帧载荷为空".to_owned(),
        RawVideoFrameError::RowPitchOverflow => "raw 视频帧行跨度溢出".to_owned(),
        RawVideoFrameError::InvalidRowPitch {
            row_pitch,
            min_row_pitch,
        } => format!("raw 视频帧行跨度 {row_pitch} 小于最小值 {min_row_pitch}"),
        RawVideoFrameError::PayloadLengthOverflow => "raw 视频帧载荷长度溢出".to_owned(),
        RawVideoFrameError::InvalidPayloadLength { actual, expected } => {
            format!("raw 视频帧载荷长度 {actual} 与期望长度 {expected} 不一致")
        }
    }
}

fn append_error_response_write_failure(
    message: String,
    write_result: Result<(), String>,
) -> String {
    match write_result {
        Ok(()) => message,
        Err(write_error) => format!("{message}；{write_error}"),
    }
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
        let _ = input_reader.set_read_timeout(Some(INPUT_READER_READ_TIMEOUT));
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
    read_input_events_until_stop_with_timeout_limit(
        reader,
        sink,
        stop_requested,
        INPUT_READER_TIMEOUT_LIMIT,
    )
}

pub(super) fn read_input_events_until_stop_with_timeout_limit(
    reader: &mut impl std::io::Read,
    sink: &mut impl InputEventSink,
    stop_requested: &AtomicBool,
    timeout_limit: u32,
) -> InputReaderEvent {
    let mut consecutive_timeouts = 0;
    loop {
        if stop_requested.load(Ordering::SeqCst) {
            return InputReaderEvent::Disconnected;
        }

        match read_message(reader) {
            Ok(ControlMessage::InputEvent(event)) => {
                consecutive_timeouts = 0;
                if let Err(error) = sink.handle_input_event(event) {
                    return InputReaderEvent::Failed(format!("处理客户端输入事件失败: {error}"));
                }
            }
            Ok(ControlMessage::Heartbeat) => {
                consecutive_timeouts = 0;
            }
            Ok(ControlMessage::StopSession) | Ok(ControlMessage::Goodbye) => {
                return InputReaderEvent::StopSession;
            }
            Ok(message) => {
                return InputReaderEvent::Failed(format!("客户端输入事件消息无效: {message:?}"));
            }
            Err(error) => {
                if is_temporary_read_timeout(&error) {
                    consecutive_timeouts += 1;
                    if consecutive_timeouts >= timeout_limit {
                        return InputReaderEvent::Disconnected;
                    }
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
