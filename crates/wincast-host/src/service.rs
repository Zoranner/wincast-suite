use std::path::PathBuf;

#[cfg(windows)]
use std::{
    ffi::{OsStr, OsString},
    os::windows::ffi::OsStrExt,
    ptr::{null, null_mut},
    sync::{Mutex, mpsc},
};

#[cfg(windows)]
use windows_sys::{
    Win32::{
        Foundation::{ERROR_CALL_NOT_IMPLEMENTED, NO_ERROR},
        System::Services::{
            CloseServiceHandle, ControlService, CreateServiceW, DeleteService, OpenSCManagerW,
            OpenServiceW, QueryServiceStatusEx, RegisterServiceCtrlHandlerExW, SC_HANDLE,
            SC_MANAGER_CONNECT, SC_MANAGER_CREATE_SERVICE, SC_STATUS_PROCESS_INFO,
            SERVICE_ACCEPT_STOP, SERVICE_CONTINUE_PENDING, SERVICE_CONTROL_INTERROGATE,
            SERVICE_CONTROL_STOP, SERVICE_DEMAND_START, SERVICE_ERROR_NORMAL,
            SERVICE_PAUSE_PENDING, SERVICE_PAUSED, SERVICE_QUERY_STATUS, SERVICE_RUNNING,
            SERVICE_START, SERVICE_START_PENDING, SERVICE_STATUS, SERVICE_STATUS_PROCESS,
            SERVICE_STOP, SERVICE_STOP_PENDING, SERVICE_STOPPED, SERVICE_TABLE_ENTRYW,
            SERVICE_WIN32_OWN_PROCESS, SetServiceStatus, StartServiceCtrlDispatcherW,
            StartServiceW,
        },
    },
    core::PWSTR,
};

const SERVICE_NAME: &str = "wincast-host";
const SERVICE_DISPLAY_NAME: &str = "WinCast Host Service";
const SERVICE_RUN_ARGUMENTS: [&str; 2] = ["service", "run"];
#[cfg(windows)]
const SERVICE_DELETE_ACCESS: u32 = 0x0001_0000;

#[cfg(any(not(windows), test))]
fn stable_service_boundary() -> &'static str {
    "当前稳定版仅支持前台 run 模式，Service 管理未启用"
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ServiceStatus {
    #[cfg(any(not(windows), test))]
    PendingImplementation,
    Installed(ServiceRuntimeStatus),
}

