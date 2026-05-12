use std::{
    collections::VecDeque,
    io,
    net::{TcpListener, TcpStream},
    sync::{Arc, atomic::AtomicUsize, mpsc},
    thread,
};

use crate::agent::{
    listener::run_control_listener_n_with_runtime,
    session::run_started_session,
    stream::{
        HostSessionEndReason, write_raw_bgra_stream_with_input_events, write_session_goodbye,
    },
    tests::*,
};
use wincast_protocol::{
    frame::{read_message, write_message},
    ipc::SessionEndReason,
    message::{ControlMessage, ErrorCode},
    raw_frame::read_raw_bgra_frame,
};

use crate::program::StartedProgram;

#[test]
fn host_reports_error_response_write_failure_without_hiding_window_failure() {
    let tcp_pair = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
    let endpoint = tcp_pair
        .local_addr()
        .expect("listener addr should be available");
    let client = TcpStream::connect(endpoint).expect("client should connect");
    let (server, _) = tcp_pair.accept().expect("server should accept");
    let mut writer = FailingWriter;
    let mut config = host_config("127.0.0.1:0".to_owned());
    config.capture.startup_timeout_ms = 1;
    let mut locator = FailingWindowLocator;
    let mut capture = RecordingCaptureStarter::default();
    let started = StartedProgram::from_process_id(42);

    let error = run_started_session(
        &mut writer,
        &server,
        &config,
        &mut locator,
        &mut capture,
        &started,
    )
    .expect_err("host should report session failure");

    assert_eq!(error.reason, HostSessionEndReason::CaptureFailed);
    assert!(error.message.contains("定位宿主端程序窗口失败"));
    assert!(error.message.contains("写入控制错误消息失败"));
    drop(client);
}

#[test]
fn host_reports_error_response_write_failure_without_hiding_capture_failure() {
    let tcp_pair = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
    let endpoint = tcp_pair
        .local_addr()
        .expect("listener addr should be available");
    let client = TcpStream::connect(endpoint).expect("client should connect");
    let (server, _) = tcp_pair.accept().expect("server should accept");
    let mut writer = FailingWriter;
    let config = host_config("127.0.0.1:0".to_owned());
    let mut locator = RecordingWindowLocator::default();
    let mut capture = FailingCaptureStarter;
    let started = StartedProgram::from_process_id(42);

    let error = run_started_session(
        &mut writer,
        &server,
        &config,
        &mut locator,
        &mut capture,
        &started,
    )
    .expect_err("host should report session failure");

    assert_eq!(error.reason, HostSessionEndReason::CaptureFailed);
    assert!(error.message.contains("初始化画面捕获失败"));
    assert!(error.message.contains("写入控制错误消息失败"));
    drop(client);
}

struct FailingWriter;

impl io::Write for FailingWriter {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "client closed before error response",
        ))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn host_cleans_program_after_stop_session_and_waits_for_next_client() {
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
        (result, runner.cleaned)
    });

    let mut first_client = connect_and_start_session(endpoint);
    read_message(&mut first_client).expect("session ready should read");
    read_message(&mut first_client).expect("video ready should read");
    read_raw_bgra_frame(&mut first_client).expect("first raw frame should read");
    write_message(&mut first_client, &ControlMessage::StopSession)
        .expect("stop session should write");
    assert_eq!(
        read_message(&mut first_client).expect("goodbye should read after stop"),
        ControlMessage::Goodbye
    );

    let second = run_short_client_session(endpoint);

    assert_eq!(second.sequence_number, 0);
    let (host_result, cleaned) = host.join().expect("host thread should finish");
    assert_eq!(host_result.expect("host should keep listening"), endpoint);
    assert_eq!(cleaned, vec![42, 43]);
}

#[test]
fn host_sends_goodbye_when_capture_session_finishes() {
    let mut writer = Vec::new();
    let mut session = RecordingCaptureRuntime {
        frames: VecDeque::from([None]),
        attempts: Arc::new(AtomicUsize::new(0)),
        block_after_empty: None,
    };
    let (_sender, receiver) = mpsc::channel();

    let reason = write_raw_bgra_stream_with_input_events(
        &mut writer,
        &captured_bgra_frame(),
        &mut session,
        &receiver,
    )
    .expect("capture end should be reported as a clean session end");

    assert_eq!(reason, HostSessionEndReason::CaptureInactive);
    assert_eq!(
        SessionEndReason::from(reason),
        SessionEndReason::DesktopUnavailable
    );

    let mut reader = writer.as_slice();
    assert_eq!(
        read_message(&mut reader).expect("video ready should decode"),
        ControlMessage::VideoReady
    );
    let frame = read_raw_bgra_frame(&mut reader).expect("first frame should decode");
    assert_eq!(frame.sequence_number, 0);
    assert_eq!(
        read_message(&mut reader).expect("goodbye should decode"),
        ControlMessage::Goodbye
    );
    assert!(
        read_message(&mut reader).is_err(),
        "capture finish should not send an error after goodbye"
    );
}

#[test]
fn host_reports_capture_error_before_returning_frame_read_failure() {
    let mut writer = Vec::new();
    let mut session = FrameReadFailingCaptureRuntime;
    let (_sender, receiver) = mpsc::channel();

    let error = write_raw_bgra_stream_with_input_events(
        &mut writer,
        &captured_bgra_frame(),
        &mut session,
        &receiver,
    )
    .expect_err("capture read failure should be returned to host");

    assert_eq!(error.reason, HostSessionEndReason::CaptureFailed);
    assert_eq!(
        SessionEndReason::from(error.reason),
        SessionEndReason::SessionFailed
    );
    assert!(error.message.contains("读取后续 raw BGRA 捕获帧失败"));
    assert!(error.message.contains("D3D readback failed"));

    let mut reader = writer.as_slice();
    assert_eq!(
        read_message(&mut reader).expect("video ready should decode"),
        ControlMessage::VideoReady
    );
    let frame = read_raw_bgra_frame(&mut reader).expect("first frame should decode");
    assert_eq!(frame.sequence_number, 0);
    assert_eq!(
        read_message(&mut reader).expect("capture error should decode"),
        ControlMessage::Error {
            code: ErrorCode::CaptureFailed,
            message: "读取后续 raw BGRA 捕获帧失败: 读取 Windows 捕获帧失败: D3D readback failed"
                .to_string(),
        }
    );
}

#[test]
fn goodbye_write_ignores_already_closed_client_connection() {
    let reader = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
    let endpoint = reader
        .local_addr()
        .expect("listener addr should be available");
    let client = TcpStream::connect(endpoint).expect("client should connect");
    let (mut server, _) = reader.accept().expect("server should accept");
    drop(client);

    write_session_goodbye(&mut server)
        .expect("closed client should still be treated as ended session");
}
