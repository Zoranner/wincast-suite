use std::slice;

use crate::{
    error::CaptureError,
    model::{
        CaptureTarget, CapturedBgraFrame, CapturedFrame, CapturedTextureMetadata, FramePixelFormat,
    },
};
use windows::{
    Win32::{
        Foundation::HMODULE,
        Graphics::{
            Direct3D::{
                D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_9_1,
                D3D_FEATURE_LEVEL_9_2, D3D_FEATURE_LEVEL_9_3, D3D_FEATURE_LEVEL_10_0,
                D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
            },
            Direct3D11::{
                D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
                D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC,
                D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext,
                ID3D11Texture2D,
            },
            Dxgi::{
                CreateDXGIFactory1, DXGI_ERROR_NOT_FOUND, DXGI_ERROR_WAIT_TIMEOUT,
                DXGI_OUTDUPL_FRAME_INFO, IDXGIAdapter, IDXGIFactory1, IDXGIOutput, IDXGIOutput1,
                IDXGIOutputDuplication, IDXGIResource,
            },
        },
    },
    core::Interface,
};

#[derive(Debug)]
pub(crate) struct WindowsCaptureState {
    backend: DesktopDuplicationBackend,
    sequence_number: u64,
}

#[derive(Debug)]
struct DesktopDuplicationBackend {
    d3d_device: ID3D11Device,
    d3d_context: ID3D11DeviceContext,
    duplication: IDXGIOutputDuplication,
    width: u32,
    height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OutputCandidateInfo {
    adapter_index: u32,
    output_index: u32,
    attached_to_desktop: bool,
    width: u32,
    height: u32,
}

#[derive(Debug)]
struct EnumeratedOutput {
    info: OutputCandidateInfo,
    output: IDXGIOutput,
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

impl WindowsCaptureState {
    pub(crate) fn is_active(&self) -> bool {
        true
    }

    pub(crate) fn try_next_frame_metadata(
        &mut self,
    ) -> Result<Option<CapturedFrame>, CaptureError> {
        let frame = self.backend.try_next_bgra_frame(self.sequence_number)?;
        let Some(frame) = frame else {
            return Ok(None);
        };
        self.sequence_number = self.sequence_number.saturating_add(1);
        Ok(Some(frame.metadata.frame))
    }

    pub(crate) fn try_next_texture_metadata(
        &mut self,
    ) -> Result<Option<CapturedTextureMetadata>, CaptureError> {
        Ok(self
            .backend
            .try_next_bgra_frame(self.sequence_number)?
            .map(|frame| {
                self.sequence_number = self.sequence_number.saturating_add(1);
                frame.metadata
            }))
    }

