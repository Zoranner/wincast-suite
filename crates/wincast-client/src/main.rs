use std::{fs, net::TcpStream, path::PathBuf, process::ExitCode};
#[cfg(any(test, target_os = "linux"))]
use std::{sync::mpsc, thread, time::Duration};

use clap::{Parser, Subcommand};
use wincast_protocol::{
    config::ClientConfig,
    frame::read_message,
    handshake::{HandshakeError, read_host_hello, send_client_hello, send_start_session},
    message::{ControlMessage, ErrorCode, RawBgraReadbackFrame},
    raw_frame::{RawBgraFrame, read_raw_bgra_frame},
};

const SUPPORTED_CLIENT_TARGETS: &[&str] =
    &["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"];
const RAW_BGRA_VALIDATION_FRAME_COUNT: usize = 1;

#[derive(Debug, Parser)]
#[command(author, version, about = "WinCast Linux 客户端")]
struct Args {
    #[arg(short, long, global = true, default_value = "wincast-client.toml")]
    config: PathBuf,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// 校验客户端配置文件
    Validate,
    /// 校验配置并进入客户端运行入口
    Run,
    /// 输出客户端支持的 Linux 目标平台
    Targets,
}

fn main() -> ExitCode {
    let args = Args::parse();
    let command = args.command.unwrap_or(Command::Run);
    run(command, &args.config)
}

