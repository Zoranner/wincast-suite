use std::{
    fs,
    net::{SocketAddr, TcpListener, TcpStream},
    path::PathBuf,
    process::ExitCode,
    thread,
    time::{Duration, Instant},
};

use clap::{Parser, Subcommand};
mod program;

use program::{ProgramRunner, StartedProgram, StdProgramRunner, launch_with_runner};
use wincast_capture::{
    CaptureError, CaptureSession, CaptureTarget, CapturedBgraFrame, wait_next_capture_result_with,
};
use wincast_protocol::{
    config::{CaptureMode, HostConfig},
    frame::{read_message, write_message},
    handshake::accept_client_hello,
    message::{ControlMessage, ErrorCode},
};

mod window;
use window::{WindowCandidate, WindowLookupError, find_main_window};

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
    let local_addr = run_control_listener_once(listener, &config)?;
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
    "运行时链路未实现：尚未实现编码传输和输入注入。"
}

fn run_control_listener_once(
    listener: TcpListener,
    config: &HostConfig,
) -> Result<SocketAddr, String> {
    let mut runner = StdProgramRunner;
    let mut locator = WindowsWindowLocator;
    let mut capture = StdCaptureStarter;
    run_control_listener_once_with_runtime(
        listener,
        config,
        &mut runner,
        &mut locator,
        &mut capture,
    )
}

fn run_control_listener_once_with_runtime(
    listener: TcpListener,
    config: &HostConfig,
    runner: &mut impl ProgramRunner,
    locator: &mut impl WindowLocator,
    capture: &mut impl CaptureStarter,
) -> Result<SocketAddr, String> {
    let local_addr = listener
        .local_addr()
        .map_err(|error| format!("读取宿主端监听地址失败: {error}"))?;
    let (mut stream, _peer_addr) = listener
        .accept()
        .map_err(|error| format!("接受客户端连接失败: {error}"))?;
    handle_control_client(&mut stream, config, runner, locator, capture)?;
    Ok(local_addr)
}

fn handle_control_client(
    stream: &mut TcpStream,
    config: &HostConfig,
    runner: &mut impl ProgramRunner,
    locator: &mut impl WindowLocator,
    capture: &mut impl CaptureStarter,
) -> Result<(), String> {
    let mut writer = stream
        .try_clone()
        .map_err(|error| format!("克隆控制连接写入端失败: {error}"))?;
    accept_client_hello(stream, &mut writer).map_err(|error| format!("控制握手失败: {error}"))?;

    match read_message(stream).map_err(|error| format!("读取控制消息失败: {error}"))? {
        ControlMessage::StartSession => {
            let started = launch_with_runner(config, runner).map_err(|error| {
                let message = format!("启动宿主端程序失败: {error}");
                let _ = write_control_error(
                    &mut writer,
                    ErrorCode::ProgramLaunchFailed,
                    message.clone(),
                );
                message
            })?;
            let window = locate_started_window(config, &started, locator).map_err(|error| {
                let message = format!("定位宿主端程序窗口失败: {error}");
                let _ =
                    write_control_error(&mut writer, ErrorCode::WindowNotFound, message.clone());
                message
            })?;
            start_capture_session(config, &window, capture).map_err(|error| {
                let message = format!("初始化画面捕获失败: {error}");
                let _ = write_control_error(&mut writer, ErrorCode::CaptureFailed, message.clone());
                message
            })?;
            write_runtime_not_implemented(&mut writer)?;
            Ok(())
        }
        message => {
            write_control_error(
                &mut writer,
                ErrorCode::TransportFailed,
                format!("控制消息顺序无效，期望 StartSession，实际收到 {message:?}"),
            )?;
            Err("控制消息顺序无效，期望 StartSession".to_owned())
        }
    }
}

trait WindowLocator {
    fn find_main_window(
        &mut self,
        process_id: u32,
        title_contains: Option<&str>,
    ) -> Result<WindowCandidate, WindowLookupError>;
}

struct WindowsWindowLocator;

