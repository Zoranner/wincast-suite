use std::{
    error::Error,
    fmt,
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
};

use wincast_protocol::ipc::{
    AgentToService, IpcFrameError, ServiceToAgent, read_agent_to_service, read_service_to_agent,
    write_agent_to_service, write_service_to_agent,
};

#[derive(Debug)]
pub struct ServiceIpcEndpoint<T> {
    transport: T,
}

impl<T> ServiceIpcEndpoint<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    pub fn into_inner(self) -> T {
        self.transport
    }
}

impl ServiceIpcEndpoint<TcpStream> {
    pub fn connect_loopback(
        endpoint: SocketAddr,
    ) -> Result<ServiceIpcEndpoint<TcpStream>, std::io::Error> {
        TcpStream::connect(endpoint).map(Self::new)
    }
}

#[derive(Debug)]
pub struct ServiceIpcLoopbackListener {
    listener: TcpListener,
}

impl ServiceIpcLoopbackListener {
    pub fn bind_localhost_ephemeral() -> Result<Self, std::io::Error> {
        Self::bind_loopback(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
    }

    pub fn bind_loopback(addr: SocketAddr) -> Result<Self, std::io::Error> {
        if !addr.ip().is_loopback() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Service IPC loopback transport 只能绑定本机 loopback 地址",
            ));
        }

        let listener = TcpListener::bind(addr)?;
        Ok(Self { listener })
    }

    pub fn local_addr(&self) -> Result<SocketAddr, std::io::Error> {
        self.listener.local_addr()
    }

    pub fn accept(&self) -> Result<ServiceIpcEndpoint<TcpStream>, std::io::Error> {
        let (stream, _) = self.listener.accept()?;
        Ok(ServiceIpcEndpoint::new(stream))
    }
}

impl<T: Write> ServiceIpcEndpoint<T> {
    pub fn send_service_message(
        &mut self,
        message: &ServiceToAgent,
    ) -> Result<(), ServiceIpcError> {
        write_service_to_agent(&mut self.transport, message)
            .map_err(|source| ServiceIpcError::frame("发送 ServiceToAgent 消息失败", source))
    }

    pub fn send_agent_message(&mut self, message: &AgentToService) -> Result<(), ServiceIpcError> {
        write_agent_to_service(&mut self.transport, message)
            .map_err(|source| ServiceIpcError::frame("发送 AgentToService 消息失败", source))
    }
}

impl<T: Read> ServiceIpcEndpoint<T> {
    pub fn read_service_message(&mut self) -> Result<ServiceToAgent, ServiceIpcError> {
        read_service_to_agent(&mut self.transport)
            .map_err(|source| ServiceIpcError::frame("读取 ServiceToAgent 消息失败", source))
    }

    pub fn read_agent_message(&mut self) -> Result<AgentToService, ServiceIpcError> {
        read_agent_to_service(&mut self.transport)
            .map_err(|source| ServiceIpcError::frame("读取 AgentToService 消息失败", source))
    }
}

#[derive(Debug)]
pub struct ServiceIpcError {
    action: &'static str,
    source: IpcFrameError,
}

impl ServiceIpcError {
    fn frame(action: &'static str, source: IpcFrameError) -> Self {
        Self { action, source }
    }
}

impl fmt::Display for ServiceIpcError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Service IPC {}: {}", self.action, self.source)
    }
}

