use thiserror::Error;
use wincast_protocol::config::VideoCodec;

mod decoder;
mod encoder;

pub use wincast_protocol::message::EncodedVideoFrame;

/// Boundary-test media backends.
///
/// These helpers produce deterministic fake H.264 payloads and BGRA frames for
/// media API tests. They are not real encoders/decoders and their payloads are
/// not playable H.264 bitstreams.
pub mod test_support {
    pub use crate::decoder::{FAKE_H264_DECODED_PAYLOAD_LIMIT, FakeH264Decoder};
    pub use crate::encoder::FakeH264Encoder;
}

pub const MAX_H264_WIDTH: u32 = 1920;
pub const MAX_H264_HEIGHT: u32 = 1080;
pub const MAX_H264_FPS: u32 = 60;

pub type MediaResult<T> = Result<T, MediaError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoLatencyMode {
    LowLatency,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoPipelineConfig {
    pub codec: VideoCodec,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate_kbps: u32,
    pub max_bitrate_kbps: u32,
    pub latency_mode: VideoLatencyMode,
}

impl VideoPipelineConfig {
    pub fn validate(&self) -> Result<(), MediaConfigError> {
        if self.codec != VideoCodec::H264 {
            return Err(MediaConfigError::UnsupportedCodec { codec: self.codec });
        }

        if self.width == 0 || self.height == 0 {
            return Err(MediaConfigError::InvalidDimensions {
                width: self.width,
                height: self.height,
            });
        }

        if self.width > MAX_H264_WIDTH || self.height > MAX_H264_HEIGHT {
            return Err(MediaConfigError::ResolutionTooLarge {
                width: self.width,
                height: self.height,
                max_width: MAX_H264_WIDTH,
                max_height: MAX_H264_HEIGHT,
            });
        }

        if self.fps == 0 || self.fps > MAX_H264_FPS {
            return Err(MediaConfigError::InvalidFps {
                fps: self.fps,
                max_fps: MAX_H264_FPS,
            });
        }

        if self.max_bitrate_kbps == 0 {
            return Err(MediaConfigError::InvalidMaxBitrate);
        }

        if self.bitrate_kbps == 0 || self.bitrate_kbps > self.max_bitrate_kbps {
            return Err(MediaConfigError::InvalidBitrate {
                bitrate_kbps: self.bitrate_kbps,
                max_bitrate_kbps: self.max_bitrate_kbps,
            });
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MediaConfigError {
    #[error("媒体链路只支持 H.264，当前配置为 {codec:?}")]
    UnsupportedCodec { codec: VideoCodec },
    #[error("视频尺寸无效: {width}x{height}")]
    InvalidDimensions { width: u32, height: u32 },
    #[error("视频尺寸 {width}x{height} 超过上限 {max_width}x{max_height}")]
    ResolutionTooLarge {
        width: u32,
        height: u32,
        max_width: u32,
        max_height: u32,
    },
    #[error("视频帧率 {fps} 无效，最大支持 {max_fps}")]
    InvalidFps { fps: u32, max_fps: u32 },
    #[error("视频码率上限必须大于 0")]
    InvalidMaxBitrate,
    #[error("视频目标码率 {bitrate_kbps} 无效，上限为 {max_bitrate_kbps}")]
    InvalidBitrate {
        bitrate_kbps: u32,
        max_bitrate_kbps: u32,
    },
}

#[derive(Debug, Error)]
pub enum MediaError {
    #[error("{0}")]
    Config(#[from] MediaConfigError),
    #[error("编码视频帧只支持 H.264，当前为 {codec:?}")]
    UnsupportedEncodedCodec { codec: VideoCodec },
    #[error("编码视频帧无效: {0:?}")]
    InvalidEncodedFrame(wincast_protocol::message::EncodedVideoFrameError),
    #[error("原始视频帧无效: {0}")]
    InvalidRawFrame(RawVideoFrameError),
    #[error("fake 解码输出载荷 {actual} 超过上限 {max}")]
    DecodedPayloadTooLarge { actual: usize, max: usize },
    #[error("媒体后端不可用: {0}")]
    BackendUnavailable(&'static str),
    #[error("媒体后端处理失败: {0}")]
    Backend(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RawVideoFrameError {
    #[error("raw frame dimensions are invalid: {width}x{height}")]
    InvalidDimensions { width: u32, height: u32 },
    #[error(
        "raw frame dimensions {width}x{height} exceed fake H.264 boundary {max_width}x{max_height}"
    )]
    ResolutionTooLarge {
        width: u32,
        height: u32,
        max_width: u32,
        max_height: u32,
    },
    #[error(
        "raw frame dimensions {frame_width}x{frame_height} do not match configured size {config_width}x{config_height}"
    )]
    ConfigDimensionMismatch {
        frame_width: u32,
        frame_height: u32,
        config_width: u32,
        config_height: u32,
    },
    #[error("fake H.264 encoder only accepts BGRA8 raw frames, current format is {format:?}")]
    UnsupportedPixelFormat { format: RawPixelFormat },
    #[error("raw frame payload is empty")]
    EmptyPayload,
    #[error("raw frame row pitch overflow")]
    RowPitchOverflow,
    #[error("raw frame row pitch {row_pitch} is below minimum {min_row_pitch}")]
    InvalidRowPitch { row_pitch: u32, min_row_pitch: u32 },
    #[error("raw frame payload length overflow")]
    PayloadLengthOverflow,
    #[error("raw frame payload length {actual} does not match expected {expected}")]
    InvalidPayloadLength { actual: usize, expected: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawPixelFormat {
    Bgra8Unorm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawVideoFrame<'a> {
    pub width: u32,
    pub height: u32,
    pub row_pitch: u32,
    pub format: RawPixelFormat,
    pub sequence_number: u64,
    pub timestamp_ns: u64,
    pub bytes: &'a [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodedPixelFormat {
    Bgra8Unorm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodedVideoFrame<'a> {
    pub width: u32,
    pub height: u32,
    pub format: DecodedPixelFormat,
    pub bytes: &'a [u8],
}

impl DecodedVideoFrame<'_> {
    pub fn row_pitch(&self) -> u32 {
        self.width * 4
    }
}

pub trait VideoEncoder {
    fn encode(&mut self, frame: RawVideoFrame<'_>) -> MediaResult<EncodedVideoFrame>;

    fn request_keyframe(&mut self) -> MediaResult<()>;
}

pub trait VideoDecoder {
    fn decode<'a>(&'a mut self, frame: &EncodedVideoFrame) -> MediaResult<DecodedVideoFrame<'a>>;
}
