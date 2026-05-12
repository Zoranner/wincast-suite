#[cfg(any(test, target_os = "linux"))]
use std::{sync::mpsc, thread, time::Duration};

#[cfg(any(test, target_os = "linux"))]
use wincast_protocol::{frame::write_message, message::ControlMessage};

#[cfg(any(test, target_os = "linux"))]
use crate::stream::{
    RawBgraReceiveSummary, RawBgraStreamEvent, RawBgraStreamItem, format_raw_bgra_stream_error,
    read_raw_bgra_stream_item, render_raw_bgra_frame,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClientRenderMode {
    SdlWindow,
    ProtocolOnly,
}

impl ClientRenderMode {
    pub(crate) fn for_current_platform() -> Self {
        if cfg!(target_os = "linux") {
            Self::SdlWindow
        } else {
            Self::ProtocolOnly
        }
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn read_first_raw_binary_frame_with_sdl_window(
    stream: &mut std::net::TcpStream,
    width: u32,
    height: u32,
) -> Result<(), String> {
    let mut renderer = wincast_render::SdlRawBgraRenderer::new(wincast_render::RenderConfig {
        title: "WinCast Client".to_owned(),
        width,
        height,
    })
    .map_err(|error| format!("创建客户端 SDL2 窗口失败: {error}"))?;
    let frame_reader = stream
        .try_clone()
        .map_err(|error| format!("克隆 raw BGRA 视频读取端失败: {error}"))?;
    let frames = spawn_raw_bgra_frame_reader(frame_reader);
    read_raw_bgra_frames_until_renderer_quit(stream, &frames, &mut renderer).map(|_| ())
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn read_first_raw_binary_frame_with_sdl_window(
    _stream: &mut std::net::TcpStream,
    _width: u32,
    _height: u32,
) -> Result<(), String> {
    Err("当前平台不支持 SDL2 客户端窗口".to_owned())
}

#[cfg(test)]
pub(crate) fn read_raw_bgra_frames_with_renderer(
    stream: &mut impl ClientStream,
    frame_count: usize,
    renderer: &mut impl wincast_render::RawBgraRenderer,
) -> Result<RawBgraReceiveSummary, String> {
    if frame_count == 0 {
        return Err("raw BGRA 视频帧接收数量不能为 0".to_owned());
    }

    read_raw_bgra_frames_with_renderer_limit(stream, Some(frame_count), renderer)
}

#[cfg(any(test, target_os = "linux"))]
pub(crate) fn read_raw_bgra_frames_until_renderer_quit(
    control_writer: &mut impl std::io::Write,
    frames: &mpsc::Receiver<RawBgraStreamEvent>,
    renderer: &mut impl wincast_render::RawBgraRenderer,
) -> Result<RawBgraReceiveSummary, String> {
    read_raw_bgra_frames_with_renderer_loop(control_writer, frames, None, renderer)
}

#[cfg(test)]
fn read_raw_bgra_frames_with_renderer_limit(
    stream: &mut impl ClientStream,
    frame_limit: Option<usize>,
    renderer: &mut impl wincast_render::RawBgraRenderer,
) -> Result<RawBgraReceiveSummary, String> {
    let (sender, receiver) = mpsc::channel();
    let mut queued_frames = 0;
    loop {
        if frame_limit.is_some_and(|limit| queued_frames == limit) {
            break;
        }
        match read_raw_bgra_stream_item(stream) {
            Ok(RawBgraStreamItem::Frame(frame)) => sender
                .send(RawBgraStreamEvent::Frame(frame))
                .map_err(|_| "raw BGRA 测试帧通道已关闭".to_owned())?,
            Ok(RawBgraStreamItem::Goodbye) => {
                sender
                    .send(RawBgraStreamEvent::Goodbye)
                    .map_err(|_| "raw BGRA 测试帧通道已关闭".to_owned())?;
                break;
            }
            Err(error) => {
                sender
                    .send(RawBgraStreamEvent::Failed(format_raw_bgra_stream_error(
                        error,
                    )))
                    .map_err(|_| "raw BGRA 测试帧通道已关闭".to_owned())?;
                break;
            }
        }
        queued_frames += 1;
    }

    read_raw_bgra_frames_with_renderer_loop(stream, &receiver, frame_limit, renderer)
}

#[cfg(any(test, target_os = "linux"))]
fn read_raw_bgra_frames_with_renderer_loop(
    control_writer: &mut impl std::io::Write,
    frame_receiver: &mpsc::Receiver<RawBgraStreamEvent>,
    frame_limit: Option<usize>,
    renderer: &mut impl wincast_render::RawBgraRenderer,
) -> Result<RawBgraReceiveSummary, String> {
    let mut last_sequence_number = None;
    let mut frame_count = 0;
    let mut quit_requested = false;
    loop {
        if frame_limit.is_some_and(|limit| frame_count == limit) {
            break;
        }

        while let Ok(event) = frame_receiver.try_recv() {
            let frame = match event {
                RawBgraStreamEvent::Frame(frame) => frame,
                RawBgraStreamEvent::Goodbye => {
                    quit_requested = true;
                    break;
                }
                RawBgraStreamEvent::Failed(error) => return Err(error),
            };
            render_raw_bgra_frame(renderer, &frame, &mut last_sequence_number)?;
            frame_count += 1;
            if frame_limit.is_some_and(|limit| frame_count == limit) {
                break;
            }
        }

        if quit_requested {
            break;
        }

        let render_loop = renderer
            .poll_input()
            .map_err(|error| format!("读取客户端输入事件失败: {error}"))?;
        for input_event in render_loop.input_events {
            write_message(control_writer, &ControlMessage::InputEvent(input_event))
                .map_err(|error| format!("发送客户端输入事件失败: {error}"))?;
        }

        if render_loop.action == wincast_render::RenderLoopAction::Quit {
            let _ = write_message(control_writer, &ControlMessage::StopSession);
            quit_requested = true;
            break;
        }

        if frame_limit.is_some_and(|limit| frame_count == limit) {
            break;
        }

        thread::sleep(Duration::from_millis(8));
    }

    let last_sequence_number = match last_sequence_number {
        Some(sequence_number) => sequence_number,
        None if quit_requested => 0,
        None => return Err("未收到 raw BGRA 视频帧".to_owned()),
    };
    Ok(RawBgraReceiveSummary {
        frames: frame_count,
        last_sequence_number,
    })
}

#[cfg(target_os = "linux")]
fn spawn_raw_bgra_frame_reader(
    mut reader: impl std::io::Read + Send + 'static,
) -> mpsc::Receiver<RawBgraStreamEvent> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        loop {
            let event = match read_raw_bgra_stream_item(&mut reader) {
                Ok(RawBgraStreamItem::Frame(frame)) => RawBgraStreamEvent::Frame(frame),
                Ok(RawBgraStreamItem::Goodbye) => RawBgraStreamEvent::Goodbye,
                Err(error) => RawBgraStreamEvent::Failed(format_raw_bgra_stream_error(error)),
            };
            let should_stop = !matches!(event, RawBgraStreamEvent::Frame(_));
            if sender.send(event).is_err() || should_stop {
                break;
            }
        }
    });
    receiver
}

#[cfg(test)]
pub(crate) trait ClientStream: std::io::Read + std::io::Write {}

#[cfg(test)]
impl<T: std::io::Read + std::io::Write> ClientStream for T {}

#[cfg(test)]
mod tests {
    use wincast_protocol::{
        frame::read_message,
        input::{ButtonState, InputEvent, Modifiers},
        message::ControlMessage,
        raw_frame::{RawBgraFrame, write_raw_bgra_frame},
    };
    use wincast_render::{RawBgraRenderer, RenderError, RenderLoopAction, RenderLoopResult};

    use crate::test_support::{
        DuplexBuffer, FailingWriteStream, queued_raw_bgra_frames, raw_binary_frame,
        raw_binary_frame_with_sequence,
    };

    use super::*;

    #[test]
    fn client_renders_raw_bgra_frames_after_video_ready() {
        let mut bytes = Vec::new();
        for sequence_number in 0..2 {
            write_raw_bgra_frame(&mut bytes, &raw_binary_frame_with_sequence(sequence_number))
                .expect("raw binary frame should encode");
        }
        let mut stream = DuplexBuffer::new(bytes);
        let mut renderer = RecordingRenderer::default();

        let summary = read_raw_bgra_frames_with_renderer(&mut stream, 2, &mut renderer)
            .expect("raw frame loop should render frames");

        assert_eq!(
            summary,
            RawBgraReceiveSummary {
                frames: 2,
                last_sequence_number: 1,
            }
        );
        assert_eq!(renderer.rendered_sequences, vec![0, 1]);
    }

    #[test]
    fn client_continues_rendering_raw_bgra_frames_until_renderer_quits() {
        let mut stream = DuplexBuffer::new(Vec::new());
        let mut renderer = RecordingRenderer {
            actions: Vec::from([
                RenderLoopAction::Continue,
                RenderLoopAction::Continue,
                RenderLoopAction::Quit,
            ]),
            ..Default::default()
        };

        let frames = queued_raw_bgra_frames([
            raw_binary_frame_with_sequence(0),
            raw_binary_frame_with_sequence(1),
            raw_binary_frame_with_sequence(2),
        ]);

        let summary = read_raw_bgra_frames_until_renderer_quit(&mut stream, &frames, &mut renderer)
            .expect("raw frame loop should continue until renderer quits");

        assert_eq!(
            summary,
            RawBgraReceiveSummary {
                frames: 3,
                last_sequence_number: 2,
            }
        );
        assert_eq!(renderer.rendered_sequences, vec![0, 1, 2]);
    }

    #[test]
    fn client_sends_stop_session_when_renderer_quits() {
        let mut stream = DuplexBuffer::new(Vec::new());
        let mut renderer = RecordingRenderer {
            actions: Vec::from([RenderLoopAction::Quit]),
            ..Default::default()
        };
        let frames = queued_raw_bgra_frames([raw_binary_frame()]);

        read_raw_bgra_frames_until_renderer_quit(&mut stream, &frames, &mut renderer)
            .expect("quit should end frame loop cleanly");

        let mut written = stream.written.as_slice();
        assert_eq!(
            read_message(&mut written).expect("stop session message should decode"),
            ControlMessage::StopSession
        );
    }

    #[test]
    fn client_still_exits_when_stop_session_send_fails_after_renderer_quit() {
        let mut stream = FailingWriteStream::new(Vec::new());
        let mut renderer = RecordingRenderer {
            actions: Vec::from([RenderLoopAction::Quit]),
            ..Default::default()
        };
        let frames = queued_raw_bgra_frames([raw_binary_frame()]);

        let summary = read_raw_bgra_frames_until_renderer_quit(&mut stream, &frames, &mut renderer)
            .expect("stop session send failure should not block local quit");

        assert_eq!(
            summary,
            RawBgraReceiveSummary {
                frames: 1,
                last_sequence_number: 0,
            }
        );
    }

    #[test]
    fn client_can_quit_even_when_no_new_raw_frame_is_available() {
        let mut stream = DuplexBuffer::new(Vec::new());
        let frames = queued_raw_bgra_frames([raw_binary_frame()]);
        let mut renderer = RecordingRenderer {
            actions: Vec::from([RenderLoopAction::Continue, RenderLoopAction::Quit]),
            ..Default::default()
        };

        read_raw_bgra_frames_until_renderer_quit(&mut stream, &frames, &mut renderer)
            .expect("quit should not wait for another raw frame");

        let mut written = stream.written.as_slice();
        assert_eq!(
            read_message(&mut written).expect("stop session message should decode"),
            ControlMessage::StopSession
        );
        assert_eq!(renderer.rendered_sequences, vec![0]);
    }

    #[test]
    fn client_can_quit_before_first_raw_frame_arrives() {
        let mut stream = DuplexBuffer::new(Vec::new());
        let (_sender, frames) = std::sync::mpsc::channel();
        let mut renderer = RecordingRenderer {
            actions: Vec::from([RenderLoopAction::Quit]),
            ..Default::default()
        };

        let summary = read_raw_bgra_frames_until_renderer_quit(&mut stream, &frames, &mut renderer)
            .expect("quit before first frame should end cleanly");

        assert_eq!(
            summary,
            RawBgraReceiveSummary {
                frames: 0,
                last_sequence_number: 0,
            }
        );
        let mut written = stream.written.as_slice();
        assert_eq!(
            read_message(&mut written).expect("stop session message should decode"),
            ControlMessage::StopSession
        );
        assert!(renderer.rendered_sequences.is_empty());
    }

    #[test]
    fn client_sends_renderer_input_events_after_rendering_raw_frame() {
        let mut bytes = Vec::new();
        write_raw_bgra_frame(&mut bytes, &raw_binary_frame())
            .expect("raw binary frame should encode");
        let mut stream = DuplexBuffer::new(bytes);
        let mut renderer = RecordingRenderer {
            input_batches: Vec::from([vec![InputEvent::Key {
                code: 65,
                state: ButtonState::Pressed,
                modifiers: Modifiers {
                    shift: true,
                    ctrl: false,
                    alt: false,
                    logo: false,
                },
            }]]),
            ..Default::default()
        };

        read_raw_bgra_frames_with_renderer(&mut stream, 1, &mut renderer)
            .expect("input event should send after rendering frame");

        let mut written = stream.written.as_slice();
        assert_eq!(
            read_message(&mut written).expect("input message should decode"),
            ControlMessage::InputEvent(InputEvent::Key {
                code: 65,
                state: ButtonState::Pressed,
                modifiers: Modifiers {
                    shift: true,
                    ctrl: false,
                    alt: false,
                    logo: false,
                },
            })
        );
    }

    #[test]
    fn client_reports_input_send_errors_in_chinese() {
        let mut bytes = Vec::new();
        write_raw_bgra_frame(&mut bytes, &raw_binary_frame())
            .expect("raw binary frame should encode");
        let mut stream = FailingWriteStream::new(bytes);
        let mut renderer = RecordingRenderer {
            input_batches: Vec::from([vec![InputEvent::MouseWheel {
                delta_x: 0,
                delta_y: 1,
            }]]),
            ..Default::default()
        };

        let error = read_raw_bgra_frames_with_renderer(&mut stream, 1, &mut renderer)
            .expect_err("input send failure should fail frame loop");

        assert!(error.contains("发送客户端输入事件失败"));
    }

    #[test]
    fn client_reports_renderer_errors_in_chinese() {
        let mut bytes = Vec::new();
        write_raw_bgra_frame(&mut bytes, &raw_binary_frame())
            .expect("raw binary frame should encode");
        let mut stream = DuplexBuffer::new(bytes);
        let mut renderer = FailingRenderer;

        let error = read_raw_bgra_frames_with_renderer(&mut stream, 1, &mut renderer)
            .expect_err("renderer failure should fail client frame loop");

        assert!(error.contains("渲染宿主端 raw BGRA 视频帧失败"));
        assert!(error.contains("测试渲染失败"));
    }

    #[test]
    fn current_platform_uses_protocol_only_render_mode_outside_linux() {
        if !cfg!(target_os = "linux") {
            assert_eq!(
                ClientRenderMode::for_current_platform(),
                ClientRenderMode::ProtocolOnly
            );
        }
    }

    #[derive(Default)]
    struct RecordingRenderer {
        rendered_sequences: Vec<u64>,
        input_batches: Vec<Vec<InputEvent>>,
        actions: Vec<RenderLoopAction>,
    }

    impl RawBgraRenderer for RecordingRenderer {
        fn render_frame(&mut self, frame: &RawBgraFrame) -> Result<(), RenderError> {
            self.rendered_sequences.push(frame.sequence_number);
            Ok(())
        }

        fn poll_input(&mut self) -> Result<RenderLoopResult, RenderError> {
            Ok(RenderLoopResult {
                action: if self.actions.is_empty() {
                    RenderLoopAction::Continue
                } else {
                    self.actions.remove(0)
                },
                input_events: if self.input_batches.is_empty() {
                    Vec::new()
                } else {
                    self.input_batches.remove(0)
                },
            })
        }
    }

    struct FailingRenderer;

    impl RawBgraRenderer for FailingRenderer {
        fn render_frame(&mut self, _frame: &RawBgraFrame) -> Result<(), RenderError> {
            Err(RenderError::Backend("测试渲染失败".to_owned()))
        }

        fn poll_input(&mut self) -> Result<RenderLoopResult, RenderError> {
            Ok(RenderLoopResult {
                action: RenderLoopAction::Continue,
                input_events: Vec::new(),
            })
        }
    }
}