fn run(command: Command, config_path: &PathBuf) -> ExitCode {
    let result = match command {
        Command::Validate => validate_config(config_path),
        Command::Run => run_client(config_path),
        Command::Targets => Ok(supported_targets_message()),
    };

    match result {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn validate_config(path: &PathBuf) -> Result<String, String> {
    let config = load_config(path)?;
    Ok(format!(
        "客户端配置有效，目标 {}，支持平台 {}",
        config.endpoint(),
        SUPPORTED_CLIENT_TARGETS.join(", ")
    ))
}

fn run_client(path: &PathBuf) -> Result<String, String> {
    let config = load_config(path)?;
    run_client_with_config(&config)
}

fn run_client_with_config(config: &ClientConfig) -> Result<String, String> {
    let endpoint = config.endpoint();
    let mut stream = TcpStream::connect(&endpoint)
        .map_err(|error| format!("无法连接宿主端 {endpoint}: {error}"))?;

    send_client_hello(&mut stream).map_err(format_handshake_error)?;
    read_host_hello(&mut stream).map_err(format_handshake_error)?;
    send_start_session(&mut stream).map_err(format_handshake_error)?;
    let render_mode = ClientRenderMode::for_current_platform();
    read_session_start_response(&mut stream, render_mode)?;

    Ok(control_channel_ready_message(config))
}

fn read_session_start_response(
    stream: &mut TcpStream,
    render_mode: ClientRenderMode,
) -> Result<(), String> {
    match read_message(stream).map_err(|error| format!("读取宿主端会话响应失败: {error}"))?
    {
        ControlMessage::SessionReady { width, height } => {
            read_first_readback_frame(stream, render_mode, width, height)
        }
        ControlMessage::Error { code, message } => Err(format_host_error(code, message)),
        message => Err(format!("宿主端会话响应无效: {message:?}")),
    }
}

fn read_first_readback_frame(
    stream: &mut TcpStream,
    render_mode: ClientRenderMode,
    width: u32,
    height: u32,
) -> Result<(), String> {
    match read_message(stream).map_err(|error| format!("读取宿主端首帧失败: {error}"))? {
        ControlMessage::RawBgraReadbackFrame(frame) => validate_readback_frame(&frame),
        ControlMessage::VideoReady => {
            read_first_raw_binary_frame(stream, render_mode, width, height)
        }
        ControlMessage::Error { code, message } => Err(format_host_error(code, message)),
        message => Err(format!("宿主端首帧消息无效: {message:?}")),
    }
}

fn read_first_raw_binary_frame(
    stream: &mut TcpStream,
    render_mode: ClientRenderMode,
    width: u32,
    height: u32,
) -> Result<(), String> {
    match render_mode {
        ClientRenderMode::SdlWindow => {
            read_first_raw_binary_frame_with_sdl_window(stream, width, height)
        }
        ClientRenderMode::ProtocolOnly => {
            read_raw_bgra_frames(stream, RAW_BGRA_VALIDATION_FRAME_COUNT).map(|_| ())
        }
    }
}

#[cfg(target_os = "linux")]
fn read_first_raw_binary_frame_with_sdl_window(
    stream: &mut TcpStream,
    width: u32,
    height: u32,
) -> Result<(), String> {
    let mut renderer = wincast_render::SdlRawBgraRenderer::new(wincast_render::RenderConfig {
        title: "WinCast Client".to_owned(),
        width,
        height,
    })
    .map_err(|error| format!("创建客户端 SDL2 窗口失败: {error}"))?;
    let frame_reader = stream
        .try_clone()
        .map_err(|error| format!("克隆 raw BGRA 视频读取端失败: {error}"))?;
    let frames = spawn_raw_bgra_frame_reader(frame_reader);
    read_raw_bgra_frames_until_renderer_quit(stream, &frames, &mut renderer).map(|_| ())
}

#[cfg(not(target_os = "linux"))]
fn read_first_raw_binary_frame_with_sdl_window(
    _stream: &mut TcpStream,
    _width: u32,
    _height: u32,
) -> Result<(), String> {
    Err("当前平台不支持 SDL2 客户端窗口".to_owned())
}

#[cfg(test)]
fn read_raw_bgra_frames_with_renderer(
    stream: &mut impl ClientStream,
    frame_count: usize,
    renderer: &mut impl wincast_render::RawBgraRenderer,
) -> Result<RawBgraReceiveSummary, String> {
    if frame_count == 0 {
        return Err("raw BGRA 视频帧接收数量不能为 0".to_owned());
    }

    read_raw_bgra_frames_with_renderer_limit(stream, Some(frame_count), renderer)
}

#[cfg(any(test, target_os = "linux"))]
fn read_raw_bgra_frames_until_renderer_quit(
    control_writer: &mut impl std::io::Write,
    frames: &mpsc::Receiver<Result<RawBgraFrame, String>>,
    renderer: &mut impl wincast_render::RawBgraRenderer,
) -> Result<RawBgraReceiveSummary, String> {
    read_raw_bgra_frames_with_renderer_loop(control_writer, frames, None, renderer)
}

#[cfg(test)]
fn read_raw_bgra_frames_with_renderer_limit(
    stream: &mut impl ClientStream,
    frame_limit: Option<usize>,
    renderer: &mut impl wincast_render::RawBgraRenderer,
) -> Result<RawBgraReceiveSummary, String> {
    let (sender, receiver) = mpsc::channel();
    let mut queued_frames = 0;
    loop {
        if frame_limit.is_some_and(|limit| queued_frames == limit) {
            break;
        }
        match read_raw_bgra_frame(stream) {
            Ok(frame) => sender
                .send(Ok(frame))
                .map_err(|_| "raw BGRA 测试帧通道已关闭".to_owned())?,
            Err(error) => {
                sender
                    .send(Err(format!("读取宿主端 raw BGRA 视频帧失败: {error}")))
                    .map_err(|_| "raw BGRA 测试帧通道已关闭".to_owned())?;
                break;
            }
        }
        queued_frames += 1;
    }

    read_raw_bgra_frames_with_renderer_loop(stream, &receiver, frame_limit, renderer)
}

#[cfg(any(test, target_os = "linux"))]
fn read_raw_bgra_frames_with_renderer_loop(
    control_writer: &mut impl std::io::Write,
    frame_receiver: &mpsc::Receiver<Result<RawBgraFrame, String>>,
    frame_limit: Option<usize>,
    renderer: &mut impl wincast_render::RawBgraRenderer,
) -> Result<RawBgraReceiveSummary, String> {
    let mut last_sequence_number = None;
    let mut frame_count = 0;
    let mut quit_requested = false;
    loop {
        if frame_limit.is_some_and(|limit| frame_count == limit) {
            break;
        }

        while let Ok(frame) = frame_receiver.try_recv() {
            let frame = frame?;
            validate_raw_binary_frame(&frame)?;
            if let Some(previous) = last_sequence_number
                && frame.sequence_number < previous
            {
                return Err(format!(
                    "宿主端 raw BGRA 视频帧序号回退: 上一帧 {previous}，当前帧 {}",
                    frame.sequence_number
                ));
            }
            renderer
                .render_frame(&frame)
                .map_err(|error| format!("渲染宿主端 raw BGRA 视频帧失败: {error}"))?;
            last_sequence_number = Some(frame.sequence_number);
            frame_count += 1;
            if frame_limit.is_some_and(|limit| frame_count == limit) {
                break;
            }
        }

        let render_loop = renderer
            .poll_input()
            .map_err(|error| format!("读取客户端输入事件失败: {error}"))?;
        for input_event in render_loop.input_events {
            wincast_protocol::frame::write_message(
                control_writer,
                &ControlMessage::InputEvent(input_event),
            )
            .map_err(|error| format!("发送客户端输入事件失败: {error}"))?;
        }

        if render_loop.action == wincast_render::RenderLoopAction::Quit {
            let _ = wincast_protocol::frame::write_message(
                control_writer,
                &ControlMessage::StopSession,
            );
            quit_requested = true;
            break;
        }

        if frame_limit.is_some_and(|limit| frame_count == limit) {
            break;
        }

        thread::sleep(Duration::from_millis(8));
    }

    let last_sequence_number = match last_sequence_number {
        Some(sequence_number) => sequence_number,
        None if quit_requested => 0,
        None => return Err("未收到 raw BGRA 视频帧".to_owned()),
    };
    Ok(RawBgraReceiveSummary {
        frames: frame_count,
        last_sequence_number,
    })
}

#[cfg(target_os = "linux")]
fn spawn_raw_bgra_frame_reader(
    mut reader: impl std::io::Read + Send + 'static,
) -> mpsc::Receiver<Result<RawBgraFrame, String>> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        loop {
            let result = read_raw_bgra_frame(&mut reader)
                .map_err(|error| format!("读取宿主端 raw BGRA 视频帧失败: {error}"));
            let should_stop = result.is_err();
            if sender.send(result).is_err() || should_stop {
                break;
            }
        }
    });
    receiver
}

