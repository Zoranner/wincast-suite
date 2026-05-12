mod error;
mod injector;
mod mapping;
mod sdl_virtual_key;
mod sink;
mod types;

pub use error::InputInjectionError;
pub use mapping::map_input_event_to_windows_actions;
pub use sink::WindowsInputEventSink;
pub use types::{CaptureInputBounds, WindowsInputAction};
