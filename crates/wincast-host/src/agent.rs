use std::{
    net::{SocketAddr, TcpListener, TcpStream},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use crate::{
    program::{ProgramRunner, StartedProgram},
    session_state::RemoteSessionStatus,
    window::{WindowCandidate, WindowLookupError, find_main_window},
};
use wincast_capture::{
    CaptureError, CaptureSession, CaptureTarget, CapturedBgraFrame, wait_next_capture_result_with,
};
use wincast_input::{CaptureInputBounds, WindowsInputEventSink};
use wincast_protocol::{
    config::{CaptureMode, HostConfig},
    frame::{FrameError, read_message, write_message},
    handshake::{accept_client_hello, reject_busy_client},
    input::InputEvent,
    ipc::SessionEndReason,
    message::{ControlMessage, ErrorCode},
    raw_frame::{RawBgraFrame, write_raw_bgra_frame},
};

const SESSION_RECLAIM_GRACE: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HostSessionEndReason {
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
struct HostSessionError {
    reason: HostSessionEndReason,
    message: String,
}

impl HostSessionError {
    fn new(reason: HostSessionEndReason, message: impl Into<String>) -> Self {
        Self {
            reason,
            message: message.into(),
        }
    }
}

pub(crate) fn run_control_listener(
    listener: TcpListener,
    config: &HostConfig,
    runner: &mut (impl ProgramRunner + Send),
) -> Result<SocketAddr, String> {
    let mut locator = WindowsWindowLocator;
    let mut capture = StdCaptureStarter;
    run_control_listener_with_runtime(listener, config, runner, &mut locator, &mut capture)
}

fn run_control_listener_with_runtime(
    listener: TcpListener,
    config: &HostConfig,
    runner: &mut (impl ProgramRunner + Send),
    locator: &mut (impl WindowLocator + Send),
    capture: &mut (impl CaptureStarter + Send),
) -> Result<SocketAddr, String> {
    let local_addr = listener
        .local_addr()
        .map_err(|error| format!("读取宿主端监听地址失败: {error}"))?;
    thread::scope(|scope| {
        run_control_listener_accept_loop(listener, config, runner, locator, capture, scope, None)
    })?;
    Ok(local_addr)
}

fn run_control_listener_accept_loop<'scope, R, L, C>(
    listener: TcpListener,
    config: &'scope HostConfig,
    runner: &'scope mut R,
    locator: &'scope mut L,
    capture: &'scope mut C,
    scope: &'scope thread::Scope<'scope, '_>,
    max_connections: Option<usize>,
) -> Result<(), String>
where
    R: ProgramRunner + Send + 'scope,
    L: WindowLocator + Send + 'scope,
    C: CaptureStarter + Send + 'scope,
{
    let (session_finished_sender, session_finished_receiver) = mpsc::channel();
    let mut state = ListenerSessionState::Idle {
        runner,
        locator,
        capture,
    };
    let mut accepted_connections = 0usize;

    loop {
        if let Some(max_connections) = max_connections
            && accepted_connections >= max_connections
        {
            join_finished_session(state);
            return Ok(());
        }

        let (stream, peer_addr) = listener
            .accept()
            .map_err(|error| format!("接受客户端连接失败: {error}"))?;
        accepted_connections += 1;
        state = join_finished_session_if_reported(state, &session_finished_receiver);

        state = match state {
            ListenerSessionState::Idle {
                runner,
                locator,
                capture,
            } => {
                let finished_sender = session_finished_sender.clone();
                ListenerSessionState::Busy(scope.spawn(move || {
                    let mut stream = stream;
                    let result =
                        handle_control_client(&mut stream, config, runner, locator, capture);
                    let _ = finished_sender.send(SessionFinished);
                    (peer_addr, result, runner, locator, capture)
                }))
            }
            ListenerSessionState::Busy(session) => {
                let state = wait_for_finished_session(
                    ListenerSessionState::Busy(session),
                    &session_finished_receiver,
                    SESSION_RECLAIM_GRACE,
                );
                match state {
                    ListenerSessionState::Idle {
                        runner,
                        locator,
                        capture,
                    } => {
                        let finished_sender = session_finished_sender.clone();
                        ListenerSessionState::Busy(scope.spawn(move || {
                            let mut stream = stream;
                            let result = handle_control_client(
                                &mut stream,
                                config,
                                runner,
                                locator,
                                capture,
                            );
                            let _ = finished_sender.send(SessionFinished);
                            (peer_addr, result, runner, locator, capture)
                        }))
                    }
                    ListenerSessionState::Busy(session) => {
                        reject_busy_control_client(stream, peer_addr);
                        ListenerSessionState::Busy(session)
                    }
                }
            }
        };
    }
}

struct SessionFinished;

enum ListenerSessionState<'scope, R, L, C>
where
    R: ProgramRunner + Send + 'scope,
    L: WindowLocator + Send + 'scope,
    C: CaptureStarter + Send + 'scope,
{
    Idle {
        runner: &'scope mut R,
        locator: &'scope mut L,
        capture: &'scope mut C,
    },
    Busy(ScopedSessionHandle<'scope, R, L, C>),
}

fn join_finished_session_if_reported<'scope, R, L, C>(
    state: ListenerSessionState<'scope, R, L, C>,
    session_finished: &mpsc::Receiver<SessionFinished>,
) -> ListenerSessionState<'scope, R, L, C>
where
    R: ProgramRunner + Send + 'scope,
    L: WindowLocator + Send + 'scope,
    C: CaptureStarter + Send + 'scope,
{
    if session_finished.try_recv().is_ok() {
        join_reported_finished_session(state)
    } else {
        state
    }
}

fn wait_for_finished_session<'scope, R, L, C>(
    state: ListenerSessionState<'scope, R, L, C>,
    session_finished: &mpsc::Receiver<SessionFinished>,
    timeout: Duration,
) -> ListenerSessionState<'scope, R, L, C>
where
    R: ProgramRunner + Send + 'scope,
    L: WindowLocator + Send + 'scope,
    C: CaptureStarter + Send + 'scope,
{
    match state {
        ListenerSessionState::Busy(session) => match session_finished.recv_timeout(timeout) {
            Ok(SessionFinished) => {
                join_reported_finished_session(ListenerSessionState::Busy(session))
            }
            Err(_) => ListenerSessionState::Busy(session),
        },
        state => state,
    }
}

fn join_reported_finished_session<'scope, R, L, C>(
    state: ListenerSessionState<'scope, R, L, C>,
) -> ListenerSessionState<'scope, R, L, C>
where
    R: ProgramRunner + Send + 'scope,
    L: WindowLocator + Send + 'scope,
    C: CaptureStarter + Send + 'scope,
{
    match state {
        ListenerSessionState::Idle {
            runner,
            locator,
            capture,
        } => ListenerSessionState::Idle {
            runner,
            locator,
            capture,
        },
        ListenerSessionState::Busy(session) => {
            let (_peer_addr, _result, runner, locator, capture) =
                log_session_result(session.join());
            ListenerSessionState::Idle {
                runner,
                locator,
                capture,
            }
        }
    }
}

type ScopedSessionHandle<'scope, R, L, C> =
    thread::ScopedJoinHandle<'scope, SessionThreadResult<'scope, R, L, C>>;

