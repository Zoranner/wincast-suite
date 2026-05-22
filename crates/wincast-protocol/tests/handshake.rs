use std::io::Cursor;

use wincast_protocol::{
    handshake::{
        PROTOCOL_VERSION, accept_client_hello, read_host_hello, read_start_session,
        reject_busy_client, send_client_hello, send_goodbye, send_session_ready,
        send_start_session,
    },
    message::{ControlMessage, ErrorCode},
};

#[test]
fn client_sends_current_protocol_hello() {
    let mut bytes = Vec::new();

    send_client_hello(&mut bytes).expect("client hello should encode");

    let message = wincast_protocol::frame::read_message(&mut Cursor::new(bytes))
        .expect("client hello should decode");
    assert_eq!(
        message,
        ControlMessage::Hello {
            version: PROTOCOL_VERSION,
        }
    );
}

#[test]
fn host_accepts_matching_protocol_hello() {
    let mut bytes = Vec::new();
    wincast_protocol::frame::write_message(
        &mut bytes,
        &ControlMessage::Hello {
            version: PROTOCOL_VERSION,
        },
    )
    .expect("hello should encode");
    let mut reader = Cursor::new(bytes);
    let mut writer = Vec::new();

    accept_client_hello(&mut reader, &mut writer).expect("matching hello should be accepted");

    let response = wincast_protocol::frame::read_message(&mut Cursor::new(writer))
        .expect("host response should decode");
    assert_eq!(
        response,
        ControlMessage::Hello {
            version: PROTOCOL_VERSION,
        }
    );
}

#[test]
fn host_rejects_mismatched_protocol_version() {
    let mut bytes = Vec::new();
    wincast_protocol::frame::write_message(
        &mut bytes,
        &ControlMessage::Hello {
            version: PROTOCOL_VERSION + 1,
        },
    )
    .expect("hello should encode");
    let mut reader = Cursor::new(bytes);
    let mut writer = Vec::new();

    accept_client_hello(&mut reader, &mut writer).expect_err("version mismatch should fail");

    let response = wincast_protocol::frame::read_message(&mut Cursor::new(writer))
        .expect("error response should decode");
    assert_eq!(
        response,
        ControlMessage::Error {
            code: ErrorCode::UnsupportedVersion,
            message: "协议版本不兼容".to_owned(),
        }
    );
}

#[test]
fn host_can_reject_busy_client_before_session_start() {
    let mut bytes = Vec::new();

    reject_busy_client(&mut bytes).expect("busy response should encode");

    let response = wincast_protocol::frame::read_message(&mut Cursor::new(bytes))
        .expect("busy response should decode");
    assert_eq!(
        response,
        ControlMessage::Error {
            code: ErrorCode::Busy,
            message: "宿主端已有客户端连接".to_owned(),
        }
    );
}

#[test]
fn client_sends_start_session_after_hello_exchange() {
    let mut bytes = Vec::new();

    send_start_session(&mut bytes).expect("start session should encode");

    let message = wincast_protocol::frame::read_message(&mut Cursor::new(bytes))
        .expect("start session should decode");
    assert_eq!(message, ControlMessage::StartSession);
}

#[test]
fn host_reads_start_session_after_hello_exchange() {
    let mut bytes = Vec::new();
    wincast_protocol::frame::write_message(&mut bytes, &ControlMessage::StartSession)
        .expect("start session should encode");

    read_start_session(&mut Cursor::new(bytes)).expect("start session should be accepted");
}

#[test]
fn host_rejects_unexpected_message_when_waiting_for_start_session() {
    let mut bytes = Vec::new();
    wincast_protocol::frame::write_message(&mut bytes, &ControlMessage::Heartbeat)
        .expect("heartbeat should encode");

    read_start_session(&mut Cursor::new(bytes)).expect_err("heartbeat is not start session");
}

#[test]
fn host_sends_session_ready_with_video_size() {
    let mut bytes = Vec::new();

    send_session_ready(&mut bytes, 1280, 720).expect("session ready should encode");

    let message = wincast_protocol::frame::read_message(&mut Cursor::new(bytes))
        .expect("session ready should decode");
    assert_eq!(
        message,
        ControlMessage::SessionReady {
            width: 1280,
            height: 720,
        }
    );
}

#[test]
fn host_sends_goodbye_when_runtime_chain_stops_at_control_stage() {
    let mut bytes = Vec::new();

    send_goodbye(&mut bytes).expect("goodbye should encode");

    let message = wincast_protocol::frame::read_message(&mut Cursor::new(bytes))
        .expect("goodbye should decode");
    assert_eq!(message, ControlMessage::Goodbye);
}

#[test]
fn client_accepts_matching_host_hello() {
    let mut bytes = Vec::new();
    wincast_protocol::frame::write_message(
        &mut bytes,
        &ControlMessage::Hello {
            version: PROTOCOL_VERSION,
        },
    )
    .expect("host hello should encode");

    read_host_hello(&mut Cursor::new(bytes)).expect("matching host hello should be accepted");
}

#[test]
fn client_rejects_host_error_response() {
    let mut bytes = Vec::new();
    wincast_protocol::frame::write_message(
        &mut bytes,
        &ControlMessage::Error {
            code: ErrorCode::Busy,
            message: "宿主端已有客户端连接".to_owned(),
        },
    )
    .expect("error response should encode");

    read_host_hello(&mut Cursor::new(bytes)).expect_err("host error should fail handshake");
}
