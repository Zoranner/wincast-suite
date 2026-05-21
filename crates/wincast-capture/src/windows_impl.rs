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
    Wdk::System::SystemServices::RtlGetVersion,
    Win32::{
        Foundation::{HMODULE, HWND},
        Graphics::{
            Direct3D::{
                D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_UNKNOWN,
                D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_9_1, D3D_FEATURE_LEVEL_9_2,
                D3D_FEATURE_LEVEL_9_3, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_10_1,
                D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
            },
            Direct3D11::{
                D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
                D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC,
                D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext,
                ID3D11Texture2D,
            },
            Dxgi::{
                CreateDXGIFactory1, DXGI_ERROR_NOT_FOUND, DXGI_ERROR_WAIT_TIMEOUT,
                DXGI_OUTDUPL_FRAME_INFO, IDXGIAdapter, IDXGIDevice, IDXGIFactory1, IDXGIOutput,
                IDXGIOutput1, IDXGIOutputDuplication, IDXGIResource,
            },
            Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow},
        },
        System::{
            SystemInformation::{GetVersionExW, OSVERSIONINFOW},
            WinRT::{
                Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess},
                Graphics::Capture::IGraphicsCaptureItemInterop,
            },
        },
        UI::WindowsAndMessaging::{IsIconic, IsWindow},
    },
    core::{Interface, factory},
};

#[derive(Debug)]
pub(crate) struct WindowsCaptureState {
    backend: WindowsCaptureBackend,
    source_window_handle: isize,
    sequence_number: u64,
}

#[derive(Debug)]
enum WindowsCaptureBackend {
    GraphicsCapture(GraphicsCaptureBackend),
    DesktopDuplication(DesktopDuplicationBackend),
}

#[derive(Debug)]
struct GraphicsCaptureBackend {
    d3d_device: ID3D11Device,
    d3d_context: ID3D11DeviceContext,
    direct3d_device: IDirect3DDevice,
    frame_pool: Direct3D11CaptureFramePool,
    _session: GraphicsCaptureSession,
    frame_pool_size: FramePoolSize,
}

#[derive(Debug)]
struct DesktopDuplicationBackend {
    d3d_device: ID3D11Device,
    d3d_context: ID3D11DeviceContext,
    duplication: IDXGIOutputDuplication,
    width: u32,
    height: u32,
}

#[derive(Debug)]
struct AcquiredDuplicationFrame<'a> {
    duplication: &'a IDXGIOutputDuplication,
    resource: IDXGIResource,
}

impl Drop for AcquiredDuplicationFrame<'_> {
    fn drop(&mut self) {
        let _ = unsafe { self.duplication.ReleaseFrame() };
    }
}

#[derive(Debug)]
struct MonitorCaptureTarget {
    monitor: windows::Win32::Graphics::Gdi::HMONITOR,
}

impl WindowsCaptureState {
    pub(crate) fn is_active(&self) -> bool {
        source_window_is_active(self.source_window_handle)
    }

    pub(crate) fn try_next_frame_metadata(
        &mut self,
    ) -> Result<Option<CapturedFrame>, CaptureError> {
        match &mut self.backend {
            WindowsCaptureBackend::GraphicsCapture(backend) => {
                let frame = backend.try_next_frame()?;
                let Some(frame) = frame else {
                    return Ok(None);
                };
                let sequence_number = self.sequence_number;
                self.sequence_number = self.sequence_number.saturating_add(1);

                Ok(Some(captured_frame_metadata(&frame, sequence_number)?))
            }
            WindowsCaptureBackend::DesktopDuplication(backend) => {
                let frame = backend.try_next_bgra_frame(self.sequence_number)?;
                let Some(frame) = frame else {
                    return Ok(None);
                };
                self.sequence_number = self.sequence_number.saturating_add(1);
                Ok(Some(frame.metadata.frame))
            }
        }
    }

