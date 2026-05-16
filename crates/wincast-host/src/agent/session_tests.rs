use std::{
    collections::VecDeque,
    io,
    net::{TcpListener, TcpStream},
    sync::{Arc, atomic::AtomicUsize, mpsc},
    thread,
    time::Duration,
};

use crate::agent::{
    listener::{
        run_control_listener_n_with_runtime,
        run_control_listener_once_with_runtime_and_session_gate,
    },
    session::{SessionGate, SharedSessionGate, run_started_session},
    stream::{
        HostSessionEndReason, write_raw_bgra_stream_with_input_events, write_session_goodbye,
    },
    tests::*,
};
use crate::session_events::DetectedDesktopSession;
use crate::session_state::{
    ClientSessionErrorCode, RemoteSessionStatus, SessionEvent, SharedSessionState,
};
use wincast_media::{VideoDecoder, test_support::FakeH264Decoder};
use wincast_protocol::{
    config::VideoCodec,
    frame::{read_message, write_message},
    handshake::send_client_hello,
    ipc::SessionEndReason,
    message::{ControlMessage, ErrorCode},
    raw_frame::read_raw_bgra_frame,
};

use crate::program::StartedProgram;

#[test]
fn shared_session_gate_reports_no_user_locked_and_agent_unavailable() {
    let shared_state = SharedSessionState::new();
    let mut gate = SharedSessionGate::new(shared_state.clone());

    assert_eq!(
        gate.remote_session_status(),
        RemoteSessionStatus::Rejected {
            code: ClientSessionErrorCode::NoUserLoggedIn,
            message: "当前没有 Windows 用户登录，无法启动远程会话。",
        }
    );

    shared_state.apply(SessionEvent::UserLoggedIn);
    assert_eq!(
        gate.remote_session_status(),
        RemoteSessionStatus::Rejected {
            code: ClientSessionErrorCode::AgentUnavailable,
            message: "宿主端 Agent 不可用，正在等待重新拉起。",
        }
    );

    shared_state.apply(SessionEvent::AgentStarted);
    shared_state.apply(SessionEvent::SessionLocked);
    assert_eq!(
        gate.remote_session_status(),
        RemoteSessionStatus::Rejected {
            code: ClientSessionErrorCode::SessionLocked,
            message: "Windows 会话已锁定，请先解锁后再启动远程会话。",
        }
    );
}

#[test]
fn foreground_detection_failure_is_conservative_rejection_by_default() {
    let state = crate::agent::session::foreground_run_session_state_from_detection(Err(
        "probe failed".into(),
    ));

    assert_eq!(
        state.remote_session_status(),
        RemoteSessionStatus::Rejected {
            code: ClientSessionErrorCode::NoUserLoggedIn,
            message: "当前没有 Windows 用户登录，无法启动远程会话。",
        }
    );
}

#[test]
fn explicit_development_fallback_keeps_detection_failure_allowed() {
    let state =
        crate::agent::session::foreground_run_session_state_from_detection_with_failure_policy(
            Err("probe failed".into()),
            true,
        );

    assert_eq!(state.remote_session_status(), RemoteSessionStatus::Allowed);
}

#[test]
fn foreground_detection_rejects_no_user_before_launching_program() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
    let endpoint = listener
        .local_addr()
        .expect("listener addr should be available");
    let config = host_config(endpoint.to_string());
    let mut runner = RecordingProgramRunner::default();
    let mut locator = RecordingWindowLocator::default();
    let mut capture = RecordingCaptureStarter::default();
    let state = crate::agent::session::foreground_run_session_state_from_detection(Ok(
        DetectedDesktopSession {
            user_logged_in: false,
            locked: false,
        },
    ));
    let mut session_gate = SharedSessionGate::new(state);
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
            code: ErrorCode::NoUserLoggedIn,
            message: "当前没有 Windows 用户登录，无法启动远程会话。".to_owned(),
        }
    );

    let (host_result, launched, cleaned, lookups, capture_targets) =
        host.join().expect("host thread should finish");
    let error = host_result.expect_err("host should report session rejection");
    assert!(error.contains("当前没有 Windows 用户登录"));
    assert!(launched.is_empty());
    assert!(cleaned.is_empty());
    assert!(lookups.is_empty());
    assert!(capture_targets.is_empty());
}

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

