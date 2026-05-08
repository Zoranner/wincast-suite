use std::{collections::VecDeque, time::Duration};

use wincast_capture::{
    CaptureError, CaptureSession, CaptureTarget, CapturedFrame, FramePixelFormat,
    wait_next_frame_metadata_with,
};

#[test]
fn capture_target_describes_desktop_and_window_targets() {
    assert_eq!(CaptureTarget::Desktop.to_string(), "整个桌面");
    assert_eq!(
        CaptureTarget::Window {
            handle: 100,
            width: 1280,
            height: 720,
            title: Some("Demo".to_owned()),
        }
        .to_string(),
        "窗口 100，尺寸 1280x720，标题 Demo"
    );
}

#[test]
fn captured_frame_keeps_metadata_without_pixel_payload() {
    let frame = captured_frame();

    assert_eq!(frame.width, 1280);
    assert_eq!(frame.height, 720);
    assert_eq!(frame.stride_bytes, 5120);
    assert_eq!(frame.pixel_format, FramePixelFormat::Bgra8Unorm);
    assert_eq!(frame.sequence_number, 7);
    assert_eq!(frame.timestamp_ns, 123_456_789);
}

#[test]
fn wait_next_frame_metadata_retries_until_frame_arrives() {
    let mut frames = VecDeque::from([None, Some(captured_frame())]);

    let frame = wait_next_frame_metadata_with(Duration::from_millis(100), || {
        Ok(frames.pop_front().flatten())
    })
    .expect("frame should arrive before timeout");

    assert_eq!(frame, captured_frame());
    assert!(frames.is_empty());
}

#[test]
fn wait_next_frame_metadata_reports_timeout() {
    let error = wait_next_frame_metadata_with(Duration::from_millis(1), || Ok(None))
        .expect_err("missing frame should time out");

    assert_eq!(
        error,
        CaptureError::windows_frame_read_failed("等待 Windows 捕获首帧超时")
    );
}

#[test]
fn capture_errors_have_clear_chinese_messages() {
    assert_eq!(
        CaptureError::windows_capture_not_implemented().to_string(),
        "Windows 画面捕获实现未完成：尚未接入帧获取循环"
    );
    assert_eq!(
        CaptureError::windows_graphics_capture_unsupported().to_string(),
        "当前 Windows 系统不支持 Windows Graphics Capture"
    );
    assert_eq!(
        CaptureError::windows_graphics_capture_support_check_failed("HRESULT 0x80004005")
            .to_string(),
        "检测 Windows Graphics Capture 支持状态失败: HRESULT 0x80004005"
    );
    assert_eq!(
        CaptureError::windows_capture_item_create_failed("invalid hwnd").to_string(),
        "创建窗口捕获目标失败: invalid hwnd"
    );
    assert_eq!(
        CaptureError::windows_frame_read_failed("no frame").to_string(),
        "读取 Windows 捕获帧失败: no frame"
    );
    assert_eq!(
        CaptureError::unsupported_platform("linux").to_string(),
        "当前平台不支持画面捕获：仅 Windows 支持宿主端捕获，当前平台 linux"
    );
}

#[cfg(windows)]
#[test]
fn windows_start_returns_capture_not_implemented() {
    let error = CaptureSession::start(CaptureTarget::Desktop)
        .expect_err("windows capture should be explicit pending work");

    assert_eq!(error, CaptureError::windows_capture_not_implemented());
}

#[cfg(not(windows))]
#[test]
fn non_windows_start_returns_unsupported_platform() {
    let error = CaptureSession::start(CaptureTarget::Desktop)
        .expect_err("non-windows capture should be unsupported");

    assert_eq!(
        error,
        CaptureError::unsupported_platform(std::env::consts::OS)
    );
}

fn captured_frame() -> CapturedFrame {
    CapturedFrame {
        width: 1280,
        height: 720,
        stride_bytes: 5120,
        pixel_format: FramePixelFormat::Bgra8Unorm,
        sequence_number: 7,
        timestamp_ns: 123_456_789,
    }
}
