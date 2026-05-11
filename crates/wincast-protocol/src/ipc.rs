use std::io::{Read, Write};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const MAX_IPC_FRAME_LEN: usize = 1024 * 1024;

#[derive(Debug, Error)]
pub enum IpcFrameError {
    #[error("IPC 消息 JSON 编解码失败: {0}")]
    Json(serde_json::Error),
    #[error("IPC 消息长度 {actual} 超过限制 {max}")]
    PayloadTooLarge { actual: usize, max: usize },
    #[error("IPC 消息载荷不完整: 期望 {expected} 字节，实际读取 {actual} 字节")]
    IncompletePayload { expected: usize, actual: usize },
    #[error("IPC 消息 IO 失败: {0}")]
    Io(std::io::Error),
}

impl PartialEq for IpcFrameError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Json(_), Self::Json(_)) | (Self::Io(_), Self::Io(_)) => true,
            (
                Self::PayloadTooLarge {
                    actual: left_actual,
                    max: left_max,
                },
                Self::PayloadTooLarge {
                    actual: right_actual,
                    max: right_max,
                },
            ) => left_actual == right_actual && left_max == right_max,
            (
                Self::IncompletePayload {
                    expected: left_expected,
                    actual: left_actual,
                },
                Self::IncompletePayload {
                    expected: right_expected,
                    actual: right_actual,
                },
            ) => left_expected == right_expected && left_actual == right_actual,
            _ => false,
        }
    }
}

impl Eq for IpcFrameError {}

impl From<std::io::Error> for IpcFrameError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

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

pub fn encode_service_to_agent_frame(message: &ServiceToAgent) -> Result<Vec<u8>, IpcFrameError> {
    encode_ipc_frame(message)
}

pub fn decode_service_to_agent_frame(frame: &[u8]) -> Result<ServiceToAgent, IpcFrameError> {
    decode_ipc_frame(frame)
}

pub fn write_service_to_agent(
    writer: &mut impl Write,
    message: &ServiceToAgent,
) -> Result<(), IpcFrameError> {
    write_ipc_frame(writer, message)
}

pub fn read_service_to_agent(reader: &mut impl Read) -> Result<ServiceToAgent, IpcFrameError> {
    read_ipc_frame(reader)
}

pub fn encode_agent_to_service_frame(message: &AgentToService) -> Result<Vec<u8>, IpcFrameError> {
    encode_ipc_frame(message)
}

pub fn decode_agent_to_service_frame(frame: &[u8]) -> Result<AgentToService, IpcFrameError> {
    decode_ipc_frame(frame)
}

pub fn write_agent_to_service(
    writer: &mut impl Write,
    message: &AgentToService,
) -> Result<(), IpcFrameError> {
    write_ipc_frame(writer, message)
}

pub fn read_agent_to_service(reader: &mut impl Read) -> Result<AgentToService, IpcFrameError> {
    read_ipc_frame(reader)
}

fn encode_ipc_frame(message: &impl Serialize) -> Result<Vec<u8>, IpcFrameError> {
    let payload = serde_json::to_vec(message).map_err(IpcFrameError::Json)?;
    if payload.len() > MAX_IPC_FRAME_LEN {
        return Err(IpcFrameError::PayloadTooLarge {
            actual: payload.len(),
            max: MAX_IPC_FRAME_LEN,
        });
    }

    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

fn decode_ipc_frame<T>(frame: &[u8]) -> Result<T, IpcFrameError>
where
    T: for<'de> Deserialize<'de>,
{
    let Some(header) = frame.get(..4) else {
        return Err(IpcFrameError::Io(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "IPC 消息长度头不完整",
        )));
    };

    let len = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
    if len > MAX_IPC_FRAME_LEN {
        return Err(IpcFrameError::PayloadTooLarge {
            actual: len,
            max: MAX_IPC_FRAME_LEN,
        });
    }

    let Some(payload) = frame.get(4..4 + len) else {
        let actual = frame.len().saturating_sub(4);
        return Err(IpcFrameError::IncompletePayload {
            expected: len,
            actual,
        });
    };

    serde_json::from_slice(payload).map_err(IpcFrameError::Json)
}

fn write_ipc_frame(writer: &mut impl Write, message: &impl Serialize) -> Result<(), IpcFrameError> {
    let frame = encode_ipc_frame(message)?;
    writer.write_all(&frame)?;
    Ok(())
}

fn read_ipc_frame<T>(reader: &mut impl Read) -> Result<T, IpcFrameError>
where
    T: for<'de> Deserialize<'de>,
{
    let mut header = [0_u8; 4];
    reader.read_exact(&mut header)?;
    let len = u32::from_be_bytes(header) as usize;
    if len > MAX_IPC_FRAME_LEN {
        return Err(IpcFrameError::PayloadTooLarge {
            actual: len,
            max: MAX_IPC_FRAME_LEN,
        });
    }

    let mut payload = vec![0_u8; len];
    let mut read = 0;
    while read < len {
        match reader.read(&mut payload[read..]) {
            Ok(0) => {
                return Err(IpcFrameError::IncompletePayload {
                    expected: len,
                    actual: read,
                });
            }
            Ok(n) => read += n,
            Err(error) => return Err(IpcFrameError::Io(error)),
        }
    }

    serde_json::from_slice(&payload).map_err(IpcFrameError::Json)
}
