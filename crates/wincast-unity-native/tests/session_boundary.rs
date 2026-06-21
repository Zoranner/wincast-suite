use std::{
    ffi::CString,
    mem,
    net::{TcpListener, TcpStream},
    time::{Duration, Instant},
};

use serial_test::serial;
use wincast_protocol::{
    config::VideoCodec,
    frame::{read_message, write_message},
    handshake::send_client_hello,
    input::{ButtonState, InputEvent, Modifiers, MouseButton},
    message::ControlMessage,
};
use wincast_unity_native::{
    WincastUnityFrameFormat, WincastUnityInputEvent, WincastUnityInputEventType,
    WincastUnityPointerButton, wincast_unity_create, wincast_unity_poll_input,
    wincast_unity_shutdown, wincast_unity_start, wincast_unity_submit_frame,
};

#[test]
#[serial]
fn native_listener_accepts_session_and_enqueues_protocol_input_event() {
    let endpoint = reserve_loopback_endpoint();
    let config = CString::new(format!(
        r#"{{
            "listen_addr": "{endpoint}",
            "width": 800,
            "height": 600,
            "fps": 30,
            "bitrate_kbps": 1200
        }}"#
    ))
    .expect("config should not contain nul");

    let handle = unsafe { wincast_unity_create(config.as_ptr()) };
    assert_ne!(handle, 0);
    assert_eq!(unsafe { wincast_unity_start(handle) }, 0);
    assert_eq!(
        unsafe { wincast_unity_start(handle) },
        0,
        "start should be idempotent after listener has started"
    );

    let mut client = connect_with_retry(&endpoint);
    send_client_hello(&mut client).expect("client hello should write");
    assert_eq!(
        read_message(&mut client).expect("native hello should read"),
        ControlMessage::Hello { version: 1 }
    );
    write_message(&mut client, &ControlMessage::StartSession).expect("start session should write");
    assert_eq!(
        read_message(&mut client).expect("session ready should read"),
        ControlMessage::SessionReady {
            width: 800,
            height: 600,
        }
    );

    write_message(
        &mut client,
        &ControlMessage::InputEvent(InputEvent::Key {
            code: 65,
            state: ButtonState::Pressed,
            modifiers: Modifiers::default(),
        }),
    )
    .expect("input event should write");
    write_message(&mut client, &ControlMessage::StopSession).expect("stop session should write");

    let event = poll_one_input_event(handle);
    assert_eq!(event.event_type, WincastUnityInputEventType::KeyDown);
    assert_eq!(event.key_code, 65);

    assert_eq!(unsafe { wincast_unity_shutdown(handle) }, 0);
}

#[test]
#[serial]
fn native_listener_maps_protocol_input_variants_to_unity_events() {
    let endpoint = reserve_loopback_endpoint();
    let handle = create_started_runtime(&endpoint, 320, 240);
    let mut client = connect_started_session(&endpoint, 320, 240);

    for input in [
        InputEvent::MouseMove { x: 10.0, y: 20.0 },
        InputEvent::MouseMoveAbsolute { x: 30.0, y: 40.0 },
        InputEvent::MouseButton {
            button: MouseButton::Left,
            state: ButtonState::Pressed,
        },
        InputEvent::MouseButton {
            button: MouseButton::Left,
            state: ButtonState::Released,
        },
        InputEvent::MouseWheel {
            delta_x: -1,
            delta_y: 2,
        },
        InputEvent::Key {
            code: 65,
            state: ButtonState::Pressed,
            modifiers: Modifiers::default(),
        },
        InputEvent::Key {
            code: 65,
            state: ButtonState::Released,
            modifiers: Modifiers::default(),
        },
    ] {
        write_message(&mut client, &ControlMessage::InputEvent(input))
            .expect("input event should write");
    }

    let events = poll_input_events(handle, 6);
    write_message(&mut client, &ControlMessage::StopSession).expect("stop session should write");
    assert!(events.iter().any(|event| {
        event.event_type == WincastUnityInputEventType::PointerMove
            && event.x == 30.0
            && event.y == 40.0
    }));
    assert!(events.iter().any(|event| {
        event.event_type == WincastUnityInputEventType::PointerDown
            && event.button == WincastUnityPointerButton::Left
    }));
    assert!(events.iter().any(|event| {
        event.event_type == WincastUnityInputEventType::PointerUp
            && event.button == WincastUnityPointerButton::Left
    }));
    assert!(events.iter().any(|event| {
        event.event_type == WincastUnityInputEventType::PointerScroll
            && event.delta_x == -1.0
            && event.delta_y == 2.0
    }));
    assert!(events.iter().any(|event| {
        event.event_type == WincastUnityInputEventType::KeyDown && event.key_code == 65
    }));
    assert!(events.iter().any(|event| {
        event.event_type == WincastUnityInputEventType::KeyUp && event.key_code == 65
    }));

    assert_eq!(unsafe { wincast_unity_shutdown(handle) }, 0);
}

