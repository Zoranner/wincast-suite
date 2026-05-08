use std::fmt;

use wincast_protocol::input::{ButtonState, InputEvent, Modifiers, MouseButton};

const WINDOWS_WHEEL_DELTA: i32 = 120;
const SDL_KEY_A: u32 = b'a' as u32;
const SDL_KEY_Z: u32 = b'z' as u32;
const SDL_KEY_0: u32 = b'0' as u32;
const SDL_KEY_9: u32 = b'9' as u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CaptureInputBounds {
    pub(crate) origin_x: i32,
    pub(crate) origin_y: i32,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputInjectionError {
    InvalidCaptureBounds,
    InvalidMouseCoordinate,
    UnsupportedKeyCode(u32),
    #[cfg(not(windows))]
    UnsupportedPlatform,
    #[cfg(windows)]
    WindowsSendInputFailed,
}

impl fmt::Display for InputInjectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCaptureBounds => {
                formatter.write_str("输入映射失败：捕获区域宽高必须大于 0")
            }
            Self::InvalidMouseCoordinate => {
                formatter.write_str("输入映射失败：鼠标坐标必须是有效数字")
            }
            Self::UnsupportedKeyCode(code) => {
                write!(
                    formatter,
                    "输入映射失败：按键码 {code} 超出 Windows virtual-key 范围"
                )
            }
            #[cfg(not(windows))]
            Self::UnsupportedPlatform => {
                formatter.write_str("当前平台不支持输入注入：仅 Windows 支持 SendInput")
            }
            #[cfg(windows)]
            Self::WindowsSendInputFailed => {
                formatter.write_str("Windows 输入注入失败：SendInput 未接受全部输入事件")
            }
        }
    }
}

impl std::error::Error for InputInjectionError {}

pub(crate) struct WindowsInputEventSink {
    bounds: CaptureInputBounds,
}

impl WindowsInputEventSink {
    pub(crate) fn new(bounds: CaptureInputBounds) -> Self {
        Self { bounds }
    }

    pub(crate) fn inject(&mut self, event: InputEvent) -> Result<(), InputInjectionError> {
        let actions = map_input_event_to_windows_actions(event, self.bounds)?;
        inject_windows_actions(&actions)
    }
}

