use std::{
    collections::VecDeque,
    net::{TcpListener, TcpStream},
    sync::mpsc,
    thread,
};

use crate::agent::{
    stream::{
        HostSessionEndReason, InputReaderEvent, spawn_input_event_reader,
        write_h264_encoded_stream_with_test_encoder,
    },
    tests::*,
};
use crate::session_state::RemoteSessionStatus;
use wincast_input::CaptureInputBounds;
use wincast_media::{
    EncodedVideoFrame, MediaResult, RawVideoFrame, VideoEncoder, VideoLatencyMode,
    VideoPipelineConfig,
};
use wincast_protocol::{
    config::VideoCodec,
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
        CaptureInputBounds::from_capture_size(0, 0, 1280, 720),
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
        CaptureInputBounds::from_capture_size(0, 0, 1280, 720),
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

#[test]
fn h264_stream_encodes_first_frame_and_latest_available_frame_only() {
    let mut writer = Vec::new();
    let mut session = RecordingCaptureRuntime {
        frames: VecDeque::from([
            Some(captured_bgra_frame_with_sequence(1)),
            Some(captured_bgra_frame_with_sequence(2)),
            Some(captured_bgra_frame_with_sequence(3)),
            None,
        ]),
        attempts: Default::default(),
        block_after_empty: None,
    };
    let (_sender, receiver) = mpsc::channel();
    let mut encoder = RecordingSequenceEncoder::default();

    let reason = write_h264_encoded_stream_with_test_encoder(
        &mut writer,
        &captured_bgra_frame_with_sequence(0),
        &mut session,
        &receiver,
        VideoPipelineConfig {
            codec: VideoCodec::H264,
            width: 1280,
            height: 720,
            fps: 30,
            bitrate_kbps: 4000,
            max_bitrate_kbps: 6000,
            latency_mode: VideoLatencyMode::LowLatency,
        },
        &FixedSessionGate(RemoteSessionStatus::Allowed),
        &mut encoder,
    )
    .expect("积压 H.264 帧应丢弃旧帧后正常结束");

    assert_eq!(reason, HostSessionEndReason::CaptureInactive);
    assert_eq!(encoder.encoded_sequences, vec![0, 3]);

    let mut reader = writer.as_slice();
    let first = expect_h264_frame(read_message(&mut reader).expect("first frame should decode"));
    let latest = expect_h264_frame(read_message(&mut reader).expect("latest frame should decode"));
    assert_eq!(first.sequence_number, 0);
    assert_eq!(latest.sequence_number, 3);
    assert_eq!(
        read_message(&mut reader).expect("goodbye should decode after capture ends"),
        ControlMessage::Goodbye
    );
}

#[derive(Default)]
struct RecordingSequenceEncoder {
    encoded_sequences: Vec<u64>,
}

impl VideoEncoder for RecordingSequenceEncoder {
    fn encode(&mut self, frame: RawVideoFrame<'_>) -> MediaResult<Option<EncodedVideoFrame>> {
        self.encoded_sequences.push(frame.sequence_number);
        Ok(Some(EncodedVideoFrame {
            codec: VideoCodec::H264,
            width: frame.width,
            height: frame.height,
            sequence_number: frame.sequence_number,
            timestamp_ns: frame.timestamp_ns,
            keyframe: true,
            bytes: vec![1],
        }))
    }

    fn request_keyframe(&mut self) -> MediaResult<()> {
        Ok(())
    }
}
