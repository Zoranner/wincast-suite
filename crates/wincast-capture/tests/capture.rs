use wincast_capture::{
    CaptureError, CaptureSession, CaptureTarget, CapturedFrame, FramePixelFormat,
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
    let frame = CapturedFrame {
        width: 1280,
        height: 720,
        stride_bytes: 5120,
        pixel_format: FramePixelFormat::Bgra8Unorm,
        sequence_number: 7,
        timestamp_ns: 123_456_789,
    };

    assert_eq!(frame.width, 1280);
    assert_eq!(frame.height, 720);
    assert_eq!(frame.stride_bytes, 5120);
    assert_eq!(frame.pixel_format, FramePixelFormat::Bgra8Unorm);
    assert_eq!(frame.sequence_number, 7);
    assert_eq!(frame.timestamp_ns, 123_456_789);
}

#[test]
fn capture_errors_have_clear_chinese_messages() {
    assert_eq!(
        CaptureError::windows_capture_not_implemented().to_string(),
        "Windows 画面捕获实现未完成：尚未接入 Windows Graphics Capture"
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
