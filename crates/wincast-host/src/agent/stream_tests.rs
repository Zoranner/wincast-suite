use std::{
    net::{TcpListener, TcpStream},
    thread,
};

use crate::agent::{
    stream::{InputReaderEvent, spawn_input_event_reader},
    tests::*,
};
use wincast_input::CaptureInputBounds;
use wincast_protocol::{
    frame::{read_message, write_message},
    message::ControlMessage,
};

#[test]
fn input_event_reader_owner_can_join_reader_thread_result() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
    let endpoint = listener
        .local_addr()
        .expect("listener addr should be available");
    let mut client = TcpStream::connect(endpoint).expect("client should connect");
    let (server, _) = listener.accept().expect("server should accept");

    let input_reader = spawn_input_event_reader(
        server,
        CaptureInputBounds {
            origin_x: 0,
            origin_y: 0,
            width: 1280,
            height: 720,
        },
    );
    write_message(&mut client, &ControlMessage::StopSession).expect("stop session should write");

    assert_eq!(
        input_reader.join().expect("input reader should join"),
        Some(InputReaderEvent::StopSession)
    );
}

#[test]
fn input_event_reader_owner_can_stop_blocked_reader_thread() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
    let endpoint = listener
        .local_addr()
        .expect("listener addr should be available");
    let _client = TcpStream::connect(endpoint).expect("client should connect");
    let (server, _) = listener.accept().expect("server should accept");
    let input_reader = spawn_input_event_reader(
        server,
        CaptureInputBounds {
            origin_x: 0,
            origin_y: 0,
            width: 1280,
            height: 720,
        },
    );

    assert_eq!(
        input_reader
            .stop_and_join()
            .expect("input reader should stop and join"),
        Some(InputReaderEvent::Disconnected)
    );
}

#[test]
fn h264_session_stops_blocked_input_reader_after_capture_inactive_goodbye() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
    let endpoint = listener
        .local_addr()
        .expect("listener addr should be available");
    let config = host_config(endpoint.to_string());
    let mut runner = RecordingProgramRunner::default();
    let mut capture = RecordingCaptureStarter {
        frames: std::collections::VecDeque::from([Some(captured_bgra_frame()), None]),
        ..Default::default()
    };
    let host = thread::spawn(move || {
        let result = crate::agent::listener::run_control_listener_once_with_runtime(
            listener,
            &config,
            &mut runner,
            &mut capture,
        );
        (result, runner.cleaned)
    });

    let mut client = connect_and_start_session(endpoint);
    read_message(&mut client).expect("session ready should read");
    expect_h264_frame(read_message(&mut client).expect("first encoded frame should read"));
    assert_eq!(
        read_message(&mut client).expect("goodbye should read after capture ends"),
        ControlMessage::Goodbye
    );

    let (host_result, cleaned) = host.join().expect("host thread should finish");
    assert_eq!(
        host_result.expect("host should handle one client"),
        endpoint
    );
    assert_eq!(cleaned, vec![42]);
}
