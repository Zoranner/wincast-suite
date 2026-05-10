use std::io::Cursor;

use wincast_protocol::{
    config::VideoCodec,
    frame::{FrameError, MAX_FRAME_LEN, decode_message, read_message, write_message},
    input::{ButtonState, InputEvent, Modifiers},
    message::{ControlMessage, EncodedVideoFrame, ErrorCode, RawBgraReadbackFrame},
    raw_frame::{
        MAX_RAW_BGRA_FRAME_BYTES, RawBgraFrame, RawFrameError, read_raw_bgra_frame,
        write_raw_bgra_frame,
    },
};

#[test]
fn length_prefixed_frame_round_trips_control_message() {
    let message = ControlMessage::InputEvent(InputEvent::Key {
        code: 65,
        state: ButtonState::Pressed,
        modifiers: Modifiers {
            shift: true,
            ctrl: false,
            alt: false,
            logo: false,
        },
    });
    let mut bytes = Vec::new();

    write_message(&mut bytes, &message).expect("message should encode");

    let decoded = read_message(&mut Cursor::new(bytes)).expect("message should decode");
    assert_eq!(decoded, message);
}

#[test]
fn length_prefixed_frame_round_trips_raw_bgra_readback_message() {
    let message = ControlMessage::RawBgraReadbackFrame(RawBgraReadbackFrame {
        width: 2,
        height: 2,
        stride_bytes: 8,
        texture_width: 2,
        texture_height: 2,
        row_pitch: 8,
        sequence_number: 1,
        timestamp_ns: 10,
        bytes: vec![1; 16],
    });
    let mut bytes = Vec::new();

    write_message(&mut bytes, &message).expect("raw frame should encode");

    let decoded = read_message(&mut Cursor::new(bytes)).expect("raw frame should decode");
    assert_eq!(decoded, message);
}

#[test]
fn length_prefixed_frame_round_trips_encoded_video_frame_message() {
    let message = ControlMessage::EncodedVideoFrame(EncodedVideoFrame {
        codec: VideoCodec::H264,
        width: 1280,
        height: 720,
        sequence_number: 2,
        timestamp_ns: 20,
        keyframe: true,
        bytes: vec![0, 0, 0, 1, 103],
    });
    let mut bytes = Vec::new();

    write_message(&mut bytes, &message).expect("encoded frame should encode");

    let decoded = read_message(&mut Cursor::new(bytes)).expect("encoded frame should decode");
    assert_eq!(decoded, message);
}

#[test]
fn length_prefixed_frame_round_trips_encoding_failed_error() {
    let message = ControlMessage::Error {
        code: ErrorCode::EncodingFailed,
        message: "Windows 视频编码器未实现：尚未接入 H.264 编码器。".to_owned(),
    };
    let mut bytes = Vec::new();

    write_message(&mut bytes, &message).expect("encoding error should encode");

    let decoded = read_message(&mut Cursor::new(bytes)).expect("encoding error should decode");
    assert_eq!(decoded, message);
}

#[test]
fn decode_rejects_frame_larger_than_limit_before_allocating_payload() {
    let mut frame = Vec::new();
    frame.extend_from_slice(&((MAX_FRAME_LEN + 1) as u32).to_be_bytes());

    let err = decode_message(&frame).expect_err("large frame should fail");

    assert_eq!(
        err,
        FrameError::FrameTooLarge {
            actual: MAX_FRAME_LEN + 1,
            max: MAX_FRAME_LEN,
        }
    );
}

#[test]
fn read_rejects_frame_larger_than_limit_before_allocating_payload() {
    let mut frame = Vec::new();
    frame.extend_from_slice(&((MAX_FRAME_LEN + 1) as u32).to_be_bytes());

    let err = read_message(&mut Cursor::new(frame)).expect_err("large frame should fail");

    assert_eq!(
        err,
        FrameError::FrameTooLarge {
            actual: MAX_FRAME_LEN + 1,
            max: MAX_FRAME_LEN,
        }
    );
}

