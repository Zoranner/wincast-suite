use std::io::Cursor;

use wincast_protocol::{
    config::VideoCodec,
    frame::{FrameError, MAX_FRAME_LEN, decode_message, read_message, write_message},
    input::{ButtonState, InputEvent, Modifiers},
    message::{ControlMessage, EncodedVideoFrame, ErrorCode},
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
