use std::{
    collections::VecDeque,
    net::{SocketAddr, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use crate::{
    agent::capture::{CaptureRuntime, CaptureStarter, InputEventSink},
    agent::session::SessionGate,
    monitor_power::{MonitorPowerController, MonitorPowerError, monitor_power_error},
    program::{self, ProgramRunner},
    session_state::RemoteSessionStatus,
};
use wincast_capture::{CaptureError, CaptureTarget, CapturedBgraFrame};
use wincast_protocol::{
    config::{
        CaptureConfig, HostConfig, MonitorPowerAfterLaunch, ProgramConfig, VideoCodec, VideoConfig,
    },
    frame::{read_message, write_message},
    handshake::send_client_hello,
    input::InputEvent,
    message::{ControlMessage, EncodedVideoFrame, ErrorCode},
};

#[derive(Default)]
pub(super) struct RecordingProgramRunner {
    pub(super) launched: Vec<(String, Vec<String>)>,
    pub(super) cleaned: Vec<u32>,
    next_process_id: u32,
}

impl ProgramRunner for RecordingProgramRunner {
    fn launch(
        &mut self,
        request: &program::LaunchRequest,
    ) -> Result<program::StartedProgram, program::LaunchError> {
        self.launched
            .push((request.program.display().to_string(), request.args.clone()));
        let process_id = if self.next_process_id == 0 {
            self.next_process_id = 43;
            42
        } else {
            let process_id = self.next_process_id;
            self.next_process_id += 1;
            process_id
        };
        Ok(program::StartedProgram::from_process_id(process_id))
    }

    fn cleanup(
        &mut self,
        started: &mut program::StartedProgram,
    ) -> Result<(), program::LaunchError> {
        self.cleaned.push(started.process_id);
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct RecordingMonitorPowerController {
    pub(super) policies: Vec<MonitorPowerAfterLaunch>,
    pub(super) fail_message: Option<&'static str>,
}

impl MonitorPowerController for RecordingMonitorPowerController {
    fn apply_after_launch(
        &mut self,
        policy: MonitorPowerAfterLaunch,
    ) -> Result<(), MonitorPowerError> {
        self.policies.push(policy);
        match self.fail_message {
            Some(message) => Err(monitor_power_error(message)),
            None => Ok(()),
        }
    }
}

pub(super) struct RecordingCaptureStarter {
    pub(super) targets: Vec<CaptureTarget>,
    pub(super) frames: VecDeque<Option<CapturedBgraFrame>>,
    pub(super) attempts: Arc<AtomicUsize>,
    pub(super) block_after_empty: Option<Arc<BlockingFrameGate>>,
}

impl Default for RecordingCaptureStarter {
    fn default() -> Self {
        Self {
            targets: Vec::new(),
            frames: VecDeque::from([Some(captured_bgra_frame())]),
            attempts: Arc::new(AtomicUsize::new(0)),
            block_after_empty: None,
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
            block_after_empty: self.block_after_empty.clone(),
        }))
    }
}

pub(super) struct FixedSessionGate(pub(super) RemoteSessionStatus);

impl SessionGate for FixedSessionGate {
    fn remote_session_status(&self) -> RemoteSessionStatus {
        self.0
    }
}

pub(super) struct SequenceSessionGate {
    statuses: Mutex<VecDeque<RemoteSessionStatus>>,
}

impl SequenceSessionGate {
    pub(super) fn new(statuses: impl IntoIterator<Item = RemoteSessionStatus>) -> Self {
        Self {
            statuses: Mutex::new(statuses.into_iter().collect()),
        }
    }
}

impl SessionGate for SequenceSessionGate {
    fn remote_session_status(&self) -> RemoteSessionStatus {
        self.statuses
            .lock()
            .expect("sequence gate lock should not be poisoned")
            .pop_front()
            .unwrap_or(RemoteSessionStatus::Allowed)
    }
}

pub(super) struct RecordingCaptureRuntime {
    pub(super) frames: VecDeque<Option<CapturedBgraFrame>>,
    pub(super) attempts: Arc<AtomicUsize>,
    pub(super) block_after_empty: Option<Arc<BlockingFrameGate>>,
}

impl CaptureRuntime for RecordingCaptureRuntime {
    fn is_active(&self) -> bool {
        !self.frames.is_empty()
    }

    fn try_next_bgra_frame(&mut self) -> Result<Option<CapturedBgraFrame>, CaptureError> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        let frame = self.frames.pop_front().flatten();
        if frame.is_none()
            && self.frames.is_empty()
            && let Some(block) = self.block_after_empty.take()
        {
            block.block_until_released();
        }
        Ok(frame)
    }
}

pub(super) struct BlockingFrameGate {
    blocked: AtomicBool,
    released: AtomicBool,
}

impl BlockingFrameGate {
    pub(super) fn new() -> Self {
        Self {
            blocked: AtomicBool::new(false),
            released: AtomicBool::new(false),
        }
    }

    pub(super) fn wait_until_blocked(&self) {
        while !self.blocked.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(1));
        }
    }

    pub(super) fn release(&self) {
        self.released.store(true, Ordering::SeqCst);
    }

    fn block_until_released(&self) {
        self.blocked.store(true, Ordering::SeqCst);
        while !self.released.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(1));
        }
    }
}

