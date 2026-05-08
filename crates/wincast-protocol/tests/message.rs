use wincast_protocol::{
    input::{ButtonState, InputEvent, Modifiers},
    message::ControlMessage,
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
