use wincast_input::input::{
    CaptureInputBounds, InputInjectionError, map_input_event_to_windows_actions,
};
use wincast_protocol::input::{ButtonState, InputEvent, Modifiers, MouseButton};

#[test]
fn maps_mouse_move_to_absolute_windows_action() {
    let bounds = CaptureInputBounds::from_capture_size(100, 200, 1280, 720);

    let actions =
        map_input_event_to_windows_actions(InputEvent::MouseMove { x: 640.0, y: 360.0 }, bounds)
            .expect("mouse move should map");

    assert_eq!(
        actions,
        vec![wincast_input::input::WindowsInputAction::MoveAbsolute { x: 740, y: 560 }]
    );
}

#[test]
fn maps_mouse_move_absolute_to_absolute_windows_action() {
    let bounds = CaptureInputBounds::from_capture_size(100, 200, 1280, 720);

    let actions = map_input_event_to_windows_actions(
        InputEvent::MouseMoveAbsolute { x: 640.0, y: 360.0 },
        bounds,
    )
    .expect("absolute mouse move should map");

    assert_eq!(
        actions,
        vec![wincast_input::input::WindowsInputAction::MoveAbsolute { x: 740, y: 560 }]
    );
}

#[test]
fn maps_mouse_move_delta_to_relative_windows_action() {
    let actions = map_input_event_to_windows_actions(
        InputEvent::MouseMoveDelta {
            delta_x: -12,
            delta_y: 34,
        },
        CaptureInputBounds::from_capture_size(0, 0, 0, 0),
    )
    .expect("relative mouse move should not depend on capture bounds");

    assert_eq!(
        actions,
        vec![wincast_input::input::WindowsInputAction::MoveRelative {
            delta_x: -12,
            delta_y: 34,
        }]
    );
}

#[test]
fn clamps_mouse_move_inside_capture_bounds() {
    let bounds = CaptureInputBounds::from_capture_size(10, 20, 100, 50);

    let actions =
        map_input_event_to_windows_actions(InputEvent::MouseMove { x: 120.0, y: -10.0 }, bounds)
            .expect("mouse move should map");

    assert_eq!(
        actions,
        vec![wincast_input::input::WindowsInputAction::MoveAbsolute { x: 109, y: 20 }]
    );
}

#[test]
fn reports_invalid_bounds_and_mouse_coordinates() {
    let invalid_bounds = map_input_event_to_windows_actions(
        InputEvent::MouseMove { x: 1.0, y: 1.0 },
        CaptureInputBounds::from_capture_size(0, 0, 0, 720),
    )
    .expect_err("zero-sized bounds should fail");

    assert_eq!(
        invalid_bounds.to_string(),
        "输入映射失败：捕获区域宽高必须大于 0"
    );

    let invalid_coordinate = map_input_event_to_windows_actions(
        InputEvent::MouseMove {
            x: f32::NAN,
            y: 1.0,
        },
        CaptureInputBounds::from_capture_size(0, 0, 1280, 720),
    )
    .expect_err("invalid coordinate should fail");

    assert_eq!(
        invalid_coordinate,
        InputInjectionError::InvalidMouseCoordinate
    );
}

#[test]
fn maps_mouse_button_wheel_and_keyboard_events() {
    let bounds = CaptureInputBounds::from_capture_size(0, 0, 1280, 720);
    let modifiers = Modifiers {
        shift: true,
        ctrl: false,
        alt: true,
        logo: false,
    };

    assert_eq!(
        map_input_event_to_windows_actions(
            InputEvent::MouseButton {
                button: MouseButton::Left,
                state: ButtonState::Pressed,
            },
            bounds,
        )
        .expect("button should map"),
        vec![wincast_input::input::WindowsInputAction::MouseButton {
            button: MouseButton::Left,
            state: ButtonState::Pressed,
        }]
    );
    assert_eq!(
        map_input_event_to_windows_actions(
            InputEvent::MouseWheel {
                delta_x: -2,
                delta_y: 3,
            },
            bounds,
        )
        .expect("wheel should map"),
        vec![
            wincast_input::input::WindowsInputAction::VerticalWheel { delta: 360 },
            wincast_input::input::WindowsInputAction::HorizontalWheel { delta: -240 },
        ]
    );
    assert_eq!(
        map_input_event_to_windows_actions(
            InputEvent::Key {
                code: 65,
                state: ButtonState::Released,
                modifiers,
            },
            bounds,
        )
        .expect("key should map"),
        vec![wincast_input::input::WindowsInputAction::Keyboard {
            virtual_key: 0x41,
            state: ButtonState::Released,
            modifiers,
        }]
    );
}

#[cfg(not(windows))]
#[test]
fn non_windows_sendinput_unsupported_message_is_chinese() {
    let error = wincast_input::input::WindowsInputEventSink::new(
        CaptureInputBounds::from_capture_size(0, 0, 1280, 720),
    )
    .inject(InputEvent::MouseButton {
        button: MouseButton::Right,
        state: ButtonState::Released,
    })
    .expect_err("non-windows should fail");

    assert_eq!(
        error.to_string(),
        "当前平台不支持输入注入：仅 Windows 支持 SendInput"
    );
}
