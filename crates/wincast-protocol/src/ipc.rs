use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentStatus {
    Starting,
    Ready,
    Busy,
    Locked,
    Stopped,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServiceToAgent {
    StartSession {
        session_id: u64,
    },
    StopSession {
        session_id: u64,
        reason: SessionEndReason,
    },
    Shutdown,
    QueryStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentToService {
    StatusChanged {
        status: AgentStatus,
    },
    SessionStarted {
        session_id: u64,
    },
    SessionEnded {
        session_id: u64,
        reason: SessionEndReason,
    },
    Error {
        reason: AgentErrorReason,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionEndReason {
    ServiceRequested,
    Shutdown,
    DesktopUnavailable,
    Locked,
    AgentFailed,
    SessionFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentErrorReason {
    DesktopUnavailable,
    Locked,
    AgentFailed,
    SessionFailed,
}
