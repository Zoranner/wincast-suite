use std::{fmt, path::PathBuf};

#[cfg(test)]
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[cfg(windows)]
use std::{mem::size_of, os::windows::io::AsRawHandle};

#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    },
};

use wincast_protocol::config::{HostConfig, ProgramConfig};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LaunchRequest {
    pub(crate) program: PathBuf,
    pub(crate) args: Vec<String>,
    pub(crate) work_dir: PathBuf,
}

impl LaunchRequest {
    pub(crate) fn from_config(config: &HostConfig) -> Self {
        Self::from_program_config(&config.program)
    }

    fn from_program_config(config: &ProgramConfig) -> Self {
        Self {
            program: PathBuf::from(&config.path),
            args: config.args.clone(),
            work_dir: PathBuf::from(&config.work_dir),
        }
    }
}

pub(crate) struct StartedProgram {
    pub(crate) process_id: u32,
    #[cfg(windows)]
    child: Option<std::process::Child>,
    #[cfg(windows)]
    job: Option<JobHandle>,
    #[cfg(test)]
    running: Option<Arc<AtomicBool>>,
}

impl StartedProgram {
    #[cfg(test)]
    pub(crate) fn from_process_id(process_id: u32) -> Self {
        Self {
            process_id,
            #[cfg(windows)]
            child: None,
            #[cfg(windows)]
            job: None,
            running: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn from_running_flag(process_id: u32, running: Arc<AtomicBool>) -> Self {
        Self {
            process_id,
            #[cfg(windows)]
            child: None,
            #[cfg(windows)]
            job: None,
            running: Some(running),
        }
    }

    pub(crate) fn is_running(&mut self) -> Result<bool, LaunchError> {
        #[cfg(test)]
        if let Some(running) = &self.running {
            return Ok(running.load(Ordering::SeqCst));
        }

        self.platform_is_running()
    }

    #[cfg(windows)]
    fn platform_is_running(&mut self) -> Result<bool, LaunchError> {
        let Some(child) = self.child.as_mut() else {
            return Ok(true);
        };
        Ok(child
            .try_wait()
            .map_err(|error| LaunchError::from_io("检查宿主端程序退出状态失败", error))?
            .is_none())
    }

    #[cfg(not(windows))]
    fn platform_is_running(&mut self) -> Result<bool, LaunchError> {
        Ok(true)
    }

    #[cfg(windows)]
    fn from_child(child: std::process::Child, job: Option<JobHandle>) -> Self {
        Self {
            process_id: child.id(),
            child: Some(child),
            job,
            #[cfg(test)]
            running: None,
        }
    }
}

#[cfg(windows)]
struct JobHandle(HANDLE);

#[cfg(windows)]
unsafe impl Send for JobHandle {}

#[cfg(windows)]
impl JobHandle {
    fn create_kill_on_close() -> Result<Self, LaunchError> {
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() {
            return Err(LaunchError::from_io(
                "创建宿主端程序作业对象失败",
                std::io::Error::last_os_error(),
            ));
        }

        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let result = unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if result == 0 {
            let error = std::io::Error::last_os_error();
            unsafe {
                CloseHandle(job);
            }
            return Err(LaunchError::from_io("配置宿主端程序作业对象失败", error));
        }

        Ok(Self(job))
    }

    fn assign_child(&self, child: &std::process::Child) -> Result<(), LaunchError> {
        let result = unsafe { AssignProcessToJobObject(self.0, child.as_raw_handle() as HANDLE) };
        if result == 0 {
            return Err(LaunchError::from_io(
                "把宿主端程序加入作业对象失败",
                std::io::Error::last_os_error(),
            ));
        }

        Ok(())
    }
}

#[cfg(windows)]
impl Drop for JobHandle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

impl fmt::Debug for StartedProgram {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StartedProgram")
            .field("process_id", &self.process_id)
            .finish_non_exhaustive()
    }
}

impl PartialEq for StartedProgram {
    fn eq(&self, other: &Self) -> bool {
        self.process_id == other.process_id
    }
}

impl Eq for StartedProgram {}

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

    fn cleanup(&mut self, _started: &mut StartedProgram) -> Result<(), LaunchError> {
        Ok(())
    }
}

pub(crate) struct StdProgramRunner;

impl ProgramRunner for StdProgramRunner {
    fn launch(&mut self, request: &LaunchRequest) -> Result<StartedProgram, LaunchError> {
        launch_std_process(request)
    }

    fn cleanup(&mut self, started: &mut StartedProgram) -> Result<(), LaunchError> {
        cleanup_std_process(started)
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
    let job = JobHandle::create_kill_on_close()?;
    let mut child = std::process::Command::new(&request.program)
        .args(&request.args)
        .current_dir(&request.work_dir)
        .spawn()
        .map_err(|error| LaunchError::from_io("启动宿主端配置程序失败", error))?;
    if let Err(error) = job.assign_child(&child) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }

    Ok(StartedProgram::from_child(child, Some(job)))
}

#[cfg(not(windows))]
fn launch_std_process(_request: &LaunchRequest) -> Result<StartedProgram, LaunchError> {
    Err(LaunchError::unsupported_platform())
}

#[cfg(windows)]
fn cleanup_std_process(started: &mut StartedProgram) -> Result<(), LaunchError> {
    let Some(child) = started.child.as_mut() else {
        return Ok(());
    };

    if let Some(_status) = child
        .try_wait()
        .map_err(|error| LaunchError::from_io("检查宿主端程序退出状态失败", error))?
    {
        let _ = started.job.take();
        return Ok(());
    }

    if let Some(job) = started.job.take() {
        drop(job);
    } else {
        child
            .kill()
            .map_err(|error| LaunchError::from_io("终止宿主端程序失败", error))?;
    }
    child
        .wait()
        .map_err(|error| LaunchError::from_io("等待宿主端程序退出失败", error))?;
    Ok(())
}

#[cfg(not(windows))]
fn cleanup_std_process(_started: &mut StartedProgram) -> Result<(), LaunchError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use wincast_protocol::config::{
        CaptureConfig, HostConfig, ProgramConfig, VideoCodec, VideoConfig,
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
            Ok(StartedProgram::from_process_id(4242))
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
            program: ProgramConfig {
                path: "C:\\Tools\\Demo App\\demo.exe".to_owned(),
                args: vec!["--profile".to_owned(), "default".to_owned()],
                work_dir: "C:\\Tools\\Demo App".to_owned(),
                startup_delay_ms: 3000,
            },
            video: VideoConfig {
                width: 1280,
                height: 720,
                fps: 30,
                codec: VideoCodec::H264,
                bitrate_kbps: 4000,
                max_bitrate_kbps: 6000,
            },
            capture: CaptureConfig {
                first_frame_timeout_ms: 5000,
            },
        }
    }
}