#[test]
fn host_sends_first_h264_encoded_frame_after_session_ready_without_sending_raw_bgra() {
    let tcp_pair = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
    let endpoint = tcp_pair
        .local_addr()
        .expect("listener addr should be available");
    let mut client = TcpStream::connect(endpoint).expect("client should connect");
    let (server, _) = tcp_pair.accept().expect("server should accept");
    let config = host_config_with_codec("127.0.0.1:0".to_owned(), VideoCodec::H264);
    let mut locator = RecordingWindowLocator::default();
    let first_frame = captured_bgra_frame_with_sequence(7);
    let mut capture = RecordingCaptureStarter {
        frames: VecDeque::from([Some(first_frame.clone())]),
        ..RecordingCaptureStarter::default()
    };
    let started = StartedProgram::from_process_id(42);

    let host = thread::spawn(move || {
        let mut writer = server
            .try_clone()
            .expect("server writer should clone for protocol output");
        run_started_session(
            &mut writer,
            &server,
            &config,
            &mut locator,
            &mut capture,
            &started,
        )
    });

    assert_eq!(
        read_message(&mut client).expect("session ready should decode"),
        ControlMessage::SessionReady {
            width: 1280,
            height: 720,
        }
    );
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("client read timeout should be set");
    let encoded = read_message(&mut client).expect("encoded frame should decode");
    let raw_after_encoded = read_raw_bgra_frame(&mut client);
    host.join()
        .expect("host thread should finish")
        .expect("h264 first-frame path should finish cleanly");

    let ControlMessage::EncodedVideoFrame(encoded) = encoded else {
        panic!("expected encoded video frame, got {encoded:?}");
    };
    encoded
        .validate()
        .expect("encoded frame should satisfy protocol boundary");
    assert_eq!(encoded.codec, VideoCodec::H264);
    assert_eq!(encoded.width, first_frame.metadata.frame.width);
    assert_eq!(encoded.height, first_frame.metadata.frame.height);
    assert_eq!(
        encoded.sequence_number,
        first_frame.metadata.frame.sequence_number
    );
    assert_eq!(
        encoded.timestamp_ns,
        first_frame.metadata.frame.timestamp_ns
    );
    assert!(encoded.keyframe);

    let mut decoder = FakeH264Decoder::new();
    let decoded = decoder
        .decode(&encoded)
        .expect("encoded frame should decode through fake H.264 boundary");
    assert_eq!(decoded.width, first_frame.metadata.frame.width);
    assert_eq!(decoded.height, first_frame.metadata.frame.height);
    assert_eq!(
        decoded.bytes.len(),
        first_frame.row_pitch as usize * first_frame.metadata.frame.height as usize
    );
    assert!(
        raw_after_encoded.is_err(),
        "h264 path must not fall back to raw BGRA binary frames"
    );
}

#[test]
fn host_reports_encoding_failed_when_h264_fake_encoder_rejects_first_capture_frame() {
    let tcp_pair = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
    let endpoint = tcp_pair
        .local_addr()
        .expect("listener addr should be available");
    let mut client = TcpStream::connect(endpoint).expect("client should connect");
    let (server, _) = tcp_pair.accept().expect("server should accept");
    let config = host_config_with_codec("127.0.0.1:0".to_owned(), VideoCodec::H264);
    let mut locator = RecordingWindowLocator::default();
    let too_wide_frame = captured_bgra_frame_with_dimensions(1921, 720);
    let mut capture = RecordingCaptureStarter {
        frames: VecDeque::from([Some(too_wide_frame)]),
        ..RecordingCaptureStarter::default()
    };
    let started = StartedProgram::from_process_id(42);

    let host = thread::spawn(move || {
        let mut writer = server
            .try_clone()
            .expect("server writer should clone for protocol output");
        run_started_session(
            &mut writer,
            &server,
            &config,
            &mut locator,
            &mut capture,
            &started,
        )
    });

    assert_eq!(
        read_message(&mut client).expect("session ready should decode"),
        ControlMessage::SessionReady {
            width: 1921,
            height: 720,
        }
    );
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("client read timeout should be set");
    let error_message = read_message(&mut client).expect("encoding failure should decode");
    let raw_after_error = read_raw_bgra_frame(&mut client);
    let error = host
        .join()
        .expect("host thread should finish")
        .expect_err("h264 encoder rejection should fail the session");

    assert_eq!(
        error_message,
        ControlMessage::Error {
            code: ErrorCode::EncodingFailed,
            message: "H.264 首帧编码失败: 视频尺寸 1921x720 超过上限 1920x1080".to_owned(),
        }
    );
    assert!(
        raw_after_error.is_err(),
        "h264 failure path must not fall back to raw BGRA binary frames"
    );
    assert!(error.message.contains("H.264 首帧编码失败"));
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
