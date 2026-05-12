use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
};

use crate::agent::{
    stream::{
        HostSessionEndReason, InputReaderEvent, read_input_events_until_stop,
        write_raw_bgra_stream_with_input_events,
    },
    tests::*,
};
use wincast_protocol::{
    frame::{read_message, write_message},
    input::{ButtonState, InputEvent, Modifiers},
    ipc::SessionEndReason,
    message::ControlMessage,
    raw_frame::read_raw_bgra_frame,
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

    let event = read_input_events_until_stop(&mut bytes.as_slice(), &mut sink);

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
fn input_reader_rejects_non_input_messages() {
    let mut bytes = Vec::new();
    write_message(&mut bytes, &ControlMessage::Heartbeat).expect("heartbeat should encode");
    let mut sink = RecordingInputEventSink::default();

    let event = read_input_events_until_stop(&mut bytes.as_slice(), &mut sink);

    assert_eq!(
        event,
        InputReaderEvent::Failed("客户端输入事件消息无效: Heartbeat".to_owned())
    );
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

    let event = read_input_events_until_stop(&mut bytes.as_slice(), &mut sink);

    assert_eq!(event, InputReaderEvent::StopSession);
    assert_eq!(
        sink.events,
        vec![InputEvent::MouseWheel {
            delta_x: 0,
            delta_y: 1,
        }]
    );
}

#[test]
fn raw_bgra_stream_stops_cleanly_when_input_reader_stops() {
    let mut writer = Vec::new();
    let mut session = RecordingCaptureRuntime {
        frames: VecDeque::from([Some(captured_bgra_frame_with_sequence(1))]),
        attempts: Arc::new(AtomicUsize::new(0)),
        block_after_empty: None,
    };
    let (sender, receiver) = mpsc::channel();
    sender
        .send(InputReaderEvent::StopSession)
        .expect("input stop should send");

    let reason = write_raw_bgra_stream_with_input_events(
        &mut writer,
        &captured_bgra_frame(),
        &mut session,
        &receiver,
    )
    .expect("stop session should end raw stream without error");

    assert_eq!(reason, HostSessionEndReason::StopSession);
    assert_eq!(
        SessionEndReason::from(reason),
        SessionEndReason::ServiceRequested
    );

    let mut reader = writer.as_slice();
    assert_eq!(
        read_message(&mut reader).expect("video ready should decode"),
        ControlMessage::VideoReady
    );
    let frame = read_raw_bgra_frame(&mut reader).expect("first frame should decode");
    assert_eq!(frame.sequence_number, 0);
    assert_eq!(session.attempts.load(Ordering::SeqCst), 0);
}
