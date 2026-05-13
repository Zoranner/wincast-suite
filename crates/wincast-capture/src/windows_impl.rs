use std::{mem, slice};

use crate::{
    error::CaptureError,
    model::{
        CaptureTarget, CapturedBgraFrame, CapturedFrame, CapturedTextureMetadata, FramePixelFormat,
    },
};
use windows::{
    Graphics::{
        Capture::{
            Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem,
            GraphicsCaptureSession,
        },
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
            Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow},
        },
        System::WinRT::{
            Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess},
            Graphics::Capture::IGraphicsCaptureItemInterop,
        },
        UI::WindowsAndMessaging::{IsIconic, IsWindow},
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
    source_window_handle: isize,
    frame_pool_size: FramePoolSize,
    sequence_number: u64,
}

impl WindowsCaptureState {
    pub(crate) fn is_active(&self) -> bool {
        source_window_is_active(self.source_window_handle)
    }

    pub(crate) fn try_next_frame_metadata(
        &mut self,
    ) -> Result<Option<CapturedFrame>, CaptureError> {
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
    ) -> Result<Option<CapturedTextureMetadata>, CaptureError> {
        Ok(self.try_next_texture()?.map(|(metadata, _, _)| metadata))
    }

    pub(crate) fn try_next_bgra_frame(
        &mut self,
    ) -> Result<Option<CapturedBgraFrame>, CaptureError> {
        let Some((metadata, texture, texture_desc)) = self.try_next_texture()? else {
            return Ok(None);
        };

        let readback =
            readback_bgra_frame(&self.d3d_device, &self.d3d_context, &texture, &texture_desc)?;

        Ok(Some(CapturedBgraFrame {
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
    CapturedTextureMetadata,
    ID3D11Texture2D,
    D3D11_TEXTURE2D_DESC,
);

struct BgraReadback {
    row_pitch: u32,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SourceWindowActivity {
    exists: bool,
    minimized: bool,
}

impl SourceWindowActivity {
    fn is_active(self) -> bool {
        self.exists && !self.minimized
    }
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
    frame: &Direct3D11CaptureFrame,
    sequence_number: u64,
) -> Result<CapturedFrame, CaptureError> {
    let size = frame
        .ContentSize()
        .map_err(|error| CaptureError::windows_frame_read_failed(error.to_string()))?;
    let timestamp = frame
        .SystemRelativeTime()
        .map_err(|error| CaptureError::windows_frame_read_failed(error.to_string()))?;

    Ok(CapturedFrame {
        width: size.Width.max(0) as u32,
        height: size.Height.max(0) as u32,
        stride_bytes: size.Width.max(0) as u32 * 4,
        pixel_format: FramePixelFormat::Bgra8Unorm,
        sequence_number,
        timestamp_ns: timestamp.Duration.max(0) as u64 * 100,
    })
}

fn frame_texture(
    frame: &Direct3D11CaptureFrame,
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
    frame: CapturedFrame,
    texture_desc: &D3D11_TEXTURE2D_DESC,
) -> CapturedTextureMetadata {
    CapturedTextureMetadata {
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

    let (item, source_window_handle) = match target {
        CaptureTarget::Window { handle, .. } => (create_window_capture_item(*handle)?, *handle),
        CaptureTarget::Desktop {
            source_window_handle,
        } => (
            create_monitor_capture_item_from_window(*source_window_handle)?,
            *source_window_handle,
        ),
    };

    let d3d_device = create_d3d_device()?;
    let d3d_context = unsafe { d3d_device.GetImmediateContext() }
        .map_err(|error| CaptureError::windows_d3d_initialization_failed(error.to_string()))?;
    let direct3d_device = create_direct3d_device(&d3d_device)?;
    let frame_pool_size = item
        .Size()
        .map_err(|error| CaptureError::windows_capture_session_create_failed(error.to_string()))?;
    let frame_pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
        &direct3d_device,
        DirectXPixelFormat::B8G8R8A8UIntNormalized,
        1,
        frame_pool_size,
    )
    .map_err(|error| CaptureError::windows_capture_session_create_failed(error.to_string()))?;
    let session = frame_pool
        .CreateCaptureSession(&item)
        .map_err(|error| CaptureError::windows_capture_session_create_failed(error.to_string()))?;
    session
        .StartCapture()
        .map_err(|error| CaptureError::windows_capture_session_start_failed(error.to_string()))?;

    Ok(WindowsCaptureState {
        d3d_device,
        d3d_context,
        direct3d_device,
        frame_pool,
        _session: session,
        source_window_handle,
        frame_pool_size: FramePoolSize::from_size(frame_pool_size),
        sequence_number: 0,
    })
}

fn source_window_is_active(handle: isize) -> bool {
    query_source_window_activity(handle).is_active()
}

fn query_source_window_activity(handle: isize) -> SourceWindowActivity {
    let hwnd = HWND(handle as *mut core::ffi::c_void);
    let exists = unsafe { IsWindow(Some(hwnd)).as_bool() };
    let minimized = exists && unsafe { IsIconic(hwnd).as_bool() };
    SourceWindowActivity { exists, minimized }
}

fn create_window_capture_item(handle: isize) -> Result<GraphicsCaptureItem, CaptureError> {
    let hwnd = HWND(handle as *mut core::ffi::c_void);
    let interop = factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
        .map_err(|error| CaptureError::windows_capture_item_create_failed(error.to_string()))?;
    unsafe { interop.CreateForWindow(hwnd) }
        .map_err(|error| CaptureError::windows_capture_item_create_failed(error.to_string()))
}

fn create_monitor_capture_item_from_window(
    handle: isize,
) -> Result<GraphicsCaptureItem, CaptureError> {
    let hwnd = HWND(handle as *mut core::ffi::c_void);
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    if monitor.is_invalid() {
        return Err(CaptureError::windows_capture_item_create_failed(
            "MonitorFromWindow 未返回显示器",
        ));
    }

    let mut monitor_info = MONITORINFO {
        cbSize: mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let got_monitor_info = unsafe { GetMonitorInfoW(monitor, &mut monitor_info) };
    if !got_monitor_info.as_bool() {
        return Err(CaptureError::windows_capture_item_create_failed(
            "GetMonitorInfoW 返回失败",
        ));
    }

    let interop = factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
        .map_err(|error| CaptureError::windows_capture_item_create_failed(error.to_string()))?;
    unsafe { interop.CreateForMonitor(monitor) }
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

    #[test]
    fn source_window_activity_is_inactive_when_missing_or_minimized() {
        assert!(
            SourceWindowActivity {
                exists: true,
                minimized: false
            }
            .is_active()
        );
        assert!(
            !SourceWindowActivity {
                exists: false,
                minimized: false
            }
            .is_active()
        );
        assert!(
            !SourceWindowActivity {
                exists: true,
                minimized: true
            }
            .is_active()
        );
    }
}
