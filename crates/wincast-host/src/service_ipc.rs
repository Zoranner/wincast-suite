use std::{
    error::Error,
    fmt,
    io::{Read, Write},
};

use wincast_protocol::ipc::{
    AgentToService, IpcFrameError, ServiceToAgent, read_agent_to_service, read_service_to_agent,
    write_agent_to_service, write_service_to_agent,
};

#[derive(Debug)]
pub(crate) struct ServiceIpcEndpoint<T> {
    transport: T,
}

impl<T> ServiceIpcEndpoint<T> {
    pub(crate) fn new(transport: T) -> Self {
        Self { transport }
    }

    pub(crate) fn into_inner(self) -> T {
        self.transport
    }
}

impl<T: Write> ServiceIpcEndpoint<T> {
    pub(crate) fn send_service_message(
        &mut self,
        message: &ServiceToAgent,
    ) -> Result<(), ServiceIpcError> {
        write_service_to_agent(&mut self.transport, message)
            .map_err(|source| ServiceIpcError::frame("发送 ServiceToAgent 消息失败", source))
    }

    pub(crate) fn send_agent_message(
        &mut self,
        message: &AgentToService,
    ) -> Result<(), ServiceIpcError> {
        write_agent_to_service(&mut self.transport, message)
            .map_err(|source| ServiceIpcError::frame("发送 AgentToService 消息失败", source))
    }
}

impl<T: Read> ServiceIpcEndpoint<T> {
    pub(crate) fn read_service_message(&mut self) -> Result<ServiceToAgent, ServiceIpcError> {
        read_service_to_agent(&mut self.transport)
            .map_err(|source| ServiceIpcError::frame("读取 ServiceToAgent 消息失败", source))
    }

    pub(crate) fn read_agent_message(&mut self) -> Result<AgentToService, ServiceIpcError> {
        read_agent_to_service(&mut self.transport)
            .map_err(|source| ServiceIpcError::frame("读取 AgentToService 消息失败", source))
    }
}

#[derive(Debug)]
pub(crate) struct ServiceIpcError {
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
    use std::{error::Error, io::Cursor};

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
