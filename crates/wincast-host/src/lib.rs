use std::{
    env, fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::{Parser, Subcommand};
use wincast_protocol::config::{CaptureMode, HostConfig, VideoCodec};

mod agent;
mod agent_runtime;
mod program;
mod service;
pub mod service_agent;
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
    #[arg(short, long, global = true)]
    config: Option<PathBuf>,
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
    /// 由 Windows SCM 启动的 Service 运行入口
    #[command(hide = true)]
    Run,
}

pub fn main() -> ExitCode {
    let args = Args::parse();
    let config_path = args.config_path();
    let command = args.command.unwrap_or(Command::Run);
    run(command, &config_path)
}

impl Args {
    fn config_path(&self) -> PathBuf {
        self.config.clone().unwrap_or_else(default_host_config_path)
    }
}

fn default_host_config_path() -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(appdata) = env::var_os("APPDATA") {
            return PathBuf::from(appdata)
                .join("WinCast")
                .join("wincast-host.toml");
        }
    }

    xdg_host_config_path(env::var_os("XDG_CONFIG_HOME"), env::var_os("HOME"))
}

fn xdg_host_config_path(
    xdg_config_home: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> PathBuf {
    xdg_config_home
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| {
            home.map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".config")
        })
        .join("wincast")
        .join("wincast-host.toml")
}

fn run(command: Command, config_path: &Path) -> ExitCode {
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

fn execute_command(command: Command, config_path: &Path) -> Result<String, String> {
    let mut service_manager = DefaultServiceManager::default();
    execute_command_with_service_manager(command, config_path, &mut service_manager)
}

fn execute_command_with_service_manager(
    command: Command,
    config_path: &Path,
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
        ServiceCommand::Run => service::run_service_dispatcher(),
    }
}

fn validate_config(path: &Path) -> Result<String, String> {
    let config = load_config(path)?;
    Ok(format!(
        "宿主端配置有效，smoke-test 摘要：监听 {}，capture mode {}，window title {}，codec {}。",
        config.listen,
        capture_mode_label(config.capture.mode),
        config.capture.window_title_contains,
        video_codec_label(config.video.codec)
    ))
}

fn run_host(path: &Path) -> Result<String, String> {
    let mut runtime = StdHostAgentRuntime;
    run_host_with_runtime(path, &mut runtime)
}

fn run_host_with_runtime(
    path: &Path,
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
    "H.264 编码传输已作为当前正式链路口径，运行期编码器和解码器仍待接入。"
}

fn load_config(path: &Path) -> Result<HostConfig, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("读取宿主端配置失败（{}）: {error}", path.display()))?;
    HostConfig::from_toml_str(&source).map_err(|error| error.to_string())
}

fn capture_mode_label(mode: CaptureMode) -> &'static str {
    match mode {
        CaptureMode::Auto => "auto",
        CaptureMode::Window => "window",
        CaptureMode::Display => "display",
    }
}

