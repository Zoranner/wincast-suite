use std::io::Cursor;

use wincast_protocol::ipc::{
    AgentErrorReason, AgentStatus, AgentToService, IpcFrameError, MAX_IPC_FRAME_LEN,
    ServiceToAgent, SessionEndReason, decode_agent_to_service_frame, decode_service_to_agent_frame,
    encode_agent_to_service_frame, encode_service_to_agent_frame, read_agent_to_service,
    read_service_to_agent, write_agent_to_service, write_service_to_agent,
};

#[test]
fn service_to_agent_start_session_round_trips_as_json() {
    let message = ServiceToAgent::StartSession { session_id: 42 };

    let json = serde_json::to_string(&message).expect("message should serialize");
    let decoded: ServiceToAgent = serde_json::from_str(&json).expect("message should deserialize");

    assert_eq!(decoded, message);
    assert!(json.contains("StartSession"));
    assert!(json.contains("session_id"));
}

#[test]
fn service_to_agent_commands_round_trip_as_json() {
    let messages = [
        ServiceToAgent::StopSession {
            session_id: 42,
            reason: SessionEndReason::ServiceRequested,
        },
        ServiceToAgent::Shutdown,
        ServiceToAgent::QueryStatus,
    ];

    for message in messages {
        let json = serde_json::to_string(&message).expect("message should serialize");
        let decoded: ServiceToAgent =
            serde_json::from_str(&json).expect("message should deserialize");

        assert_eq!(decoded, message);
    }
}

#[test]
fn agent_to_service_status_changed_round_trips_as_json() {
    let message = AgentToService::StatusChanged {
        status: AgentStatus::Locked,
    };

    let json = serde_json::to_string(&message).expect("message should serialize");
    let decoded: AgentToService = serde_json::from_str(&json).expect("message should deserialize");

    assert_eq!(decoded, message);
    assert!(json.contains("StatusChanged"));
    assert!(json.contains("Locked"));
}

#[test]
fn agent_to_service_session_events_round_trip_as_json() {
    let messages = [
        AgentToService::SessionStarted { session_id: 42 },
        AgentToService::SessionEnded {
            session_id: 42,
            reason: SessionEndReason::DesktopUnavailable,
        },
    ];

    for message in messages {
        let json = serde_json::to_string(&message).expect("message should serialize");
        let decoded: AgentToService =
            serde_json::from_str(&json).expect("message should deserialize");

        assert_eq!(decoded, message);
    }
}

#[test]
fn agent_to_service_error_round_trips_as_json() {
    let message = AgentToService::Error {
        reason: AgentErrorReason::AgentFailed,
        message: "捕获循环退出".to_string(),
    };

    let json = serde_json::to_string(&message).expect("message should serialize");
    let decoded: AgentToService = serde_json::from_str(&json).expect("message should deserialize");

    assert_eq!(decoded, message);
    assert!(json.contains("AgentFailed"));
}

#[test]
fn ipc_frame_round_trips_service_to_agent_message() {
    let message = ServiceToAgent::StartSession { session_id: 7 };
    let mut bytes = Vec::new();

    write_service_to_agent(&mut bytes, &message).expect("service message should encode");

    let decoded =
        read_service_to_agent(&mut Cursor::new(bytes)).expect("service message should decode");
    assert_eq!(decoded, message);

    let frame = encode_service_to_agent_frame(&message).expect("service frame should encode");
    let decoded_frame = decode_service_to_agent_frame(&frame).expect("service frame should decode");
    assert_eq!(decoded_frame, message);
}

#[test]
fn ipc_frame_round_trips_agent_to_service_message() {
    let message = AgentToService::StatusChanged {
        status: AgentStatus::Ready,
    };
    let mut bytes = Vec::new();

    write_agent_to_service(&mut bytes, &message).expect("agent message should encode");

    let decoded =
        read_agent_to_service(&mut Cursor::new(bytes)).expect("agent message should decode");
    assert_eq!(decoded, message);

    let frame = encode_agent_to_service_frame(&message).expect("agent frame should encode");
    let decoded_frame = decode_agent_to_service_frame(&frame).expect("agent frame should decode");
    assert_eq!(decoded_frame, message);
}

#[test]
fn ipc_frame_reads_consecutive_messages_from_same_stream() {
    let first = ServiceToAgent::QueryStatus;
    let second = ServiceToAgent::StopSession {
        session_id: 9,
        reason: SessionEndReason::Locked,
    };
    let mut bytes = Vec::new();
    write_service_to_agent(&mut bytes, &first).expect("first message should encode");
    write_service_to_agent(&mut bytes, &second).expect("second message should encode");

    let mut cursor = Cursor::new(bytes);
    let decoded_first = read_service_to_agent(&mut cursor).expect("first message should decode");
    let decoded_second = read_service_to_agent(&mut cursor).expect("second message should decode");

    assert_eq!(decoded_first, first);
    assert_eq!(decoded_second, second);
}

#[test]
fn ipc_frame_rejects_payload_larger_than_limit_before_allocating() {
    let mut frame = Vec::new();
    frame.extend_from_slice(&((MAX_IPC_FRAME_LEN + 1) as u32).to_be_bytes());

    let err = read_service_to_agent(&mut Cursor::new(frame)).expect_err("large frame should fail");

    assert_eq!(
        err,
        IpcFrameError::PayloadTooLarge {
            actual: MAX_IPC_FRAME_LEN + 1,
            max: MAX_IPC_FRAME_LEN,
        }
    );
}

#[test]
fn ipc_frame_reports_incomplete_payload() {
    let mut frame = Vec::new();
    frame.extend_from_slice(&8_u32.to_be_bytes());
    frame.extend_from_slice(b"{}");

    let err =
        read_agent_to_service(&mut Cursor::new(frame)).expect_err("short payload should fail");

    assert!(matches!(err, IpcFrameError::IncompletePayload { .. }));
    assert!(err.to_string().contains("IPC 消息载荷不完整"));
}

#[test]
fn ipc_frame_reports_invalid_json_payload() {
    let mut frame = Vec::new();
    frame.extend_from_slice(&4_u32.to_be_bytes());
    frame.extend_from_slice(b"nope");

    let err = read_service_to_agent(&mut Cursor::new(frame)).expect_err("invalid JSON should fail");

    assert!(matches!(err, IpcFrameError::Json(_)));
    assert!(err.to_string().contains("IPC 消息 JSON 编解码失败"));
}

#[test]
fn ipc_frame_error_messages_are_clear() {
    let err = IpcFrameError::PayloadTooLarge {
        actual: MAX_IPC_FRAME_LEN + 1,
        max: MAX_IPC_FRAME_LEN,
    };

    assert!(err.to_string().contains("IPC 消息长度"));
    assert!(err.to_string().contains("超过限制"));
}
