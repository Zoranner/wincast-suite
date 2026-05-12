use std::time::Duration;

use crate::{
    error::CaptureError,
    model::{CaptureTarget, CapturedBgraFrame, CapturedFrame, CapturedTextureMetadata},
    wait_next_capture_result_with,
};

#[derive(Debug)]
pub struct CaptureSession {
    target: CaptureTarget,
    #[cfg(windows)]
    state: crate::windows_impl::WindowsCaptureState,
}

impl CaptureSession {
    pub fn start(target: CaptureTarget) -> Result<Self, CaptureError> {
        start_platform_capture(target)
    }

    pub fn target(&self) -> &CaptureTarget {
        &self.target
    }

    pub fn is_active(&self) -> bool {
        #[cfg(windows)]
        {
            self.state.is_active()
        }
        #[cfg(not(windows))]
        {
            false
        }
    }

    pub fn try_next_frame_metadata(&mut self) -> Result<Option<CapturedFrame>, CaptureError> {
        #[cfg(windows)]
        {
            self.state.try_next_frame_metadata()
        }
        #[cfg(not(windows))]
        {
            Ok(None)
        }
    }

    pub fn wait_next_frame_metadata(
        &mut self,
        timeout: Duration,
    ) -> Result<CapturedFrame, CaptureError> {
        wait_next_capture_result_with(timeout, || self.try_next_frame_metadata())
    }

    pub fn try_next_texture_metadata(
        &mut self,
    ) -> Result<Option<CapturedTextureMetadata>, CaptureError> {
        #[cfg(windows)]
        {
            self.state.try_next_texture_metadata()
        }
        #[cfg(not(windows))]
        {
            Ok(None)
        }
    }

    pub fn try_next_bgra_frame(&mut self) -> Result<Option<CapturedBgraFrame>, CaptureError> {
        #[cfg(windows)]
        {
            self.state.try_next_bgra_frame()
        }
        #[cfg(not(windows))]
        {
            Ok(None)
        }
    }
}

#[cfg(windows)]
fn start_platform_capture(target: CaptureTarget) -> Result<CaptureSession, CaptureError> {
    let state = crate::windows_impl::start_windows_capture(&target)?;

    Ok(CaptureSession { target, state })
}

#[cfg(not(windows))]
fn start_platform_capture(_target: CaptureTarget) -> Result<CaptureSession, CaptureError> {
    Err(CaptureError::unsupported_platform(std::env::consts::OS))
}
