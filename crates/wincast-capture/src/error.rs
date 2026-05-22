use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CaptureError {
    #[error("当前平台不支持桌面捕获")]
    WindowsCaptureNotImplemented,
    #[error("枚举 Windows 桌面复制输出失败: {0}")]
    WindowsDesktopOutputEnumerationFailed(String),
    #[error("初始化 Direct3D 捕获设备失败: {0}")]
    WindowsD3dInitializationFailed(String),
    #[error("创建 Windows 桌面复制会话失败: {0}")]
    WindowsCaptureSessionCreateFailed(String),
    #[error("读取 Windows 捕获帧失败: {0}")]
    WindowsFrameReadFailed(String),
    #[error("当前平台不支持画面捕获：仅 Windows 支持宿主端捕获，当前平台 {platform}")]
    UnsupportedPlatform { platform: String },
}

impl CaptureError {
    pub fn windows_capture_not_implemented() -> Self {
        Self::WindowsCaptureNotImplemented
    }

    pub fn windows_desktop_output_enumeration_failed(error: impl Into<String>) -> Self {
        Self::WindowsDesktopOutputEnumerationFailed(error.into())
    }

    pub fn windows_d3d_initialization_failed(error: impl Into<String>) -> Self {
        Self::WindowsD3dInitializationFailed(error.into())
    }

    pub fn windows_capture_session_create_failed(error: impl Into<String>) -> Self {
        Self::WindowsCaptureSessionCreateFailed(error.into())
    }

    pub fn windows_frame_read_failed(error: impl Into<String>) -> Self {
        Self::WindowsFrameReadFailed(error.into())
    }

    pub fn unsupported_platform(platform: impl Into<String>) -> Self {
        Self::UnsupportedPlatform {
            platform: platform.into(),
        }
    }
}
