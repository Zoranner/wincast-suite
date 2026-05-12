use std::fmt;

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
