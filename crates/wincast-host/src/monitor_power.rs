use std::fmt;

use wincast_protocol::config::{MonitorPowerAfterLaunch, REAL_MONITOR_POWER_OFF_REJECTION_REASON};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MonitorPowerError {
    message: String,
}

impl MonitorPowerError {
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

pub(crate) fn monitor_power_error(message: impl Into<String>) -> MonitorPowerError {
    make_monitor_power_error(message)
}

pub(crate) trait MonitorPowerController {
    fn apply_after_launch(
        &mut self,
        policy: MonitorPowerAfterLaunch,
    ) -> Result<(), MonitorPowerError>;
}

#[derive(Debug, Default)]
pub(crate) struct StdMonitorPowerController;

impl MonitorPowerController for StdMonitorPowerController {
    fn apply_after_launch(
        &mut self,
        policy: MonitorPowerAfterLaunch,
    ) -> Result<(), MonitorPowerError> {
        apply_after_launch(policy)
    }
}

fn apply_after_launch(policy: MonitorPowerAfterLaunch) -> Result<(), MonitorPowerError> {
    if policy.breaks_desktop_duplication() {
        return Err(make_monitor_power_error(
            REAL_MONITOR_POWER_OFF_REJECTION_REASON,
        ));
    }

    match policy {
        MonitorPowerAfterLaunch::Disabled => Ok(()),
        MonitorPowerAfterLaunch::WindowsPowerMessage | MonitorPowerAfterLaunch::DdcCiPowerOff => {
            unreachable!("real monitor power-off policies are rejected before platform calls")
        }
        MonitorPowerAfterLaunch::DdcCiDim => platform_ddc_ci_dim(),
    }
}

#[cfg(windows)]
fn platform_ddc_ci_dim() -> Result<(), MonitorPowerError> {
    with_single_physical_monitor(set_monitor_brightness_to_minimum)
}

#[cfg(windows)]
fn with_single_physical_monitor(
    action: impl FnOnce(windows_sys::Win32::Foundation::HANDLE) -> Result<(), MonitorPowerError>,
) -> Result<(), MonitorPowerError> {
    use windows_sys::Win32::{
        Foundation::{LPARAM, RECT},
        Graphics::Gdi::{EnumDisplayMonitors, HDC, HMONITOR},
    };

    unsafe extern "system" fn enum_monitor(
        monitor: HMONITOR,
        _hdc: HDC,
        _rect: *mut RECT,
        data: LPARAM,
    ) -> windows_sys::core::BOOL {
        let monitors = unsafe { &mut *(data as *mut Vec<HMONITOR>) };
        monitors.push(monitor);
        1
    }

    let mut monitors = Vec::new();
    let enum_result = unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(enum_monitor),
            (&mut monitors as *mut Vec<HMONITOR>) as LPARAM,
        )
    };
    if enum_result == 0 {
        return Err(last_os_error("枚举宿主端显示器失败"));
    }

    let [monitor] = monitors.as_slice() else {
        return Err(make_monitor_power_error(format!(
            "DDC/CI 显示器控制需要单显示器，当前检测到 {} 个显示器",
            monitors.len()
        )));
    };
    let physical_monitors = PhysicalMonitors::from_hmonitor(*monitor)?;
    let [physical_monitor] = physical_monitors.as_slice() else {
        return Err(make_monitor_power_error(format!(
            "DDC/CI 显示器控制需要单个物理显示器，当前检测到 {} 个物理显示器",
            physical_monitors.len()
        )));
    };

    action(physical_monitor.hPhysicalMonitor)
}

#[cfg(windows)]
struct PhysicalMonitors {
    monitors: Vec<windows_sys::Win32::Devices::Display::PHYSICAL_MONITOR>,
}

