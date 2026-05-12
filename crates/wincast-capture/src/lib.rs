mod error;
mod model;
mod session;
mod wait;

#[cfg(windows)]
mod windows_impl;

pub use error::CaptureError;
pub use model::{
    CaptureTarget, CapturedBgraFrame, CapturedFrame, CapturedTextureMetadata, FramePixelFormat,
};
pub use session::CaptureSession;
pub use wait::{wait_next_capture_result_with, wait_next_frame_metadata_with};
