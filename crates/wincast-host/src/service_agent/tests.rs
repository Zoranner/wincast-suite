use std::{error::Error, io::Cursor, net::TcpStream, thread, time::Duration};

use wincast_protocol::ipc::{
    AgentErrorReason, AgentStatus, AgentToService, ServiceToAgent, SessionEndReason,
};

use crate::{
    service_agent::ServiceAgentCoordinator,
    service_ipc::{ServiceIpcEndpoint, ServiceIpcLoopbackListener},
};

#[test]
fn query_status_sends_command_and_returns_agent_status() {
    let mut coordinator = ServiceAgentCoordinator::new(ServiceIpcEndpoint::new(
        DuplexTransport::with_response(&AgentToService::StatusChanged {
            status: AgentStatus::Ready,
        }),
    ));

    let status = coordinator
        .query_status()
        .expect("status query should return agent status");

    assert_eq!(status, AgentStatus::Ready);
    let sent_bytes = coordinator.into_endpoint().into_inner().written;
    let mut agent_reader = ServiceIpcEndpoint::new(Cursor::new(sent_bytes));
    assert_eq!(
        agent_reader.read_service_message().unwrap(),
        ServiceToAgent::QueryStatus
    );
}

#[test]
fn query_status_rejects_unexpected_agent_response() {
    let mut coordinator = ServiceAgentCoordinator::new(ServiceIpcEndpoint::new(
        DuplexTransport::with_response(&AgentToService::SessionStarted { session_id: 7 }),
    ));

    let error = coordinator
        .query_status()
        .expect_err("non-status response should fail");

    assert!(error.to_string().contains("Agent 状态查询响应类型错误"));
    assert!(error.to_string().contains("SessionStarted"));
}

#[test]
fn query_status_returns_agent_error_response_with_reason_and_message() {
    let mut coordinator = ServiceAgentCoordinator::new(ServiceIpcEndpoint::new(
        DuplexTransport::with_response(&AgentToService::Error {
            reason: AgentErrorReason::AgentFailed,
            message: "Agent 状态采集失败".to_owned(),
        }),
    ));

    let error = coordinator
        .query_status()
        .expect_err("agent error response should fail status query");

    assert!(error.to_string().contains("Agent 状态查询失败"));
    assert!(error.to_string().contains("AgentFailed"));
    assert!(error.to_string().contains("Agent 状态采集失败"));
}

#[test]
fn query_status_wraps_ipc_write_error_with_context() {
    let mut coordinator =
        ServiceAgentCoordinator::new(ServiceIpcEndpoint::new(FailingTransport::fail_write()));

    let error = coordinator
        .query_status()
        .expect_err("write failure should fail status query");

    assert!(error.to_string().contains("发送 Agent 状态查询失败"));
    assert!(error.source().is_some());
}

#[test]
fn query_status_wraps_ipc_read_error_with_context() {
    let mut coordinator =
        ServiceAgentCoordinator::new(ServiceIpcEndpoint::new(Cursor::new(Vec::new())));

    let error = coordinator
        .query_status()
        .expect_err("empty response should fail status query");

    assert!(error.to_string().contains("读取 Agent 状态响应失败"));
    assert!(error.source().is_some());
}

#[test]
fn start_session_sends_command_and_returns_confirmed_session_id() {
    let mut coordinator = ServiceAgentCoordinator::new(ServiceIpcEndpoint::new(
        DuplexTransport::with_response(&AgentToService::SessionStarted { session_id: 42 }),
    ));

    let session_id = coordinator
        .start_session(42)
        .expect("session start should return confirmed session id");

    assert_eq!(session_id, 42);
    let sent_bytes = coordinator.into_endpoint().into_inner().written;
    let mut agent_reader = ServiceIpcEndpoint::new(Cursor::new(sent_bytes));
    assert_eq!(
        agent_reader.read_service_message().unwrap(),
        ServiceToAgent::StartSession { session_id: 42 }
    );
}

#[test]
fn stop_session_sends_command_and_returns_confirmed_reason() {
    let mut coordinator = ServiceAgentCoordinator::new(ServiceIpcEndpoint::new(
        DuplexTransport::with_response(&AgentToService::SessionEnded {
            session_id: 42,
            reason: SessionEndReason::ServiceRequested,
        }),
    ));

    let reason = coordinator
        .stop_session(42, SessionEndReason::ServiceRequested)
        .expect("session stop should return confirmed end reason");

    assert_eq!(reason, SessionEndReason::ServiceRequested);
    let sent_bytes = coordinator.into_endpoint().into_inner().written;
    let mut agent_reader = ServiceIpcEndpoint::new(Cursor::new(sent_bytes));
    assert_eq!(
        agent_reader.read_service_message().unwrap(),
        ServiceToAgent::StopSession {
            session_id: 42,
            reason: SessionEndReason::ServiceRequested
        }
    );
}