#[test]
fn read_reports_incomplete_payload() {
    let mut frame = Vec::new();
    frame.extend_from_slice(&8_u32.to_be_bytes());
    frame.extend_from_slice(b"{}");

    let err = read_message(&mut Cursor::new(frame)).expect_err("short payload should fail");

    assert!(matches!(err, FrameError::Io(_)));
}

#[test]
fn raw_bgra_binary_frame_round_trips_without_json_control_envelope() {
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
fn raw_bgra_binary_frame_reads_consecutive_frames_from_same_stream() {
    let first = RawBgraFrame {
        width: 2,
        height: 2,
        row_pitch: 8,
        sequence_number: 1,
        timestamp_ns: 10,
        bytes: vec![1; 16],
    };
    let second = RawBgraFrame {
        width: 2,
        height: 2,
        row_pitch: 8,
        sequence_number: 2,
        timestamp_ns: 20,
        bytes: vec![2; 16],
    };
    let mut bytes = Vec::new();

    write_raw_bgra_frame(&mut bytes, &first).expect("first raw frame should encode");
    write_raw_bgra_frame(&mut bytes, &second).expect("second raw frame should encode");

    let mut cursor = Cursor::new(bytes);
    let decoded_first = read_raw_bgra_frame(&mut cursor).expect("first raw frame should decode");
    let decoded_second = read_raw_bgra_frame(&mut cursor).expect("second raw frame should decode");

    assert_eq!(decoded_first, first);
    assert_eq!(decoded_second, second);
}

#[test]
fn raw_bgra_stream_item_reads_raw_and_control_messages_from_same_stream() {
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
    let decoded_first = wincast_protocol::raw_frame::read_raw_bgra_stream_item(&mut cursor)
        .expect("first stream item should decode");
    let decoded_message = wincast_protocol::raw_frame::read_raw_bgra_stream_item(&mut cursor)
        .expect("control stream item should decode");
    let decoded_second = wincast_protocol::raw_frame::read_raw_bgra_stream_item(&mut cursor)
        .expect("second stream item should decode");

    assert_eq!(
        decoded_first,
        wincast_protocol::raw_frame::RawBgraStreamItem::Frame(first)
    );
    assert_eq!(
        decoded_message,
        wincast_protocol::raw_frame::RawBgraStreamItem::Control(message)
    );
    assert_eq!(
        decoded_second,
        wincast_protocol::raw_frame::RawBgraStreamItem::Frame(second)
    );
}

#[test]
fn raw_bgra_stream_item_reads_binary_frame_without_json_control_envelope() {
    let frame = RawBgraFrame {
        width: 2,
        height: 2,
        row_pitch: 8,
        sequence_number: 3,
        timestamp_ns: 30,
        bytes: vec![3; 16],
    };
    let mut bytes = Vec::new();
    write_raw_bgra_frame(&mut bytes, &frame).expect("raw frame should encode");

    let item = wincast_protocol::raw_frame::read_raw_bgra_stream_item(&mut Cursor::new(bytes))
        .expect("stream item should decode");

    assert_eq!(
        item,
        wincast_protocol::raw_frame::RawBgraStreamItem::Frame(frame)
    );
}

#[test]
fn raw_bgra_stream_item_reads_interleaved_control_message() {
    let message = ControlMessage::Goodbye;
    let mut bytes = Vec::new();
    write_message(&mut bytes, &message).expect("control message should encode");

    let item = wincast_protocol::raw_frame::read_raw_bgra_stream_item(&mut Cursor::new(bytes))
        .expect("stream item should decode");

    assert_eq!(
        item,
        wincast_protocol::raw_frame::RawBgraStreamItem::Control(message)
    );
}

#[test]
fn raw_bgra_stream_item_rejects_oversized_control_message_after_unknown_magic() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&((MAX_FRAME_LEN + 1) as u32).to_be_bytes());

    let err = wincast_protocol::raw_frame::read_raw_bgra_stream_item(&mut Cursor::new(bytes))
        .expect_err("oversized control message should fail");

    assert_eq!(
        err,
        RawFrameError::InvalidMagicAndControlMessageTooLarge {
            actual: MAX_FRAME_LEN + 1,
            max: MAX_FRAME_LEN,
        }
    );
}

