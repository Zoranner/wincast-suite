use wincast_protocol::message::ErrorCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    NoUserLoggedIn,
    Unlocked,
    Locked,
    AgentUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEvent {
    UserLoggedIn,
    SessionUnlocked,
    SessionLocked,
    UserLoggedOut,
    AgentStarted,
    AgentExited,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostDesktopEvent {
    UserLoggedIn,
    Locked,
    Unlocked,
    LoggedOut,
    AgentStarted,
    AgentExited,
}

impl From<HostDesktopEvent> for SessionEvent {
    fn from(event: HostDesktopEvent) -> Self {
        match event {
            HostDesktopEvent::UserLoggedIn => Self::UserLoggedIn,
            HostDesktopEvent::Locked => Self::SessionLocked,
            HostDesktopEvent::Unlocked => Self::SessionUnlocked,
            HostDesktopEvent::LoggedOut => Self::UserLoggedOut,
            HostDesktopEvent::AgentStarted => Self::AgentStarted,
            HostDesktopEvent::AgentExited => Self::AgentExited,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientSessionErrorCode {
    NoUserLoggedIn,
    SessionLocked,
    AgentUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteSessionStatus {
    Allowed,
    Rejected {
        code: ClientSessionErrorCode,
        message: &'static str,
    },
}

impl ClientSessionErrorCode {
    pub fn to_protocol_error_code(self) -> ErrorCode {
        match self {
            Self::NoUserLoggedIn => ErrorCode::NoUserLoggedIn,
            Self::SessionLocked => ErrorCode::SessionLocked,
            Self::AgentUnavailable => ErrorCode::AgentUnavailable,
        }
    }
}

impl RemoteSessionStatus {
    pub fn to_protocol_error(self) -> Option<(ErrorCode, &'static str)> {
        match self {
            Self::Allowed => None,
            Self::Rejected { code, message } => Some((code.to_protocol_error_code(), message)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionStateMachine {
    state: SessionState,
    agent_running: bool,
}

impl Default for SessionStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionStateMachine {
    pub fn new() -> Self {
        Self {
            state: SessionState::NoUserLoggedIn,
            agent_running: false,
        }
    }

    pub fn state(&self) -> SessionState {
        self.state
    }

    pub fn apply(&mut self, event: SessionEvent) {
        match event {
            SessionEvent::UserLoggedIn | SessionEvent::SessionUnlocked => {
                self.state = SessionState::Unlocked;
            }
            SessionEvent::SessionLocked => {
                if self.state != SessionState::NoUserLoggedIn {
                    self.state = SessionState::Locked;
                }
            }
            SessionEvent::UserLoggedOut => {
                self.state = SessionState::NoUserLoggedIn;
                self.agent_running = false;
            }
            SessionEvent::AgentStarted => {
                self.agent_running = true;
                if self.state != SessionState::NoUserLoggedIn {
                    self.state = SessionState::Unlocked;
                }
            }
            SessionEvent::AgentExited => {
                self.agent_running = false;
                if self.state != SessionState::NoUserLoggedIn {
                    self.state = SessionState::AgentUnavailable;
                }
            }
        }
    }

    pub fn apply_host_desktop_event(&mut self, event: HostDesktopEvent) {
        self.apply(event.into());
    }

    pub fn remote_session_status(&self) -> RemoteSessionStatus {
        match (self.state, self.agent_running) {
            (SessionState::Unlocked, true) => RemoteSessionStatus::Allowed,
            (SessionState::NoUserLoggedIn, _) => RemoteSessionStatus::Rejected {
                code: ClientSessionErrorCode::NoUserLoggedIn,
                message: "当前没有 Windows 用户登录，无法启动远程会话。",
            },
            (SessionState::Locked, _) => RemoteSessionStatus::Rejected {
                code: ClientSessionErrorCode::SessionLocked,
                message: "Windows 会话已锁定，请先解锁后再启动远程会话。",
            },
            (SessionState::AgentUnavailable, _) | (SessionState::Unlocked, false) => {
                RemoteSessionStatus::Rejected {
                    code: ClientSessionErrorCode::AgentUnavailable,
                    message: "宿主端 Agent 不可用，正在等待重新拉起。",
                }
            }
        }
    }

    pub fn should_start_agent(&self) -> bool {
        self.state != SessionState::NoUserLoggedIn
            && self.state != SessionState::Locked
            && !self.agent_running
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wincast_protocol::message::ErrorCode;

    #[test]
    fn denies_remote_session_when_no_user_is_logged_in() {
        let machine = SessionStateMachine::new();

        assert_eq!(machine.state(), SessionState::NoUserLoggedIn);
        assert_eq!(
            machine.remote_session_status(),
            RemoteSessionStatus::Rejected {
                code: ClientSessionErrorCode::NoUserLoggedIn,
                message: "当前没有 Windows 用户登录，无法启动远程会话。",
            }
        );
        assert!(!machine.should_start_agent());
    }

    #[test]
    fn allows_remote_session_after_login_and_agent_start() {
        let mut machine = SessionStateMachine::new();

        machine.apply(SessionEvent::UserLoggedIn);
        assert_eq!(machine.state(), SessionState::Unlocked);
        assert!(machine.should_start_agent());

        machine.apply(SessionEvent::AgentStarted);
        assert_eq!(
            machine.remote_session_status(),
            RemoteSessionStatus::Allowed
        );
        assert!(!machine.should_start_agent());
    }

    #[test]
    fn rejects_remote_session_while_locked_and_restores_after_unlock() {
        let mut machine = SessionStateMachine::new();

        machine.apply(SessionEvent::UserLoggedIn);
        machine.apply(SessionEvent::AgentStarted);
        machine.apply(SessionEvent::SessionLocked);

        assert_eq!(machine.state(), SessionState::Locked);
        assert_eq!(
            machine.remote_session_status(),
            RemoteSessionStatus::Rejected {
                code: ClientSessionErrorCode::SessionLocked,
                message: "Windows 会话已锁定，请先解锁后再启动远程会话。",
            }
        );
        assert!(!machine.should_start_agent());

        machine.apply(SessionEvent::SessionUnlocked);

        assert_eq!(machine.state(), SessionState::Unlocked);
        assert_eq!(
            machine.remote_session_status(),
            RemoteSessionStatus::Allowed
        );
    }

    #[test]
    fn logout_releases_session_and_requires_new_login() {
        let mut machine = SessionStateMachine::new();

        machine.apply(SessionEvent::UserLoggedIn);
        machine.apply(SessionEvent::AgentStarted);
        machine.apply(SessionEvent::SessionLocked);
        machine.apply(SessionEvent::UserLoggedOut);

        assert_eq!(machine.state(), SessionState::NoUserLoggedIn);
        assert_eq!(
            machine.remote_session_status(),
            RemoteSessionStatus::Rejected {
                code: ClientSessionErrorCode::NoUserLoggedIn,
                message: "当前没有 Windows 用户登录，无法启动远程会话。",
            }
        );
        assert!(!machine.should_start_agent());
    }

    #[test]
    fn agent_exit_marks_session_unavailable_until_agent_restarts() {
        let mut machine = SessionStateMachine::new();

        machine.apply(SessionEvent::UserLoggedIn);
        machine.apply(SessionEvent::AgentStarted);
        machine.apply(SessionEvent::AgentExited);

        assert_eq!(machine.state(), SessionState::AgentUnavailable);
        assert_eq!(
            machine.remote_session_status(),
            RemoteSessionStatus::Rejected {
                code: ClientSessionErrorCode::AgentUnavailable,
                message: "宿主端 Agent 不可用，正在等待重新拉起。",
            }
        );
        assert!(machine.should_start_agent());

        machine.apply(SessionEvent::AgentStarted);

        assert_eq!(machine.state(), SessionState::Unlocked);
        assert_eq!(
            machine.remote_session_status(),
            RemoteSessionStatus::Allowed
        );
        assert!(!machine.should_start_agent());
    }

    #[test]
    fn maps_locked_desktop_event_to_session_locked_rejection() {
        let mut machine = SessionStateMachine::new();

        machine.apply_host_desktop_event(HostDesktopEvent::UserLoggedIn);
        machine.apply_host_desktop_event(HostDesktopEvent::AgentStarted);
        machine.apply_host_desktop_event(HostDesktopEvent::Locked);

        assert_eq!(
            machine.remote_session_status().to_protocol_error(),
            Some((
                ErrorCode::SessionLocked,
                "Windows 会话已锁定，请先解锁后再启动远程会话。"
            ))
        );
    }

    #[test]
    fn maps_logged_out_desktop_event_to_no_user_rejection() {
        let mut machine = SessionStateMachine::new();

        machine.apply_host_desktop_event(HostDesktopEvent::UserLoggedIn);
        machine.apply_host_desktop_event(HostDesktopEvent::AgentStarted);
        machine.apply_host_desktop_event(HostDesktopEvent::LoggedOut);

        assert_eq!(
            machine.remote_session_status().to_protocol_error(),
            Some((
                ErrorCode::NoUserLoggedIn,
                "当前没有 Windows 用户登录，无法启动远程会话。"
            ))
        );
    }

    #[test]
    fn maps_agent_exit_desktop_event_to_agent_unavailable_rejection() {
        let mut machine = SessionStateMachine::new();

        machine.apply_host_desktop_event(HostDesktopEvent::UserLoggedIn);
        machine.apply_host_desktop_event(HostDesktopEvent::AgentStarted);
        machine.apply_host_desktop_event(HostDesktopEvent::AgentExited);

        assert_eq!(
            machine.remote_session_status().to_protocol_error(),
            Some((
                ErrorCode::AgentUnavailable,
                "宿主端 Agent 不可用，正在等待重新拉起。"
            ))
        );
    }

    #[test]
    fn unlocked_and_agent_started_desktop_events_restore_allowed_status() {
        let mut machine = SessionStateMachine::new();

        machine.apply_host_desktop_event(HostDesktopEvent::UserLoggedIn);
        machine.apply_host_desktop_event(HostDesktopEvent::AgentStarted);
        machine.apply_host_desktop_event(HostDesktopEvent::Locked);
        machine.apply_host_desktop_event(HostDesktopEvent::AgentExited);
        machine.apply_host_desktop_event(HostDesktopEvent::Unlocked);
        machine.apply_host_desktop_event(HostDesktopEvent::AgentStarted);

        assert_eq!(
            machine.remote_session_status(),
            RemoteSessionStatus::Allowed
        );
        assert_eq!(machine.remote_session_status().to_protocol_error(), None);
    }
}
