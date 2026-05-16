use std::{fs, net::TcpStream, path::PathBuf, time::Duration};

use wincast_media::{VideoDecoder, test_support::FakeH264Decoder};
use wincast_protocol::{
    config::{ClientConfig, VideoCodec},
    frame::read_message,
    handshake::{HandshakeError, read_host_hello, send_client_hello, send_start_session},
    message::{ControlMessage, EncodedVideoFrame, ErrorCode},
};

use crate::{
    errors::format_host_error,
    render_loop::ClientRenderMode,
    stream::{read_first_raw_binary_frame, validate_readback_frame},
};

pub(crate) fn load_config(path: &PathBuf) -> Result<ClientConfig, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("读取客户端配置失败 {}: {error}", path.display()))?;
    ClientConfig::from_toml_str(&source).map_err(|error| error.to_string())
}

pub(crate) fn run_client(path: &PathBuf, retry_options: RetryOptions) -> Result<String, String> {
    let config = load_config(path)?;
    run_with_retry_and_reporter(
        &retry_options,
        || run_client_attempt(&config),
        std::thread::sleep,
        |report| eprintln!("{}", format_retry_report(report)),
    )
}

fn run_client_attempt(config: &ClientConfig) -> Result<String, ClientRunError> {
    let endpoint = config.endpoint();
    let mut stream = TcpStream::connect(&endpoint).map_err(|error| {
        ClientRunError::Connection(format!("无法连接宿主端 {endpoint}: {error}"))
    })?;

    send_client_hello(&mut stream).map_err(ClientRunError::Handshake)?;
    read_host_hello(&mut stream).map_err(ClientRunError::Handshake)?;
    send_start_session(&mut stream).map_err(ClientRunError::Handshake)?;
    let render_mode = ClientRenderMode::for_current_platform();
    read_session_start_response(&mut stream, render_mode)?;

    Ok(control_channel_ready_message(config))
}