fn video_codec_label(codec: VideoCodec) -> &'static str {
    match codec {
        VideoCodec::RawBgra => "raw_bgra",
        VideoCodec::H264 => "h264",
    }
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

        assert_eq!(args.config_path(), PathBuf::from("custom-host.toml"));
        match args.command {
            Some(Command::Validate) => {}
            _ => panic!("validate command should parse"),
        }
    }

    #[test]
    fn parses_default_run_with_config_path() {
        let args = Args::try_parse_from(["wincast-host"]).expect("args should parse");

        assert_eq!(args.config_path(), expected_default_config_path());
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
            ("run", ServiceCommand::Run),
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

    #[cfg(not(windows))]
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
            assert!(message.contains("当前稳定版仅支持前台 run 模式，Service 管理未启用"));
            if !matches!(command, ServiceCommand::Status) {
                assert!(message.contains("未执行真实系统服务操作"));
            }
            if matches!(command, ServiceCommand::Status) {
                assert!(message.contains("未执行真实系统服务状态查询"));
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
                assert!(message.contains("当前稳定版仅支持前台 run 模式，Service 管理未启用"));
            } else {
                assert!(message.contains(expected_call));
            }
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn pending_service_status_reports_clear_pending_state() {
        let mut manager = DefaultServiceManager::default();
        let message = execute_command_with_service_manager(
            Command::Service(ServiceCommand::Status),
            &PathBuf::from("unused.toml"),
            &mut manager,
        )
        .expect("status should return a user-facing message");

        assert_eq!(
            message,
            "Windows Service 状态：当前稳定版仅支持前台 run 模式，Service 管理未启用；未安装，未执行真实系统服务状态查询。"
        );
        assert!(message.contains("未安装"));
        assert!(!message.contains("安装成功"));
        assert!(!message.contains("已启动"));
    }

    #[test]
    fn runtime_message_reports_h264_transport_pending() {
        let config = host_config("0.0.0.0:7856".to_owned());

        let message = runtime_not_implemented_message(&config);

        assert!(!message.contains("运行时链路未实现"));
        assert!(message.contains("H.264 编码传输已作为当前正式链路口径"));
        assert!(message.contains("运行期编码器和解码器仍待接入"));
    }

    #[test]
    fn validate_command_accepts_display_capture_mode() {
        let config_path = temp_host_config_path("validate-accepts-display");
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
max_bitrate_kbps = 6000

[capture]
mode = "display"
window_title_contains = ""
startup_timeout_ms = 15000
"#,
        )
        .expect("host config should be written");

        let message = validate_config(&config_path).expect("display capture should validate");

        assert!(message.contains("smoke-test"));
        assert!(message.contains("capture mode display"));

        fs::remove_file(config_path).expect("temp host config should be removed");
    }

    #[test]
    fn validate_command_reports_smoke_test_summary() {
        let config_path = temp_host_config_path("validate-smoke-summary");
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
max_bitrate_kbps = 6000

[capture]
mode = "window"
window_title_contains = "SomeApp"
startup_timeout_ms = 15000
"#,
        )
        .expect("host config should be written");

        let message = validate_config(&config_path).expect("window capture should validate");

        assert!(message.contains("smoke-test"));
        assert!(message.contains("监听 127.0.0.1:0"));
        assert!(message.contains("capture mode window"));
        assert!(message.contains("window title SomeApp"));
        assert!(message.contains("codec h264"));
        assert!(!message.contains("args"));
        assert!(!message.contains("work_dir"));

        fs::remove_file(config_path).expect("temp host config should be removed");
    }

    #[test]
    fn validate_command_reports_config_path_when_read_fails() {
        let config_path = temp_host_config_path("validate-missing-config");

        let error = validate_config(&config_path).expect_err("missing config should be reported");

        assert!(error.contains("读取宿主端配置失败"));
        assert!(error.contains(&config_path.display().to_string()));
    }

    #[test]
    fn run_command_loads_window_config_and_delegates_host_agent_runtime() {
        let config_path = temp_host_config_path("run-delegates-window-runtime");
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
max_bitrate_kbps = 6000

[capture]
mode = "window"
window_title_contains = "SomeApp"
startup_timeout_ms = 15000
"#,
        )
        .expect("host config should be written");
        let mut runtime = RecordingHostAgentRuntime::default();

        let message = run_host_with_runtime(&config_path, &mut runtime)
            .expect("window capture should delegate to runtime");

        assert_eq!(runtime.calls.len(), 1);
        assert_eq!(runtime.calls[0].listen, "127.0.0.1:0");
        assert_eq!(runtime.calls[0].capture.mode, CaptureMode::Window);
        assert_eq!(
            message,
            "宿主端配置有效，监听 127.0.0.1:0，程序 C:\\Program Files\\SomeApp\\app.exe。H.264 编码传输已作为当前正式链路口径，运行期编码器和解码器仍待接入。 控制通道已进入持续监听，实际监听 127.0.0.1:49152。"
        );

        fs::remove_file(config_path).expect("temp host config should be removed");
    }

    #[test]
    fn run_command_loads_display_config_and_delegates_host_agent_runtime() {
        let config_path = temp_host_config_path("run-delegates-display-runtime");
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
max_bitrate_kbps = 6000

[capture]
mode = "display"
window_title_contains = ""
startup_timeout_ms = 15000
"#,
        )
        .expect("host config should be written");
        let mut runtime = RecordingHostAgentRuntime::default();

        let message = run_host_with_runtime(&config_path, &mut runtime)
            .expect("display capture should delegate to runtime");

        assert_eq!(runtime.calls.len(), 1);
        assert_eq!(runtime.calls[0].capture.mode, CaptureMode::Display);
        assert_eq!(
            message,
            "宿主端配置有效，监听 127.0.0.1:0，程序 C:\\Program Files\\SomeApp\\app.exe。H.264 编码传输已作为当前正式链路口径，运行期编码器和解码器仍待接入。 控制通道已进入持续监听，实际监听 127.0.0.1:49152。"
        );

        fs::remove_file(config_path).expect("temp host config should be removed");
    }

    #[test]
    fn protocol_config_rejects_desktop_capture_mode() {
        let config = HostConfig::from_toml_str(
            r#"
listen = "127.0.0.1:0"
program = "C:\\Program Files\\SomeApp\\app.exe"
args = []
work_dir = "C:\\Program Files\\SomeApp"

[video]
width = 1280
height = 720
fps = 30
codec = "h264"
bitrate_kbps = 4000
max_bitrate_kbps = 6000

[capture]
mode = "desktop"
window_title_contains = ""
startup_timeout_ms = 15000
"#,
        )
        .expect_err("protocol config should reject old desktop mode");

        assert!(matches!(
            config,
            wincast_protocol::config::ConfigError::InvalidToml(_)
        ));
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
                max_bitrate_kbps: 6000,
            },
            capture: CaptureConfig {
                mode: CaptureMode::Display,
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

    fn expected_default_config_path() -> PathBuf {
        #[cfg(windows)]
        {
            PathBuf::from(std::env::var_os("APPDATA").expect("APPDATA should be set"))
                .join("WinCast")
                .join("wincast-host.toml")
        }

        #[cfg(not(windows))]
        {
            if let Some(config_home) = std::env::var_os("XDG_CONFIG_HOME") {
                return PathBuf::from(config_home)
                    .join("wincast")
                    .join("wincast-host.toml");
            }

            PathBuf::from(std::env::var_os("HOME").expect("HOME should be set"))
                .join(".config")
                .join("wincast")
                .join("wincast-host.toml")
        }
    }

    #[test]
    fn xdg_host_config_path_falls_back_when_xdg_config_home_is_empty() {
        let home = absolute_test_home();
        let path = xdg_host_config_path(Some("".into()), Some(home.as_os_str().into()));

        assert_eq!(path, expected_host_config_under_home(&home));
    }

    #[test]
    fn xdg_host_config_path_ignores_relative_xdg_config_home() {
        let home = absolute_test_home();
        let path = xdg_host_config_path(
            Some("relative-config".into()),
            Some(home.as_os_str().into()),
        );

        assert_eq!(path, expected_host_config_under_home(&home));
    }

    fn absolute_test_home() -> PathBuf {
        #[cfg(windows)]
        {
            PathBuf::from(r"C:\Users\tester")
        }

        #[cfg(not(windows))]
        {
            PathBuf::from("/home/tester")
        }
    }

    fn expected_host_config_under_home(home: &std::path::Path) -> PathBuf {
        home.join(".config")
            .join("wincast")
            .join("wincast-host.toml")
    }
}
