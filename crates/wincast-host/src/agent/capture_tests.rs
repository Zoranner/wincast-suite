use std::{
    collections::VecDeque,
    net::{TcpListener, TcpStream},
    sync::atomic::Ordering,
    thread,
};

use crate::{
    agent::{
        capture::{capture_input_bounds, start_capture_session},
        listener::run_control_listener_once_with_runtime,
        tests::*,
    },
    window,
};
use wincast_capture::{CaptureError, CaptureTarget};
use wincast_input::CaptureInputBounds;
use wincast_protocol::{
    config::CaptureMode,
    frame::{read_message, write_message},
    handshake::send_client_hello,
    message::{ControlMessage, ErrorCode},
};

#[test]
fn host_reports_window_not_found_after_program_launch() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
    let endpoint = listener
        .local_addr()
        .expect("listener addr should be available");
    let mut config = host_config(endpoint.to_string());
    config.capture.startup_timeout_ms = 1;
    let mut runner = RecordingProgramRunner::default();
    let mut locator = FailingWindowLocator;
    let mut capture = RecordingCaptureStarter::default();
    let host = thread::spawn(move || {
        run_control_listener_once_with_runtime(
            listener,
            &config,
            &mut runner,
            &mut locator,
            &mut capture,
        )
    });

    let mut client = TcpStream::connect(endpoint).expect("client should connect");
    send_client_hello(&mut client).expect("client hello should write");
    read_message(&mut client).expect("host hello should read");
    write_message(&mut client, &ControlMessage::StartSession).expect("start session should write");

    assert_eq!(
        read_message(&mut client).expect("window error should read"),
        ControlMessage::Error {
            code: ErrorCode::WindowNotFound,
            message: "定位宿主端程序窗口失败: 未找到进程 42 的主窗口".to_owned(),
        }
    );

    let error = host
        .join()
        .expect("host thread should finish")
        .expect_err("host should report window lookup failure");
    assert!(error.contains("定位宿主端程序窗口失败"));
}

#[test]
fn host_reports_capture_failed_after_window_lookup() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
    let endpoint = listener
        .local_addr()
        .expect("listener addr should be available");
    let config = host_config(endpoint.to_string());
    let mut runner = RecordingProgramRunner::default();
    let mut locator = RecordingWindowLocator::default();
    let mut capture = FailingCaptureStarter;
    let host = thread::spawn(move || {
        run_control_listener_once_with_runtime(
            listener,
            &config,
            &mut runner,
            &mut locator,
            &mut capture,
        )
    });

    let mut client = TcpStream::connect(endpoint).expect("client should connect");
    send_client_hello(&mut client).expect("client hello should write");
    read_message(&mut client).expect("host hello should read");
    write_message(&mut client, &ControlMessage::StartSession).expect("start session should write");

    assert_eq!(
        read_message(&mut client).expect("capture error should read"),
        ControlMessage::Error {
            code: ErrorCode::CaptureFailed,
            message: "初始化画面捕获失败: 桌面捕获尚未实现：当前稳定版仅接入 Windows 窗口捕获"
                .to_owned(),
        }
    );

    let error = host
        .join()
        .expect("host thread should finish")
        .expect_err("host should report capture failure");
    assert!(error.contains("初始化画面捕获失败"));
}

#[test]
fn host_treats_missing_initial_frame_as_waitable_state() {
    let config = host_config("127.0.0.1:0".to_owned());
    let window = window_candidate();
    let mut capture = RecordingCaptureStarter {
        frames: VecDeque::from([None, Some(captured_bgra_frame())]),
        ..Default::default()
    };
    let attempts = capture.attempts.clone();

    let (_session, frame) = start_capture_session(&config, &window, &mut capture)
        .expect("host should wait until first frame metadata is available");

    assert_eq!(capture.targets, vec![CaptureTarget::Desktop]);
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    assert_eq!(frame.row_pitch, 5120);
    assert_eq!(frame.bytes.len(), 5120 * 720);
}

#[test]
fn capture_input_bounds_keep_window_origin_for_window_capture() {
    let mut config = host_config("127.0.0.1:0".to_owned());
    config.capture.mode = CaptureMode::Window;
    let mut window = window_candidate();
    window.rect = window::WindowRect {
        left: 50,
        top: 75,
        right: 1330,
        bottom: 795,
    };
    let frame = captured_bgra_frame();

    let bounds = capture_input_bounds(&config, &window, &frame);

    assert_eq!(
        bounds,
        CaptureInputBounds {
            origin_x: 50,
            origin_y: 75,
            width: 1280,
            height: 720,
        }
    );
}

#[test]
fn capture_input_bounds_use_frame_size_for_desktop_capture() {
    let config = host_config("127.0.0.1:0".to_owned());
    let window = window_candidate();
    let frame = captured_bgra_frame();

    let bounds = capture_input_bounds(&config, &window, &frame);

    assert_eq!(
        bounds,
        CaptureInputBounds {
            origin_x: 0,
            origin_y: 0,
            width: 1280,
            height: 720,
        }
    );
}

#[test]
fn host_reports_capture_failed_when_initial_frame_times_out() {
    let mut config = host_config("127.0.0.1:0".to_owned());
    config.capture.startup_timeout_ms = 1;
    let window = window_candidate();
    let mut capture = RecordingCaptureStarter {
        frames: VecDeque::from([None, None]),
        ..Default::default()
    };

    let error = match start_capture_session(&config, &window, &mut capture) {
        Ok(_) => panic!("host should fail when no frame metadata arrives before timeout"),
        Err(error) => error,
    };

    assert_eq!(
        error,
        CaptureError::windows_frame_read_failed("等待 Windows 捕获首帧超时")
    );
}
