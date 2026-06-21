use std::mem;
use std::os::raw::{c_char, c_int};
use std::slice;

use crate::error::{UnityNativeError, clear_last_error, set_last_error, write_last_error};
use crate::frame::WincastUnityFrameFormat;
use crate::input::WincastUnityInputEvent;
use crate::runtime::{
    WincastUnityStatus, create_runtime, get_status, poll_input, shutdown_runtime, start_runtime,
    submit_frame,
};

const SUCCESS: c_int = 0;
const FAILURE: c_int = -1;

#[unsafe(no_mangle)]
/// Creates a phase-one Unity native runtime from a JSON configuration.
///
/// # Safety
///
/// `config_json` must point to a valid nul-terminated UTF-8 C string for the
/// duration of this call.
pub unsafe extern "C" fn wincast_unity_create(config_json: *const c_char) -> u64 {
    match create_runtime(config_json) {
        Ok(handle) => {
            clear_last_error();
            handle
        }
        Err(error) => {
            set_last_error(error);
            0
        }
    }
}

#[unsafe(no_mangle)]
/// Moves a created runtime into the started state.
///
/// # Safety
///
/// `handle` must be a value returned by `wincast_unity_create` in this process.
pub unsafe extern "C" fn wincast_unity_start(handle: u64) -> c_int {
    match start_runtime(handle) {
        Ok(()) => {
            clear_last_error();
            SUCCESS
        }
        Err(error) => {
            set_last_error(error);
            FAILURE
        }
    }
}

#[unsafe(no_mangle)]
/// Submits one raw Unity frame to the native runtime.
///
/// # Safety
///
/// `handle` must be a valid runtime handle. `frame_ptr` must point to readable
/// frame memory for at least `height * stride_bytes` bytes for the duration of
/// this call.
pub unsafe extern "C" fn wincast_unity_submit_frame(
    handle: u64,
    frame_ptr: *const u8,
    width: u32,
    height: u32,
    stride_bytes: u32,
    format: WincastUnityFrameFormat,
    timestamp_ns: u64,
) -> c_int {
    match submit_frame(
        handle,
        frame_ptr,
        width,
        height,
        stride_bytes,
        format,
        timestamp_ns,
    ) {
        Ok(()) => {
            clear_last_error();
            SUCCESS
        }
        Err(error) => {
            set_last_error(error);
            FAILURE
        }
    }
}

#[unsafe(no_mangle)]
/// Polls pending remote input events into a caller-provided event buffer.
///
/// # Safety
///
/// `handle` must be a valid runtime handle. When `buffer_len` is greater than
/// zero, `output_buffer` must point to writable memory of at least
/// `buffer_len` bytes.
pub unsafe extern "C" fn wincast_unity_poll_input(
    handle: u64,
    output_buffer: *mut u8,
    buffer_len: usize,
) -> usize {
    let result = unsafe { output_event_buffer(output_buffer, buffer_len) }
        .and_then(|output| poll_input(handle, output));

    match result {
        Ok(count) => {
            clear_last_error();
            count
        }
        Err(error) => {
            set_last_error(error);
            0
        }
    }
}

#[unsafe(no_mangle)]
/// Returns the current runtime status for a handle.
///
/// # Safety
///
/// `handle` may be any integer value. Unknown handles return
/// `WincastUnityStatus::Invalid`.
pub unsafe extern "C" fn wincast_unity_get_status(handle: u64) -> WincastUnityStatus {
    get_status(handle)
}

#[unsafe(no_mangle)]
/// Copies the current thread-shared last error message into `buffer`.
///
/// # Safety
///
/// When `buffer_len` is greater than zero, `buffer` must point to writable
/// memory of at least `buffer_len` bytes.
pub unsafe extern "C" fn wincast_unity_get_last_error(
    buffer: *mut c_char,
    buffer_len: usize,
) -> usize {
    unsafe { write_last_error(buffer, buffer_len) }
}

#[unsafe(no_mangle)]
/// Moves a runtime into the stopped state.
///
/// # Safety
///
/// `handle` must be a value returned by `wincast_unity_create` in this process.
pub unsafe extern "C" fn wincast_unity_shutdown(handle: u64) -> c_int {
    match shutdown_runtime(handle) {
        Ok(()) => {
            clear_last_error();
            SUCCESS
        }
        Err(error) => {
            set_last_error(error);
            FAILURE
        }
    }
}

unsafe fn output_event_buffer<'a>(
    output_buffer: *mut u8,
    buffer_len: usize,
) -> Result<&'a mut [WincastUnityInputEvent], UnityNativeError> {
    if buffer_len == 0 {
        return Ok(&mut []);
    }
    if output_buffer.is_null() {
        return Err(UnityNativeError::NullOutputBuffer);
    }

    let event_size = mem::size_of::<WincastUnityInputEvent>();
    let event_count = buffer_len / event_size;
    if event_count == 0 {
        return Err(UnityNativeError::InputBufferTooSmall);
    }

    // SAFETY: The caller provides a writable byte buffer. We only expose the
    // fully covered prefix as event slots and ignore trailing bytes.
    Ok(unsafe { slice::from_raw_parts_mut(output_buffer.cast(), event_count) })
}
