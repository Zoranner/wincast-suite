use std::io::Cursor;

use wincast_protocol::{
    frame::{FrameError, MAX_FRAME_LEN, decode_message, read_message, write_message},
    input::{ButtonState, InputEvent, Modifiers},
    message::{ControlMessage, RawBgraReadbackFrame},
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
