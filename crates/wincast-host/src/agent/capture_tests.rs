use std::{collections::VecDeque, sync::atomic::Ordering};

use crate::agent::{
    capture::{screen_input_bounds, start_screen_capture_session},
    tests::*,
};
use wincast_capture::CaptureError;
use wincast_input::CaptureInputBounds;

#[test]
fn screen_capture_waits_until_first_frame_is_available() {
    let config = host_config("127.0.0.1:0".to_owned());
    let mut capture = RecordingCaptureStarter {
        frames: VecDeque::from([None, Some(captured_bgra_frame())]),
        ..Default::default()
    };
    let attempts = capture.attempts.clone();

    let (_session, frame) = start_screen_capture_session(&config, &mut capture)
        .expect("host should wait until first screen frame is available");

    assert_eq!(capture.targets, vec![screen_capture_target()]);
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    assert_eq!(frame.row_pitch, 5120);
    assert_eq!(frame.bytes.len(), 5120 * 720);
}

#[test]
fn screen_input_bounds_use_capture_frame_dimensions() {
    let frame = captured_bgra_frame_with_dimensions(1920, 1080);

    let bounds = screen_input_bounds(&frame);

    assert_eq!(
        bounds,
        CaptureInputBounds {
            origin_x: 0,
            origin_y: 0,
            width: 1920,
            height: 1080,
        }
    );
}

#[test]
fn host_reports_capture_failed_when_initial_frame_times_out() {
    let mut config = host_config("127.0.0.1:0".to_owned());
    config.capture.first_frame_timeout_ms = 1;
    let mut capture = RecordingCaptureStarter {
        frames: VecDeque::from([None, None]),
        ..Default::default()
    };

    let error = match start_screen_capture_session(&config, &mut capture) {
        Ok(_) => panic!("host should fail when no screen frame arrives before timeout"),
        Err(error) => error,
    };

    assert_eq!(
        error,
        CaptureError::windows_frame_read_failed("等待 Windows 捕获首帧超时")
    );
}
