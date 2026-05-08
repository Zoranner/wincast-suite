use std::{fmt, path::PathBuf};

use wincast_protocol::config::HostConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LaunchRequest {
    pub(crate) program: PathBuf,
    pub(crate) args: Vec<String>,
    pub(crate) work_dir: PathBuf,
}

impl LaunchRequest {
    pub(crate) fn from_config(config: &HostConfig) -> Self {
        Self {
            program: PathBuf::from(&config.program),
            args: config.args.clone(),
            work_dir: PathBuf::from(&config.work_dir),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StartedProgram {
    pub(crate) process_id: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LaunchError {
    message: String,
}

impl LaunchError {
    #[cfg(any(not(windows), test))]
    pub(crate) fn unsupported_platform() -> Self {
        Self {
            message: "当前平台不支持宿主端程序启动：仅 Windows 支持启动配置程序".to_owned(),
        }
    }

    #[cfg(windows)]
    fn from_io(action: &'static str, error: std::io::Error) -> Self {
        Self {
            message: format!("{action}: {error}"),
        }
    }
}

impl fmt::Display for LaunchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for LaunchError {}

pub(crate) trait ProgramRunner {
    fn launch(&mut self, request: &LaunchRequest) -> Result<StartedProgram, LaunchError>;
}

pub(crate) struct StdProgramRunner;

impl ProgramRunner for StdProgramRunner {
    fn launch(&mut self, request: &LaunchRequest) -> Result<StartedProgram, LaunchError> {
        launch_std_process(request)
    }
}

pub(crate) fn launch_with_runner(
    config: &HostConfig,
    runner: &mut impl ProgramRunner,
) -> Result<StartedProgram, LaunchError> {
    let request = LaunchRequest::from_config(config);
    runner.launch(&request)
}

#[cfg(windows)]
fn launch_std_process(request: &LaunchRequest) -> Result<StartedProgram, LaunchError> {
    let child = std::process::Command::new(&request.program)
        .args(&request.args)
        .current_dir(&request.work_dir)
        .spawn()
        .map_err(|error| LaunchError::from_io("启动宿主端配置程序失败", error))?;

    Ok(StartedProgram {
        process_id: child.id(),
    })
}

#[cfg(not(windows))]
fn launch_std_process(_request: &LaunchRequest) -> Result<StartedProgram, LaunchError> {
    Err(LaunchError::unsupported_platform())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use wincast_protocol::config::{
        CaptureConfig, CaptureMode, HostConfig, VideoCodec, VideoConfig,
    };

    #[test]
    fn builds_launch_request_from_host_config() {
        let config = host_config();
        let request = LaunchRequest::from_config(&config);

        assert_eq!(
            request.program,
            PathBuf::from("C:\\Tools\\Demo App\\demo.exe")
        );
        assert_eq!(request.args, ["--profile", "default"]);
        assert_eq!(request.work_dir, PathBuf::from("C:\\Tools\\Demo App"));
    }

    #[test]
    fn injectable_runner_receives_configured_launch_request() {
        let config = host_config();
        let mut runner = RecordingRunner::default();

        let started = launch_with_runner(&config, &mut runner).expect("program should start");

        assert_eq!(started.process_id, 4242);
        assert_eq!(
            runner.request.expect("request should be recorded"),
            LaunchRequest {
                program: PathBuf::from("C:\\Tools\\Demo App\\demo.exe"),
                args: vec!["--profile".to_owned(), "default".to_owned()],
                work_dir: PathBuf::from("C:\\Tools\\Demo App"),
            }
        );
    }

    #[test]
    fn non_windows_runner_returns_clear_chinese_error() {
        let config = host_config();
        let mut runner = UnsupportedPlatformRunner;

        let error = launch_with_runner(&config, &mut runner).expect_err("platform should fail");

        assert_eq!(
            error.to_string(),
            "当前平台不支持宿主端程序启动：仅 Windows 支持启动配置程序"
        );
    }

    #[derive(Default)]
    struct RecordingRunner {
        request: Option<LaunchRequest>,
    }

    impl ProgramRunner for RecordingRunner {
        fn launch(&mut self, request: &LaunchRequest) -> Result<StartedProgram, LaunchError> {
            self.request = Some(request.clone());
            Ok(StartedProgram { process_id: 4242 })
        }
    }

    struct UnsupportedPlatformRunner;

    impl ProgramRunner for UnsupportedPlatformRunner {
        fn launch(&mut self, _request: &LaunchRequest) -> Result<StartedProgram, LaunchError> {
            Err(LaunchError::unsupported_platform())
        }
    }

    fn host_config() -> HostConfig {
        HostConfig {
            listen: "127.0.0.1:7856".to_owned(),
            program: "C:\\Tools\\Demo App\\demo.exe".to_owned(),
            args: vec!["--profile".to_owned(), "default".to_owned()],
            work_dir: "C:\\Tools\\Demo App".to_owned(),
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