    pub(crate) fn try_next_bgra_frame(
        &mut self,
    ) -> Result<Option<CapturedBgraFrame>, CaptureError> {
        let frame = self.backend.try_next_bgra_frame(self.sequence_number)?;
        if frame.is_some() {
            self.sequence_number = self.sequence_number.saturating_add(1);
        }
        Ok(frame)
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
    let CaptureTarget::Screen = target;
    let backend = start_desktop_duplication_backend()?;
    Ok(WindowsCaptureState {
        backend,
        sequence_number: 0,
    })
}

fn start_desktop_duplication_backend() -> Result<DesktopDuplicationBackend, CaptureError> {
    let output = primary_output()?;
    let adapter = adapter_for_output(&output)?;
    let d3d_device = create_d3d_device_for_adapter(&adapter)?;
    let d3d_context = unsafe { d3d_device.GetImmediateContext() }
        .map_err(|error| CaptureError::windows_d3d_initialization_failed(error.to_string()))?;
    let output1: IDXGIOutput1 = output.cast().map_err(|error| {
        CaptureError::windows_desktop_output_enumeration_failed(error.to_string())
    })?;
    let duplication = unsafe { output1.DuplicateOutput(&d3d_device) }
        .map_err(|error| CaptureError::windows_capture_session_create_failed(error.to_string()))?;
    let desc = unsafe { output.GetDesc() }.map_err(|error| {
        CaptureError::windows_desktop_output_enumeration_failed(error.to_string())
    })?;
    let width = (desc.DesktopCoordinates.right - desc.DesktopCoordinates.left)
        .try_into()
        .map_err(|_| CaptureError::windows_desktop_output_enumeration_failed("显示器宽度无效"))?;
    let height = (desc.DesktopCoordinates.bottom - desc.DesktopCoordinates.top)
        .try_into()
        .map_err(|_| CaptureError::windows_desktop_output_enumeration_failed("显示器高度无效"))?;

    Ok(DesktopDuplicationBackend {
        d3d_device,
        d3d_context,
        duplication,
        width,
        height,
    })
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

fn primary_output() -> Result<IDXGIOutput, CaptureError> {
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }
        .map_err(|error| CaptureError::windows_d3d_initialization_failed(error.to_string()))?;
    let mut outputs = enumerate_desktop_outputs(&factory)?;
    let infos = outputs
        .iter()
        .map(|candidate| candidate.info)
        .collect::<Vec<_>>();
    let selected = select_single_attached_desktop_output(&infos)?;
    Ok(outputs.swap_remove(selected).output)
}

fn enumerate_desktop_outputs(
    factory: &IDXGIFactory1,
) -> Result<Vec<EnumeratedOutput>, CaptureError> {
    let mut outputs = Vec::new();
    let mut adapter_index = 0;
    loop {
        let adapter = match unsafe { factory.EnumAdapters1(adapter_index) } {
            Ok(adapter) => adapter,
            Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
            Err(error) => {
                return Err(CaptureError::windows_desktop_output_enumeration_failed(
                    error.to_string(),
                ));
            }
        };

        let mut output_index = 0;
        loop {
            let output = match unsafe { adapter.EnumOutputs(output_index) } {
                Ok(output) => output,
                Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
                Err(error) => {
                    return Err(CaptureError::windows_desktop_output_enumeration_failed(
                        error.to_string(),
                    ));
                }
            };
            let desc = unsafe { output.GetDesc() }.map_err(|error| {
                CaptureError::windows_desktop_output_enumeration_failed(error.to_string())
            })?;
            let width = (desc.DesktopCoordinates.right - desc.DesktopCoordinates.left)
                .try_into()
                .unwrap_or(0);
            let height = (desc.DesktopCoordinates.bottom - desc.DesktopCoordinates.top)
                .try_into()
                .unwrap_or(0);
            outputs.push(EnumeratedOutput {
                info: OutputCandidateInfo {
                    adapter_index,
                    output_index,
                    attached_to_desktop: desc.AttachedToDesktop.as_bool(),
                    width,
                    height,
                },
                output,
            });
            output_index += 1;
        }

        adapter_index += 1;
    }

    Ok(outputs)
}

fn select_single_attached_desktop_output(
    outputs: &[OutputCandidateInfo],
) -> Result<usize, CaptureError> {
    let attached = outputs
        .iter()
        .enumerate()
        .filter(|(_, output)| output.attached_to_desktop && output.width > 0 && output.height > 0)
        .collect::<Vec<_>>();

    match attached.as_slice() {
        [] => Err(CaptureError::windows_desktop_output_enumeration_failed(
            "没有找到可用桌面输出",
        )),
        [(index, _)] => Ok(*index),
        _ => {
            let details = attached
                .iter()
                .map(|(_, output)| {
                    format!(
                        "adapter {}/output {} {}x{}",
                        output.adapter_index, output.output_index, output.width, output.height
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            Err(CaptureError::windows_desktop_output_enumeration_failed(
                format!(
                    "检测到 {} 个可用桌面输出，当前稳定版只支持单显示器: {details}",
                    attached.len()
                ),
            ))
        }
    }
}

fn adapter_for_output(output: &IDXGIOutput) -> Result<IDXGIAdapter, CaptureError> {
    unsafe { output.GetParent() }
        .map_err(|error| CaptureError::windows_desktop_output_enumeration_failed(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

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

    #[test]
    fn desktop_output_selection_uses_single_attached_desktop_output() {
        let outputs = [
            OutputCandidateInfo {
                adapter_index: 0,
                output_index: 0,
                attached_to_desktop: false,
                width: 0,
                height: 0,
            },
            OutputCandidateInfo {
                adapter_index: 1,
                output_index: 0,
                attached_to_desktop: true,
                width: 1920,
                height: 1080,
            },
        ];

        let selected = select_single_attached_desktop_output(&outputs)
            .expect("single attached output should be selected");

        assert_eq!(selected, 1);
    }

    #[test]
    fn desktop_output_selection_rejects_multiple_attached_desktop_outputs() {
        let outputs = [
            OutputCandidateInfo {
                adapter_index: 0,
                output_index: 0,
                attached_to_desktop: true,
                width: 1920,
                height: 1080,
            },
            OutputCandidateInfo {
                adapter_index: 0,
                output_index: 1,
                attached_to_desktop: true,
                width: 1280,
                height: 720,
            },
        ];

        let error = select_single_attached_desktop_output(&outputs)
            .expect_err("multiple attached outputs should be rejected");

        assert_eq!(
            error,
            CaptureError::windows_desktop_output_enumeration_failed(
                "检测到 2 个可用桌面输出，当前稳定版只支持单显示器: adapter 0/output 0 1920x1080; adapter 0/output 1 1280x720"
            )
        );
    }
}
