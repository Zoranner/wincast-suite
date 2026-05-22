use std::{
    net::TcpStream,
    time::{Duration, Instant},
};

use crate::{
    program::{ProgramRunner, StartedProgram},
    session_events::{
        DesktopSessionDetector, DesktopSessionError, DetectedDesktopSession,
        PlatformDesktopSessionDetector, detect_desktop_session,
    },
    session_state::{RemoteSessionStatus, SessionEvent, SharedSessionState},
};
use wincast_media::{VideoLatencyMode, VideoPipelineConfig};
use wincast_protocol::{
    config::{HostConfig, VideoCodec},
    frame::{FrameError, read_message},
    handshake::accept_client_hello,
    message::{ControlMessage, ErrorCode},
};

use super::{
    capture::{CaptureStarter, screen_input_bounds, start_screen_capture_session},
    stream::{
        H264StreamRuntime, HostSessionEndReason, HostSessionError, write_control_error,
        write_h264_encoded_stream, write_session_goodbye, write_session_ready,
    },
};

const STARTUP_DELAY_POLL_INTERVAL: Duration = Duration::from_millis(50);

pub(super) fn handle_control_client(
    stream: &mut TcpStream,
    config: &HostConfig,
    runner: &mut impl ProgramRunner,
    capture: &mut impl CaptureStarter,
) -> Result<(), String> {
    let mut session_gate = PollingSessionGate::new(
        foreground_run_session_state(),
        PlatformDesktopSessionDetector,
        foreground_detection_failure_should_fallback_to_development(),
    );
    handle_control_client_with_session_gate(stream, config, runner, capture, &mut session_gate)
}

