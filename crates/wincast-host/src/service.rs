#[cfg(test)]
#[path = "service_ipc.rs"]
mod service_ipc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ServiceStatus {
    PendingImplementation,
}

impl ServiceStatus {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::PendingImplementation => {
                "Windows Service 状态：未实现真实系统服务管理，未安装；当前仍需使用前台 run 模式。"
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ServiceError {
    PendingImplementation(&'static str),
    #[cfg(not(windows))]
    UnsupportedPlatform(&'static str),
    SystemError(String),
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PendingImplementation(operation) => write!(
                formatter,
                "Windows Service {operation}尚未实现，未执行真实系统服务操作；当前仍需使用前台 run 模式。"
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
    fn operation_error(operation: &'static str) -> ServiceError {
        ServiceError::PendingImplementation(operation)
    }

    fn operation_result(operation: &'static str) -> Result<String, String> {
        Err(Self::operation_error(operation).to_string())
    }
}

#[cfg(windows)]
impl ServiceManager for WindowsServiceManager {
    fn install(&mut self) -> Result<String, String> {
        Self::operation_result("安装")
    }

    fn uninstall(&mut self) -> Result<String, String> {
        Self::operation_result("卸载")
    }

    fn start(&mut self) -> Result<String, String> {
        Self::operation_result("启动")
    }

    fn stop(&mut self) -> Result<String, String> {
        Self::operation_result("停止")
    }

    fn status(&mut self) -> Result<ServiceStatus, String> {
        Ok(ServiceStatus::PendingImplementation)
    }
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
            "Windows Service 安装尚未实现，未执行真实系统服务操作；当前仍需使用前台 run 模式。"
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
        assert!(message.contains("未实现"));
        assert!(message.contains("未安装"));
        assert!(message.contains("当前仍需使用前台 run 模式"));
        assert!(!message.contains("安装成功"));
        assert!(!message.contains("已启动"));
    }

    #[test]
    fn service_error_reports_system_error_in_chinese() {
        let error = ServiceError::from(std::io::Error::other("SCM 打开失败"));

        assert_eq!(error, ServiceError::SystemError("SCM 打开失败".to_owned()));
        assert_eq!(error.to_string(), "Windows Service 系统错误：SCM 打开失败");
    }

    #[cfg(windows)]
    #[test]
    fn windows_service_manager_keeps_real_scm_operations_pending() {
        let mut manager = WindowsServiceManager;

        assert_eq!(
            manager.install().expect_err("install should stay pending"),
            ServiceError::PendingImplementation("安装").to_string()
        );

        let status = manager
            .status()
            .expect("pending Windows status should be available");
        assert_eq!(status, ServiceStatus::PendingImplementation);
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
