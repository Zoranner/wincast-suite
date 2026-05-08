use wincast_protocol::{
    input::{ButtonState, InputEvent, Modifiers},
    message::{
        ControlMessage, MAX_RAW_BGRA_READBACK_BYTES, RawBgraReadbackFrame,
        RawBgraReadbackFrameError,
    },
};

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