#[cfg(test)]
pub(crate) fn run_client_with_config(config: &ClientConfig) -> Result<String, String> {
    run_client_attempt(config).map_err(ClientRunError::into_message)
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RetryOptions {
    pub(crate) retries: u32,
    pub(crate) retry_delay: Duration,
}

#[derive(Debug)]
pub(crate) struct RetryReport {
    pub(crate) attempt: u32,
    pub(crate) max_attempts: u32,
    pub(crate) retry_delay: Duration,
    pub(crate) reason: String,
}

#[derive(Debug)]
pub(crate) enum ClientRunError {
    Connection(String),
    Handshake(HandshakeError),
    HostStatus { code: ErrorCode, message: String },
    Fatal(String),
}

impl ClientRunError {
    #[cfg(test)]
    pub(crate) fn host_status(code: ErrorCode, message: impl Into<String>) -> Self {
        Self::HostStatus {
            code,
            message: message.into(),
        }
    }

    fn is_retriable(&self) -> bool {
        match self {
            Self::Connection(_) => true,
            Self::Handshake(HandshakeError::HostRejected { code, .. })
            | Self::HostStatus { code, .. } => is_retriable_host_status(*code),
            Self::Handshake(_) | Self::Fatal(_) => false,
        }
    }

    fn into_message(self) -> String {
        match self {
            Self::Connection(message) | Self::Fatal(message) => message,
            Self::Handshake(error) => format_handshake_error(error),
            Self::HostStatus { code, message } => format_host_error(code, message),
        }
    }
}

#[cfg(test)]
pub(crate) fn run_with_retry(
    options: &RetryOptions,
    attempt: impl FnMut() -> Result<String, ClientRunError>,
    sleep: impl FnMut(Duration),
) -> Result<String, String> {
    run_with_retry_and_reporter(options, attempt, sleep, |_| {})
}

pub(crate) fn run_with_retry_and_reporter(
    options: &RetryOptions,
    mut attempt: impl FnMut() -> Result<String, ClientRunError>,
    mut sleep: impl FnMut(Duration),
    mut reporter: impl FnMut(&RetryReport),
) -> Result<String, String> {
    let max_attempts = options.retries.saturating_add(1);
    let mut attempts = 0;
    loop {
        attempts += 1;
        match attempt() {
            Ok(message) => return Ok(message),
            Err(error) => {
                let is_retriable = error.is_retriable();
                let should_retry = is_retriable && attempts < max_attempts;
                if !should_retry {
                    if max_attempts == 1 || !is_retriable {
                        return Err(error.into_message());
                    }
                    return Err(format!(
                        "客户端运行尝试 {attempts} 次后失败，最后原因: {}",
                        error.into_message()
                    ));
                }
                reporter(&RetryReport {
                    attempt: attempts,
                    max_attempts,
                    retry_delay: options.retry_delay,
                    reason: error.into_message(),
                });
                sleep(options.retry_delay);
            }
        }
    }
}

pub(crate) fn format_retry_report(report: &RetryReport) -> String {
    format!(
        "客户端运行第 {}/{} 次失败：{}；{} ms 后重试。",
        report.attempt,
        report.max_attempts,
        report.reason,
        report.retry_delay.as_millis()
    )
}

fn is_retriable_host_status(code: ErrorCode) -> bool {
    matches!(
        code,
        ErrorCode::Busy
            | ErrorCode::NoUserLoggedIn
            | ErrorCode::SessionLocked
            | ErrorCode::AgentUnavailable
    )
}

fn read_session_start_response(
    stream: &mut TcpStream,
    render_mode: ClientRenderMode,
) -> Result<(), ClientRunError> {
    match read_message(stream)
        .map_err(|error| ClientRunError::Fatal(format!("读取宿主端会话响应失败: {error}")))?
    {
        ControlMessage::SessionReady { width, height } => {
            read_first_readback_frame(stream, render_mode, width, height)
        }
        ControlMessage::Error { code, message } => {
            Err(ClientRunError::HostStatus { code, message })
        }
        message => Err(ClientRunError::Fatal(format!(
            "宿主端会话响应无效: {message:?}"
        ))),
    }
}

fn read_first_readback_frame(
    stream: &mut TcpStream,
    render_mode: ClientRenderMode,
    width: u32,
    height: u32,
) -> Result<(), ClientRunError> {
    match read_message(stream)
        .map_err(|error| ClientRunError::Fatal(format!("读取宿主端首帧失败: {error}")))?
    {
        ControlMessage::RawBgraReadbackFrame(frame) => {
            validate_readback_frame(&frame).map_err(ClientRunError::Fatal)
        }
        ControlMessage::EncodedVideoFrame(frame) => validate_encoded_video_frame(&frame)
            .map(|_| ())
            .map_err(ClientRunError::Fatal),
        ControlMessage::VideoReady => {
            read_first_raw_binary_frame(stream, render_mode, width, height)
                .map_err(ClientRunError::Fatal)
        }
        ControlMessage::Error { code, message } => {
            Err(ClientRunError::HostStatus { code, message })
        }
        message => Err(ClientRunError::Fatal(format!(
            "宿主端首帧消息无效: {message:?}"
        ))),
    }
}

pub(crate) fn control_channel_ready_message(config: &ClientConfig) -> String {
    format!(
        "客户端配置有效，已建立宿主端控制通道 {}，已发送会话启动请求。客户端已完成宿主端首个视频响应的解码边界校验；宿主端已接入基础 Windows 输入注入。",
        config.endpoint()
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DecodedVideoFrameBoundary {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) row_pitch: u32,
    pub(crate) bytes_len: usize,
}

pub(crate) fn validate_encoded_video_frame(
    frame: &EncodedVideoFrame,
) -> Result<DecodedVideoFrameBoundary, String> {
    if frame.codec != VideoCodec::H264 {
        return Err(format!("宿主端编码帧 codec 无效: {:?}", frame.codec));
    }
    frame
        .validate()
        .map_err(|error| format!("宿主端 H.264 编码帧无效: {error:?}"))?;

    let mut decoder = FakeH264Decoder::new();
    let decoded = decoder
        .decode(frame)
        .map_err(|error| format!("宿主端 H.264 编码帧解码失败: {error}"))?;
    let row_pitch = decoded.row_pitch();
    let expected_len = row_pitch
        .checked_mul(decoded.height)
        .ok_or_else(|| "宿主端 H.264 编码帧解码失败: decoded frame 尺寸溢出".to_owned())?
        as usize;
    if decoded.bytes.len() != expected_len {
        return Err(format!(
            "宿主端 H.264 编码帧解码失败: decoded frame 字节数 {} 与 row_pitch * height {expected_len} 不一致",
            decoded.bytes.len()
        ));
    }

    Ok(DecodedVideoFrameBoundary {
        width: decoded.width,
        height: decoded.height,
        row_pitch,
        bytes_len: decoded.bytes.len(),
    })
}

fn format_handshake_error(error: HandshakeError) -> String {
    match error {
        HandshakeError::HostRejected { code, message } => format_host_error(code, message),
        HandshakeError::UnsupportedVersion => "协议版本不匹配".to_owned(),
        HandshakeError::InvalidMessage => "宿主端握手消息无效".to_owned(),
        HandshakeError::Frame(error) => format!("控制通道握手失败: {error}"),
    }
}
