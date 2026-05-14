use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum InputEvent {
    MouseMove {
        x: f32,
        y: f32,
    },
    MouseMoveAbsolute {
        x: f32,
        y: f32,
    },
    MouseMoveDelta {
        delta_x: i32,
        delta_y: i32,
    },
    MouseButton {
        button: MouseButton,
        state: ButtonState,
    },
    MouseWheel {
        delta_x: i32,
        delta_y: i32,
    },
    Key {
        code: u32,
        state: ButtonState,
        modifiers: Modifiers,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ButtonState {
    Pressed,
    Released,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub logo: bool,
}

#[cfg(test)]
mod tests {
    use super::InputEvent;

    #[test]
    fn serializes_absolute_and_delta_mouse_move_variants() {
        assert_eq!(
            serde_json::to_value(InputEvent::MouseMoveAbsolute { x: 1.5, y: 2.5 })
                .expect("absolute move should serialize"),
            serde_json::json!({
                "MouseMoveAbsolute": {
                    "x": 1.5,
                    "y": 2.5
                }
            })
        );
        assert_eq!(
            serde_json::to_value(InputEvent::MouseMoveDelta {
                delta_x: -7,
                delta_y: 9,
            })
            .expect("delta move should serialize"),
            serde_json::json!({
                "MouseMoveDelta": {
                    "delta_x": -7,
                    "delta_y": 9
                }
            })
        );
    }

    #[test]
    fn keeps_legacy_mouse_move_wire_compatibility() {
        let event: InputEvent = serde_json::from_value(serde_json::json!({
            "MouseMove": {
                "x": 10.0,
                "y": 20.0
            }
        }))
        .expect("legacy mouse move should still deserialize");

        assert_eq!(event, InputEvent::MouseMove { x: 10.0, y: 20.0 });
    }
}
