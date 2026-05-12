use std::{
    error::Error,
    fmt,
    io::{Read, Write},
};

use wincast_protocol::ipc::{
    AgentErrorReason, AgentStatus, AgentToService, ServiceToAgent, SessionEndReason,
};

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

    pub fn start_session(&mut self, session_id: u64) -> Result<u64, ServiceAgentError> {
        self.endpoint
            .send_service_message(&ServiceToAgent::StartSession { session_id })
            .map_err(ServiceAgentError::start_write)?;

        match self
            .endpoint
            .read_agent_message()
            .map_err(ServiceAgentError::start_read)?
        {
            AgentToService::SessionStarted {
                session_id: confirmed_id,
            } if confirmed_id == session_id => Ok(confirmed_id),
            AgentToService::SessionStarted { session_id: actual } => Err(
                ServiceAgentError::mismatched_start_session_id(session_id, actual),
            ),
            AgentToService::Error { reason, message } => {
                Err(ServiceAgentError::start_agent_error(reason, message))
            }
            response => Err(ServiceAgentError::unexpected_start_response(response)),
        }
    }

    pub fn stop_session(
        &mut self,
        session_id: u64,
        reason: SessionEndReason,
    ) -> Result<SessionEndReason, ServiceAgentError> {
        self.endpoint
            .send_service_message(&ServiceToAgent::StopSession { session_id, reason })
            .map_err(ServiceAgentError::stop_write)?;

        match self
            .endpoint
            .read_agent_message()
            .map_err(ServiceAgentError::stop_read)?
        {
            AgentToService::SessionEnded {
                session_id: confirmed_id,
                reason,
            } if confirmed_id == session_id => Ok(reason),
            AgentToService::SessionEnded {
                session_id: actual, ..
            } => Err(ServiceAgentError::mismatched_stop_session_id(
                session_id, actual,
            )),
            AgentToService::Error { reason, message } => {
                Err(ServiceAgentError::stop_agent_error(reason, message))
            }
            response => Err(ServiceAgentError::unexpected_stop_response(response)),
        }
    }
}

#[derive(Debug)]
pub enum ServiceAgentError {
    QueryWrite {
        source: ServiceIpcError,
    },
    QueryRead {
        source: ServiceIpcError,
    },
    UnexpectedStatusResponse {
        response: AgentToService,
    },
    StartWrite {
        source: ServiceIpcError,
    },
    StartRead {
        source: ServiceIpcError,
    },
    UnexpectedStartResponse {
        response: AgentToService,
    },
    MismatchedStartSessionId {
        expected: u64,
        actual: u64,
    },
    StartAgentError {
        reason: AgentErrorReason,
        message: String,
    },
    StopWrite {
        source: ServiceIpcError,
    },
    StopRead {
        source: ServiceIpcError,
    },
    UnexpectedStopResponse {
        response: AgentToService,
    },
    MismatchedStopSessionId {
        expected: u64,
        actual: u64,
    },
    StopAgentError {
        reason: AgentErrorReason,
        message: String,
    },
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

    fn start_write(source: ServiceIpcError) -> Self {
        Self::StartWrite { source }
    }

    fn start_read(source: ServiceIpcError) -> Self {
        Self::StartRead { source }
    }

    fn unexpected_start_response(response: AgentToService) -> Self {
        Self::UnexpectedStartResponse { response }
    }

    fn mismatched_start_session_id(expected: u64, actual: u64) -> Self {
        Self::MismatchedStartSessionId { expected, actual }
    }

    fn start_agent_error(reason: AgentErrorReason, message: String) -> Self {
        Self::StartAgentError { reason, message }
    }

    fn stop_write(source: ServiceIpcError) -> Self {
        Self::StopWrite { source }
    }

    fn stop_read(source: ServiceIpcError) -> Self {
        Self::StopRead { source }
    }

    fn unexpected_stop_response(response: AgentToService) -> Self {
        Self::UnexpectedStopResponse { response }
    }

    fn mismatched_stop_session_id(expected: u64, actual: u64) -> Self {
        Self::MismatchedStopSessionId { expected, actual }
    }

    fn stop_agent_error(reason: AgentErrorReason, message: String) -> Self {
        Self::StopAgentError { reason, message }
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
            Self::StartWrite { source } => {
                write!(formatter, "发送 Agent 会话启动命令失败：{source}")
            }
            Self::StartRead { source } => {
                write!(formatter, "读取 Agent 会话启动响应失败：{source}")
            }
            Self::UnexpectedStartResponse { response } => write!(
                formatter,
                "Agent 会话启动响应类型错误：期望 SessionStarted，实际收到 {response:?}"
            ),
            Self::MismatchedStartSessionId { expected, actual } => write!(
                formatter,
                "Agent 会话启动确认 session_id 不一致：期望 {expected}，实际 {actual}"
            ),
            Self::StartAgentError { reason, message } => {
                write!(formatter, "Agent 会话启动失败：{reason:?}：{message}")
            }
            Self::StopWrite { source } => {
                write!(formatter, "发送 Agent 会话停止命令失败：{source}")
            }
            Self::StopRead { source } => {
                write!(formatter, "读取 Agent 会话停止响应失败：{source}")
            }
            Self::UnexpectedStopResponse { response } => write!(
                formatter,
                "Agent 会话停止响应类型错误：期望 SessionEnded，实际收到 {response:?}"
            ),
            Self::MismatchedStopSessionId { expected, actual } => write!(
                formatter,
                "Agent 会话停止确认 session_id 不一致：期望 {expected}，实际 {actual}"
            ),
            Self::StopAgentError { reason, message } => {
                write!(formatter, "Agent 会话停止失败：{reason:?}：{message}")
            }
        }
    }
}

impl Error for ServiceAgentError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::QueryWrite { source }
            | Self::QueryRead { source }
            | Self::StartWrite { source }
            | Self::StartRead { source }
            | Self::StopWrite { source }
            | Self::StopRead { source } => Some(source),
            Self::UnexpectedStatusResponse { .. }
            | Self::UnexpectedStartResponse { .. }
            | Self::MismatchedStartSessionId { .. }
            | Self::StartAgentError { .. }
            | Self::UnexpectedStopResponse { .. }
            | Self::MismatchedStopSessionId { .. }
            | Self::StopAgentError { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests;
