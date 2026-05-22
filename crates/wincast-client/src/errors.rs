use wincast_protocol::message::ErrorCode;

pub(crate) fn format_host_error(code: ErrorCode, message: String) -> String {
    let prefix = match code {
        ErrorCode::Busy => "宿主端忙碌",
        ErrorCode::InvalidConfig => "宿主端配置无效",
        ErrorCode::NoUserLoggedIn => "宿主端未登录 Windows 用户",
        ErrorCode::SessionLocked => "宿主端 Windows 会话已锁屏",
        ErrorCode::AgentUnavailable => "宿主端 Agent 不可用或不在线",
        ErrorCode::ProgramLaunchFailed => "宿主端程序启动失败",
        ErrorCode::ProgramExited => "宿主端程序已退出",
        ErrorCode::CaptureFailed => "宿主端画面捕获失败",
        ErrorCode::UnsupportedVersion => "协议版本不匹配",
        ErrorCode::EncodingFailed => "宿主端视频编码失败",
        ErrorCode::TransportFailed => "宿主端传输链路失败",
    };
    prepend_context_once(prefix, message)
}

pub(crate) fn prepend_context_once(prefix: &str, message: String) -> String {
    if message == prefix || message.starts_with(&format!("{prefix}: ")) {
        return message;
    }
    if message.starts_with(&format!("{prefix}：")) {
        return message;
    }
    format!("{prefix}: {message}")
}
