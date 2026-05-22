use std::{
    env, fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use wincast_protocol::config::HostConfig;

mod agent;
mod agent_runtime;
mod program;
pub mod session_events;
pub mod session_state;

use agent_runtime::{HostAgentRuntime, StdHostAgentRuntime};

pub fn main() -> ExitCode {
    let mut runtime = StdHostAgentRuntime;
    match run_default_host(&mut runtime) {
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

fn run_default_host(runtime: &mut impl HostAgentRuntime) -> Result<String, String> {
    run_host_with_runtime(&default_host_config_path(), runtime)
}

fn default_host_config_path() -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(appdata) = env::var_os("APPDATA") {
            return PathBuf::from(appdata).join("WinCast").join("host.toml");
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
        .join("host.toml")
}

fn run_host_with_runtime(
    path: &Path,
    runtime: &mut impl HostAgentRuntime,
) -> Result<String, String> {
    let config = load_config(path)?;
    let startup_message = runtime_status_message(&config);
    let local_addr = runtime.run(&config)?;
    Ok(format!(
        "{startup_message} 控制通道已进入持续监听，实际监听 {local_addr}。"
    ))
}

fn runtime_status_message(config: &HostConfig) -> String {
    format!(
        "宿主端已启动，监听 {}，程序 {}。{}",
        config.listen,
        config.program.path,
        runtime_status_detail()
    )
}

fn runtime_status_detail() -> &'static str {
    "收到客户端会话请求后会直接启动配置程序，并通过 H.264 编码链路传输画面。"
}

fn load_config(path: &Path) -> Result<HostConfig, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("读取宿主端配置失败（{}）: {error}", path.display()))?;
    HostConfig::from_toml_str(&source).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executable_startup_uses_default_config_path_without_subcommands() {
        let config_path = temp_host_config_path("default-startup");
        write_host_config(&config_path, "127.0.0.1:0");
        let mut runtime = RecordingHostAgentRuntime::default();

        let message = run_host_with_runtime(&config_path, &mut runtime).expect("host should start");

        assert_eq!(runtime.calls.len(), 1);
        assert_eq!(runtime.calls[0].listen, "127.0.0.1:0");
        assert_eq!(runtime.calls[0].program.startup_delay_ms, 3000);
        assert_eq!(runtime.calls[0].capture.first_frame_timeout_ms, 5000);
        assert_eq!(
            message,
            "宿主端已启动，监听 127.0.0.1:0，程序 C:\\Program Files\\SomeApp\\app.exe。收到客户端会话请求后会直接启动配置程序，并通过 H.264 编码链路传输画面。 控制通道已进入持续监听，实际监听 127.0.0.1:49152。"
        );

        fs::remove_file(config_path).expect("temp host config should be removed");
    }

    #[test]
    fn startup_reports_config_path_when_read_fails() {
        let config_path = temp_host_config_path("missing-config");

        let error = run_host_with_runtime(&config_path, &mut RecordingHostAgentRuntime::default())
            .expect_err("missing config should be reported");

        assert!(error.contains("读取宿主端配置失败"));
        assert!(error.contains(&config_path.display().to_string()));
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

    fn write_host_config(path: &Path, listen: &str) {
        fs::write(
            path,
            format!(
                r#"
listen = "{listen}"

[program]
path = 'C:\Program Files\SomeApp\app.exe'
args = ["--profile", "demo"]
work_dir = 'C:\Program Files\SomeApp'
startup_delay_ms = 3000

[video]
width = 1280
height = 720
fps = 30
codec = "h264"
bitrate_kbps = 4000
max_bitrate_kbps = 6000

[capture]
first_frame_timeout_ms = 5000
"#
            ),
        )
        .expect("host config should be written");
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
        home.join(".config").join("wincast").join("host.toml")
    }
}