impl Error for ServiceIpcError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{error::Error, io::Cursor, thread};

    use wincast_protocol::ipc::{
        AgentStatus, AgentToService, IpcFrameError, ServiceToAgent, SessionEndReason,
    };

    #[test]
    fn service_sends_command_and_agent_reads_it() {
        let mut service = ServiceIpcEndpoint::new(Cursor::new(Vec::new()));
        let command = ServiceToAgent::StartSession { session_id: 42 };

        service
            .send_service_message(&command)
            .expect("service command should write");

        let bytes = service.into_inner().into_inner();
        let mut agent = ServiceIpcEndpoint::new(Cursor::new(bytes));
        let received = agent
            .read_service_message()
            .expect("agent should read service command");

        assert_eq!(received, command);
    }

    #[test]
    fn agent_sends_status_and_service_reads_it() {
        let mut agent = ServiceIpcEndpoint::new(Cursor::new(Vec::new()));
        let status = AgentToService::StatusChanged {
            status: AgentStatus::Ready,
        };

        agent
            .send_agent_message(&status)
            .expect("agent status should write");

        let bytes = agent.into_inner().into_inner();
        let mut service = ServiceIpcEndpoint::new(Cursor::new(bytes));
        let received = service
            .read_agent_message()
            .expect("service should read agent status");

        assert_eq!(received, status);
    }

    #[test]
    fn loopback_transport_round_trips_service_and_agent_messages() {
        let listener = ServiceIpcLoopbackListener::bind_localhost_ephemeral()
            .expect("loopback listener should bind");
        let endpoint = listener
            .local_addr()
            .expect("loopback listener should expose local address");
        assert!(endpoint.ip().is_loopback());

        let service_thread = thread::spawn(move || {
            let mut service = listener
                .accept()
                .expect("service should accept agent connection");
            let command = ServiceToAgent::QueryStatus;

            service
                .send_service_message(&command)
                .expect("service command should write to loopback transport");

            let status = service
                .read_agent_message()
                .expect("service should read agent status from loopback transport");

            assert_eq!(
                status,
                AgentToService::StatusChanged {
                    status: AgentStatus::Ready
                }
            );
        });

        let mut agent = ServiceIpcEndpoint::connect_loopback(endpoint)
            .expect("agent should connect to loopback listener");

        let received = agent
            .read_service_message()
            .expect("agent should read service command from loopback transport");
        assert_eq!(received, ServiceToAgent::QueryStatus);

        agent
            .send_agent_message(&AgentToService::StatusChanged {
                status: AgentStatus::Ready,
            })
            .expect("agent status should write to loopback transport");

        service_thread
            .join()
            .expect("service loopback round-trip should complete");
    }

    #[test]
    fn loopback_listener_rejects_non_loopback_bind_address() {
        let error =
            ServiceIpcLoopbackListener::bind_loopback(SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)))
                .expect_err("wildcard address should not be accepted for loopback IPC");

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(
            error
                .to_string()
                .contains("Service IPC loopback transport 只能绑定本机 loopback 地址")
        );
    }

    #[test]
    fn reads_consecutive_service_to_agent_messages() {
        let mut service = ServiceIpcEndpoint::new(Cursor::new(Vec::new()));
        let first = ServiceToAgent::QueryStatus;
        let second = ServiceToAgent::StopSession {
            session_id: 42,
            reason: SessionEndReason::ServiceRequested,
        };

        service
            .send_service_message(&first)
            .expect("first command should write");
        service
            .send_service_message(&second)
            .expect("second command should write");

        let bytes = service.into_inner().into_inner();
        let mut agent = ServiceIpcEndpoint::new(Cursor::new(bytes));

        assert_eq!(agent.read_service_message().unwrap(), first);
        assert_eq!(agent.read_service_message().unwrap(), second);
    }

    #[test]
    fn truncated_message_returns_clear_chinese_error_with_frame_source() {
        let mut frame = Vec::new();
        frame.extend_from_slice(&8_u32.to_be_bytes());
        frame.extend_from_slice(b"{}");
        let mut service = ServiceIpcEndpoint::new(Cursor::new(frame));

        let error = service
            .read_agent_message()
            .expect_err("truncated message should fail");

        assert!(error.to_string().contains("Service IPC"));
        assert!(error.to_string().contains("读取 AgentToService 消息失败"));
        assert!(error.to_string().contains("IPC 消息载荷不完整"));
        assert!(matches!(
            error
                .source()
                .and_then(|source| source.downcast_ref::<IpcFrameError>()),
            Some(IpcFrameError::IncompletePayload {
                expected: 8,
                actual: 2
            })
        ));
    }
}