#[cfg(windows)]
impl PhysicalMonitors {
    fn from_hmonitor(
        monitor: windows_sys::Win32::Graphics::Gdi::HMONITOR,
    ) -> Result<Self, MonitorPowerError> {
        use windows_sys::Win32::Devices::Display::{
            GetNumberOfPhysicalMonitorsFromHMONITOR, GetPhysicalMonitorsFromHMONITOR,
            PHYSICAL_MONITOR,
        };

        let mut count = 0;
        let count_result = unsafe { GetNumberOfPhysicalMonitorsFromHMONITOR(monitor, &mut count) };
        if count_result == 0 {
            return Err(last_os_error("获取宿主端物理显示器数量失败"));
        }
        if count == 0 {
            return Err(make_monitor_power_error("没有找到可用的宿主端物理显示器"));
        }

        let mut monitors = vec![PHYSICAL_MONITOR::default(); count as usize];
        let result =
            unsafe { GetPhysicalMonitorsFromHMONITOR(monitor, count, monitors.as_mut_ptr()) };
        if result == 0 {
            return Err(last_os_error("获取宿主端物理显示器句柄失败"));
        }

        Ok(Self { monitors })
    }

    fn as_slice(&self) -> &[windows_sys::Win32::Devices::Display::PHYSICAL_MONITOR] {
        &self.monitors
    }

    fn len(&self) -> usize {
        self.monitors.len()
    }
}

#[cfg(windows)]
impl Drop for PhysicalMonitors {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Devices::Display::DestroyPhysicalMonitors(
                self.monitors.len() as u32,
                self.monitors.as_ptr(),
            );
        }
    }
}

#[cfg(windows)]
fn set_monitor_brightness_to_minimum(
    monitor: windows_sys::Win32::Foundation::HANDLE,
) -> Result<(), MonitorPowerError> {
    let mut minimum = 0;
    let mut _current = 0;
    let mut _maximum = 0;
    let read_result = unsafe {
        windows_sys::Win32::Devices::Display::GetMonitorBrightness(
            monitor,
            &mut minimum,
            &mut _current,
            &mut _maximum,
        )
    };
    if read_result == 0 {
        return Err(last_os_error("读取显示器亮度范围失败"));
    }

    let write_result =
        unsafe { windows_sys::Win32::Devices::Display::SetMonitorBrightness(monitor, minimum) };
    if write_result == 0 {
        return Err(last_os_error("通过 DDC/CI 调整显示器亮度失败"));
    }

    Ok(())
}

#[cfg(windows)]
fn last_os_error(action: &'static str) -> MonitorPowerError {
    make_monitor_power_error(format!("{action}: {}", std::io::Error::last_os_error()))
}

#[cfg(not(windows))]
fn platform_ddc_ci_dim() -> Result<(), MonitorPowerError> {
    unsupported_monitor_power_platform()
}

#[cfg(not(windows))]
fn unsupported_monitor_power_platform() -> Result<(), MonitorPowerError> {
    Err(make_monitor_power_error(
        "当前平台不支持宿主端显示器控制：仅 Windows 支持显示器电源控制",
    ))
}

fn make_monitor_power_error(message: impl Into<String>) -> MonitorPowerError {
    MonitorPowerError::new(message)
}

#[cfg(test)]
mod tests {
    #[test]
    fn unsupported_real_power_off_policies_are_rejected_before_platform_calls() {
        assert_eq!(
            super::apply_after_launch(
                wincast_protocol::config::MonitorPowerAfterLaunch::WindowsPowerMessage
            )
            .expect_err("Windows power message should not be allowed")
            .to_string(),
            "真正关闭显示器会破坏 DXGI Desktop Duplication 画面捕获；请使用 disabled 或 ddc_ci_dim"
        );
        assert_eq!(
            super::apply_after_launch(
                wincast_protocol::config::MonitorPowerAfterLaunch::DdcCiPowerOff
            )
            .expect_err("DDC/CI power off should not be allowed")
            .to_string(),
            "真正关闭显示器会破坏 DXGI Desktop Duplication 画面捕获；请使用 disabled 或 ddc_ci_dim"
        );
    }
}
