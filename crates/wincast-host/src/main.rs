use std::{fs, path::PathBuf, process::ExitCode};

use clap::{Parser, Subcommand};
use wincast_protocol::config::HostConfig;

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
    Ok(runtime_not_implemented_message(&config))
}

fn runtime_not_implemented_message(config: &HostConfig) -> String {
    format!(
        "宿主端配置有效，监听 {}，程序 {}。运行时链路未实现：尚未启动 TCP 监听、程序生命周期、画面捕获、编码传输和输入注入。",
        config.listen, config.program
    )
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
    fn run_message_does_not_claim_runtime_chain_is_ready() {
        let config = HostConfig {
            listen: "0.0.0.0:7856".to_owned(),
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
        };

        let message = runtime_not_implemented_message(&config);

        assert!(message.contains("运行时链路未实现"));
        assert!(message.contains("尚未启动 TCP 监听"));
    }
}
