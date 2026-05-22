use sdl2::{
    EventPump, Sdl,
    event::Event,
    keyboard::Mod,
    mouse::MouseButton as SdlMouseButton,
    pixels::PixelFormatEnum,
    render::{Canvas, TextureCreator},
    video::{FullscreenType, Window, WindowContext},
};
use wincast_protocol::input::{ButtonState, InputEvent, Modifiers, MouseButton};

use crate::{
    BgraPixelFrame, BgraPixelRenderer, LoadingStatus, PixelDimensions, RenderConfig, RenderError,
    RenderLoopAction, RenderLoopResult, map_window_point_to_frame_pixels,
    mouse_button_input_events,
};

pub struct SdlBgraPixelRenderer {
    _sdl: Sdl,
    canvas: Canvas<Window>,
    texture_creator: TextureCreator<WindowContext>,
    event_pump: EventPump,
    frame_dimensions: PixelDimensions,
}

impl SdlBgraPixelRenderer {
    pub fn new(config: RenderConfig) -> Result<Self, RenderError> {
        config.validate()?;
        let sdl = sdl2::init().map_err(RenderError::Backend)?;
        let video = sdl.video().map_err(RenderError::Backend)?;
        let mut window_builder = video.window(&config.title, config.width, config.height);
        window_builder.position_centered().resizable();
        let mut window = window_builder
            .build()
            .map_err(|error| RenderError::Backend(error.to_string()))?;
        if config.fullscreen {
            window
                .set_fullscreen(FullscreenType::Desktop)
                .map_err(RenderError::Backend)?;
        }
        let canvas = window
            .into_canvas()
            .accelerated()
            .present_vsync()
            .build()
            .map_err(|error| RenderError::Backend(error.to_string()))?;
        let texture_creator = canvas.texture_creator();
        let event_pump = sdl.event_pump().map_err(RenderError::Backend)?;

        Ok(Self {
            _sdl: sdl,
            canvas,
            texture_creator,
            event_pump,
            frame_dimensions: PixelDimensions {
                width: config.width,
                height: config.height,
            },
        })
    }

    fn window_dimensions(&self) -> PixelDimensions {
        let (width, height) = self.canvas.window().size();
        PixelDimensions { width, height }
    }
}

impl BgraPixelRenderer for SdlBgraPixelRenderer {
    fn render_loading(&mut self, status: &LoadingStatus) -> Result<(), RenderError> {
        status.validate()?;
        let window = self.window_dimensions();
        let width = window.width.max(1);
        let height = window.height.max(1);
        let bar_width = (width / 2).max(160);
        let bar_height = (height / 80).clamp(8, 18);
        let x = ((width - bar_width) / 2) as i32;
        let y = ((height / 2) + (height / 12)) as i32;
        let progress_width = ((status.tick % 100) as u32 * bar_width / 100).max(8);

        self.canvas
            .set_draw_color(sdl2::pixels::Color::RGB(12, 16, 20));
        self.canvas.clear();
        self.canvas
            .set_draw_color(sdl2::pixels::Color::RGB(54, 64, 76));
        self.canvas
            .fill_rect(sdl2::rect::Rect::new(x, y, bar_width, bar_height))
            .map_err(RenderError::Backend)?;
        self.canvas
            .set_draw_color(sdl2::pixels::Color::RGB(84, 180, 132));
        self.canvas
            .fill_rect(sdl2::rect::Rect::new(x, y, progress_width, bar_height))
            .map_err(RenderError::Backend)?;
        self.canvas.present();
        Ok(())
    }

    fn render_frame(&mut self, frame: &BgraPixelFrame) -> Result<(), RenderError> {
        frame.validate()?;
        let mut texture = self
            .texture_creator
            .create_texture_streaming(PixelFormatEnum::BGRA32, frame.width, frame.height)
            .map_err(|error| RenderError::Backend(error.to_string()))?;

        texture
            .update(None, &frame.bytes, frame.row_pitch as usize)
            .map_err(|error| RenderError::Backend(error.to_string()))?;
        self.canvas.clear();
        self.canvas
            .copy(&texture, None, None)
            .map_err(RenderError::Backend)?;
        self.canvas.present();
        self.frame_dimensions = PixelDimensions {
            width: frame.width,
            height: frame.height,
        };
        Ok(())
    }

    fn poll_input(&mut self) -> Result<RenderLoopResult, RenderError> {
        let mut input_events = Vec::new();
        let mut action = RenderLoopAction::Continue;
        let window_dimensions = self.window_dimensions();
        let frame_dimensions = self.frame_dimensions;

        for event in self.event_pump.poll_iter() {
            match event {
                Event::Quit { .. } => action = RenderLoopAction::Quit,
                Event::MouseMotion { x, y, .. } => {
                    let position =
                        map_window_point_to_frame_pixels(x, y, window_dimensions, frame_dimensions);
                    input_events.push(InputEvent::MouseMoveAbsolute {
                        x: position.x,
                        y: position.y,
                    });
                }
                Event::MouseButtonDown {
                    mouse_btn, x, y, ..
                } => {
                    if let Some(button) = map_mouse_button(mouse_btn) {
                        input_events.extend(mouse_button_input_events(
                            x,
                            y,
                            button,
                            ButtonState::Pressed,
                            window_dimensions,
                            frame_dimensions,
                        ));
                    }
                }
                Event::MouseButtonUp {
                    mouse_btn, x, y, ..
                } => {
                    if let Some(button) = map_mouse_button(mouse_btn) {
                        input_events.extend(mouse_button_input_events(
                            x,
                            y,
                            button,
                            ButtonState::Released,
                            window_dimensions,
                            frame_dimensions,
                        ));
                    }
                }
                Event::MouseWheel { x, y, .. } => input_events.push(InputEvent::MouseWheel {
                    delta_x: x,
                    delta_y: y,
                }),
                Event::KeyDown {
                    keycode,
                    keymod,
                    repeat,
                    ..
                } if !repeat => {
                    if let Some(code) = keycode.map(|code| code.into_i32() as u32) {
                        input_events.push(InputEvent::Key {
                            code,
                            state: ButtonState::Pressed,
                            modifiers: map_modifiers(keymod),
                        });
                    }
                }
                Event::KeyUp {
                    keycode, keymod, ..
                } => {
                    if let Some(code) = keycode.map(|code| code.into_i32() as u32) {
                        input_events.push(InputEvent::Key {
                            code,
                            state: ButtonState::Released,
                            modifiers: map_modifiers(keymod),
                        });
                    }
                }
                _ => {}
            }
        }

        Ok(RenderLoopResult {
            action,
            input_events,
        })
    }
}

fn map_mouse_button(button: SdlMouseButton) -> Option<MouseButton> {
    match button {
        SdlMouseButton::Left => Some(MouseButton::Left),
        SdlMouseButton::Right => Some(MouseButton::Right),
        SdlMouseButton::Middle => Some(MouseButton::Middle),
        _ => None,
    }
}

fn map_modifiers(modifiers: Mod) -> Modifiers {
    Modifiers {
        shift: modifiers.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD),
        ctrl: modifiers.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD),
        alt: modifiers.intersects(Mod::LALTMOD | Mod::RALTMOD),
        logo: modifiers.intersects(Mod::LGUIMOD | Mod::RGUIMOD),
    }
}