#[cfg(test)]
trait ClientStream: std::io::Read + std::io::Write {}

#[cfg(test)]
impl<T: std::io::Read + std::io::Write> ClientStream for T {}

fn read_raw_bgra_frames(
    reader: &mut impl std::io::Read,
    frame_count: usize,
) -> Result<RawBgraReceiveSummary, String> {
    if frame_count == 0 {
        return Err("raw BGRA 视频帧接收数量不能为 0".to_owned());
    }

    let mut last_sequence_number = None;
    for _ in 0..frame_count {
        let frame = read_raw_bgra_frame(reader)
            .map_err(|error| format!("读取宿主端 raw BGRA 视频帧失败: {error}"))?;
        validate_raw_binary_frame(&frame)?;
        if let Some(previous) = last_sequence_number
            && frame.sequence_number < previous
        {
            return Err(format!(
                "宿主端 raw BGRA 视频帧序号回退: 上一帧 {previous}，当前帧 {}",
                frame.sequence_number
            ));
        }
        last_sequence_number = Some(frame.sequence_number);
    }

    Ok(RawBgraReceiveSummary {
        frames: frame_count,
        last_sequence_number: last_sequence_number.expect("frame_count is non-zero"),
    })
}

fn validate_readback_frame(frame: &RawBgraReadbackFrame) -> Result<(), String> {
    frame
        .validate()
        .map_err(|error| format!("宿主端首帧 BGRA readback 无效: {error:?}"))
}

