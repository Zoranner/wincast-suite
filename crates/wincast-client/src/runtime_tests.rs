use std::{net::TcpListener, sync::mpsc, thread, time::Duration};

use wincast_media::{
    OpenH264Encoder, RawPixelFormat, RawVideoFrame, VideoEncoder, VideoLatencyMode,
    VideoPipelineConfig,
};
use wincast_protocol::config::VideoCodec;
use wincast_protocol::{
    config::ClientConfig,
    frame::{read_message, write_message},
    handshake::{HandshakeError, send_client_hello},
    message::{ControlMessage, EncodedVideoFrame, ErrorCode},
};

use crate::{
    errors::format_host_error,
    runtime::{
        ClientRunError, RetryOptions, RetryReport, control_channel_ready_message,
        format_retry_report, run_client_with_config, run_with_retry, run_with_retry_and_reporter,
    },
    stream::validate_encoded_video_frame,
};

#[test]
fn client_run_performs_tcp_control_handshake() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
    let endpoint = listener
        .local_addr()
        .expect("listener address should exist");
    let (observed_messages_tx, observed_messages_rx) = mpsc::channel();

    let host_thread = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("client should connect");
        let hello = read_message(&mut stream).expect("client hello should decode");
        send_client_hello(&mut stream).expect("host hello should encode");
        let start_session = read_message(&mut stream).expect("start session should decode");
        write_message(
            &mut stream,
            &ControlMessage::SessionReady {
                width: 2,
                height: 2,
            },
        )
        .expect("session ready should encode");
        for frame in test_h264_frames(3) {
            write_message(&mut stream, &ControlMessage::EncodedVideoFrame(frame))
                .expect("encoded frame should encode");
        }
        observed_messages_tx
            .send((hello, start_session))
            .expect("observed messages should send");
    });

    let config = ClientConfig {
        host: endpoint.ip().to_string(),
        port: endpoint.port(),
    };
    let message = run_client_with_config(&config).expect("client run should complete handshake");

    let observed_messages = observed_messages_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("host should observe client control messages");
    host_thread.join().expect("host thread should finish");
    assert_eq!(
        observed_messages,
        (
            ControlMessage::Hello {
                version: wincast_protocol::handshake::PROTOCOL_VERSION,
            },
            ControlMessage::StartSession,
        )
    );
    assert!(message.contains("已建立宿主端控制通道"));
    assert!(message.contains(&config.endpoint()));
    assert!(!message.contains("运行时链路未实现"));
}

#[test]
fn client_run_reports_runtime_unimplemented_response_from_host() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
    let endpoint = listener
        .local_addr()
        .expect("listener address should exist");

    let host_thread = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("client should connect");
        read_message(&mut stream).expect("client hello should decode");
        send_client_hello(&mut stream).expect("host hello should encode");
        read_message(&mut stream).expect("start session should decode");
        write_message(
            &mut stream,
            &ControlMessage::Error {
                code: ErrorCode::TransportFailed,
                message: "运行时链路未实现：尚未启动程序生命周期、画面捕获、编码传输和输入注入。"
                    .to_owned(),
            },
        )
        .expect("runtime error should encode");
    });

    let config = ClientConfig {
        host: endpoint.ip().to_string(),
        port: endpoint.port(),
    };
    let error = run_client_with_config(&config).expect_err("runtime unimplemented should fail");

    host_thread.join().expect("host thread should finish");
    assert!(error.contains("宿主端传输链路失败"));
    assert!(error.contains("运行时链路未实现"));
}

#[test]
fn client_reports_encoding_failure_in_chinese() {
    let error = format_host_error(ErrorCode::EncodingFailed, "硬件编码器初始化失败".to_owned());

    assert!(error.contains("宿主端视频编码失败"));
    assert!(error.contains("硬件编码器初始化失败"));
    assert!(!error.contains("未实现"));
    assert!(!error.contains("尚未接入 H.264"));
}

#[test]
fn client_formats_host_session_errors_with_specific_chinese_prefixes() {
    let cases = [
        (ErrorCode::Busy, "宿主端忙碌"),
        (ErrorCode::ProgramLaunchFailed, "宿主端程序启动失败"),
        (ErrorCode::ProgramExited, "宿主端程序已退出"),
        (ErrorCode::CaptureFailed, "宿主端画面捕获失败"),
        (ErrorCode::TransportFailed, "宿主端传输链路失败"),
        (ErrorCode::InvalidConfig, "宿主端配置无效"),
        (ErrorCode::NoUserLoggedIn, "宿主端未登录 Windows 用户"),
        (ErrorCode::SessionLocked, "宿主端 Windows 会话已锁屏"),
        (ErrorCode::AgentUnavailable, "宿主端 Agent 不可用或不在线"),
    ];

    for (code, expected_prefix) in cases {
        let error = format_host_error(code, "测试错误详情".to_owned());

        assert!(
            error.starts_with(expected_prefix),
            "{code:?} should start with {expected_prefix}, got {error}"
        );
        assert!(error.contains("测试错误详情"));
        assert!(!error.contains("宿主端拒绝连接"));
    }
}