#[test]
#[serial]
fn native_listener_sends_latest_submitted_frame_as_h264_encoded_video_frame() {
    let endpoint = reserve_loopback_endpoint();
    let handle = create_started_runtime(&endpoint, 64, 48);
    let mut client = connect_started_session(&endpoint, 64, 48);

    let timestamp_ns = 123_456_789;
    let frame = rgba_test_frame(64, 48);
    assert_eq!(
        unsafe {
            wincast_unity_submit_frame(
                handle,
                frame.as_ptr(),
                64,
                48,
                64 * 4,
                WincastUnityFrameFormat::Rgba8,
                timestamp_ns,
            )
        },
        0
    );

    let encoded = read_encoded_video_frame(&mut client);
    assert_eq!(encoded.codec, VideoCodec::H264);
    assert_eq!(encoded.width, 64);
    assert_eq!(encoded.height, 48);
    assert_eq!(encoded.sequence_number, 1);
    assert_eq!(encoded.timestamp_ns, timestamp_ns);
    assert!(!encoded.bytes.is_empty());

    write_message(&mut client, &ControlMessage::StopSession).expect("stop session should write");
    assert_eq!(
        read_message(&mut client).expect("goodbye should read"),
        ControlMessage::Goodbye
    );
    assert_eq!(unsafe { wincast_unity_shutdown(handle) }, 0);
}

#[test]
#[serial]
fn native_listener_ends_session_on_stop_session_without_blocking_runtime_shutdown() {
    let endpoint = reserve_loopback_endpoint();
    let handle = create_started_runtime(&endpoint, 64, 48);
    let mut client = connect_started_session(&endpoint, 64, 48);

    write_message(&mut client, &ControlMessage::StopSession).expect("stop session should write");

    assert_eq!(
        read_message(&mut client).expect("goodbye should read"),
        ControlMessage::Goodbye
    );
    assert_shutdown_completes(handle);
}

#[test]
#[serial]
fn native_listener_ends_session_on_goodbye_without_blocking_runtime_shutdown() {
    let endpoint = reserve_loopback_endpoint();
    let handle = create_started_runtime(&endpoint, 64, 48);
    let mut client = connect_started_session(&endpoint, 64, 48);

    write_message(&mut client, &ControlMessage::Goodbye).expect("goodbye should write");

    assert_eq!(
        read_message(&mut client).expect("goodbye response should read"),
        ControlMessage::Goodbye
    );
    assert_shutdown_completes(handle);
}

