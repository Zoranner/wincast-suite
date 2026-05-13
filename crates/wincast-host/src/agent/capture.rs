use std::{
    thread,
    time::{Duration, Instant},
};

use crate::{
    program::StartedProgram,
    window::{WindowCandidate, WindowLookupError, find_main_window},
};
use wincast_capture::{
    CaptureError, CaptureSession, CaptureTarget, CapturedBgraFrame, wait_next_capture_result_with,
};
use wincast_input::{CaptureInputBounds, WindowsInputEventSink};
use wincast_protocol::{
    config::{CaptureMode, HostConfig},
    input::InputEvent,
};

pub(super) trait WindowLocator {
    fn find_main_window(
        &mut self,
        process_id: u32,
        title_contains: Option<&str>,
    ) -> Result<WindowCandidate, WindowLookupError>;
}

pub(super) struct WindowsWindowLocator;

impl WindowLocator for WindowsWindowLocator {
    fn find_main_window(
        &mut self,
        process_id: u32,
        title_contains: Option<&str>,
    ) -> Result<WindowCandidate, WindowLookupError> {
        find_main_window(process_id, title_contains)
    }
}

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

pub(super) fn locate_started_window(
    config: &HostConfig,
    started: &StartedProgram,
    locator: &mut impl WindowLocator,
) -> Result<WindowCandidate, WindowLookupError> {
    let deadline = Instant::now() + Duration::from_millis(config.capture.startup_timeout_ms);
    let title_contains = Some(config.capture.window_title_contains.as_str());

    loop {
        let last_error = match locator.find_main_window(started.process_id, title_contains) {
            Ok(window) => return Ok(window),
            Err(error) => error,
        };

        if Instant::now() >= deadline {
            return Err(last_error);
        }

        thread::sleep(Duration::from_millis(100));
    }
}

pub(super) fn start_capture_session(
    config: &HostConfig,
    window: &WindowCandidate,
    capture: &mut impl CaptureStarter,
) -> Result<(Box<dyn CaptureRuntime>, CapturedBgraFrame), CaptureError> {
    let mut session = capture.start_capture(capture_target(config, window))?;
    let first_frame = wait_next_capture_result_with(
        Duration::from_millis(config.capture.startup_timeout_ms),
        || session.try_next_bgra_frame(),
    )?;
    Ok((session, first_frame))
}

fn capture_target(config: &HostConfig, window: &WindowCandidate) -> CaptureTarget {
    match config.capture.mode {
        CaptureMode::Desktop => CaptureTarget::Desktop {
            source_window_handle: window.handle,
        },
        CaptureMode::Window => CaptureTarget::Window {
            handle: window.handle,
            width: window.rect.width() as u32,
            height: window.rect.height() as u32,
            title: (!window.title.is_empty()).then_some(window.title.clone()),
        },
    }
}

pub(super) fn capture_input_bounds(
    config: &HostConfig,
    window: &WindowCandidate,
    frame: &CapturedBgraFrame,
) -> CaptureInputBounds {
    match config.capture.mode {
        CaptureMode::Desktop => CaptureInputBounds {
            origin_x: window.monitor_rect.left,
            origin_y: window.monitor_rect.top,
            width: frame.metadata.frame.width,
            height: frame.metadata.frame.height,
        },
        CaptureMode::Window => CaptureInputBounds {
            origin_x: window.rect.left,
            origin_y: window.rect.top,
            width: frame.metadata.frame.width,
            height: frame.metadata.frame.height,
        },
    }
}