fn validate_raw_binary_frame(frame: &RawBgraFrame) -> Result<(), String> {
    frame
        .validate()
        .map_err(|error| format!("宿主端 raw BGRA 视频帧无效: {error}"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RawBgraReceiveSummary {
    frames: usize,
    last_sequence_number: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClientRenderMode {
    SdlWindow,
    ProtocolOnly,
}

impl ClientRenderMode {
    fn for_current_platform() -> Self {
        if cfg!(target_os = "linux") {
            Self::SdlWindow
        } else {
            Self::ProtocolOnly
        }
    }
}

fn control_channel_ready_message(config: &ClientConfig) -> String {
    format!(
        "客户端配置有效，已建立宿主端控制通道 {}，已发送会话启动请求。Linux 客户端已接入 raw BGRA 窗口渲染和输入事件回传；宿主端已接入基础 Windows 输入注入，H.264/WebRTC 编码传输尚未实现。",
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

fn format_host_error(code: ErrorCode, message: String) -> String {
    match code {
        ErrorCode::Busy => format!("宿主端忙碌: {message}"),
        ErrorCode::UnsupportedVersion => format!("协议版本不匹配: {message}"),
        ErrorCode::EncodingFailed => format!("宿主端视频编码失败: {message}"),
        _ => format!("宿主端拒绝连接: {message}"),
    }
}

fn supported_targets_message() -> String {
    format!(
        "客户端支持平台：Linux x86_64 ({})；Linux aarch64/ARM64 ({})",
        SUPPORTED_CLIENT_TARGETS[0], SUPPORTED_CLIENT_TARGETS[1]
    )
}

fn load_config(path: &PathBuf) -> Result<ClientConfig, String> {
    let source =
        fs::read_to_string(path).map_err(|error| format!("读取客户端配置失败: {error}"))?;
    ClientConfig::from_toml_str(&source).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{net::TcpListener, sync::mpsc, thread, time::Duration};
    use wincast_protocol::{
        frame::write_message,
        handshake::send_client_hello,
        input::{ButtonState, InputEvent, Modifiers},
        message::{ControlMessage, ErrorCode},
        raw_frame::{RawBgraFrame, write_raw_bgra_frame},
    };
    use wincast_render::{RawBgraRenderer, RenderError, RenderLoopAction, RenderLoopResult};

    #[test]
    fn client_run_performs_tcp_control_handshake() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let endpoint = listener
            .local_addr()
            .expect("listener address should exist");
        let (observed_messages_tx, observed_messages_rx) = mpsc::channel();

        let host_thread = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("client should connect");
            let hello = read_message(&mut stream).expect("client hello should decode");
            send_client_hello(&mut stream).expect("host hello should encode");
            let start_session = read_message(&mut stream).expect("start session should decode");
            write_message(
                &mut stream,
                &ControlMessage::SessionReady {
                    width: 2,
                    height: 2,
                },
            )
            .expect("session ready should encode");
            write_message(&mut stream, &ControlMessage::VideoReady)
                .expect("video ready should encode");
            write_raw_bgra_frame(&mut stream, &raw_binary_frame())
                .expect("raw binary frame should encode");
            observed_messages_tx
                .send((hello, start_session))
                .expect("observed messages should send");
        });

        let config = ClientConfig {
            host: endpoint.ip().to_string(),
            port: endpoint.port(),
        };

        let message =
            run_client_with_config(&config).expect("client run should complete handshake");

        let observed_messages = observed_messages_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("host should observe client control messages");
        host_thread.join().expect("host thread should finish");
        assert_eq!(
            observed_messages,
            (
                ControlMessage::Hello {
                    version: wincast_protocol::handshake::PROTOCOL_VERSION,
                },
                ControlMessage::StartSession,
            )
        );
        assert!(message.contains("已建立宿主端控制通道"));
        assert!(message.contains(&config.endpoint()));
        assert!(!message.contains("运行时链路未实现"));
    }

    #[test]
    fn client_run_reports_runtime_unimplemented_response_from_host() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let endpoint = listener
            .local_addr()
            .expect("listener address should exist");

        let host_thread = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("client should connect");
            read_message(&mut stream).expect("client hello should decode");
            send_client_hello(&mut stream).expect("host hello should encode");
            read_message(&mut stream).expect("start session should decode");
            write_message(
                &mut stream,
                &ControlMessage::Error {
                    code: ErrorCode::TransportFailed,
                    message:
                        "运行时链路未实现：尚未启动程序生命周期、画面捕获、编码传输和输入注入。"
                            .to_owned(),
                },
            )
            .expect("runtime error should encode");
        });

        let config = ClientConfig {
            host: endpoint.ip().to_string(),
            port: endpoint.port(),
        };

        let error = run_client_with_config(&config).expect_err("runtime unimplemented should fail");

        host_thread.join().expect("host thread should finish");
        assert!(error.contains("宿主端拒绝连接"));
        assert!(error.contains("运行时链路未实现"));
    }

    #[test]
    fn client_reports_encoding_failure_in_chinese() {
        let error = format_host_error(
            ErrorCode::EncodingFailed,
            "Windows 视频编码器未实现：尚未接入 H.264 编码器。".to_owned(),
        );

        assert!(error.contains("宿主端视频编码失败"));
        assert!(error.contains("尚未接入 H.264 编码器"));
    }

    #[test]
    fn client_rejects_invalid_raw_bgra_frame_from_host() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let endpoint = listener
            .local_addr()
            .expect("listener address should exist");

        let host_thread = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("client should connect");
            read_message(&mut stream).expect("client hello should decode");
            send_client_hello(&mut stream).expect("host hello should encode");
            read_message(&mut stream).expect("start session should decode");
            write_message(
                &mut stream,
                &ControlMessage::SessionReady {
                    width: 2,
                    height: 2,
                },
            )
            .expect("session ready should encode");
            let mut frame = raw_bgra_frame();
            frame.bytes.pop();
            write_message(&mut stream, &ControlMessage::RawBgraReadbackFrame(frame))
                .expect("raw frame should encode");
        });

        let config = ClientConfig {
            host: endpoint.ip().to_string(),
            port: endpoint.port(),
        };

        let error = run_client_with_config(&config).expect_err("invalid frame should fail");

        host_thread.join().expect("host thread should finish");
        assert!(error.contains("首帧 BGRA readback 无效"));
    }

    #[test]
    fn client_rejects_invalid_message_after_video_ready() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let endpoint = listener
            .local_addr()
            .expect("listener address should exist");

        let host_thread = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("client should connect");
            read_message(&mut stream).expect("client hello should decode");
            send_client_hello(&mut stream).expect("host hello should encode");
            read_message(&mut stream).expect("start session should decode");
            write_message(
                &mut stream,
                &ControlMessage::SessionReady {
                    width: 2,
                    height: 2,
                },
            )
            .expect("session ready should encode");
            write_message(&mut stream, &ControlMessage::VideoReady)
                .expect("video ready should encode");
            write_message(&mut stream, &ControlMessage::Heartbeat)
                .expect("heartbeat should encode");
        });

        let config = ClientConfig {
            host: endpoint.ip().to_string(),
            port: endpoint.port(),
        };

        let error = run_client_with_config(&config).expect_err("invalid message should fail");

        host_thread.join().expect("host thread should finish");
        assert!(error.contains("raw BGRA 视频帧失败"));
    }

    #[test]
    fn client_reads_multiple_raw_bgra_frames_after_video_ready() {
        let mut bytes = Vec::new();
        for sequence_number in 0..3 {
            write_raw_bgra_frame(&mut bytes, &raw_binary_frame_with_sequence(sequence_number))
                .expect("raw binary frame should encode");
        }

        let summary = read_raw_bgra_frames(&mut bytes.as_slice(), 3)
            .expect("raw frame loop should accept three frames");

        assert_eq!(
            summary,
            RawBgraReceiveSummary {
                frames: 3,
                last_sequence_number: 2,
            }
        );
    }

    #[test]
    fn client_renders_raw_bgra_frames_after_video_ready() {
        let mut bytes = Vec::new();
        for sequence_number in 0..2 {
            write_raw_bgra_frame(&mut bytes, &raw_binary_frame_with_sequence(sequence_number))
                .expect("raw binary frame should encode");
        }
        let mut stream = DuplexBuffer::new(bytes);
        let mut renderer = RecordingRenderer::default();

        let summary = read_raw_bgra_frames_with_renderer(&mut stream, 2, &mut renderer)
            .expect("raw frame loop should render frames");

        assert_eq!(
            summary,
            RawBgraReceiveSummary {
                frames: 2,
                last_sequence_number: 1,
            }
        );
        assert_eq!(renderer.rendered_sequences, vec![0, 1]);
    }

    #[test]
    fn client_continues_rendering_raw_bgra_frames_until_renderer_quits() {
        let mut bytes = Vec::new();
        for sequence_number in 0..3 {
            write_raw_bgra_frame(&mut bytes, &raw_binary_frame_with_sequence(sequence_number))
                .expect("raw binary frame should encode");
        }
        let mut stream = DuplexBuffer::new(bytes);
        let mut renderer = RecordingRenderer {
            actions: Vec::from([
                RenderLoopAction::Continue,
                RenderLoopAction::Continue,
                RenderLoopAction::Quit,
            ]),
            ..Default::default()
        };

        let frames = queued_raw_bgra_frames([
            raw_binary_frame_with_sequence(0),
            raw_binary_frame_with_sequence(1),
            raw_binary_frame_with_sequence(2),
        ]);

        let summary = read_raw_bgra_frames_until_renderer_quit(&mut stream, &frames, &mut renderer)
            .expect("raw frame loop should continue until renderer quits");

        assert_eq!(
            summary,
            RawBgraReceiveSummary {
                frames: 3,
                last_sequence_number: 2,
            }
        );
        assert_eq!(renderer.rendered_sequences, vec![0, 1, 2]);
    }

    #[test]
    fn client_sends_stop_session_when_renderer_quits() {
        let mut bytes = Vec::new();
        write_raw_bgra_frame(&mut bytes, &raw_binary_frame())
            .expect("raw binary frame should encode");
        let mut stream = DuplexBuffer::new(bytes);
        let mut renderer = RecordingRenderer {
            actions: Vec::from([RenderLoopAction::Quit]),
            ..Default::default()
        };
        let frames = queued_raw_bgra_frames([raw_binary_frame()]);

        read_raw_bgra_frames_until_renderer_quit(&mut stream, &frames, &mut renderer)
            .expect("quit should end frame loop cleanly");

        let mut written = stream.written.as_slice();
        assert_eq!(
            read_message(&mut written).expect("stop session message should decode"),
            ControlMessage::StopSession
        );
    }

    #[test]
    fn client_still_exits_when_stop_session_send_fails_after_renderer_quit() {
        let mut bytes = Vec::new();
        write_raw_bgra_frame(&mut bytes, &raw_binary_frame())
            .expect("raw binary frame should encode");
        let mut stream = FailingWriteStream::new(bytes);
        let mut renderer = RecordingRenderer {
            actions: Vec::from([RenderLoopAction::Quit]),
            ..Default::default()
        };
        let frames = queued_raw_bgra_frames([raw_binary_frame()]);

        let summary = read_raw_bgra_frames_until_renderer_quit(&mut stream, &frames, &mut renderer)
            .expect("stop session send failure should not block local quit");

        assert_eq!(
            summary,
            RawBgraReceiveSummary {
                frames: 1,
                last_sequence_number: 0,
            }
        );
    }

    #[test]
    fn client_can_quit_even_when_no_new_raw_frame_is_available() {
        let mut stream = DuplexBuffer::new(Vec::new());
        let frames = queued_raw_bgra_frames([raw_binary_frame()]);
        let mut renderer = RecordingRenderer {
            actions: Vec::from([RenderLoopAction::Continue, RenderLoopAction::Quit]),
            ..Default::default()
        };

        read_raw_bgra_frames_until_renderer_quit(&mut stream, &frames, &mut renderer)
            .expect("quit should not wait for another raw frame");

        let mut written = stream.written.as_slice();
        assert_eq!(
            read_message(&mut written).expect("stop session message should decode"),
            ControlMessage::StopSession
        );
        assert_eq!(renderer.rendered_sequences, vec![0]);
    }

    #[test]
    fn client_can_quit_before_first_raw_frame_arrives() {
        let mut stream = DuplexBuffer::new(Vec::new());
        let (_sender, frames) = mpsc::channel();
        let mut renderer = RecordingRenderer {
            actions: Vec::from([RenderLoopAction::Quit]),
            ..Default::default()
        };

        let summary = read_raw_bgra_frames_until_renderer_quit(&mut stream, &frames, &mut renderer)
            .expect("quit before first frame should end cleanly");

        assert_eq!(
            summary,
            RawBgraReceiveSummary {
                frames: 0,
                last_sequence_number: 0,
            }
        );
        let mut written = stream.written.as_slice();
        assert_eq!(
            read_message(&mut written).expect("stop session message should decode"),
            ControlMessage::StopSession
        );
        assert!(renderer.rendered_sequences.is_empty());
    }

    #[test]
    fn client_sends_renderer_input_events_after_rendering_raw_frame() {
        let mut bytes = Vec::new();
        write_raw_bgra_frame(&mut bytes, &raw_binary_frame())
            .expect("raw binary frame should encode");
        let mut stream = DuplexBuffer::new(bytes);
        let mut renderer = RecordingRenderer {
            input_batches: Vec::from([vec![InputEvent::Key {
                code: 65,
                state: ButtonState::Pressed,
                modifiers: Modifiers {
                    shift: true,
                    ctrl: false,
                    alt: false,
                    logo: false,
                },
            }]]),
            ..Default::default()
        };

        read_raw_bgra_frames_with_renderer(&mut stream, 1, &mut renderer)
            .expect("input event should send after rendering frame");

        let mut written = stream.written.as_slice();
        assert_eq!(
            read_message(&mut written).expect("input message should decode"),
            ControlMessage::InputEvent(InputEvent::Key {
                code: 65,
                state: ButtonState::Pressed,
                modifiers: Modifiers {
                    shift: true,
                    ctrl: false,
                    alt: false,
                    logo: false,
                },
            })
        );
    }

    #[test]
    fn client_reports_input_send_errors_in_chinese() {
        let mut bytes = Vec::new();
        write_raw_bgra_frame(&mut bytes, &raw_binary_frame())
            .expect("raw binary frame should encode");
        let mut stream = FailingWriteStream::new(bytes);
        let mut renderer = RecordingRenderer {
            input_batches: Vec::from([vec![InputEvent::MouseWheel {
                delta_x: 0,
                delta_y: 1,
            }]]),
            ..Default::default()
        };

        let error = read_raw_bgra_frames_with_renderer(&mut stream, 1, &mut renderer)
            .expect_err("input send failure should fail frame loop");

        assert!(error.contains("发送客户端输入事件失败"));
    }

    #[test]
    fn client_reports_renderer_errors_in_chinese() {
        let mut bytes = Vec::new();
        write_raw_bgra_frame(&mut bytes, &raw_binary_frame())
            .expect("raw binary frame should encode");
        let mut stream = DuplexBuffer::new(bytes);
        let mut renderer = FailingRenderer;

        let error = read_raw_bgra_frames_with_renderer(&mut stream, 1, &mut renderer)
            .expect_err("renderer failure should fail client frame loop");

        assert!(error.contains("渲染宿主端 raw BGRA 视频帧失败"));
        assert!(error.contains("测试渲染失败"));
    }

    #[test]
    fn client_rejects_sequence_number_regression_in_raw_bgra_loop() {
        let mut bytes = Vec::new();
        write_raw_bgra_frame(&mut bytes, &raw_binary_frame_with_sequence(2))
            .expect("first frame should encode");
        write_raw_bgra_frame(&mut bytes, &raw_binary_frame_with_sequence(1))
            .expect("second frame should encode");

        let error = read_raw_bgra_frames(&mut bytes.as_slice(), 2)
            .expect_err("sequence regression should fail");

        assert!(error.contains("raw BGRA 视频帧序号回退"));
    }

    #[test]
    fn parses_run_command_with_config_path() {
        let args =
            Args::try_parse_from(["wincast-client", "--config", "custom-client.toml", "run"])
                .expect("args should parse");

        assert_eq!(args.config, PathBuf::from("custom-client.toml"));
        match args.command {
            Some(Command::Run) => {}
            _ => panic!("run command should parse"),
        }
    }

    #[test]
    fn client_run_reports_host_error_in_chinese() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let endpoint = listener
            .local_addr()
            .expect("listener address should exist");

        let host_thread = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("client should connect");
            read_message(&mut stream).expect("client hello should decode");
            write_message(
                &mut stream,
                &ControlMessage::Error {
                    code: ErrorCode::Busy,
                    message: "宿主端已有客户端连接".to_owned(),
                },
            )
            .expect("host error should encode");
        });

        let config = ClientConfig {
            host: endpoint.ip().to_string(),
            port: endpoint.port(),
        };

        let error = run_client_with_config(&config).expect_err("host error should fail client run");

        host_thread.join().expect("host thread should finish");
        assert!(error.contains("宿主端忙碌"));
        assert!(error.contains("宿主端已有客户端连接"));
    }

    #[test]
    fn parses_default_run_with_config_path() {
        let args = Args::try_parse_from(["wincast-client", "--config", "custom-client.toml"])
            .expect("args should parse");

        assert_eq!(args.config, PathBuf::from("custom-client.toml"));
        assert!(args.command.is_none());
    }

    #[test]
    fn targets_message_lists_x86_64_and_arm64_linux() {
        let message = supported_targets_message();

        assert!(message.contains("x86_64-unknown-linux-gnu"));
        assert!(message.contains("aarch64-unknown-linux-gnu"));
        assert!(message.contains("ARM64"));
    }

    #[test]
    fn run_message_does_not_claim_runtime_chain_is_ready() {
        let config = ClientConfig {
            host: "192.168.10.25".to_owned(),
            port: 7856,
        };

        let message = control_channel_ready_message(&config);

        assert!(message.contains("已建立宿主端控制通道"));
        assert!(message.contains("Linux 客户端已接入 raw BGRA 窗口渲染和输入事件回传"));
        assert!(message.contains("宿主端已接入基础 Windows 输入注入"));
        assert!(message.contains("H.264/WebRTC 编码传输尚未实现"));
        assert!(!message.contains("视频解码渲染和输入事件链路尚未实现"));
    }

    #[test]
    fn current_platform_uses_protocol_only_render_mode_outside_linux() {
        if !cfg!(target_os = "linux") {
            assert_eq!(
                ClientRenderMode::for_current_platform(),
                ClientRenderMode::ProtocolOnly
            );
        }
    }

    fn raw_bgra_frame() -> RawBgraReadbackFrame {
        RawBgraReadbackFrame {
            width: 2,
            height: 2,
            stride_bytes: 8,
            texture_width: 2,
            texture_height: 2,
            row_pitch: 8,
            sequence_number: 0,
            timestamp_ns: 0,
            bytes: vec![0; 16],
        }
    }

    fn raw_binary_frame() -> RawBgraFrame {
        raw_binary_frame_with_sequence(0)
    }

    fn raw_binary_frame_with_sequence(sequence_number: u64) -> RawBgraFrame {
        RawBgraFrame {
            width: 2,
            height: 2,
            row_pitch: 8,
            sequence_number,
            timestamp_ns: sequence_number * 1_000_000,
            bytes: vec![0; 16],
        }
    }

    fn queued_raw_bgra_frames(
        frames: impl IntoIterator<Item = RawBgraFrame>,
    ) -> mpsc::Receiver<Result<RawBgraFrame, String>> {
        let (sender, receiver) = mpsc::channel();
        for frame in frames {
            sender.send(Ok(frame)).expect("test frame should queue");
        }
        receiver
    }

    #[derive(Default)]
    struct RecordingRenderer {
        rendered_sequences: Vec<u64>,
        input_batches: Vec<Vec<InputEvent>>,
        actions: Vec<RenderLoopAction>,
    }

    impl RawBgraRenderer for RecordingRenderer {
        fn render_frame(&mut self, frame: &RawBgraFrame) -> Result<(), RenderError> {
            self.rendered_sequences.push(frame.sequence_number);
            Ok(())
        }

        fn poll_input(&mut self) -> Result<RenderLoopResult, RenderError> {
            Ok(RenderLoopResult {
                action: if self.actions.is_empty() {
                    RenderLoopAction::Continue
                } else {
                    self.actions.remove(0)
                },
                input_events: if self.input_batches.is_empty() {
                    Vec::new()
                } else {
                    self.input_batches.remove(0)
                },
            })
        }
    }

    struct FailingRenderer;

    impl RawBgraRenderer for FailingRenderer {
        fn render_frame(&mut self, _frame: &RawBgraFrame) -> Result<(), RenderError> {
            Err(RenderError::Backend("测试渲染失败".to_owned()))
        }

        fn poll_input(&mut self) -> Result<RenderLoopResult, RenderError> {
            Ok(RenderLoopResult {
                action: RenderLoopAction::Continue,
                input_events: Vec::new(),
            })
        }
    }

    struct DuplexBuffer {
        read: std::io::Cursor<Vec<u8>>,
        written: Vec<u8>,
    }

    impl DuplexBuffer {
        fn new(read: Vec<u8>) -> Self {
            Self {
                read: std::io::Cursor::new(read),
                written: Vec::new(),
            }
        }
    }

    impl std::io::Read for DuplexBuffer {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.read.read(buf)
        }
    }

    impl std::io::Write for DuplexBuffer {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.written.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct FailingWriteStream {
        read: std::io::Cursor<Vec<u8>>,
    }

    impl FailingWriteStream {
        fn new(read: Vec<u8>) -> Self {
            Self {
                read: std::io::Cursor::new(read),
            }
        }
    }

    impl std::io::Read for FailingWriteStream {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.read.read(buf)
        }
    }

    impl std::io::Write for FailingWriteStream {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "write failed",
            ))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
}