type SessionThreadResult<'scope, R, L, C> = (
    SocketAddr,
    Result<(), String>,
    &'scope mut R,
    &'scope mut L,
    &'scope mut C,
);

fn join_finished_session<'scope, R, L, C>(
    state: ListenerSessionState<'scope, R, L, C>,
) -> ListenerSessionState<'scope, R, L, C>
where
    R: ProgramRunner + Send + 'scope,
    L: WindowLocator + Send + 'scope,
    C: CaptureStarter + Send + 'scope,
{
    match state {
        ListenerSessionState::Idle {
            runner,
            locator,
            capture,
        } => ListenerSessionState::Idle {
            runner,
            locator,
            capture,
        },
        ListenerSessionState::Busy(session) if session.is_finished() => {
            let (_peer_addr, _result, runner, locator, capture) =
                log_session_result(session.join());
            ListenerSessionState::Idle {
                runner,
                locator,
                capture,
            }
        }
        ListenerSessionState::Busy(session) => ListenerSessionState::Busy(session),
    }
}

fn log_session_result<'scope, R, L, C>(
    result: std::thread::Result<SessionThreadResult<'scope, R, L, C>>,
) -> SessionThreadResult<'scope, R, L, C> {
    match result {
        Ok((peer_addr, session_result, runner, locator, capture)) => {
            if let Err(error) = &session_result {
                eprintln!("客户端 {peer_addr} 会话结束: {error}");
            } else {
                eprintln!("客户端 {peer_addr} 会话结束");
            }
            (peer_addr, session_result, runner, locator, capture)
        }
        Err(_) => {
            panic!("客户端会话线程异常结束");
        }
    }
}

fn reject_busy_control_client(mut stream: TcpStream, peer_addr: SocketAddr) {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
    let _ = read_message(&mut stream);
    if let Err(error) = reject_busy_client(&mut stream) {
        eprintln!("客户端 {peer_addr} 忙碌拒绝失败: {error}");
    }
}

fn handle_control_client(
    stream: &mut TcpStream,
    config: &HostConfig,
    runner: &mut impl ProgramRunner,
    locator: &mut impl WindowLocator,
    capture: &mut impl CaptureStarter,
) -> Result<(), String> {
    let mut session_gate = ForegroundRunSessionGate;
    handle_control_client_with_session_gate(
        stream,
        config,
        runner,
        locator,
        capture,
        &mut session_gate,
    )
}

fn handle_control_client_with_session_gate(
    stream: &mut TcpStream,
    config: &HostConfig,
    runner: &mut impl ProgramRunner,
    locator: &mut impl WindowLocator,
    capture: &mut impl CaptureStarter,
    session_gate: &mut impl SessionGate,
) -> Result<(), String> {
    let mut writer = stream
        .try_clone()
        .map_err(|error| format!("克隆控制连接写入端失败: {error}"))?;
    accept_client_hello(stream, &mut writer).map_err(|error| format!("控制握手失败: {error}"))?;

    match read_message(stream).map_err(|error| format!("读取控制消息失败: {error}"))? {
        ControlMessage::StartSession => {
            ensure_remote_session_allowed(&mut writer, session_gate)?;
            let mut started =
                crate::program::launch_with_runner(config, runner).map_err(|error| {
                    let message = format!("启动宿主端程序失败: {error}");
                    let _ = write_control_error(
                        &mut writer,
                        ErrorCode::ProgramLaunchFailed,
                        message.clone(),
                    );
                    message
                })?;
            let result =
                run_started_session(&mut writer, stream, config, locator, capture, &started);
            let cleanup_result = runner
                .cleanup(&mut started)
                .map_err(|error| format!("清理宿主端程序失败: {error}"));
            match (result, cleanup_result) {
                (Ok(reason), Ok(())) => {
                    let _ = SessionEndReason::from(reason);
                    Ok(())
                }
                (Err(error), Ok(())) => Err(error.message),
                (Ok(reason), Err(error)) => {
                    let _ = SessionEndReason::from(reason);
                    Err(error)
                }
                (Err(session_error), Err(cleanup_error)) => {
                    Err(format!("{}；{cleanup_error}", session_error.message))
                }
            }
        }
        message => {
            write_control_error(
                &mut writer,
                ErrorCode::TransportFailed,
                format!("控制消息顺序无效，期望 StartSession，实际收到 {message:?}"),
            )?;
            Err("控制消息顺序无效，期望 StartSession".to_owned())
        }
    }
}

trait SessionGate {
    fn remote_session_status(&mut self) -> RemoteSessionStatus;
}

struct ForegroundRunSessionGate;

impl SessionGate for ForegroundRunSessionGate {
    fn remote_session_status(&mut self) -> RemoteSessionStatus {
        RemoteSessionStatus::Allowed
    }
}

fn ensure_remote_session_allowed(
    writer: &mut impl std::io::Write,
    session_gate: &mut impl SessionGate,
) -> Result<(), String> {
    match session_gate.remote_session_status().to_protocol_error() {
        Some((code, message)) => {
            write_control_error(writer, code, message.to_owned())?;
            Err(message.to_owned())
        }
        None => Ok(()),
    }
}

fn run_started_session(
    writer: &mut impl std::io::Write,
    stream: &TcpStream,
    config: &HostConfig,
    locator: &mut impl WindowLocator,
    capture: &mut impl CaptureStarter,
    started: &StartedProgram,
) -> Result<HostSessionEndReason, HostSessionError> {
    let window = locate_started_window(config, started, locator).map_err(|error| {
        let message = format!("定位宿主端程序窗口失败: {error}");
        let _ = write_control_error(writer, ErrorCode::WindowNotFound, message.clone());
        HostSessionError::new(HostSessionEndReason::CaptureFailed, message)
    })?;
    let (mut session, first_frame) =
        start_capture_session(config, &window, capture).map_err(|error| {
            let message = format!("初始化画面捕获失败: {error}");
            let _ = write_control_error(writer, ErrorCode::CaptureFailed, message.clone());
            HostSessionError::new(HostSessionEndReason::CaptureFailed, message)
        })?;
    write_session_ready(writer, &first_frame)
        .map_err(|message| HostSessionError::new(HostSessionEndReason::TransportFailed, message))?;
    write_raw_bgra_stream(
        writer,
        stream,
        &first_frame,
        session.as_mut(),
        capture_input_bounds(config, &window, &first_frame),
    )
}

trait WindowLocator {
    fn find_main_window(
        &mut self,
        process_id: u32,
        title_contains: Option<&str>,
    ) -> Result<WindowCandidate, WindowLookupError>;
}

struct WindowsWindowLocator;

impl WindowLocator for WindowsWindowLocator {
    fn find_main_window(
        &mut self,
        process_id: u32,
        title_contains: Option<&str>,
    ) -> Result<WindowCandidate, WindowLookupError> {
        find_main_window(process_id, title_contains)
    }
}

trait CaptureStarter {
    fn start_capture(
        &mut self,
        target: CaptureTarget,
    ) -> Result<Box<dyn CaptureRuntime>, CaptureError>;
}

trait CaptureRuntime {
    fn is_active(&self) -> bool;

    fn try_next_bgra_frame(&mut self) -> Result<Option<CapturedBgraFrame>, CaptureError>;
}

