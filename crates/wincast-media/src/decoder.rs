use openh264::formats::YUVSource;
use wincast_protocol::config::VideoCodec;

use crate::{
    DecodedPixelFormat, DecodedVideoFrame, EncodedVideoFrame, MAX_H264_HEIGHT, MAX_H264_WIDTH,
    MediaError, MediaResult, VideoDecoder,
};

pub const FAKE_H264_DECODED_PAYLOAD_LIMIT: usize =
    MAX_H264_WIDTH as usize * MAX_H264_HEIGHT as usize * 4;

pub struct OpenH264Decoder {
    decoder: openh264::decoder::Decoder,
    buffer: Vec<u8>,
}

impl OpenH264Decoder {
    pub fn new() -> MediaResult<Self> {
        let decoder = openh264::decoder::Decoder::new()
            .map_err(|error| MediaError::Backend(format!("初始化 OpenH264 解码器失败: {error}")))?;
        Ok(Self {
            decoder,
            buffer: Vec::new(),
        })
    }
}

impl VideoDecoder for OpenH264Decoder {
    fn decode<'a>(&'a mut self, frame: &EncodedVideoFrame) -> MediaResult<DecodedVideoFrame<'a>> {
        validate_h264_frame(frame)?;
        let Some(yuv) = self
            .decoder
            .decode(&frame.bytes)
            .map_err(|error| MediaError::Backend(format!("OpenH264 解码失败: {error}")))?
        else {
            return Err(MediaError::Backend(
                "OpenH264 解码器需要更多 H.264 数据".to_owned(),
            ));
        };
        if yuv.dimensions() != (frame.width as usize, frame.height as usize) {
            return Err(MediaError::Backend(format!(
                "OpenH264 解码尺寸 {}x{} 与协议帧 {}x{} 不一致",
                yuv.dimensions().0,
                yuv.dimensions().1,
                frame.width,
                frame.height
            )));
        }

        self.buffer.clear();
        self.buffer
            .resize(frame.width as usize * frame.height as usize * 4, 0);
        yuv.write_rgba8(&mut self.buffer);
        for pixel in self.buffer.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }

        Ok(DecodedVideoFrame {
            width: frame.width,
            height: frame.height,
            format: DecodedPixelFormat::Bgra8Unorm,
            bytes: self.buffer.as_slice(),
        })
    }
}

#[derive(Debug, Default)]
pub struct FakeH264Decoder {
    buffer: Vec<u8>,
}

impl FakeH264Decoder {
    pub fn new() -> Self {
        Self::default()
    }
}

impl VideoDecoder for FakeH264Decoder {
    fn decode<'a>(&'a mut self, frame: &EncodedVideoFrame) -> MediaResult<DecodedVideoFrame<'a>> {
        validate_h264_frame(frame)?;

        let row_pitch = frame.width.checked_mul(4).ok_or({
            MediaError::DecodedPayloadTooLarge {
                actual: usize::MAX,
                max: FAKE_H264_DECODED_PAYLOAD_LIMIT,
            }
        })?;
        let decoded_len = row_pitch.checked_mul(frame.height).ok_or({
            MediaError::DecodedPayloadTooLarge {
                actual: usize::MAX,
                max: FAKE_H264_DECODED_PAYLOAD_LIMIT,
            }
        })? as usize;

        if decoded_len > FAKE_H264_DECODED_PAYLOAD_LIMIT {
            return Err(MediaError::DecodedPayloadTooLarge {
                actual: decoded_len,
                max: FAKE_H264_DECODED_PAYLOAD_LIMIT,
            });
        }

        self.buffer.clear();
        self.buffer.reserve(decoded_len);
        for index in 0..decoded_len {
            let source = frame.bytes[(index / 4) % frame.bytes.len()];
            self.buffer.push(match index % 4 {
                3 => 0xff,
                _ => source,
            });
        }

        Ok(DecodedVideoFrame {
            width: frame.width,
            height: frame.height,
            format: DecodedPixelFormat::Bgra8Unorm,
            bytes: self.buffer.as_slice(),
        })
    }
}

fn validate_h264_frame(frame: &EncodedVideoFrame) -> MediaResult<()> {
    if frame.codec != VideoCodec::H264 {
        return Err(MediaError::UnsupportedEncodedCodec { codec: frame.codec });
    }

    frame.validate().map_err(MediaError::InvalidEncodedFrame)
}
