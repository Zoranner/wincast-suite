use std::net::TcpStream;

use crate::{
    program::{ProgramRunner, StartedProgram},
    session_events::{DetectedDesktopSession, detect_desktop_session},
    session_state::{RemoteSessionStatus, SessionEvent, SharedSessionState},
};
use wincast_capture::CapturedBgraFrame;
use wincast_media::{
    MediaConfigError, MediaError, RawPixelFormat, RawVideoFrame, RawVideoFrameError, VideoEncoder,
    VideoLatencyMode, VideoPipelineConfig, test_support::FakeH264Encoder,
};
use wincast_protocol::{
    config::{HostConfig, VideoCodec},
    frame::{read_message, write_message},
    handshake::accept_client_hello,
    ipc::SessionEndReason,
    message::{ControlMessage, ErrorCode},
};

use super::{
    capture::{
        CaptureStarter, WindowLocator, capture_input_bounds, locate_started_window,
        start_capture_session,
    },
    stream::{
        HostSessionEndReason, HostSessionError, write_control_error, write_raw_bgra_stream,
        write_session_ready,
    },
};

pub(super) fn handle_control_client(
    stream: &mut TcpStream,
    config: &HostConfig,
    runner: &mut impl ProgramRunner,
    locator: &mut impl WindowLocator,
    capture: &mut impl CaptureStarter,
) -> Result<(), String> {
    let mut session_gate = SharedSessionGate::new(foreground_run_session_state());
    handle_control_client_with_session_gate(
        stream,
        config,
        runner,
        locator,
        capture,
        &mut session_gate,
    )
}

