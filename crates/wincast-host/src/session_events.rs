use crate::session_state::HostDesktopEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsSessionChange {
    ConsoleConnect,
    ConsoleDisconnect,
    SessionLock,
    SessionUnlock,
    SessionLogon,
    SessionLogoff,
    Other(u32),
}

impl WindowsSessionChange {
    pub fn to_host_desktop_event(self) -> Option<HostDesktopEvent> {
        match self {
            Self::ConsoleConnect | Self::Other(_) => None,
            Self::ConsoleDisconnect | Self::SessionLogoff => Some(HostDesktopEvent::LoggedOut),
            Self::SessionLock => Some(HostDesktopEvent::Locked),
            Self::SessionUnlock => Some(HostDesktopEvent::Unlocked),
            Self::SessionLogon => Some(HostDesktopEvent::UserLoggedIn),
        }
    }

    #[cfg(windows)]
    pub fn from_wts_session_change(reason: u32) -> Self {
        match reason {
            WTS_CONSOLE_CONNECT => Self::ConsoleConnect,
            WTS_CONSOLE_DISCONNECT => Self::ConsoleDisconnect,
            WTS_SESSION_LOCK => Self::SessionLock,
            WTS_SESSION_UNLOCK => Self::SessionUnlock,
            WTS_SESSION_LOGON => Self::SessionLogon,
            WTS_SESSION_LOGOFF => Self::SessionLogoff,
            other => Self::Other(other),
        }
    }
}

#[cfg(windows)]
const WTS_CONSOLE_CONNECT: u32 = 0x1;
#[cfg(windows)]
const WTS_CONSOLE_DISCONNECT: u32 = 0x2;
#[cfg(windows)]
const WTS_SESSION_LOGON: u32 = 0x5;
#[cfg(windows)]
const WTS_SESSION_LOGOFF: u32 = 0x6;
#[cfg(windows)]
const WTS_SESSION_LOCK: u32 = 0x7;
#[cfg(windows)]
const WTS_SESSION_UNLOCK: u32 = 0x8;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_lock_to_locked_desktop_event() {
        assert_eq!(
            WindowsSessionChange::SessionLock.to_host_desktop_event(),
            Some(HostDesktopEvent::Locked)
        );
    }

    #[test]
    fn maps_unlock_to_unlocked_desktop_event() {
        assert_eq!(
            WindowsSessionChange::SessionUnlock.to_host_desktop_event(),
            Some(HostDesktopEvent::Unlocked)
        );
    }

    #[test]
    fn maps_logon_to_user_logged_in_desktop_event() {
        assert_eq!(
            WindowsSessionChange::SessionLogon.to_host_desktop_event(),
            Some(HostDesktopEvent::UserLoggedIn)
        );
    }

    #[test]
    fn maps_logoff_to_logged_out_desktop_event() {
        assert_eq!(
            WindowsSessionChange::SessionLogoff.to_host_desktop_event(),
            Some(HostDesktopEvent::LoggedOut)
        );
    }

    #[test]
    fn maps_console_disconnect_to_logged_out_desktop_event() {
        assert_eq!(
            WindowsSessionChange::ConsoleDisconnect.to_host_desktop_event(),
            Some(HostDesktopEvent::LoggedOut)
        );
    }

    #[test]
    fn ignores_unrelated_session_change() {
        assert_eq!(WindowsSessionChange::Other(0).to_host_desktop_event(), None);
    }
}
