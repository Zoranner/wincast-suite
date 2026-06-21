use std::ffi::{CStr, CString};
use std::mem;
use std::ptr;

use serial_test::serial;
use wincast_unity_native::{
    WincastUnityFrameFormat, WincastUnityInputEvent, WincastUnityInputEventType,
    WincastUnityPointerButton, WincastUnityStatus, inject_input_event_for_test,
    runtime_snapshot_for_test, wincast_unity_create, wincast_unity_get_last_error,
    wincast_unity_get_status, wincast_unity_poll_input, wincast_unity_shutdown,
    wincast_unity_start, wincast_unity_submit_frame,
};

fn read_last_error(buffer_len: usize) -> String {
    let mut buffer = vec![0_u8; buffer_len];
    let written = unsafe { wincast_unity_get_last_error(buffer.as_mut_ptr().cast(), buffer.len()) };

    if buffer_len == 0 {
        assert_eq!(written, 0);
        return String::new();
    }

    let cstr = CStr::from_bytes_until_nul(&buffer).expect("error should be nul terminated");
    let message = cstr.to_str().expect("error should be valid UTF-8");
    assert_eq!(written, message.len());
    message.to_owned()
}

fn valid_config() -> CString {
    CString::new(
        r#"{
            "listen_addr": "127.0.0.1:0",
            "width": 1280,
            "height": 720,
            "fps": 30
        }"#,
    )
    .unwrap()
}

#[test]
#[serial]
fn invalid_config_json_create_fails_and_exposes_last_error() {
    let config = CString::new("{not json").unwrap();

    let handle = unsafe { wincast_unity_create(config.as_ptr()) };

    assert_eq!(handle, 0);
    let error = read_last_error(256);
    assert!(error.contains("config_json"));
}

#[test]
#[serial]
fn valid_config_create_start_status_and_shutdown() {
    let config = valid_config();

    let handle = unsafe { wincast_unity_create(config.as_ptr()) };

    assert_ne!(handle, 0);
    assert_eq!(
        unsafe { wincast_unity_get_status(handle) },
        WincastUnityStatus::Created
    );
    assert_eq!(unsafe { wincast_unity_start(handle) }, 0);
    assert_eq!(
        unsafe { wincast_unity_get_status(handle) },
        WincastUnityStatus::Started
    );
    assert_eq!(unsafe { wincast_unity_shutdown(handle) }, 0);
    assert_eq!(
        unsafe { wincast_unity_get_status(handle) },
        WincastUnityStatus::Stopped
    );
}

#[test]
#[serial]
fn config_accepts_bitrate_and_defaults_max_bitrate_for_json_compatibility() {
    let config = CString::new(
        r#"{
            "listen_addr": "127.0.0.1:0",
            "width": 1280,
            "height": 720,
            "fps": 30,
            "bitrate_kbps": 2500
        }"#,
    )
    .unwrap();

    let handle = unsafe { wincast_unity_create(config.as_ptr()) };

    assert_ne!(handle, 0);
    assert_eq!(
        unsafe { wincast_unity_get_status(handle) },
        WincastUnityStatus::Created
    );
    assert_eq!(unsafe { wincast_unity_shutdown(handle) }, 0);
}

#[test]
#[serial]
fn config_rejects_bitrate_above_max_bitrate() {
    let config = CString::new(
        r#"{
            "listen_addr": "127.0.0.1:0",
            "width": 1280,
            "height": 720,
            "fps": 30,
            "bitrate_kbps": 2500,
            "max_bitrate_kbps": 1200
        }"#,
    )
    .unwrap();

    let handle = unsafe { wincast_unity_create(config.as_ptr()) };

    assert_eq!(handle, 0);
    assert!(read_last_error(256).contains("bitrate_kbps"));
}

#[test]
#[serial]
fn submit_frame_rejects_null_pointer_or_invalid_dimensions() {
    let config = valid_config();
    let handle = unsafe { wincast_unity_create(config.as_ptr()) };
    assert_ne!(handle, 0);
    assert_eq!(unsafe { wincast_unity_start(handle) }, 0);

    let null_result = unsafe {
        wincast_unity_submit_frame(
            handle,
            ptr::null(),
            1280,
            720,
            1280 * 4,
            WincastUnityFrameFormat::Rgba8,
            1,
        )
    };
    assert_eq!(null_result, -1);
    assert!(read_last_error(256).contains("frame_ptr"));

    let frame = [0_u8; 4];
    let invalid_size_result = unsafe {
        wincast_unity_submit_frame(
            handle,
            frame.as_ptr(),
            0,
            720,
            1280 * 4,
            WincastUnityFrameFormat::Rgba8,
            2,
        )
    };
    assert_eq!(invalid_size_result, -1);
    assert!(read_last_error(256).contains("width"));

    assert_eq!(unsafe { wincast_unity_shutdown(handle) }, 0);
}

