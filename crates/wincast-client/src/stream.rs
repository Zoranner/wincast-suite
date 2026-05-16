use wincast_media::{OpenH264Decoder, VideoDecoder};
use wincast_protocol::{
    config::VideoCodec,
    frame::read_message,
    message::{ControlMessage, EncodedVideoFrame, RawBgraReadbackFrame},
    raw_frame::{
        RawBgraFrame, RawBgraStreamItem as ProtocolRawBgraStreamItem,
        read_raw_bgra_stream_item as read_protocol_raw_bgra_stream_item,
    },
};

use crate::{errors::format_host_error, render_loop::ClientRenderMode};

const RAW_BGRA_VALIDATION_FRAME_COUNT: usize = 1;

pub(crate) fn read_first_raw_binary_frame(
    stream: &mut std::net::TcpStream,
    render_mode: ClientRenderMode,
    width: u32,
    height: u32,
) -> Result<(), String> {
    match render_mode {
        ClientRenderMode::SdlWindow => {
            crate::render_loop::read_first_raw_binary_frame_with_sdl_window(stream, width, height)
        }
        ClientRenderMode::ProtocolOnly => {
            read_raw_bgra_frames(stream, RAW_BGRA_VALIDATION_FRAME_COUNT).map(|_| ())
        }
    }
}

pub(crate) fn read_raw_bgra_frames(
    reader: &mut impl std::io::Read,
    frame_count: usize,
) -> Result<RawBgraReceiveSummary, String> {
    if frame_count == 0 {
        return Err("raw BGRA 视频帧接收数量不能为 0".to_owned());
    }

    let mut last_sequence_number = None;
    let mut frames = 0;
    for _ in 0..frame_count {
        match read_raw_bgra_stream_item(reader).map_err(format_raw_bgra_stream_error)? {
            RawBgraStreamItem::Frame(frame) => {
                validate_raw_frame_sequence(&frame, &mut last_sequence_number)?;
                frames += 1;
            }
            RawBgraStreamItem::Goodbye => {
                break;
            }
        }
    }

    let Some(last_sequence_number) = last_sequence_number else {
        return Err("未收到 raw BGRA 视频帧".to_owned());
    };

    Ok(RawBgraReceiveSummary {
        frames,
        last_sequence_number,
    })
}

pub(crate) fn read_h264_encoded_frames_from_first(
    reader: &mut impl std::io::Read,
    first_frame: EncodedVideoFrame,
    frame_count: usize,
) -> Result<EncodedVideoReceiveSummary, String> {
    if frame_count == 0 {
        return Err("H.264 编码视频帧接收数量不能为 0".to_owned());
    }

    let mut decoder = OpenH264Decoder::new()
        .map_err(|error| format!("初始化客户端 OpenH264 解码器失败: {error}"))?;
    let mut last_sequence_number = None;
    let mut frames = 0;

    validate_h264_frame_sequence(&first_frame, &mut last_sequence_number)?;
    decode_h264_frame_boundary(&mut decoder, &first_frame)?;
    frames += 1;

    for _ in 1..frame_count {
        match read_next_h264_encoded_stream_item(reader)
            .map_err(format_h264_encoded_stream_error)?
        {
            NextEncodedVideoStreamItem::Frame(frame) => {
                validate_h264_frame_sequence(&frame, &mut last_sequence_number)?;
                decode_h264_frame_boundary(&mut decoder, &frame)?;
                frames += 1;
            }
            NextEncodedVideoStreamItem::Goodbye => {
                break;
            }
        }
    }

    let Some(last_sequence_number) = last_sequence_number else {
        return Err("未收到 H.264 编码视频帧".to_owned());
    };

    Ok(EncodedVideoReceiveSummary {
        frames,
        last_sequence_number,
    })
}

