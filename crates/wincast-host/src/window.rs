use std::fmt;

const MIN_WINDOW_WIDTH: i32 = 64;
const MIN_WINDOW_HEIGHT: i32 = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WindowCandidate {
    pub(crate) handle: isize,
    pub(crate) process_id: u32,
    pub(crate) title: String,
    pub(crate) visible: bool,
    pub(crate) tool_window: bool,
    pub(crate) rect: WindowRect,
    pub(crate) monitor_rect: WindowRect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WindowRect {
    pub(crate) left: i32,
    pub(crate) top: i32,
    pub(crate) right: i32,
    pub(crate) bottom: i32,
}

impl WindowRect {
    pub(crate) fn width(self) -> i32 {
        self.right - self.left
    }

    pub(crate) fn height(self) -> i32 {
        self.bottom - self.top
    }

    fn area(self) -> i64 {
        i64::from(self.width()) * i64::from(self.height())
    }

    fn has_normal_size(self) -> bool {
        self.width() >= MIN_WINDOW_WIDTH && self.height() >= MIN_WINDOW_HEIGHT
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WindowLookupError {
    NotFound {
        process_id: u32,
        title_contains: Option<String>,
    },
    #[cfg(not(windows))]
    UnsupportedPlatform,
    #[cfg(windows)]
    EnumerationFailed(String),
}

impl fmt::Display for WindowLookupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound {
                process_id,
                title_contains,
            } => {
                write!(formatter, "未找到进程 {process_id} 的主窗口")?;
                if let Some(title_contains) = title_contains {
                    write!(formatter, "，标题需包含 {title_contains}")?;
                }
                Ok(())
            }
            #[cfg(not(windows))]
            Self::UnsupportedPlatform => {
                formatter.write_str("当前平台不支持主窗口定位：仅 Windows 支持按进程枚举顶层窗口")
            }
            #[cfg(windows)]
            Self::EnumerationFailed(message) => {
                write!(formatter, "枚举 Windows 顶层窗口失败: {message}")
            }
        }
    }
}

impl std::error::Error for WindowLookupError {}

pub(crate) fn select_main_window(
    windows: &[WindowCandidate],
    process_id: u32,
    title_contains: Option<&str>,
) -> Result<WindowCandidate, WindowLookupError> {
    eligible_windows(windows, process_id, title_contains)
        .into_iter()
        .max_by_key(|window| window.rect.area())
        .ok_or_else(|| WindowLookupError::NotFound {
            process_id,
            title_contains: normalized_title_filter(title_contains).map(ToOwned::to_owned),
        })
}

pub(crate) fn eligible_windows(
    windows: &[WindowCandidate],
    process_id: u32,
    title_contains: Option<&str>,
) -> Vec<WindowCandidate> {
    let title_contains = normalized_title_filter(title_contains);
    windows
        .iter()
        .filter(|window| is_eligible_window(window, process_id, title_contains))
        .cloned()
        .collect()
}

fn is_eligible_window(
    window: &WindowCandidate,
    process_id: u32,
    title_contains: Option<&str>,
) -> bool {
    window.process_id == process_id
        && window.visible
        && !window.tool_window
        && window.rect.has_normal_size()
        && title_matches(&window.title, title_contains)
}

fn title_matches(title: &str, title_contains: Option<&str>) -> bool {
    match title_contains {
        Some(filter) => title.contains(filter),
        None => true,
    }
}

fn normalized_title_filter(title_contains: Option<&str>) -> Option<&str> {
    title_contains
        .map(str::trim)
        .filter(|title_contains| !title_contains.is_empty())
}

#[cfg(windows)]
pub(crate) fn find_main_window(
    process_id: u32,
    title_contains: Option<&str>,
) -> Result<WindowCandidate, WindowLookupError> {
    let windows = enumerate_top_level_windows()?;
    select_main_window(&windows, process_id, title_contains)
}

#[cfg(not(windows))]
pub(crate) fn find_main_window(
    _process_id: u32,
    _title_contains: Option<&str>,
) -> Result<WindowCandidate, WindowLookupError> {
    Err(WindowLookupError::UnsupportedPlatform)
}