#[test]
fn client_accepts_encoded_video_frame_after_openh264_decode_without_reading_raw_bgra_stream() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
    let endpoint = listener
        .local_addr()
        .expect("listener address should exist");

    let host_thread = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("client should connect");
        read_message(&mut stream).expect("client hello should decode");
        send_client_hello(&mut stream).expect("host hello should encode");
        read_message(&mut stream).expect("start session should decode");
        write_message(
            &mut stream,
            &ControlMessage::SessionReady {
                width: 2,
                height: 2,
            },
        )
        .expect("session ready should encode");
        for frame in test_h264_frames(3) {
            write_message(&mut stream, &ControlMessage::EncodedVideoFrame(frame))
                .expect("encoded frame should encode");
        }
    });

    let config = ClientConfig {
        host: endpoint.ip().to_string(),
        port: endpoint.port(),
    };
    let message = run_client_with_config(&config)
        .expect("encoded video frame should validate without raw BGRA read");

    host_thread.join().expect("host thread should finish");
    assert!(message.contains("客户端已完成宿主端首个视频响应的解码边界校验"));
    assert!(!message.contains("H.264 编码帧"));
    assert!(!message.contains("raw BGRA"));
}

#[test]
fn client_decodes_valid_encoded_video_frame_to_complete_bgra_boundary() {
    let decoded = validate_encoded_video_frame(&test_h264_frame(9))
        .expect("valid H.264 frame should decode through OpenH264 boundary");

    assert_eq!(decoded.width, 16);
    assert_eq!(decoded.height, 16);
    assert_eq!(decoded.row_pitch, 64);
    assert_eq!(
        decoded.bytes_len,
        decoded.row_pitch as usize * decoded.height as usize
    );
}

#[test]
fn client_rejects_invalid_encoded_video_frame_from_host() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
    let endpoint = listener
        .local_addr()
        .expect("listener address should exist");

    let host_thread = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("client should connect");
        read_message(&mut stream).expect("client hello should decode");
        send_client_hello(&mut stream).expect("host hello should encode");
        read_message(&mut stream).expect("start session should decode");
        write_message(
            &mut stream,
            &ControlMessage::SessionReady {
                width: 2,
                height: 2,
            },
        )
        .expect("session ready should encode");
        write_message(
            &mut stream,
            &ControlMessage::EncodedVideoFrame(EncodedVideoFrame {
                codec: wincast_protocol::config::VideoCodec::H264,
                width: 2,
                height: 2,
                sequence_number: 1,
                timestamp_ns: 1_000,
                keyframe: true,
                bytes: Vec::new(),
            }),
        )
        .expect("encoded frame should encode");
    });

    let config = ClientConfig {
        host: endpoint.ip().to_string(),
        port: endpoint.port(),
    };
    let error = run_client_with_config(&config).expect_err("invalid encoded frame should fail");

    host_thread.join().expect("host thread should finish");
    assert!(error.contains("H.264 编码帧无效"));
    assert!(!error.contains("raw BGRA"));
}

#[test]
fn default_retry_policy_does_not_retry() {
    let mut attempts = 0;
    let mut sleeps = Vec::new();
    let options = RetryOptions {
        retries: 0,
        retry_delay: Duration::from_millis(10),
    };

    let error = run_with_retry(
        &options,
        || {
            attempts += 1;
            Err(ClientRunError::host_status(
                ErrorCode::Busy,
                "宿主端已有客户端连接",
            ))
        },
        |delay| sleeps.push(delay),
    )
    .expect_err("default retry policy should fail after one attempt");

    assert_eq!(attempts, 1);
    assert!(sleeps.is_empty());
    assert!(error.contains("宿主端忙碌"));
    assert!(!error.contains("尝试 1 次后失败"));
}

#[test]
fn retry_policy_succeeds_after_busy_status_recovers() {
    let mut attempts = 0;
    let mut sleeps = Vec::new();
    let options = RetryOptions {
        retries: 2,
        retry_delay: Duration::from_millis(25),
    };

    let message = run_with_retry(
        &options,
        || {
            attempts += 1;
            if attempts == 1 {
                Err(ClientRunError::host_status(
                    ErrorCode::Busy,
                    "宿主端已有客户端连接",
                ))
            } else {
                Ok("已连接".to_owned())
            }
        },
        |delay| sleeps.push(delay),
    )
    .expect("busy status should be retried and recover");

    assert_eq!(message, "已连接");
    assert_eq!(attempts, 2);
    assert_eq!(sleeps, vec![Duration::from_millis(25)]);
}