impl WindowLocator for WindowsWindowLocator {
    fn find_main_window(
        &mut self,
        process_id: u32,
        title_contains: Option<&str>,
    ) -> Result<WindowCandidate, WindowLookupError> {
        find_main_window(process_id, title_contains)
    }
}

trait CaptureStarter {
    fn start_capture(
        &mut self,
        target: CaptureTarget,
    ) -> Result<Box<dyn CaptureRuntime>, CaptureError>;
}

trait CaptureRuntime {
    fn try_next_bgra_frame(&mut self) -> Result<Option<CapturedBgraFrame>, CaptureError>;
}

struct StdCaptureStarter;

impl CaptureStarter for StdCaptureStarter {
    fn start_capture(
        &mut self,
        target: CaptureTarget,
    ) -> Result<Box<dyn CaptureRuntime>, CaptureError> {
        Ok(Box::new(CaptureSession::start(target)?))
    }
}

impl CaptureRuntime for CaptureSession {
    fn try_next_bgra_frame(&mut self) -> Result<Option<CapturedBgraFrame>, CaptureError> {
        self.try_next_bgra_frame()
    }
}

fn locate_started_window(
    config: &HostConfig,
    started: &StartedProgram,
    locator: &mut impl WindowLocator,
) -> Result<WindowCandidate, WindowLookupError> {
    let deadline = Instant::now() + Duration::from_millis(config.capture.startup_timeout_ms);
    let title_contains = Some(config.capture.window_title_contains.as_str());

    loop {
        let last_error = match locator.find_main_window(started.process_id, title_contains) {
            Ok(window) => return Ok(window),
            Err(error) => error,
        };

        if Instant::now() >= deadline {
            return Err(last_error);
        }

        thread::sleep(Duration::from_millis(100));
    }
}

fn start_capture_session(
    config: &HostConfig,
    window: &WindowCandidate,
    capture: &mut impl CaptureStarter,
) -> Result<(), CaptureError> {
    let mut session = capture.start_capture(capture_target(config, window))?;
    let _ = wait_next_capture_result_with(
        Duration::from_millis(config.capture.startup_timeout_ms),
        || session.try_next_bgra_frame(),
    )?;
    Ok(())
}

fn capture_target(config: &HostConfig, window: &WindowCandidate) -> CaptureTarget {
    match config.capture.mode {
        CaptureMode::Desktop => CaptureTarget::Desktop,
        CaptureMode::Window => CaptureTarget::Window {
            handle: window.handle,
            width: window.rect.width() as u32,
            height: window.rect.height() as u32,
            title: (!window.title.is_empty()).then_some(window.title.clone()),
        },
    }
}

