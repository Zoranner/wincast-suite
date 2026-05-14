use super::{error::InputInjectionError, types::WindowsInputAction};

#[cfg(windows)]
use wincast_protocol::input::{ButtonState, MouseButton};
#[cfg(windows)]
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE,
};

#[cfg(not(windows))]
pub(super) fn inject_windows_actions(
    _actions: &[WindowsInputAction],
) -> Result<(), InputInjectionError> {
    Err(InputInjectionError::UnsupportedPlatform)
}

#[cfg(windows)]
pub(super) fn inject_windows_actions(
    actions: &[WindowsInputAction],
) -> Result<(), InputInjectionError> {
    use std::mem::size_of;

    use windows_sys::Win32::UI::{
        Input::KeyboardAndMouse::{
            MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_MOVE, MOUSEEVENTF_VIRTUALDESK,
            MOUSEEVENTF_WHEEL, SendInput,
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
            WindowsInputAction::MoveRelative { delta_x, delta_y } => {
                mouse_input(delta_x, delta_y, MOUSEEVENTF_MOVE, 0)
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
    if sent == inputs.len() as u32 {
        Ok(())
    } else {
        Err(InputInjectionError::WindowsSendInputFailed)
    }
}

#[cfg(windows)]
fn mouse_input(dx: i32, dy: i32, flags: u32, mouse_data: u32) -> INPUT {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::MOUSEINPUT;

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

#[cfg(windows)]
fn keyboard_input(virtual_key: u16, flags: u32) -> INPUT {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::KEYBDINPUT;

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

#[cfg(windows)]
fn mouse_button_flags(button: MouseButton, state: ButtonState) -> u32 {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP,
        MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
    };

    match (button, state) {
        (MouseButton::Left, ButtonState::Pressed) => MOUSEEVENTF_LEFTDOWN,
        (MouseButton::Left, ButtonState::Released) => MOUSEEVENTF_LEFTUP,
        (MouseButton::Right, ButtonState::Pressed) => MOUSEEVENTF_RIGHTDOWN,
        (MouseButton::Right, ButtonState::Released) => MOUSEEVENTF_RIGHTUP,
        (MouseButton::Middle, ButtonState::Pressed) => MOUSEEVENTF_MIDDLEDOWN,
        (MouseButton::Middle, ButtonState::Released) => MOUSEEVENTF_MIDDLEUP,
    }
}

#[cfg(windows)]
fn key_flags(state: ButtonState) -> u32 {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::KEYEVENTF_KEYUP;

    match state {
        ButtonState::Pressed => 0,
        ButtonState::Released => KEYEVENTF_KEYUP,
    }
}

#[cfg(windows)]
struct VirtualDesktop {
    origin_x: i32,
    origin_y: i32,
    width: i32,
    height: i32,
}

#[cfg(windows)]
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
