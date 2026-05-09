use wincast_protocol::ipc::{
    AgentErrorReason, AgentStatus, AgentToService, ServiceToAgent, SessionEndReason,
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
        message: "capture loop exited".to_string(),
    };

    let json = serde_json::to_string(&message).expect("message should serialize");
    let decoded: AgentToService = serde_json::from_str(&json).expect("message should deserialize");

    assert_eq!(decoded, message);
    assert!(json.contains("AgentFailed"));
}