#[cfg(target_os = "linux")]
pub(crate) fn read_h264_encoded_frames_with_sdl_window_from_first(
    stream: &mut std::net::TcpStream,
    first_frame: EncodedVideoFrame,
    width: u32,
    height: u32,
) -> Result<(), String> {
    let mut renderer = wincast_render::SdlRawBgraRenderer::new(wincast_render::RenderConfig {
        title: "WinCast Client".to_owned(),
        width,
        height,
    })
    .map_err(|error| format!("创建客户端 SDL2 窗口失败: {error}"))?;
    read_h264_encoded_frames_with_renderer_from_first(stream, first_frame, None, &mut renderer)
        .map(|_| ())
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn read_h264_encoded_frames_with_sdl_window_from_first(
    _stream: &mut std::net::TcpStream,
    _first_frame: EncodedVideoFrame,
    _width: u32,
    _height: u32,
) -> Result<(), String> {
    Err("当前平台不支持 SDL2 客户端窗口".to_owned())
}

#[cfg(any(test, target_os = "linux"))]
pub(crate) fn read_h264_encoded_frames_with_renderer_from_first(
    control_stream: &mut (impl std::io::Read + std::io::Write),
    first_frame: EncodedVideoFrame,
    frame_limit: Option<usize>,
    renderer: &mut impl wincast_render::RawBgraRenderer,
) -> Result<EncodedVideoReceiveSummary, String> {
    let mut decoder = OpenH264Decoder::new()
        .map_err(|error| format!("初始化客户端 OpenH264 解码器失败: {error}"))?;
    let mut last_sequence_number = None;
    let mut frames = 0;
    render_h264_encoded_frame(
        &mut decoder,
        renderer,
        first_frame,
        &mut last_sequence_number,
    )?;
    frames += 1;

    loop {
        let render_loop = renderer
            .poll_input()
            .map_err(|error| format!("读取客户端输入事件失败: {error}"))?;
        for input_event in render_loop.input_events {
            wincast_protocol::frame::write_message(
                control_stream,
                &ControlMessage::InputEvent(input_event),
            )
            .map_err(|error| format!("发送客户端输入事件失败: {error}"))?;
        }
        if render_loop.action == wincast_render::RenderLoopAction::Quit {
            let _ = wincast_protocol::frame::write_message(
                control_stream,
                &ControlMessage::StopSession,
            );
            break;
        }
        if frame_limit.is_some_and(|limit| frames >= limit) {
            break;
        }
        match read_next_h264_encoded_stream_item(control_stream)
            .map_err(format_h264_encoded_stream_error)?
        {
            NextEncodedVideoStreamItem::Frame(frame) => {
                render_h264_encoded_frame(
                    &mut decoder,
                    renderer,
                    frame,
                    &mut last_sequence_number,
                )?;
                frames += 1;
            }
            NextEncodedVideoStreamItem::Goodbye => break,
        }
    }

    let Some(last_sequence_number) = last_sequence_number else {
        return Err("未收到 H.264 编码视频帧".to_owned());
    };
    Ok(EncodedVideoReceiveSummary {
        frames,
        last_sequence_number,
    })
}

pub(crate) fn validate_readback_frame(frame: &RawBgraReadbackFrame) -> Result<(), String> {
    frame
        .validate()
        .map_err(|error| format!("宿主端首帧 BGRA readback 无效: {error:?}"))
}

fn validate_raw_binary_frame(frame: &RawBgraFrame) -> Result<(), String> {
    frame
        .validate()
        .map_err(|error| format!("宿主端 raw BGRA 视频帧无效: {error}"))
}

#[cfg(any(test, target_os = "linux"))]
pub(crate) fn render_raw_bgra_frame(
    renderer: &mut impl wincast_render::RawBgraRenderer,
    frame: &RawBgraFrame,
    last_sequence_number: &mut Option<u64>,
) -> Result<(), String> {
    validate_raw_frame_sequence(frame, last_sequence_number)?;
    renderer
        .render_frame(frame)
        .map_err(|error| format!("渲染宿主端 raw BGRA 视频帧失败: {error}"))
}

fn validate_raw_frame_sequence(
    frame: &RawBgraFrame,
    last_sequence_number: &mut Option<u64>,
) -> Result<(), String> {
    validate_raw_binary_frame(frame)?;
    if let Some(previous) = *last_sequence_number
        && frame.sequence_number < previous
    {
        return Err(format!(
            "宿主端 raw BGRA 视频帧序号回退: 上一帧 {previous}，当前帧 {}",
            frame.sequence_number
        ));
    }
    *last_sequence_number = Some(frame.sequence_number);
    Ok(())
}

pub(crate) fn read_raw_bgra_stream_item(
    reader: &mut impl std::io::Read,
) -> Result<RawBgraStreamItem, RawBgraStreamReadError> {
    match read_protocol_raw_bgra_stream_item(reader)
        .map_err(|error| RawBgraStreamReadError::Interrupted(error.to_string()))?
    {
        ProtocolRawBgraStreamItem::Frame(frame) => Ok(RawBgraStreamItem::Frame(frame)),
        ProtocolRawBgraStreamItem::Control(ControlMessage::Error { code, message }) => Err(
            RawBgraStreamReadError::Host(format_host_error(code, message)),
        ),
        ProtocolRawBgraStreamItem::Control(ControlMessage::Goodbye) => {
            Ok(RawBgraStreamItem::Goodbye)
        }
        ProtocolRawBgraStreamItem::Control(message) => Err(RawBgraStreamReadError::InvalidControl(
            format!("宿主端 raw BGRA 流中收到无效控制消息: {message:?}"),
        )),
    }
}

fn read_next_h264_encoded_stream_item(
    reader: &mut impl std::io::Read,
) -> Result<NextEncodedVideoStreamItem, EncodedVideoStreamReadError> {
    match read_message(reader)
        .map_err(|error| EncodedVideoStreamReadError::Interrupted(error.to_string()))?
    {
        ControlMessage::EncodedVideoFrame(frame) => Ok(NextEncodedVideoStreamItem::Frame(frame)),
        ControlMessage::Error { code, message } => Err(EncodedVideoStreamReadError::Host(
            format_host_error(code, message),
        )),
        ControlMessage::Goodbye => Ok(NextEncodedVideoStreamItem::Goodbye),
        message => Err(EncodedVideoStreamReadError::InvalidControl(format!(
            "宿主端 H.264 编码流中收到无效控制消息: {message:?}"
        ))),
    }
}

pub(crate) fn format_raw_bgra_read_error(error: impl std::fmt::Display) -> String {
    format!("视频流中断: 读取宿主端 raw BGRA 视频帧失败: {error}")
}

pub(crate) fn format_h264_encoded_read_error(error: impl std::fmt::Display) -> String {
    format!("视频流中断: 读取宿主端 H.264 编码视频帧失败: {error}")
}

pub(crate) fn format_raw_bgra_stream_error(error: RawBgraStreamReadError) -> String {
    match error {
        RawBgraStreamReadError::Host(message) | RawBgraStreamReadError::InvalidControl(message) => {
            message
        }
        RawBgraStreamReadError::Interrupted(message) => format_raw_bgra_read_error(message),
    }
}

fn format_h264_encoded_stream_error(error: EncodedVideoStreamReadError) -> String {
    match error {
        EncodedVideoStreamReadError::Host(message)
        | EncodedVideoStreamReadError::InvalidControl(message) => message,
        EncodedVideoStreamReadError::Interrupted(message) => {
            format_h264_encoded_read_error(message)
        }
    }
}

pub(crate) enum RawBgraStreamReadError {
    Host(String),
    InvalidControl(String),
    Interrupted(String),
}

pub(crate) enum RawBgraStreamItem {
    Frame(RawBgraFrame),
    Goodbye,
}

enum EncodedVideoStreamReadError {
    Host(String),
    InvalidControl(String),
    Interrupted(String),
}

enum NextEncodedVideoStreamItem {
    Frame(EncodedVideoFrame),
    Goodbye,
}

#[cfg(any(test, target_os = "linux"))]
pub(crate) enum RawBgraStreamEvent {
    Frame(RawBgraFrame),
    Goodbye,
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RawBgraReceiveSummary {
    pub(crate) frames: usize,
    pub(crate) last_sequence_number: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EncodedVideoReceiveSummary {
    pub(crate) frames: usize,
    pub(crate) last_sequence_number: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DecodedVideoFrameBoundary {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) row_pitch: u32,
    pub(crate) bytes_len: usize,
}

#[cfg(test)]
pub(crate) fn validate_encoded_video_frame(
    frame: &EncodedVideoFrame,
) -> Result<DecodedVideoFrameBoundary, String> {
    let mut decoder = OpenH264Decoder::new()
        .map_err(|error| format!("初始化客户端 OpenH264 解码器失败: {error}"))?;
    decode_h264_frame_boundary(&mut decoder, frame)
}

fn validate_h264_frame_sequence(
    frame: &EncodedVideoFrame,
    last_sequence_number: &mut Option<u64>,
) -> Result<(), String> {
    if frame.codec != VideoCodec::H264 {
        return Err(format!("宿主端编码帧 codec 无效: {:?}", frame.codec));
    }
    frame
        .validate()
        .map_err(|error| format!("宿主端 H.264 编码帧无效: {error:?}"))?;
    if let Some(previous) = *last_sequence_number
        && frame.sequence_number < previous
    {
        return Err(format!(
            "宿主端 H.264 编码帧序号回退: 上一帧 {previous}，当前帧 {}",
            frame.sequence_number
        ));
    }
    *last_sequence_number = Some(frame.sequence_number);
    Ok(())
}

fn decode_h264_frame_boundary(
    decoder: &mut OpenH264Decoder,
    frame: &EncodedVideoFrame,
) -> Result<DecodedVideoFrameBoundary, String> {
    if frame.codec != VideoCodec::H264 {
        return Err(format!("宿主端编码帧 codec 无效: {:?}", frame.codec));
    }
    frame
        .validate()
        .map_err(|error| format!("宿主端 H.264 编码帧无效: {error:?}"))?;

    let decoded = decoder
        .decode(frame)
        .map_err(|error| format!("宿主端 H.264 编码帧解码失败: {error}"))?;
    let row_pitch = decoded.row_pitch();
    let expected_len = row_pitch
        .checked_mul(decoded.height)
        .ok_or_else(|| "宿主端 H.264 编码帧解码失败: decoded frame 尺寸溢出".to_owned())?
        as usize;
    if decoded.bytes.len() != expected_len {
        return Err(format!(
            "宿主端 H.264 编码帧解码失败: decoded frame 字节数 {} 与 row_pitch * height {expected_len} 不一致",
            decoded.bytes.len()
        ));
    }

    Ok(DecodedVideoFrameBoundary {
        width: decoded.width,
        height: decoded.height,
        row_pitch,
        bytes_len: decoded.bytes.len(),
    })
}

#[cfg(any(test, target_os = "linux"))]
fn render_h264_encoded_frame(
    decoder: &mut OpenH264Decoder,
    renderer: &mut impl wincast_render::RawBgraRenderer,
    frame: EncodedVideoFrame,
    last_sequence_number: &mut Option<u64>,
) -> Result<(), String> {
    validate_h264_frame_sequence(&frame, last_sequence_number)?;
    let decoded = decoder
        .decode(&frame)
        .map_err(|error| format!("宿主端 H.264 编码帧解码失败: {error}"))?;
    let raw = RawBgraFrame {
        width: decoded.width,
        height: decoded.height,
        row_pitch: decoded.row_pitch(),
        sequence_number: frame.sequence_number,
        timestamp_ns: frame.timestamp_ns,
        bytes: decoded.bytes.to_vec(),
    };
    renderer
        .render_frame(&raw)
        .map_err(|error| format!("渲染宿主端 H.264 视频帧失败: {error}"))
}

#[cfg(test)]
mod tests {
    use wincast_media::{
        OpenH264Encoder, RawPixelFormat, RawVideoFrame, VideoEncoder, VideoLatencyMode,
        VideoPipelineConfig,
    };
    use wincast_protocol::{
        config::VideoCodec,
        frame::write_message,
        message::{ControlMessage, EncodedVideoFrame, ErrorCode},
        raw_frame::write_raw_bgra_frame,
    };

    use crate::test_support::raw_binary_frame_with_sequence;

    use super::*;

    #[test]
    fn client_reads_multiple_raw_bgra_frames_after_video_ready() {
        let mut bytes = Vec::new();
        for sequence_number in 0..3 {
            write_raw_bgra_frame(&mut bytes, &raw_binary_frame_with_sequence(sequence_number))
                .expect("raw binary frame should encode");
        }

        let summary = read_raw_bgra_frames(&mut bytes.as_slice(), 3)
            .expect("raw frame loop should accept three frames");

        assert_eq!(
            summary,
            RawBgraReceiveSummary {
                frames: 3,
                last_sequence_number: 2,
            }
        );
    }

    #[test]
    fn client_rejects_sequence_number_regression_in_raw_bgra_loop() {
        let mut bytes = Vec::new();
        write_raw_bgra_frame(&mut bytes, &raw_binary_frame_with_sequence(2))
            .expect("first frame should encode");
        write_raw_bgra_frame(&mut bytes, &raw_binary_frame_with_sequence(1))
            .expect("second frame should encode");

        let error = read_raw_bgra_frames(&mut bytes.as_slice(), 2)
            .expect_err("sequence regression should fail");

        assert!(error.contains("raw BGRA 视频帧序号回退"));
    }

    #[test]
    fn client_reports_raw_bgra_eof_as_video_stream_interruption() {
        let bytes = Vec::new();

        let error = read_raw_bgra_frames(&mut bytes.as_slice(), 1)
            .expect_err("eof should fail the raw BGRA frame loop");

        assert!(error.contains("视频流中断"));
    }

    #[test]
    fn client_reports_host_error_inside_raw_bgra_stream() {
        let mut bytes = Vec::new();
        write_message(
            &mut bytes,
            &ControlMessage::Error {
                code: ErrorCode::CaptureFailed,
                message: "读取后续 raw BGRA 捕获帧失败".to_owned(),
            },
        )
        .expect("host error should encode");

        let error = read_raw_bgra_frames(&mut bytes.as_slice(), 1)
            .expect_err("host error should fail the raw BGRA frame loop");

        assert!(error.contains("宿主端画面捕获失败"));
        assert!(error.contains("读取后续 raw BGRA 捕获帧失败"));
        assert!(
            !error.contains("视频流中断"),
            "host error should not be reported as raw stream interruption: {error}"
        );
    }

    #[test]
    fn client_treats_goodbye_inside_raw_bgra_stream_as_session_end() {
        let mut bytes = Vec::new();
        write_raw_bgra_frame(&mut bytes, &raw_binary_frame_with_sequence(7))
            .expect("raw frame should encode");
        write_message(&mut bytes, &ControlMessage::Goodbye).expect("goodbye should encode");

        let summary = read_raw_bgra_frames(&mut bytes.as_slice(), 2)
            .expect("goodbye should end the raw BGRA frame loop");

        assert_eq!(
            summary,
            RawBgraReceiveSummary {
                frames: 1,
                last_sequence_number: 7,
            }
        );
    }

    #[test]
    fn client_reads_and_decodes_multiple_h264_encoded_frames_until_limit() {
        let mut bytes = Vec::new();
        for frame in valid_h264_frames(3) {
            write_message(&mut bytes, &ControlMessage::EncodedVideoFrame(frame))
                .expect("encoded frame should encode");
        }

        let mut cursor = bytes.as_slice();
        let first_frame = match read_message(&mut cursor).expect("first message should decode") {
            ControlMessage::EncodedVideoFrame(frame) => frame,
            message => panic!("unexpected first message: {message:?}"),
        };

        let summary = read_h264_encoded_frames_from_first(&mut cursor, first_frame, 3)
            .expect("H.264 helper should decode three frames");

        assert_eq!(
            summary,
            EncodedVideoReceiveSummary {
                frames: 3,
                last_sequence_number: 3,
            }
        );
    }

    #[test]
    fn client_treats_goodbye_inside_h264_stream_as_session_end() {
        let mut bytes = Vec::new();
        write_message(
            &mut bytes,
            &ControlMessage::EncodedVideoFrame(valid_h264_frame(7)),
        )
        .expect("encoded frame should encode");
        write_message(&mut bytes, &ControlMessage::Goodbye).expect("goodbye should encode");

        let mut cursor = bytes.as_slice();
        let first_frame = match read_message(&mut cursor).expect("first message should decode") {
            ControlMessage::EncodedVideoFrame(frame) => frame,
            message => panic!("unexpected first message: {message:?}"),
        };

        let summary = read_h264_encoded_frames_from_first(&mut cursor, first_frame, 4)
            .expect("goodbye should end H.264 helper");

        assert_eq!(
            summary,
            EncodedVideoReceiveSummary {
                frames: 1,
                last_sequence_number: 7,
            }
        );
    }

    #[test]
    fn client_decodes_and_renders_h264_frames_with_input_feedback() {
        let frames = valid_h264_frames(2);
        let mut bytes = Vec::new();
        write_message(
            &mut bytes,
            &ControlMessage::EncodedVideoFrame(frames[1].clone()),
        )
        .expect("second encoded frame should encode");
        let mut stream = DuplexTestStream::new(bytes);
        let mut renderer = RecordingRenderer::with_input_event();

        let summary = read_h264_encoded_frames_with_renderer_from_first(
            &mut stream,
            frames[0].clone(),
            Some(2),
            &mut renderer,
        )
        .expect("H.264 renderer helper should decode and render two frames");

        assert_eq!(
            summary,
            EncodedVideoReceiveSummary {
                frames: 2,
                last_sequence_number: 2,
            }
        );
        assert_eq!(renderer.rendered_sequences, vec![1, 2]);
        assert_eq!(renderer.rendered_dimensions, vec![(16, 16), (16, 16)]);

        let mut written = stream.written.as_slice();
        let message = read_message(&mut written).expect("input event should be sent");
        assert!(matches!(
            message,
            ControlMessage::InputEvent(wincast_protocol::input::InputEvent::MouseMoveDelta {
                delta_x: 1,
                delta_y: -1
            })
        ));
    }

    #[test]
    fn client_rejects_non_h264_codec_inside_h264_stream_in_chinese() {
        let mut frame = h264_frame(1, vec![0x33]);
        frame.codec = VideoCodec::RawBgra;

        let error = read_h264_encoded_frames_from_first(&mut std::io::empty(), frame, 1)
            .expect_err("invalid codec should fail H.264 helper");

        assert!(error.contains("宿主端编码帧 codec 无效"));
        assert!(error.contains("RawBgra"));
        assert!(!error.contains("raw BGRA 流"));
    }

    #[test]
    fn client_rejects_empty_h264_payload_inside_h264_stream_in_chinese() {
        let mut bytes = Vec::new();
        write_message(
            &mut bytes,
            &ControlMessage::EncodedVideoFrame(h264_frame(1, Vec::new())),
        )
        .expect("encoded frame should encode");

        let mut cursor = bytes.as_slice();
        let first_frame = match read_message(&mut cursor).expect("first message should decode") {
            ControlMessage::EncodedVideoFrame(frame) => frame,
            message => panic!("unexpected first message: {message:?}"),
        };

        let error = read_h264_encoded_frames_from_first(&mut cursor, first_frame, 1)
            .expect_err("empty payload should fail H.264 helper");

        assert!(error.contains("宿主端 H.264 编码帧无效"));
        assert!(!error.contains("raw BGRA 流"));
    }

    #[test]
    fn client_rejects_h264_decode_failure_inside_h264_stream_in_chinese() {
        let mut bytes = Vec::new();
        write_message(
            &mut bytes,
            &ControlMessage::EncodedVideoFrame(EncodedVideoFrame {
                width: 1920,
                height: 1081,
                ..h264_frame(1, vec![0x44])
            }),
        )
        .expect("encoded frame should encode");

        let mut cursor = bytes.as_slice();
        let first_frame = match read_message(&mut cursor).expect("first message should decode") {
            ControlMessage::EncodedVideoFrame(frame) => frame,
            message => panic!("unexpected first message: {message:?}"),
        };

        let error = read_h264_encoded_frames_from_first(&mut cursor, first_frame, 1)
            .expect_err("decoded payload overflow should fail H.264 helper");

        assert!(error.contains("宿主端 H.264 编码帧解码失败"));
        assert!(!error.contains("raw BGRA 流"));
    }

    fn h264_frame(sequence_number: u64, bytes: Vec<u8>) -> EncodedVideoFrame {
        EncodedVideoFrame {
            codec: VideoCodec::H264,
            width: 2,
            height: 2,
            sequence_number,
            timestamp_ns: sequence_number * 1_000,
            keyframe: sequence_number == 0,
            bytes,
        }
    }

    fn valid_h264_frames(count: u64) -> Vec<EncodedVideoFrame> {
        let mut encoder = OpenH264Encoder::new(VideoPipelineConfig {
            codec: VideoCodec::H264,
            width: 16,
            height: 16,
            fps: 30,
            bitrate_kbps: 300,
            max_bitrate_kbps: 1_000,
            latency_mode: VideoLatencyMode::LowLatency,
        })
        .expect("OpenH264 encoder should initialize");
        (1..=count)
            .map(|sequence_number| {
                encoder
                    .encode(RawVideoFrame {
                        width: 16,
                        height: 16,
                        row_pitch: 64,
                        format: RawPixelFormat::Bgra8Unorm,
                        sequence_number,
                        timestamp_ns: sequence_number * 1_000_000,
                        bytes: &test_bgra_frame(16, 16),
                    })
                    .expect("test H.264 frame should encode")
            })
            .collect()
    }

    fn valid_h264_frame(sequence_number: u64) -> EncodedVideoFrame {
        let mut encoder = OpenH264Encoder::new(VideoPipelineConfig {
            codec: VideoCodec::H264,
            width: 16,
            height: 16,
            fps: 30,
            bitrate_kbps: 300,
            max_bitrate_kbps: 1_000,
            latency_mode: VideoLatencyMode::LowLatency,
        })
        .expect("OpenH264 encoder should initialize");
        encoder
            .encode(RawVideoFrame {
                width: 16,
                height: 16,
                row_pitch: 64,
                format: RawPixelFormat::Bgra8Unorm,
                sequence_number,
                timestamp_ns: sequence_number * 1_000_000,
                bytes: &test_bgra_frame(16, 16),
            })
            .expect("test H.264 frame should encode")
    }

    fn test_bgra_frame(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(width as usize * height as usize * 4);
        for y in 0..height {
            for x in 0..width {
                bytes.push((x * 11) as u8);
                bytes.push((y * 13) as u8);
                bytes.push(((x + y) * 7) as u8);
                bytes.push(0xff);
            }
        }
        bytes
    }

    struct RecordingRenderer {
        rendered_sequences: Vec<u64>,
        rendered_dimensions: Vec<(u32, u32)>,
        poll_count: usize,
    }

    impl RecordingRenderer {
        fn with_input_event() -> Self {
            Self {
                rendered_sequences: Vec::new(),
                rendered_dimensions: Vec::new(),
                poll_count: 0,
            }
        }
    }

    impl wincast_render::RawBgraRenderer for RecordingRenderer {
        fn render_frame(
            &mut self,
            frame: &RawBgraFrame,
        ) -> Result<(), wincast_render::RenderError> {
            frame
                .validate()
                .map_err(|error| wincast_render::RenderError::InvalidFrame(error.to_string()))?;
            self.rendered_sequences.push(frame.sequence_number);
            self.rendered_dimensions.push((frame.width, frame.height));
            Ok(())
        }

        fn poll_input(
            &mut self,
        ) -> Result<wincast_render::RenderLoopResult, wincast_render::RenderError> {
            self.poll_count += 1;
            let input_events = if self.poll_count == 1 {
                vec![wincast_protocol::input::InputEvent::MouseMoveDelta {
                    delta_x: 1,
                    delta_y: -1,
                }]
            } else {
                Vec::new()
            };
            Ok(wincast_render::RenderLoopResult {
                action: wincast_render::RenderLoopAction::Continue,
                input_events,
            })
        }
    }

    struct DuplexTestStream {
        reader: std::io::Cursor<Vec<u8>>,
        written: Vec<u8>,
    }

    impl DuplexTestStream {
        fn new(read_bytes: Vec<u8>) -> Self {
            Self {
                reader: std::io::Cursor::new(read_bytes),
                written: Vec::new(),
            }
        }
    }

    impl std::io::Read for DuplexTestStream {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            std::io::Read::read(&mut self.reader, buffer)
        }
    }

    impl std::io::Write for DuplexTestStream {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.written.extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
}
