use std::io::Read;
use wincast_media::{OpenH264Decoder, VideoDecoder};
use wincast_protocol::{
    config::VideoCodec,
    frame::{MAX_FRAME_LEN, decode_message},
    message::{ControlMessage, EncodedVideoFrame},
};

use crate::errors::{format_host_error, prepend_context_once};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VideoStreamEnd {
    Frames(EncodedVideoReceiveSummary),
    HostProgramExited(String),
}

pub(crate) fn read_h264_encoded_frames_from_first(
    reader: &mut impl std::io::Read,
    first_frame: EncodedVideoFrame,
    frame_count: usize,
) -> Result<VideoStreamEnd, String> {
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
            NextEncodedVideoStreamItem::HostProgramExited(message) => {
                return Ok(VideoStreamEnd::HostProgramExited(message));
            }
        }
    }

    let Some(last_sequence_number) = last_sequence_number else {
        return Err("未收到 H.264 编码视频帧".to_owned());
    };

    Ok(VideoStreamEnd::Frames(EncodedVideoReceiveSummary {
        frames,
        last_sequence_number,
    }))
}

#[cfg(target_os = "linux")]
pub(crate) fn read_h264_encoded_frames_with_sdl_window_from_first(
    stream: &mut std::net::TcpStream,
    first_frame: EncodedVideoFrame,
    width: u32,
    height: u32,
) -> Result<VideoStreamEnd, String> {
    stream
        .set_read_timeout(Some(std::time::Duration::from_millis(20)))
        .map_err(|error| format!("设置客户端 H.264 视频流读取超时失败: {error}"))?;
    let mut renderer = wincast_render::SdlBgraPixelRenderer::new(wincast_render::RenderConfig {
        title: "WinCast Client".to_owned(),
        width,
        height,
        fullscreen: true,
        vsync: false,
    })
    .map_err(|error| format!("创建客户端 SDL2 窗口失败: {error}"))?;
    read_h264_encoded_frames_with_renderer_from_first(stream, first_frame, None, &mut renderer)
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn read_h264_encoded_frames_with_sdl_window_from_first(
    _stream: &mut std::net::TcpStream,
    _first_frame: EncodedVideoFrame,
    _width: u32,
    _height: u32,
) -> Result<VideoStreamEnd, String> {
    Err("当前平台不支持 SDL2 客户端窗口".to_owned())
}

#[cfg(any(test, target_os = "linux"))]
pub(crate) fn read_h264_encoded_frames_with_renderer_from_first(
    control_stream: &mut (impl std::io::Read + std::io::Write),
    first_frame: EncodedVideoFrame,
    frame_limit: Option<usize>,
    renderer: &mut impl wincast_render::BgraPixelRenderer,
) -> Result<VideoStreamEnd, String> {
    renderer
        .render_loading(&wincast_render::LoadingStatus {
            message: "正在接收宿主端首帧".to_owned(),
            tick: 75,
        })
        .map_err(|error| format!("渲染客户端加载状态失败: {error}"))?;
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
    let mut stream_reader = H264EncodedStreamReader::default();

    loop {
        if frame_limit.is_some_and(|limit| frames >= limit) {
            break;
        }
        if let Some(end) = handle_available_h264_stream_items(
            &mut stream_reader,
            control_stream,
            &mut decoder,
            renderer,
            &mut last_sequence_number,
            &mut frames,
        )? {
            return Ok(end);
        }
        let render_loop = renderer
            .poll_input()
            .map_err(|error| format!("读取客户端输入事件失败: {error}"))?;
        if render_loop.action == wincast_render::RenderLoopAction::Quit {
            let _ = write_client_control_message(control_stream, &ControlMessage::StopSession);
            break;
        }
        for input_event in render_loop.input_events {
            if write_client_control_message(
                control_stream,
                &ControlMessage::InputEvent(input_event),
            )? {
                return Ok(VideoStreamEnd::Frames(encoded_video_receive_summary(
                    frames,
                    last_sequence_number,
                )?));
            }
        }
        if write_client_control_message(control_stream, &ControlMessage::Heartbeat)? {
            break;
        }
        if let Some(end) = handle_available_h264_stream_items(
            &mut stream_reader,
            control_stream,
            &mut decoder,
            renderer,
            &mut last_sequence_number,
            &mut frames,
        )? {
            return Ok(end);
        }
    }

    let Some(last_sequence_number) = last_sequence_number else {
        return Err("未收到 H.264 编码视频帧".to_owned());
    };
    Ok(VideoStreamEnd::Frames(EncodedVideoReceiveSummary {
        frames,
        last_sequence_number,
    }))
}

#[cfg(any(test, target_os = "linux"))]
fn handle_available_h264_stream_items(
    stream_reader: &mut H264EncodedStreamReader,
    control_stream: &mut (impl Read + std::io::Write),
    decoder: &mut OpenH264Decoder,
    renderer: &mut impl wincast_render::BgraPixelRenderer,
    last_sequence_number: &mut Option<u64>,
    frames: &mut usize,
) -> Result<Option<VideoStreamEnd>, String> {
    let mut latest_frame = None;
    loop {
        match stream_reader
            .read_next(control_stream)
            .map_err(format_h264_encoded_stream_error)?
        {
            Some(NextEncodedVideoStreamItem::Frame(frame)) => {
                latest_frame = Some(decode_h264_encoded_frame(
                    decoder,
                    frame,
                    last_sequence_number,
                )?);
                *frames += 1;
            }
            Some(NextEncodedVideoStreamItem::Goodbye) => {
                render_latest_decoded_frame(renderer, latest_frame)?;
                return Ok(Some(VideoStreamEnd::Frames(encoded_video_receive_summary(
                    *frames,
                    *last_sequence_number,
                )?)));
            }
            Some(NextEncodedVideoStreamItem::HostProgramExited(message)) => {
                render_latest_decoded_frame(renderer, latest_frame)?;
                return Ok(Some(VideoStreamEnd::HostProgramExited(message)));
            }
            None => {
                render_latest_decoded_frame(renderer, latest_frame)?;
                return Ok(None);
            }
        }
    }
}

#[cfg(any(test, target_os = "linux"))]
fn render_latest_decoded_frame(
    renderer: &mut impl wincast_render::BgraPixelRenderer,
    frame: Option<wincast_render::BgraPixelFrame>,
) -> Result<(), String> {
    let Some(frame) = frame else {
        return Ok(());
    };
    renderer
        .render_frame(&frame)
        .map_err(|error| format!("渲染宿主端 H.264 视频帧失败: {error}"))
}

#[cfg(any(test, target_os = "linux"))]
fn encoded_video_receive_summary(
    frames: usize,
    last_sequence_number: Option<u64>,
) -> Result<EncodedVideoReceiveSummary, String> {
    let Some(last_sequence_number) = last_sequence_number else {
        return Err("未收到 H.264 编码视频帧".to_owned());
    };
    Ok(EncodedVideoReceiveSummary {
        frames,
        last_sequence_number,
    })
}

#[cfg(any(test, target_os = "linux"))]
fn write_client_control_message(
    writer: &mut impl std::io::Write,
    message: &ControlMessage,
) -> Result<bool, String> {
    match wincast_protocol::frame::write_message(writer, message) {
        Ok(()) => Ok(false),
        Err(error) if is_control_stream_closed(&error) => Ok(true),
        Err(error) => Err(format_client_control_write_error(message, error)),
    }
}

#[cfg(any(test, target_os = "linux"))]
fn format_client_control_write_error(
    message: &ControlMessage,
    error: impl std::fmt::Display,
) -> String {
    let action = match message {
        ControlMessage::InputEvent(_) => "发送客户端输入事件失败",
        ControlMessage::Heartbeat => "发送客户端心跳失败",
        ControlMessage::StopSession => "发送客户端停止会话消息失败",
        _ => "发送客户端控制消息失败",
    };
    format!("{action}: {error}")
}

#[cfg(any(test, target_os = "linux"))]
fn is_control_stream_closed(error: &wincast_protocol::frame::FrameError) -> bool {
    match error {
        wincast_protocol::frame::FrameError::Io(error) => matches!(
            error.kind(),
            std::io::ErrorKind::UnexpectedEof
                | std::io::ErrorKind::ConnectionAborted
                | std::io::ErrorKind::ConnectionReset
                | std::io::ErrorKind::BrokenPipe
        ),
        _ => false,
    }
}

fn read_next_h264_encoded_stream_item(
    reader: &mut impl Read,
) -> Result<NextEncodedVideoStreamItem, EncodedVideoStreamReadError> {
    let mut stream_reader = H264EncodedStreamReader::default();
    if let Some(item) = stream_reader.read_next(reader)? {
        return Ok(item);
    }
    Err(EncodedVideoStreamReadError::Interrupted(
        "暂时没有收到 H.264 编码流数据".to_owned(),
    ))
}

pub(crate) fn format_h264_encoded_read_error(error: impl std::fmt::Display) -> String {
    let message = error.to_string();
    if message.starts_with("视频流中断: ") || message.starts_with("视频流中断：") {
        return message;
    }
    prepend_context_once(
        "视频流中断",
        format!("读取宿主端 H.264 编码视频帧失败: {message}"),
    )
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

enum EncodedVideoStreamReadError {
    Host(String),
    InvalidControl(String),
    Interrupted(String),
}

enum NextEncodedVideoStreamItem {
    Frame(EncodedVideoFrame),
    HostProgramExited(String),
    Goodbye,
}

#[derive(Default)]
struct H264EncodedStreamReader {
    header: [u8; 4],
    header_read: usize,
    payload: Vec<u8>,
    payload_read: usize,
}

impl H264EncodedStreamReader {
    fn read_next(
        &mut self,
        reader: &mut impl Read,
    ) -> Result<Option<NextEncodedVideoStreamItem>, EncodedVideoStreamReadError> {
        while self.header_read < self.header.len() {
            let read = read_stream_bytes(reader, &mut self.header[self.header_read..])?;
            let Some(read) = read else {
                return Ok(None);
            };
            if read == 0 {
                if self.header_read == 0 {
                    return Ok(None);
                }
                return Err(EncodedVideoStreamReadError::Interrupted(
                    "控制消息长度头不完整".to_owned(),
                ));
            }
            self.header_read += read;
        }

        if self.payload.is_empty() {
            let len = u32::from_be_bytes(self.header) as usize;
            if len > MAX_FRAME_LEN {
                return Err(EncodedVideoStreamReadError::InvalidControl(format!(
                    "控制消息长度 {len} 超过限制 {MAX_FRAME_LEN}"
                )));
            }
            self.payload.resize(len, 0);
        }

        while self.payload_read < self.payload.len() {
            let read = read_stream_bytes(reader, &mut self.payload[self.payload_read..])?;
            let Some(read) = read else {
                return Ok(None);
            };
            if read == 0 {
                return Err(EncodedVideoStreamReadError::Interrupted(
                    "控制消息载荷不完整".to_owned(),
                ));
            }
            self.payload_read += read;
        }

        let mut frame = Vec::with_capacity(4 + self.payload.len());
        frame.extend_from_slice(&self.header);
        frame.extend_from_slice(&self.payload);
        let message = decode_message(&frame)
            .map_err(|error| EncodedVideoStreamReadError::InvalidControl(error.to_string()))?;
        self.reset();
        decode_h264_stream_message(message).map(Some)
    }

    fn reset(&mut self) {
        self.header = [0; 4];
        self.header_read = 0;
        self.payload.clear();
        self.payload_read = 0;
    }
}

fn read_stream_bytes(
    reader: &mut impl Read,
    buffer: &mut [u8],
) -> Result<Option<usize>, EncodedVideoStreamReadError> {
    match reader.read(buffer) {
        Ok(read) => Ok(Some(read)),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) =>
        {
            Ok(None)
        }
        Err(error) => Err(EncodedVideoStreamReadError::Interrupted(error.to_string())),
    }
}

fn decode_h264_stream_message(
    message: ControlMessage,
) -> Result<NextEncodedVideoStreamItem, EncodedVideoStreamReadError> {
    match message {
        ControlMessage::EncodedVideoFrame(frame) => Ok(NextEncodedVideoStreamItem::Frame(frame)),
        ControlMessage::Error { code, message }
            if code == wincast_protocol::message::ErrorCode::ProgramExited =>
        {
            Ok(NextEncodedVideoStreamItem::HostProgramExited(
                format_host_error(code, message),
            ))
        }
        ControlMessage::Error { code, message } => Err(EncodedVideoStreamReadError::Host(
            format_host_error(code, message),
        )),
        ControlMessage::Goodbye => Ok(NextEncodedVideoStreamItem::Goodbye),
        message => Err(EncodedVideoStreamReadError::InvalidControl(format!(
            "宿主端 H.264 编码流中收到无效控制消息: {message:?}"
        ))),
    }
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
    renderer: &mut impl wincast_render::BgraPixelRenderer,
    frame: EncodedVideoFrame,
    last_sequence_number: &mut Option<u64>,
) -> Result<(), String> {
    let raw = decode_h264_encoded_frame(decoder, frame, last_sequence_number)?;
    renderer
        .render_frame(&raw)
        .map_err(|error| format!("渲染宿主端 H.264 视频帧失败: {error}"))
}

#[cfg(any(test, target_os = "linux"))]
fn decode_h264_encoded_frame(
    decoder: &mut OpenH264Decoder,
    frame: EncodedVideoFrame,
    last_sequence_number: &mut Option<u64>,
) -> Result<wincast_render::BgraPixelFrame, String> {
    validate_h264_frame_sequence(&frame, last_sequence_number)?;
    let decoded = decoder
        .decode(&frame)
        .map_err(|error| format!("宿主端 H.264 编码帧解码失败: {error}"))?;
    Ok(wincast_render::BgraPixelFrame {
        width: decoded.width,
        height: decoded.height,
        row_pitch: decoded.row_pitch(),
        sequence_number: frame.sequence_number,
        timestamp_ns: frame.timestamp_ns,
        bytes: decoded.bytes.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use wincast_media::{
        OpenH264Encoder, RawPixelFormat, RawVideoFrame, VideoEncoder, VideoLatencyMode,
        VideoPipelineConfig,
    };
    use wincast_protocol::{
        config::VideoCodec,
        frame::{read_message, write_message},
        message::{ControlMessage, EncodedVideoFrame},
    };

    use super::*;

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
            VideoStreamEnd::Frames(EncodedVideoReceiveSummary {
                frames: 3,
                last_sequence_number: 3,
            })
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
            VideoStreamEnd::Frames(EncodedVideoReceiveSummary {
                frames: 1,
                last_sequence_number: 7,
            })
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
        let mut stream = DuplexTestStream::with_initial_timeout(bytes);
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
            VideoStreamEnd::Frames(EncodedVideoReceiveSummary {
                frames: 2,
                last_sequence_number: 2,
            })
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
    fn client_sends_heartbeat_from_h264_render_loop_without_input() {
        let frames = valid_h264_frames(2);
        let mut bytes = Vec::new();
        write_message(
            &mut bytes,
            &ControlMessage::EncodedVideoFrame(frames[1].clone()),
        )
        .expect("second encoded frame should encode");
        let mut stream = DuplexTestStream::with_initial_timeout(bytes);
        let mut renderer = RecordingRenderer::default();

        read_h264_encoded_frames_with_renderer_from_first(
            &mut stream,
            frames[0].clone(),
            Some(2),
            &mut renderer,
        )
        .expect("H.264 renderer helper should decode and render two frames");

        let mut written = stream.written.as_slice();
        assert_eq!(
            read_message(&mut written).expect("heartbeat should be sent"),
            ControlMessage::Heartbeat
        );
    }

    #[test]
    fn client_keeps_polling_input_and_sending_heartbeat_when_next_h264_frame_times_out() {
        let frames = valid_h264_frames(2);
        let mut bytes = Vec::new();
        write_message(
            &mut bytes,
            &ControlMessage::EncodedVideoFrame(frames[1].clone()),
        )
        .expect("second encoded frame should encode");
        let mut stream = DuplexTestStream::with_initial_timeout(bytes);
        let mut renderer = RecordingRenderer::default();

        let summary = read_h264_encoded_frames_with_renderer_from_first(
            &mut stream,
            frames[0].clone(),
            Some(2),
            &mut renderer,
        )
        .expect("temporary frame read timeout should not stop the render loop");

        assert_eq!(
            summary,
            VideoStreamEnd::Frames(EncodedVideoReceiveSummary {
                frames: 2,
                last_sequence_number: 2,
            })
        );
        assert_eq!(renderer.poll_count, 1);

        let mut written = stream.written.as_slice();
        assert_eq!(
            read_message(&mut written).expect("first heartbeat should be sent"),
            ControlMessage::Heartbeat
        );
        assert!(
            written.is_empty(),
            "only the timeout loop should send heartbeat"
        );
    }

    #[test]
    fn client_renders_latest_available_h264_frame_only() {
        let frames = valid_h264_frames(4);
        let mut bytes = Vec::new();
        for frame in &frames[1..] {
            write_message(
                &mut bytes,
                &ControlMessage::EncodedVideoFrame(frame.clone()),
            )
            .expect("encoded frame should encode");
        }
        let mut stream = DuplexTestStream::with_initial_timeout(bytes);
        let mut renderer = RecordingRenderer::default();

        let summary = read_h264_encoded_frames_with_renderer_from_first(
            &mut stream,
            frames[0].clone(),
            Some(4),
            &mut renderer,
        )
        .expect("available H.264 frames should drain and render latest frame");

        assert_eq!(
            summary,
            VideoStreamEnd::Frames(EncodedVideoReceiveSummary {
                frames: 4,
                last_sequence_number: 4,
            })
        );
        assert_eq!(renderer.rendered_sequences, vec![1, 4]);
        assert_eq!(renderer.poll_count, 1);
    }

    #[test]
    fn client_finishes_h264_render_loop_when_heartbeat_write_sees_closed_control_stream() {
        let frames = valid_h264_frames(1);
        let mut stream = DuplexTestStream::with_initial_timeout_and_write_failure(
            Vec::new(),
            std::io::ErrorKind::BrokenPipe,
            "host closed control stream",
        );
        let mut renderer = RecordingRenderer::default();

        let summary = read_h264_encoded_frames_with_renderer_from_first(
            &mut stream,
            frames[0].clone(),
            None,
            &mut renderer,
        )
        .expect("closed control stream during heartbeat should finish current stream");

        assert_eq!(
            summary,
            VideoStreamEnd::Frames(EncodedVideoReceiveSummary {
                frames: 1,
                last_sequence_number: 1,
            })
        );
        assert_eq!(renderer.poll_count, 1);
    }

    #[test]
    fn client_sends_stop_session_before_heartbeat_when_renderer_requests_quit() {
        let frames = valid_h264_frames(1);
        let mut stream = DuplexTestStream::with_initial_timeout(Vec::new());
        let mut renderer = RecordingRenderer::with_quit_on_first_poll();

        let summary = read_h264_encoded_frames_with_renderer_from_first(
            &mut stream,
            frames[0].clone(),
            None,
            &mut renderer,
        )
        .expect("renderer quit should stop H.264 render loop cleanly");

        assert_eq!(
            summary,
            VideoStreamEnd::Frames(EncodedVideoReceiveSummary {
                frames: 1,
                last_sequence_number: 1,
            })
        );

        let mut written = stream.written.as_slice();
        assert_eq!(
            read_message(&mut written).expect("stop session should be sent"),
            ControlMessage::StopSession
        );
        assert!(
            written.is_empty(),
            "heartbeat should not be sent after quit"
        );
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
    fn protocol_only_h264_reader_reports_timeout_instead_of_spinning() {
        let first_frame = valid_h264_frames(1).remove(0);
        let mut stream = DuplexTestStream::with_initial_timeout(Vec::new());

        let error = read_h264_encoded_frames_from_first(&mut stream, first_frame, 2)
            .expect_err("protocol-only reader should report temporary timeout");

        assert!(error.contains("视频流中断"));
        assert!(error.contains("读取宿主端 H.264 编码视频帧失败"));
        assert!(error.contains("暂时没有收到 H.264 编码流数据"));
    }

    #[test]
    fn h264_read_error_formatter_does_not_repeat_stream_interrupted_prefix() {
        let error = format_h264_encoded_read_error(
            "视频流中断: 读取宿主端 H.264 编码视频帧失败: 连接被重置",
        );

        assert_eq!(
            error,
            "视频流中断: 读取宿主端 H.264 编码视频帧失败: 连接被重置"
        );
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
                    .expect("test H.264 frame should not be skipped")
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
            .expect("test H.264 frame should not be skipped")
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

    #[derive(Default)]
    struct RecordingRenderer {
        rendered_sequences: Vec<u64>,
        rendered_dimensions: Vec<(u32, u32)>,
        poll_count: usize,
        emit_input: bool,
        quit_on_first_poll: bool,
    }

    impl RecordingRenderer {
        fn with_input_event() -> Self {
            Self {
                emit_input: true,
                ..Self::default()
            }
        }

        fn with_quit_on_first_poll() -> Self {
            Self {
                quit_on_first_poll: true,
                ..Self::default()
            }
        }
    }

    impl wincast_render::BgraPixelRenderer for RecordingRenderer {
        fn render_loading(
            &mut self,
            status: &wincast_render::LoadingStatus,
        ) -> Result<(), wincast_render::RenderError> {
            status.validate()?;
            Ok(())
        }

        fn render_frame(
            &mut self,
            frame: &wincast_render::BgraPixelFrame,
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
            let input_events = if self.emit_input && self.poll_count == 1 {
                vec![wincast_protocol::input::InputEvent::MouseMoveDelta {
                    delta_x: 1,
                    delta_y: -1,
                }]
            } else {
                Vec::new()
            };
            Ok(wincast_render::RenderLoopResult {
                action: if self.quit_on_first_poll && self.poll_count == 1 {
                    wincast_render::RenderLoopAction::Quit
                } else {
                    wincast_render::RenderLoopAction::Continue
                },
                input_events,
            })
        }
    }

    struct DuplexTestStream {
        reader: std::io::Cursor<Vec<u8>>,
        written: Vec<u8>,
        timeouts_before_read: usize,
        write_error: Option<(std::io::ErrorKind, &'static str)>,
    }

    impl DuplexTestStream {
        fn with_initial_timeout(read_bytes: Vec<u8>) -> Self {
            Self {
                reader: std::io::Cursor::new(read_bytes),
                written: Vec::new(),
                timeouts_before_read: 1,
                write_error: None,
            }
        }

        fn with_initial_timeout_and_write_failure(
            read_bytes: Vec<u8>,
            kind: std::io::ErrorKind,
            message: &'static str,
        ) -> Self {
            Self {
                reader: std::io::Cursor::new(read_bytes),
                written: Vec::new(),
                timeouts_before_read: 1,
                write_error: Some((kind, message)),
            }
        }
    }

    impl std::io::Read for DuplexTestStream {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            if self.timeouts_before_read > 0 {
                self.timeouts_before_read -= 1;
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "temporary frame timeout",
                ));
            }
            std::io::Read::read(&mut self.reader, buffer)
        }
    }

    impl std::io::Write for DuplexTestStream {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            if let Some((kind, message)) = self.write_error {
                return Err(std::io::Error::new(kind, message));
            }
            self.written.extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
}
