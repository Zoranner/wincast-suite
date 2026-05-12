use std::{
    collections::VecDeque,
    net::{TcpListener, TcpStream},
    sync::atomic::Ordering,
    thread,
};

use crate::agent::{listener::run_control_listener_once_with_runtime, tests::*};
use wincast_capture::CaptureTarget;
use wincast_protocol::{
    frame::{read_message, write_message},
    handshake::send_client_hello,
    message::ControlMessage,
    raw_frame::read_raw_bgra_frame,
};

#[test]
fn host_can_send_first_raw_binary_frame_after_session_ready() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
    let endpoint = listener
        .local_addr()
        .expect("listener addr should be available");
    let config = host_config(endpoint.to_string());
    let mut runner = RecordingProgramRunner::default();
    let mut locator = RecordingWindowLocator::default();
    let mut capture = RecordingCaptureStarter::default();
    let host = thread::spawn(move || {
        let result = run_control_listener_once_with_runtime(
            listener,
            &config,
            &mut runner,
            &mut locator,
            &mut capture,
        );
        (result, runner.launched, locator.lookups, capture.targets)
    });

    let mut client = TcpStream::connect(endpoint).expect("client should connect");
    send_client_hello(&mut client).expect("client hello should write");
    read_message(&mut client).expect("host hello should read");
    write_message(&mut client, &ControlMessage::StartSession).expect("start session should write");

    assert_eq!(
        read_message(&mut client).expect("session ready should read"),
        ControlMessage::SessionReady {
            width: 1280,
            height: 720,
        }
    );
    assert_eq!(
        read_message(&mut client).expect("video ready should read"),
        ControlMessage::VideoReady
    );
    let frame = read_raw_bgra_frame(&mut client).expect("raw binary frame should read");
    assert_eq!(frame.width, 1280);
    assert_eq!(frame.height, 720);
    assert_eq!(frame.row_pitch, 5120);
    assert_eq!(frame.sequence_number, 0);
    assert_eq!(frame.timestamp_ns, 0);
    assert_eq!(frame.bytes.len(), 5120 * 720);

    let (host_result, launched, lookups, capture_targets) =
        host.join().expect("host thread should finish");
    assert_eq!(
        host_result.expect("host should handle one client"),
        endpoint
    );
    assert_eq!(
        launched,
        vec![("C:\\Program Files\\SomeApp\\app.exe".to_owned(), Vec::new())]
    );
    assert_eq!(lookups, vec![(42, None)]);
    assert_eq!(capture_targets, vec![CaptureTarget::Desktop]);
}

#[test]
fn host_streams_available_raw_binary_frames_after_first_frame() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
    let endpoint = listener
        .local_addr()
        .expect("listener addr should be available");
    let config = host_config(endpoint.to_string());
    let mut runner = RecordingProgramRunner::default();
    let mut locator = RecordingWindowLocator::default();
    let mut capture = RecordingCaptureStarter {
        frames: VecDeque::from([
            Some(captured_bgra_frame_with_sequence(0)),
            Some(captured_bgra_frame_with_sequence(1)),
            Some(captured_bgra_frame_with_sequence(2)),
            None,
        ]),
        ..Default::default()
    };
    let attempts = capture.attempts.clone();
    let host = thread::spawn(move || {
        let result = run_control_listener_once_with_runtime(
            listener,
            &config,
            &mut runner,
            &mut locator,
            &mut capture,
        );
        (result, runner.launched, locator.lookups, capture.targets)
    });

    let mut client = TcpStream::connect(endpoint).expect("client should connect");
    send_client_hello(&mut client).expect("client hello should write");
    read_message(&mut client).expect("host hello should read");
    write_message(&mut client, &ControlMessage::StartSession).expect("start session should write");

    assert_eq!(
        read_message(&mut client).expect("session ready should read"),
        ControlMessage::SessionReady {
            width: 1280,
            height: 720,
        }
    );
    assert_eq!(
        read_message(&mut client).expect("video ready should read"),
        ControlMessage::VideoReady
    );
    let first = read_raw_bgra_frame(&mut client).expect("first raw frame should read");
    let second = read_raw_bgra_frame(&mut client).expect("second raw frame should read");
    let third = read_raw_bgra_frame(&mut client).expect("third raw frame should read");
    assert_eq!(first.sequence_number, 0);
    assert_eq!(second.sequence_number, 1);
    assert_eq!(third.sequence_number, 2);

    let (host_result, launched, lookups, capture_targets) =
        host.join().expect("host thread should finish");
    assert_eq!(
        host_result.expect("host should handle one client"),
        endpoint
    );
    assert_eq!(attempts.load(Ordering::SeqCst), 4);
    assert_eq!(
        launched,
        vec![("C:\\Program Files\\SomeApp\\app.exe".to_owned(), Vec::new())]
    );
    assert_eq!(lookups, vec![(42, None)]);
    assert_eq!(capture_targets, vec![CaptureTarget::Desktop]);
}

#[test]
fn host_keeps_raw_stream_alive_when_no_frame_is_temporarily_available() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
    let endpoint = listener
        .local_addr()
        .expect("listener addr should be available");
    let config = host_config(endpoint.to_string());
    let mut runner = RecordingProgramRunner::default();
    let mut locator = RecordingWindowLocator::default();
    let mut capture = RecordingCaptureStarter {
        frames: VecDeque::from([
            Some(captured_bgra_frame_with_sequence(0)),
            None,
            Some(captured_bgra_frame_with_sequence(1)),
            None,
        ]),
        ..Default::default()
    };
    let host = thread::spawn(move || {
        let result = run_control_listener_once_with_runtime(
            listener,
            &config,
            &mut runner,
            &mut locator,
            &mut capture,
        );
        (result, runner.launched, locator.lookups, capture.targets)
    });

    let mut client = TcpStream::connect(endpoint).expect("client should connect");
    send_client_hello(&mut client).expect("client hello should write");
    read_message(&mut client).expect("host hello should read");
    write_message(&mut client, &ControlMessage::StartSession).expect("start session should write");

    read_message(&mut client).expect("session ready should read");
    read_message(&mut client).expect("video ready should read");
    let first = read_raw_bgra_frame(&mut client).expect("first raw frame should read");
    let second = read_raw_bgra_frame(&mut client).expect("second raw frame should read");

    assert_eq!(first.sequence_number, 0);
    assert_eq!(second.sequence_number, 1);

    let (host_result, launched, lookups, capture_targets) =
        host.join().expect("host thread should finish");
    assert_eq!(
        host_result.expect("host should handle one client"),
        endpoint
    );
    assert_eq!(
        launched,
        vec![("C:\\Program Files\\SomeApp\\app.exe".to_owned(), Vec::new())]
    );
    assert_eq!(lookups, vec![(42, None)]);
    assert_eq!(capture_targets, vec![CaptureTarget::Desktop]);
}
