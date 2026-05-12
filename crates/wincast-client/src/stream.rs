use wincast_protocol::{
    message::{ControlMessage, RawBgraReadbackFrame},
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
        match read_raw_bgra_stream_item(reader).map_err(format_raw_bgra_read_error)? {
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
) -> Result<RawBgraStreamItem, String> {
    match read_protocol_raw_bgra_stream_item(reader).map_err(|error| error.to_string())? {
        ProtocolRawBgraStreamItem::Frame(frame) => Ok(RawBgraStreamItem::Frame(frame)),
        ProtocolRawBgraStreamItem::Control(ControlMessage::Error { code, message }) => {
            Err(format_host_error(code, message))
        }
        ProtocolRawBgraStreamItem::Control(ControlMessage::Goodbye) => {
            Ok(RawBgraStreamItem::Goodbye)
        }
        ProtocolRawBgraStreamItem::Control(message) => {
            Err(format!("宿主端 raw BGRA 流中收到无效控制消息: {message:?}"))
        }
    }
}

pub(crate) fn format_raw_bgra_read_error(error: impl std::fmt::Display) -> String {
    format!("视频流中断: 读取宿主端 raw BGRA 视频帧失败: {error}")
}

pub(crate) enum RawBgraStreamItem {
    Frame(RawBgraFrame),
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

#[cfg(test)]
mod tests {
    use wincast_protocol::{
        frame::write_message,
        message::{ControlMessage, ErrorCode},
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
}