#[test]
fn start_session_rejects_unexpected_agent_response() {
    let mut coordinator = ServiceAgentCoordinator::new(ServiceIpcEndpoint::new(
        DuplexTransport::with_response(&AgentToService::StatusChanged {
            status: AgentStatus::Ready,
        }),
    ));

    let error = coordinator
        .start_session(42)
        .expect_err("non-session-start response should fail");

    assert!(error.to_string().contains("Agent 会话启动响应类型错误"));
    assert!(error.to_string().contains("StatusChanged"));
}

#[test]
fn stop_session_rejects_unexpected_agent_response() {
    let mut coordinator = ServiceAgentCoordinator::new(ServiceIpcEndpoint::new(
        DuplexTransport::with_response(&AgentToService::StatusChanged {
            status: AgentStatus::Ready,
        }),
    ));

    let error = coordinator
        .stop_session(42, SessionEndReason::ServiceRequested)
        .expect_err("non-session-ended response should fail");

    assert!(error.to_string().contains("Agent 会话停止响应类型错误"));
    assert!(error.to_string().contains("StatusChanged"));
}

#[test]
fn start_session_rejects_mismatched_session_id() {
    let mut coordinator = ServiceAgentCoordinator::new(ServiceIpcEndpoint::new(
        DuplexTransport::with_response(&AgentToService::SessionStarted { session_id: 7 }),
    ));

    let error = coordinator
        .start_session(42)
        .expect_err("mismatched session id should fail");

    assert!(
        error
            .to_string()
            .contains("Agent 会话启动确认 session_id 不一致")
    );
    assert!(error.to_string().contains("期望 42"));
    assert!(error.to_string().contains("实际 7"));
}

#[test]
fn stop_session_rejects_mismatched_session_id() {
    let mut coordinator = ServiceAgentCoordinator::new(ServiceIpcEndpoint::new(
        DuplexTransport::with_response(&AgentToService::SessionEnded {
            session_id: 7,
            reason: SessionEndReason::ServiceRequested,
        }),
    ));

    let error = coordinator
        .stop_session(42, SessionEndReason::ServiceRequested)
        .expect_err("mismatched session id should fail");

    assert!(
        error
            .to_string()
            .contains("Agent 会话停止确认 session_id 不一致")
    );
    assert!(error.to_string().contains("期望 42"));
    assert!(error.to_string().contains("实际 7"));
}

#[test]
fn start_session_returns_agent_error_response_with_reason_and_message() {
    let mut coordinator = ServiceAgentCoordinator::new(ServiceIpcEndpoint::new(
        DuplexTransport::with_response(&AgentToService::Error {
            reason: AgentErrorReason::Locked,
            message: "桌面已锁定".to_owned(),
        }),
    ));

    let error = coordinator
        .start_session(42)
        .expect_err("agent error response should fail");

    assert!(error.to_string().contains("Agent 会话启动失败"));
    assert!(error.to_string().contains("Locked"));
    assert!(error.to_string().contains("桌面已锁定"));
}

#[test]
fn stop_session_returns_agent_error_response_with_reason_and_message() {
    let mut coordinator = ServiceAgentCoordinator::new(ServiceIpcEndpoint::new(
        DuplexTransport::with_response(&AgentToService::Error {
            reason: AgentErrorReason::AgentFailed,
            message: "Agent 内部失败".to_owned(),
        }),
    ));

    let error = coordinator
        .stop_session(42, SessionEndReason::ServiceRequested)
        .expect_err("agent error response should fail");

    assert!(error.to_string().contains("Agent 会话停止失败"));
    assert!(error.to_string().contains("AgentFailed"));
    assert!(error.to_string().contains("Agent 内部失败"));
}

#[test]
fn start_session_wraps_ipc_write_error_with_context() {
    let mut coordinator =
        ServiceAgentCoordinator::new(ServiceIpcEndpoint::new(FailingTransport::fail_write()));

    let error = coordinator
        .start_session(42)
        .expect_err("write failure should fail session start");

    assert!(error.to_string().contains("发送 Agent 会话启动命令失败"));
    assert!(error.source().is_some());
}

#[test]
fn stop_session_wraps_ipc_write_error_with_context() {
    let mut coordinator =
        ServiceAgentCoordinator::new(ServiceIpcEndpoint::new(FailingTransport::fail_write()));

    let error = coordinator
        .stop_session(42, SessionEndReason::ServiceRequested)
        .expect_err("write failure should fail session stop");

    assert!(error.to_string().contains("发送 Agent 会话停止命令失败"));
    assert!(error.source().is_some());
}

