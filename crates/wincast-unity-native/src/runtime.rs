use std::collections::HashMap;
use std::ffi::CStr;
use std::os::raw::c_char;
use std::slice;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::config::UnityNativeConfig;
use crate::error::{UnityNativeError, UnityNativeResult};
use crate::frame::{FrameMetadata, RuntimeSnapshot, SubmittedFrame, WincastUnityFrameFormat};
use crate::input::{InputQueue, WincastUnityInputEvent};
use crate::session::SessionListener;

static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);
static RUNTIMES: OnceLock<Mutex<HashMap<u64, Runtime>>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub enum WincastUnityStatus {
    Invalid = -1,
    Created = 0,
    Started = 1,
    Stopped = 2,
    Failed = 3,
}

#[derive(Debug)]
pub(crate) struct Runtime {
    config: UnityNativeConfig,
    status: WincastUnityStatus,
    submitted_frame_count: u64,
    latest_frame: Option<SubmittedFrame>,
    input_queue: InputQueue,
    listener: Option<SessionListener>,
}

impl Runtime {
    fn new(config: UnityNativeConfig) -> Self {
        Self {
            config,
            status: WincastUnityStatus::Created,
            submitted_frame_count: 0,
            latest_frame: None,
            input_queue: InputQueue::new(),
            listener: None,
        }
    }

    fn start(&mut self, handle: u64) -> UnityNativeResult<()> {
        if self.status == WincastUnityStatus::Stopped {
            return Err(UnityNativeError::RuntimeStopped);
        }
        if self.listener.is_some() {
            self.status = WincastUnityStatus::Started;
            return Ok(());
        }

        let listener = SessionListener::start(handle, self.config.clone())?;
        self.listener = Some(listener);
        self.status = WincastUnityStatus::Started;
        Ok(())
    }

    fn shutdown(&mut self) -> Option<SessionListener> {
        self.status = WincastUnityStatus::Stopped;
        self.listener.take()
    }

    fn submit_frame(&mut self, frame: SubmittedFrame) -> UnityNativeResult<()> {
        if self.status == WincastUnityStatus::Stopped {
            return Err(UnityNativeError::RuntimeStopped);
        }

        self.submitted_frame_count = self.submitted_frame_count.saturating_add(1);
        self.latest_frame = Some(frame);
        Ok(())
    }

    fn push_input(&mut self, event: WincastUnityInputEvent) -> UnityNativeResult<()> {
        self.input_queue.push(event)
    }

    fn poll_input(&mut self, output: &mut [WincastUnityInputEvent]) -> usize {
        self.input_queue.drain_into(output)
    }

    fn snapshot(&self) -> RuntimeSnapshot {
        RuntimeSnapshot {
            submitted_frame_count: self.submitted_frame_count,
            latest_frame: self.latest_frame.clone(),
        }
    }
}

pub(crate) fn create_runtime(config_json: *const c_char) -> UnityNativeResult<u64> {
    if config_json.is_null() {
        return Err(UnityNativeError::NullConfig);
    }

    // SAFETY: The caller must pass a valid nul-terminated C string pointer.
    let config_str = unsafe { CStr::from_ptr(config_json) }
        .to_str()
        .map_err(|_| UnityNativeError::InvalidUtf8)?;
    let config = UnityNativeConfig::parse(config_str)?;

    let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
    let runtime = Runtime::new(config);

    runtimes()
        .lock()
        .expect("runtime registry lock poisoned")
        .insert(handle, runtime);

    Ok(handle)
}

pub(crate) fn start_runtime(handle: u64) -> UnityNativeResult<()> {
    with_runtime(handle, |runtime| runtime.start(handle))
}

pub(crate) fn shutdown_runtime(handle: u64) -> UnityNativeResult<()> {
    let listener = with_runtime(handle, |runtime| Ok(runtime.shutdown()))?;
    if let Some(mut listener) = listener {
        listener.stop();
    }
    Ok(())
}

pub(crate) fn submit_frame(
    handle: u64,
    frame_ptr: *const u8,
    width: u32,
    height: u32,
    stride_bytes: u32,
    format: WincastUnityFrameFormat,
    timestamp_ns: u64,
) -> UnityNativeResult<()> {
    if frame_ptr.is_null() {
        return Err(UnityNativeError::NullFrame);
    }

    let metadata = FrameMetadata::validate(width, height, stride_bytes, format, timestamp_ns)?;
    // SAFETY: The caller guarantees that `frame_ptr` is readable for
    // `metadata.byte_len` bytes during this call. Copying here gives the
    // runtime ownership and prevents later background work from observing a
    // caller-owned buffer after it is mutated or freed.
    let bytes = unsafe { slice::from_raw_parts(frame_ptr, metadata.byte_len) }.to_vec();
    let frame = SubmittedFrame { metadata, bytes };

    with_runtime(handle, |runtime| runtime.submit_frame(frame))
}

pub(crate) fn poll_input(
    handle: u64,
    output: &mut [WincastUnityInputEvent],
) -> UnityNativeResult<usize> {
    with_runtime(handle, |runtime| Ok(runtime.poll_input(output)))
}

pub(crate) fn push_input_for_session(
    handle: u64,
    event: WincastUnityInputEvent,
) -> UnityNativeResult<()> {
    with_runtime(handle, |runtime| runtime.push_input(event))
}

pub(crate) fn runtime_snapshot_for_session(handle: u64) -> UnityNativeResult<RuntimeSnapshot> {
    with_runtime(handle, |runtime| Ok(runtime.snapshot()))
}

pub(crate) fn get_status(handle: u64) -> WincastUnityStatus {
    let runtimes = runtimes().lock().expect("runtime registry lock poisoned");
    runtimes
        .get(&handle)
        .map(|runtime| runtime.status)
        .unwrap_or(WincastUnityStatus::Invalid)
}

pub fn inject_input_event_for_test(
    handle: u64,
    event: WincastUnityInputEvent,
) -> Result<(), String> {
    with_runtime(handle, |runtime| runtime.push_input(event)).map_err(|error| error.to_string())
}

pub fn runtime_snapshot_for_test(handle: u64) -> Option<RuntimeSnapshot> {
    let runtimes = runtimes().lock().expect("runtime registry lock poisoned");
    runtimes.get(&handle).map(Runtime::snapshot)
}

fn with_runtime<T>(
    handle: u64,
    operation: impl FnOnce(&mut Runtime) -> UnityNativeResult<T>,
) -> UnityNativeResult<T> {
    let mut runtimes = runtimes().lock().expect("runtime registry lock poisoned");
    let runtime = runtimes
        .get_mut(&handle)
        .ok_or(UnityNativeError::InvalidHandle)?;
    operation(runtime)
}

fn runtimes() -> &'static Mutex<HashMap<u64, Runtime>> {
    RUNTIMES.get_or_init(|| Mutex::new(HashMap::new()))
}
