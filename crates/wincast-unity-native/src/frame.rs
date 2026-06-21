use crate::error::{UnityNativeError, UnityNativeResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub enum WincastUnityFrameFormat {
    Rgba8 = 0,
    Bgra8 = 1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameMetadata {
    pub width: u32,
    pub height: u32,
    pub stride_bytes: u32,
    pub format: WincastUnityFrameFormat,
    pub timestamp_ns: u64,
    pub byte_len: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmittedFrame {
    pub metadata: FrameMetadata,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSnapshot {
    pub submitted_frame_count: u64,
    pub latest_frame: Option<SubmittedFrame>,
}

impl FrameMetadata {
    pub(crate) fn validate(
        width: u32,
        height: u32,
        stride_bytes: u32,
        format: WincastUnityFrameFormat,
        timestamp_ns: u64,
    ) -> UnityNativeResult<Self> {
        if width == 0 {
            return Err(UnityNativeError::InvalidWidth);
        }
        if height == 0 {
            return Err(UnityNativeError::InvalidHeight);
        }

        let bytes_per_pixel = match format {
            WincastUnityFrameFormat::Rgba8 | WincastUnityFrameFormat::Bgra8 => 4,
        };
        let minimum_stride = width
            .checked_mul(bytes_per_pixel)
            .ok_or(UnityNativeError::InvalidStride)?;
        if stride_bytes < minimum_stride {
            return Err(UnityNativeError::InvalidStride);
        }

        let byte_len = stride_bytes
            .checked_mul(height)
            .ok_or(UnityNativeError::InvalidStride)? as usize;

        Ok(Self {
            width,
            height,
            stride_bytes,
            format,
            timestamp_ns,
            byte_len,
        })
    }
}