trait InputEventSink {
    fn handle_input_event(&mut self, event: InputEvent) -> Result<(), String>;
}

impl InputEventSink for WindowsInputEventSink {
    fn handle_input_event(&mut self, event: InputEvent) -> Result<(), String> {
        self.inject(event).map_err(|error| error.to_string())
    }
}

struct StdCaptureStarter;

impl CaptureStarter for StdCaptureStarter {
    fn start_capture(
        &mut self,
        target: CaptureTarget,
    ) -> Result<Box<dyn CaptureRuntime>, CaptureError> {
        Ok(Box::new(CaptureSession::start(target)?))
    }
}

impl CaptureRuntime for CaptureSession {
    fn is_active(&self) -> bool {
        self.is_active()
    }

    fn try_next_bgra_frame(&mut self) -> Result<Option<CapturedBgraFrame>, CaptureError> {
        self.try_next_bgra_frame()
    }
}

fn locate_started_window(
    config: &HostConfig,
    started: &StartedProgram,
    locator: &mut impl WindowLocator,
) -> Result<WindowCandidate, WindowLookupError> {
    let deadline = Instant::now() + Duration::from_millis(config.capture.startup_timeout_ms);
    let title_contains = Some(config.capture.window_title_contains.as_str());

    loop {
        let last_error = match locator.find_main_window(started.process_id, title_contains) {
            Ok(window) => return Ok(window),
            Err(error) => error,
        };

        if Instant::now() >= deadline {
            return Err(last_error);
        }

        thread::sleep(Duration::from_millis(100));
    }
}

fn start_capture_session(
    config: &HostConfig,
    window: &WindowCandidate,
    capture: &mut impl CaptureStarter,
) -> Result<(Box<dyn CaptureRuntime>, CapturedBgraFrame), CaptureError> {
    let mut session = capture.start_capture(capture_target(config, window))?;
    let first_frame = wait_next_capture_result_with(
        Duration::from_millis(config.capture.startup_timeout_ms),
        || session.try_next_bgra_frame(),
    )?;
    Ok((session, first_frame))
}

fn capture_target(config: &HostConfig, window: &WindowCandidate) -> CaptureTarget {
    match config.capture.mode {
        CaptureMode::Desktop => CaptureTarget::Desktop,
        CaptureMode::Window => CaptureTarget::Window {
            handle: window.handle,
            width: window.rect.width() as u32,
            height: window.rect.height() as u32,
            title: (!window.title.is_empty()).then_some(window.title.clone()),
        },
    }
}

fn capture_input_bounds(
    config: &HostConfig,
    window: &WindowCandidate,
    frame: &CapturedBgraFrame,
) -> CaptureInputBounds {
    match config.capture.mode {
        CaptureMode::Desktop => CaptureInputBounds {
            origin_x: 0,
            origin_y: 0,
            width: frame.metadata.frame.width,
            height: frame.metadata.frame.height,
        },
        CaptureMode::Window => CaptureInputBounds {
            origin_x: window.rect.left,
            origin_y: window.rect.top,
            width: frame.metadata.frame.width,
            height: frame.metadata.frame.height,
        },
    }
}

fn write_control_error(
    writer: &mut impl std::io::Write,
    code: ErrorCode,
    message: String,
) -> Result<(), String> {
    write_message(writer, &ControlMessage::Error { code, message })
        .map_err(|error| format!("写入控制错误消息失败: {error}"))
}

fn write_session_ready(
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

fn write_raw_bgra_stream(
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
    write_raw_bgra_stream_with_input_events(writer, first_frame, session, &input_events)
}

fn write_raw_bgra_stream_with_input_events(
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

fn write_session_goodbye(writer: &mut impl std::io::Write) -> Result<(), String> {
    match write_message(writer, &ControlMessage::Goodbye) {
        Ok(()) => Ok(()),
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
enum InputReaderEvent {
    StopSession,
    Disconnected,
    Failed(String),
}

fn spawn_input_event_reader(
    mut input_reader: TcpStream,
    input_bounds: CaptureInputBounds,
) -> mpsc::Receiver<InputReaderEvent> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut input_sink = WindowsInputEventSink::new(input_bounds);
        let event = read_input_events_until_stop(&mut input_reader, &mut input_sink);
        let _ = sender.send(event);
    });
    receiver
}

fn read_input_events_until_stop(
    reader: &mut impl std::io::Read,
    sink: &mut impl InputEventSink,
) -> InputReaderEvent {
    loop {
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
                if is_control_stream_closed(&error) {
                    return InputReaderEvent::Disconnected;
                }
                return InputReaderEvent::Failed(format!("读取客户端输入事件失败: {error}"));
            }
        }
    }
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

#[cfg(test)]
fn run_control_listener_once_with_runtime(
    listener: TcpListener,
    config: &HostConfig,
    runner: &mut impl ProgramRunner,
    locator: &mut impl WindowLocator,
    capture: &mut impl CaptureStarter,
) -> Result<SocketAddr, String> {
    let local_addr = listener
        .local_addr()
        .map_err(|error| format!("读取宿主端监听地址失败: {error}"))?;
    let (mut stream, _peer_addr) = listener
        .accept()
        .map_err(|error| format!("接受客户端连接失败: {error}"))?;
    handle_control_client(&mut stream, config, runner, locator, capture)?;
    Ok(local_addr)
}

#[cfg(test)]
fn run_control_listener_once_with_runtime_and_session_gate(
    listener: TcpListener,
    config: &HostConfig,
    runner: &mut impl ProgramRunner,
    locator: &mut impl WindowLocator,
    capture: &mut impl CaptureStarter,
    session_gate: &mut impl SessionGate,
) -> Result<SocketAddr, String> {
    let local_addr = listener
        .local_addr()
        .map_err(|error| format!("读取宿主端监听地址失败: {error}"))?;
    let (mut stream, _peer_addr) = listener
        .accept()
        .map_err(|error| format!("接受客户端连接失败: {error}"))?;
    handle_control_client_with_session_gate(
        &mut stream,
        config,
        runner,
        locator,
        capture,
        session_gate,
    )?;
    Ok(local_addr)
}

