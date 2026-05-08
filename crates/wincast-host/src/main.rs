use std::{
    fs,
    net::{SocketAddr, TcpListener, TcpStream},
    path::PathBuf,
    process::ExitCode,
};

use clap::{Parser, Subcommand};
use wincast_protocol::{
    config::HostConfig,
    frame::{read_message, write_message},
    handshake::accept_client_hello,
    message::{ControlMessage, ErrorCode},
};

#[derive(Debug, Parser)]
#[command(author, version, about = "WinCast Windows 宿主端")]
struct Args {
    #[arg(short, long, global = true, default_value = "wincast-host.toml")]
    config: PathBuf,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// 校验宿主端配置文件
    Validate,
    /// 校验配置并进入宿主端运行入口
    Run,
}

fn main() -> ExitCode {
    let args = Args::parse();
    let command = args.command.unwrap_or(Command::Run);
    run(command, &args.config)
}

fn run(command: Command, config_path: &PathBuf) -> ExitCode {
    let result = match command {
        Command::Validate => validate_config(config_path),
        Command::Run => run_host(config_path),
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
        "宿主端配置有效，监听 {}，程序 {}，视频 {}x{}@{}fps",
        config.listen, config.program, config.video.width, config.video.height, config.video.fps
    ))
}

fn run_host(path: &PathBuf) -> Result<String, String> {
    let config = load_config(path)?;
    let listener = TcpListener::bind(&config.listen)
        .map_err(|error| format!("宿主端 TCP 监听失败: {error}"))?;
    let startup_message = runtime_not_implemented_message(&config);
    let local_addr = run_control_listener_once(listener)?;
    Ok(format!(
        "{startup_message} 控制通道已处理一个客户端连接，实际监听 {local_addr}。"
    ))
}

fn runtime_not_implemented_message(config: &HostConfig) -> String {
    format!(
        "宿主端配置有效，监听 {}，程序 {}。{}",
        config.listen,
        config.program,
        runtime_not_implemented_detail()
    )
}

fn runtime_not_implemented_detail() -> &'static str {
    "运行时链路未实现：尚未启动程序生命周期、画面捕获、编码传输和输入注入。"
}

fn run_control_listener_once(listener: TcpListener) -> Result<SocketAddr, String> {
    let local_addr = listener
        .local_addr()
        .map_err(|error| format!("读取宿主端监听地址失败: {error}"))?;
    let (mut stream, _peer_addr) = listener
        .accept()
        .map_err(|error| format!("接受客户端连接失败: {error}"))?;
    handle_control_client(&mut stream)?;
    Ok(local_addr)
}

fn handle_control_client(stream: &mut TcpStream) -> Result<(), String> {
    let mut writer = stream
        .try_clone()
        .map_err(|error| format!("克隆控制连接写入端失败: {error}"))?;
    accept_client_hello(stream, &mut writer).map_err(|error| format!("控制握手失败: {error}"))?;

    match read_message(stream).map_err(|error| format!("读取控制消息失败: {error}"))? {
        ControlMessage::StartSession => {
            write_runtime_not_implemented(&mut writer)?;
            Ok(())
        }
        message => {
            write_message(
                &mut writer,
                &ControlMessage::Error {
                    code: ErrorCode::TransportFailed,
                    message: format!("控制消息顺序无效，期望 StartSession，实际收到 {message:?}"),
                },
            )
            .map_err(|error| format!("写入控制错误消息失败: {error}"))?;
            Err("控制消息顺序无效，期望 StartSession".to_owned())
        }
    }
}

fn write_runtime_not_implemented(writer: &mut impl std::io::Write) -> Result<(), String> {
    write_message(
        writer,
        &ControlMessage::Error {
            code: ErrorCode::TransportFailed,
            message: runtime_not_implemented_detail().to_owned(),
        },
    )
    .map_err(|error| format!("写入运行时未实现消息失败: {error}"))
}

fn load_config(path: &PathBuf) -> Result<HostConfig, String> {
    let source =
        fs::read_to_string(path).map_err(|error| format!("读取宿主端配置失败: {error}"))?;
    HostConfig::from_toml_str(&source).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        net::{TcpListener, TcpStream},
        thread,
    };
    use wincast_protocol::{
        config::{CaptureConfig, CaptureMode, VideoCodec, VideoConfig},
        frame::read_message,
        handshake::send_client_hello,
        message::{ControlMessage, ErrorCode},
    };

    #[test]
    fn parses_validate_command_with_config_path() {
        let args =
            Args::try_parse_from(["wincast-host", "--config", "custom-host.toml", "validate"])
                .expect("args should parse");

        assert_eq!(args.config, PathBuf::from("custom-host.toml"));
        match args.command {
            Some(Command::Validate) => {}
            _ => panic!("validate command should parse"),
        }
    }

    #[test]
    fn parses_default_run_with_config_path() {
        let args = Args::try_parse_from(["wincast-host", "--config", "custom-host.toml"])
            .expect("args should parse");

        assert_eq!(args.config, PathBuf::from("custom-host.toml"));
        assert!(args.command.is_none());
    }

    #[test]
    fn runtime_message_does_not_claim_runtime_chain_is_ready() {
        let config = host_config("0.0.0.0:7856".to_owned());

        let message = runtime_not_implemented_message(&config);

        assert!(message.contains("运行时链路未实现"));
        assert!(message.contains("尚未启动程序生命周期"));
    }

    #[test]
    fn host_accepts_one_tcp_control_handshake_before_reporting_runtime_unimplemented() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let endpoint = listener
            .local_addr()
            .expect("listener addr should be available");
        let host = thread::spawn(move || run_control_listener_once(listener));

        let mut client = TcpStream::connect(endpoint).expect("client should connect");
        send_client_hello(&mut client).expect("client hello should write");
        assert_eq!(
            read_message(&mut client).expect("host hello should read"),
            ControlMessage::Hello { version: 1 }
        );

        wincast_protocol::frame::write_message(&mut client, &ControlMessage::StartSession)
            .expect("start session should write");

        assert_eq!(
            read_message(&mut client).expect("runtime error should read"),
            ControlMessage::Error {
                code: ErrorCode::TransportFailed,
                message: "运行时链路未实现：尚未启动程序生命周期、画面捕获、编码传输和输入注入。"
                    .to_owned(),
            }
        );

        let host_result = host.join().expect("host thread should finish");
        assert_eq!(
            host_result.expect("host should handle one client"),
            endpoint
        );
    }

    fn host_config(listen: String) -> HostConfig {
        HostConfig {
            listen,
            program: "C:\\Program Files\\SomeApp\\app.exe".to_owned(),
            args: Vec::new(),
            work_dir: "C:\\Program Files\\SomeApp".to_owned(),
            video: VideoConfig {
                width: 1280,
                height: 720,
                fps: 30,
                codec: VideoCodec::H264,
                bitrate_kbps: 4000,
            },
            capture: CaptureConfig {
                mode: CaptureMode::Desktop,
                window_title_contains: String::new(),
                startup_timeout_ms: 15000,
            },
        }
    }
}