#[test]
fn retry_policy_reconnects_after_video_stream_interruption() {
    let mut attempts = 0;
    let mut sleeps = Vec::new();
    let options = RetryOptions {
        retries: 2,
        retry_delay: Duration::from_millis(25),
    };

    let message = run_with_retry(
        &options,
        || {
            attempts += 1;
            if attempts == 1 {
                Err(ClientRunError::VideoStreamInterrupted(
                    "视频流中断: 读取宿主端 H.264 编码视频帧失败: 连接被重置".to_owned(),
                ))
            } else {
                Ok("已重新连接".to_owned())
            }
        },
        |delay| sleeps.push(delay),
    )
    .expect("video stream interruption should be retried and recover");

    assert_eq!(message, "已重新连接");
    assert_eq!(attempts, 2);
    assert_eq!(sleeps, vec![Duration::from_millis(25)]);
}

#[test]
fn retry_report_formats_session_locked_reason_with_delay() {
    let report = RetryReport {
        attempt: 1,
        max_attempts: 4,
        retry_delay: Duration::from_millis(1_000),
        reason: "宿主端 Windows 会话已锁屏: 当前用户锁屏".to_owned(),
    };

    let message = format_retry_report(&report);

    assert!(message.contains("客户端运行第 1/4 次失败"));
    assert!(message.contains("宿主端 Windows 会话已锁屏: 当前用户锁屏"));
    assert!(message.contains("1000 ms 后重试"));
}

#[test]
fn retry_policy_reports_retriable_error_before_recovering() {
    let mut attempts = 0;
    let mut reports = Vec::new();
    let mut sleeps = Vec::new();
    let options = RetryOptions {
        retries: 2,
        retry_delay: Duration::from_millis(30),
    };

    let message = run_with_retry_and_reporter(
        &options,
        || {
            attempts += 1;
            if attempts == 1 {
                Err(ClientRunError::host_status(
                    ErrorCode::SessionLocked,
                    "当前 Windows 用户已锁屏",
                ))
            } else {
                Ok("已连接".to_owned())
            }
        },
        |delay| sleeps.push(delay),
        |report| reports.push(format_retry_report(report)),
    )
    .expect("session locked status should be retried and recover");

    assert_eq!(message, "已连接");
    assert_eq!(attempts, 2);
    assert_eq!(sleeps, vec![Duration::from_millis(30)]);
    assert_eq!(reports.len(), 1);
    assert!(reports[0].contains("客户端运行第 1/3 次失败"));
    assert!(reports[0].contains("宿主端 Windows 会话已锁屏: 当前 Windows 用户已锁屏"));
    assert!(reports[0].contains("30 ms 后重试"));
}

#[test]
fn retry_policy_reports_last_session_locked_error_after_limit() {
    let mut attempts = 0;
    let mut sleeps = Vec::new();
    let options = RetryOptions {
        retries: 2,
        retry_delay: Duration::from_millis(15),
    };

    let error = run_with_retry(
        &options,
        || {
            attempts += 1;
            Err(ClientRunError::host_status(
                ErrorCode::SessionLocked,
                format!("第 {attempts} 次仍锁屏"),
            ))
        },
        |delay| sleeps.push(delay),
    )
    .expect_err("session locked should fail after retry limit");

    assert_eq!(attempts, 3);
    assert_eq!(
        sleeps,
        vec![Duration::from_millis(15), Duration::from_millis(15)]
    );
    assert!(error.contains("尝试 3 次后失败"));
    assert!(error.contains("宿主端 Windows 会话已锁屏"));
    assert!(error.contains("第 3 次仍锁屏"));
}

#[test]
fn retry_policy_retries_session_gate_rejections_until_recovering() {
    for code in [
        ErrorCode::NoUserLoggedIn,
        ErrorCode::SessionLocked,
        ErrorCode::AgentUnavailable,
    ] {
        let mut attempts = 0;
        let mut sleeps = Vec::new();
        let options = RetryOptions {
            retries: 3,
            retry_delay: Duration::from_millis(20),
        };

        let message = run_with_retry(
            &options,
            || {
                attempts += 1;
                if attempts < 3 {
                    Err(ClientRunError::host_status(
                        code,
                        format!("第 {attempts} 次门禁拒绝"),
                    ))
                } else {
                    Ok(format!("{code:?} recovered"))
                }
            },
            |delay| sleeps.push(delay),
        )
        .expect("session gate rejection should be retried until recovery");

        assert_eq!(message, format!("{code:?} recovered"));
        assert_eq!(attempts, 3);
        assert_eq!(
            sleeps,
            vec![Duration::from_millis(20), Duration::from_millis(20)]
        );
    }
}

