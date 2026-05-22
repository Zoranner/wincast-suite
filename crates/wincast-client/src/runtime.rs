use std::{fs, net::TcpStream, path::PathBuf, time::Duration};

use wincast_protocol::{
    config::ClientConfig,
    frame::read_message,
    handshake::{HandshakeError, read_host_hello, send_client_hello, send_start_session},
    message::{ControlMessage, ErrorCode},
};

use crate::{
    errors::format_host_error,
    render_loop::ClientRenderMode,
    stream::{
        VideoStreamEnd, read_h264_encoded_frames_from_first,
        read_h264_encoded_frames_with_sdl_window_from_first,
    },
};

#[cfg(target_os = "linux")]
use crate::stream::read_h264_encoded_frames_with_renderer_from_first;

const H264_VALIDATION_FRAME_COUNT: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClientRuntimeEvent {
    Connecting,
    Handshaking,
    StartingSession,
    WaitingForSession,
    WaitingForFirstFrame,
    Streaming,
    SessionEnded,
}

pub(crate) fn load_config(path: &PathBuf) -> Result<ClientConfig, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("读取客户端配置失败 {}: {error}", path.display()))?;
    ClientConfig::from_toml_str(&source).map_err(|error| error.to_string())
}

pub(crate) fn run_client(path: &PathBuf, retry_options: RetryOptions) -> Result<String, String> {
    let config = load_config(path)?;
    run_client_with_config_and_retry(&config, retry_options)
}

pub(crate) fn run_client_with_config_and_retry(
    config: &ClientConfig,
    retry_options: RetryOptions,
) -> Result<String, String> {
    run_with_retry_and_reporter(
        &retry_options,
        || run_client_attempt(config),
        std::thread::sleep,
        |report| eprintln!("{}", format_retry_report(report)),
    )
}

#[cfg(target_os = "linux")]
pub(crate) fn run_fullscreen_client(
    path: &PathBuf,
    retry_options: RetryOptions,
) -> Result<String, String> {
    let config = load_config(path)?;
    let mut renderer = wincast_render::SdlBgraPixelRenderer::new(wincast_render::RenderConfig {
        title: "WinCast Client".to_owned(),
        width: 1280,
        height: 720,
        fullscreen: true,
    })
    .map_err(|error| format!("创建客户端 SDL2 全屏窗口失败: {error}"))?;
    let mut tick = 0;
    render_loading_status(&mut renderer, "正在启动客户端", &mut tick)?;
    run_with_retry_and_reporter(
        &retry_options,
        || run_client_attempt_with_renderer(&config, &mut renderer, &mut tick),
        std::thread::sleep,
        |report| {
            let _ = render_loading_status(
                &mut renderer,
                &format!("{} ms 后重试", report.retry_delay.as_millis()),
                &mut tick,
            );
            eprintln!("{}", format_retry_report(report));
        },
    )
}

#[cfg(target_os = "linux")]
fn render_loading_status(
    renderer: &mut impl wincast_render::BgraPixelRenderer,
    message: &str,
    tick: &mut u64,
) -> Result<(), String> {
    *tick += 17;
    renderer
        .render_loading(&wincast_render::LoadingStatus {
            message: message.to_owned(),
            tick: *tick,
        })
        .map_err(|error| format!("渲染客户端加载状态失败: {error}"))
}

#[cfg(target_os = "linux")]
fn render_runtime_loading_event(
    event: ClientRuntimeEvent,
    renderer: &mut impl wincast_render::BgraPixelRenderer,
    tick: &mut u64,
) -> Result<(), String> {
    let message = match event {
        ClientRuntimeEvent::Connecting => "正在连接宿主端",
        ClientRuntimeEvent::Handshaking => "正在握手",
        ClientRuntimeEvent::StartingSession => "正在请求启动会话",
        ClientRuntimeEvent::WaitingForSession => "正在等待宿主端启动程序",
        ClientRuntimeEvent::WaitingForFirstFrame => "正在等待首帧",
        ClientRuntimeEvent::Streaming => "正在进入远程画面",
        ClientRuntimeEvent::SessionEnded => "远程会话已结束",
    };
    render_loading_status(renderer, message, tick)
}

fn run_client_attempt(config: &ClientConfig) -> Result<String, ClientRunError> {
    run_client_attempt_with_reporter(config, |_| {})
}

