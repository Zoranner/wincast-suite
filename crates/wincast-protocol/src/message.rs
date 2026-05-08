use serde::{Deserialize, Serialize};

use crate::input::InputEvent;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ControlMessage {
    Hello { version: u16 },
    StartSession,
    SessionReady { width: u32, height: u32 },
    VideoReady,
    RawBgraReadbackFrame(RawBgraReadbackFrame),
    InputEvent(InputEvent),
    StopSession,
    Error { code: ErrorCode, message: String },
    Heartbeat,
    Goodbye,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawBgraReadbackFrame {
    pub width: u32,
    pub height: u32,
    pub stride_bytes: u32,
    pub texture_width: u32,
    pub texture_height: u32,
    pub row_pitch: u32,
    pub sequence_number: u64,
    pub timestamp_ns: u64,
    pub bytes: Vec<u8>,
}

impl RawBgraReadbackFrame {
    pub fn validate(&self) -> Result<(), RawBgraReadbackFrameError> {
        if self.width == 0 || self.height == 0 {
            return Err(RawBgraReadbackFrameError::InvalidDimensions);
        }
        let min_row_pitch = self
            .width
            .checked_mul(4)
            .ok_or(RawBgraReadbackFrameError::SizeOverflow)?;
        if self.row_pitch < min_row_pitch || self.stride_bytes < min_row_pitch {
            return Err(RawBgraReadbackFrameError::InvalidRowPitch);
        }
        let expected_len = self
            .row_pitch
            .checked_mul(self.height)
            .ok_or(RawBgraReadbackFrameError::SizeOverflow)? as usize;
        if self.bytes.len() != expected_len {
            return Err(RawBgraReadbackFrameError::InvalidPayloadLength {
                actual: self.bytes.len(),
                expected: expected_len,
            });
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawBgraReadbackFrameError {
    InvalidDimensions,
    InvalidRowPitch,
    InvalidPayloadLength { actual: usize, expected: usize },
    SizeOverflow,
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
