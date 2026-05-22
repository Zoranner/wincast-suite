use serde::{Deserialize, Serialize};

use crate::{config::VideoCodec, input::InputEvent};

pub const MAX_ENCODED_VIDEO_FRAME_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ControlMessage {
    Hello { version: u16 },
    StartSession,
    SessionReady { width: u32, height: u32 },
    EncodedVideoFrame(EncodedVideoFrame),
    InputEvent(InputEvent),
    StopSession,
    Error { code: ErrorCode, message: String },
    Heartbeat,
    Goodbye,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncodedVideoFrame {
    pub codec: VideoCodec,
    pub width: u32,
    pub height: u32,
    pub sequence_number: u64,
    pub timestamp_ns: u64,
    pub keyframe: bool,
    pub bytes: Vec<u8>,
}

impl EncodedVideoFrame {
    pub fn validate(&self) -> Result<(), EncodedVideoFrameError> {
        if self.width == 0 || self.height == 0 {
            return Err(EncodedVideoFrameError::InvalidDimensions);
        }
        if self.bytes.is_empty() {
            return Err(EncodedVideoFrameError::EmptyPayload);
        }
        if self.bytes.len() > MAX_ENCODED_VIDEO_FRAME_BYTES {
            return Err(EncodedVideoFrameError::PayloadTooLarge {
                actual: self.bytes.len(),
                max: MAX_ENCODED_VIDEO_FRAME_BYTES,
            });
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncodedVideoFrameError {
    InvalidDimensions,
    EmptyPayload,
    PayloadTooLarge { actual: usize, max: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCode {
    Busy,
    InvalidConfig,
    NoUserLoggedIn,
    SessionLocked,
    AgentUnavailable,
    ProgramLaunchFailed,
    ProgramExited,
    CaptureFailed,
    EncodingFailed,
    TransportFailed,
    UnsupportedVersion,
}
