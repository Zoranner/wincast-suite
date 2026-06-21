use std::io::{self, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use wincast_media::{
    MediaError, OpenH264Encoder, RawPixelFormat, RawVideoFrame, RawVideoFrameError, VideoEncoder,
};
use wincast_protocol::frame::{FrameError, read_message, write_message};
use wincast_protocol::handshake::{
    accept_client_hello, read_start_session, send_goodbye, send_session_ready,
};
use wincast_protocol::message::{ControlMessage, ErrorCode};

use crate::config::UnityNativeConfig;
use crate::error::{UnityNativeError, UnityNativeResult};
use crate::frame::{SubmittedFrame, WincastUnityFrameFormat};
use crate::input::from_protocol_input;
use crate::runtime::{
    mark_client_connected_for_session, mark_client_disconnected_for_session,
    push_input_for_session, record_dropped_frames_for_session, record_sent_frame_for_session,
    runtime_snapshot_for_session,
};

const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(20);
const CLIENT_READ_TIMEOUT: Duration = Duration::from_millis(100);
const FRAME_POLL_INTERVAL: Duration = Duration::from_millis(2);

#[derive(Debug)]
pub(crate) struct SessionListener {
    stop_requested: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl SessionListener {
    pub(crate) fn start(handle: u64, config: UnityNativeConfig) -> UnityNativeResult<Self> {
        let listener =
            TcpListener::bind(&config.listen_addr).map_err(UnityNativeError::ListenerBind)?;
        listener
            .set_nonblocking(true)
            .map_err(UnityNativeError::ListenerSetup)?;

        let stop_requested = Arc::new(AtomicBool::new(false));
        let worker_stop_requested = Arc::clone(&stop_requested);
        let thread_handle = thread::spawn(move || {
            run_listener(listener, handle, config, worker_stop_requested);
        });

        Ok(Self {
            stop_requested,
            handle: Some(thread_handle),
        })
    }

    pub(crate) fn stop(&mut self) {
        self.stop_requested.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for SessionListener {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run_listener(
    listener: TcpListener,
    handle: u64,
    config: UnityNativeConfig,
    stop_requested: Arc<AtomicBool>,
) {
    while !stop_requested.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => handle_client(stream, handle, &config, &stop_requested),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(ACCEPT_POLL_INTERVAL);
            }
            Err(_) => return,
        }
    }
}

fn handle_client(
    mut stream: TcpStream,
    handle: u64,
    config: &UnityNativeConfig,
    stop_requested: &AtomicBool,
) {
    if stream.set_nonblocking(false).is_err() {
        return;
    }
    if stream.set_nodelay(true).is_err() {
        return;
    }

    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    if accept_client_hello(&mut stream, &mut writer).is_err() {
        return;
    }
    if read_start_session(&mut stream).is_err() {
        return;
    }
    if send_session_ready(&mut stream, config.width, config.height).is_err() {
        return;
    }
    if mark_client_connected_for_session(handle).is_err() {
        return;
    }
    let _connected_client = ConnectedClientGuard::new(handle);
    let Ok(mut encoder) = OpenH264Encoder::new(config.video_pipeline_config()) else {
        let _ = write_control_error(
            &mut stream,
            ErrorCode::EncodingFailed,
            "H.264 encoder initialization failed".to_owned(),
        );
        return;
    };
    let mut last_encoded_frame_count = 0;
    let mut bgra_buffer = Vec::new();

    let _ = stream.set_read_timeout(Some(CLIENT_READ_TIMEOUT));
    loop {
        if stop_requested.load(Ordering::SeqCst) {
            let _ = send_goodbye(&mut stream);
            return;
        }

        if let Err(error) = write_latest_submitted_frame(
            &mut stream,
            handle,
            &mut encoder,
            &mut last_encoded_frame_count,
            &mut bgra_buffer,
        ) {
            let _ = write_control_error(
                &mut stream,
                ErrorCode::EncodingFailed,
                format!("H.264 frame encoding failed: {error}"),
            );
            return;
        }

        match drain_available_control_messages(&mut stream, handle) {
            ControlDrainResult::Continue => {}
            ControlDrainResult::StopSession => {
                let _ = send_goodbye(&mut stream);
                return;
            }
            ControlDrainResult::Disconnect => return,
        }
        thread::sleep(FRAME_POLL_INTERVAL);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ControlDrainResult {
    Continue,
    StopSession,
    Disconnect,
}

fn drain_available_control_messages(stream: &mut TcpStream, handle: u64) -> ControlDrainResult {
    loop {
        match read_message(stream) {
            Ok(ControlMessage::InputEvent(event)) => {
                if let Some(event) = from_protocol_input(event) {
                    let _ = push_input_for_session(handle, event);
                }
            }
            Ok(ControlMessage::Heartbeat) => {}
            Ok(ControlMessage::StopSession) | Ok(ControlMessage::Goodbye) => {
                return ControlDrainResult::StopSession;
            }
            Ok(_) => return ControlDrainResult::Disconnect,
            Err(error) if is_temporary_read_timeout(&error) => return ControlDrainResult::Continue,
            Err(_) => return ControlDrainResult::Disconnect,
        }
    }
}

fn write_latest_submitted_frame(
    writer: &mut impl Write,
    handle: u64,
    encoder: &mut impl VideoEncoder,
    last_encoded_frame_count: &mut u64,
    bgra_buffer: &mut Vec<u8>,
) -> UnityNativeResult<()> {
    let snapshot = runtime_snapshot_for_session(handle)?;
    if snapshot.submitted_frame_count == *last_encoded_frame_count {
        return Ok(());
    }
    let Some(frame) = snapshot.latest_frame else {
        *last_encoded_frame_count = snapshot.submitted_frame_count;
        return Ok(());
    };

    let raw_frame =
        raw_video_frame_from_submitted(&frame, snapshot.submitted_frame_count, bgra_buffer)?;
    if let Some(encoded) = encoder.encode(raw_frame)? {
        write_message(writer, &ControlMessage::EncodedVideoFrame(encoded))?;
        let dropped_count = snapshot
            .submitted_frame_count
            .saturating_sub(*last_encoded_frame_count)
            .saturating_sub(1);
        if dropped_count > 0 {
            record_dropped_frames_for_session(handle, dropped_count)?;
        }
        record_sent_frame_for_session(handle)?;
    }
    *last_encoded_frame_count = snapshot.submitted_frame_count;
    Ok(())
}

#[derive(Debug)]
struct ConnectedClientGuard {
    handle: u64,
}

impl ConnectedClientGuard {
    fn new(handle: u64) -> Self {
        Self { handle }
    }
}

impl Drop for ConnectedClientGuard {
    fn drop(&mut self) {
        let _ = mark_client_disconnected_for_session(self.handle);
    }
}

fn raw_video_frame_from_submitted<'a>(
    frame: &'a SubmittedFrame,
    sequence_number: u64,
    bgra_buffer: &'a mut Vec<u8>,
) -> Result<RawVideoFrame<'a>, MediaError> {
    match frame.metadata.format {
        WincastUnityFrameFormat::Bgra8 => Ok(RawVideoFrame {
            width: frame.metadata.width,
            height: frame.metadata.height,
            row_pitch: frame.metadata.stride_bytes,
            format: RawPixelFormat::Bgra8Unorm,
            sequence_number,
            timestamp_ns: frame.metadata.timestamp_ns,
            bytes: &frame.bytes,
        }),
        WincastUnityFrameFormat::Rgba8 => {
            rgba_to_bgra(frame, bgra_buffer)?;
            Ok(RawVideoFrame {
                width: frame.metadata.width,
                height: frame.metadata.height,
                row_pitch: frame.metadata.stride_bytes,
                format: RawPixelFormat::Bgra8Unorm,
                sequence_number,
                timestamp_ns: frame.metadata.timestamp_ns,
                bytes: bgra_buffer,
            })
        }
    }
}

fn rgba_to_bgra(frame: &SubmittedFrame, output: &mut Vec<u8>) -> Result<(), MediaError> {
    let width = frame.metadata.width as usize;
    let height = frame.metadata.height as usize;
    let stride = frame.metadata.stride_bytes as usize;
    let minimum_stride = width.checked_mul(4).ok_or(MediaError::InvalidRawFrame(
        RawVideoFrameError::RowPitchOverflow,
    ))?;
    if stride < minimum_stride {
        return Err(MediaError::InvalidRawFrame(
            RawVideoFrameError::InvalidRowPitch {
                row_pitch: frame.metadata.stride_bytes,
                min_row_pitch: minimum_stride as u32,
            },
        ));
    }
    let expected_len = stride
        .checked_mul(height)
        .ok_or(MediaError::InvalidRawFrame(
            RawVideoFrameError::PayloadLengthOverflow,
        ))?;
    if frame.bytes.len() != expected_len {
        return Err(MediaError::InvalidRawFrame(
            RawVideoFrameError::InvalidPayloadLength {
                actual: frame.bytes.len(),
                expected: expected_len,
            },
        ));
    }

    output.clear();
    output.reserve(expected_len);
    for row in frame.bytes.chunks_exact(stride) {
        let (pixels, padding) = row.split_at(minimum_stride);
        for rgba in pixels.chunks_exact(4) {
            output.extend_from_slice(&[rgba[2], rgba[1], rgba[0], rgba[3]]);
        }
        output.extend_from_slice(padding);
    }
    Ok(())
}

fn write_control_error(
    writer: &mut impl Write,
    code: ErrorCode,
    message: String,
) -> Result<(), FrameError> {
    write_message(writer, &ControlMessage::Error { code, message })
}

fn is_temporary_read_timeout(error: &FrameError) -> bool {
    matches!(
        error,
        FrameError::Io(error)
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            )
    )
}

#[cfg(test)]
mod tests {
    use std::ffi::CString;

    use wincast_media::{EncodedVideoFrame, MediaError, RawVideoFrame, VideoEncoder};
    use wincast_protocol::{
        config::VideoCodec,
        frame::read_message,
        message::{ControlMessage, EncodedVideoFrame as ProtocolEncodedVideoFrame},
    };

    use crate::{
        frame::WincastUnityFrameFormat,
        runtime::{create_runtime, get_status, shutdown_runtime, submit_frame},
    };

    use super::{FRAME_POLL_INTERVAL, write_latest_submitted_frame};

    #[test]
    fn latest_submitted_frame_is_encoded_once_and_stale_frames_are_skipped() {
        let config = CString::new(
            r#"{
                "listen_addr": "127.0.0.1:0",
                "width": 2,
                "height": 2,
                "fps": 30,
                "bitrate_kbps": 1200
            }"#,
        )
        .expect("config should not contain nul");
        let handle = create_runtime(config.as_ptr()).expect("runtime should create");
        let first = rgba_test_frame(2, 2, 1);
        let second = rgba_test_frame(2, 2, 2);
        let latest = rgba_test_frame(2, 2, 3);
        submit_test_frame(handle, &first, 10);
        submit_test_frame(handle, &second, 20);
        submit_test_frame(handle, &latest, 30);

        let mut writer = Vec::new();
        let mut encoder = RecordingEncoder::default();
        let mut last_encoded_frame_count = 0;
        let mut bgra_buffer = Vec::new();

        write_latest_submitted_frame(
            &mut writer,
            handle,
            &mut encoder,
            &mut last_encoded_frame_count,
            &mut bgra_buffer,
        )
        .expect("latest submitted frame should encode");

        assert_eq!(encoder.encoded_sequences, [3]);
        assert_eq!(encoder.encoded_timestamps, [30]);
        assert_eq!(last_encoded_frame_count, 3);
        assert_encoded_sequence(&writer, 3, 30);
        assert_eq!(get_status(handle).dropped_frame_count, 2);

        write_latest_submitted_frame(
            &mut writer,
            handle,
            &mut encoder,
            &mut last_encoded_frame_count,
            &mut bgra_buffer,
        )
        .expect("unchanged latest frame should not encode again");

        assert_eq!(encoder.encoded_sequences, [3]);
        shutdown_runtime(handle).expect("runtime should shutdown");
    }

    #[test]
    fn frame_poll_interval_prevents_empty_frame_busy_wait() {
        assert!(
            !FRAME_POLL_INTERVAL.is_zero(),
            "session loop must yield when no submitted frame is available"
        );
    }

    #[derive(Default)]
    struct RecordingEncoder {
        encoded_sequences: Vec<u64>,
        encoded_timestamps: Vec<u64>,
    }

    impl VideoEncoder for RecordingEncoder {
        fn encode(
            &mut self,
            frame: RawVideoFrame<'_>,
        ) -> Result<Option<EncodedVideoFrame>, MediaError> {
            self.encoded_sequences.push(frame.sequence_number);
            self.encoded_timestamps.push(frame.timestamp_ns);
            Ok(Some(EncodedVideoFrame {
                codec: VideoCodec::H264,
                width: frame.width,
                height: frame.height,
                sequence_number: frame.sequence_number,
                timestamp_ns: frame.timestamp_ns,
                keyframe: false,
                bytes: vec![frame.sequence_number as u8],
            }))
        }

        fn request_keyframe(&mut self) -> Result<(), MediaError> {
            Ok(())
        }
    }

    fn submit_test_frame(handle: u64, frame: &[u8], timestamp_ns: u64) {
        submit_frame(
            handle,
            frame.as_ptr(),
            2,
            2,
            2 * 4,
            WincastUnityFrameFormat::Rgba8,
            timestamp_ns,
        )
        .expect("frame should submit");
    }

    fn rgba_test_frame(width: u32, height: u32, seed: u8) -> Vec<u8> {
        let mut bytes = Vec::with_capacity((width * height * 4) as usize);
        for index in 0..(width * height) {
            bytes.push(seed);
            bytes.push((index % 255) as u8);
            bytes.push(seed.saturating_add(1));
            bytes.push(255);
        }
        bytes
    }

    fn assert_encoded_sequence(bytes: &[u8], sequence_number: u64, timestamp_ns: u64) {
        let mut cursor = bytes;
        assert_eq!(
            read_message(&mut cursor).expect("encoded message should decode"),
            ControlMessage::EncodedVideoFrame(ProtocolEncodedVideoFrame {
                codec: VideoCodec::H264,
                width: 2,
                height: 2,
                sequence_number,
                timestamp_ns,
                keyframe: false,
                bytes: vec![sequence_number as u8],
            })
        );
        assert!(cursor.is_empty());
    }
}