#[test]
fn raw_bgra_stream_item_allows_control_message_at_length_limit_until_payload_read() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&(MAX_FRAME_LEN as u32).to_be_bytes());

    let err = wincast_protocol::raw_frame::read_raw_bgra_stream_item(&mut Cursor::new(bytes))
        .expect_err("missing max-sized control payload should fail while reading payload");

    assert_eq!(
        err,
        RawFrameError::Io(std::io::ErrorKind::UnexpectedEof.into())
    );
}

#[test]
fn raw_bgra_stream_item_wraps_invalid_control_payload_after_unknown_magic() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&4_u32.to_be_bytes());
    bytes.extend_from_slice(b"nope");

    let err = wincast_protocol::raw_frame::read_raw_bgra_stream_item(&mut Cursor::new(bytes))
        .expect_err("invalid JSON control payload should fail");

    assert!(matches!(
        err,
        RawFrameError::ControlFrame(FrameError::Decode(_))
    ));
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
fn raw_bgra_binary_frame_rejects_row_pitch_smaller_than_bgra_width() {
    let frame = RawBgraFrame {
        width: 2,
        height: 1,
        row_pitch: 7,
        sequence_number: 1,
        timestamp_ns: 10,
        bytes: vec![0; 7],
    };

    assert_eq!(frame.validate(), Err(RawFrameError::InvalidRowPitch));
}

#[test]
fn raw_bgra_binary_frame_rejects_width_to_bgra_row_pitch_overflow() {
    let frame = RawBgraFrame {
        width: u32::MAX,
        height: 1,
        row_pitch: u32::MAX,
        sequence_number: 1,
        timestamp_ns: 10,
        bytes: Vec::new(),
    };

    assert_eq!(frame.validate(), Err(RawFrameError::SizeOverflow));
}

#[test]
fn raw_bgra_binary_frame_rejects_row_pitch_height_overflow() {
    let frame = RawBgraFrame {
        width: 1,
        height: u32::MAX,
        row_pitch: u32::MAX,
        sequence_number: 1,
        timestamp_ns: 10,
        bytes: Vec::new(),
    };

    assert_eq!(frame.validate(), Err(RawFrameError::SizeOverflow));
}

#[test]
fn raw_bgra_binary_frame_rejects_length_mismatch_after_payload_read() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"WCBG");
    bytes.extend_from_slice(&2_u32.to_be_bytes());
    bytes.extend_from_slice(&2_u32.to_be_bytes());
    bytes.extend_from_slice(&8_u32.to_be_bytes());
    bytes.extend_from_slice(&9_u64.to_be_bytes());
    bytes.extend_from_slice(&123_456_u64.to_be_bytes());
    bytes.extend_from_slice(&15_u32.to_be_bytes());
    bytes.extend_from_slice(&[0_u8; 15]);

    let err = read_raw_bgra_frame(&mut Cursor::new(bytes))
        .expect_err("payload length inconsistent with shape should fail");

    assert_eq!(
        err,
        RawFrameError::InvalidPayloadLength {
            actual: 15,
            expected: 16,
        }
    );
}

#[test]
fn raw_bgra_binary_frame_rejects_unknown_magic_before_payload_read() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"NOPE");
    bytes.extend_from_slice(&2_u32.to_be_bytes());
    bytes.extend_from_slice(&2_u32.to_be_bytes());
    bytes.extend_from_slice(&8_u32.to_be_bytes());
    bytes.extend_from_slice(&9_u64.to_be_bytes());
    bytes.extend_from_slice(&123_456_u64.to_be_bytes());
    bytes.extend_from_slice(&16_u32.to_be_bytes());

    let err = read_raw_bgra_frame(&mut Cursor::new(bytes)).expect_err("invalid magic should fail");

    assert_eq!(err, RawFrameError::InvalidMagic([b'N', b'O', b'P', b'E']));
}
