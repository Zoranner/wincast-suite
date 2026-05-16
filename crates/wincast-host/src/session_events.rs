use crate::session_state::{
    HostDesktopEvent, SessionEvent, SessionStateMachine, SharedSessionState,
};

#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::HWND,
    System::RemoteDesktop::{
        NOTIFY_FOR_THIS_SESSION, WTS_CURRENT_SERVER_HANDLE, WTSFreeMemory,
        WTSGetActiveConsoleSessionId, WTSINFOEXW, WTSQuerySessionInformationW,
        WTSRegisterSessionNotification, WTSSessionInfoEx, WTSUnRegisterSessionNotification,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DetectedDesktopSession {
    pub user_logged_in: bool,
    pub locked: bool,
}

pub trait DesktopSessionDetector {
    fn detect_desktop_session(&self) -> Result<DetectedDesktopSession, String>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PlatformDesktopSessionDetector;

impl DesktopSessionDetector for PlatformDesktopSessionDetector {
    #[cfg(windows)]
    fn detect_desktop_session(&self) -> Result<DetectedDesktopSession, String> {
        detect_windows_desktop_session()
    }

    #[cfg(not(windows))]
    fn detect_desktop_session(&self) -> Result<DetectedDesktopSession, String> {
        Err("运行时桌面状态探测只支持 Windows 平台".to_owned())
    }
}

pub fn detect_desktop_session() -> Result<DetectedDesktopSession, String> {
    PlatformDesktopSessionDetector.detect_desktop_session()
}

pub fn apply_detected_desktop_session(
    machine: &mut SessionStateMachine,
    detected: DetectedDesktopSession,
) {
    if !detected.user_logged_in {
        machine.apply(SessionEvent::UserLoggedOut);
        return;
    }

    machine.apply(SessionEvent::UserLoggedIn);
    if detected.locked {
        machine.apply(SessionEvent::SessionLocked);
    } else {
        machine.apply(SessionEvent::SessionUnlocked);
    }
}

pub fn shared_session_state_from_detected_desktop_session(
    detected: DetectedDesktopSession,
) -> SharedSessionState {
    let mut machine = SessionStateMachine::new();
    apply_detected_desktop_session(&mut machine, detected);
    SharedSessionState::from_machine(machine)
}

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

#[cfg(windows)]
fn detect_windows_desktop_session() -> Result<DetectedDesktopSession, String> {
    // SAFETY: WTSGetActiveConsoleSessionId takes no pointers and has no Rust-side lifetime
    // requirements. The sentinel u32::MAX is handled as "no active console session".
    let session_id = unsafe { WTSGetActiveConsoleSessionId() };
    if session_id == NO_ACTIVE_CONSOLE_SESSION {
        return Ok(DetectedDesktopSession {
            user_logged_in: false,
            locked: false,
        });
    }

    query_session_info_ex(session_id)
}

#[cfg(windows)]
fn query_session_info_ex(session_id: u32) -> Result<DetectedDesktopSession, String> {
    let mut buffer = std::ptr::null_mut();
    let mut bytes_returned = 0_u32;
    // SAFETY: buffer and bytes_returned are valid out-parameters. WTS returns an allocated buffer
    // on success, which is released with WTSFreeMemory below after the contents are copied.
    let result = unsafe {
        WTSQuerySessionInformationW(
            WTS_CURRENT_SERVER_HANDLE,
            session_id,
            WTSSessionInfoEx,
            &mut buffer,
            &mut bytes_returned,
        )
    };

    if result == 0 {
        return Err(format!(
            "查询 Windows 会话扩展状态失败，session_id={session_id}"
        ));
    }

    let detected = read_session_info_ex_buffer(buffer, bytes_returned);
    if !buffer.is_null() {
        // SAFETY: buffer was allocated by WTSQuerySessionInformationW on success and is freed once
        // after read_session_info_ex_buffer has copied the fixed-size WTSINFOEXW value.
        unsafe { WTSFreeMemory(buffer.cast()) };
    }
    detected
}

#[cfg(windows)]
fn read_session_info_ex_buffer(
    buffer: windows_sys::core::PWSTR,
    bytes_returned: u32,
) -> Result<DetectedDesktopSession, String> {
    if buffer.is_null() || bytes_returned < std::mem::size_of::<WTSINFOEXW>() as u32 {
        return Err("Windows 会话扩展状态返回数据无效".to_owned());
    }

    // SAFETY: buffer is non-null and bytes_returned proves it contains at least one WTSINFOEXW.
    // The value is copied before the WTS buffer is freed by the caller.
    let info = unsafe { *(buffer.cast::<WTSINFOEXW>()) };
    if info.Level != 1 {
        return Err(format!("Windows 会话扩展状态层级无效: {}", info.Level));
    }

    // SAFETY: info.Level == 1 selects the WTSInfoExLevel1 union field according to the WTS API.
    let level1 = unsafe { info.Data.WTSInfoExLevel1 };
    Ok(detected_desktop_session_from_session_info_ex_level1(
        level1.SessionState,
        level1.SessionFlags as u32,
        &level1.UserName,
    ))
}

fn detected_desktop_session_from_session_info_ex_level1(
    session_state: i32,
    session_flags: u32,
    user_name: &[u16],
) -> DetectedDesktopSession {
    let user_logged_in = user_name.first().copied().unwrap_or_default() != 0;
    let connected = matches!(
        session_state,
        WINDOWS_SESSION_STATE_ACTIVE | WINDOWS_SESSION_STATE_CONNECTED
    );
    let flag_locked = match session_flags {
        WINDOWS_SESSION_FLAG_UNLOCK => false,
        WINDOWS_SESSION_FLAG_LOCK | WINDOWS_SESSION_FLAG_UNKNOWN => true,
        _ => true,
    };

    DetectedDesktopSession {
        user_logged_in,
        locked: !user_logged_in || !connected || flag_locked,
    }
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
        // SAFETY: hwnd is supplied by the host window/message-loop owner. The API only registers
        // notification delivery for that window; a zero return is reported as failure.
        let result = unsafe { WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) };
        if result == 0 {
            Err("注册 Windows 会话通知失败".to_owned())
        } else {
            Ok(())
        }
    }

    fn unregister(&self, hwnd: SessionNotificationWindow) -> Result<(), String> {
        // SAFETY: hwnd is the same handle previously registered by this owner. If Windows rejects
        // it or it is already invalid, the zero return is reported to the caller.
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
#[cfg(windows)]
const NO_ACTIVE_CONSOLE_SESSION: u32 = u32::MAX;
const WINDOWS_SESSION_STATE_ACTIVE: i32 = 0;
const WINDOWS_SESSION_STATE_CONNECTED: i32 = 1;
const WINDOWS_SESSION_FLAG_LOCK: u32 = 0;
const WINDOWS_SESSION_FLAG_UNLOCK: u32 = 1;
const WINDOWS_SESSION_FLAG_UNKNOWN: u32 = u32::MAX;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_state::RemoteSessionStatus;
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
    fn detected_no_user_maps_to_no_user_rejection() {
        let state = shared_session_state_from_detected_desktop_session(DetectedDesktopSession {
            user_logged_in: false,
            locked: false,
        });

        assert_eq!(
            state.remote_session_status(),
            RemoteSessionStatus::Rejected {
                code: crate::session_state::ClientSessionErrorCode::NoUserLoggedIn,
                message: "当前没有 Windows 用户登录，无法启动远程会话。",
            }
        );
    }

    #[test]
    fn detected_locked_user_maps_to_locked_rejection() {
        let state = shared_session_state_from_detected_desktop_session(DetectedDesktopSession {
            user_logged_in: true,
            locked: true,
        });
        state.apply(SessionEvent::AgentStarted);

        assert_eq!(
            state.remote_session_status(),
            RemoteSessionStatus::Rejected {
                code: crate::session_state::ClientSessionErrorCode::SessionLocked,
                message: "Windows 会话已锁定，请先解锁后再启动远程会话。",
            }
        );
    }

    #[test]
    fn detected_unlocked_user_allows_after_agent_started() {
        let state = shared_session_state_from_detected_desktop_session(DetectedDesktopSession {
            user_logged_in: true,
            locked: false,
        });
        state.apply(SessionEvent::AgentStarted);

        assert_eq!(state.remote_session_status(), RemoteSessionStatus::Allowed);
    }

    #[test]
    fn session_info_ex_unlock_flag_with_username_detects_unlocked_user() {
        assert_eq!(
            detected_desktop_session_from_session_info_ex_level1(
                WINDOWS_SESSION_STATE_ACTIVE,
                WINDOWS_SESSION_FLAG_UNLOCK,
                &user_name("tester"),
            ),
            DetectedDesktopSession {
                user_logged_in: true,
                locked: false,
            }
        );
    }

    #[test]
    fn session_info_ex_lock_flag_with_username_detects_locked_user() {
        assert_eq!(
            detected_desktop_session_from_session_info_ex_level1(
                WINDOWS_SESSION_STATE_ACTIVE,
                WINDOWS_SESSION_FLAG_LOCK,
                &user_name("tester"),
            ),
            DetectedDesktopSession {
                user_logged_in: true,
                locked: true,
            }
        );
    }

    #[test]
    fn session_info_ex_unknown_flag_is_conservative_locked() {
        assert_eq!(
            detected_desktop_session_from_session_info_ex_level1(
                WINDOWS_SESSION_STATE_ACTIVE,
                WINDOWS_SESSION_FLAG_UNKNOWN,
                &user_name("tester"),
            ),
            DetectedDesktopSession {
                user_logged_in: true,
                locked: true,
            }
        );
    }

    #[test]
    fn session_info_ex_empty_username_detects_no_logged_in_user() {
        let detected = detected_desktop_session_from_session_info_ex_level1(
            WINDOWS_SESSION_STATE_ACTIVE,
            WINDOWS_SESSION_FLAG_UNLOCK,
            &user_name(""),
        );

        assert!(!detected.user_logged_in);
        assert!(detected.locked);
    }

    #[test]
    fn session_info_ex_non_active_state_is_conservative_locked() {
        assert_eq!(
            detected_desktop_session_from_session_info_ex_level1(
                4,
                WINDOWS_SESSION_FLAG_UNLOCK,
                &user_name("tester"),
            ),
            DetectedDesktopSession {
                user_logged_in: true,
                locked: true,
            }
        );
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

    fn user_name(value: &str) -> [u16; 21] {
        let mut buffer = [0_u16; 21];
        for (index, code_unit) in value.encode_utf16().take(buffer.len()).enumerate() {
            buffer[index] = code_unit;
        }
        buffer
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