impl ServiceStatus {
    pub(crate) fn message(self) -> String {
        match self {
            #[cfg(any(not(windows), test))]
            Self::PendingImplementation => format!(
                "Windows Service 状态：{}；未安装，未执行真实系统服务状态查询。",
                stable_service_boundary()
            ),
            Self::Installed(status) => status.message(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ServiceRuntimeStatus {
    pub(crate) state: ServiceRunState,
    pub(crate) accepts_stop: bool,
    pub(crate) process_id: Option<u32>,
}

impl ServiceRuntimeStatus {
    fn message(self) -> String {
        let stop_capability = if self.accepts_stop {
            "可响应 stop"
        } else {
            "不可响应 stop"
        };
        let process = self
            .process_id
            .map(|process_id| format!("PID {process_id}"))
            .unwrap_or_else(|| "PID 未知".to_owned());

        format!(
            "Windows Service 状态：已安装，当前状态 {}，{}，{}。",
            self.state.label(),
            stop_capability,
            process
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ServiceRunState {
    Stopped,
    StartPending,
    StopPending,
    Running,
    ContinuePending,
    PausePending,
    Paused,
    Unknown(u32),
}

impl ServiceRunState {
    fn label(self) -> String {
        match self {
            Self::Stopped => "stopped".to_owned(),
            Self::StartPending => "start_pending".to_owned(),
            Self::StopPending => "stop_pending".to_owned(),
            Self::Running => "running".to_owned(),
            Self::ContinuePending => "continue_pending".to_owned(),
            Self::PausePending => "pause_pending".to_owned(),
            Self::Paused => "paused".to_owned(),
            Self::Unknown(value) => format!("unknown({value})"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ServiceError {
    #[cfg(any(not(windows), test))]
    PendingImplementation(&'static str),
    #[cfg(not(windows))]
    UnsupportedPlatform(&'static str),
    SystemError(String),
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            #[cfg(any(not(windows), test))]
            Self::PendingImplementation(operation) => write!(
                formatter,
                "Windows Service {operation}不可用：{}；未执行真实系统服务操作。",
                stable_service_boundary()
            ),
            #[cfg(not(windows))]
            Self::UnsupportedPlatform(operation) => write!(
                formatter,
                "Windows Service {operation}仅支持 Windows 平台，当前平台未执行真实系统服务操作。"
            ),
            Self::SystemError(message) => write!(formatter, "Windows Service 系统错误：{message}"),
        }
    }
}

impl std::error::Error for ServiceError {}

impl From<std::io::Error> for ServiceError {
    fn from(error: std::io::Error) -> Self {
        Self::SystemError(error.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServiceInstallPlan {
    pub(crate) name: &'static str,
    pub(crate) display_name: &'static str,
    pub(crate) executable_path: PathBuf,
    pub(crate) launch_arguments: Vec<&'static str>,
}

impl ServiceInstallPlan {
    pub(crate) fn for_executable(executable_path: PathBuf) -> Self {
        Self {
            name: SERVICE_NAME,
            display_name: SERVICE_DISPLAY_NAME,
            executable_path,
            launch_arguments: SERVICE_RUN_ARGUMENTS.to_vec(),
        }
    }

    #[cfg(windows)]
    fn for_current_exe() -> Result<Self, ServiceError> {
        Ok(Self::for_executable(std::env::current_exe()?))
    }

    #[cfg(windows)]
    fn binary_path_name(&self) -> OsString {
        let mut command = OsString::from("\"");
        command.push(&self.executable_path);
        command.push("\"");
        for argument in &self.launch_arguments {
            command.push(" ");
            command.push(argument);
        }
        command
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ServiceControlEvent {
    Stop,
    Interrogate,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ServiceControlOutcome {
    Stop,
    Continue,
    NotImplemented,
}

pub(crate) fn handle_service_control(event: ServiceControlEvent) -> ServiceControlOutcome {
    match event {
        ServiceControlEvent::Stop => ServiceControlOutcome::Stop,
        ServiceControlEvent::Interrogate => ServiceControlOutcome::Continue,
        ServiceControlEvent::Unsupported => ServiceControlOutcome::NotImplemented,
    }
}

#[cfg(any(not(windows), test))]
#[derive(Debug, Default)]
pub(crate) struct PendingServiceManager;

#[cfg(windows)]
#[derive(Debug, Default)]
pub(crate) struct WindowsServiceManager;

#[cfg(windows)]
pub(crate) type DefaultServiceManager = WindowsServiceManager;

#[cfg(not(windows))]
pub(crate) type DefaultServiceManager = PendingServiceManager;

pub(crate) trait ServiceManager {
    fn install(&mut self) -> Result<String, String>;
    fn uninstall(&mut self) -> Result<String, String>;
    fn start(&mut self) -> Result<String, String>;
    fn stop(&mut self) -> Result<String, String>;
    fn status(&mut self) -> Result<ServiceStatus, String>;
}

#[cfg(any(not(windows), test))]
impl PendingServiceManager {
    fn pending_operation_error(operation: &'static str) -> ServiceError {
        ServiceError::PendingImplementation(operation)
    }

    fn pending_operation_result(operation: &'static str) -> Result<String, String> {
        Err(Self::pending_operation_error(operation).to_string())
    }
}

#[cfg(any(not(windows), test))]
impl ServiceManager for PendingServiceManager {
    fn install(&mut self) -> Result<String, String> {
        Self::pending_operation_result("安装")
    }

    fn uninstall(&mut self) -> Result<String, String> {
        Self::pending_operation_result("卸载")
    }

    fn start(&mut self) -> Result<String, String> {
        Self::pending_operation_result("启动")
    }

    fn stop(&mut self) -> Result<String, String> {
        Self::pending_operation_result("停止")
    }

    fn status(&mut self) -> Result<ServiceStatus, String> {
        Ok(ServiceStatus::PendingImplementation)
    }
}

#[cfg(windows)]
impl WindowsServiceManager {
    fn open_scm(request_access: u32) -> Result<ServiceHandle, ServiceError> {
        let handle = unsafe { OpenSCManagerW(null(), null(), request_access) };
        ServiceHandle::from_raw(handle, "打开 SCM")
    }

    fn open_service(request_access: u32) -> Result<ServiceHandle, ServiceError> {
        let manager = Self::open_scm(SC_MANAGER_CONNECT)?;
        let service_name = wide_null(SERVICE_NAME);
        let handle = unsafe { OpenServiceW(manager.raw(), service_name.as_ptr(), request_access) };
        ServiceHandle::from_raw(handle, "打开 Service")
    }
}

#[cfg(windows)]
impl ServiceManager for WindowsServiceManager {
    fn install(&mut self) -> Result<String, String> {
        let plan = ServiceInstallPlan::for_current_exe().map_err(|error| error.to_string())?;
        let manager =
            Self::open_scm(SC_MANAGER_CREATE_SERVICE).map_err(|error| error.to_string())?;
        let service_name = wide_null(plan.name);
        let display_name = wide_null(plan.display_name);
        let binary_path = wide_null(plan.binary_path_name());

        let service = unsafe {
            CreateServiceW(
                manager.raw(),
                service_name.as_ptr(),
                display_name.as_ptr(),
                SERVICE_QUERY_STATUS,
                SERVICE_WIN32_OWN_PROCESS,
                SERVICE_DEMAND_START,
                SERVICE_ERROR_NORMAL,
                binary_path.as_ptr(),
                null(),
                null_mut(),
                null(),
                null(),
                null(),
            )
        };
        let service =
            ServiceHandle::from_raw(service, "创建 Service").map_err(|error| error.to_string())?;
        drop(service);

        Ok(format!("Windows Service 已安装：{}。", plan.name))
    }

    fn uninstall(&mut self) -> Result<String, String> {
        let service =
            Self::open_service(SERVICE_DELETE_ACCESS).map_err(|error| error.to_string())?;

        if unsafe { DeleteService(service.raw()) } == 0 {
            return Err(ServiceError::last_os_error("删除 Service").to_string());
        }

        Ok(format!("Windows Service 已标记卸载：{SERVICE_NAME}。"))
    }

    fn start(&mut self) -> Result<String, String> {
        let service = Self::open_service(SERVICE_START).map_err(|error| error.to_string())?;

        if unsafe { StartServiceW(service.raw(), 0, null()) } == 0 {
            return Err(ServiceError::last_os_error("启动 Service").to_string());
        }

        Ok(format!("Windows Service 启动请求已发送：{SERVICE_NAME}。"))
    }

    fn stop(&mut self) -> Result<String, String> {
        let service = Self::open_service(SERVICE_STOP | SERVICE_QUERY_STATUS)
            .map_err(|error| error.to_string())?;
        let mut raw_status = SERVICE_STATUS::default();

        if unsafe { ControlService(service.raw(), SERVICE_CONTROL_STOP, &mut raw_status) } == 0 {
            return Err(ServiceError::last_os_error("停止 Service").to_string());
        }

        Ok(format!(
            "Windows Service 停止请求已发送。{}",
            ServiceStatus::from(raw_status).message()
        ))
    }

    fn status(&mut self) -> Result<ServiceStatus, String> {
        let service =
            Self::open_service(SERVICE_QUERY_STATUS).map_err(|error| error.to_string())?;
        let mut raw_status = SERVICE_STATUS_PROCESS::default();
        let mut bytes_needed = 0;

        if unsafe {
            QueryServiceStatusEx(
                service.raw(),
                SC_STATUS_PROCESS_INFO,
                &mut raw_status as *mut SERVICE_STATUS_PROCESS as *mut u8,
                std::mem::size_of::<SERVICE_STATUS_PROCESS>() as u32,
                &mut bytes_needed,
            )
        } == 0
        {
            return Err(ServiceError::last_os_error("查询 Service 状态").to_string());
        }

        Ok(ServiceStatus::from(raw_status))
    }
}

#[cfg(windows)]
impl From<SERVICE_STATUS_PROCESS> for ServiceStatus {
    fn from(status: SERVICE_STATUS_PROCESS) -> Self {
        Self::Installed(ServiceRuntimeStatus {
            state: ServiceRunState::from(status.dwCurrentState),
            accepts_stop: status.dwControlsAccepted & SERVICE_ACCEPT_STOP != 0,
            process_id: (status.dwProcessId != 0).then_some(status.dwProcessId),
        })
    }
}

#[cfg(windows)]
impl From<SERVICE_STATUS> for ServiceStatus {
    fn from(status: SERVICE_STATUS) -> Self {
        Self::Installed(ServiceRuntimeStatus {
            state: ServiceRunState::from(status.dwCurrentState),
            accepts_stop: status.dwControlsAccepted & SERVICE_ACCEPT_STOP != 0,
            process_id: None,
        })
    }
}

#[cfg(windows)]
impl From<u32> for ServiceRunState {
    fn from(state: u32) -> Self {
        match state {
            SERVICE_STOPPED => Self::Stopped,
            SERVICE_START_PENDING => Self::StartPending,
            SERVICE_STOP_PENDING => Self::StopPending,
            SERVICE_RUNNING => Self::Running,
            SERVICE_CONTINUE_PENDING => Self::ContinuePending,
            SERVICE_PAUSE_PENDING => Self::PausePending,
            SERVICE_PAUSED => Self::Paused,
            value => Self::Unknown(value),
        }
    }
}

#[cfg(windows)]
impl From<u32> for ServiceControlEvent {
    fn from(control: u32) -> Self {
        match control {
            SERVICE_CONTROL_STOP => Self::Stop,
            SERVICE_CONTROL_INTERROGATE => Self::Interrogate,
            _ => Self::Unsupported,
        }
    }
}

#[cfg(windows)]
pub(crate) fn run_service_dispatcher() -> Result<String, String> {
    let mut service_name = wide_null(SERVICE_NAME);
    let mut service_table = [
        SERVICE_TABLE_ENTRYW {
            lpServiceName: service_name.as_mut_ptr(),
            lpServiceProc: Some(service_main),
        },
        SERVICE_TABLE_ENTRYW {
            lpServiceName: null_mut(),
            lpServiceProc: None,
        },
    ];

    if unsafe { StartServiceCtrlDispatcherW(service_table.as_mut_ptr()) } == 0 {
        return Err(ServiceError::last_os_error("启动 Service Dispatcher").to_string());
    }

    Ok("Windows Service 运行入口已退出。".to_owned())
}

#[cfg(not(windows))]
pub(crate) fn run_service_dispatcher() -> Result<String, String> {
    Err(ServiceError::UnsupportedPlatform("运行").to_string())
}

#[cfg(windows)]
unsafe extern "system" fn service_main(_argc: u32, _argv: *mut PWSTR) {
    if let Err(error) = run_service_main() {
        eprintln!("Windows Service 运行入口失败：{error}");
    }
}

#[cfg(windows)]
fn run_service_main() -> Result<(), ServiceError> {
    let (stop_sender, stop_receiver) = mpsc::channel();
    let service_context = Box::new(ServiceControlContext::new(stop_sender));
    let service_name = wide_null(SERVICE_NAME);
    let status_handle = unsafe {
        // SAFETY: `service_context` owns the handler context for this service main invocation.
        // The boxed allocation gives the SCM callback a stable address, and it is kept alive
        // until after the service reports `SERVICE_STOPPED`. The handler only reads this pointer
        // while the service control dispatcher is running; stop signaling is synchronized by the
        // `Mutex` inside `ServiceControlContext`.
        RegisterServiceCtrlHandlerExW(
            service_name.as_ptr(),
            Some(service_control_handler),
            service_context.as_handler_context(),
        )
    };
    if status_handle.is_null() {
        return Err(ServiceError::last_os_error("注册 Service 控制处理器"));
    }

    set_service_status(status_handle, SERVICE_RUNNING, SERVICE_ACCEPT_STOP)?;

    let _ = stop_receiver.recv();

    set_service_status(status_handle, SERVICE_STOP_PENDING, 0)?;
    set_service_status(status_handle, SERVICE_STOPPED, 0)?;

    Ok(())
}

#[cfg(windows)]
unsafe extern "system" fn service_control_handler(
    control: u32,
    _event_type: u32,
    _event_data: *mut core::ffi::c_void,
    context: *mut core::ffi::c_void,
) -> u32 {
    match handle_service_control(ServiceControlEvent::from(control)) {
        ServiceControlOutcome::Stop => {
            ServiceControlContext::notify_stop_from_handler_context(context);
            NO_ERROR
        }
        ServiceControlOutcome::Continue => NO_ERROR,
        ServiceControlOutcome::NotImplemented => ERROR_CALL_NOT_IMPLEMENTED,
    }
}

#[cfg(windows)]
#[derive(Debug)]
struct ServiceControlContext {
    stop_sender: Mutex<mpsc::Sender<()>>,
}

#[cfg(windows)]
impl ServiceControlContext {
    fn new(stop_sender: mpsc::Sender<()>) -> Self {
        Self {
            stop_sender: Mutex::new(stop_sender),
        }
    }

    fn as_handler_context(&self) -> *mut core::ffi::c_void {
        self as *const Self as *mut core::ffi::c_void
    }

    fn notify_stop_from_handler_context(context: *mut core::ffi::c_void) {
        if context.is_null() {
            return;
        }

        // SAFETY: `context` must be the pointer returned by
        // `ServiceControlContext::as_handler_context` for the currently running service main.
        // `run_service_main` stores that context in a `Box`, so the pointee has a stable address,
        // and drops it only after the stop notification has been received and the service has
        // reported `SERVICE_STOPPED`. The SCM may call the handler on a dispatcher thread, so the
        // sender is accessed only through the context-owned `Mutex`.
        let service_context = unsafe { &*(context as *const Self) };
        if let Ok(sender) = service_context.stop_sender.lock() {
            let _ = sender.send(());
        }
    }
}

#[cfg(windows)]
fn set_service_status(
    status_handle: windows_sys::Win32::System::Services::SERVICE_STATUS_HANDLE,
    current_state: u32,
    controls_accepted: u32,
) -> Result<(), ServiceError> {
    let status = SERVICE_STATUS {
        dwServiceType: SERVICE_WIN32_OWN_PROCESS,
        dwCurrentState: current_state,
        dwControlsAccepted: controls_accepted,
        dwWin32ExitCode: NO_ERROR,
        dwServiceSpecificExitCode: 0,
        dwCheckPoint: 0,
        dwWaitHint: 0,
    };

    if unsafe { SetServiceStatus(status_handle, &status) } == 0 {
        return Err(ServiceError::last_os_error("设置 Service 状态"));
    }

    Ok(())
}

#[cfg(windows)]
#[derive(Debug)]
struct ServiceHandle(SC_HANDLE);

#[cfg(windows)]
impl ServiceHandle {
    fn from_raw(handle: SC_HANDLE, operation: &'static str) -> Result<Self, ServiceError> {
        if handle.is_null() {
            Err(ServiceError::last_os_error(operation))
        } else {
            Ok(Self(handle))
        }
    }

    fn raw(&self) -> SC_HANDLE {
        self.0
    }
}

#[cfg(windows)]
impl Drop for ServiceHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CloseServiceHandle(self.0);
            }
        }
    }
}

#[cfg(windows)]
impl ServiceError {
    fn last_os_error(operation: &'static str) -> Self {
        Self::SystemError(format!(
            "{operation}失败：{}",
            std::io::Error::last_os_error()
        ))
    }
}

#[cfg(windows)]
fn wide_null(value: impl AsRef<OsStr>) -> Vec<u16> {
    value
        .as_ref()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_service_manager_uses_structured_pending_error() {
        let error = PendingServiceManager::pending_operation_error("安装");

        assert_eq!(error, ServiceError::PendingImplementation("安装"));
        assert_eq!(
            error.to_string(),
            "Windows Service 安装不可用：当前稳定版仅支持前台 run 模式，Service 管理未启用；未执行真实系统服务操作。"
        );
    }

    #[test]
    fn pending_service_status_reports_pending_without_success_claims() {
        let mut manager = PendingServiceManager;

        let status = manager
            .status()
            .expect("pending status should be available");
        let message = status.message();

        assert_eq!(status, ServiceStatus::PendingImplementation);
        assert_eq!(
            message,
            "Windows Service 状态：当前稳定版仅支持前台 run 模式，Service 管理未启用；未安装，未执行真实系统服务状态查询。"
        );
        assert!(message.contains("未安装"));
        assert!(message.contains("当前稳定版仅支持前台 run 模式，Service 管理未启用"));
        assert!(!message.contains("安装成功"));
        assert!(!message.contains("已启动"));
    }

    #[test]
    fn service_error_reports_system_error_in_chinese() {
        let error = ServiceError::from(std::io::Error::other("SCM 打开失败"));

        assert_eq!(error, ServiceError::SystemError("SCM 打开失败".to_owned()));
        assert_eq!(error.to_string(), "Windows Service 系统错误：SCM 打开失败");
    }

    #[test]
    fn service_install_plan_uses_service_run_entrypoint() {
        let executable_path =
            std::path::PathBuf::from(r"C:\Program Files\WinCast\wincast-host.exe");

        let plan = ServiceInstallPlan::for_executable(executable_path.clone());

        assert_eq!(plan.name, "wincast-host");
        assert_eq!(plan.display_name, "WinCast Host Service");
        assert_eq!(plan.executable_path, executable_path);
        assert_eq!(plan.launch_arguments, ["service", "run"]);
    }

    #[test]
    fn installed_service_status_reports_runtime_state_without_pending_claims() {
        let status = ServiceStatus::Installed(ServiceRuntimeStatus {
            state: ServiceRunState::Running,
            accepts_stop: true,
            process_id: Some(4242),
        });
        let message = status.message();

        assert_eq!(
            message,
            "Windows Service 状态：已安装，当前状态 running，可响应 stop，PID 4242。"
        );
        assert!(!message.contains("未启用"));
        assert!(!message.contains("未执行真实系统服务状态查询"));
    }

    #[test]
    fn minimal_service_control_policy_accepts_stop_and_interrogate() {
        assert_eq!(
            handle_service_control(ServiceControlEvent::Interrogate),
            ServiceControlOutcome::Continue
        );
        assert_eq!(
            handle_service_control(ServiceControlEvent::Stop),
            ServiceControlOutcome::Stop
        );
        assert_eq!(
            handle_service_control(ServiceControlEvent::Unsupported),
            ServiceControlOutcome::NotImplemented
        );
    }

    #[cfg(windows)]
    #[test]
    fn service_control_context_exposes_stable_handler_pointer_and_notifies_stop() {
        let (stop_sender, stop_receiver) = mpsc::channel();
        let context = ServiceControlContext::new(stop_sender);
        let context_ptr = context.as_handler_context();

        assert!(!context_ptr.is_null());
        assert_eq!(context_ptr, context.as_handler_context());

        let result =
            unsafe { service_control_handler(SERVICE_CONTROL_STOP, 0, null_mut(), context_ptr) };

        assert_eq!(result, NO_ERROR);
        stop_receiver
            .try_recv()
            .expect("stop control should notify service main");
    }

    #[cfg(not(windows))]
    #[test]
    fn unsupported_platform_error_does_not_claim_real_service_support() {
        let error = ServiceError::UnsupportedPlatform("安装").to_string();

        assert!(error.contains("仅支持 Windows 平台"));
        assert!(error.contains("未执行真实系统服务操作"));
        assert!(!error.contains("安装成功"));
        assert!(!error.contains("已启动"));
    }
}
