use std::{
    net::{SocketAddr, TcpListener, TcpStream},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::mpsc,
    thread,
    time::Duration,
};

use crate::program::ProgramRunner;
use wincast_protocol::{config::HostConfig, frame::read_message, handshake::reject_busy_client};

use super::{
    capture::{CaptureStarter, StdCaptureStarter},
    session::handle_control_client,
};

const SESSION_RECLAIM_GRACE: Duration = Duration::from_millis(250);

pub(crate) fn run_control_listener(
    listener: TcpListener,
    config: &HostConfig,
    runner: &mut (impl ProgramRunner + Send),
) -> Result<SocketAddr, String> {
    let mut capture = StdCaptureStarter;
    run_control_listener_with_runtime(listener, config, runner, &mut capture)
}

pub(super) fn run_control_listener_with_runtime(
    listener: TcpListener,
    config: &HostConfig,
    runner: &mut (impl ProgramRunner + Send),
    capture: &mut (impl CaptureStarter + Send),
) -> Result<SocketAddr, String> {
    let local_addr = listener
        .local_addr()
        .map_err(|error| format!("读取宿主端监听地址失败: {error}"))?;
    thread::scope(|scope| {
        run_control_listener_accept_loop(listener, config, runner, capture, scope, None)
    })?;
    Ok(local_addr)
}