    pub(crate) fn try_next_texture_metadata(
        &mut self,
    ) -> Result<Option<CapturedTextureMetadata>, CaptureError> {
        match &mut self.backend {
            WindowsCaptureBackend::GraphicsCapture(backend) => Ok(
                try_next_graphics_capture_texture(backend, &mut self.sequence_number)?
                    .map(|(metadata, _, _)| metadata),
            ),
            WindowsCaptureBackend::DesktopDuplication(backend) => Ok(backend
                .try_next_bgra_frame(self.sequence_number)?
                .map(|frame| {
                    self.sequence_number = self.sequence_number.saturating_add(1);
                    frame.metadata
                })),
        }
    }

    pub(crate) fn try_next_bgra_frame(
        &mut self,
    ) -> Result<Option<CapturedBgraFrame>, CaptureError> {
        match &mut self.backend {
            WindowsCaptureBackend::GraphicsCapture(backend) => {
                let Some((metadata, texture, texture_desc)) =
                    try_next_graphics_capture_texture(backend, &mut self.sequence_number)?
                else {
                    return Ok(None);
                };

                let readback = readback_bgra_frame(
                    &backend.d3d_device,
                    &backend.d3d_context,
                    &texture,
                    &texture_desc,
                )?;

                Ok(Some(CapturedBgraFrame {
                    metadata,
                    row_pitch: readback.row_pitch,
                    bytes: readback.bytes,
                }))
            }
            WindowsCaptureBackend::DesktopDuplication(backend) => {
                let frame = backend.try_next_bgra_frame(self.sequence_number)?;
                if frame.is_some() {
                    self.sequence_number = self.sequence_number.saturating_add(1);
                }
                Ok(frame)
            }
        }
    }
}

fn try_next_graphics_capture_texture(
    backend: &mut GraphicsCaptureBackend,
    sequence_number: &mut u64,
) -> Result<Option<FrameTexture>, CaptureError> {
    let frame = backend.try_next_frame()?;
    let Some(frame) = frame else {
        return Ok(None);
    };
    let metadata = captured_frame_metadata(&frame, *sequence_number)?;
    *sequence_number = sequence_number.saturating_add(1);
    let (texture, texture_desc) = frame_texture(&frame)?;
    let metadata = captured_texture_metadata(metadata, &texture_desc);

    Ok(Some((metadata, texture, texture_desc)))
}

