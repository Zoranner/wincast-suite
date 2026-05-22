use wincast_protocol::input::InputEvent;

use super::{
    error::InputInjectionError,
    sdl_virtual_key::map_sdl_keycode_to_windows_virtual_key,
    types::{CaptureInputBounds, WindowsInputAction},
};

const WINDOWS_WHEEL_DELTA: i32 = 120;

pub fn map_input_event_to_windows_actions(
    event: InputEvent,
    bounds: CaptureInputBounds,
) -> Result<Vec<WindowsInputAction>, InputInjectionError> {
    match event {
        InputEvent::MouseMove { x, y } | InputEvent::MouseMoveAbsolute { x, y } => {
            if bounds.width == 0
                || bounds.height == 0
                || bounds.client_width == 0
                || bounds.client_height == 0
            {
                return Err(InputInjectionError::InvalidCaptureBounds);
            }

            let x = map_capture_coordinate(x, bounds.origin_x, bounds.width, bounds.client_width)?;
            let y =
                map_capture_coordinate(y, bounds.origin_y, bounds.height, bounds.client_height)?;
            Ok(vec![WindowsInputAction::MoveAbsolute { x, y }])
        }
        InputEvent::MouseMoveDelta { delta_x, delta_y } => {
            Ok(vec![WindowsInputAction::MoveRelative { delta_x, delta_y }])
        }
        InputEvent::MouseButton { button, state } => {
            Ok(vec![WindowsInputAction::MouseButton { button, state }])
        }
        InputEvent::MouseWheel { delta_x, delta_y } => {
            let mut actions = Vec::new();
            if delta_y != 0 {
                actions.push(WindowsInputAction::VerticalWheel {
                    delta: delta_y.saturating_mul(WINDOWS_WHEEL_DELTA),
                });
            }
            if delta_x != 0 {
                actions.push(WindowsInputAction::HorizontalWheel {
                    delta: delta_x.saturating_mul(WINDOWS_WHEEL_DELTA),
                });
            }
            Ok(actions)
        }
        InputEvent::Key {
            code,
            state,
            modifiers,
        } => {
            let virtual_key = map_sdl_keycode_to_windows_virtual_key(code)
                .ok_or(InputInjectionError::UnsupportedKeyCode(code))?;
            Ok(vec![WindowsInputAction::Keyboard {
                virtual_key,
                state,
                modifiers,
            }])
        }
    }
}

fn map_capture_coordinate(
    coordinate: f32,
    origin: i32,
    capture_span: u32,
    client_span: u32,
) -> Result<i32, InputInjectionError> {
    if !coordinate.is_finite() {
        return Err(InputInjectionError::InvalidMouseCoordinate);
    }

    let max_client_offset = client_span.saturating_sub(1) as f32;
    let clamped = coordinate.clamp(0.0, max_client_offset);
    let scaled = if client_span <= 1 {
        0.0
    } else {
        clamped * capture_span.saturating_sub(1) as f32 / max_client_offset
    };
    Ok(origin.saturating_add(scaled.round() as i32))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wincast_protocol::input::{ButtonState, InputEvent, Modifiers, MouseButton};

    #[test]
    fn maps_mouse_move_from_capture_pixels_to_host_coordinates() {
        let bounds = CaptureInputBounds::from_capture_size(100, 200, 1280, 720);

        let actions = map_input_event_to_windows_actions(
            InputEvent::MouseMove { x: 640.0, y: 360.0 },
            bounds,
        )
        .expect("mouse move should map");

        assert_eq!(
            actions,
            vec![WindowsInputAction::MoveAbsolute { x: 740, y: 560 }]
        );
    }

    #[test]
    fn maps_mouse_move_absolute_from_capture_pixels_to_host_coordinates() {
        let bounds = CaptureInputBounds::from_capture_size(100, 200, 1280, 720);

        let actions = map_input_event_to_windows_actions(
            InputEvent::MouseMoveAbsolute { x: 640.0, y: 360.0 },
            bounds,
        )
        .expect("absolute mouse move should map");

        assert_eq!(
            actions,
            vec![WindowsInputAction::MoveAbsolute { x: 740, y: 560 }]
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
            vec![WindowsInputAction::MoveRelative {
                delta_x: -12,
                delta_y: 34,
            }]
        );
    }

    #[test]
    fn clamps_mouse_move_inside_capture_bounds() {
        let bounds = CaptureInputBounds::from_capture_size(10, 20, 100, 50);

        let actions = map_input_event_to_windows_actions(
            InputEvent::MouseMove { x: 120.0, y: -10.0 },
            bounds,
        )
        .expect("mouse move should map");

        assert_eq!(
            actions,
            vec![WindowsInputAction::MoveAbsolute { x: 109, y: 20 }]
        );
    }

    #[test]
    fn maps_mouse_button_and_wheel_events() {
        let bounds = CaptureInputBounds::from_capture_size(0, 0, 1280, 720);

        assert_eq!(
            map_input_event_to_windows_actions(
                InputEvent::MouseButton {
                    button: MouseButton::Left,
                    state: ButtonState::Pressed,
                },
                bounds,
            )
            .expect("button should map"),
            vec![WindowsInputAction::MouseButton {
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
                WindowsInputAction::VerticalWheel { delta: 360 },
                WindowsInputAction::HorizontalWheel { delta: -240 },
            ]
        );
    }

    #[test]
    fn maps_keyboard_events_without_hiding_modifier_state() {
        let bounds = CaptureInputBounds::from_capture_size(0, 0, 1280, 720);
        let modifiers = Modifiers {
            shift: true,
            ctrl: false,
            alt: true,
            logo: false,
        };

        let actions = map_input_event_to_windows_actions(
            InputEvent::Key {
                code: 65,
                state: ButtonState::Released,
                modifiers,
            },
            bounds,
        )
        .expect("key should map");

        assert_eq!(
            actions,
            vec![WindowsInputAction::Keyboard {
                virtual_key: 0x41,
                state: ButtonState::Released,
                modifiers,
            }]
        );
    }

    #[test]
    fn mapper_reports_invalid_capture_bounds_in_chinese() {
        let error = map_input_event_to_windows_actions(
            InputEvent::MouseMove { x: 1.0, y: 1.0 },
            CaptureInputBounds::from_capture_size(0, 0, 0, 720),
        )
        .expect_err("zero-sized bounds should fail");

        assert_eq!(error.to_string(), "输入映射失败：捕获区域宽高必须大于 0");
    }

    #[test]
    fn maps_scaled_video_coordinates_back_to_capture_pixels() {
        let bounds =
            CaptureInputBounds::from_capture_size(10, 20, 1920, 1080).with_client_size(1280, 720);

        let actions = map_input_event_to_windows_actions(
            InputEvent::MouseMove { x: 640.0, y: 360.0 },
            bounds,
        )
        .expect("scaled mouse move should map");

        assert_eq!(
            actions,
            vec![WindowsInputAction::MoveAbsolute { x: 970, y: 560 }]
        );
    }
}
