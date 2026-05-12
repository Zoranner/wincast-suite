use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CaptureError {
    #[error("桌面捕获尚未实现：当前稳定版仅接入 Windows 窗口捕获")]
    WindowsCaptureNotImplemented,
    #[error("当前 Windows 系统不支持 Windows Graphics Capture")]
    WindowsGraphicsCaptureUnsupported,
    #[error("检测 Windows Graphics Capture 支持状态失败: {0}")]
    WindowsGraphicsCaptureSupportCheckFailed(String),
    #[error("创建窗口捕获目标失败: {0}")]
    WindowsCaptureItemCreateFailed(String),
    #[error("初始化 Direct3D 捕获设备失败: {0}")]
    WindowsD3dInitializationFailed(String),
    #[error("创建 Windows 捕获会话失败: {0}")]
    WindowsCaptureSessionCreateFailed(String),
    #[error("启动 Windows 捕获会话失败: {0}")]
    WindowsCaptureSessionStartFailed(String),
    #[error("读取 Windows 捕获帧失败: {0}")]
    WindowsFrameReadFailed(String),
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

    pub fn windows_d3d_initialization_failed(error: impl Into<String>) -> Self {
        Self::WindowsD3dInitializationFailed(error.into())
    }

    pub fn windows_capture_session_create_failed(error: impl Into<String>) -> Self {
        Self::WindowsCaptureSessionCreateFailed(error.into())
    }

    pub fn windows_capture_session_start_failed(error: impl Into<String>) -> Self {
        Self::WindowsCaptureSessionStartFailed(error.into())
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
