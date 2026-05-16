use std::io::{Read, Write};

use thiserror::Error;

use crate::{
    frame::{FrameError, MAX_FRAME_LEN},
    message::ControlMessage,
};

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

#[derive(Debug, Clone, PartialEq)]
pub enum RawBgraStreamItem {
    Frame(RawBgraFrame),
    Control(ControlMessage),
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
    #[error("raw BGRA 帧 magic 无效且控制消息长度 {actual} 超过限制 {max}")]
    InvalidMagicAndControlMessageTooLarge { actual: usize, max: usize },
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
    #[error("{0}")]
    ControlFrame(FrameError),
}

impl PartialEq for RawFrameError {
    fn eq(&self, other: &Self) -> bool {
        matches!((self, other), (Self::Io(_), Self::Io(_)))
            || matches!((self, other), (Self::ControlFrame(left), Self::ControlFrame(right)) if left == right)
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
                    Self::InvalidMagicAndControlMessageTooLarge {
                        actual: left_actual,
                        max: left_max,
                    },
                    Self::InvalidMagicAndControlMessageTooLarge {
                        actual: right_actual,
                        max: right_max,
                    },
                ) if left_actual == right_actual && left_max == right_max
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

impl From<FrameError> for RawFrameError {
    fn from(error: FrameError) -> Self {
        Self::ControlFrame(error)
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
    let mut magic = [0_u8; 4];
    reader.read_exact(&mut magic)?;
    if magic != RAW_BGRA_MAGIC {
        return Err(RawFrameError::InvalidMagic(magic));
    }

    read_raw_bgra_frame_after_magic(reader)
}

pub fn read_raw_bgra_stream_item(
    reader: &mut impl Read,
) -> Result<RawBgraStreamItem, RawFrameError> {
    let mut prefix = [0_u8; 4];
    reader.read_exact(&mut prefix)?;

    if prefix == RAW_BGRA_MAGIC {
        return read_raw_bgra_frame_after_magic(reader).map(RawBgraStreamItem::Frame);
    }

    let len = u32::from_be_bytes(prefix) as usize;
    if len > MAX_FRAME_LEN {
        return Err(RawFrameError::InvalidMagicAndControlMessageTooLarge {
            actual: len,
            max: MAX_FRAME_LEN,
        });
    }

    let mut payload = vec![0_u8; len];
    reader.read_exact(&mut payload)?;
    let message = serde_json::from_slice(&payload).map_err(FrameError::Decode)?;
    Ok(RawBgraStreamItem::Control(message))
}

fn read_raw_bgra_frame_after_magic(reader: &mut impl Read) -> Result<RawBgraFrame, RawFrameError> {
    let mut rest = [0_u8; RAW_BGRA_HEADER_LEN - 4];
    reader.read_exact(&mut rest)?;

    let payload_len = u32::from_be_bytes([rest[28], rest[29], rest[30], rest[31]]) as usize;
    if payload_len > MAX_RAW_BGRA_FRAME_BYTES {
        return Err(RawFrameError::PayloadTooLarge {
            actual: payload_len,
            max: MAX_RAW_BGRA_FRAME_BYTES,
        });
    }

    let mut bytes = vec![0_u8; payload_len];
    reader.read_exact(&mut bytes)?;

    let frame = RawBgraFrame {
        width: u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]),
        height: u32::from_be_bytes([rest[4], rest[5], rest[6], rest[7]]),
        row_pitch: u32::from_be_bytes([rest[8], rest[9], rest[10], rest[11]]),
        sequence_number: u64::from_be_bytes([
            rest[12], rest[13], rest[14], rest[15], rest[16], rest[17], rest[18], rest[19],
        ]),
        timestamp_ns: u64::from_be_bytes([
            rest[20], rest[21], rest[22], rest[23], rest[24], rest[25], rest[26], rest[27],
        ]),
        bytes,
    };
    frame.validate()?;
    Ok(frame)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::{frame::write_message, message::ControlMessage};

    #[test]
    fn raw_bgra_binary_frame_round_trips_inside_crate_only() {
        let frame = RawBgraFrame {
            width: 2,
            height: 2,
            row_pitch: 8,
            sequence_number: 9,
            timestamp_ns: 123_456,
            bytes: vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
        };
        let mut bytes = Vec::new();

        write_raw_bgra_frame(&mut bytes, &frame).expect("raw BGRA frame should encode");

        assert_eq!(&bytes[..4], b"WCBG");
        let decoded =
            read_raw_bgra_frame(&mut Cursor::new(bytes)).expect("raw BGRA frame should decode");
        assert_eq!(decoded, frame);
    }

    #[test]
    fn raw_bgra_stream_item_reads_raw_and_control_messages_inside_crate_only() {
        let first = RawBgraFrame {
            width: 2,
            height: 2,
            row_pitch: 8,
            sequence_number: 1,
            timestamp_ns: 10,
            bytes: vec![1; 16],
        };
        let message = ControlMessage::Goodbye;
        let second = RawBgraFrame {
            width: 1,
            height: 1,
            row_pitch: 4,
            sequence_number: 2,
            timestamp_ns: 20,
            bytes: vec![2; 4],
        };
        let mut bytes = Vec::new();
        write_raw_bgra_frame(&mut bytes, &first).expect("first raw frame should encode");
        write_message(&mut bytes, &message).expect("control message should encode");
        write_raw_bgra_frame(&mut bytes, &second).expect("second raw frame should encode");

        let mut cursor = Cursor::new(bytes);
        let decoded_first =
            read_raw_bgra_stream_item(&mut cursor).expect("first stream item should decode");
        let decoded_message =
            read_raw_bgra_stream_item(&mut cursor).expect("control stream item should decode");
        let decoded_second =
            read_raw_bgra_stream_item(&mut cursor).expect("second stream item should decode");

        assert_eq!(decoded_first, RawBgraStreamItem::Frame(first));
        assert_eq!(decoded_message, RawBgraStreamItem::Control(message));
        assert_eq!(decoded_second, RawBgraStreamItem::Frame(second));
    }

    #[test]
    fn raw_bgra_binary_frame_rejects_payload_above_limit_before_reading_payload() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"WCBG");
        bytes.extend_from_slice(&1_u32.to_be_bytes());
        bytes.extend_from_slice(&((MAX_RAW_BGRA_FRAME_BYTES / 4 + 1) as u32).to_be_bytes());
        bytes.extend_from_slice(&4_u32.to_be_bytes());
        bytes.extend_from_slice(&9_u64.to_be_bytes());
        bytes.extend_from_slice(&123_456_u64.to_be_bytes());
        bytes.extend_from_slice(&((MAX_RAW_BGRA_FRAME_BYTES + 4) as u32).to_be_bytes());

        let err = read_raw_bgra_frame(&mut Cursor::new(bytes))
            .expect_err("oversized raw payload should fail before payload read");

        assert_eq!(
            err,
            RawFrameError::PayloadTooLarge {
                actual: MAX_RAW_BGRA_FRAME_BYTES + 4,
                max: MAX_RAW_BGRA_FRAME_BYTES,
            }
        );
    }

    #[test]
    fn raw_bgra_binary_frame_rejects_invalid_payload_shape() {
        let frame = RawBgraFrame {
            width: 2,
            height: 2,
            row_pitch: 8,
            sequence_number: 9,
            timestamp_ns: 123_456,
            bytes: vec![0; 15],
        };

        assert_eq!(
            frame.validate(),
            Err(RawFrameError::InvalidPayloadLength {
                actual: 15,
                expected: 16,
            })
        );
    }
}
