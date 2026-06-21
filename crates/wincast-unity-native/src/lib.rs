mod config;
mod error;
mod ffi;
mod frame;
mod input;
mod runtime;
mod session;

pub use ffi::{
    wincast_unity_create, wincast_unity_get_last_error, wincast_unity_get_status,
    wincast_unity_poll_input, wincast_unity_shutdown, wincast_unity_start,
    wincast_unity_submit_frame,
};
pub use frame::{FrameMetadata, RuntimeSnapshot, WincastUnityFrameFormat};
pub use input::{WincastUnityInputEvent, WincastUnityInputEventType, WincastUnityPointerButton};
pub use runtime::{WincastUnityStatus, inject_input_event_for_test, runtime_snapshot_for_test};