#[test]
#[serial]
fn native_listener_treats_client_disconnect_as_session_end_without_corrupting_runtime() {
    let endpoint = reserve_loopback_endpoint();
    let handle = create_started_runtime(&endpoint, 64, 48);
    let client = connect_started_session(&endpoint, 64, 48);

    drop(client);

    let frame = rgba_test_frame(64, 48);
    assert_eq!(
        unsafe {
            wincast_unity_submit_frame(
                handle,
                frame.as_ptr(),
                64,
                48,
                64 * 4,
                WincastUnityFrameFormat::Rgba8,
                987_654_321,
            )
        },
        0,
        "runtime should still accept frame submission after client EOF"
    );
    assert_shutdown_completes(handle);
}

fn reserve_loopback_endpoint() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("port reservation should bind");
    let endpoint = listener
        .local_addr()
        .expect("reserved endpoint should be readable")
        .to_string();
    drop(listener);
    endpoint
}

fn assert_shutdown_completes(handle: u64) {
    let started = Instant::now();
    assert_eq!(unsafe { wincast_unity_shutdown(handle) }, 0);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "shutdown should not wait on a completed or disconnected session"
    );
}

fn create_started_runtime(endpoint: &str, width: u32, height: u32) -> u64 {
    let config = CString::new(format!(
        r#"{{
            "listen_addr": "{endpoint}",
            "width": {width},
            "height": {height},
            "fps": 30,
            "bitrate_kbps": 1200
        }}"#
    ))
    .expect("config should not contain nul");

    let handle = unsafe { wincast_unity_create(config.as_ptr()) };
    assert_ne!(handle, 0);
    assert_eq!(unsafe { wincast_unity_start(handle) }, 0);
    handle
}

fn connect_started_session(endpoint: &str, width: u32, height: u32) -> TcpStream {
    let mut client = connect_with_retry(endpoint);
    send_client_hello(&mut client).expect("client hello should write");
    assert_eq!(
        read_message(&mut client).expect("native hello should read"),
        ControlMessage::Hello { version: 1 }
    );
    write_message(&mut client, &ControlMessage::StartSession).expect("start session should write");
    assert_eq!(
        read_message(&mut client).expect("session ready should read"),
        ControlMessage::SessionReady { width, height }
    );
    client
}

fn read_encoded_video_frame(
    client: &mut TcpStream,
) -> wincast_protocol::message::EncodedVideoFrame {
    client
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("read timeout should configure");
    loop {
        match read_message(client).expect("encoded frame should read") {
            ControlMessage::EncodedVideoFrame(frame) => return frame,
            ControlMessage::Heartbeat => {}
            message => panic!("expected encoded video frame, got {message:?}"),
        }
    }
}

fn rgba_test_frame(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            bytes.push(((x * 3) % 256) as u8);
            bytes.push(((y * 5) % 256) as u8);
            bytes.push(((x + y) % 256) as u8);
            bytes.push(255);
        }
    }
    bytes
}

fn connect_with_retry(endpoint: &str) -> TcpStream {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match TcpStream::connect(endpoint) {
            Ok(stream) => return stream,
            Err(error) if Instant::now() < deadline => {
                let _ = error;
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("client should connect to native listener: {error}"),
        }
    }
}

fn poll_one_input_event(handle: u64) -> WincastUnityInputEvent {
    poll_input_events(handle, 1)[0]
}

fn poll_input_events(handle: u64, expected_count: usize) -> Vec<WincastUnityInputEvent> {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut events = Vec::with_capacity(expected_count + 2);
    let mut buffer = vec![WincastUnityInputEvent::default(); expected_count + 2];
    loop {
        let count = unsafe {
            wincast_unity_poll_input(
                handle,
                buffer.as_mut_ptr().cast(),
                buffer.len() * mem::size_of::<WincastUnityInputEvent>(),
            )
        };
        events.extend_from_slice(&buffer[..count]);
        if count >= expected_count {
            events.truncate(expected_count);
            return events;
        }
        if events.len() >= expected_count {
            events.truncate(expected_count);
            return events;
        }
        if Instant::now() >= deadline {
            panic!(
                "native runtime should enqueue {expected_count} input events, got {}",
                events.len()
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
