use std::{
    collections::VecDeque,
    io,
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

use crate::agent::{
    stream::{
        HostSessionEndReason, InputReaderEvent, read_input_events_until_stop,
        read_input_events_until_stop_with_timeout_limit, spawn_input_event_reader,
        write_h264_encoded_stream_with_input_events,
    },
    tests::*,
};
use crate::session_state::RemoteSessionStatus;
use wincast_input::CaptureInputBounds;
use wincast_media::{VideoLatencyMode, VideoPipelineConfig};
use wincast_protocol::{
    config::VideoCodec,
    frame::{read_message, write_message},
    input::{ButtonState, InputEvent, Modifiers},
    message::ControlMessage,
};

#[test]
fn input_reader_handles_client_input_events_until_stop_session() {
    let mut bytes = Vec::new();
    write_message(
        &mut bytes,
        &ControlMessage::InputEvent(InputEvent::Key {
            code: 65,
            state: ButtonState::Pressed,
            modifiers: Modifiers {
                shift: true,
                ctrl: false,
                alt: false,
                logo: false,
            },
        }),
    )
    .expect("input event should encode");
    write_message(&mut bytes, &ControlMessage::StopSession).expect("stop should encode");
    let mut sink = RecordingInputEventSink::default();

    let stop_requested = AtomicBool::new(false);
    let event = read_input_events_until_stop(&mut bytes.as_slice(), &mut sink, &stop_requested);

    assert_eq!(event, InputReaderEvent::StopSession);
    assert_eq!(
        sink.events,
        vec![InputEvent::Key {
            code: 65,
            state: ButtonState::Pressed,
            modifiers: Modifiers {
                shift: true,
                ctrl: false,
                alt: false,
                logo: false,
            },
        }]
    );
}

#[test]
fn input_reader_accepts_heartbeat_without_ending_session() {
    let mut bytes = Vec::new();
    write_message(&mut bytes, &ControlMessage::Heartbeat).expect("heartbeat should encode");
    write_message(&mut bytes, &ControlMessage::StopSession).expect("stop should encode");
    let mut sink = RecordingInputEventSink::default();

    let stop_requested = AtomicBool::new(false);
    let event = read_input_events_until_stop(&mut bytes.as_slice(), &mut sink, &stop_requested);

    assert_eq!(event, InputReaderEvent::StopSession);
    assert!(sink.events.is_empty());
}

#[test]
fn input_reader_reports_disconnected_after_repeated_heartbeat_timeouts() {
    let mut reader = TimeoutReader;
    let mut sink = RecordingInputEventSink::default();
    let stop_requested = AtomicBool::new(false);

    let event =
        read_input_events_until_stop_with_timeout_limit(&mut reader, &mut sink, &stop_requested, 3);

    assert_eq!(event, InputReaderEvent::Disconnected);
    assert!(sink.events.is_empty());
}

#[test]
fn input_reader_reports_stop_session_after_input_events() {
    let mut bytes = Vec::new();
    write_message(
        &mut bytes,
        &ControlMessage::InputEvent(InputEvent::MouseWheel {
            delta_x: 0,
            delta_y: 1,
        }),
    )
    .expect("input event should encode");
    write_message(&mut bytes, &ControlMessage::StopSession).expect("stop should encode");
    let mut sink = RecordingInputEventSink::default();

    let stop_requested = AtomicBool::new(false);
    let event = read_input_events_until_stop(&mut bytes.as_slice(), &mut sink, &stop_requested);

    assert_eq!(event, InputReaderEvent::StopSession);
    assert_eq!(
        sink.events,
        vec![InputEvent::MouseWheel {
            delta_x: 0,
            delta_y: 1,
        }]
    );
}

struct TimeoutReader;

impl io::Read for TimeoutReader {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "simulated heartbeat timeout",
        ))
    }
}

#[test]
fn spawned_input_reader_owner_joins_after_stop_session() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
    let endpoint = listener.local_addr().expect("listener addr should exist");
    let mut client = TcpStream::connect(endpoint).expect("client should connect");
    let (server, _) = listener.accept().expect("server should accept");
    let reader = spawn_input_event_reader(
        server,
        CaptureInputBounds {
            origin_x: 0,
            origin_y: 0,
            width: 1280,
            height: 720,
        },
    );

    write_message(&mut client, &ControlMessage::StopSession).expect("stop should write");

    assert_eq!(
        reader
            .receiver()
            .recv()
            .expect("input reader event should be received"),
        InputReaderEvent::StopSession
    );
    assert_eq!(
        reader.join().expect("input reader thread should join"),
        Some(InputReaderEvent::StopSession)
    );
}

#[test]
fn h264_stream_stops_cleanly_when_input_reader_stops() {
    let mut writer = Vec::new();
    let mut session = RecordingCaptureRuntime {
        frames: VecDeque::from([Some(captured_bgra_frame_with_sequence(1))]),
        attempts: Default::default(),
        block_after_empty: None,
    };
    let (sender, receiver) = mpsc::channel();
    sender
        .send(InputReaderEvent::StopSession)
        .expect("input stop should send");

    let reason = write_h264_encoded_stream_with_input_events(
        &mut writer,
        &captured_bgra_frame(),
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
    )
    .expect("stop session should end H.264 stream without error");

    assert_eq!(reason, HostSessionEndReason::StopSession);

    let mut reader = writer.as_slice();
    let frame = expect_h264_frame(read_message(&mut reader).expect("first frame should decode"));
    assert_eq!(frame.sequence_number, 0);
    assert_eq!(session.attempts.load(Ordering::SeqCst), 0);
}