#[test]
#[serial]
fn get_last_error_writes_utf8_and_respects_buffer_length() {
    let config = CString::new("{not json").unwrap();
    assert_eq!(unsafe { wincast_unity_create(config.as_ptr()) }, 0);

    let full = read_last_error(256);
    assert!(full.is_char_boundary(full.len()));

    let mut small = [0_u8; 8];
    let written = unsafe { wincast_unity_get_last_error(small.as_mut_ptr().cast(), small.len()) };

    assert_eq!(written, 7);
    assert_eq!(small[7], 0);
    let truncated = CStr::from_bytes_until_nul(&small)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(full.starts_with(truncated));
}

#[test]
#[serial]
fn submit_frame_replaces_latest_frame_and_increments_count() {
    let config = valid_config();
    let handle = unsafe { wincast_unity_create(config.as_ptr()) };
    assert_ne!(handle, 0);
    assert_eq!(unsafe { wincast_unity_start(handle) }, 0);

    let mut first_frame = [1_u8; 16];
    assert_eq!(
        unsafe {
            wincast_unity_submit_frame(
                handle,
                first_frame.as_ptr(),
                2,
                2,
                8,
                WincastUnityFrameFormat::Rgba8,
                10,
            )
        },
        0
    );
    first_frame.fill(9);

    let mut second_frame = vec![2_u8; 24];
    assert_eq!(
        unsafe {
            wincast_unity_submit_frame(
                handle,
                second_frame.as_ptr(),
                3,
                2,
                12,
                WincastUnityFrameFormat::Bgra8,
                20,
            )
        },
        0
    );
    second_frame.fill(8);
    drop(second_frame);

    let snapshot = runtime_snapshot_for_test(handle).expect("runtime should exist");
    assert_eq!(snapshot.submitted_frame_count, 2);
    let latest = snapshot
        .latest_frame
        .expect("latest frame should be recorded");
    assert_eq!(latest.metadata.width, 3);
    assert_eq!(latest.metadata.height, 2);
    assert_eq!(latest.metadata.stride_bytes, 12);
    assert_eq!(latest.metadata.format, WincastUnityFrameFormat::Bgra8);
    assert_eq!(latest.metadata.timestamp_ns, 20);
    assert_eq!(latest.metadata.byte_len, 24);
    assert_eq!(latest.bytes, vec![2_u8; 24]);
    assert_eq!(unsafe { wincast_unity_shutdown(handle) }, 0);
}

