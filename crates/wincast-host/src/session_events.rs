use crate::session_state::HostDesktopEvent;

#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::HWND,
    System::RemoteDesktop::{
        NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification, WTSUnRegisterSessionNotification,
    },
};

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

pub struct SessionNotificationRegistration {
    inner: SessionNotificationRegistrationInner<PlatformSessionNotificationApi>,
}

impl SessionNotificationRegistration {
    pub fn register_session_notifications(hwnd: SessionNotificationWindow) -> Result<Self, String> {
        Ok(Self {
            inner: SessionNotificationRegistrationInner::register_with_api(
                hwnd,
                PlatformSessionNotificationApi,
            )?,
        })
    }

    pub fn unregister(&mut self) -> Result<bool, String> {
        self.inner.unregister()
    }
}

pub fn register_session_notifications(
    hwnd: SessionNotificationWindow,
) -> Result<SessionNotificationRegistration, String> {
    SessionNotificationRegistration::register_session_notifications(hwnd)
}

struct SessionNotificationRegistrationInner<A>
where
    A: SessionNotificationApi,
{
    hwnd: SessionNotificationWindow,
    api: A,
    registered: bool,
}

impl<A> SessionNotificationRegistrationInner<A>
where
    A: SessionNotificationApi,
{
    fn register_with_api(hwnd: SessionNotificationWindow, api: A) -> Result<Self, String> {
        api.register(hwnd)?;
        Ok(Self {
            hwnd,
            api,
            registered: true,
        })
    }

    pub fn unregister(&mut self) -> Result<bool, String> {
        if !self.registered {
            return Ok(false);
        }

        self.api.unregister(self.hwnd)?;
        self.registered = false;
        Ok(true)
    }
}

impl<A> Drop for SessionNotificationRegistrationInner<A>
where
    A: SessionNotificationApi,
{
    fn drop(&mut self) {
        if self.registered && self.api.unregister(self.hwnd).is_ok() {
            self.registered = false;
        }
    }
}

#[cfg(windows)]
pub type SessionNotificationWindow = HWND;

#[cfg(not(windows))]
pub type SessionNotificationWindow = isize;

trait SessionNotificationApi {
    fn register(&self, hwnd: SessionNotificationWindow) -> Result<(), String>;

    fn unregister(&self, hwnd: SessionNotificationWindow) -> Result<(), String>;
}

#[derive(Clone, Copy)]
struct PlatformSessionNotificationApi;

#[cfg(windows)]
impl SessionNotificationApi for PlatformSessionNotificationApi {
    fn register(&self, hwnd: SessionNotificationWindow) -> Result<(), String> {
        let result = unsafe { WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) };
        if result == 0 {
            Err("注册 Windows 会话通知失败".to_owned())
        } else {
            Ok(())
        }
    }

    fn unregister(&self, hwnd: SessionNotificationWindow) -> Result<(), String> {
        let result = unsafe { WTSUnRegisterSessionNotification(hwnd) };
        if result == 0 {
            Err("注销 Windows 会话通知失败".to_owned())
        } else {
            Ok(())
        }
    }
}

#[cfg(not(windows))]
impl SessionNotificationApi for PlatformSessionNotificationApi {
    fn register(&self, _hwnd: SessionNotificationWindow) -> Result<(), String> {
        Err("Windows 会话通知只支持 Windows 平台".to_owned())
    }

    fn unregister(&self, _hwnd: SessionNotificationWindow) -> Result<(), String> {
        Err("Windows 会话通知只支持 Windows 平台".to_owned())
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
    use std::{cell::RefCell, rc::Rc};

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

    #[test]
    fn drop_unregisters_session_notification_once() {
        let api = RecordingSessionNotificationApi::new();

        {
            let _registration =
                SessionNotificationRegistrationInner::register_with_api(test_hwnd(42), api.clone())
                    .unwrap();
        }

        assert_eq!(api.registered_windows(), vec![42_usize]);
        assert_eq!(api.unregistered_windows(), vec![42_usize]);
    }

    #[test]
    fn explicit_unregister_prevents_drop_from_unregistering_twice() {
        let api = RecordingSessionNotificationApi::new();

        {
            let mut registration =
                SessionNotificationRegistrationInner::register_with_api(test_hwnd(7), api.clone())
                    .unwrap();

            assert!(registration.unregister().unwrap());
            assert!(!registration.unregister().unwrap());
        }

        assert_eq!(api.registered_windows(), vec![7_usize]);
        assert_eq!(api.unregistered_windows(), vec![7_usize]);
    }

    #[derive(Clone, Default)]
    struct RecordingSessionNotificationApi {
        calls: Rc<RefCell<RecordedNotificationCalls>>,
    }

    impl RecordingSessionNotificationApi {
        fn new() -> Self {
            Self::default()
        }

        fn registered_windows(&self) -> Vec<usize> {
            self.calls
                .borrow()
                .registered
                .iter()
                .copied()
                .map(hwnd_id)
                .collect()
        }

        fn unregistered_windows(&self) -> Vec<usize> {
            self.calls
                .borrow()
                .unregistered
                .iter()
                .copied()
                .map(hwnd_id)
                .collect()
        }
    }

    impl SessionNotificationApi for RecordingSessionNotificationApi {
        fn register(&self, hwnd: SessionNotificationWindow) -> Result<(), String> {
            self.calls.borrow_mut().registered.push(hwnd);
            Ok(())
        }

        fn unregister(&self, hwnd: SessionNotificationWindow) -> Result<(), String> {
            self.calls.borrow_mut().unregistered.push(hwnd);
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordedNotificationCalls {
        registered: Vec<SessionNotificationWindow>,
        unregistered: Vec<SessionNotificationWindow>,
    }

    #[cfg(windows)]
    fn test_hwnd(value: usize) -> SessionNotificationWindow {
        value as SessionNotificationWindow
    }

    #[cfg(not(windows))]
    fn test_hwnd(value: usize) -> SessionNotificationWindow {
        value as SessionNotificationWindow
    }

    #[cfg(windows)]
    fn hwnd_id(hwnd: SessionNotificationWindow) -> usize {
        hwnd as usize
    }

    #[cfg(not(windows))]
    fn hwnd_id(hwnd: SessionNotificationWindow) -> usize {
        hwnd as usize
    }
}