#[cfg(test)]
fn run_control_listener_n_with_runtime(
    listener: TcpListener,
    config: &HostConfig,
    runner: &mut (impl ProgramRunner + Send),
    locator: &mut (impl WindowLocator + Send),
    capture: &mut (impl CaptureStarter + Send),
    sessions: usize,
) -> Result<SocketAddr, String> {
    let local_addr = listener
        .local_addr()
        .map_err(|error| format!("读取宿主端监听地址失败: {error}"))?;
    thread::scope(|scope| {
        run_control_listener_accept_loop(
            listener,
            config,
            runner,
            locator,
            capture,
            scope,
            Some(sessions),
        )
    })?;
    Ok(local_addr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_state::{ClientSessionErrorCode, RemoteSessionStatus};
    use crate::{program, window};
    use std::{
        collections::VecDeque,
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
    };
    use wincast_protocol::{
        config::{CaptureConfig, VideoCodec, VideoConfig},
        handshake::send_client_hello,
        input::{ButtonState, Modifiers},
        raw_frame::read_raw_bgra_frame,
    };

    #[test]
    fn host_accepts_one_tcp_control_handshake_and_launches_program_before_streaming_raw_bgra() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let endpoint = listener
            .local_addr()
            .expect("listener addr should be available");
        let config = host_config(endpoint.to_string());
        let mut runner = RecordingProgramRunner::default();
        let mut locator = RecordingWindowLocator::default();
        let mut capture = RecordingCaptureStarter::default();
        let host = thread::spawn(move || {
            let result = run_control_listener_once_with_runtime(
                listener,
                &config,
                &mut runner,
                &mut locator,
                &mut capture,
            );
            (result, runner.launched, locator.lookups, capture.targets)
        });

        let mut client = TcpStream::connect(endpoint).expect("client should connect");
        send_client_hello(&mut client).expect("client hello should write");
        assert_eq!(
            read_message(&mut client).expect("host hello should read"),
            ControlMessage::Hello { version: 1 }
        );
        write_message(&mut client, &ControlMessage::StartSession)
            .expect("start session should write");

        assert_eq!(
            read_message(&mut client).expect("session ready should read"),
            ControlMessage::SessionReady {
                width: 1280,
                height: 720,
            }
        );
        assert_eq!(
            read_message(&mut client).expect("video ready should read"),
            ControlMessage::VideoReady
        );
        let frame = read_raw_bgra_frame(&mut client).expect("raw binary frame should read");
        assert_eq!(frame.width, 1280);
        assert_eq!(frame.height, 720);
        assert_eq!(frame.row_pitch, 5120);
        assert_eq!(frame.bytes.len(), 5120 * 720);

        let (host_result, launched, lookups, capture_targets) =
            host.join().expect("host thread should finish");
        assert_eq!(
            host_result.expect("host should handle one client"),
            endpoint
        );
        assert_eq!(
            launched,
            vec![("C:\\Program Files\\SomeApp\\app.exe".to_owned(), Vec::new())]
        );
        assert_eq!(lookups, vec![(42, None)]);
        assert_eq!(capture_targets, vec![CaptureTarget::Desktop]);
    }

    #[test]
    fn host_accepts_two_clients_in_sequence_without_rebinding() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let endpoint = listener
            .local_addr()
            .expect("listener addr should be available");
        let config = host_config(endpoint.to_string());
        let mut runner = RecordingProgramRunner::default();
        let mut locator = RecordingWindowLocator::default();
        let mut capture = RecordingCaptureStarter::default();
        let host = thread::spawn(move || {
            let result = run_control_listener_n_with_runtime(
                listener,
                &config,
                &mut runner,
                &mut locator,
                &mut capture,
                2,
            );
            (
                result,
                runner.launched,
                runner.cleaned,
                locator.lookups,
                capture.targets,
            )
        });

        let first = run_short_client_session(endpoint);
        let second = run_short_client_session(endpoint);

        assert_eq!(first.sequence_number, 0);
        assert_eq!(second.sequence_number, 0);
        let (host_result, launched, cleaned, lookups, capture_targets) =
            host.join().expect("host thread should finish");
        assert_eq!(
            host_result.expect("host should handle two clients"),
            endpoint
        );
        assert_eq!(launched.len(), 2);
        assert_eq!(cleaned, vec![42, 43]);
        assert_eq!(lookups, vec![(42, None), (43, None)]);
        assert_eq!(
            capture_targets,
            vec![CaptureTarget::Desktop, CaptureTarget::Desktop]
        );
    }

    #[test]
    fn host_rejects_second_client_while_session_active() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let endpoint = listener
            .local_addr()
            .expect("listener addr should be available");
        let config = host_config(endpoint.to_string());
        let mut runner = RecordingProgramRunner::default();
        let mut locator = RecordingWindowLocator::default();
        let block = Arc::new(BlockingFrameGate::new());
        let mut capture = RecordingCaptureStarter {
            frames: VecDeque::from([Some(captured_bgra_frame()), None]),
            block_after_empty: Some(block.clone()),
            ..Default::default()
        };
        let host = thread::spawn(move || {
            let result = run_control_listener_n_with_runtime(
                listener,
                &config,
                &mut runner,
                &mut locator,
                &mut capture,
                3,
            );
            (result, runner.launched, runner.cleaned)
        });

        let mut first_client = connect_and_start_session(endpoint);
        read_message(&mut first_client).expect("first session ready should read");
        read_message(&mut first_client).expect("first video ready should read");
        read_raw_bgra_frame(&mut first_client).expect("first raw frame should read");
        block.wait_until_blocked();

        let mut second_client = TcpStream::connect(endpoint).expect("second client should connect");
        send_client_hello(&mut second_client).expect("second client hello should write");
        assert_eq!(
            read_message(&mut second_client).expect("busy response should read"),
            ControlMessage::Error {
                code: ErrorCode::Busy,
                message: "宿主端已有客户端连接".to_owned(),
            }
        );

        write_message(&mut first_client, &ControlMessage::StopSession)
            .expect("stop session should write");
        block.release();
        assert_eq!(
            read_message(&mut first_client).expect("first goodbye should read"),
            ControlMessage::Goodbye
        );

        let third = run_short_client_session(endpoint);

        assert_eq!(third.sequence_number, 0);
        let (host_result, launched, cleaned) = host.join().expect("host thread should finish");
        assert_eq!(host_result.expect("host should keep listening"), endpoint);
        assert_eq!(launched.len(), 2);
        assert_eq!(cleaned, vec![42, 43]);
    }

    #[test]
    fn host_rejects_start_session_when_remote_session_is_locked_before_launching_program() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let endpoint = listener
            .local_addr()
            .expect("listener addr should be available");
        let config = host_config(endpoint.to_string());
        let mut runner = RecordingProgramRunner::default();
        let mut locator = RecordingWindowLocator::default();
        let mut capture = RecordingCaptureStarter::default();
        let mut session_gate = FixedSessionGate(RemoteSessionStatus::Rejected {
            code: ClientSessionErrorCode::SessionLocked,
            message: "Windows 会话已锁定，请先解锁后再启动远程会话。",
        });
        let host = thread::spawn(move || {
            let result = run_control_listener_once_with_runtime_and_session_gate(
                listener,
                &config,
                &mut runner,
                &mut locator,
                &mut capture,
                &mut session_gate,
            );
            (
                result,
                runner.launched,
                runner.cleaned,
                locator.lookups,
                capture.targets,
            )
        });

        let mut client = TcpStream::connect(endpoint).expect("client should connect");
        send_client_hello(&mut client).expect("client hello should write");
        assert_eq!(
            read_message(&mut client).expect("host hello should read"),
            ControlMessage::Hello { version: 1 }
        );
        write_message(&mut client, &ControlMessage::StartSession)
            .expect("start session should write");

        assert_eq!(
            read_message(&mut client).expect("session rejection should read"),
            ControlMessage::Error {
                code: ErrorCode::SessionLocked,
                message: "Windows 会话已锁定，请先解锁后再启动远程会话。".to_owned(),
            }
        );

        let (host_result, launched, cleaned, lookups, capture_targets) =
            host.join().expect("host thread should finish");
        let error = host_result.expect_err("host should report session rejection");
        assert!(error.contains("Windows 会话已锁定"));
        assert!(launched.is_empty());
        assert!(cleaned.is_empty());
        assert!(lookups.is_empty());
        assert!(capture_targets.is_empty());
    }

    #[test]
    fn host_cleans_program_after_stop_session_and_waits_for_next_client() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let endpoint = listener
            .local_addr()
            .expect("listener addr should be available");
        let config = host_config(endpoint.to_string());
        let mut runner = RecordingProgramRunner::default();
        let mut locator = RecordingWindowLocator::default();
        let mut capture = RecordingCaptureStarter::default();
        let host = thread::spawn(move || {
            let result = run_control_listener_n_with_runtime(
                listener,
                &config,
                &mut runner,
                &mut locator,
                &mut capture,
                2,
            );
            (result, runner.cleaned)
        });

        let mut first_client = connect_and_start_session(endpoint);
        read_message(&mut first_client).expect("session ready should read");
        read_message(&mut first_client).expect("video ready should read");
        read_raw_bgra_frame(&mut first_client).expect("first raw frame should read");
        write_message(&mut first_client, &ControlMessage::StopSession)
            .expect("stop session should write");
        assert_eq!(
            read_message(&mut first_client).expect("goodbye should read after stop"),
            ControlMessage::Goodbye
        );

        let second = run_short_client_session(endpoint);

        assert_eq!(second.sequence_number, 0);
        let (host_result, cleaned) = host.join().expect("host thread should finish");
        assert_eq!(host_result.expect("host should keep listening"), endpoint);
        assert_eq!(cleaned, vec![42, 43]);
    }

    #[test]
    fn host_sends_goodbye_when_capture_session_finishes() {
        let mut writer = Vec::new();
        let mut session = RecordingCaptureRuntime {
            frames: VecDeque::from([None]),
            attempts: Arc::new(AtomicUsize::new(0)),
            block_after_empty: None,
        };
        let (_sender, receiver) = mpsc::channel();

        let reason = write_raw_bgra_stream_with_input_events(
            &mut writer,
            &captured_bgra_frame(),
            &mut session,
            &receiver,
        )
        .expect("capture end should be reported as a clean session end");

        assert_eq!(reason, HostSessionEndReason::CaptureInactive);
        assert_eq!(
            SessionEndReason::from(reason),
            SessionEndReason::DesktopUnavailable
        );

        let mut reader = writer.as_slice();
        assert_eq!(
            read_message(&mut reader).expect("video ready should decode"),
            ControlMessage::VideoReady
        );
        let frame = read_raw_bgra_frame(&mut reader).expect("first frame should decode");
        assert_eq!(frame.sequence_number, 0);
        assert_eq!(
            read_message(&mut reader).expect("goodbye should decode"),
            ControlMessage::Goodbye
        );
        assert!(
            read_message(&mut reader).is_err(),
            "capture finish should not send an error after goodbye"
        );
    }

    #[test]
    fn host_reports_capture_error_before_returning_frame_read_failure() {
        let mut writer = Vec::new();
        let mut session = FrameReadFailingCaptureRuntime;
        let (_sender, receiver) = mpsc::channel();

        let error = write_raw_bgra_stream_with_input_events(
            &mut writer,
            &captured_bgra_frame(),
            &mut session,
            &receiver,
        )
        .expect_err("capture read failure should be returned to host");

        assert_eq!(error.reason, HostSessionEndReason::CaptureFailed);
        assert_eq!(
            SessionEndReason::from(error.reason),
            SessionEndReason::SessionFailed
        );
        assert!(error.message.contains("读取后续 raw BGRA 捕获帧失败"));
        assert!(error.message.contains("D3D readback failed"));

        let mut reader = writer.as_slice();
        assert_eq!(
            read_message(&mut reader).expect("video ready should decode"),
            ControlMessage::VideoReady
        );
        let frame = read_raw_bgra_frame(&mut reader).expect("first frame should decode");
        assert_eq!(frame.sequence_number, 0);
        assert_eq!(
            read_message(&mut reader).expect("capture error should decode"),
            ControlMessage::Error {
                code: ErrorCode::CaptureFailed,
                message:
                    "读取后续 raw BGRA 捕获帧失败: 读取 Windows 捕获帧失败: D3D readback failed"
                        .to_string(),
            }
        );
    }

    #[test]
    fn goodbye_write_ignores_already_closed_client_connection() {
        let reader = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let endpoint = reader
            .local_addr()
            .expect("listener addr should be available");
        let client = TcpStream::connect(endpoint).expect("client should connect");
        let (mut server, _) = reader.accept().expect("server should accept");
        drop(client);

        write_session_goodbye(&mut server)
            .expect("closed client should still be treated as ended session");
    }

    #[test]
    fn host_can_send_first_raw_binary_frame_after_session_ready() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let endpoint = listener
            .local_addr()
            .expect("listener addr should be available");
        let config = host_config(endpoint.to_string());
        let mut runner = RecordingProgramRunner::default();
        let mut locator = RecordingWindowLocator::default();
        let mut capture = RecordingCaptureStarter::default();
        let host = thread::spawn(move || {
            let result = run_control_listener_once_with_runtime(
                listener,
                &config,
                &mut runner,
                &mut locator,
                &mut capture,
            );
            (result, runner.launched, locator.lookups, capture.targets)
        });

        let mut client = TcpStream::connect(endpoint).expect("client should connect");
        send_client_hello(&mut client).expect("client hello should write");
        read_message(&mut client).expect("host hello should read");
        write_message(&mut client, &ControlMessage::StartSession)
            .expect("start session should write");

        assert_eq!(
            read_message(&mut client).expect("session ready should read"),
            ControlMessage::SessionReady {
                width: 1280,
                height: 720,
            }
        );
        assert_eq!(
            read_message(&mut client).expect("video ready should read"),
            ControlMessage::VideoReady
        );
        let frame = read_raw_bgra_frame(&mut client).expect("raw binary frame should read");
        assert_eq!(frame.width, 1280);
        assert_eq!(frame.height, 720);
        assert_eq!(frame.row_pitch, 5120);
        assert_eq!(frame.sequence_number, 0);
        assert_eq!(frame.timestamp_ns, 0);
        assert_eq!(frame.bytes.len(), 5120 * 720);

        let (host_result, launched, lookups, capture_targets) =
            host.join().expect("host thread should finish");
        assert_eq!(
            host_result.expect("host should handle one client"),
            endpoint
        );
        assert_eq!(
            launched,
            vec![("C:\\Program Files\\SomeApp\\app.exe".to_owned(), Vec::new())]
        );
        assert_eq!(lookups, vec![(42, None)]);
        assert_eq!(capture_targets, vec![CaptureTarget::Desktop]);
    }

    #[test]
    fn host_streams_available_raw_binary_frames_after_first_frame() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let endpoint = listener
            .local_addr()
            .expect("listener addr should be available");
        let config = host_config(endpoint.to_string());
        let mut runner = RecordingProgramRunner::default();
        let mut locator = RecordingWindowLocator::default();
        let mut capture = RecordingCaptureStarter {
            frames: VecDeque::from([
                Some(captured_bgra_frame_with_sequence(0)),
                Some(captured_bgra_frame_with_sequence(1)),
                Some(captured_bgra_frame_with_sequence(2)),
                None,
            ]),
            ..Default::default()
        };
        let attempts = capture.attempts.clone();
        let host = thread::spawn(move || {
            let result = run_control_listener_once_with_runtime(
                listener,
                &config,
                &mut runner,
                &mut locator,
                &mut capture,
            );
            (result, runner.launched, locator.lookups, capture.targets)
        });

        let mut client = TcpStream::connect(endpoint).expect("client should connect");
        send_client_hello(&mut client).expect("client hello should write");
        read_message(&mut client).expect("host hello should read");
        write_message(&mut client, &ControlMessage::StartSession)
            .expect("start session should write");

        assert_eq!(
            read_message(&mut client).expect("session ready should read"),
            ControlMessage::SessionReady {
                width: 1280,
                height: 720,
            }
        );
        assert_eq!(
            read_message(&mut client).expect("video ready should read"),
            ControlMessage::VideoReady
        );
        let first = read_raw_bgra_frame(&mut client).expect("first raw frame should read");
        let second = read_raw_bgra_frame(&mut client).expect("second raw frame should read");
        let third = read_raw_bgra_frame(&mut client).expect("third raw frame should read");
        assert_eq!(first.sequence_number, 0);
        assert_eq!(second.sequence_number, 1);
        assert_eq!(third.sequence_number, 2);

        let (host_result, launched, lookups, capture_targets) =
            host.join().expect("host thread should finish");
        assert_eq!(
            host_result.expect("host should handle one client"),
            endpoint
        );
        assert_eq!(attempts.load(Ordering::SeqCst), 4);
        assert_eq!(
            launched,
            vec![("C:\\Program Files\\SomeApp\\app.exe".to_owned(), Vec::new())]
        );
        assert_eq!(lookups, vec![(42, None)]);
        assert_eq!(capture_targets, vec![CaptureTarget::Desktop]);
    }

    #[test]
    fn host_keeps_raw_stream_alive_when_no_frame_is_temporarily_available() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let endpoint = listener
            .local_addr()
            .expect("listener addr should be available");
        let config = host_config(endpoint.to_string());
        let mut runner = RecordingProgramRunner::default();
        let mut locator = RecordingWindowLocator::default();
        let mut capture = RecordingCaptureStarter {
            frames: VecDeque::from([
                Some(captured_bgra_frame_with_sequence(0)),
                None,
                Some(captured_bgra_frame_with_sequence(1)),
                None,
            ]),
            ..Default::default()
        };
        let host = thread::spawn(move || {
            let result = run_control_listener_once_with_runtime(
                listener,
                &config,
                &mut runner,
                &mut locator,
                &mut capture,
            );
            (result, runner.launched, locator.lookups, capture.targets)
        });

        let mut client = TcpStream::connect(endpoint).expect("client should connect");
        send_client_hello(&mut client).expect("client hello should write");
        read_message(&mut client).expect("host hello should read");
        write_message(&mut client, &ControlMessage::StartSession)
            .expect("start session should write");

        read_message(&mut client).expect("session ready should read");
        read_message(&mut client).expect("video ready should read");
        let first = read_raw_bgra_frame(&mut client).expect("first raw frame should read");
        let second = read_raw_bgra_frame(&mut client).expect("second raw frame should read");

        assert_eq!(first.sequence_number, 0);
        assert_eq!(second.sequence_number, 1);

        let (host_result, launched, lookups, capture_targets) =
            host.join().expect("host thread should finish");
        assert_eq!(
            host_result.expect("host should handle one client"),
            endpoint
        );
        assert_eq!(
            launched,
            vec![("C:\\Program Files\\SomeApp\\app.exe".to_owned(), Vec::new())]
        );
        assert_eq!(lookups, vec![(42, None)]);
        assert_eq!(capture_targets, vec![CaptureTarget::Desktop]);
    }

    #[test]
    fn host_reports_window_not_found_after_program_launch() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let endpoint = listener
            .local_addr()
            .expect("listener addr should be available");
        let mut config = host_config(endpoint.to_string());
        config.capture.startup_timeout_ms = 1;
        let mut runner = RecordingProgramRunner::default();
        let mut locator = FailingWindowLocator;
        let mut capture = RecordingCaptureStarter::default();
        let host = thread::spawn(move || {
            run_control_listener_once_with_runtime(
                listener,
                &config,
                &mut runner,
                &mut locator,
                &mut capture,
            )
        });

        let mut client = TcpStream::connect(endpoint).expect("client should connect");
        send_client_hello(&mut client).expect("client hello should write");
        read_message(&mut client).expect("host hello should read");
        write_message(&mut client, &ControlMessage::StartSession)
            .expect("start session should write");

        assert_eq!(
            read_message(&mut client).expect("window error should read"),
            ControlMessage::Error {
                code: ErrorCode::WindowNotFound,
                message: "定位宿主端程序窗口失败: 未找到进程 42 的主窗口".to_owned(),
            }
        );

        let error = host
            .join()
            .expect("host thread should finish")
            .expect_err("host should report window lookup failure");
        assert!(error.contains("定位宿主端程序窗口失败"));
    }

    #[test]
    fn host_reports_capture_failed_after_window_lookup() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let endpoint = listener
            .local_addr()
            .expect("listener addr should be available");
        let config = host_config(endpoint.to_string());
        let mut runner = RecordingProgramRunner::default();
        let mut locator = RecordingWindowLocator::default();
        let mut capture = FailingCaptureStarter;
        let host = thread::spawn(move || {
            run_control_listener_once_with_runtime(
                listener,
                &config,
                &mut runner,
                &mut locator,
                &mut capture,
            )
        });

        let mut client = TcpStream::connect(endpoint).expect("client should connect");
        send_client_hello(&mut client).expect("client hello should write");
        read_message(&mut client).expect("host hello should read");
        write_message(&mut client, &ControlMessage::StartSession)
            .expect("start session should write");

        assert_eq!(
            read_message(&mut client).expect("capture error should read"),
            ControlMessage::Error {
                code: ErrorCode::CaptureFailed,
                message: "初始化画面捕获失败: Windows 画面捕获实现未完成：尚未接入帧获取循环"
                    .to_owned(),
            }
        );

        let error = host
            .join()
            .expect("host thread should finish")
            .expect_err("host should report capture failure");
        assert!(error.contains("初始化画面捕获失败"));
    }

    #[test]
    fn host_treats_missing_initial_frame_as_waitable_state() {
        let config = host_config("127.0.0.1:0".to_owned());
        let window = window_candidate();
        let mut capture = RecordingCaptureStarter {
            frames: VecDeque::from([None, Some(captured_bgra_frame())]),
            ..Default::default()
        };
        let attempts = capture.attempts.clone();

        let (_session, frame) = start_capture_session(&config, &window, &mut capture)
            .expect("host should wait until first frame metadata is available");

        assert_eq!(capture.targets, vec![CaptureTarget::Desktop]);
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        assert_eq!(frame.row_pitch, 5120);
        assert_eq!(frame.bytes.len(), 5120 * 720);
    }

    #[test]
    fn capture_input_bounds_keep_window_origin_for_window_capture() {
        let mut config = host_config("127.0.0.1:0".to_owned());
        config.capture.mode = CaptureMode::Window;
        let mut window = window_candidate();
        window.rect = window::WindowRect {
            left: 50,
            top: 75,
            right: 1330,
            bottom: 795,
        };
        let frame = captured_bgra_frame();

        let bounds = capture_input_bounds(&config, &window, &frame);

        assert_eq!(
            bounds,
            CaptureInputBounds {
                origin_x: 50,
                origin_y: 75,
                width: 1280,
                height: 720,
            }
        );
    }

    #[test]
    fn capture_input_bounds_use_frame_size_for_desktop_capture() {
        let config = host_config("127.0.0.1:0".to_owned());
        let window = window_candidate();
        let frame = captured_bgra_frame();

        let bounds = capture_input_bounds(&config, &window, &frame);

        assert_eq!(
            bounds,
            CaptureInputBounds {
                origin_x: 0,
                origin_y: 0,
                width: 1280,
                height: 720,
            }
        );
    }

    #[test]
    fn host_reports_capture_failed_when_initial_frame_times_out() {
        let mut config = host_config("127.0.0.1:0".to_owned());
        config.capture.startup_timeout_ms = 1;
        let window = window_candidate();
        let mut capture = RecordingCaptureStarter {
            frames: VecDeque::from([None, None]),
            ..Default::default()
        };

        let error = match start_capture_session(&config, &window, &mut capture) {
            Ok(_) => panic!("host should fail when no frame metadata arrives before timeout"),
            Err(error) => error,
        };

        assert_eq!(
            error,
            CaptureError::windows_frame_read_failed("等待 Windows 捕获首帧超时")
        );
    }

    #[test]
    fn input_reader_handles_client_input_events_until_stop_session() {
        let mut bytes = Vec::new();
        write_message(
            &mut bytes,
            &ControlMessage::InputEvent(InputEvent::Key {
                code: 65,
                state: ButtonState::Pressed,
                modifiers: Modifiers {
                    shift: true,
                    ctrl: false,
                    alt: false,
                    logo: false,
                },
            }),
        )
        .expect("input event should encode");
        write_message(&mut bytes, &ControlMessage::StopSession).expect("stop should encode");
        let mut sink = RecordingInputEventSink::default();

        let event = read_input_events_until_stop(&mut bytes.as_slice(), &mut sink);

        assert_eq!(event, InputReaderEvent::StopSession);
        assert_eq!(
            sink.events,
            vec![InputEvent::Key {
                code: 65,
                state: ButtonState::Pressed,
                modifiers: Modifiers {
                    shift: true,
                    ctrl: false,
                    alt: false,
                    logo: false,
                },
            }]
        );
    }

    #[test]
    fn input_reader_rejects_non_input_messages() {
        let mut bytes = Vec::new();
        write_message(&mut bytes, &ControlMessage::Heartbeat).expect("heartbeat should encode");
        let mut sink = RecordingInputEventSink::default();

        let event = read_input_events_until_stop(&mut bytes.as_slice(), &mut sink);

        assert_eq!(
            event,
            InputReaderEvent::Failed("客户端输入事件消息无效: Heartbeat".to_owned())
        );
        assert!(sink.events.is_empty());
    }

    #[test]
    fn input_reader_reports_stop_session_after_input_events() {
        let mut bytes = Vec::new();
        write_message(
            &mut bytes,
            &ControlMessage::InputEvent(InputEvent::MouseWheel {
                delta_x: 0,
                delta_y: 1,
            }),
        )
        .expect("input event should encode");
        write_message(&mut bytes, &ControlMessage::StopSession).expect("stop should encode");
        let mut sink = RecordingInputEventSink::default();

        let event = read_input_events_until_stop(&mut bytes.as_slice(), &mut sink);

        assert_eq!(event, InputReaderEvent::StopSession);
        assert_eq!(
            sink.events,
            vec![InputEvent::MouseWheel {
                delta_x: 0,
                delta_y: 1,
            }]
        );
    }

    #[test]
    fn raw_bgra_stream_stops_cleanly_when_input_reader_stops() {
        let mut writer = Vec::new();
        let mut session = RecordingCaptureRuntime {
            frames: VecDeque::from([Some(captured_bgra_frame_with_sequence(1))]),
            attempts: Arc::new(AtomicUsize::new(0)),
            block_after_empty: None,
        };
        let (sender, receiver) = mpsc::channel();
        sender
            .send(InputReaderEvent::StopSession)
            .expect("input stop should send");

        let reason = write_raw_bgra_stream_with_input_events(
            &mut writer,
            &captured_bgra_frame(),
            &mut session,
            &receiver,
        )
        .expect("stop session should end raw stream without error");

        assert_eq!(reason, HostSessionEndReason::StopSession);
        assert_eq!(
            SessionEndReason::from(reason),
            SessionEndReason::ServiceRequested
        );

        let mut reader = writer.as_slice();
        assert_eq!(
            read_message(&mut reader).expect("video ready should decode"),
            ControlMessage::VideoReady
        );
        let frame = read_raw_bgra_frame(&mut reader).expect("first frame should decode");
        assert_eq!(frame.sequence_number, 0);
        assert_eq!(session.attempts.load(Ordering::SeqCst), 0);
    }

    #[derive(Default)]
    struct RecordingProgramRunner {
        launched: Vec<(String, Vec<String>)>,
        cleaned: Vec<u32>,
        next_process_id: u32,
    }

    impl ProgramRunner for RecordingProgramRunner {
        fn launch(
            &mut self,
            request: &program::LaunchRequest,
        ) -> Result<program::StartedProgram, program::LaunchError> {
            self.launched
                .push((request.program.display().to_string(), request.args.clone()));
            let process_id = if self.next_process_id == 0 {
                self.next_process_id = 43;
                42
            } else {
                let process_id = self.next_process_id;
                self.next_process_id += 1;
                process_id
            };
            Ok(program::StartedProgram::from_process_id(process_id))
        }

        fn cleanup(
            &mut self,
            started: &mut program::StartedProgram,
        ) -> Result<(), program::LaunchError> {
            self.cleaned.push(started.process_id);
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordingWindowLocator {
        lookups: Vec<(u32, Option<String>)>,
    }

    impl WindowLocator for RecordingWindowLocator {
        fn find_main_window(
            &mut self,
            process_id: u32,
            title_contains: Option<&str>,
        ) -> Result<WindowCandidate, WindowLookupError> {
            self.lookups.push((
                process_id,
                title_contains
                    .map(str::trim)
                    .filter(|title| !title.is_empty())
                    .map(str::to_owned),
            ));
            Ok(WindowCandidate {
                handle: 100,
                process_id,
                title: "SomeApp".to_owned(),
                visible: true,
                tool_window: false,
                rect: window::WindowRect {
                    left: 0,
                    top: 0,
                    right: 1280,
                    bottom: 720,
                },
            })
        }
    }

    struct RecordingCaptureStarter {
        targets: Vec<CaptureTarget>,
        frames: VecDeque<Option<CapturedBgraFrame>>,
        attempts: Arc<AtomicUsize>,
        block_after_empty: Option<Arc<BlockingFrameGate>>,
    }

    impl Default for RecordingCaptureStarter {
        fn default() -> Self {
            Self {
                targets: Vec::new(),
                frames: VecDeque::from([Some(captured_bgra_frame())]),
                attempts: Arc::new(AtomicUsize::new(0)),
                block_after_empty: None,
            }
        }
    }

    impl CaptureStarter for RecordingCaptureStarter {
        fn start_capture(
            &mut self,
            target: CaptureTarget,
        ) -> Result<Box<dyn CaptureRuntime>, CaptureError> {
            self.targets.push(target);
            Ok(Box::new(RecordingCaptureRuntime {
                frames: self.frames.clone(),
                attempts: self.attempts.clone(),
                block_after_empty: self.block_after_empty.clone(),
            }))
        }
    }

    struct FixedSessionGate(RemoteSessionStatus);

    impl SessionGate for FixedSessionGate {
        fn remote_session_status(&mut self) -> RemoteSessionStatus {
            self.0
        }
    }

    struct RecordingCaptureRuntime {
        frames: VecDeque<Option<CapturedBgraFrame>>,
        attempts: Arc<AtomicUsize>,
        block_after_empty: Option<Arc<BlockingFrameGate>>,
    }

    impl CaptureRuntime for RecordingCaptureRuntime {
        fn is_active(&self) -> bool {
            !self.frames.is_empty()
        }

        fn try_next_bgra_frame(&mut self) -> Result<Option<CapturedBgraFrame>, CaptureError> {
            self.attempts.fetch_add(1, Ordering::SeqCst);
            let frame = self.frames.pop_front().flatten();
            if frame.is_none()
                && self.frames.is_empty()
                && let Some(block) = self.block_after_empty.take()
            {
                block.block_until_released();
            }
            Ok(frame)
        }
    }

    struct BlockingFrameGate {
        blocked: AtomicBool,
        released: AtomicBool,
    }

    impl BlockingFrameGate {
        fn new() -> Self {
            Self {
                blocked: AtomicBool::new(false),
                released: AtomicBool::new(false),
            }
        }

        fn wait_until_blocked(&self) {
            while !self.blocked.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(1));
            }
        }

        fn release(&self) {
            self.released.store(true, Ordering::SeqCst);
        }

        fn block_until_released(&self) {
            self.blocked.store(true, Ordering::SeqCst);
            while !self.released.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(1));
            }
        }
    }

    impl Drop for BlockingFrameGate {
        fn drop(&mut self) {
            self.released.store(true, Ordering::SeqCst);
        }
    }

    struct FrameReadFailingCaptureRuntime;

    impl CaptureRuntime for FrameReadFailingCaptureRuntime {
        fn is_active(&self) -> bool {
            true
        }

        fn try_next_bgra_frame(&mut self) -> Result<Option<CapturedBgraFrame>, CaptureError> {
            Err(CaptureError::windows_frame_read_failed(
                "D3D readback failed",
            ))
        }
    }

    struct FailingCaptureStarter;

    impl CaptureStarter for FailingCaptureStarter {
        fn start_capture(
            &mut self,
            _target: CaptureTarget,
        ) -> Result<Box<dyn CaptureRuntime>, CaptureError> {
            Err(CaptureError::windows_capture_not_implemented())
        }
    }

    struct FailingWindowLocator;

    impl WindowLocator for FailingWindowLocator {
        fn find_main_window(
            &mut self,
            process_id: u32,
            _title_contains: Option<&str>,
        ) -> Result<WindowCandidate, WindowLookupError> {
            Err(WindowLookupError::NotFound {
                process_id,
                title_contains: None,
            })
        }
    }

    #[derive(Default)]
    struct RecordingInputEventSink {
        events: Vec<InputEvent>,
    }

    impl InputEventSink for RecordingInputEventSink {
        fn handle_input_event(&mut self, event: InputEvent) -> Result<(), String> {
            self.events.push(event);
            Ok(())
        }
    }

    fn host_config(listen: String) -> HostConfig {
        HostConfig {
            listen,
            program: "C:\\Program Files\\SomeApp\\app.exe".to_owned(),
            args: Vec::new(),
            work_dir: "C:\\Program Files\\SomeApp".to_owned(),
            video: VideoConfig {
                width: 1280,
                height: 720,
                fps: 30,
                codec: VideoCodec::H264,
                bitrate_kbps: 4000,
            },
            capture: CaptureConfig {
                mode: CaptureMode::Desktop,
                window_title_contains: String::new(),
                startup_timeout_ms: 15000,
            },
        }
    }

    fn window_candidate() -> WindowCandidate {
        WindowCandidate {
            handle: 100,
            process_id: 42,
            title: "SomeApp".to_owned(),
            visible: true,
            tool_window: false,
            rect: window::WindowRect {
                left: 0,
                top: 0,
                right: 1280,
                bottom: 720,
            },
        }
    }

    fn captured_bgra_frame() -> CapturedBgraFrame {
        captured_bgra_frame_with_sequence(0)
    }

    fn captured_bgra_frame_with_sequence(sequence_number: u64) -> CapturedBgraFrame {
        CapturedBgraFrame {
            metadata: wincast_capture::CapturedTextureMetadata {
                frame: wincast_capture::CapturedFrame {
                    width: 1280,
                    height: 720,
                    stride_bytes: 5120,
                    pixel_format: wincast_capture::FramePixelFormat::Bgra8Unorm,
                    sequence_number,
                    timestamp_ns: sequence_number * 1_000_000,
                },
                texture_width: 1280,
                texture_height: 720,
                mip_levels: 1,
                array_size: 1,
                sample_count: 1,
            },
            row_pitch: 5120,
            bytes: vec![0; 5120 * 720],
        }
    }

    fn connect_and_start_session(endpoint: SocketAddr) -> TcpStream {
        let mut client = TcpStream::connect(endpoint).expect("client should connect");
        send_client_hello(&mut client).expect("client hello should write");
        assert_eq!(
            read_message(&mut client).expect("host hello should read"),
            ControlMessage::Hello { version: 1 }
        );
        write_message(&mut client, &ControlMessage::StartSession)
            .expect("start session should write");
        client
    }

    fn run_short_client_session(endpoint: SocketAddr) -> RawBgraFrame {
        let mut client = connect_and_start_session_when_ready(endpoint);
        read_message(&mut client).expect("session ready should read");
        read_message(&mut client).expect("video ready should read");
        let frame = read_raw_bgra_frame(&mut client).expect("raw frame should read");
        write_message(&mut client, &ControlMessage::StopSession)
            .expect("stop session should write");
        assert_eq!(
            read_message(&mut client).expect("goodbye should read after stop"),
            ControlMessage::Goodbye
        );
        frame
    }

    fn connect_and_start_session_when_ready(endpoint: SocketAddr) -> TcpStream {
        let mut last_error = None;
        for _ in 0..50 {
            let mut client = TcpStream::connect(endpoint).expect("client should connect");
            send_client_hello(&mut client).expect("client hello should write");
            match read_message(&mut client).expect("host hello or busy should read") {
                ControlMessage::Hello { version: 1 } => {
                    write_message(&mut client, &ControlMessage::StartSession)
                        .expect("start session should write");
                    return client;
                }
                ControlMessage::Error {
                    code: ErrorCode::Busy,
                    message,
                } => {
                    last_error = Some(message);
                    thread::sleep(Duration::from_millis(20));
                }
                message => {
                    panic!("unexpected host response while waiting for session: {message:?}")
                }
            }
        }
        panic!(
            "host should accept a new session after previous cleanup, last busy: {:?}",
            last_error
        );
    }
}
