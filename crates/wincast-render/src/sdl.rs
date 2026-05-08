use sdl2::{
    EventPump, Sdl,
    event::Event,
    keyboard::Mod,
    mouse::MouseButton as SdlMouseButton,
    pixels::PixelFormatEnum,
    render::{Canvas, TextureCreator},
    video::{Window, WindowContext},
};
use wincast_protocol::{
    input::{ButtonState, InputEvent, Modifiers, MouseButton},
    raw_frame::RawBgraFrame,
};

use crate::{RawBgraRenderer, RenderConfig, RenderError, RenderLoopAction, RenderLoopResult};

pub struct SdlRawBgraRenderer {
    _sdl: Sdl,
    canvas: Canvas<Window>,
    texture_creator: TextureCreator<WindowContext>,
    event_pump: EventPump,
}

impl SdlRawBgraRenderer {
    pub fn new(config: RenderConfig) -> Result<Self, RenderError> {
        config.validate()?;
        let sdl = sdl2::init().map_err(RenderError::Backend)?;
        let video = sdl.video().map_err(RenderError::Backend)?;
        let window = video
            .window(&config.title, config.width, config.height)
            .position_centered()
            .resizable()
            .build()
            .map_err(|error| RenderError::Backend(error.to_string()))?;
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
        })
    }
}

impl RawBgraRenderer for SdlRawBgraRenderer {
    fn render_frame(&mut self, frame: &RawBgraFrame) -> Result<(), RenderError> {
        frame
            .validate()
            .map_err(|error| RenderError::InvalidFrame(error.to_string()))?;
        let mut texture = self
            .texture_creator
            .create_texture_streaming(PixelFormatEnum::BGRA8888, frame.width, frame.height)
            .map_err(|error| RenderError::Backend(error.to_string()))?;

        texture
            .update(None, &frame.bytes, frame.row_pitch as usize)
            .map_err(|error| RenderError::Backend(error.to_string()))?;
        self.canvas.clear();
        self.canvas
            .copy(&texture, None, None)
            .map_err(RenderError::Backend)?;
        self.canvas.present();
        Ok(())
    }

    fn poll_input(&mut self) -> Result<RenderLoopResult, RenderError> {
        let mut input_events = Vec::new();
        let mut action = RenderLoopAction::Continue;

        for event in self.event_pump.poll_iter() {
            match event {
                Event::Quit { .. } => action = RenderLoopAction::Quit,
                Event::MouseMotion { x, y, .. } => input_events.push(InputEvent::MouseMove {
                    x: x as f32,
                    y: y as f32,
                }),
                Event::MouseButtonDown { mouse_btn, .. } => {
                    if let Some(button) = map_mouse_button(mouse_btn) {
                        input_events.push(InputEvent::MouseButton {
                            button,
                            state: ButtonState::Pressed,
                        });
                    }
                }
                Event::MouseButtonUp { mouse_btn, .. } => {
                    if let Some(button) = map_mouse_button(mouse_btn) {
                        input_events.push(InputEvent::MouseButton {
                            button,
                            state: ButtonState::Released,
                        });
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
                    if let Some(code) = keycode.map(|code| code as i32 as u32) {
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
                    if let Some(code) = keycode.map(|code| code as i32 as u32) {
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
