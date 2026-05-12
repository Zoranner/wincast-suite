use wincast_protocol::input::{ButtonState, Modifiers, MouseButton};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureInputBounds {
    pub origin_x: i32,
    pub origin_y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsInputAction {
    MoveAbsolute {
        x: i32,
        y: i32,
    },
    MouseButton {
        button: MouseButton,
        state: ButtonState,
    },
    VerticalWheel {
        delta: i32,
    },
    HorizontalWheel {
        delta: i32,
    },
    Keyboard {
        virtual_key: u16,
        state: ButtonState,
        modifiers: Modifiers,
    },
}