pub(crate) fn run_client_attempt_with_reporter(
    config: &ClientConfig,
    mut reporter: impl FnMut(ClientRuntimeEvent),
) -> Result<String, ClientRunError> {
    let endpoint = config.endpoint();
    reporter(ClientRuntimeEvent::Connecting);
    let mut stream = TcpStream::connect(&endpoint).map_err(|error| {
        ClientRunError::Connection(format!("无法连接宿主端 {endpoint}: {error}"))
    })?;

    reporter(ClientRuntimeEvent::Handshaking);
    send_client_hello(&mut stream).map_err(ClientRunError::Handshake)?;
    read_host_hello(&mut stream).map_err(ClientRunError::Handshake)?;
    reporter(ClientRuntimeEvent::StartingSession);
    send_start_session(&mut stream).map_err(ClientRunError::Handshake)?;
    let render_mode = ClientRenderMode::for_current_platform();
    if let Err(error) = read_session_start_response(&mut stream, render_mode, &mut reporter) {
        if let ClientRunError::NormalSessionEnd(message) = error {
            reporter(ClientRuntimeEvent::SessionEnded);
            return Ok(message);
        }
        return Err(error);
    }
    reporter(ClientRuntimeEvent::SessionEnded);

    Ok(control_channel_ready_message(config))
}

