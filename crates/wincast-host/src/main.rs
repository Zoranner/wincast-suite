use std::{fs, path::PathBuf, process::ExitCode};

use clap::{Parser, Subcommand};
use wincast_protocol::config::HostConfig;

mod agent;
mod agent_runtime;
mod program;
mod service;
pub mod service_ipc;
pub mod session_events;
pub mod session_state;
mod window;

use agent_runtime::{HostAgentRuntime, StdHostAgentRuntime};
#[cfg(test)]
use service::ServiceStatus;
use service::{DefaultServiceManager, ServiceManager};

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
    /// 管理 Windows Service 入口
    #[command(subcommand)]
    Service(ServiceCommand),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Subcommand)]
enum ServiceCommand {
    /// 安装 Windows Service
    Install,
    /// 卸载 Windows Service
    Uninstall,
    /// 启动 Windows Service
    Start,
    /// 停止 Windows Service
    Stop,
    /// 查看 Windows Service 状态
    Status,
}

fn main() -> ExitCode {
    let args = Args::parse();
    let command = args.command.unwrap_or(Command::Run);
    run(command, &args.config)
}

fn run(command: Command, config_path: &PathBuf) -> ExitCode {
    let result = execute_command(command, config_path);

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

fn execute_command(command: Command, config_path: &PathBuf) -> Result<String, String> {
    let mut service_manager = DefaultServiceManager::default();
    execute_command_with_service_manager(command, config_path, &mut service_manager)
}

fn execute_command_with_service_manager(
    command: Command,
    config_path: &PathBuf,
    service_manager: &mut impl ServiceManager,
) -> Result<String, String> {
    match command {
        Command::Validate => validate_config(config_path),
        Command::Run => run_host(config_path),
        Command::Service(command) => execute_service_command(command, service_manager),
    }
}

fn execute_service_command(
    command: ServiceCommand,
    service_manager: &mut impl ServiceManager,
) -> Result<String, String> {
    match command {
        ServiceCommand::Install => service_manager.install(),
        ServiceCommand::Uninstall => service_manager.uninstall(),
        ServiceCommand::Start => service_manager.start(),
        ServiceCommand::Stop => service_manager.stop(),
        ServiceCommand::Status => service_manager
            .status()
            .map(|status| status.message().to_owned()),
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
    let mut runtime = StdHostAgentRuntime;
    run_host_with_runtime(path, &mut runtime)
}

fn run_host_with_runtime(
    path: &PathBuf,
    runtime: &mut impl HostAgentRuntime,
) -> Result<String, String> {
    let config = load_config(path)?;
    let startup_message = runtime_not_implemented_message(&config);
    let local_addr = runtime.run(&config)?;
    Ok(format!(
        "{startup_message} 控制通道已进入持续监听，实际监听 {local_addr}。"
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
    "raw BGRA 画面链路已接入，H.264/WebRTC 编码传输尚未接入。"
}

fn load_config(path: &PathBuf) -> Result<HostConfig, String> {
    let source =
        fs::read_to_string(path).map_err(|error| format!("读取宿主端配置失败: {error}"))?;
    HostConfig::from_toml_str(&source).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wincast_protocol::config::{CaptureConfig, CaptureMode, VideoCodec, VideoConfig};

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
    fn parses_service_subcommands() {
        for (name, expected) in [
            ("install", ServiceCommand::Install),
            ("uninstall", ServiceCommand::Uninstall),
            ("start", ServiceCommand::Start),
            ("stop", ServiceCommand::Stop),
            ("status", ServiceCommand::Status),
        ] {
            let args = Args::try_parse_from(["wincast-host", "service", name])
                .expect("service command should parse");

            match args.command {
                Some(Command::Service(actual)) => {
                    assert_eq!(actual, expected);
                }
                _ => panic!("service {name} should parse"),
            }
        }
    }

    #[test]
    fn service_commands_report_explicitly_not_implemented() {
        for command in [
            ServiceCommand::Install,
            ServiceCommand::Uninstall,
            ServiceCommand::Start,
            ServiceCommand::Stop,
            ServiceCommand::Status,
        ] {
            let message = execute_command(Command::Service(command), &PathBuf::from("unused.toml"))
                .unwrap_or_else(|error| error);

            assert!(message.contains("Windows Service"));
            assert!(message.contains("未实现"));
            assert!(message.contains("当前仍需使用前台 run 模式"));
            if !matches!(command, ServiceCommand::Status) {
                assert!(message.contains("未执行真实系统服务操作"));
            }
            assert!(!message.contains("安装成功"));
            assert!(!message.contains("Service 已安装"));
            assert!(!message.contains("已启动"));
        }
    }

    #[test]
    fn service_commands_are_dispatched_through_manager() {
        for (command, expected_call) in [
            (ServiceCommand::Install, "install"),
            (ServiceCommand::Uninstall, "uninstall"),
            (ServiceCommand::Start, "start"),
            (ServiceCommand::Stop, "stop"),
            (ServiceCommand::Status, "status"),
        ] {
            let mut manager = RecordingServiceManager::default();
            let message = execute_command_with_service_manager(
                Command::Service(command),
                &PathBuf::from("unused.toml"),
                &mut manager,
            )
            .expect("service command should return manager message");

            assert_eq!(manager.calls, vec![expected_call]);
            if matches!(command, ServiceCommand::Status) {
                assert!(message.contains("未实现"));
            } else {
                assert!(message.contains(expected_call));
            }
        }
    }

    #[test]
    fn pending_service_status_reports_clear_pending_state() {
        let mut manager = DefaultServiceManager::default();
        let message = execute_command_with_service_manager(
            Command::Service(ServiceCommand::Status),
            &PathBuf::from("unused.toml"),
            &mut manager,
        )
        .expect("status should return a user-facing message");

        assert!(message.contains("未实现"));
        assert!(message.contains("未安装"));
        assert!(message.contains("当前仍需使用前台 run 模式"));
        assert!(!message.contains("安装成功"));
        assert!(!message.contains("已启动"));
    }

    #[test]
    fn runtime_message_reports_raw_bgra_ready_and_codec_transport_pending() {
        let config = host_config("0.0.0.0:7856".to_owned());

        let message = runtime_not_implemented_message(&config);

        assert!(!message.contains("运行时链路未实现"));
        assert!(message.contains("raw BGRA 画面链路已接入"));
        assert!(message.contains("H.264/WebRTC 编码传输尚未接入"));
    }

    #[test]
    fn run_command_loads_config_and_delegates_host_agent_runtime() {
        let config_path = temp_host_config_path("run-delegates-runtime");
        fs::write(
            &config_path,
            r#"
listen = "127.0.0.1:0"
program = "C:\\Program Files\\SomeApp\\app.exe"
args = ["--profile", "demo"]
work_dir = "C:\\Program Files\\SomeApp"

[video]
width = 1280
height = 720
fps = 30
codec = "h264"
bitrate_kbps = 4000

[capture]
mode = "desktop"
window_title_contains = ""
startup_timeout_ms = 15000
"#,
        )
        .expect("host config should be written");
        let mut runtime = RecordingHostAgentRuntime::default();

        let message = run_host_with_runtime(&config_path, &mut runtime)
            .expect("run command should delegate to runtime");

        assert_eq!(runtime.calls.len(), 1);
        assert_eq!(runtime.calls[0].listen, "127.0.0.1:0");
        assert_eq!(
            message,
            "宿主端配置有效，监听 127.0.0.1:0，程序 C:\\Program Files\\SomeApp\\app.exe。raw BGRA 画面链路已接入，H.264/WebRTC 编码传输尚未接入。 控制通道已进入持续监听，实际监听 127.0.0.1:49152。"
        );

        fs::remove_file(config_path).expect("temp host config should be removed");
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

    #[derive(Default)]
    struct RecordingServiceManager {
        calls: Vec<&'static str>,
    }

    impl ServiceManager for RecordingServiceManager {
        fn install(&mut self) -> Result<String, String> {
            self.calls.push("install");
            Ok("install dispatched".to_owned())
        }

        fn uninstall(&mut self) -> Result<String, String> {
            self.calls.push("uninstall");
            Ok("uninstall dispatched".to_owned())
        }

        fn start(&mut self) -> Result<String, String> {
            self.calls.push("start");
            Ok("start dispatched".to_owned())
        }

        fn stop(&mut self) -> Result<String, String> {
            self.calls.push("stop");
            Ok("stop dispatched".to_owned())
        }

        fn status(&mut self) -> Result<ServiceStatus, String> {
            self.calls.push("status");
            Ok(ServiceStatus::PendingImplementation)
        }
    }

    #[derive(Default)]
    struct RecordingHostAgentRuntime {
        calls: Vec<HostConfig>,
    }

    impl HostAgentRuntime for RecordingHostAgentRuntime {
        fn run(&mut self, config: &HostConfig) -> Result<std::net::SocketAddr, String> {
            self.calls.push(config.clone());
            Ok("127.0.0.1:49152".parse().expect("test addr should parse"))
        }
    }

    fn temp_host_config_path(name: &str) -> PathBuf {
        let unique = format!(
            "wincast-host-{name}-{}-{}.toml",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time should be after unix epoch")
                .as_nanos()
        );
        std::env::temp_dir().join(unique)
    }
}
