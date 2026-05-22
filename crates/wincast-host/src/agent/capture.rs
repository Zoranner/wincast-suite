use std::time::Duration;

use wincast_capture::{
    CaptureError, CaptureSession, CaptureTarget, CapturedBgraFrame, wait_next_capture_result_with,
};
use wincast_input::{CaptureInputBounds, WindowsInputEventSink};
use wincast_protocol::{config::HostConfig, input::InputEvent};

pub(super) trait CaptureStarter {
    fn start_capture(
        &mut self,
        target: CaptureTarget,
    ) -> Result<Box<dyn CaptureRuntime>, CaptureError>;
}

pub(super) trait CaptureRuntime {
    fn is_active(&self) -> bool;

    fn try_next_bgra_frame(&mut self) -> Result<Option<CapturedBgraFrame>, CaptureError>;
}

pub(super) trait InputEventSink {
    fn handle_input_event(&mut self, event: InputEvent) -> Result<(), String>;
}

impl InputEventSink for WindowsInputEventSink {
    fn handle_input_event(&mut self, event: InputEvent) -> Result<(), String> {
        self.inject(event).map_err(|error| error.to_string())
    }
}

pub(super) struct StdCaptureStarter;

impl CaptureStarter for StdCaptureStarter {
    fn start_capture(
        &mut self,
        target: CaptureTarget,
    ) -> Result<Box<dyn CaptureRuntime>, CaptureError> {
        Ok(Box::new(CaptureSession::start(target)?))
    }
}

impl CaptureRuntime for CaptureSession {
    fn is_active(&self) -> bool {
        self.is_active()
    }

    fn try_next_bgra_frame(&mut self) -> Result<Option<CapturedBgraFrame>, CaptureError> {
        self.try_next_bgra_frame()
    }
}

pub(super) fn start_screen_capture_session(
    config: &HostConfig,
    capture: &mut impl CaptureStarter,
) -> Result<(Box<dyn CaptureRuntime>, CapturedBgraFrame), CaptureError> {
    let mut session = capture.start_capture(CaptureTarget::Screen)?;
    let first_frame = wait_next_capture_result_with(
        Duration::from_millis(config.capture.first_frame_timeout_ms),
        || session.try_next_bgra_frame(),
    )?;
    Ok((session, first_frame))
}

pub(super) fn screen_input_bounds(frame: &CapturedBgraFrame) -> CaptureInputBounds {
    CaptureInputBounds {
        origin_x: 0,
        origin_y: 0,
        width: frame.metadata.frame.width,
        height: frame.metadata.frame.height,
    }
}
