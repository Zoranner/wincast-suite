use std::slice;
use std::sync::{Mutex, OnceLock};

use thiserror::Error;

pub(crate) type UnityNativeResult<T> = Result<T, UnityNativeError>;

static LAST_ERROR: OnceLock<Mutex<String>> = OnceLock::new();

#[derive(Debug, Error)]
pub(crate) enum UnityNativeError {
    #[error("config_json pointer is null")]
    NullConfig,
    #[error("config_json is not valid UTF-8")]
    InvalidUtf8,
    #[error("config_json parse failed: {0}")]
    InvalidConfig(#[from] serde_json::Error),
    #[error("config_json field `{0}` must not be empty")]
    EmptyConfigField(&'static str),
    #[error("config_json field `{0}` must be greater than zero")]
    ZeroConfigField(&'static str),
    #[error("config_json field `{field}` is invalid: {reason}")]
    InvalidConfigField {
        field: &'static str,
        reason: &'static str,
    },
    #[error("runtime handle is invalid")]
    InvalidHandle,
    #[error("runtime is stopped")]
    RuntimeStopped,
    #[error("frame_ptr pointer is null")]
    NullFrame,
    #[error("width must be greater than zero")]
    InvalidWidth,
    #[error("height must be greater than zero")]
    InvalidHeight,
    #[error("stride_bytes is too small for width and frame format")]
    InvalidStride,
    #[error("output_buffer pointer is null")]
    NullOutputBuffer,
    #[error("input output buffer length is smaller than one event")]
    InputBufferTooSmall,
    #[error("input event queue is full")]
    InputQueueFull,
    #[error("listener bind failed: {0}")]
    ListenerBind(std::io::Error),
    #[error("listener setup failed: {0}")]
    ListenerSetup(std::io::Error),
    #[error("protocol handshake failed: {0}")]
    ProtocolHandshake(#[from] wincast_protocol::handshake::HandshakeError),
    #[error("protocol frame failed: {0}")]
    ProtocolFrame(#[from] wincast_protocol::frame::FrameError),
    #[error("media failed: {0}")]
    Media(#[from] wincast_media::MediaError),
}

pub(crate) fn set_last_error(error: UnityNativeError) {
    *last_error().lock().expect("last error lock poisoned") = error.to_string();
}

pub(crate) fn clear_last_error() {
    last_error()
        .lock()
        .expect("last error lock poisoned")
        .clear();
}

pub(crate) unsafe fn write_last_error(
    buffer: *mut std::os::raw::c_char,
    buffer_len: usize,
) -> usize {
    if buffer.is_null() || buffer_len == 0 {
        return 0;
    }

    let message = {
        let error = last_error().lock().expect("last error lock poisoned");
        error.clone()
    };
    let bytes = message.as_bytes();
    let writable_len = buffer_len.saturating_sub(1);
    let copy_len = utf8_prefix_len(bytes, writable_len);

    // SAFETY: The caller provides a writable buffer of `buffer_len` bytes.
    // We return immediately above for null or zero-length buffers and only
    // write at most buffer_len - 1 bytes plus the terminating nul byte.
    let output = unsafe { slice::from_raw_parts_mut(buffer.cast::<u8>(), buffer_len) };
    output[..copy_len].copy_from_slice(&bytes[..copy_len]);
    output[copy_len] = 0;
    copy_len
}

fn last_error() -> &'static Mutex<String> {
    LAST_ERROR.get_or_init(|| Mutex::new(String::new()))
}

fn utf8_prefix_len(bytes: &[u8], max_len: usize) -> usize {
    if bytes.len() <= max_len {
        return bytes.len();
    }

    let mut len = max_len;
    while len > 0 && std::str::from_utf8(&bytes[..len]).is_err() {
        len -= 1;
    }
    len
}
