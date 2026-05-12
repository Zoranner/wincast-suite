use wincast_protocol::input::InputEvent;

use super::{
    error::InputInjectionError, injector::inject_windows_actions,
    mapping::map_input_event_to_windows_actions, types::CaptureInputBounds,
};

pub struct WindowsInputEventSink {
    bounds: CaptureInputBounds,
}

impl WindowsInputEventSink {
    pub fn new(bounds: CaptureInputBounds) -> Self {
        Self { bounds }
    }

    pub fn inject(&mut self, event: InputEvent) -> Result<(), InputInjectionError> {
        let actions = map_input_event_to_windows_actions(event, self.bounds)?;
        inject_windows_actions(&actions)
    }
}

#[cfg(test)]
mod tests {
    #[cfg(not(windows))]
    #[test]
    fn non_windows_injector_returns_clear_unsupported_error() {
        use super::*;
        use wincast_protocol::input::{ButtonState, MouseButton};

        let mut sink = WindowsInputEventSink::new(CaptureInputBounds {
            origin_x: 0,
            origin_y: 0,
            width: 1280,
            height: 720,
        });

        let error = sink
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
}
