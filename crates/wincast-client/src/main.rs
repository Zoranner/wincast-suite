use std::{fs, path::PathBuf, process::ExitCode};

use clap::{Parser, Subcommand};
use wincast_protocol::config::ClientConfig;

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
    Ok(runtime_not_implemented_message(&config))
}

fn runtime_not_implemented_message(config: &ClientConfig) -> String {
    format!(
        "客户端配置有效，目标 {}，支持 Linux x86_64 与 Linux aarch64/ARM64。运行时链路未实现：尚未建立宿主端连接、信令、视频解码渲染和输入事件发送。",
        config.endpoint()
    )
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

        let message = runtime_not_implemented_message(&config);

        assert!(message.contains("运行时链路未实现"));
        assert!(message.contains("尚未建立宿主端连接"));
        assert!(message.contains("Linux aarch64/ARM64"));
    }
}