fn write_control_error(
    writer: &mut impl std::io::Write,
    code: ErrorCode,
    message: String,
) -> Result<(), String> {
    write_message(writer, &ControlMessage::Error { code, message })
        .map_err(|error| format!("写入控制错误消息失败: {error}"))
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
        collections::VecDeque,
        net::{TcpListener, TcpStream},
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
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
        assert!(message.contains("尚未实现编码传输和输入注入"));
    }

    #[test]
    fn host_accepts_one_tcp_control_handshake_and_launches_program_before_reporting_runtime_unimplemented()
     {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let endpoint = listener
            .local_addr()
            .expect("listener addr should be available");
        let config = host_config(endpoint.to_string());
        let mut runner = RecordingProgramRunner::default();
        let mut locator = RecordingWindowLocator::default();
        let mut capture = RecordingCaptureStarter::default();
        let host = thread::spawn(move || {
            let result = run_control_listener_once_with_runtime(
                listener,
                &config,
                &mut runner,
                &mut locator,
                &mut capture,
            );
            (result, runner.launched, locator.lookups, capture.targets)
        });

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
                message: "运行时链路未实现：尚未实现编码传输和输入注入。".to_owned(),
            }
        );

        let (host_result, launched, lookups, capture_targets) =
            host.join().expect("host thread should finish");
        assert_eq!(
            host_result.expect("host should handle one client"),
            endpoint
        );
        assert_eq!(
            launched,
            vec![("C:\\Program Files\\SomeApp\\app.exe".to_owned(), Vec::new())]
        );
        assert_eq!(lookups, vec![(42, None)]);
        assert_eq!(capture_targets, vec![CaptureTarget::Desktop]);
    }

    #[test]
    fn host_reports_window_not_found_after_program_launch() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let endpoint = listener
            .local_addr()
            .expect("listener addr should be available");
        let mut config = host_config(endpoint.to_string());
        config.capture.startup_timeout_ms = 1;
        let mut runner = RecordingProgramRunner::default();
        let mut locator = FailingWindowLocator;
        let mut capture = RecordingCaptureStarter::default();
        let host = thread::spawn(move || {
            run_control_listener_once_with_runtime(
                listener,
                &config,
                &mut runner,
                &mut locator,
                &mut capture,
            )
        });

        let mut client = TcpStream::connect(endpoint).expect("client should connect");
        send_client_hello(&mut client).expect("client hello should write");
        read_message(&mut client).expect("host hello should read");
        wincast_protocol::frame::write_message(&mut client, &ControlMessage::StartSession)
            .expect("start session should write");

        assert_eq!(
            read_message(&mut client).expect("window error should read"),
            ControlMessage::Error {
                code: ErrorCode::WindowNotFound,
                message: "定位宿主端程序窗口失败: 未找到进程 42 的主窗口".to_owned(),
            }
        );

        let error = host
            .join()
            .expect("host thread should finish")
            .expect_err("host should report window lookup failure");
        assert!(error.contains("定位宿主端程序窗口失败"));
    }

    #[test]
    fn host_reports_capture_failed_after_window_lookup() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let endpoint = listener
            .local_addr()
            .expect("listener addr should be available");
        let config = host_config(endpoint.to_string());
        let mut runner = RecordingProgramRunner::default();
        let mut locator = RecordingWindowLocator::default();
        let mut capture = FailingCaptureStarter;
        let host = thread::spawn(move || {
            run_control_listener_once_with_runtime(
                listener,
                &config,
                &mut runner,
                &mut locator,
                &mut capture,
            )
        });

        let mut client = TcpStream::connect(endpoint).expect("client should connect");
        send_client_hello(&mut client).expect("client hello should write");
        read_message(&mut client).expect("host hello should read");
        wincast_protocol::frame::write_message(&mut client, &ControlMessage::StartSession)
            .expect("start session should write");

        assert_eq!(
            read_message(&mut client).expect("capture error should read"),
            ControlMessage::Error {
                code: ErrorCode::CaptureFailed,
                message: "初始化画面捕获失败: Windows 画面捕获实现未完成：尚未接入帧获取循环"
                    .to_owned(),
            }
        );

        let error = host
            .join()
            .expect("host thread should finish")
            .expect_err("host should report capture failure");
        assert!(error.contains("初始化画面捕获失败"));
    }

    #[test]
    fn host_treats_missing_initial_frame_as_waitable_state() {
        let config = host_config("127.0.0.1:0".to_owned());
        let window = window_candidate();
        let mut capture = RecordingCaptureStarter {
            frames: VecDeque::from([None, Some(captured_bgra_frame())]),
            ..Default::default()
        };
        let attempts = capture.attempts.clone();

        start_capture_session(&config, &window, &mut capture)
            .expect("host should wait until first frame metadata is available");

        assert_eq!(capture.targets, vec![CaptureTarget::Desktop]);
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn host_reports_capture_failed_when_initial_frame_times_out() {
        let mut config = host_config("127.0.0.1:0".to_owned());
        config.capture.startup_timeout_ms = 1;
        let window = window_candidate();
        let mut capture = RecordingCaptureStarter {
            frames: VecDeque::from([None, None]),
            ..Default::default()
        };

        let error = start_capture_session(&config, &window, &mut capture)
            .expect_err("host should fail when no frame metadata arrives before timeout");

        assert_eq!(
            error,
            CaptureError::windows_frame_read_failed("等待 Windows 捕获首帧超时")
        );
    }

    #[derive(Default)]
    struct RecordingProgramRunner {
        launched: Vec<(String, Vec<String>)>,
    }

    impl ProgramRunner for RecordingProgramRunner {
        fn launch(
            &mut self,
            request: &program::LaunchRequest,
        ) -> Result<program::StartedProgram, program::LaunchError> {
            self.launched
                .push((request.program.display().to_string(), request.args.clone()));
            Ok(program::StartedProgram { process_id: 42 })
        }
    }

    #[derive(Default)]
    struct RecordingWindowLocator {
        lookups: Vec<(u32, Option<String>)>,
    }

    impl WindowLocator for RecordingWindowLocator {
        fn find_main_window(
            &mut self,
            process_id: u32,
            title_contains: Option<&str>,
        ) -> Result<WindowCandidate, WindowLookupError> {
            self.lookups.push((
                process_id,
                title_contains
                    .map(str::trim)
                    .filter(|title| !title.is_empty())
                    .map(str::to_owned),
            ));
            Ok(WindowCandidate {
                handle: 100,
                process_id,
                title: "SomeApp".to_owned(),
                visible: true,
                tool_window: false,
                rect: window::WindowRect {
                    left: 0,
                    top: 0,
                    right: 1280,
                    bottom: 720,
                },
            })
        }
    }

    struct RecordingCaptureStarter {
        targets: Vec<CaptureTarget>,
        frames: VecDeque<Option<CapturedBgraFrame>>,
        attempts: Arc<AtomicUsize>,
    }

    impl Default for RecordingCaptureStarter {
        fn default() -> Self {
            Self {
                targets: Vec::new(),
                frames: VecDeque::from([Some(captured_bgra_frame())]),
                attempts: Arc::new(AtomicUsize::new(0)),
            }
        }
    }

    impl CaptureStarter for RecordingCaptureStarter {
        fn start_capture(
            &mut self,
            target: CaptureTarget,
        ) -> Result<Box<dyn CaptureRuntime>, CaptureError> {
            self.targets.push(target);
            Ok(Box::new(RecordingCaptureRuntime {
                frames: self.frames.clone(),
                attempts: self.attempts.clone(),
            }))
        }
    }

    struct RecordingCaptureRuntime {
        frames: VecDeque<Option<CapturedBgraFrame>>,
        attempts: Arc<AtomicUsize>,
    }

    impl CaptureRuntime for RecordingCaptureRuntime {
        fn try_next_bgra_frame(&mut self) -> Result<Option<CapturedBgraFrame>, CaptureError> {
            self.attempts.fetch_add(1, Ordering::SeqCst);
            Ok(self.frames.pop_front().flatten())
        }
    }

    struct FailingCaptureStarter;

    impl CaptureStarter for FailingCaptureStarter {
        fn start_capture(
            &mut self,
            _target: CaptureTarget,
        ) -> Result<Box<dyn CaptureRuntime>, CaptureError> {
            Err(CaptureError::windows_capture_not_implemented())
        }
    }

    struct FailingWindowLocator;

    impl WindowLocator for FailingWindowLocator {
        fn find_main_window(
            &mut self,
            process_id: u32,
            _title_contains: Option<&str>,
        ) -> Result<WindowCandidate, WindowLookupError> {
            Err(WindowLookupError::NotFound {
                process_id,
                title_contains: None,
            })
        }
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

    fn window_candidate() -> WindowCandidate {
        WindowCandidate {
            handle: 100,
            process_id: 42,
            title: "SomeApp".to_owned(),
            visible: true,
            tool_window: false,
            rect: window::WindowRect {
                left: 0,
                top: 0,
                right: 1280,
                bottom: 720,
            },
        }
    }

    fn captured_bgra_frame() -> CapturedBgraFrame {
        CapturedBgraFrame {
            metadata: wincast_capture::CapturedTextureMetadata {
                frame: wincast_capture::CapturedFrame {
                    width: 1280,
                    height: 720,
                    stride_bytes: 5120,
                    pixel_format: wincast_capture::FramePixelFormat::Bgra8Unorm,
                    sequence_number: 0,
                    timestamp_ns: 0,
                },
                texture_width: 1280,
                texture_height: 720,
                mip_levels: 1,
                array_size: 1,
                sample_count: 1,
            },
            row_pitch: 5120,
            bytes: vec![0; 5120],
        }
    }
}