impl Drop for BlockingFrameGate {
    fn drop(&mut self) {
        self.released.store(true, Ordering::SeqCst);
    }
}

pub(super) struct FailingCaptureStarter;

impl CaptureStarter for FailingCaptureStarter {
    fn start_capture(
        &mut self,
        _target: CaptureTarget,
    ) -> Result<Box<dyn CaptureRuntime>, CaptureError> {
        Err(CaptureError::windows_capture_not_implemented())
    }
}

#[derive(Default)]
pub(super) struct RecordingInputEventSink {
    pub(super) events: Vec<InputEvent>,
}

impl InputEventSink for RecordingInputEventSink {
    fn handle_input_event(&mut self, event: InputEvent) -> Result<(), String> {
        self.events.push(event);
        Ok(())
    }
}

pub(super) fn host_config(listen: String) -> HostConfig {
    host_config_with_codec(listen, VideoCodec::H264)
}

pub(super) fn host_config_with_codec(listen: String, codec: VideoCodec) -> HostConfig {
    HostConfig {
        listen,
        program: ProgramConfig {
            path: "C:\\Program Files\\SomeApp\\app.exe".to_owned(),
            args: Vec::new(),
            work_dir: "C:\\Program Files\\SomeApp".to_owned(),
            startup_delay_ms: 0,
            turn_off_monitor_after_launch: MonitorPowerAfterLaunch::Disabled,
        },
        video: VideoConfig {
            width: 1280,
            height: 720,
            fps: 30,
            codec,
            bitrate_kbps: 4000,
            max_bitrate_kbps: 6000,
        },
        capture: CaptureConfig {
            first_frame_timeout_ms: 5000,
        },
    }
}

pub(super) fn screen_capture_target() -> CaptureTarget {
    CaptureTarget::Screen
}

pub(super) fn captured_bgra_frame() -> CapturedBgraFrame {
    captured_bgra_frame_with_sequence(0)
}

pub(super) fn captured_bgra_frame_with_sequence(sequence_number: u64) -> CapturedBgraFrame {
    captured_bgra_frame_with_dimensions_and_sequence(1280, 720, sequence_number)
}

pub(super) fn captured_bgra_frame_with_dimensions(width: u32, height: u32) -> CapturedBgraFrame {
    captured_bgra_frame_with_dimensions_and_sequence(width, height, 0)
}

fn captured_bgra_frame_with_dimensions_and_sequence(
    width: u32,
    height: u32,
    sequence_number: u64,
) -> CapturedBgraFrame {
    let row_pitch = width * 4;
    CapturedBgraFrame {
        metadata: wincast_capture::CapturedTextureMetadata {
            frame: wincast_capture::CapturedFrame {
                width,
                height,
                stride_bytes: row_pitch,
                pixel_format: wincast_capture::FramePixelFormat::Bgra8Unorm,
                sequence_number,
                timestamp_ns: sequence_number * 1_000_000,
            },
            texture_width: width,
            texture_height: height,
            mip_levels: 1,
            array_size: 1,
            sample_count: 1,
        },
        row_pitch,
        bytes: vec![0; row_pitch as usize * height as usize],
    }
}

pub(super) fn connect_and_start_session(endpoint: SocketAddr) -> TcpStream {
    let mut client = TcpStream::connect(endpoint).expect("client should connect");
    send_client_hello(&mut client).expect("client hello should write");
    assert_eq!(
        read_message(&mut client).expect("host hello should read"),
        ControlMessage::Hello { version: 1 }
    );
    write_message(&mut client, &ControlMessage::StartSession).expect("start session should write");
    client
}

pub(super) fn run_short_client_session(endpoint: SocketAddr) -> EncodedVideoFrame {
    let mut client = connect_and_start_session_when_ready(endpoint);
    read_message(&mut client).expect("session ready should read");
    let frame = expect_h264_frame(read_message(&mut client).expect("encoded frame should read"));
    write_message(&mut client, &ControlMessage::StopSession).expect("stop session should write");
    assert_eq!(
        read_message(&mut client).expect("goodbye should read after stop"),
        ControlMessage::Goodbye
    );
    frame
}

pub(super) fn expect_h264_frame(message: ControlMessage) -> EncodedVideoFrame {
    let ControlMessage::EncodedVideoFrame(encoded) = message else {
        panic!("expected encoded video frame, got {message:?}");
    };
    encoded
        .validate()
        .expect("encoded frame should satisfy protocol boundary");
    assert_eq!(encoded.codec, VideoCodec::H264);
    encoded
}

pub(super) fn connect_and_start_session_when_ready(endpoint: SocketAddr) -> TcpStream {
    let mut last_error = None;
    for _ in 0..50 {
        let mut client = TcpStream::connect(endpoint).expect("client should connect");
        send_client_hello(&mut client).expect("client hello should write");
        match read_message(&mut client).expect("host hello or busy should read") {
            ControlMessage::Hello { version: 1 } => {
                write_message(&mut client, &ControlMessage::StartSession)
                    .expect("start session should write");
                return client;
            }
            ControlMessage::Error {
                code: ErrorCode::Busy,
                message,
            } => {
                last_error = Some(message);
                thread::sleep(Duration::from_millis(20));
            }
            message => {
                panic!("unexpected host response while waiting for session: {message:?}")
            }
        }
    }
    panic!(
        "host should accept a new session after previous cleanup, last busy: {:?}",
        last_error
    );
}