#[cfg(windows)]
fn enumerate_top_level_windows() -> Result<Vec<WindowCandidate>, WindowLookupError> {
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, RECT},
        Graphics::Gdi::{
            GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
        },
        UI::WindowsAndMessaging::{
            EnumWindows, GWL_EXSTYLE, GetWindowLongPtrW, GetWindowRect, GetWindowTextLengthW,
            GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible, WS_EX_TOOLWINDOW,
        },
    };

    struct EnumState {
        windows: Vec<WindowCandidate>,
        error: Option<String>,
    }

    unsafe extern "system" fn enum_window(hwnd: HWND, lparam: LPARAM) -> i32 {
        let state = unsafe { &mut *(lparam as *mut EnumState) };

        match unsafe { inspect_window(hwnd) } {
            Ok(candidate) => state.windows.push(candidate),
            Err(error) => {
                state.error = Some(error);
                return 0;
            }
        }

        1
    }

    unsafe fn inspect_window(hwnd: HWND) -> Result<WindowCandidate, String> {
        let mut process_id = 0_u32;
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut process_id);
        }

        let mut rect = RECT::default();
        let got_rect = unsafe { GetWindowRect(hwnd, &mut rect) };
        if got_rect == 0 {
            return Err("GetWindowRect 返回失败".to_owned());
        }

        let title = unsafe { read_window_title(hwnd) };
        let extended_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
        let tool_window = (extended_style & WS_EX_TOOLWINDOW as isize) != 0;
        let monitor_rect = unsafe { monitor_rect_for_window(hwnd) }?;

        Ok(WindowCandidate {
            handle: hwnd as isize,
            process_id,
            title,
            visible: unsafe { IsWindowVisible(hwnd) != 0 },
            tool_window,
            rect: WindowRect {
                left: rect.left,
                top: rect.top,
                right: rect.right,
                bottom: rect.bottom,
            },
            monitor_rect,
        })
    }

    unsafe fn monitor_rect_for_window(hwnd: HWND) -> Result<WindowRect, String> {
        let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
        if monitor.is_null() {
            return Err("MonitorFromWindow 未返回显示器".to_owned());
        }

        let mut monitor_info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let got_monitor = unsafe { GetMonitorInfoW(monitor, &mut monitor_info) };
        if got_monitor == 0 {
            return Err("GetMonitorInfoW 返回失败".to_owned());
        }

        Ok(WindowRect {
            left: monitor_info.rcMonitor.left,
            top: monitor_info.rcMonitor.top,
            right: monitor_info.rcMonitor.right,
            bottom: monitor_info.rcMonitor.bottom,
        })
    }

    unsafe fn read_window_title(hwnd: HWND) -> String {
        let length = unsafe { GetWindowTextLengthW(hwnd) };
        if length <= 0 {
            return String::new();
        }

        let mut buffer = vec![0_u16; length as usize + 1];
        let copied = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
        String::from_utf16_lossy(&buffer[..copied as usize])
    }

    let mut state = EnumState {
        windows: Vec::new(),
        error: None,
    };

    let ok = unsafe { EnumWindows(Some(enum_window), (&mut state as *mut EnumState) as LPARAM) };
    if ok == 0 {
        return Err(WindowLookupError::EnumerationFailed(
            state
                .error
                .unwrap_or_else(|| "EnumWindows 返回失败".to_owned()),
        ));
    }

    Ok(state.windows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_windows_by_process_visibility_tool_style_size_and_title() {
        let windows = [
            window(1, 42, "Main Editor", true, false, rect(0, 0, 1280, 720)),
            window(2, 7, "Other Process", true, false, rect(0, 0, 1280, 720)),
            window(3, 42, "Hidden Main", false, false, rect(0, 0, 1280, 720)),
            window(4, 42, "Tool Main", true, true, rect(0, 0, 1280, 720)),
            window(5, 42, "Tiny Main", true, false, rect(0, 0, 16, 16)),
            window(6, 42, "Settings", true, false, rect(0, 0, 1280, 720)),
        ];

        let matches = eligible_windows(&windows, 42, Some("Main"));

        assert_eq!(matches, vec![windows[0].clone()]);
    }

    #[test]
    fn selects_largest_eligible_window_as_main_window() {
        let windows = [
            window(1, 42, "Splash", true, false, rect(0, 0, 300, 200)),
            window(2, 42, "Application", true, false, rect(0, 0, 1024, 768)),
            window(3, 42, "Dialog", true, false, rect(0, 0, 640, 480)),
        ];

        let selected = select_main_window(&windows, 42, None).expect("main window should match");

        assert_eq!(selected.handle, 2);
    }

    #[test]
    fn reports_no_matching_window_with_chinese_message() {
        let error = select_main_window(&[], 42, Some("不存在"))
            .expect_err("empty window list should not match");

        assert_eq!(
            error.to_string(),
            "未找到进程 42 的主窗口，标题需包含 不存在"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_find_main_window_returns_clear_chinese_error() {
        let error = find_main_window(42, None).expect_err("non-windows should fail");

        assert_eq!(
            error.to_string(),
            "当前平台不支持主窗口定位：仅 Windows 支持按进程枚举顶层窗口"
        );
    }

    fn window(
        handle: isize,
        process_id: u32,
        title: &str,
        visible: bool,
        tool_window: bool,
        rect: WindowRect,
    ) -> WindowCandidate {
        WindowCandidate {
            handle,
            process_id,
            title: title.to_owned(),
            visible,
            tool_window,
            rect,
            monitor_rect: rect,
        }
    }

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> WindowRect {
        WindowRect {
            left,
            top,
            right,
            bottom,
        }
    }
}
