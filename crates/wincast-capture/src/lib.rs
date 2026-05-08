use std::{
    fmt, thread,
    time::{Duration, Instant},
};

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureTarget {
    Desktop,
    Window {
        handle: isize,
        width: u32,
        height: u32,
        title: Option<String>,
    },
}

impl fmt::Display for CaptureTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Desktop => formatter.write_str("整个桌面"),
            Self::Window {
                handle,
                width,
                height,
                title,
            } => {
                write!(formatter, "窗口 {handle}，尺寸 {width}x{height}")?;
                if let Some(title) = title {
                    write!(formatter, "，标题 {title}")?;
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FramePixelFormat {
    Bgra8Unorm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapturedFrame {
    pub width: u32,
    pub height: u32,
    pub stride_bytes: u32,
    pub pixel_format: FramePixelFormat,
    pub sequence_number: u64,
    pub timestamp_ns: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapturedTextureMetadata {
    pub frame: CapturedFrame,
    pub texture_width: u32,
    pub texture_height: u32,
    pub mip_levels: u32,
    pub array_size: u32,
    pub sample_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedBgraFrame {
    pub metadata: CapturedTextureMetadata,
    pub row_pitch: u32,
    pub bytes: Vec<u8>,
}

#[derive(Debug)]
pub struct CaptureSession {
    target: CaptureTarget,
    #[cfg(windows)]
    state: windows_impl::WindowsCaptureState,
}

impl CaptureSession {
    pub fn start(target: CaptureTarget) -> Result<Self, CaptureError> {
        start_platform_capture(target)
    }

    pub fn target(&self) -> &CaptureTarget {
        &self.target
    }

    pub fn is_active(&self) -> bool {
        #[cfg(windows)]
        {
            self.state.is_active()
        }
        #[cfg(not(windows))]
        {
            false
        }
    }

    pub fn try_next_frame_metadata(&mut self) -> Result<Option<CapturedFrame>, CaptureError> {
        #[cfg(windows)]
        {
            self.state.try_next_frame_metadata()
        }
        #[cfg(not(windows))]
        {
            Ok(None)
        }
    }

    pub fn wait_next_frame_metadata(
        &mut self,
        timeout: Duration,
    ) -> Result<CapturedFrame, CaptureError> {
        wait_next_capture_result_with(timeout, || self.try_next_frame_metadata())
    }

    pub fn try_next_texture_metadata(
        &mut self,
    ) -> Result<Option<CapturedTextureMetadata>, CaptureError> {
        #[cfg(windows)]
        {
            self.state.try_next_texture_metadata()
        }
        #[cfg(not(windows))]
        {
            Ok(None)
        }
    }

    pub fn try_next_bgra_frame(&mut self) -> Result<Option<CapturedBgraFrame>, CaptureError> {
        #[cfg(windows)]
        {
            self.state.try_next_bgra_frame()
        }
        #[cfg(not(windows))]
        {
            Ok(None)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CaptureError {
    #[error("Windows 画面捕获实现未完成：尚未接入帧获取循环")]
    WindowsCaptureNotImplemented,
    #[error("当前 Windows 系统不支持 Windows Graphics Capture")]
    WindowsGraphicsCaptureUnsupported,
    #[error("检测 Windows Graphics Capture 支持状态失败: {0}")]
    WindowsGraphicsCaptureSupportCheckFailed(String),
    #[error("创建窗口捕获目标失败: {0}")]
    WindowsCaptureItemCreateFailed(String),
    #[error("初始化 Direct3D 捕获设备失败: {0}")]
    WindowsD3dInitializationFailed(String),
    #[error("创建 Windows 捕获会话失败: {0}")]
    WindowsCaptureSessionCreateFailed(String),
    #[error("启动 Windows 捕获会话失败: {0}")]
    WindowsCaptureSessionStartFailed(String),
    #[error("读取 Windows 捕获帧失败: {0}")]
    WindowsFrameReadFailed(String),
    #[error("当前平台不支持画面捕获：仅 Windows 支持宿主端捕获，当前平台 {platform}")]
    UnsupportedPlatform { platform: String },
}

impl CaptureError {
    pub fn windows_capture_not_implemented() -> Self {
        Self::WindowsCaptureNotImplemented
    }

    pub fn windows_graphics_capture_unsupported() -> Self {
        Self::WindowsGraphicsCaptureUnsupported
    }

    pub fn windows_graphics_capture_support_check_failed(error: impl Into<String>) -> Self {
        Self::WindowsGraphicsCaptureSupportCheckFailed(error.into())
    }

    pub fn windows_capture_item_create_failed(error: impl Into<String>) -> Self {
        Self::WindowsCaptureItemCreateFailed(error.into())
    }

    pub fn windows_d3d_initialization_failed(error: impl Into<String>) -> Self {
        Self::WindowsD3dInitializationFailed(error.into())
    }

    pub fn windows_capture_session_create_failed(error: impl Into<String>) -> Self {
        Self::WindowsCaptureSessionCreateFailed(error.into())
    }

    pub fn windows_capture_session_start_failed(error: impl Into<String>) -> Self {
        Self::WindowsCaptureSessionStartFailed(error.into())
    }

    pub fn windows_frame_read_failed(error: impl Into<String>) -> Self {
        Self::WindowsFrameReadFailed(error.into())
    }

    pub fn unsupported_platform(platform: impl Into<String>) -> Self {
        Self::UnsupportedPlatform {
            platform: platform.into(),
        }
    }
}

pub fn wait_next_frame_metadata_with(
    timeout: Duration,
    mut try_next_frame: impl FnMut() -> Result<Option<CapturedFrame>, CaptureError>,
) -> Result<CapturedFrame, CaptureError> {
    wait_next_capture_result_with(timeout, &mut try_next_frame)
}

pub fn wait_next_capture_result_with<T>(
    timeout: Duration,
    mut try_next_frame: impl FnMut() -> Result<Option<T>, CaptureError>,
) -> Result<T, CaptureError> {
    let deadline = Instant::now() + timeout;

    loop {
        if let Some(frame) = try_next_frame()? {
            return Ok(frame);
        }

        if Instant::now() >= deadline {
            return Err(CaptureError::windows_frame_read_failed(
                "等待 Windows 捕获首帧超时",
            ));
        }

        thread::sleep(Duration::from_millis(16));
    }
}

#[cfg(windows)]
fn start_platform_capture(target: CaptureTarget) -> Result<CaptureSession, CaptureError> {
    let state = windows_impl::start_windows_capture(&target)?;

    Ok(CaptureSession { target, state })
}

#[cfg(windows)]
mod windows_impl {
    use super::{CaptureError, CaptureTarget};
    use std::slice;
    use windows::{
        Graphics::{
            Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession},
            DirectX::{Direct3D11::IDirect3DDevice, DirectXPixelFormat},
            SizeInt32,
        },
        Win32::{
            Foundation::{HMODULE, HWND},
            Graphics::{
                Direct3D::{
                    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_9_1,
                    D3D_FEATURE_LEVEL_9_2, D3D_FEATURE_LEVEL_9_3, D3D_FEATURE_LEVEL_10_0,
                    D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
                },
                Direct3D11::{
                    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
                    D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC,
                    D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext,
                    ID3D11Texture2D,
                },
                Dxgi::IDXGIDevice,
            },
            System::WinRT::{
                Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess},
                Graphics::Capture::IGraphicsCaptureItemInterop,
            },
        },
        core::{Interface, factory},
    };

    #[derive(Debug)]
    pub(crate) struct WindowsCaptureState {
        d3d_device: ID3D11Device,
        d3d_context: ID3D11DeviceContext,
        direct3d_device: IDirect3DDevice,
        frame_pool: Direct3D11CaptureFramePool,
        _session: GraphicsCaptureSession,
        frame_pool_size: FramePoolSize,
        sequence_number: u64,
    }

    impl WindowsCaptureState {
        pub(crate) fn is_active(&self) -> bool {
            true
        }

        pub(crate) fn try_next_frame_metadata(
            &mut self,
        ) -> Result<Option<super::CapturedFrame>, CaptureError> {
            let frame = match self.frame_pool.TryGetNextFrame() {
                Ok(frame) => frame,
                Err(error) if error.code().0 == 0 => return Ok(None),
                Err(error) => {
                    return Err(CaptureError::windows_frame_read_failed(error.to_string()));
                }
            };
            self.recreate_frame_pool_if_needed(
                frame
                    .ContentSize()
                    .map_err(|error| CaptureError::windows_frame_read_failed(error.to_string()))?,
            )?;
            let sequence_number = self.sequence_number;
            self.sequence_number = self.sequence_number.saturating_add(1);

            Ok(Some(captured_frame_metadata(&frame, sequence_number)?))
        }

        pub(crate) fn try_next_texture_metadata(
            &mut self,
        ) -> Result<Option<super::CapturedTextureMetadata>, CaptureError> {
            Ok(self.try_next_texture()?.map(|(metadata, _, _)| metadata))
        }

        pub(crate) fn try_next_bgra_frame(
            &mut self,
        ) -> Result<Option<super::CapturedBgraFrame>, CaptureError> {
            let Some((metadata, texture, texture_desc)) = self.try_next_texture()? else {
                return Ok(None);
            };

            let readback =
                readback_bgra_frame(&self.d3d_device, &self.d3d_context, &texture, &texture_desc)?;

            Ok(Some(super::CapturedBgraFrame {
                metadata,
                row_pitch: readback.row_pitch,
                bytes: readback.bytes,
            }))
        }

        fn try_next_texture(&mut self) -> Result<Option<FrameTexture>, CaptureError> {
            let frame = match self.frame_pool.TryGetNextFrame() {
                Ok(frame) => frame,
                Err(error) if error.code().0 == 0 => return Ok(None),
                Err(error) => {
                    return Err(CaptureError::windows_frame_read_failed(error.to_string()));
                }
            };
            self.recreate_frame_pool_if_needed(
                frame
                    .ContentSize()
                    .map_err(|error| CaptureError::windows_frame_read_failed(error.to_string()))?,
            )?;
            let metadata = captured_frame_metadata(&frame, self.sequence_number)?;
            self.sequence_number = self.sequence_number.saturating_add(1);
            let (texture, texture_desc) = frame_texture(&frame)?;
            let metadata = captured_texture_metadata(metadata, &texture_desc);

            Ok(Some((metadata, texture, texture_desc)))
        }

        fn recreate_frame_pool_if_needed(&mut self, size: SizeInt32) -> Result<(), CaptureError> {
            let Some(new_size) = self
                .frame_pool_size
                .update_if_changed(size.Width, size.Height)
            else {
                return Ok(());
            };

            self.frame_pool
                .Recreate(
                    &self.direct3d_device,
                    DirectXPixelFormat::B8G8R8A8UIntNormalized,
                    1,
                    SizeInt32 {
                        Width: new_size.width,
                        Height: new_size.height,
                    },
                )
                .map_err(|error| CaptureError::windows_frame_read_failed(error.to_string()))
        }
    }

    type FrameTexture = (
        super::CapturedTextureMetadata,
        ID3D11Texture2D,
        D3D11_TEXTURE2D_DESC,
    );

    struct BgraReadback {
        row_pitch: u32,
        bytes: Vec<u8>,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct FramePoolSize {
        width: i32,
        height: i32,
    }

    impl FramePoolSize {
        fn from_size(size: SizeInt32) -> Self {
            Self {
                width: size.Width,
                height: size.Height,
            }
        }

        fn update_if_changed(&mut self, width: i32, height: i32) -> Option<Self> {
            if width <= 0 || height <= 0 || (self.width == width && self.height == height) {
                return None;
            }

            *self = Self { width, height };
            Some(*self)
        }
    }

    fn captured_frame_metadata(
        frame: &windows::Graphics::Capture::Direct3D11CaptureFrame,
        sequence_number: u64,
    ) -> Result<super::CapturedFrame, CaptureError> {
        let size = frame
            .ContentSize()
            .map_err(|error| CaptureError::windows_frame_read_failed(error.to_string()))?;
        let timestamp = frame
            .SystemRelativeTime()
            .map_err(|error| CaptureError::windows_frame_read_failed(error.to_string()))?;

        Ok(super::CapturedFrame {
            width: size.Width.max(0) as u32,
            height: size.Height.max(0) as u32,
            stride_bytes: size.Width.max(0) as u32 * 4,
            pixel_format: super::FramePixelFormat::Bgra8Unorm,
            sequence_number,
            timestamp_ns: timestamp.Duration.max(0) as u64 * 100,
        })
    }

    fn frame_texture(
        frame: &windows::Graphics::Capture::Direct3D11CaptureFrame,
    ) -> Result<(ID3D11Texture2D, D3D11_TEXTURE2D_DESC), CaptureError> {
        let surface = frame
            .Surface()
            .map_err(|error| CaptureError::windows_frame_read_failed(error.to_string()))?;
        let dxgi_access: IDirect3DDxgiInterfaceAccess = surface
            .cast()
            .map_err(|error| CaptureError::windows_frame_read_failed(error.to_string()))?;
        let texture = unsafe { dxgi_access.GetInterface::<ID3D11Texture2D>() }
            .map_err(|error| CaptureError::windows_frame_read_failed(error.to_string()))?;
        let mut texture_desc = D3D11_TEXTURE2D_DESC::default();
        unsafe {
            texture.GetDesc(&mut texture_desc);
        }

        Ok((texture, texture_desc))
    }

    fn captured_texture_metadata(
        frame: super::CapturedFrame,
        texture_desc: &D3D11_TEXTURE2D_DESC,
    ) -> super::CapturedTextureMetadata {
        super::CapturedTextureMetadata {
            frame,
            texture_width: texture_desc.Width,
            texture_height: texture_desc.Height,
            mip_levels: texture_desc.MipLevels,
            array_size: texture_desc.ArraySize,
            sample_count: texture_desc.SampleDesc.Count,
        }
    }

    fn readback_bgra_frame(
        d3d_device: &ID3D11Device,
        d3d_context: &ID3D11DeviceContext,
        texture: &ID3D11Texture2D,
        texture_desc: &D3D11_TEXTURE2D_DESC,
    ) -> Result<BgraReadback, CaptureError> {
        let staging_desc = D3D11_TEXTURE2D_DESC {
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
            ..*texture_desc
        };
        let mut staging = None;
        unsafe {
            d3d_device
                .CreateTexture2D(&staging_desc, None, Some(&mut staging))
                .map_err(|error| CaptureError::windows_frame_read_failed(error.to_string()))?;
        }
        let staging = staging.ok_or_else(|| {
            CaptureError::windows_frame_read_failed("CreateTexture2D 未返回 staging texture")
        })?;

        unsafe {
            d3d_context.CopyResource(&staging, texture);
        }

        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe {
            d3d_context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .map_err(|error| CaptureError::windows_frame_read_failed(error.to_string()))?;
        }

        let byte_len = texture_desc.Height as usize * mapped.RowPitch as usize;
        let bytes = unsafe { slice::from_raw_parts(mapped.pData.cast::<u8>(), byte_len) }.to_vec();

        unsafe {
            d3d_context.Unmap(&staging, 0);
        }

        Ok(BgraReadback {
            row_pitch: mapped.RowPitch,
            bytes,
        })
    }

    pub(crate) fn start_windows_capture(
        target: &CaptureTarget,
    ) -> Result<WindowsCaptureState, CaptureError> {
        let supported = GraphicsCaptureSession::IsSupported().map_err(|error| {
            CaptureError::windows_graphics_capture_support_check_failed(error.to_string())
        })?;
        if !supported {
            return Err(CaptureError::windows_graphics_capture_unsupported());
        }

        let item = match target {
            CaptureTarget::Window { handle, .. } => create_window_capture_item(*handle)?,
            CaptureTarget::Desktop => return Err(CaptureError::windows_capture_not_implemented()),
        };

        let d3d_device = create_d3d_device()?;
        let d3d_context = unsafe { d3d_device.GetImmediateContext() }
            .map_err(|error| CaptureError::windows_d3d_initialization_failed(error.to_string()))?;
        let direct3d_device = create_direct3d_device(&d3d_device)?;
        let frame_pool_size = item.Size().map_err(|error| {
            CaptureError::windows_capture_session_create_failed(error.to_string())
        })?;
        let frame_pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &direct3d_device,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            1,
            frame_pool_size,
        )
        .map_err(|error| CaptureError::windows_capture_session_create_failed(error.to_string()))?;
        let session = frame_pool.CreateCaptureSession(&item).map_err(|error| {
            CaptureError::windows_capture_session_create_failed(error.to_string())
        })?;
        session.StartCapture().map_err(|error| {
            CaptureError::windows_capture_session_start_failed(error.to_string())
        })?;

        Ok(WindowsCaptureState {
            d3d_device,
            d3d_context,
            direct3d_device,
            frame_pool,
            _session: session,
            frame_pool_size: FramePoolSize::from_size(frame_pool_size),
            sequence_number: 0,
        })
    }

    fn create_window_capture_item(handle: isize) -> Result<GraphicsCaptureItem, CaptureError> {
        let hwnd = HWND(handle as *mut core::ffi::c_void);
        let interop = factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
            .map_err(|error| CaptureError::windows_capture_item_create_failed(error.to_string()))?;
        unsafe { interop.CreateForWindow(hwnd) }
            .map_err(|error| CaptureError::windows_capture_item_create_failed(error.to_string()))
    }

    fn create_d3d_device() -> Result<ID3D11Device, CaptureError> {
        let feature_flags = [
            D3D_FEATURE_LEVEL_11_1,
            D3D_FEATURE_LEVEL_11_0,
            D3D_FEATURE_LEVEL_10_1,
            D3D_FEATURE_LEVEL_10_0,
            D3D_FEATURE_LEVEL_9_3,
            D3D_FEATURE_LEVEL_9_2,
            D3D_FEATURE_LEVEL_9_1,
        ];
        let mut d3d_device = None;
        let mut feature_level = D3D_FEATURE_LEVEL::default();
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&feature_flags),
                D3D11_SDK_VERSION,
                Some(&mut d3d_device),
                Some(&mut feature_level),
                None,
            )
        }
        .map_err(|error| CaptureError::windows_d3d_initialization_failed(error.to_string()))?;

        if feature_level.0 < D3D_FEATURE_LEVEL_11_0.0 {
            return Err(CaptureError::windows_d3d_initialization_failed(
                "Direct3D feature level 低于 11.0",
            ));
        }

        d3d_device.ok_or_else(|| {
            CaptureError::windows_d3d_initialization_failed("D3D11CreateDevice 未返回设备")
        })
    }

    fn create_direct3d_device(d3d_device: &ID3D11Device) -> Result<IDirect3DDevice, CaptureError> {
        let dxgi_device: IDXGIDevice = d3d_device
            .cast()
            .map_err(|error| CaptureError::windows_d3d_initialization_failed(error.to_string()))?;
        let inspectable = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi_device) }
            .map_err(|error| CaptureError::windows_d3d_initialization_failed(error.to_string()))?;
        inspectable
            .cast()
            .map_err(|error| CaptureError::windows_d3d_initialization_failed(error.to_string()))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn frame_pool_size_updates_only_for_positive_changes() {
            let mut size = FramePoolSize {
                width: 1280,
                height: 720,
            };

            assert_eq!(size.update_if_changed(1280, 720), None);
            assert_eq!(size.update_if_changed(0, 720), None);
            assert_eq!(size.update_if_changed(1280, -1), None);
            assert_eq!(
                size.update_if_changed(1920, 1080),
                Some(FramePoolSize {
                    width: 1920,
                    height: 1080
                })
            );
            assert_eq!(
                size,
                FramePoolSize {
                    width: 1920,
                    height: 1080
                }
            );
        }
    }
}

#[cfg(not(windows))]
fn start_platform_capture(_target: CaptureTarget) -> Result<CaptureSession, CaptureError> {
    Err(CaptureError::unsupported_platform(std::env::consts::OS))
}
