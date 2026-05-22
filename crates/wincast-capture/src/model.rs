use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureTarget {
    Screen,
}

impl fmt::Display for CaptureTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Screen => write!(formatter, "当前交互桌面整屏"),
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
