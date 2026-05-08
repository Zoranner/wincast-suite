use std::io::{Read, Write};

use thiserror::Error;

use crate::message::ControlMessage;

const MAX_FRAME_LEN: usize = 1024 * 1024;

#[derive(Debug, Error)]
pub enum FrameError {
    #[error("控制消息编码失败: {0}")]
    Encode(serde_json::Error),
    #[error("控制消息解码失败: {0}")]
    Decode(serde_json::Error),
    #[error("控制消息长度 {actual} 超过限制 {max}")]
    FrameTooLarge { actual: usize, max: usize },
    #[error("控制消息 IO 失败: {0}")]
    Io(std::io::Error),
}

impl PartialEq for FrameError {
    fn eq(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (Self::Encode(_), Self::Encode(_))
                | (Self::Decode(_), Self::Decode(_))
                | (Self::Io(_), Self::Io(_))
        ) || matches!(
            (self, other),
            (
                Self::FrameTooLarge {
                    actual: left_actual,
                    max: left_max,
                },
                Self::FrameTooLarge {
                    actual: right_actual,
                    max: right_max,
                },
            ) if left_actual == right_actual && left_max == right_max
        )
    }
}

impl Eq for FrameError {}

impl From<std::io::Error> for FrameError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

pub fn encode_message(message: &ControlMessage) -> Result<Vec<u8>, FrameError> {
    let payload = serde_json::to_vec(message).map_err(FrameError::Encode)?;
    if payload.len() > MAX_FRAME_LEN {
        return Err(FrameError::FrameTooLarge {
            actual: payload.len(),
            max: MAX_FRAME_LEN,
        });
    }

    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

pub fn decode_message(frame: &[u8]) -> Result<ControlMessage, FrameError> {
    let Some(header) = frame.get(..4) else {
        return Err(FrameError::Io(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "控制消息长度头不完整",
        )));
    };
    let len = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
    if len > MAX_FRAME_LEN {
        return Err(FrameError::FrameTooLarge {
            actual: len,
            max: MAX_FRAME_LEN,
        });
    }

    let payload = frame.get(4..4 + len).ok_or_else(|| {
        FrameError::Io(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "控制消息载荷不完整",
        ))
    })?;

    serde_json::from_slice(payload).map_err(FrameError::Decode)
}

pub fn write_message(writer: &mut impl Write, message: &ControlMessage) -> Result<(), FrameError> {
    let frame = encode_message(message)?;
    writer.write_all(&frame)?;
    Ok(())
}

pub fn read_message(reader: &mut impl Read) -> Result<ControlMessage, FrameError> {
    let mut header = [0_u8; 4];
    reader.read_exact(&mut header)?;
    let len = u32::from_be_bytes(header) as usize;
    if len > MAX_FRAME_LEN {
        return Err(FrameError::FrameTooLarge {
            actual: len,
            max: MAX_FRAME_LEN,
        });
    }

    let mut payload = vec![0_u8; len];
    reader.read_exact(&mut payload)?;
    serde_json::from_slice(&payload).map_err(FrameError::Decode)
}
