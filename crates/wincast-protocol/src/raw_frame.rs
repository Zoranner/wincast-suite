use std::io::{Read, Write};

use thiserror::Error;

const RAW_BGRA_MAGIC: [u8; 4] = *b"WCBG";
const RAW_BGRA_HEADER_LEN: usize = 36;
pub const MAX_RAW_BGRA_FRAME_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawBgraFrame {
    pub width: u32,
    pub height: u32,
    pub row_pitch: u32,
    pub sequence_number: u64,
    pub timestamp_ns: u64,
    pub bytes: Vec<u8>,
}

impl RawBgraFrame {
    pub fn validate(&self) -> Result<(), RawFrameError> {
        if self.width == 0 || self.height == 0 {
            return Err(RawFrameError::InvalidDimensions);
        }

        let min_row_pitch = self
            .width
            .checked_mul(4)
            .ok_or(RawFrameError::SizeOverflow)?;
        if self.row_pitch < min_row_pitch {
            return Err(RawFrameError::InvalidRowPitch);
        }

        let expected = self
            .row_pitch
            .checked_mul(self.height)
            .ok_or(RawFrameError::SizeOverflow)? as usize;
        if self.bytes.len() != expected {
            return Err(RawFrameError::InvalidPayloadLength {
                actual: self.bytes.len(),
                expected,
            });
        }

        if self.bytes.len() > MAX_RAW_BGRA_FRAME_BYTES {
            return Err(RawFrameError::PayloadTooLarge {
                actual: self.bytes.len(),
                max: MAX_RAW_BGRA_FRAME_BYTES,
            });
        }

        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum RawFrameError {
    #[error("raw BGRA 帧 magic 无效: {0:?}")]
    InvalidMagic([u8; 4]),
    #[error("raw BGRA 帧尺寸无效")]
    InvalidDimensions,
    #[error("raw BGRA 帧 row pitch 无效")]
    InvalidRowPitch,
    #[error("raw BGRA 帧载荷长度 {actual} 与期望 {expected} 不一致")]
    InvalidPayloadLength { actual: usize, expected: usize },
    #[error("raw BGRA 帧载荷 {actual} 超过限制 {max}")]
    PayloadTooLarge { actual: usize, max: usize },
    #[error("raw BGRA 帧尺寸计算溢出")]
    SizeOverflow,
    #[error("raw BGRA 帧 IO 失败: {0}")]
    Io(std::io::Error),
}

impl PartialEq for RawFrameError {
    fn eq(&self, other: &Self) -> bool {
        matches!((self, other), (Self::Io(_), Self::Io(_)))
            || matches!(
                (self, other),
                (Self::InvalidMagic(left), Self::InvalidMagic(right)) if left == right
            )
            || matches!(
                (self, other),
                (Self::InvalidDimensions, Self::InvalidDimensions)
                    | (Self::InvalidRowPitch, Self::InvalidRowPitch)
                    | (Self::SizeOverflow, Self::SizeOverflow)
            )
            || matches!(
                (self, other),
                (
                    Self::InvalidPayloadLength {
                        actual: left_actual,
                        expected: left_expected,
                    },
                    Self::InvalidPayloadLength {
                        actual: right_actual,
                        expected: right_expected,
                    },
                ) if left_actual == right_actual && left_expected == right_expected
            )
            || matches!(
                (self, other),
                (
                    Self::PayloadTooLarge {
                        actual: left_actual,
                        max: left_max,
                    },
                    Self::PayloadTooLarge {
                        actual: right_actual,
                        max: right_max,
                    },
                ) if left_actual == right_actual && left_max == right_max
            )
    }
}

impl Eq for RawFrameError {}

impl From<std::io::Error> for RawFrameError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

pub fn write_raw_bgra_frame(
    writer: &mut impl Write,
    frame: &RawBgraFrame,
) -> Result<(), RawFrameError> {
    frame.validate()?;

    let mut header = [0_u8; RAW_BGRA_HEADER_LEN];
    header[0..4].copy_from_slice(&RAW_BGRA_MAGIC);
    header[4..8].copy_from_slice(&frame.width.to_be_bytes());
    header[8..12].copy_from_slice(&frame.height.to_be_bytes());
    header[12..16].copy_from_slice(&frame.row_pitch.to_be_bytes());
    header[16..24].copy_from_slice(&frame.sequence_number.to_be_bytes());
    header[24..32].copy_from_slice(&frame.timestamp_ns.to_be_bytes());
    header[32..36].copy_from_slice(&(frame.bytes.len() as u32).to_be_bytes());

    writer.write_all(&header)?;
    writer.write_all(&frame.bytes)?;
    Ok(())
}

pub fn read_raw_bgra_frame(reader: &mut impl Read) -> Result<RawBgraFrame, RawFrameError> {
    let mut header = [0_u8; RAW_BGRA_HEADER_LEN];
    reader.read_exact(&mut header)?;

    let magic = [header[0], header[1], header[2], header[3]];
    if magic != RAW_BGRA_MAGIC {
        return Err(RawFrameError::InvalidMagic(magic));
    }

    let payload_len = u32::from_be_bytes([header[32], header[33], header[34], header[35]]) as usize;
    if payload_len > MAX_RAW_BGRA_FRAME_BYTES {
        return Err(RawFrameError::PayloadTooLarge {
            actual: payload_len,
            max: MAX_RAW_BGRA_FRAME_BYTES,
        });
    }

    let mut bytes = vec![0_u8; payload_len];
    reader.read_exact(&mut bytes)?;

    let frame = RawBgraFrame {
        width: u32::from_be_bytes([header[4], header[5], header[6], header[7]]),
        height: u32::from_be_bytes([header[8], header[9], header[10], header[11]]),
        row_pitch: u32::from_be_bytes([header[12], header[13], header[14], header[15]]),
        sequence_number: u64::from_be_bytes([
            header[16], header[17], header[18], header[19], header[20], header[21], header[22],
            header[23],
        ]),
        timestamp_ns: u64::from_be_bytes([
            header[24], header[25], header[26], header[27], header[28], header[29], header[30],
            header[31],
        ]),
        bytes,
    };
    frame.validate()?;
    Ok(frame)
}
