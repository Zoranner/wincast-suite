pub mod input;

pub use input::{
    CaptureInputBounds, InputInjectionError, WindowsInputAction, WindowsInputEventSink,
    map_input_event_to_windows_actions,
};
