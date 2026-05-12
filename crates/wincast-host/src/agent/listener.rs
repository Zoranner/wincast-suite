use std::{
    net::{SocketAddr, TcpListener, TcpStream},
    sync::mpsc,
    thread,
    time::Duration,
};

use crate::program::ProgramRunner;
use wincast_protocol::{config::HostConfig, frame::read_message, handshake::reject_busy_client};

use super::{
    capture::{CaptureStarter, StdCaptureStarter, WindowLocator, WindowsWindowLocator},
    session::handle_control_client,
};

const SESSION_RECLAIM_GRACE: Duration = Duration::from_millis(250);

pub(crate) fn run_control_listener(
    listener: TcpListener,
    config: &HostConfig,
    runner: &mut (impl ProgramRunner + Send),
) -> Result<SocketAddr, String> {
    let mut locator = WindowsWindowLocator;
    let mut capture = StdCaptureStarter;
    run_control_listener_with_runtime(listener, config, runner, &mut locator, &mut capture)
}

pub(super) fn run_control_listener_with_runtime(
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

pub(super) fn run_control_listener_accept_loop<'scope, R, L, C>(
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

#[cfg(test)]
pub(super) fn run_control_listener_once_with_runtime(
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
pub(super) fn run_control_listener_once_with_runtime_and_session_gate(
    listener: TcpListener,
    config: &HostConfig,
    runner: &mut impl ProgramRunner,
    locator: &mut impl WindowLocator,
    capture: &mut impl CaptureStarter,
    session_gate: &mut impl super::session::SessionGate,
) -> Result<SocketAddr, String> {
    let local_addr = listener
        .local_addr()
        .map_err(|error| format!("读取宿主端监听地址失败: {error}"))?;
    let (mut stream, _peer_addr) = listener
        .accept()
        .map_err(|error| format!("接受客户端连接失败: {error}"))?;
    super::session::handle_control_client_with_session_gate(
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
pub(super) fn run_control_listener_n_with_runtime(
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
