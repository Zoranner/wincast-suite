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

#[derive(Debug, Default)]
pub(crate) struct PendingServiceManager;

pub(crate) trait ServiceManager {
    fn install(&mut self) -> Result<String, String>;
    fn uninstall(&mut self) -> Result<String, String>;
    fn start(&mut self) -> Result<String, String>;
    fn stop(&mut self) -> Result<String, String>;
    fn status(&mut self) -> Result<ServiceStatus, String>;
}

impl PendingServiceManager {
    fn pending_operation_error(operation: &str) -> String {
        format!(
            "Windows Service {operation}尚未实现，未执行真实系统服务操作；当前仍需使用前台 run 模式。"
        )
    }
}

impl ServiceManager for PendingServiceManager {
    fn install(&mut self) -> Result<String, String> {
        Err(Self::pending_operation_error("安装"))
    }

    fn uninstall(&mut self) -> Result<String, String> {
        Err(Self::pending_operation_error("卸载"))
    }

    fn start(&mut self) -> Result<String, String> {
        Err(Self::pending_operation_error("启动"))
    }

    fn stop(&mut self) -> Result<String, String> {
        Err(Self::pending_operation_error("停止"))
    }

    fn status(&mut self) -> Result<ServiceStatus, String> {
        Ok(ServiceStatus::PendingImplementation)
    }
}
