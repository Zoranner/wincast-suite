use std::fmt;

use wincast_protocol::{input::InputEvent, raw_frame::RawBgraFrame};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderConfig {
    pub title: String,
    pub width: u32,
    pub height: u32,
}

impl RenderConfig {
    pub fn validate(&self) -> Result<(), RenderError> {
        if self.title.trim().is_empty() {
            return Err(RenderError::InvalidConfig("窗口标题不能为空".to_owned()));
        }
        if self.width == 0 || self.height == 0 {
            return Err(RenderError::InvalidConfig("窗口尺寸必须大于 0".to_owned()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderLoopAction {
    Continue,
    Quit,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RenderLoopResult {
    pub action: RenderLoopAction,
    pub input_events: Vec<InputEvent>,
}

pub trait RawBgraRenderer {
    fn render_frame(&mut self, frame: &RawBgraFrame) -> Result<(), RenderError>;

    fn poll_input(&mut self) -> Result<RenderLoopResult, RenderError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderError {
    InvalidConfig(String),
    InvalidFrame(String),
    Backend(String),
    UnsupportedPlatform { platform: &'static str },
}

impl fmt::Display for RenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(message) => write!(formatter, "渲染配置无效: {message}"),
            Self::InvalidFrame(message) => write!(formatter, "raw BGRA 渲染帧无效: {message}"),
            Self::Backend(message) => write!(formatter, "SDL2 渲染后端失败: {message}"),
            Self::UnsupportedPlatform { platform } => {
                write!(formatter, "当前平台不支持 SDL2 客户端窗口: {platform}")
            }
        }
    }
}

impl std::error::Error for RenderError {}

#[cfg(target_os = "linux")]
mod sdl;

#[cfg(target_os = "linux")]
pub use sdl::SdlRawBgraRenderer;

#[cfg(not(target_os = "linux"))]
#[derive(Debug)]
pub struct SdlRawBgraRenderer;

#[cfg(not(target_os = "linux"))]
impl SdlRawBgraRenderer {
    pub fn new(_config: RenderConfig) -> Result<Self, RenderError> {
        Err(RenderError::UnsupportedPlatform {
            platform: std::env::consts::OS,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_config_rejects_empty_title() {
        let config = RenderConfig {
            title: "  ".to_owned(),
            width: 800,
            height: 600,
        };

        assert_eq!(
            config.validate(),
            Err(RenderError::InvalidConfig("窗口标题不能为空".to_owned()))
        );
    }

    #[test]
    fn render_config_rejects_zero_dimensions() {
        let config = RenderConfig {
            title: "WinCast".to_owned(),
            width: 0,
            height: 600,
        };

        assert_eq!(
            config.validate(),
            Err(RenderError::InvalidConfig("窗口尺寸必须大于 0".to_owned()))
        );
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn sdl_renderer_reports_unsupported_platform_outside_linux() {
        let error = SdlRawBgraRenderer::new(RenderConfig {
            title: "WinCast".to_owned(),
            width: 800,
            height: 600,
        })
        .expect_err("non-Linux platform should not construct SDL renderer");

        assert_eq!(
            error,
            RenderError::UnsupportedPlatform {
                platform: std::env::consts::OS,
            }
        );
    }
}
