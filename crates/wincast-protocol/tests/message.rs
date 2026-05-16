use wincast_protocol::{
    config::VideoCodec,
    frame::{decode_message, encode_message},
    input::{ButtonState, InputEvent, Modifiers},
    message::{
        ControlMessage, EncodedVideoFrame, EncodedVideoFrameError, ErrorCode,
        MAX_ENCODED_VIDEO_FRAME_BYTES,
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
