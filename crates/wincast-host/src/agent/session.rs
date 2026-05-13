use std::net::TcpStream;

use crate::{
    program::{ProgramRunner, StartedProgram},
    session_state::{RemoteSessionStatus, SessionEvent, SharedSessionState},
};
use wincast_protocol::{
    config::HostConfig,
    frame::read_message,
    handshake::accept_client_hello,
    ipc::SessionEndReason,
    message::{ControlMessage, ErrorCode},
};

use super::{
    capture::{
        CaptureStarter, WindowLocator, capture_input_bounds, locate_started_window,
        start_capture_session,
    },
    stream::{
        HostSessionEndReason, HostSessionError, write_control_error, write_raw_bgra_stream,
        write_session_ready,
    },
};

pub(super) fn handle_control_client(
    stream: &mut TcpStream,
    config: &HostConfig,
    runner: &mut impl ProgramRunner,
    locator: &mut impl WindowLocator,
    capture: &mut impl CaptureStarter,
) -> Result<(), String> {
    let mut session_gate = SharedSessionGate::new(foreground_run_session_state());
    handle_control_client_with_session_gate(
        stream,
        config,
        runner,
        locator,
        capture,
        &mut session_gate,
    )
}

pub(super) fn handle_control_client_with_session_gate(
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
                (Err(error), Ok(())) => {
                    let _ = SessionEndReason::from(error.reason);
                    Err(error.message)
                }
                (Ok(reason), Err(error)) => {
                    let _ = SessionEndReason::from(reason);
                    Err(error)
                }
                (Err(session_error), Err(cleanup_error)) => {
                    let _ = SessionEndReason::from(session_error.reason);
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
    fn remote_session_status(&mut self) -> RemoteSessionStatus;
}

pub(super) struct SharedSessionGate {
    state: SharedSessionState,
}

impl SharedSessionGate {
    pub(super) fn new(state: SharedSessionState) -> Self {
        Self { state }
    }
}

impl SessionGate for SharedSessionGate {
    fn remote_session_status(&mut self) -> RemoteSessionStatus {
        self.state.remote_session_status()
    }
}

fn foreground_run_session_state() -> SharedSessionState {
    let state = SharedSessionState::new();
    state.apply(SessionEvent::UserLoggedIn);
    state.apply(SessionEvent::AgentStarted);
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
    locator: &mut impl WindowLocator,
    capture: &mut impl CaptureStarter,
    started: &StartedProgram,
) -> Result<HostSessionEndReason, HostSessionError> {
    let window = locate_started_window(config, started, locator).map_err(|error| {
        let message = format!("定位宿主端程序窗口失败: {error}");
        let write_result = write_control_error(writer, ErrorCode::WindowNotFound, message.clone());
        let message = append_error_response_write_failure(message, write_result);
        HostSessionError::new(HostSessionEndReason::CaptureFailed, message)
    })?;
    let (mut session, first_frame) =
        start_capture_session(config, &window, capture).map_err(|error| {
            let message = format!("初始化画面捕获失败: {error}");
            let write_result =
                write_control_error(writer, ErrorCode::CaptureFailed, message.clone());
            let message = append_error_response_write_failure(message, write_result);
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

fn append_error_response_write_failure(
    message: String,
    write_result: Result<(), String>,
) -> String {
    match write_result {
        Ok(()) => message,
        Err(write_error) => format!("{message}；{write_error}"),
    }
}