#[test]
fn retry_policy_does_not_retry_unsupported_version() {
    let mut attempts = 0;
    let options = RetryOptions {
        retries: 3,
        retry_delay: Duration::from_millis(10),
    };

    let error = run_with_retry(
        &options,
        || {
            attempts += 1;
            Err(ClientRunError::Handshake(
                HandshakeError::UnsupportedVersion,
            ))
        },
        |_| panic!("unsupported version should not sleep"),
    )
    .expect_err("unsupported version should not retry");

    assert_eq!(attempts, 1);
    assert!(error.contains("协议版本不匹配"));
    assert!(
        !error.contains("尝试"),
        "不可重试错误不应伪装成重试耗尽: {error}"
    );
}

#[test]
fn client_run_reports_host_error_in_chinese() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
    let endpoint = listener
        .local_addr()
        .expect("listener address should exist");

    let host_thread = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("client should connect");
        read_message(&mut stream).expect("client hello should decode");
        write_message(
            &mut stream,
            &ControlMessage::Error {
                code: ErrorCode::Busy,
                message: "宿主端已有客户端连接".to_owned(),
            },
        )
        .expect("host error should encode");
    });

    let config = ClientConfig {
        host: endpoint.ip().to_string(),
        port: endpoint.port(),
    };
    let error = run_client_with_config(&config).expect_err("host error should fail client run");

    host_thread.join().expect("host thread should finish");
    assert!(error.contains("宿主端忙碌"));
    assert!(error.contains("宿主端已有客户端连接"));
}

#[test]
fn run_message_does_not_claim_runtime_chain_is_ready() {
    let config = ClientConfig {
        host: "192.168.10.25".to_owned(),
        port: 7856,
    };

    let message = control_channel_ready_message(&config);

    assert!(message.contains("已建立宿主端控制通道"));
    assert!(message.contains("客户端已完成宿主端首个视频响应的解码边界校验"));
    assert!(message.contains("宿主端已接入基础 Windows 输入注入"));
    assert!(!message.contains("H.264 编码帧"));
    assert!(!message.contains("raw BGRA 已接入"));
    assert!(!message.contains("H.264/WebRTC 编码传输尚未实现"));
    assert!(!message.contains("视频解码渲染和输入事件链路尚未实现"));
}

fn test_h264_frames(count: u64) -> Vec<EncodedVideoFrame> {
    let mut encoder = OpenH264Encoder::new(VideoPipelineConfig {
        codec: VideoCodec::H264,
        width: 16,
        height: 16,
        fps: 30,
        bitrate_kbps: 300,
        max_bitrate_kbps: 1_000,
        latency_mode: VideoLatencyMode::LowLatency,
    })
    .expect("OpenH264 encoder should initialize");
    (1..=count)
        .map(|sequence_number| {
            encoder
                .encode(RawVideoFrame {
                    width: 16,
                    height: 16,
                    row_pitch: 64,
                    format: RawPixelFormat::Bgra8Unorm,
                    sequence_number,
                    timestamp_ns: sequence_number * 1_000_000,
                    bytes: &test_bgra_frame(16, 16),
                })
                .expect("test H.264 frame should encode")
        })
        .collect()
}

fn test_bgra_frame(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        for x in 0..width {
            bytes.push((x * 11) as u8);
            bytes.push((y * 13) as u8);
            bytes.push(((x + y) * 7) as u8);
            bytes.push(0xff);
        }
    }
    bytes
}

fn test_h264_frame(sequence_number: u64) -> EncodedVideoFrame {
    let mut encoder = OpenH264Encoder::new(VideoPipelineConfig {
        codec: VideoCodec::H264,
        width: 16,
        height: 16,
        fps: 30,
        bitrate_kbps: 300,
        max_bitrate_kbps: 1_000,
        latency_mode: VideoLatencyMode::LowLatency,
    })
    .expect("OpenH264 encoder should initialize");
    encoder
        .encode(RawVideoFrame {
            width: 16,
            height: 16,
            row_pitch: 64,
            format: RawPixelFormat::Bgra8Unorm,
            sequence_number,
            timestamp_ns: sequence_number * 1_000_000,
            bytes: &test_bgra_frame(16, 16),
        })
        .expect("test H.264 frame should encode")
}