pub(super) fn handle_control_client_with_session_gate(
    stream: &mut TcpStream,
    config: &HostConfig,
    runner: &mut impl ProgramRunner,
    locator: &mut impl WindowLocator,
    capture: &mut impl CaptureStarter,
    session_gate: &mut impl SessionGate,
) -> Result<(), String> {
    let mut writer = stream
        .try_clone()
        .map_err(|error| format!("克隆控制连接写入端失败: {error}"))?;
    accept_client_hello(stream, &mut writer).map_err(|error| format!("控制握手失败: {error}"))?;

    match read_message(stream).map_err(|error| format!("读取控制消息失败: {error}"))? {
        ControlMessage::StartSession => {
            ensure_remote_session_allowed(&mut writer, session_gate)?;
            // 与 Service↔Agent IPC 的 `StatusChanged` 语义对齐，后续由 Service 拉起的 Agent 应上报同一映射。
            let _ = session_gate.remote_session_status().to_ipc_agent_status();
            let mut started =
                crate::program::launch_with_runner(config, runner).map_err(|error| {
                    let message = format!("启动宿主端程序失败: {error}");
                    let _ = write_control_error(
                        &mut writer,
                        ErrorCode::ProgramLaunchFailed,
                        message.clone(),
                    );
                    message
                })?;
            let result =
                run_started_session(&mut writer, stream, config, locator, capture, &started);
            let cleanup_result = runner
                .cleanup(&mut started)
                .map_err(|error| format!("清理宿主端程序失败: {error}"));
            match (result, cleanup_result) {
                (Ok(reason), Ok(())) => {
                    let _ = SessionEndReason::from(reason);
                    Ok(())
                }
                (Err(error), Ok(())) => {
                    let _ = SessionEndReason::from(error.reason);
                    Err(error.message)
                }
                (Ok(reason), Err(error)) => {
                    let _ = SessionEndReason::from(reason);
                    Err(error)
                }
                (Err(session_error), Err(cleanup_error)) => {
                    let _ = SessionEndReason::from(session_error.reason);
                    Err(format!("{}；{cleanup_error}", session_error.message))
                }
            }
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

pub(super) trait SessionGate {
    fn remote_session_status(&mut self) -> RemoteSessionStatus;
}

pub(super) struct SharedSessionGate {
    state: SharedSessionState,
}

impl SharedSessionGate {
    pub(super) fn new(state: SharedSessionState) -> Self {
        Self { state }
    }
}

impl SessionGate for SharedSessionGate {
    fn remote_session_status(&mut self) -> RemoteSessionStatus {
        self.state.remote_session_status()
    }
}

fn foreground_run_session_state() -> SharedSessionState {
    foreground_run_session_state_from_detection_with_failure_policy(
        detect_desktop_session(),
        foreground_detection_failure_should_fallback_to_development(),
    )
}

#[cfg(test)]
pub(super) fn foreground_run_session_state_from_detection(
    detected: Result<DetectedDesktopSession, String>,
) -> SharedSessionState {
    foreground_run_session_state_from_detection_with_failure_policy(detected, false)
}

pub(super) fn foreground_run_session_state_from_detection_with_failure_policy(
    detected: Result<DetectedDesktopSession, String>,
    fallback_to_development: bool,
) -> SharedSessionState {
    let state = detected
        .map(crate::session_events::shared_session_state_from_detected_desktop_session)
        .unwrap_or_else(|_| {
            foreground_session_state_after_detection_failure(fallback_to_development)
        });
    state.apply(SessionEvent::AgentStarted);
    state
}

fn foreground_session_state_after_detection_failure(
    fallback_to_development: bool,
) -> SharedSessionState {
    if fallback_to_development {
        foreground_development_session_state()
    } else {
        SharedSessionState::new()
    }
}

#[cfg(windows)]
fn foreground_detection_failure_should_fallback_to_development() -> bool {
    false
}

#[cfg(not(windows))]
fn foreground_detection_failure_should_fallback_to_development() -> bool {
    true
}

fn foreground_development_session_state() -> SharedSessionState {
    let state = SharedSessionState::new();
    state.apply(SessionEvent::UserLoggedIn);
    state
}

fn ensure_remote_session_allowed(
    writer: &mut impl std::io::Write,
    session_gate: &mut impl SessionGate,
) -> Result<(), String> {
    match session_gate.remote_session_status().to_protocol_error() {
        Some((code, message)) => {
            write_control_error(writer, code, message.to_owned())?;
            Err(message.to_owned())
        }
        None => Ok(()),
    }
}

pub(super) fn run_started_session(
    writer: &mut impl std::io::Write,
    stream: &TcpStream,
    config: &HostConfig,
    locator: &mut impl WindowLocator,
    capture: &mut impl CaptureStarter,
    started: &StartedProgram,
) -> Result<HostSessionEndReason, HostSessionError> {
    let window = locate_started_window(config, started, locator).map_err(|error| {
        let message = format!("定位宿主端程序窗口失败: {error}");
        let write_result = write_control_error(writer, ErrorCode::WindowNotFound, message.clone());
        let message = append_error_response_write_failure(message, write_result);
        HostSessionError::new(HostSessionEndReason::CaptureFailed, message)
    })?;
    let (mut session, first_frame, active_capture_mode) =
        start_capture_session(config, &window, capture).map_err(|error| {
            let message = format!("初始化画面捕获失败: {error}");
            let write_result =
                write_control_error(writer, ErrorCode::CaptureFailed, message.clone());
            let message = append_error_response_write_failure(message, write_result);
            HostSessionError::new(HostSessionEndReason::CaptureFailed, message)
        })?;
    write_session_ready(writer, &first_frame)
        .map_err(|message| HostSessionError::new(HostSessionEndReason::TransportFailed, message))?;
    match config.video.codec {
        VideoCodec::RawBgra => write_raw_bgra_stream(
            writer,
            stream,
            &first_frame,
            session.as_mut(),
            capture_input_bounds(active_capture_mode, &window, &first_frame),
        ),
        VideoCodec::H264 => write_first_h264_frame(writer, config, &first_frame),
    }
}

fn write_first_h264_frame(
    writer: &mut impl std::io::Write,
    config: &HostConfig,
    first_frame: &CapturedBgraFrame,
) -> Result<HostSessionEndReason, HostSessionError> {
    let pipeline_config = VideoPipelineConfig {
        codec: VideoCodec::H264,
        width: config.video.width,
        height: config.video.height,
        fps: config.video.fps,
        bitrate_kbps: config.video.bitrate_kbps,
        max_bitrate_kbps: config.video.max_bitrate_kbps,
        latency_mode: VideoLatencyMode::LowLatency,
    };
    let mut encoder = FakeH264Encoder::new(pipeline_config).map_err(|error| {
        write_h264_encoding_error(
            writer,
            format!("H.264 编码器初始化失败: {}", media_error_message(&error)),
        )
    })?;
    let encoded = encoder
        .encode(raw_video_frame_from_capture(first_frame))
        .map_err(|error| {
            write_h264_encoding_error(
                writer,
                format!("H.264 首帧编码失败: {}", media_error_message(&error)),
            )
        })?;
    write_message(writer, &ControlMessage::EncodedVideoFrame(encoded)).map_err(|error| {
        HostSessionError::new(
            HostSessionEndReason::TransportFailed,
            format!("写入 H.264 编码视频帧失败: {error}"),
        )
    })?;

    Ok(HostSessionEndReason::CaptureInactive)
}

fn raw_video_frame_from_capture(frame: &CapturedBgraFrame) -> RawVideoFrame<'_> {
    RawVideoFrame {
        width: frame.metadata.frame.width,
        height: frame.metadata.frame.height,
        row_pitch: frame.row_pitch,
        format: RawPixelFormat::Bgra8Unorm,
        sequence_number: frame.metadata.frame.sequence_number,
        timestamp_ns: frame.metadata.frame.timestamp_ns,
        bytes: &frame.bytes,
    }
}

fn write_h264_encoding_error(
    writer: &mut impl std::io::Write,
    message: String,
) -> HostSessionError {
    let write_result = write_control_error(writer, ErrorCode::EncodingFailed, message.clone());
    let message = append_error_response_write_failure(message, write_result);
    HostSessionError::new(HostSessionEndReason::CaptureFailed, message)
}

fn media_error_message(error: &MediaError) -> String {
    match error {
        MediaError::Config(error) => media_config_error_message(error),
        MediaError::InvalidRawFrame(error) => raw_video_frame_error_message(error),
        MediaError::UnsupportedEncodedCodec { codec } => {
            format!("编码视频帧只支持 H.264，当前为 {codec:?}")
        }
        MediaError::InvalidEncodedFrame(error) => format!("编码视频帧无效: {error:?}"),
        MediaError::DecodedPayloadTooLarge { actual, max } => {
            format!("fake 解码输出载荷 {actual} 超过上限 {max}")
        }
        MediaError::BackendUnavailable(message) => format!("媒体后端不可用: {message}"),
        MediaError::Backend(message) => format!("媒体后端处理失败: {message}"),
    }
}

fn media_config_error_message(error: &MediaConfigError) -> String {
    match error {
        MediaConfigError::UnsupportedCodec { codec } => {
            format!("媒体链路只支持 H.264，当前配置为 {codec:?}")
        }
        MediaConfigError::InvalidDimensions { width, height } => {
            format!("视频尺寸无效: {width}x{height}")
        }
        MediaConfigError::ResolutionTooLarge {
            width,
            height,
            max_width,
            max_height,
        } => format!("视频尺寸 {width}x{height} 超过上限 {max_width}x{max_height}"),
        MediaConfigError::InvalidFps { fps, max_fps } => {
            format!("视频帧率 {fps} 无效，最大支持 {max_fps}")
        }
        MediaConfigError::InvalidMaxBitrate => "视频码率上限必须大于 0".to_owned(),
        MediaConfigError::InvalidBitrate {
            bitrate_kbps,
            max_bitrate_kbps,
        } => format!("视频目标码率 {bitrate_kbps} 无效，上限为 {max_bitrate_kbps}"),
    }
}

fn raw_video_frame_error_message(error: &RawVideoFrameError) -> String {
    match error {
        RawVideoFrameError::InvalidDimensions { width, height } => {
            format!("raw 视频尺寸无效: {width}x{height}")
        }
        RawVideoFrameError::ResolutionTooLarge {
            width,
            height,
            max_width,
            max_height,
        } => format!("视频尺寸 {width}x{height} 超过上限 {max_width}x{max_height}"),
        RawVideoFrameError::ConfigDimensionMismatch {
            frame_width,
            frame_height,
            config_width,
            config_height,
        } => format!(
            "raw 视频尺寸 {frame_width}x{frame_height} 与配置尺寸 {config_width}x{config_height} 不一致"
        ),
        RawVideoFrameError::UnsupportedPixelFormat { format } => {
            format!("fake H.264 编码器只支持 BGRA8 raw 帧，当前为 {format:?}")
        }
        RawVideoFrameError::EmptyPayload => "raw 视频帧载荷为空".to_owned(),
        RawVideoFrameError::RowPitchOverflow => "raw 视频帧行跨度溢出".to_owned(),
        RawVideoFrameError::InvalidRowPitch {
            row_pitch,
            min_row_pitch,
        } => format!("raw 视频帧行跨度 {row_pitch} 小于最小值 {min_row_pitch}"),
        RawVideoFrameError::PayloadLengthOverflow => "raw 视频帧载荷长度溢出".to_owned(),
        RawVideoFrameError::InvalidPayloadLength { actual, expected } => {
            format!("raw 视频帧载荷长度 {actual} 与期望长度 {expected} 不一致")
        }
    }
}

fn append_error_response_write_failure(
    message: String,
    write_result: Result<(), String>,
) -> String {
    match write_result {
        Ok(()) => message,
        Err(write_error) => format!("{message}；{write_error}"),
    }
}
