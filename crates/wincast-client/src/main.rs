use std::{fs, net::TcpStream, path::PathBuf, process::ExitCode};

use clap::{Parser, Subcommand};
use wincast_protocol::{
    config::ClientConfig,
    frame::read_message,
    handshake::{HandshakeError, read_host_hello, send_client_hello, send_start_session},
    message::{ControlMessage, ErrorCode},
};

const SUPPORTED_CLIENT_TARGETS: &[&str] =
    &["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"];

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
    read_session_start_response(&mut stream)?;

    Ok(control_channel_ready_message(config))
}

fn read_session_start_response(stream: &mut TcpStream) -> Result<(), String> {
    match read_message(stream).map_err(|error| format!("读取宿主端会话响应失败: {error}"))?
    {
        ControlMessage::SessionReady { .. } => Ok(()),
        ControlMessage::Error { code, message } => Err(format_host_error(code, message)),
        message => Err(format!("宿主端会话响应无效: {message:?}")),
    }
}

fn control_channel_ready_message(config: &ClientConfig) -> String {
    format!(
        "客户端配置有效，已建立宿主端控制通道 {}，已发送会话启动请求。后续窗口、视频解码渲染和输入事件链路尚未实现。",
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
        message::{ControlMessage, ErrorCode},
    };

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
                    width: 1280,
                    height: 720,
                },
            )
            .expect("session ready should encode");
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
        assert!(message.contains("视频解码渲染和输入事件链路尚未实现"));
    }
}
