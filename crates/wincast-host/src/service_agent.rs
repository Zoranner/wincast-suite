use std::{
    error::Error,
    fmt,
    io::{Read, Write},
};

use wincast_protocol::ipc::{AgentStatus, AgentToService, ServiceToAgent};

use crate::service_ipc::{ServiceIpcEndpoint, ServiceIpcError};

#[derive(Debug)]
pub struct ServiceAgentCoordinator<T> {
    endpoint: ServiceIpcEndpoint<T>,
}

impl<T> ServiceAgentCoordinator<T> {
    pub fn new(endpoint: ServiceIpcEndpoint<T>) -> Self {
        Self { endpoint }
    }

    #[cfg(test)]
    pub fn into_endpoint(self) -> ServiceIpcEndpoint<T> {
        self.endpoint
    }
}

impl<T: Read + Write> ServiceAgentCoordinator<T> {
    pub fn query_status(&mut self) -> Result<AgentStatus, ServiceAgentError> {
        self.endpoint
            .send_service_message(&ServiceToAgent::QueryStatus)
            .map_err(ServiceAgentError::query_write)?;

        match self
            .endpoint
            .read_agent_message()
            .map_err(ServiceAgentError::query_read)?
        {
            AgentToService::StatusChanged { status } => Ok(status),
            response => Err(ServiceAgentError::unexpected_status_response(response)),
        }
    }
}

#[derive(Debug)]
pub enum ServiceAgentError {
    QueryWrite { source: ServiceIpcError },
    QueryRead { source: ServiceIpcError },
    UnexpectedStatusResponse { response: AgentToService },
}

impl ServiceAgentError {
    fn query_write(source: ServiceIpcError) -> Self {
        Self::QueryWrite { source }
    }

    fn query_read(source: ServiceIpcError) -> Self {
        Self::QueryRead { source }
    }

    fn unexpected_status_response(response: AgentToService) -> Self {
        Self::UnexpectedStatusResponse { response }
    }
}

impl fmt::Display for ServiceAgentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::QueryWrite { source } => {
                write!(formatter, "发送 Agent 状态查询失败：{source}")
            }
            Self::QueryRead { source } => {
                write!(formatter, "读取 Agent 状态响应失败：{source}")
            }
            Self::UnexpectedStatusResponse { response } => write!(
                formatter,
                "Agent 状态查询响应类型错误：期望 StatusChanged，实际收到 {response:?}"
            ),
        }
    }
}

impl Error for ServiceAgentError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::QueryWrite { source } | Self::QueryRead { source } => Some(source),
            Self::UnexpectedStatusResponse { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{error::Error, io::Cursor, net::TcpStream, thread, time::Duration};

    use wincast_protocol::ipc::{AgentStatus, AgentToService, ServiceToAgent};

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
}
