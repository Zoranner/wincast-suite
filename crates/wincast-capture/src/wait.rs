use std::{
    thread,
    time::{Duration, Instant},
};

use crate::{error::CaptureError, model::CapturedFrame};

pub fn wait_next_frame_metadata_with(
    timeout: Duration,
    mut try_next_frame: impl FnMut() -> Result<Option<CapturedFrame>, CaptureError>,
) -> Result<CapturedFrame, CaptureError> {
    wait_next_capture_result_with(timeout, &mut try_next_frame)
}

pub fn wait_next_capture_result_with<T>(
    timeout: Duration,
    mut try_next_frame: impl FnMut() -> Result<Option<T>, CaptureError>,
) -> Result<T, CaptureError> {
    let deadline = Instant::now() + timeout;

    loop {
        if let Some(frame) = try_next_frame()? {
            return Ok(frame);
        }

        if Instant::now() >= deadline {
            return Err(CaptureError::windows_frame_read_failed(
                "等待 Windows 捕获首帧超时",
            ));
        }

        thread::sleep(Duration::from_millis(16));
    }
}
