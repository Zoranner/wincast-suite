use wincast_protocol::{
    config::VideoCodec,
    frame::{decode_message, encode_message},
    input::{ButtonState, InputEvent, Modifiers},
    message::{
        ControlMessage, EncodedVideoFrame, EncodedVideoFrameError, ErrorCode,
        MAX_ENCODED_VIDEO_FRAME_BYTES, MAX_RAW_BGRA_READBACK_BYTES, RawBgraReadbackFrame,
        RawBgraReadbackFrameError,
    },
};

#[test]
fn session_state_error_codes_round_trip_in_error_messages() {
    for code in [
        ErrorCode::NoUserLoggedIn,
        ErrorCode::SessionLocked,
        ErrorCode::AgentUnavailable,
    ] {
        let message = ControlMessage::Error {
            code,
            message: "宿主端会话状态不可用".to_owned(),
        };

        let frame = encode_message(&message).expect("error message should encode");
        let decoded = decode_message(&frame).expect("error message should decode");

        assert_eq!(decoded, message);
    }
}

#[test]
fn input_event_message_keeps_keyboard_state() {
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

    assert_eq!(
        message,
        ControlMessage::InputEvent(InputEvent::Key {
            code: 65,
            state: ButtonState::Pressed,
            modifiers: Modifiers {
                shift: true,
                ctrl: false,
                alt: false,
                logo: false,
            },
        })
    );
}

#[test]
fn raw_bgra_readback_frame_validates_payload_shape() {
    let frame = raw_bgra_frame(2, 2, 8, vec![0; 16]);

    frame.validate().expect("frame should be valid");

    assert_eq!(
        ControlMessage::RawBgraReadbackFrame(frame.clone()),
        ControlMessage::RawBgraReadbackFrame(frame)
    );
}

#[test]
fn raw_bgra_readback_frame_rejects_invalid_payload_shape() {
    let short_payload = raw_bgra_frame(2, 2, 8, vec![0; 15]);
    assert_eq!(
        short_payload.validate(),
        Err(RawBgraReadbackFrameError::InvalidPayloadLength {
            actual: 15,
            expected: 16,
        })
    );

    let narrow_row = raw_bgra_frame(2, 2, 4, vec![0; 8]);
    assert_eq!(
        narrow_row.validate(),
        Err(RawBgraReadbackFrameError::InvalidRowPitch)
    );

    let empty = raw_bgra_frame(0, 2, 8, vec![0; 16]);
    assert_eq!(
        empty.validate(),
        Err(RawBgraReadbackFrameError::InvalidDimensions)
    );
}

#[test]
fn raw_bgra_readback_frame_rejects_payloads_above_debug_limit() {
    let frame = raw_bgra_frame(
        1,
        (MAX_RAW_BGRA_READBACK_BYTES / 4 + 1) as u32,
        4,
        vec![0; MAX_RAW_BGRA_READBACK_BYTES + 4],
    );

    assert_eq!(
        frame.validate(),
        Err(RawBgraReadbackFrameError::PayloadTooLarge {
            actual: MAX_RAW_BGRA_READBACK_BYTES + 4,
            max: MAX_RAW_BGRA_READBACK_BYTES,
        })
    );
}

#[test]
fn encoded_video_frame_validates_payload_shape() {
    let frame = encoded_frame(vec![1, 2, 3]);

    frame.validate().expect("encoded frame should be valid");

    assert_eq!(
        ControlMessage::EncodedVideoFrame(frame.clone()),
        ControlMessage::EncodedVideoFrame(frame)
    );
}

#[test]
fn encoded_video_frame_rejects_invalid_payload_shape() {
    let empty_payload = encoded_frame(Vec::new());
    assert_eq!(
        empty_payload.validate(),
        Err(EncodedVideoFrameError::EmptyPayload)
    );

    let empty_width = EncodedVideoFrame {
        width: 0,
        ..encoded_frame(vec![1])
    };
    assert_eq!(
        empty_width.validate(),
        Err(EncodedVideoFrameError::InvalidDimensions)
    );
}

#[test]
fn encoded_video_frame_rejects_payloads_above_frame_limit() {
    let frame = encoded_frame(vec![0; MAX_ENCODED_VIDEO_FRAME_BYTES + 1]);

    assert_eq!(
        frame.validate(),
        Err(EncodedVideoFrameError::PayloadTooLarge {
            actual: MAX_ENCODED_VIDEO_FRAME_BYTES + 1,
            max: MAX_ENCODED_VIDEO_FRAME_BYTES,
        })
    );
}

fn raw_bgra_frame(width: u32, height: u32, row_pitch: u32, bytes: Vec<u8>) -> RawBgraReadbackFrame {
    RawBgraReadbackFrame {
        width,
        height,
        stride_bytes: width.saturating_mul(4),
        texture_width: width,
        texture_height: height,
        row_pitch,
        sequence_number: 7,
        timestamp_ns: 123_456,
        bytes,
    }
}

fn encoded_frame(bytes: Vec<u8>) -> EncodedVideoFrame {
    EncodedVideoFrame {
        codec: VideoCodec::H264,
        width: 1280,
        height: 720,
        sequence_number: 7,
        timestamp_ns: 123_456,
        keyframe: true,
        bytes,
    }
}