impl GraphicsCaptureBackend {
    fn try_next_frame(&mut self) -> Result<Option<Direct3D11CaptureFrame>, CaptureError> {
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
        Ok(Some(frame))
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

impl DesktopDuplicationBackend {
    fn try_next_bgra_frame(
        &mut self,
        sequence_number: u64,
    ) -> Result<Option<CapturedBgraFrame>, CaptureError> {
        let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource = None;
        let acquire_result = unsafe {
            self.duplication
                .AcquireNextFrame(0, &mut frame_info, &mut resource)
        };
        match acquire_result {
            Ok(()) => {}
            Err(error) if error.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(None),
            Err(error) => {
                return Err(CaptureError::windows_frame_read_failed(error.to_string()));
            }
        }
        let resource = resource.ok_or_else(|| {
            let _ = unsafe { self.duplication.ReleaseFrame() };
            CaptureError::windows_frame_read_failed("DXGI Desktop Duplication 未返回帧资源")
        })?;
        let acquired = AcquiredDuplicationFrame {
            duplication: &self.duplication,
            resource,
        };

        let texture: ID3D11Texture2D = acquired
            .resource
            .cast()
            .map_err(|error| CaptureError::windows_frame_read_failed(error.to_string()))?;
        let texture_desc = texture_desc(&texture);
        let readback =
            readback_bgra_frame(&self.d3d_device, &self.d3d_context, &texture, &texture_desc)?;

        let frame = CapturedFrame {
            width: self.width,
            height: self.height,
            stride_bytes: readback.row_pitch,
            pixel_format: FramePixelFormat::Bgra8Unorm,
            sequence_number,
            timestamp_ns: frame_info.LastPresentTime.max(0) as u64,
        };
        Ok(Some(CapturedBgraFrame {
            metadata: CapturedTextureMetadata {
                frame,
                texture_width: texture_desc.Width,
                texture_height: texture_desc.Height,
                mip_levels: texture_desc.MipLevels,
                array_size: texture_desc.ArraySize,
                sample_count: texture_desc.SampleDesc.Count,
            },
            row_pitch: readback.row_pitch,
            bytes: readback.bytes,
        }))
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

trait ReadbackUnmapper {
    fn unmap(&mut self);
}

struct D3dReadbackUnmapper<'a> {
    context: &'a ID3D11DeviceContext,
    texture: &'a ID3D11Texture2D,
    subresource: u32,
}

impl ReadbackUnmapper for D3dReadbackUnmapper<'_> {
    fn unmap(&mut self) {
        unsafe {
            self.context.Unmap(self.texture, self.subresource);
        }
    }
}

struct MappedReadback<U: ReadbackUnmapper> {
    mapped: D3D11_MAPPED_SUBRESOURCE,
    unmapper: U,
}

impl<U: ReadbackUnmapper> MappedReadback<U> {
    fn new(mapped: D3D11_MAPPED_SUBRESOURCE, unmapper: U) -> Self {
        Self { mapped, unmapper }
    }

    fn row_pitch(&self) -> u32 {
        self.mapped.RowPitch
    }

    fn copy_rows_to_vec(&self, height: u32) -> Result<Vec<u8>, CaptureError> {
        let byte_len = mapped_byte_len(height, self.row_pitch())?;
        if byte_len == 0 {
            return Ok(Vec::new());
        }
        if self.mapped.pData.is_null() {
            return Err(CaptureError::windows_frame_read_failed("Map 返回空指针"));
        }

        // SAFETY:
        // - `MappedReadback` 只会在 D3D11 `Map` 成功后构造，因此 `pData` 来自有效映射。
        // - `mapped_byte_len` 已验证 `height * row_pitch` 不溢出且不超过 `isize::MAX`。
        // - 本切片仅在当前 guard 生命周期内短暂存在；guard drop 前不会 `Unmap`。
        // - 读取范围仅覆盖当前 subresource 的 `height * RowPitch` 字节，不跨越映射边界。
        Ok(unsafe { slice::from_raw_parts(self.mapped.pData.cast::<u8>(), byte_len) }.to_vec())
    }
}

impl<U: ReadbackUnmapper> Drop for MappedReadback<U> {
    fn drop(&mut self) {
        self.unmapper.unmap();
    }
}

fn mapped_byte_len(height: u32, row_pitch: u32) -> Result<usize, CaptureError> {
    let byte_len = (height as usize)
        .checked_mul(row_pitch as usize)
        .ok_or_else(|| CaptureError::windows_frame_read_failed("readback 字节长度溢出"))?;
    if byte_len > isize::MAX as usize {
        return Err(CaptureError::windows_frame_read_failed(
            "readback 字节长度超过 isize::MAX",
        ));
    }

    Ok(byte_len)
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

fn texture_desc(texture: &ID3D11Texture2D) -> D3D11_TEXTURE2D_DESC {
    let mut texture_desc = D3D11_TEXTURE2D_DESC::default();
    unsafe {
        texture.GetDesc(&mut texture_desc);
    }
    texture_desc
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
    let mapped = MappedReadback::new(
        mapped,
        D3dReadbackUnmapper {
            context: d3d_context,
            texture: &staging,
            subresource: 0,
        },
    );

    let row_pitch = mapped.row_pitch();
    let bytes = mapped.copy_rows_to_vec(texture_desc.Height)?;

    Ok(BgraReadback { row_pitch, bytes })
}

pub(crate) fn start_windows_capture(
    target: &CaptureTarget,
) -> Result<WindowsCaptureState, CaptureError> {
    let source_window_handle = source_window_handle(target);
    let backend = match target {
        CaptureTarget::Window { handle, .. } => {
            if !windows_graphics_capture_interop_supported() {
                return Err(CaptureError::windows_window_capture_unsupported(
                    windows_build_number(),
                ));
            }
            WindowsCaptureBackend::GraphicsCapture(start_graphics_capture_backend(
                create_window_capture_item(*handle)?,
            )?)
        }
        CaptureTarget::Desktop {
            source_window_handle,
        } => WindowsCaptureBackend::DesktopDuplication(start_desktop_duplication_backend(
            monitor_capture_target_from_window(*source_window_handle)?,
        )?),
    };
    Ok(WindowsCaptureState {
        backend,
        source_window_handle,
        sequence_number: 0,
    })
}

fn start_graphics_capture_backend(
    item: GraphicsCaptureItem,
) -> Result<GraphicsCaptureBackend, CaptureError> {
    let supported = GraphicsCaptureSession::IsSupported().map_err(|error| {
        CaptureError::windows_graphics_capture_support_check_failed(error.to_string())
    })?;
    if !supported {
        return Err(CaptureError::windows_graphics_capture_unsupported());
    }
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

    Ok(GraphicsCaptureBackend {
        d3d_device,
        d3d_context,
        direct3d_device,
        frame_pool,
        _session: session,
        frame_pool_size: FramePoolSize::from_size(frame_pool_size),
    })
}

fn start_desktop_duplication_backend(
    target: MonitorCaptureTarget,
) -> Result<DesktopDuplicationBackend, CaptureError> {
    let output = output_for_monitor(target.monitor)?;
    let adapter = adapter_for_output(&output)?;
    let d3d_device = create_d3d_device_for_adapter(&adapter)?;
    let d3d_context = unsafe { d3d_device.GetImmediateContext() }
        .map_err(|error| CaptureError::windows_d3d_initialization_failed(error.to_string()))?;
    let output1: IDXGIOutput1 = output
        .cast()
        .map_err(|error| CaptureError::windows_capture_item_create_failed(error.to_string()))?;
    let duplication = unsafe { output1.DuplicateOutput(&d3d_device) }
        .map_err(|error| CaptureError::windows_capture_session_create_failed(error.to_string()))?;
    let desc = unsafe { output.GetDesc() }
        .map_err(|error| CaptureError::windows_capture_item_create_failed(error.to_string()))?;
    let width = (desc.DesktopCoordinates.right - desc.DesktopCoordinates.left)
        .try_into()
        .map_err(|_| CaptureError::windows_capture_item_create_failed("显示器宽度无效"))?;
    let height = (desc.DesktopCoordinates.bottom - desc.DesktopCoordinates.top)
        .try_into()
        .map_err(|_| CaptureError::windows_capture_item_create_failed("显示器高度无效"))?;

    Ok(DesktopDuplicationBackend {
        d3d_device,
        d3d_context,
        duplication,
        width,
        height,
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

fn monitor_capture_target_from_window(handle: isize) -> Result<MonitorCaptureTarget, CaptureError> {
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
    Ok(MonitorCaptureTarget { monitor })
}

fn create_d3d_device() -> Result<ID3D11Device, CaptureError> {
    create_d3d_device_inner(None, D3D_DRIVER_TYPE_HARDWARE)
}

fn create_d3d_device_for_adapter(adapter: &IDXGIAdapter) -> Result<ID3D11Device, CaptureError> {
    create_d3d_device_inner(Some(adapter), D3D_DRIVER_TYPE_UNKNOWN)
}

fn create_d3d_device_inner(
    adapter: Option<&IDXGIAdapter>,
    driver_type: D3D_DRIVER_TYPE,
) -> Result<ID3D11Device, CaptureError> {
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
            adapter,
            driver_type,
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

fn output_for_monitor(
    monitor: windows::Win32::Graphics::Gdi::HMONITOR,
) -> Result<IDXGIOutput, CaptureError> {
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }
        .map_err(|error| CaptureError::windows_d3d_initialization_failed(error.to_string()))?;
    let mut adapter_index = 0;
    loop {
        match unsafe { factory.EnumAdapters1(adapter_index) } {
            Ok(adapter) => {
                if let Some(output) = output_for_monitor_on_adapter(&adapter, monitor)? {
                    return Ok(output);
                }
            }
            Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => {
                return Err(CaptureError::windows_capture_item_create_failed(
                    "未找到窗口所在显示器的 DXGI 输出",
                ));
            }
            Err(error) => {
                return Err(CaptureError::windows_capture_item_create_failed(
                    error.to_string(),
                ));
            }
        }
        adapter_index += 1;
    }
}

fn output_for_monitor_on_adapter(
    adapter: &IDXGIAdapter,
    monitor: windows::Win32::Graphics::Gdi::HMONITOR,
) -> Result<Option<IDXGIOutput>, CaptureError> {
    let mut index = 0;
    loop {
        match unsafe { adapter.EnumOutputs(index) } {
            Ok(output) => {
                let desc = unsafe { output.GetDesc() }.map_err(|error| {
                    CaptureError::windows_capture_item_create_failed(error.to_string())
                })?;
                if desc.Monitor == monitor {
                    return Ok(Some(output));
                }
            }
            Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => return Ok(None),
            Err(error) => {
                return Err(CaptureError::windows_capture_item_create_failed(
                    error.to_string(),
                ));
            }
        }
        index += 1;
    }
}

fn adapter_for_output(output: &IDXGIOutput) -> Result<IDXGIAdapter, CaptureError> {
    unsafe { output.GetParent() }
        .map_err(|error| CaptureError::windows_capture_item_create_failed(error.to_string()))
}

fn windows_graphics_capture_interop_supported() -> bool {
    windows_build_number() >= 18_362
}

fn windows_build_number() -> u32 {
    let mut version = OSVERSIONINFOW {
        dwOSVersionInfoSize: mem::size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    let status = unsafe { RtlGetVersion(&mut version) };
    if status.is_ok() {
        return version.dwBuildNumber;
    }
    match unsafe { GetVersionExW(&mut version) } {
        Ok(()) => version.dwBuildNumber,
        Err(_) => 0,
    }
}

fn source_window_handle(target: &CaptureTarget) -> isize {
    match target {
        CaptureTarget::Desktop {
            source_window_handle,
        } => *source_window_handle,
        CaptureTarget::Window { handle, .. } => *handle,
    }
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
    use std::cell::Cell;

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

    #[test]
    fn mapped_readback_unmaps_when_dropped() {
        struct CountingUnmapper<'a> {
            calls: &'a Cell<usize>,
        }

        impl ReadbackUnmapper for CountingUnmapper<'_> {
            fn unmap(&mut self) {
                self.calls.set(self.calls.get() + 1);
            }
        }

        let calls = Cell::new(0);
        {
            let _mapping = MappedReadback::new(
                D3D11_MAPPED_SUBRESOURCE::default(),
                CountingUnmapper { calls: &calls },
            );
            assert_eq!(calls.get(), 0);
        }

        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn mapped_readback_rejects_null_pointer_for_non_empty_copy() {
        struct NoopUnmapper;

        impl ReadbackUnmapper for NoopUnmapper {
            fn unmap(&mut self) {}
        }

        let mapping = MappedReadback::new(
            D3D11_MAPPED_SUBRESOURCE {
                pData: std::ptr::null_mut(),
                RowPitch: 4,
                DepthPitch: 4,
            },
            NoopUnmapper,
        );

        let error = mapping
            .copy_rows_to_vec(1)
            .expect_err("null pointer must be rejected for non-empty copy");

        assert_eq!(
            error,
            CaptureError::windows_frame_read_failed("Map 返回空指针")
        );
    }

    #[test]
    fn mapped_readback_allows_zero_length_copy_without_pointer() {
        struct NoopUnmapper;

        impl ReadbackUnmapper for NoopUnmapper {
            fn unmap(&mut self) {}
        }

        let mapping = MappedReadback::new(
            D3D11_MAPPED_SUBRESOURCE {
                pData: std::ptr::null_mut(),
                RowPitch: 128,
                DepthPitch: 128,
            },
            NoopUnmapper,
        );

        let bytes = mapping
            .copy_rows_to_vec(0)
            .expect("zero-length copy should not require a backing pointer");

        assert!(bytes.is_empty());
    }

    #[test]
    fn mapped_byte_len_rejects_overflow() {
        let error = mapped_byte_len(u32::MAX, u32::MAX).expect_err("overflow must be rejected");

        assert_eq!(
            error,
            CaptureError::windows_frame_read_failed("readback 字节长度超过 isize::MAX")
        );
    }
}
