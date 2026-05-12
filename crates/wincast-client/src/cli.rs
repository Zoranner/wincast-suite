use std::{path::PathBuf, process::ExitCode, time::Duration};

use clap::{Parser, Subcommand};

use crate::runtime::{RetryOptions, run_client};

const SUPPORTED_CLIENT_TARGETS: &[&str] =
    &["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"];

#[derive(Debug, Parser)]
#[command(author, version, about = "WinCast Linux 客户端")]
pub(crate) struct Args {
    #[arg(short, long, global = true, default_value = "wincast-client.toml")]
    pub(crate) config: PathBuf,
    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// 校验客户端配置文件
    Validate,
    /// 校验配置并进入客户端运行入口
    Run {
        /// 宿主端可恢复状态或连接失败后的重试次数
        #[arg(long, default_value_t = 0)]
        retries: u32,
        /// 每次重试前等待的毫秒数
        #[arg(long, default_value_t = 1_000)]
        retry_delay_ms: u64,
    },
    /// 输出客户端支持的 Linux 目标平台
    Targets,
}

pub(crate) fn main_entry() -> ExitCode {
    let args = Args::parse();
    let command = args.command.unwrap_or(Command::Run {
        retries: 0,
        retry_delay_ms: 1_000,
    });
    run(command, &args.config)
}

pub(crate) fn run(command: Command, config_path: &PathBuf) -> ExitCode {
    let result = match command {
        Command::Validate => validate_config(config_path),
        Command::Run {
            retries,
            retry_delay_ms,
        } => run_client(
            config_path,
            RetryOptions {
                retries,
                retry_delay: Duration::from_millis(retry_delay_ms),
            },
        ),
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
    let config = crate::runtime::load_config(path)?;
    Ok(format!(
        "客户端配置有效，smoke-test 摘要：host:port {}，支持目标 {}。",
        config.endpoint(),
        SUPPORTED_CLIENT_TARGETS.join(", ")
    ))
}

fn supported_targets_message() -> String {
    format!(
        "客户端支持平台：Linux x86_64 ({})；Linux aarch64/ARM64 ({})",
        SUPPORTED_CLIENT_TARGETS[0], SUPPORTED_CLIENT_TARGETS[1]
    )
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use clap::Parser;

    use super::{Args, Command, supported_targets_message, validate_config};

    #[test]
    fn parses_run_command_with_config_path() {
        let args =
            Args::try_parse_from(["wincast-client", "--config", "custom-client.toml", "run"])
                .expect("args should parse");

        assert_eq!(args.config, PathBuf::from("custom-client.toml"));
        match args.command {
            Some(Command::Run {
                retries,
                retry_delay_ms,
            }) => {
                assert_eq!(retries, 0);
                assert_eq!(retry_delay_ms, 1_000);
            }
            _ => panic!("run command should parse"),
        }
    }

    #[test]
    fn parses_run_command_retry_options() {
        let args = Args::try_parse_from([
            "wincast-client",
            "--config",
            "custom-client.toml",
            "run",
            "--retries",
            "3",
            "--retry-delay-ms",
            "250",
        ])
        .expect("args should parse");

        match args.command {
            Some(Command::Run {
                retries,
                retry_delay_ms,
            }) => {
                assert_eq!(retries, 3);
                assert_eq!(retry_delay_ms, 250);
            }
            _ => panic!("run command with retry options should parse"),
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
    fn validate_command_reports_smoke_test_summary() {
        let config_path = temp_client_config_path("validate-smoke-summary");
        fs::write(
            &config_path,
            r#"
host = "192.168.1.20"
port = 7856
"#,
        )
        .expect("client config should be written");

        let message = validate_config(&config_path).expect("client config should validate");

        assert!(message.contains("smoke-test"));
        assert!(message.contains("host:port 192.168.1.20:7856"));
        assert!(message.contains("支持目标"));
        assert!(message.contains("x86_64-unknown-linux-gnu"));
        assert!(message.contains("aarch64-unknown-linux-gnu"));

        fs::remove_file(config_path).expect("temp client config should be removed");
    }

    fn temp_client_config_path(name: &str) -> PathBuf {
        let unique = format!(
            "wincast-client-{name}-{}-{}.toml",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time should be after unix epoch")
                .as_nanos()
        );
        std::env::temp_dir().join(unique)
    }
}
