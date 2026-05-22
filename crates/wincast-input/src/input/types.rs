use wincast_protocol::input::{ButtonState, Modifiers, MouseButton};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureInputBounds {
    pub origin_x: i32,
    pub origin_y: i32,
    pub width: u32,
    pub height: u32,
    pub client_width: u32,
    pub client_height: u32,
}

impl CaptureInputBounds {
    pub fn from_capture_size(origin_x: i32, origin_y: i32, width: u32, height: u32) -> Self {
        Self {
            origin_x,
            origin_y,
            width,
            height,
            client_width: width,
            client_height: height,
        }
    }

    pub fn with_client_size(mut self, client_width: u32, client_height: u32) -> Self {
        self.client_width = client_width;
        self.client_height = client_height;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsInputAction {
    MoveAbsolute {
        x: i32,
        y: i32,
    },
    MoveRelative {
        delta_x: i32,
        delta_y: i32,
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