pub(super) fn run_control_listener_accept_loop<'scope, R, C>(
    listener: TcpListener,
    config: &'scope HostConfig,
    runner: &'scope mut R,
    capture: &'scope mut C,
    scope: &'scope thread::Scope<'scope, '_>,
    max_connections: Option<usize>,
) -> Result<(), String>
where
    R: ProgramRunner + Send + 'scope,
    C: CaptureStarter + Send + 'scope,
{
    let (session_finished_sender, session_finished_receiver) = mpsc::channel();
    let mut state = ListenerSessionState::Idle { runner, capture };
    let mut accepted_connections = 0usize;

    loop {
        if let Some(max_connections) = max_connections
            && accepted_connections >= max_connections
        {
            join_finished_session(state)?;
            return Ok(());
        }

        let (stream, peer_addr) = listener
            .accept()
            .map_err(|error| format!("接受客户端连接失败: {error}"))?;
        configure_control_stream(&stream)?;
        accepted_connections += 1;
        state = join_finished_session_if_reported(state, &session_finished_receiver)?;

        state = match state {
            ListenerSessionState::Idle { runner, capture } => {
                let finished_sender = session_finished_sender.clone();
                ListenerSessionState::Busy(spawn_control_session(
                    scope,
                    stream,
                    peer_addr,
                    SessionRuntime {
                        config,
                        runner,
                        capture,
                    },
                    finished_sender,
                ))
            }
            ListenerSessionState::Busy(session) => {
                let state = wait_for_finished_session(
                    ListenerSessionState::Busy(session),
                    &session_finished_receiver,
                    SESSION_RECLAIM_GRACE,
                )?;
                match state {
                    ListenerSessionState::Idle { runner, capture } => {
                        let finished_sender = session_finished_sender.clone();
                        ListenerSessionState::Busy(spawn_control_session(
                            scope,
                            stream,
                            peer_addr,
                            SessionRuntime {
                                config,
                                runner,
                                capture,
                            },
                            finished_sender,
                        ))
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

struct SessionRuntime<'scope, R, C>
where
    R: ProgramRunner + Send + 'scope,
    C: CaptureStarter + Send + 'scope,
{
    config: &'scope HostConfig,
    runner: &'scope mut R,
    capture: &'scope mut C,
}

fn spawn_control_session<'scope, R, C>(
    scope: &'scope thread::Scope<'scope, '_>,
    stream: TcpStream,
    peer_addr: SocketAddr,
    runtime: SessionRuntime<'scope, R, C>,
    finished_sender: mpsc::Sender<SessionFinished>,
) -> ScopedSessionHandle<'scope, R, C>
where
    R: ProgramRunner + Send + 'scope,
    C: CaptureStarter + Send + 'scope,
{
    scope.spawn(move || {
        let mut stream = stream;
        let SessionRuntime {
            config,
            runner,
            capture,
        } = runtime;
        let result = catch_unwind(AssertUnwindSafe(|| {
            handle_control_client(&mut stream, config, runner, capture)
        }))
        .unwrap_or_else(|_| Err("客户端会话线程异常结束".to_owned()));
        let _ = finished_sender.send(SessionFinished);
        (peer_addr, result, runner, capture)
    })
}

enum ListenerSessionState<'scope, R, C>
where
    R: ProgramRunner + Send + 'scope,
    C: CaptureStarter + Send + 'scope,
{
    Idle {
        runner: &'scope mut R,
        capture: &'scope mut C,
    },
    Busy(ScopedSessionHandle<'scope, R, C>),
}

fn join_finished_session_if_reported<'scope, R, C>(
    state: ListenerSessionState<'scope, R, C>,
    session_finished: &mpsc::Receiver<SessionFinished>,
) -> Result<ListenerSessionState<'scope, R, C>, String>
where
    R: ProgramRunner + Send + 'scope,
    C: CaptureStarter + Send + 'scope,
{
    if session_finished.try_recv().is_ok() {
        join_reported_finished_session(state)
    } else {
        Ok(state)
    }
}

fn wait_for_finished_session<'scope, R, C>(
    state: ListenerSessionState<'scope, R, C>,
    session_finished: &mpsc::Receiver<SessionFinished>,
    timeout: Duration,
) -> Result<ListenerSessionState<'scope, R, C>, String>
where
    R: ProgramRunner + Send + 'scope,
    C: CaptureStarter + Send + 'scope,
{
    match state {
        ListenerSessionState::Busy(session) => match session_finished.recv_timeout(timeout) {
            Ok(SessionFinished) => {
                join_reported_finished_session(ListenerSessionState::Busy(session))
            }
            Err(_) => Ok(ListenerSessionState::Busy(session)),
        },
        state => Ok(state),
    }
}

fn join_reported_finished_session<'scope, R, C>(
    state: ListenerSessionState<'scope, R, C>,
) -> Result<ListenerSessionState<'scope, R, C>, String>
where
    R: ProgramRunner + Send + 'scope,
    C: CaptureStarter + Send + 'scope,
{
    match state {
        ListenerSessionState::Idle { runner, capture } => {
            Ok(ListenerSessionState::Idle { runner, capture })
        }
        ListenerSessionState::Busy(session) => {
            let (_peer_addr, _result, runner, capture) = log_session_result(session.join())?;
            Ok(ListenerSessionState::Idle { runner, capture })
        }
    }
}

type ScopedSessionHandle<'scope, R, C> =
    thread::ScopedJoinHandle<'scope, SessionThreadResult<'scope, R, C>>;

type SessionThreadResult<'scope, R, C> =
    (SocketAddr, Result<(), String>, &'scope mut R, &'scope mut C);

fn join_finished_session<'scope, R, C>(
    state: ListenerSessionState<'scope, R, C>,
) -> Result<ListenerSessionState<'scope, R, C>, String>
where
    R: ProgramRunner + Send + 'scope,
    C: CaptureStarter + Send + 'scope,
{
    match state {
        ListenerSessionState::Idle { runner, capture } => {
            Ok(ListenerSessionState::Idle { runner, capture })
        }
        ListenerSessionState::Busy(session) if session.is_finished() => {
            let (_peer_addr, _result, runner, capture) = log_session_result(session.join())?;
            Ok(ListenerSessionState::Idle { runner, capture })
        }
        ListenerSessionState::Busy(session) => Ok(ListenerSessionState::Busy(session)),
    }
}

pub(super) fn log_session_result<'scope, R, C>(
    result: std::thread::Result<SessionThreadResult<'scope, R, C>>,
) -> Result<SessionThreadResult<'scope, R, C>, String> {
    match result {
        Ok((peer_addr, session_result, runner, capture)) => {
            if let Err(error) = &session_result {
                eprintln!("客户端 {peer_addr} 会话结束: {error}");
            } else {
                eprintln!("客户端 {peer_addr} 会话结束");
            }
            Ok((peer_addr, session_result, runner, capture))
        }
        Err(_) => {
            let error = "客户端会话线程异常结束且无法恢复会话资源".to_owned();
            eprintln!("{error}");
            Err(error)
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

pub(super) fn configure_control_stream(stream: &TcpStream) -> Result<(), String> {
    stream
        .set_nodelay(true)
        .map_err(|error| format!("配置宿主端 TCP 低延迟模式失败: {error}"))
}

#[cfg(test)]
pub(super) fn run_control_listener_once_with_runtime(
    listener: TcpListener,
    config: &HostConfig,
    runner: &mut impl ProgramRunner,
    capture: &mut impl CaptureStarter,
) -> Result<SocketAddr, String> {
    let local_addr = listener
        .local_addr()
        .map_err(|error| format!("读取宿主端监听地址失败: {error}"))?;
    let (mut stream, _peer_addr) = listener
        .accept()
        .map_err(|error| format!("接受客户端连接失败: {error}"))?;
    configure_control_stream(&stream)?;
    handle_control_client(&mut stream, config, runner, capture)?;
    Ok(local_addr)
}

#[cfg(test)]
pub(super) fn run_control_listener_once_with_runtime_and_session_gate(
    listener: TcpListener,
    config: &HostConfig,
    runner: &mut impl ProgramRunner,
    capture: &mut impl CaptureStarter,
    session_gate: &mut impl super::session::SessionGate,
) -> Result<SocketAddr, String> {
    let local_addr = listener
        .local_addr()
        .map_err(|error| format!("读取宿主端监听地址失败: {error}"))?;
    let (mut stream, _peer_addr) = listener
        .accept()
        .map_err(|error| format!("接受客户端连接失败: {error}"))?;
    configure_control_stream(&stream)?;
    super::session::handle_control_client_with_session_gate(
        &mut stream,
        config,
        runner,
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
    capture: &mut (impl CaptureStarter + Send),
    sessions: usize,
) -> Result<SocketAddr, String> {
    let local_addr = listener
        .local_addr()
        .map_err(|error| format!("读取宿主端监听地址失败: {error}"))?;
    thread::scope(|scope| {
        run_control_listener_accept_loop(listener, config, runner, capture, scope, Some(sessions))
    })?;
    Ok(local_addr)
}