#[test]
fn start_session_wraps_ipc_read_error_with_context() {
    let mut coordinator =
        ServiceAgentCoordinator::new(ServiceIpcEndpoint::new(Cursor::new(Vec::new())));

    let error = coordinator
        .start_session(42)
        .expect_err("empty response should fail session start");

    assert!(error.to_string().contains("读取 Agent 会话启动响应失败"));
    assert!(error.source().is_some());
}

#[test]
fn stop_session_wraps_ipc_read_error_with_context() {
    let mut coordinator =
        ServiceAgentCoordinator::new(ServiceIpcEndpoint::new(Cursor::new(Vec::new())));

    let error = coordinator
        .stop_session(42, SessionEndReason::ServiceRequested)
        .expect_err("empty response should fail session stop");

    assert!(error.to_string().contains("读取 Agent 会话停止响应失败"));
    assert!(error.source().is_some());
}

#[test]
fn query_status_round_trips_over_loopback_endpoint() {
    let listener = ServiceIpcLoopbackListener::bind_localhost_ephemeral()
        .expect("loopback listener should bind");
    let endpoint = listener
        .local_addr()
        .expect("loopback listener should expose address");

    let agent_thread = thread::spawn(move || {
        let stream = TcpStream::connect_timeout(&endpoint, Duration::from_secs(2))
            .expect("agent should connect to service loopback endpoint");
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("agent read timeout should set");
        let mut agent = ServiceIpcEndpoint::new(stream);
        let command = agent
            .read_service_message()
            .expect("agent should read status query");
        assert_eq!(command, ServiceToAgent::QueryStatus);
        agent
            .send_agent_message(&AgentToService::StatusChanged {
                status: AgentStatus::Ready,
            })
            .expect("agent should send status response");
    });

    let service_stream = listener
        .accept()
        .expect("service should accept agent")
        .into_inner();
    service_stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("service read timeout should set");
    let service_endpoint = ServiceIpcEndpoint::new(service_stream);

    let mut coordinator = ServiceAgentCoordinator::new(service_endpoint);
    let status = coordinator
        .query_status()
        .expect("status query should round-trip over TCP loopback");

    agent_thread
        .join()
        .expect("agent loopback response should complete");

    assert_eq!(status, AgentStatus::Ready);
}

#[test]
fn query_status_round_trips_over_existing_connect_endpoint() {
    let listener = ServiceIpcLoopbackListener::bind_localhost_ephemeral()
        .expect("loopback listener should bind");
    let endpoint = listener
        .local_addr()
        .expect("loopback listener should expose address");

    let service_thread = thread::spawn(move || {
        let mut service = listener.accept().expect("service should accept agent");
        let command = service
            .read_service_message()
            .expect("service should read status query");
        assert_eq!(command, ServiceToAgent::QueryStatus);
        service
            .send_agent_message(&AgentToService::StatusChanged {
                status: AgentStatus::Ready,
            })
            .expect("service should send status response");
    });

    let stream = TcpStream::connect_timeout(&endpoint, Duration::from_secs(2))
        .expect("agent should connect to service loopback endpoint");
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("client read timeout should set");
    let mut coordinator = ServiceAgentCoordinator::new(ServiceIpcEndpoint::new(stream));

    let status = coordinator
        .query_status()
        .expect("status query should round-trip over TCP loopback");

    service_thread
        .join()
        .expect("service loopback response should complete");

    assert_eq!(status, AgentStatus::Ready);
}

struct FailingTransport {
    fail_write: bool,
}

struct DuplexTransport {
    read: Cursor<Vec<u8>>,
    written: Vec<u8>,
}

impl DuplexTransport {
    fn with_response(response: &AgentToService) -> Self {
        let mut agent = ServiceIpcEndpoint::new(Cursor::new(Vec::new()));
        agent
            .send_agent_message(response)
            .expect("agent response should encode");

        Self {
            read: Cursor::new(agent.into_inner().into_inner()),
            written: Vec::new(),
        }
    }
}

impl std::io::Read for DuplexTransport {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.read.read(buf)
    }
}

impl std::io::Write for DuplexTransport {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.written.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl FailingTransport {
    fn fail_write() -> Self {
        Self { fail_write: true }
    }
}

impl std::io::Read for FailingTransport {
    fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
        Ok(0)
    }
}

impl std::io::Write for FailingTransport {
    fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
        if self.fail_write {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "写入失败",
            ))
        } else {
            Ok(0)
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
