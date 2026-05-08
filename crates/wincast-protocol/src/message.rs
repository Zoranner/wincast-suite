use serde::{Deserialize, Serialize};

use crate::input::InputEvent;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ControlMessage {
    Hello { version: u16 },
    StartSession,
    SessionReady { width: u32, height: u32 },
    VideoReady,
    InputEvent(InputEvent),
    StopSession,
    Error { code: ErrorCode, message: String },
    Heartbeat,
    Goodbye,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCode {
    Busy,
    InvalidConfig,
    ProgramLaunchFailed,
    WindowNotFound,
    CaptureFailed,
    TransportFailed,
    UnsupportedVersion,
}
