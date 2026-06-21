use std::{
    net::{SocketAddr, TcpListener},
    thread,
    time::Duration,
};

use wincast_protocol::config::{HostBackendMode, HostConfig};

use crate::{
    agent,
    program::{ProgramRunner, StdProgramRunner},
};

pub(crate) trait HostAgentRuntime {
    fn run(&mut self, config: &HostConfig) -> Result<SocketAddr, String>;
}

#[derive(Debug, Default)]
pub(crate) struct StdHostAgentRuntime;

impl HostAgentRuntime for StdHostAgentRuntime {
    fn run(&mut self, config: &HostConfig) -> Result<SocketAddr, String> {
        match config.mode {
            HostBackendMode::DesktopDxgi => run_desktop_dxgi(config),
            HostBackendMode::UnityEmbedded => {
                let mut runner = StdProgramRunner;
                run_unity_embedded_with_runner(config, &mut runner, Duration::from_secs(1))
            }
        }
    }
}

fn run_desktop_dxgi(config: &HostConfig) -> Result<SocketAddr, String> {
    let listener = TcpListener::bind(&config.listen)
        .map_err(|error| format!("宿主端 TCP 监听失败: {error}"))?;
    let mut runner = StdProgramRunner;
    agent::run_control_listener(listener, config, &mut runner)
}

fn run_unity_embedded_with_runner(
    config: &HostConfig,
    runner: &mut impl ProgramRunner,
    startup_delay: Duration,
) -> Result<SocketAddr, String> {
    let unity = config
        .unity
        .as_ref()
        .ok_or_else(|| "unity_embedded 模式缺少 [unity] 配置".to_owned())?;
    let port = unity.port;
    let request = crate::program::LaunchRequest::from_config(config);
    let mut started = runner
        .launch(&request)
        .map_err(|error| format!("启动 Unity 实例失败: {error}"))?;
    if !startup_delay.is_zero() {
        thread::sleep(startup_delay);
        if !started
            .is_running()
            .map_err(|error| format!("检查 Unity 实例状态失败: {error}"))?
        {
            runner
                .cleanup(&mut started)
                .map_err(|error| format!("清理 Unity 实例失败: {error}"))?;
            return Err("Unity 实例启动后已退出".to_owned());
        }
    }

    let endpoint = SocketAddr::from(([127, 0, 0, 1], port));
    println!("Unity 内嵌实例已启动，远控端口 {endpoint}。");
    supervise_started_unity_instance(started);
    Err("Unity 实例已退出，远控端口已释放。".to_owned())
}

fn supervise_started_unity_instance(mut started: crate::program::StartedProgram) {
    loop {
        match started.is_running() {
            Ok(true) => thread::sleep(Duration::from_millis(500)),
            Ok(false) => break,
            Err(error) => {
                eprintln!("检查 Unity 实例状态失败: {error}");
                break;
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread,
        time::{Duration, Instant},
    };

    use wincast_protocol::config::{HostBackendMode, HostConfig, UnityConfig};

    use crate::program::{LaunchError, LaunchRequest, ProgramRunner, StartedProgram};

    use super::run_unity_embedded_with_runner;

    #[test]
    fn unity_embedded_runtime_launches_unity_with_port_only_and_reports_instance_exit() {
        let mut config = host_config("127.0.0.1:7856".to_owned());
        config.mode = HostBackendMode::UnityEmbedded;
        config.unity = Some(UnityConfig {
            executable: "C:\\UnityApps\\Demo\\Demo.exe".to_owned(),
            work_dir: "C:\\UnityApps\\Demo".to_owned(),
            port: 7900,
        });
        let running = Arc::new(AtomicBool::new(true));
        let mut runner = RecordingUnityRunner::new(Arc::clone(&running));
        let release = thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            running.store(false, Ordering::SeqCst);
        });

        let started = Instant::now();
        let error = run_unity_embedded_with_runner(&config, &mut runner, Duration::ZERO)
            .expect_err("unity embedded runtime should report when instance exits");
        release.join().expect("release thread should finish");

        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(error, "Unity 实例已退出，远控端口已释放。");
        let request = runner.request.expect("launch request should be recorded");
        assert_eq!(
            request.program,
            std::path::PathBuf::from("C:\\UnityApps\\Demo\\Demo.exe")
        );
        assert_eq!(
            request.work_dir,
            std::path::PathBuf::from("C:\\UnityApps\\Demo")
        );
        assert_eq!(request.args, ["--wincast-port", "7900"]);
    }

    fn host_config(listen: String) -> HostConfig {
        HostConfig {
            listen,
            mode: HostBackendMode::DesktopDxgi,
            program: wincast_protocol::config::ProgramConfig {
                path: "C:\\Tools\\Demo App\\demo.exe".to_owned(),
                args: vec!["--profile".to_owned(), "default".to_owned()],
                work_dir: "C:\\Tools\\Demo App".to_owned(),
                startup_delay_ms: 3000,
                turn_off_monitor_after_launch:
                    wincast_protocol::config::MonitorPowerAfterLaunch::Disabled,
            },
            unity: None,
            video: wincast_protocol::config::VideoConfig {
                width: 1280,
                height: 720,
                fps: 30,
                codec: wincast_protocol::config::VideoCodec::H264,
                bitrate_kbps: 4000,
                max_bitrate_kbps: 6000,
            },
            capture: wincast_protocol::config::CaptureConfig {
                first_frame_timeout_ms: 5000,
            },
        }
    }

    struct RecordingUnityRunner {
        request: Option<LaunchRequest>,
        running: Arc<AtomicBool>,
    }

    impl RecordingUnityRunner {
        fn new(running: Arc<AtomicBool>) -> Self {
            Self {
                request: None,
                running,
            }
        }
    }

    impl ProgramRunner for RecordingUnityRunner {
        fn launch(&mut self, request: &LaunchRequest) -> Result<StartedProgram, LaunchError> {
            self.request = Some(request.clone());
            Ok(StartedProgram::from_running_flag(
                7001,
                Arc::clone(&self.running),
            ))
        }

        fn cleanup(&mut self, started: &mut StartedProgram) -> Result<(), LaunchError> {
            self.running.store(false, Ordering::SeqCst);
            assert_eq!(started.process_id, 7001);
            Ok(())
        }
    }
}