pub(super) fn handle_control_client_with_session_gate(
    stream: &mut TcpStream,
    config: &HostConfig,
    runner: &mut impl ProgramRunner,
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
            let result = run_started_session(
                &mut writer,
                stream,
                config,
                capture,
                &mut started,
                session_gate,
            );
            let cleanup_result = runner
                .cleanup(&mut started)
                .map_err(|error| format!("清理宿主端程序失败: {error}"));
            match (result, cleanup_result) {
                (Ok(_reason), Ok(())) => Ok(()),
                (Err(error), Ok(())) => Err(error.message),
                (Ok(_reason), Err(error)) => Err(error),
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

pub(super) trait SessionGate {
    fn remote_session_status(&self) -> RemoteSessionStatus;
}

pub(super) struct PollingSessionGate<D>
where
    D: DesktopSessionDetector,
{
    state: SharedSessionState,
    detector: D,
    fallback_to_development: bool,
}

impl<D> PollingSessionGate<D>
where
    D: DesktopSessionDetector,
{
    pub(super) fn new(
        state: SharedSessionState,
        detector: D,
        fallback_to_development: bool,
    ) -> Self {
        Self {
            state,
            detector,
            fallback_to_development,
        }
    }
}

impl<D> SessionGate for PollingSessionGate<D>
where
    D: DesktopSessionDetector,
{
    fn remote_session_status(&self) -> RemoteSessionStatus {
        match self.detector.detect_desktop_session() {
            Ok(detected) => {
                self.state.apply_detected_desktop_session(detected);
            }
            Err(_) if self.fallback_to_development => {}
            Err(_) => {
                return RemoteSessionStatus::Rejected {
                    code: crate::session_state::ClientSessionErrorCode::NoUserLoggedIn,
                    message: "当前没有 Windows 用户登录，无法启动远程会话。",
                };
            }
        }
        self.state.remote_session_status()
    }
}

fn foreground_run_session_state() -> SharedSessionState {
    foreground_run_session_state_from_detection_with_failure_policy(
        detect_desktop_session(),
        foreground_detection_failure_should_fallback_to_development(),
    )
}

#[cfg(test)]
pub(super) fn foreground_run_session_state_from_detection(
    detected: Result<DetectedDesktopSession, DesktopSessionError>,
) -> SharedSessionState {
    foreground_run_session_state_from_detection_with_failure_policy(detected, false)
}

pub(super) fn foreground_run_session_state_from_detection_with_failure_policy(
    detected: Result<DetectedDesktopSession, DesktopSessionError>,
    fallback_to_development: bool,
) -> SharedSessionState {
    let state = detected
        .map(crate::session_events::shared_session_state_from_detected_desktop_session)
        .unwrap_or_else(|_| {
            foreground_session_state_after_detection_failure(fallback_to_development)
        });
    state.apply(SessionEvent::AgentStarted);
    state
}

fn foreground_session_state_after_detection_failure(
    fallback_to_development: bool,
) -> SharedSessionState {
    if fallback_to_development {
        foreground_development_session_state()
    } else {
        SharedSessionState::new()
    }
}

#[cfg(windows)]
fn foreground_detection_failure_should_fallback_to_development() -> bool {
    false
}

#[cfg(not(windows))]
fn foreground_detection_failure_should_fallback_to_development() -> bool {
    true
}

fn foreground_development_session_state() -> SharedSessionState {
    let state = SharedSessionState::new();
    state.apply(SessionEvent::UserLoggedIn);
    state
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

pub(super) fn run_started_session(
    writer: &mut impl std::io::Write,
    stream: &TcpStream,
    config: &HostConfig,
    capture: &mut impl CaptureStarter,
    started: &mut StartedProgram,
    session_gate: &impl SessionGate,
) -> Result<HostSessionEndReason, HostSessionError> {
    if let Some(reason) = wait_for_startup_delay(
        writer,
        stream,
        config.program.startup_delay_ms,
        started,
        session_gate,
    )? {
        return Ok(reason);
    }
    let (mut session, first_frame) =
        start_screen_capture_session(config, capture).map_err(|error| {
            let message = format!("初始化画面捕获失败: {error}");
            let write_result =
                write_control_error(writer, ErrorCode::CaptureFailed, message.clone());
            let message = append_error_response_write_failure(message, write_result);
            HostSessionError::new(HostSessionEndReason::CaptureFailed, message)
        })?;
    write_session_ready(writer, &first_frame)
        .map_err(|message| HostSessionError::new(HostSessionEndReason::TransportFailed, message))?;
    write_h264_encoded_stream(
        writer,
        stream,
        &first_frame,
        session.as_mut(),
        H264StreamRuntime {
            input_bounds: screen_input_bounds(&first_frame),
            pipeline_config: h264_pipeline_config(config),
            session_gate,
            program_status: started,
        },
    )
}

fn wait_for_startup_delay(
    writer: &mut impl std::io::Write,
    stream: &TcpStream,
    startup_delay_ms: u64,
    started: &mut StartedProgram,
    session_gate: &impl SessionGate,
) -> Result<Option<HostSessionEndReason>, HostSessionError> {
    let delay = Duration::from_millis(startup_delay_ms);
    if let Some(reason) = check_startup_runtime(writer, started, session_gate)? {
        return Ok(Some(reason));
    }
    if delay.is_zero() {
        return Ok(None);
    }

    let mut reader = stream.try_clone().map_err(|error| {
        HostSessionError::new(
            HostSessionEndReason::TransportFailed,
            format!("克隆启动延迟控制读取端失败: {error}"),
        )
    })?;
    let original_timeout = reader.read_timeout().map_err(|error| {
        HostSessionError::new(
            HostSessionEndReason::TransportFailed,
            format!("读取启动延迟控制超时配置失败: {error}"),
        )
    })?;
    let deadline = Instant::now() + delay;

    let result = loop {
        let now = Instant::now();
        if now >= deadline {
            break Ok(None);
        }
        let timeout = (deadline - now).min(STARTUP_DELAY_POLL_INTERVAL);
        reader.set_read_timeout(Some(timeout)).map_err(|error| {
            HostSessionError::new(
                HostSessionEndReason::TransportFailed,
                format!("配置启动延迟控制读取超时失败: {error}"),
            )
        })?;

        match read_message(&mut reader) {
            Ok(ControlMessage::StopSession | ControlMessage::Goodbye) => {
                write_session_goodbye(writer).map_err(|message| {
                    HostSessionError::new(HostSessionEndReason::TransportFailed, message)
                })?;
                break Ok(Some(HostSessionEndReason::StopSession));
            }
            Ok(ControlMessage::Heartbeat) => {}
            Ok(message) => {
                let message = format!("启动延迟期间控制消息无效: {message:?}");
                let _ = write_control_error(writer, ErrorCode::TransportFailed, message.clone());
                break Err(HostSessionError::new(
                    HostSessionEndReason::TransportFailed,
                    message,
                ));
            }
            Err(error) if is_temporary_read_timeout(&error) => {}
            Err(error) if is_control_stream_closed(&error) => {
                break Ok(Some(HostSessionEndReason::ClientDisconnected));
            }
            Err(error) => {
                break Err(HostSessionError::new(
                    HostSessionEndReason::TransportFailed,
                    format!("启动延迟期间读取控制消息失败: {error}"),
                ));
            }
        }

        if let Some(reason) = check_startup_runtime(writer, started, session_gate)? {
            break Ok(Some(reason));
        }
    };

    reader.set_read_timeout(original_timeout).map_err(|error| {
        HostSessionError::new(
            HostSessionEndReason::TransportFailed,
            format!("恢复启动延迟控制读取超时配置失败: {error}"),
        )
    })?;
    result
}

fn check_startup_runtime(
    writer: &mut impl std::io::Write,
    started: &mut StartedProgram,
    session_gate: &impl SessionGate,
) -> Result<Option<HostSessionEndReason>, HostSessionError> {
    if !started.is_running().map_err(|error| {
        HostSessionError::new(
            HostSessionEndReason::ProgramExited,
            format!("检查宿主端程序退出状态失败: {error}"),
        )
    })? {
        let message = "宿主端程序已退出，结束远程会话。".to_owned();
        write_control_error(writer, ErrorCode::ProgramExited, message.clone()).map_err(
            |message| HostSessionError::new(HostSessionEndReason::TransportFailed, message),
        )?;
        write_session_goodbye(writer).map_err(|message| {
            HostSessionError::new(HostSessionEndReason::TransportFailed, message)
        })?;
        return Ok(Some(HostSessionEndReason::ProgramExited));
    }

    if let Some((code, message)) = session_gate.remote_session_status().to_protocol_error() {
        write_control_error(writer, code, message.to_owned()).map_err(|message| {
            HostSessionError::new(HostSessionEndReason::TransportFailed, message)
        })?;
        write_session_goodbye(writer).map_err(|message| {
            HostSessionError::new(HostSessionEndReason::TransportFailed, message)
        })?;
        return Ok(Some(HostSessionEndReason::SessionUnavailable));
    }

    Ok(None)
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

fn h264_pipeline_config(config: &HostConfig) -> VideoPipelineConfig {
    VideoPipelineConfig {
        codec: VideoCodec::H264,
        width: config.video.width,
        height: config.video.height,
        fps: config.video.fps,
        bitrate_kbps: config.video.bitrate_kbps,
        max_bitrate_kbps: config.video.max_bitrate_kbps,
        latency_mode: VideoLatencyMode::LowLatency,
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