impl crate::InputEventSink for WindowsInputEventSink {
    fn handle_input_event(&mut self, event: InputEvent) -> Result<(), String> {
        self.inject(event).map_err(|error| error.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindowsInputAction {
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

pub(crate) fn map_input_event_to_windows_actions(
    event: InputEvent,
    bounds: CaptureInputBounds,
) -> Result<Vec<WindowsInputAction>, InputInjectionError> {
    if bounds.width == 0 || bounds.height == 0 {
        return Err(InputInjectionError::InvalidCaptureBounds);
    }

    match event {
        InputEvent::MouseMove { x, y } => {
            let x = map_capture_coordinate(x, bounds.origin_x, bounds.width)?;
            let y = map_capture_coordinate(y, bounds.origin_y, bounds.height)?;
            Ok(vec![WindowsInputAction::MoveAbsolute { x, y }])
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

fn map_sdl_keycode_to_windows_virtual_key(code: u32) -> Option<u16> {
    match code {
        SDL_KEY_A..=SDL_KEY_Z => Some((code - 32) as u16),
        SDL_KEY_0..=SDL_KEY_9 => Some(code as u16),
        8 => Some(0x08),
        9 => Some(0x09),
        13 => Some(0x0D),
        27 => Some(0x1B),
        32 => Some(0x20),
        127 => Some(0x2E),
        1_073_741_897 => Some(0x2D),
        1_073_741_898 => Some(0x24),
        1_073_741_899 => Some(0x21),
        1_073_741_901 => Some(0x23),
        1_073_741_902 => Some(0x22),
        1_073_741_903 => Some(0x27),
        1_073_741_904 => Some(0x25),
        1_073_741_905 => Some(0x28),
        1_073_741_906 => Some(0x26),
        1_073_741_882..=1_073_741_893 => Some((0x70 + (code - 1_073_741_882)) as u16),
        1_073_742_048 => Some(0xA2),
        1_073_742_049 => Some(0xA0),
        1_073_742_050 => Some(0xA4),
        1_073_742_052 => Some(0xA3),
        1_073_742_053 => Some(0xA1),
        1_073_742_054 => Some(0xA5),
        _ => u16::try_from(code).ok(),
    }
}

fn map_capture_coordinate(
    coordinate: f32,
    origin: i32,
    span: u32,
) -> Result<i32, InputInjectionError> {
    if !coordinate.is_finite() {
        return Err(InputInjectionError::InvalidMouseCoordinate);
    }

    let max_offset = span.saturating_sub(1) as f32;
    Ok(origin.saturating_add(coordinate.clamp(0.0, max_offset).round() as i32))
}

#[cfg(not(windows))]
fn inject_windows_actions(_actions: &[WindowsInputAction]) -> Result<(), InputInjectionError> {
    Err(InputInjectionError::UnsupportedPlatform)
}

#[cfg(windows)]
fn inject_windows_actions(actions: &[WindowsInputAction]) -> Result<(), InputInjectionError> {
    use std::mem::size_of;

    use windows_sys::Win32::UI::{
        Input::KeyboardAndMouse::{
            INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_KEYUP,
            MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
            MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN,
            MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL, MOUSEINPUT, SendInput,
        },
        WindowsAndMessaging::{
            GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
            SM_YVIRTUALSCREEN,
        },
    };

    let virtual_desktop = VirtualDesktop {
        origin_x: unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) },
        origin_y: unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) },
        width: unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) },
        height: unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) },
    };

    let inputs: Vec<INPUT> = actions
        .iter()
        .map(|action| match *action {
            WindowsInputAction::MoveAbsolute { x, y } => {
                let (dx, dy) = virtual_desktop.normalize_absolute(x, y);
                mouse_input(
                    dx,
                    dy,
                    MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                    0,
                )
            }
            WindowsInputAction::MouseButton { button, state } => {
                mouse_input(0, 0, mouse_button_flags(button, state), 0)
            }
            WindowsInputAction::VerticalWheel { delta } => {
                mouse_input(0, 0, MOUSEEVENTF_WHEEL, delta as u32)
            }
            WindowsInputAction::HorizontalWheel { delta } => {
                mouse_input(0, 0, MOUSEEVENTF_HWHEEL, delta as u32)
            }
            WindowsInputAction::Keyboard {
                virtual_key, state, ..
            } => keyboard_input(virtual_key, key_flags(state)),
        })
        .collect();

    if inputs.is_empty() {
        return Ok(());
    }

    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            size_of::<INPUT>() as i32,
        )
    };
    let result = if sent == inputs.len() as u32 {
        Ok(())
    } else {
        Err(InputInjectionError::WindowsSendInputFailed)
    };

    fn mouse_input(dx: i32, dy: i32, flags: u32, mouse_data: u32) -> INPUT {
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx,
                    dy,
                    mouseData: mouse_data,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn keyboard_input(virtual_key: u16, flags: u32) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: virtual_key,
                    wScan: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn mouse_button_flags(button: MouseButton, state: ButtonState) -> u32 {
        match (button, state) {
            (MouseButton::Left, ButtonState::Pressed) => MOUSEEVENTF_LEFTDOWN,
            (MouseButton::Left, ButtonState::Released) => MOUSEEVENTF_LEFTUP,
            (MouseButton::Right, ButtonState::Pressed) => MOUSEEVENTF_RIGHTDOWN,
            (MouseButton::Right, ButtonState::Released) => MOUSEEVENTF_RIGHTUP,
            (MouseButton::Middle, ButtonState::Pressed) => MOUSEEVENTF_MIDDLEDOWN,
            (MouseButton::Middle, ButtonState::Released) => MOUSEEVENTF_MIDDLEUP,
        }
    }

    fn key_flags(state: ButtonState) -> u32 {
        match state {
            ButtonState::Pressed => 0,
            ButtonState::Released => KEYEVENTF_KEYUP,
        }
    }

    struct VirtualDesktop {
        origin_x: i32,
        origin_y: i32,
        width: i32,
        height: i32,
    }

    impl VirtualDesktop {
        fn normalize_absolute(&self, x: i32, y: i32) -> (i32, i32) {
            let width = self.width.max(1);
            let height = self.height.max(1);
            let x = x.clamp(self.origin_x, self.origin_x.saturating_add(width - 1));
            let y = y.clamp(self.origin_y, self.origin_y.saturating_add(height - 1));
            let x_span = (width - 1).max(1);
            let y_span = (height - 1).max(1);
            (
                ((i64::from(x - self.origin_x) * 65_535) / i64::from(x_span)) as i32,
                ((i64::from(y - self.origin_y) * 65_535) / i64::from(y_span)) as i32,
            )
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use wincast_protocol::input::{ButtonState, InputEvent, Modifiers, MouseButton};

    #[test]
    fn maps_mouse_move_from_capture_pixels_to_host_coordinates() {
        let bounds = CaptureInputBounds {
            origin_x: 100,
            origin_y: 200,
            width: 1280,
            height: 720,
        };

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
    fn clamps_mouse_move_inside_capture_bounds() {
        let bounds = CaptureInputBounds {
            origin_x: 10,
            origin_y: 20,
            width: 100,
            height: 50,
        };

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
        let bounds = CaptureInputBounds {
            origin_x: 0,
            origin_y: 0,
            width: 1280,
            height: 720,
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
        let bounds = CaptureInputBounds {
            origin_x: 0,
            origin_y: 0,
            width: 1280,
            height: 720,
        };
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
    fn maps_common_sdl_keycodes_to_windows_virtual_keys() {
        assert_eq!(
            map_sdl_keycode_to_windows_virtual_key(b'a' as u32),
            Some(0x41)
        );
        assert_eq!(
            map_sdl_keycode_to_windows_virtual_key(b'z' as u32),
            Some(0x5A)
        );
        assert_eq!(
            map_sdl_keycode_to_windows_virtual_key(b'7' as u32),
            Some(0x37)
        );
        assert_eq!(
            map_sdl_keycode_to_windows_virtual_key(1_073_741_904),
            Some(0x25)
        );
        assert_eq!(
            map_sdl_keycode_to_windows_virtual_key(1_073_741_906),
            Some(0x26)
        );
        assert_eq!(
            map_sdl_keycode_to_windows_virtual_key(1_073_741_882),
            Some(0x70)
        );
        assert_eq!(
            map_sdl_keycode_to_windows_virtual_key(1_073_742_048),
            Some(0xA2)
        );
    }

    #[test]
    fn mapper_reports_invalid_capture_bounds_in_chinese() {
        let error = map_input_event_to_windows_actions(
            InputEvent::MouseMove { x: 1.0, y: 1.0 },
            CaptureInputBounds {
                origin_x: 0,
                origin_y: 0,
                width: 0,
                height: 720,
            },
        )
        .expect_err("zero-sized bounds should fail");

        assert_eq!(error.to_string(), "输入映射失败：捕获区域宽高必须大于 0");
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_injector_returns_clear_unsupported_error() {
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
