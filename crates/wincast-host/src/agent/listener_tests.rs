use std::{
    collections::VecDeque,
    net::{TcpListener, TcpStream},
    sync::Arc,
    thread,
};

use crate::{
    agent::{
        listener::{
            run_control_listener_n_with_runtime, run_control_listener_once_with_runtime,
            run_control_listener_once_with_runtime_and_session_gate,
        },
        tests::*,
    },
    session_state::{ClientSessionErrorCode, RemoteSessionStatus},
};
use wincast_capture::CaptureTarget;
use wincast_protocol::{
    frame::{read_message, write_message},
    handshake::send_client_hello,
    message::{ControlMessage, ErrorCode},
    raw_frame::read_raw_bgra_frame,
};

#[test]
fn host_accepts_one_tcp_control_handshake_and_launches_program_before_streaming_raw_bgra() {
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
    assert_eq!(
        read_message(&mut client).expect("host hello should read"),
        ControlMessage::Hello { version: 1 }
    );
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
fn host_accepts_two_clients_in_sequence_without_rebinding() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
    let endpoint = listener
        .local_addr()
        .expect("listener addr should be available");
    let config = host_config(endpoint.to_string());
    let mut runner = RecordingProgramRunner::default();
    let mut locator = RecordingWindowLocator::default();
    let mut capture = RecordingCaptureStarter::default();
    let host = thread::spawn(move || {
        let result = run_control_listener_n_with_runtime(
            listener,
            &config,
            &mut runner,
            &mut locator,
            &mut capture,
            2,
        );
        (
            result,
            runner.launched,
            runner.cleaned,
            locator.lookups,
            capture.targets,
        )
    });

    let first = run_short_client_session(endpoint);
    let second = run_short_client_session(endpoint);

    assert_eq!(first.sequence_number, 0);
    assert_eq!(second.sequence_number, 0);
    let (host_result, launched, cleaned, lookups, capture_targets) =
        host.join().expect("host thread should finish");
    assert_eq!(
        host_result.expect("host should handle two clients"),
        endpoint
    );
    assert_eq!(launched.len(), 2);
    assert_eq!(cleaned, vec![42, 43]);
    assert_eq!(lookups, vec![(42, None), (43, None)]);
    assert_eq!(
        capture_targets,
        vec![CaptureTarget::Desktop, CaptureTarget::Desktop]
    );
}

#[test]
fn host_rejects_second_client_while_session_active() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
    let endpoint = listener
        .local_addr()
        .expect("listener addr should be available");
    let config = host_config(endpoint.to_string());
    let mut runner = RecordingProgramRunner::default();
    let mut locator = RecordingWindowLocator::default();
    let block = Arc::new(BlockingFrameGate::new());
    let mut capture = RecordingCaptureStarter {
        frames: VecDeque::from([Some(captured_bgra_frame()), None]),
        block_after_empty: Some(block.clone()),
        ..Default::default()
    };
    let host = thread::spawn(move || {
        let result = run_control_listener_n_with_runtime(
            listener,
            &config,
            &mut runner,
            &mut locator,
            &mut capture,
            3,
        );
        (result, runner.launched, runner.cleaned)
    });

    let mut first_client = connect_and_start_session(endpoint);
    read_message(&mut first_client).expect("first session ready should read");
    read_message(&mut first_client).expect("first video ready should read");
    read_raw_bgra_frame(&mut first_client).expect("first raw frame should read");
    block.wait_until_blocked();

    let mut second_client = TcpStream::connect(endpoint).expect("second client should connect");
    send_client_hello(&mut second_client).expect("second client hello should write");
    assert_eq!(
        read_message(&mut second_client).expect("busy response should read"),
        ControlMessage::Error {
            code: ErrorCode::Busy,
            message: "宿主端已有客户端连接".to_owned(),
        }
    );

    write_message(&mut first_client, &ControlMessage::StopSession)
        .expect("stop session should write");
    block.release();
    assert_eq!(
        read_message(&mut first_client).expect("first goodbye should read"),
        ControlMessage::Goodbye
    );

    let third = run_short_client_session(endpoint);

    assert_eq!(third.sequence_number, 0);
    let (host_result, launched, cleaned) = host.join().expect("host thread should finish");
    assert_eq!(host_result.expect("host should keep listening"), endpoint);
    assert_eq!(launched.len(), 2);
    assert_eq!(cleaned, vec![42, 43]);
}

#[test]
fn host_rejects_start_session_when_remote_session_is_locked_before_launching_program() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
    let endpoint = listener
        .local_addr()
        .expect("listener addr should be available");
    let config = host_config(endpoint.to_string());
    let mut runner = RecordingProgramRunner::default();
    let mut locator = RecordingWindowLocator::default();
    let mut capture = RecordingCaptureStarter::default();
    let mut session_gate = FixedSessionGate(RemoteSessionStatus::Rejected {
        code: ClientSessionErrorCode::SessionLocked,
        message: "Windows 会话已锁定，请先解锁后再启动远程会话。",
    });
    let host = thread::spawn(move || {
        let result = run_control_listener_once_with_runtime_and_session_gate(
            listener,
            &config,
            &mut runner,
            &mut locator,
            &mut capture,
            &mut session_gate,
        );
        (
            result,
            runner.launched,
            runner.cleaned,
            locator.lookups,
            capture.targets,
        )
    });

    let mut client = TcpStream::connect(endpoint).expect("client should connect");
    send_client_hello(&mut client).expect("client hello should write");
    assert_eq!(
        read_message(&mut client).expect("host hello should read"),
        ControlMessage::Hello { version: 1 }
    );
    write_message(&mut client, &ControlMessage::StartSession).expect("start session should write");

    assert_eq!(
        read_message(&mut client).expect("session rejection should read"),
        ControlMessage::Error {
            code: ErrorCode::SessionLocked,
            message: "Windows 会话已锁定，请先解锁后再启动远程会话。".to_owned(),
        }
    );

    let (host_result, launched, cleaned, lookups, capture_targets) =
        host.join().expect("host thread should finish");
    let error = host_result.expect_err("host should report session rejection");
    assert!(error.contains("Windows 会话已锁定"));
    assert!(launched.is_empty());
    assert!(cleaned.is_empty());
    assert!(lookups.is_empty());
    assert!(capture_targets.is_empty());
}
