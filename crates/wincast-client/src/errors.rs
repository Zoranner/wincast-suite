use wincast_protocol::message::ErrorCode;

pub(crate) fn format_host_error(code: ErrorCode, message: String) -> String {
    match code {
        ErrorCode::Busy => format!("宿主端忙碌: {message}"),
        ErrorCode::InvalidConfig => format!("宿主端配置无效: {message}"),
        ErrorCode::NoUserLoggedIn => format!("宿主端未登录 Windows 用户: {message}"),
        ErrorCode::SessionLocked => format!("宿主端 Windows 会话已锁屏: {message}"),
        ErrorCode::AgentUnavailable => format!("宿主端 Agent 不可用或不在线: {message}"),
        ErrorCode::ProgramLaunchFailed => format!("宿主端程序启动失败: {message}"),
        ErrorCode::ProgramExited => format!("宿主端程序已退出: {message}"),
        ErrorCode::CaptureFailed => format!("宿主端画面捕获失败: {message}"),
        ErrorCode::UnsupportedVersion => format!("协议版本不匹配: {message}"),
        ErrorCode::EncodingFailed => format!("宿主端视频编码失败: {message}"),
        ErrorCode::TransportFailed => format!("宿主端传输链路失败: {message}"),
    }
}
