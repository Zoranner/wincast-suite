use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MonitorPowerError {
    message: String,
}

impl MonitorPowerError {
    #[cfg(any(not(windows), test))]
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for MonitorPowerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for MonitorPowerError {}

#[cfg(test)]
pub(crate) fn monitor_power_error(message: impl Into<String>) -> MonitorPowerError {
    make_monitor_power_error(message)
}

pub(crate) trait MonitorPowerController {
    fn turn_off_monitor(&mut self) -> Result<(), MonitorPowerError>;
}

#[derive(Debug, Default)]
pub(crate) struct StdMonitorPowerController;

impl MonitorPowerController for StdMonitorPowerController {
    fn turn_off_monitor(&mut self) -> Result<(), MonitorPowerError> {
        platform_turn_off_monitor()
    }
}

#[cfg(windows)]
fn platform_turn_off_monitor() -> Result<(), MonitorPowerError> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        HWND_BROADCAST, SC_MONITORPOWER, SendMessageW, WM_SYSCOMMAND,
    };

    const MONITOR_POWER_OFF: isize = 2;

    unsafe {
        SendMessageW(
            HWND_BROADCAST,
            WM_SYSCOMMAND,
            SC_MONITORPOWER as usize,
            MONITOR_POWER_OFF,
        );
    }
    Ok(())
}

#[cfg(not(windows))]
fn platform_turn_off_monitor() -> Result<(), MonitorPowerError> {
    Err(make_monitor_power_error(
        "当前平台不支持关闭宿主端显示器：仅 Windows 支持显示器电源控制",
    ))
}

#[cfg(any(not(windows), test))]
fn make_monitor_power_error(message: impl Into<String>) -> MonitorPowerError {
    MonitorPowerError::new(message)
}

#[cfg(test)]
mod tests {
    #[test]
    fn non_windows_monitor_power_control_reports_clear_error() {
        #[cfg(not(windows))]
        {
            let error = super::platform_turn_off_monitor().expect_err("non-Windows should fail");

            assert_eq!(
                error.to_string(),
                "当前平台不支持关闭宿主端显示器：仅 Windows 支持显示器电源控制"
            );
        }
    }
}