#[test]
#[serial]
fn poll_input_outputs_injected_pointer_key_and_text_events() {
    let config = valid_config();
    let handle = unsafe { wincast_unity_create(config.as_ptr()) };
    assert_ne!(handle, 0);

    inject_input_event_for_test(
        handle,
        WincastUnityInputEvent {
            event_type: WincastUnityInputEventType::PointerMove,
            pointer_id: 7,
            x: 10.0,
            y: 20.0,
            delta_x: 1.0,
            delta_y: -1.0,
            button: WincastUnityPointerButton::None,
            key_code: 0,
            unicode_scalar: 0,
            timestamp_microseconds: 101,
        },
    )
    .expect("pointer event should enqueue");
    inject_input_event_for_test(
        handle,
        WincastUnityInputEvent {
            event_type: WincastUnityInputEventType::KeyDown,
            pointer_id: 0,
            x: 0.0,
            y: 0.0,
            delta_x: 0.0,
            delta_y: 0.0,
            button: WincastUnityPointerButton::None,
            key_code: 65,
            unicode_scalar: 0,
            timestamp_microseconds: 102,
        },
    )
    .expect("key event should enqueue");
    inject_input_event_for_test(
        handle,
        WincastUnityInputEvent {
            event_type: WincastUnityInputEventType::Text,
            pointer_id: 0,
            x: 0.0,
            y: 0.0,
            delta_x: 0.0,
            delta_y: 0.0,
            button: WincastUnityPointerButton::None,
            key_code: 0,
            unicode_scalar: '中' as u32,
            timestamp_microseconds: 103,
        },
    )
    .expect("text event should enqueue");

    let mut events = vec![WincastUnityInputEvent::default(); 4];
    let count = unsafe {
        wincast_unity_poll_input(
            handle,
            events.as_mut_ptr().cast(),
            events.len() * mem::size_of::<WincastUnityInputEvent>(),
        )
    };

    assert_eq!(count, 3);
    assert_eq!(
        events[0].event_type,
        WincastUnityInputEventType::PointerMove
    );
    assert_eq!(events[0].pointer_id, 7);
    assert_eq!(events[0].x, 10.0);
    assert_eq!(events[1].event_type, WincastUnityInputEventType::KeyDown);
    assert_eq!(events[1].key_code, 65);
    assert_eq!(events[2].event_type, WincastUnityInputEventType::Text);
    assert_eq!(events[2].unicode_scalar, '中' as u32);

    let count_after_drain = unsafe {
        wincast_unity_poll_input(
            handle,
            events.as_mut_ptr().cast(),
            events.len() * mem::size_of::<WincastUnityInputEvent>(),
        )
    };
    assert_eq!(count_after_drain, 0);
    assert_eq!(unsafe { wincast_unity_shutdown(handle) }, 0);
}

#[test]
#[serial]
fn full_input_queue_drops_old_move_but_keeps_button_and_key_events() {
    let config = valid_config();
    let handle = unsafe { wincast_unity_create(config.as_ptr()) };
    assert_ne!(handle, 0);

    for index in 0..96 {
        inject_input_event_for_test(
            handle,
            WincastUnityInputEvent {
                event_type: WincastUnityInputEventType::PointerMove,
                pointer_id: 1,
                x: index as f32,
                y: index as f32,
                delta_x: 0.0,
                delta_y: 0.0,
                button: WincastUnityPointerButton::None,
                key_code: 0,
                unicode_scalar: 0,
                timestamp_microseconds: index,
            },
        )
        .expect("move may be coalesced but should not fail");
    }

    inject_input_event_for_test(
        handle,
        WincastUnityInputEvent {
            event_type: WincastUnityInputEventType::PointerDown,
            pointer_id: 1,
            x: 96.0,
            y: 96.0,
            delta_x: 0.0,
            delta_y: 0.0,
            button: WincastUnityPointerButton::Left,
            key_code: 0,
            unicode_scalar: 0,
            timestamp_microseconds: 200,
        },
    )
    .expect("button event must be preserved");
    inject_input_event_for_test(
        handle,
        WincastUnityInputEvent {
            event_type: WincastUnityInputEventType::KeyUp,
            pointer_id: 0,
            x: 0.0,
            y: 0.0,
            delta_x: 0.0,
            delta_y: 0.0,
            button: WincastUnityPointerButton::None,
            key_code: 27,
            unicode_scalar: 0,
            timestamp_microseconds: 201,
        },
    )
    .expect("key event must be preserved");

    let mut events = vec![WincastUnityInputEvent::default(); 128];
    let count = unsafe {
        wincast_unity_poll_input(
            handle,
            events.as_mut_ptr().cast(),
            events.len() * mem::size_of::<WincastUnityInputEvent>(),
        )
    };
    let events = &events[..count];

    assert!(events.iter().any(|event| {
        event.event_type == WincastUnityInputEventType::PointerDown
            && event.button == WincastUnityPointerButton::Left
    }));
    assert!(events.iter().any(
        |event| event.event_type == WincastUnityInputEventType::KeyUp && event.key_code == 27
    ));
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == WincastUnityInputEventType::PointerMove)
            .count(),
        1
    );
    assert_eq!(unsafe { wincast_unity_shutdown(handle) }, 0);
}

#[test]
#[serial]
fn poll_input_with_invalid_handle_reports_error() {
    let mut events = vec![WincastUnityInputEvent::default(); 1];

    let count = unsafe {
        wincast_unity_poll_input(
            99_999,
            events.as_mut_ptr().cast(),
            events.len() * mem::size_of::<WincastUnityInputEvent>(),
        )
    };

    assert_eq!(count, 0);
    assert!(read_last_error(256).contains("runtime handle is invalid"));
}
