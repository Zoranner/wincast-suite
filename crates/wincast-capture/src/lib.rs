use std::fmt;

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureTarget {
    Desktop,
    Window {
        handle: isize,
        width: u32,
        height: u32,
        title: Option<String>,
    },
}

impl fmt::Display for CaptureTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Desktop => formatter.write_str("整个桌面"),
            Self::Window {
                handle,
                width,
                height,
                title,
            } => {
                write!(formatter, "窗口 {handle}，尺寸 {width}x{height}")?;
                if let Some(title) = title {
                    write!(formatter, "，标题 {title}")?;
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FramePixelFormat {
    Bgra8Unorm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapturedFrame {
    pub width: u32,
    pub height: u32,
    pub stride_bytes: u32,
    pub pixel_format: FramePixelFormat,
    pub sequence_number: u64,
    pub timestamp_ns: u64,
}

#[derive(Debug)]
pub struct CaptureSession {
    target: CaptureTarget,
}

impl CaptureSession {
    pub fn start(target: CaptureTarget) -> Result<Self, CaptureError> {
        start_platform_capture(target)
    }

    pub fn target(&self) -> &CaptureTarget {
        &self.target
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CaptureError {
    #[error("Windows 画面捕获实现未完成：尚未接入帧池和帧获取循环")]
    WindowsCaptureNotImplemented,
    #[error("当前 Windows 系统不支持 Windows Graphics Capture")]
    WindowsGraphicsCaptureUnsupported,
    #[error("检测 Windows Graphics Capture 支持状态失败: {0}")]
    WindowsGraphicsCaptureSupportCheckFailed(String),
    #[error("创建窗口捕获目标失败: {0}")]
    WindowsCaptureItemCreateFailed(String),
    #[error("当前平台不支持画面捕获：仅 Windows 支持宿主端捕获，当前平台 {platform}")]
    UnsupportedPlatform { platform: String },
}

impl CaptureError {
    pub fn windows_capture_not_implemented() -> Self {
        Self::WindowsCaptureNotImplemented
    }

    pub fn windows_graphics_capture_unsupported() -> Self {
        Self::WindowsGraphicsCaptureUnsupported
    }

    pub fn windows_graphics_capture_support_check_failed(error: impl Into<String>) -> Self {
        Self::WindowsGraphicsCaptureSupportCheckFailed(error.into())
    }

    pub fn windows_capture_item_create_failed(error: impl Into<String>) -> Self {
        Self::WindowsCaptureItemCreateFailed(error.into())
    }

    pub fn unsupported_platform(platform: impl Into<String>) -> Self {
        Self::UnsupportedPlatform {
            platform: platform.into(),
        }
    }
}

#[cfg(windows)]
fn start_platform_capture(target: CaptureTarget) -> Result<CaptureSession, CaptureError> {
    use windows::Graphics::Capture::GraphicsCaptureSession;

    let supported = GraphicsCaptureSession::IsSupported().map_err(|error| {
        CaptureError::windows_graphics_capture_support_check_failed(error.to_string())
    })?;
    if !supported {
        return Err(CaptureError::windows_graphics_capture_unsupported());
    }

    if let CaptureTarget::Window { handle, .. } = target {
        let _item = create_window_capture_item(handle)?;
    }

    Err(CaptureError::windows_capture_not_implemented())
}

#[cfg(windows)]
fn create_window_capture_item(
    handle: isize,
) -> Result<windows::Graphics::Capture::GraphicsCaptureItem, CaptureError> {
    use windows::{
        Graphics::Capture::GraphicsCaptureItem,
        Win32::{Foundation::HWND, System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop},
        core::factory,
    };

    let hwnd = HWND(handle as *mut core::ffi::c_void);
    let interop = factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
        .map_err(|error| CaptureError::windows_capture_item_create_failed(error.to_string()))?;
    unsafe { interop.CreateForWindow(hwnd) }
        .map_err(|error| CaptureError::windows_capture_item_create_failed(error.to_string()))
}

#[cfg(not(windows))]
fn start_platform_capture(_target: CaptureTarget) -> Result<CaptureSession, CaptureError> {
    Err(CaptureError::unsupported_platform(std::env::consts::OS))
}
