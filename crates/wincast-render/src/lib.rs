use std::fmt;

use wincast_protocol::input::InputEvent;

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

#[cfg(any(test, target_os = "linux"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PixelDimensions {
    pub width: u32,
    pub height: u32,
}

#[cfg(any(test, target_os = "linux"))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FrameMousePosition {
    pub x: f32,
    pub y: f32,
}

#[cfg(any(test, target_os = "linux"))]
pub(crate) fn map_window_point_to_frame_pixels(
    x: i32,
    y: i32,
    window: PixelDimensions,
    frame: PixelDimensions,
) -> FrameMousePosition {
    FrameMousePosition {
        x: map_window_axis_to_frame_axis(x, window.width, frame.width),
        y: map_window_axis_to_frame_axis(y, window.height, frame.height),
    }
}

#[cfg(any(test, target_os = "linux"))]
pub(crate) fn mouse_button_input_events(
    x: i32,
    y: i32,
    button: wincast_protocol::input::MouseButton,
    state: wincast_protocol::input::ButtonState,
    window: PixelDimensions,
    frame: PixelDimensions,
) -> [InputEvent; 2] {
    let position = map_window_point_to_frame_pixels(x, y, window, frame);

    [
        InputEvent::MouseMoveAbsolute {
            x: position.x,
            y: position.y,
        },
        InputEvent::MouseButton { button, state },
    ]
}

#[cfg(any(test, target_os = "linux"))]
fn map_window_axis_to_frame_axis(coordinate: i32, window_span: u32, frame_span: u32) -> f32 {
    if frame_span <= 1 {
        return 0.0;
    }

    if window_span <= 1 {
        return if coordinate <= 0 {
            0.0
        } else {
            frame_span.saturating_sub(1) as f32
        };
    }

    let mapped = coordinate as f32 * frame_span.saturating_sub(1) as f32
        / window_span.saturating_sub(1) as f32;
    mapped
        .round()
        .clamp(0.0, frame_span.saturating_sub(1) as f32)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderLoopAction {
    Continue,
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BgraPixelFrame {
    pub width: u32,
    pub height: u32,
    pub row_pitch: u32,
    pub sequence_number: u64,
    pub timestamp_ns: u64,
    pub bytes: Vec<u8>,
}

impl BgraPixelFrame {
    pub fn validate(&self) -> Result<(), RenderError> {
        if self.width == 0 || self.height == 0 {
            return Err(RenderError::InvalidFrame(
                "BGRA 像素帧尺寸必须大于 0".to_owned(),
            ));
        }

        let min_row_pitch = self
            .width
            .checked_mul(4)
            .ok_or_else(|| RenderError::InvalidFrame("BGRA 像素帧尺寸计算溢出".to_owned()))?;
        if self.row_pitch < min_row_pitch {
            return Err(RenderError::InvalidFrame(
                "BGRA 像素帧 row pitch 小于像素宽度".to_owned(),
            ));
        }

        let expected = self
            .row_pitch
            .checked_mul(self.height)
            .ok_or_else(|| RenderError::InvalidFrame("BGRA 像素帧尺寸计算溢出".to_owned()))?
            as usize;
        if self.bytes.len() != expected {
            return Err(RenderError::InvalidFrame(format!(
                "BGRA 像素帧载荷长度 {} 与期望 {expected} 不一致",
                self.bytes.len()
            )));
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RenderLoopResult {
    pub action: RenderLoopAction,
    pub input_events: Vec<InputEvent>,
}

pub trait BgraPixelRenderer {
    fn render_frame(&mut self, frame: &BgraPixelFrame) -> Result<(), RenderError>;

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
            Self::InvalidFrame(message) => write!(formatter, "BGRA 像素帧无效: {message}"),
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
pub use sdl::SdlBgraPixelRenderer;

#[cfg(not(target_os = "linux"))]
#[derive(Debug)]
pub struct SdlBgraPixelRenderer;

#[cfg(not(target_os = "linux"))]
impl SdlBgraPixelRenderer {
    pub fn new(_config: RenderConfig) -> Result<Self, RenderError> {
        Err(RenderError::UnsupportedPlatform {
            platform: std::env::consts::OS,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wincast_protocol::input::{ButtonState, MouseButton};

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

    #[test]
    fn bgra_pixel_frame_rejects_invalid_payload_shape() {
        let frame = BgraPixelFrame {
            width: 2,
            height: 2,
            row_pitch: 8,
            sequence_number: 1,
            timestamp_ns: 10,
            bytes: vec![0; 15],
        };

        assert_eq!(
            frame.validate(),
            Err(RenderError::InvalidFrame(
                "BGRA 像素帧载荷长度 15 与期望 16 不一致".to_owned()
            ))
        );
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn sdl_renderer_reports_unsupported_platform_outside_linux() {
        let error = SdlBgraPixelRenderer::new(RenderConfig {
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

    #[test]
    fn maps_window_mouse_coordinates_to_remote_frame_pixels() {
        let position = map_window_point_to_frame_pixels(
            1280,
            720,
            PixelDimensions {
                width: 2560,
                height: 1440,
            },
            PixelDimensions {
                width: 1280,
                height: 720,
            },
        );

        assert_eq!(position, FrameMousePosition { x: 640.0, y: 360.0 });
    }

    #[test]
    fn maps_bottom_right_window_pixel_to_bottom_right_frame_pixel() {
        let position = map_window_point_to_frame_pixels(
            1279,
            719,
            PixelDimensions {
                width: 1280,
                height: 720,
            },
            PixelDimensions {
                width: 2560,
                height: 1440,
            },
        );

        assert_eq!(
            position,
            FrameMousePosition {
                x: 2559.0,
                y: 1439.0,
            }
        );
    }

    #[test]
    fn clamps_window_mouse_coordinates_inside_remote_frame() {
        let position = map_window_point_to_frame_pixels(
            3000,
            -20,
            PixelDimensions {
                width: 2560,
                height: 1440,
            },
            PixelDimensions {
                width: 1280,
                height: 720,
            },
        );

        assert_eq!(position, FrameMousePosition { x: 1279.0, y: 0.0 });
    }

    #[test]
    fn mouse_button_input_events_move_then_click_at_mapped_position() {
        let events = mouse_button_input_events(
            1280,
            720,
            MouseButton::Left,
            ButtonState::Pressed,
            PixelDimensions {
                width: 2560,
                height: 1440,
            },
            PixelDimensions {
                width: 1280,
                height: 720,
            },
        );

        assert_eq!(
            events,
            [
                InputEvent::MouseMoveAbsolute { x: 640.0, y: 360.0 },
                InputEvent::MouseButton {
                    button: MouseButton::Left,
                    state: ButtonState::Pressed,
                },
            ]
        );
    }

    #[test]
    fn mouse_button_input_events_move_then_release_at_mapped_position() {
        let events = mouse_button_input_events(
            1279,
            719,
            MouseButton::Right,
            ButtonState::Released,
            PixelDimensions {
                width: 1280,
                height: 720,
            },
            PixelDimensions {
                width: 2560,
                height: 1440,
            },
        );

        assert_eq!(
            events,
            [
                InputEvent::MouseMoveAbsolute {
                    x: 2559.0,
                    y: 1439.0,
                },
                InputEvent::MouseButton {
                    button: MouseButton::Right,
                    state: ButtonState::Released,
                },
            ]
        );
    }
}