#[cfg(target_os = "linux")]
fn run_client_attempt_with_renderer(
    config: &ClientConfig,
    renderer: &mut impl wincast_render::BgraPixelRenderer,
    tick: &mut u64,
) -> Result<String, ClientRunError> {
    let endpoint = config.endpoint();
    let _ = render_runtime_loading_event(ClientRuntimeEvent::Connecting, renderer, tick);
    let mut stream = TcpStream::connect(&endpoint).map_err(|error| {
        ClientRunError::Connection(format!("无法连接宿主端 {endpoint}: {error}"))
    })?;

    let _ = render_runtime_loading_event(ClientRuntimeEvent::Handshaking, renderer, tick);
    send_client_hello(&mut stream).map_err(ClientRunError::Handshake)?;
    read_host_hello(&mut stream).map_err(ClientRunError::Handshake)?;
    let _ = render_runtime_loading_event(ClientRuntimeEvent::StartingSession, renderer, tick);
    send_start_session(&mut stream).map_err(ClientRunError::Handshake)?;
    if let Err(error) = read_session_start_response_with_renderer(&mut stream, renderer, tick) {
        if let ClientRunError::NormalSessionEnd(message) = error {
            let _ = render_runtime_loading_event(ClientRuntimeEvent::SessionEnded, renderer, tick);
            return Ok(message);
        }
        return Err(error);
    }
    let _ = render_runtime_loading_event(ClientRuntimeEvent::SessionEnded, renderer, tick);

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
    VideoStreamInterrupted(String),
    NormalSessionEnd(String),
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
            Self::Connection(_) | Self::VideoStreamInterrupted(_) => true,
            Self::Handshake(HandshakeError::HostRejected { code, .. })
            | Self::HostStatus { code, .. } => is_retriable_host_status(*code),
            Self::Handshake(_) | Self::NormalSessionEnd(_) | Self::Fatal(_) => false,
        }
    }

    fn into_message(self) -> String {
        match self {
            Self::Connection(message)
            | Self::VideoStreamInterrupted(message)
            | Self::NormalSessionEnd(message)
            | Self::Fatal(message) => message,
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
                if let ClientRunError::NormalSessionEnd(message) = error {
                    return Ok(message);
                }
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
    reporter: &mut impl FnMut(ClientRuntimeEvent),
) -> Result<(), ClientRunError> {
    reporter(ClientRuntimeEvent::WaitingForSession);
    match read_message(stream)
        .map_err(|error| ClientRunError::Fatal(format!("读取宿主端会话响应失败: {error}")))?
    {
        ControlMessage::SessionReady { width, height } => {
            read_first_readback_frame(stream, render_mode, width, height, reporter)
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
    reporter: &mut impl FnMut(ClientRuntimeEvent),
) -> Result<(), ClientRunError> {
    reporter(ClientRuntimeEvent::WaitingForFirstFrame);
    match read_message(stream)
        .map_err(|error| ClientRunError::Fatal(format!("读取宿主端首帧失败: {error}")))?
    {
        ControlMessage::EncodedVideoFrame(frame) => match render_mode {
            ClientRenderMode::SdlWindow => {
                reporter(ClientRuntimeEvent::Streaming);
                handle_video_stream_end(
                    read_h264_encoded_frames_with_sdl_window_from_first(
                        stream, frame, width, height,
                    )
                    .map_err(classify_video_stream_error)?,
                )
            }
            ClientRenderMode::ProtocolOnly => {
                reporter(ClientRuntimeEvent::Streaming);
                handle_video_stream_end(
                    read_h264_encoded_frames_from_first(stream, frame, H264_VALIDATION_FRAME_COUNT)
                        .map_err(classify_video_stream_error)?,
                )
            }
        },
        ControlMessage::Error { code, message } => {
            Err(ClientRunError::HostStatus { code, message })
        }
        message => Err(ClientRunError::Fatal(format!(
            "宿主端首帧消息无效: {message:?}"
        ))),
    }
}

#[cfg(target_os = "linux")]
fn read_session_start_response_with_renderer(
    stream: &mut TcpStream,
    renderer: &mut impl wincast_render::BgraPixelRenderer,
    tick: &mut u64,
) -> Result<(), ClientRunError> {
    let _ = render_runtime_loading_event(ClientRuntimeEvent::WaitingForSession, renderer, tick);
    match read_message(stream)
        .map_err(|error| ClientRunError::Fatal(format!("读取宿主端会话响应失败: {error}")))?
    {
        ControlMessage::SessionReady { .. } => {
            read_first_readback_frame_with_renderer(stream, renderer, tick)
        }
        ControlMessage::Error { code, message } => {
            Err(ClientRunError::HostStatus { code, message })
        }
        message => Err(ClientRunError::Fatal(format!(
            "宿主端会话响应无效: {message:?}"
        ))),
    }
}

#[cfg(target_os = "linux")]
fn read_first_readback_frame_with_renderer(
    stream: &mut TcpStream,
    renderer: &mut impl wincast_render::BgraPixelRenderer,
    tick: &mut u64,
) -> Result<(), ClientRunError> {
    let _ = render_runtime_loading_event(ClientRuntimeEvent::WaitingForFirstFrame, renderer, tick);
    match read_message(stream)
        .map_err(|error| ClientRunError::Fatal(format!("读取宿主端首帧失败: {error}")))?
    {
        ControlMessage::EncodedVideoFrame(frame) => {
            let _ = render_runtime_loading_event(ClientRuntimeEvent::Streaming, renderer, tick);
            stream
                .set_read_timeout(Some(std::time::Duration::from_millis(20)))
                .map_err(|error| {
                    ClientRunError::Fatal(format!("设置客户端 H.264 视频流读取超时失败: {error}"))
                })?;
            handle_video_stream_end(
                read_h264_encoded_frames_with_renderer_from_first(stream, frame, None, renderer)
                    .map_err(classify_video_stream_error)?,
            )
        }
        ControlMessage::Error { code, message } => {
            Err(ClientRunError::HostStatus { code, message })
        }
        message => Err(ClientRunError::Fatal(format!(
            "宿主端首帧消息无效: {message:?}"
        ))),
    }
}

fn handle_video_stream_end(end: VideoStreamEnd) -> Result<(), ClientRunError> {
    match end {
        VideoStreamEnd::Frames(_) => Ok(()),
        VideoStreamEnd::HostProgramExited(message) => {
            Err(ClientRunError::NormalSessionEnd(message))
        }
    }
}

fn classify_video_stream_error(message: String) -> ClientRunError {
    if message.contains("视频流中断") {
        ClientRunError::VideoStreamInterrupted(message)
    } else {
        ClientRunError::Fatal(message)
    }
}

pub(crate) fn control_channel_ready_message(config: &ClientConfig) -> String {
    format!(
        "客户端配置有效，已建立宿主端控制通道 {}，已发送会话启动请求。客户端已完成宿主端首个视频响应的解码边界校验；宿主端已接入基础 Windows 输入注入。",
        config.endpoint()
    )
}

fn format_handshake_error(error: HandshakeError) -> String {
    match error {
        HandshakeError::HostRejected { code, message } => format_host_error(code, message),
        HandshakeError::UnsupportedVersion => "协议版本不匹配".to_owned(),
        HandshakeError::InvalidMessage => "宿主端握手消息无效".to_owned(),
        HandshakeError::Frame(error) => format!("控制通道握手失败: {error}"),
    }
}
