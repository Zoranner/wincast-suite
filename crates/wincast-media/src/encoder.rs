use wincast_protocol::config::VideoCodec;

use crate::{
    EncodedVideoFrame, MAX_H264_HEIGHT, MAX_H264_WIDTH, MediaError, MediaResult, RawPixelFormat,
    RawVideoFrame, RawVideoFrameError, VideoEncoder, VideoPipelineConfig,
};

const FAKE_H264_MAGIC: &[u8] = b"WINCAST_FAKE_H264";

pub struct OpenH264Encoder {
    config: VideoPipelineConfig,
    encoder: openh264::encoder::Encoder,
    rgb_buffer: Vec<u8>,
}

impl OpenH264Encoder {
    pub fn new(config: VideoPipelineConfig) -> MediaResult<Self> {
        config.validate()?;
        let encoder_config = openh264::encoder::EncoderConfig::new()
            .bitrate(openh264::encoder::BitRate::from_bps(
                config.bitrate_kbps.saturating_mul(1_000),
            ))
            .max_frame_rate(openh264::encoder::FrameRate::from_hz(config.fps as f32))
            // OpenH264 requires frame skipping for bitrate RC to enforce the target bitrate.
            // Keep the crate default skip-frame behavior instead of overriding it to false.
            .rate_control_mode(openh264::encoder::RateControlMode::Bitrate)
            .num_threads(1)
            .vui(openh264::encoder::VuiConfig::srgb().full_range(true));
        let encoder = openh264::encoder::Encoder::with_api_config(
            openh264::OpenH264API::from_source(),
            encoder_config,
        )
        .map_err(|error| MediaError::Backend(format!("初始化 OpenH264 编码器失败: {error}")))?;

        Ok(Self {
            config,
            encoder,
            rgb_buffer: Vec::new(),
        })
    }
}

impl VideoEncoder for OpenH264Encoder {
    fn encode(&mut self, frame: RawVideoFrame<'_>) -> MediaResult<EncodedVideoFrame> {
        validate_raw_frame(self.config, frame)?;

        self.rgb_buffer.clear();
        bgra_to_rgb(frame, &mut self.rgb_buffer)?;
        let rgb = openh264::formats::RgbSliceU8::new(
            &self.rgb_buffer,
            (frame.width as usize, frame.height as usize),
        );
        let yuv = openh264::formats::YUVBuffer::from_rgb8_source(rgb);
        let bitstream = self
            .encoder
            .encode_at(
                &yuv,
                openh264::Timestamp::from_millis(frame.timestamp_ns / 1_000_000),
            )
            .map_err(|error| MediaError::Backend(format!("OpenH264 编码失败: {error}")))?;
        let bytes = bitstream.to_vec();
        if bytes.is_empty() {
            return Err(MediaError::Backend("OpenH264 编码器输出空载荷".to_owned()));
        }

        Ok(EncodedVideoFrame {
            codec: VideoCodec::H264,
            width: frame.width,
            height: frame.height,
            sequence_number: frame.sequence_number,
            timestamp_ns: frame.timestamp_ns,
            keyframe: matches!(
                bitstream.frame_type(),
                openh264::encoder::FrameType::IDR | openh264::encoder::FrameType::I
            ),
            bytes,
        })
    }

    fn request_keyframe(&mut self) -> MediaResult<()> {
        self.encoder.force_intra_frame();
        Ok(())
    }
}

#[derive(Debug)]
/// Test-only H.264 boundary encoder; payloads are deterministic stubs, not playable H.264 bitstreams.
pub struct FakeH264Encoder {
    config: VideoPipelineConfig,
    force_keyframe: bool,
}

impl FakeH264Encoder {
    pub fn new(config: VideoPipelineConfig) -> MediaResult<Self> {
        config.validate()?;

        Ok(Self {
            config,
            force_keyframe: true,
        })
    }
}

impl VideoEncoder for FakeH264Encoder {
    fn encode(&mut self, frame: RawVideoFrame<'_>) -> MediaResult<EncodedVideoFrame> {
        validate_raw_frame(self.config, frame)?;

        let keyframe = self.force_keyframe;
        self.force_keyframe = false;

        let mut bytes = Vec::with_capacity(FAKE_H264_MAGIC.len() + 40);
        bytes.extend_from_slice(FAKE_H264_MAGIC);
        bytes.extend_from_slice(&frame.width.to_be_bytes());
        bytes.extend_from_slice(&frame.height.to_be_bytes());
        bytes.extend_from_slice(&frame.sequence_number.to_be_bytes());
        bytes.extend_from_slice(&frame.timestamp_ns.to_be_bytes());
        bytes.extend_from_slice(&(frame.bytes.len() as u64).to_be_bytes());
        bytes.extend_from_slice(&frame.bytes[..frame.bytes.len().min(16)]);

        if bytes.is_empty() {
            return Err(MediaError::Backend(
                "fake H.264 encoder produced empty payload".to_owned(),
            ));
        }

        Ok(EncodedVideoFrame {
            codec: VideoCodec::H264,
            width: frame.width,
            height: frame.height,
            sequence_number: frame.sequence_number,
            timestamp_ns: frame.timestamp_ns,
            keyframe,
            bytes,
        })
    }

    fn request_keyframe(&mut self) -> MediaResult<()> {
        self.force_keyframe = true;
        Ok(())
    }
}

fn validate_raw_frame(config: VideoPipelineConfig, frame: RawVideoFrame<'_>) -> MediaResult<()> {
    validate_config_dimensions(config)?;
    if frame.width == 0 || frame.height == 0 {
        return Err(MediaError::InvalidRawFrame(
            RawVideoFrameError::InvalidDimensions {
                width: frame.width,
                height: frame.height,
            },
        ));
    }

    if frame.width > MAX_H264_WIDTH || frame.height > MAX_H264_HEIGHT {
        return Err(MediaError::InvalidRawFrame(
            RawVideoFrameError::ResolutionTooLarge {
                width: frame.width,
                height: frame.height,
                max_width: MAX_H264_WIDTH,
                max_height: MAX_H264_HEIGHT,
            },
        ));
    }
    if !frame.width.is_multiple_of(2) || !frame.height.is_multiple_of(2) {
        return Err(MediaError::InvalidRawFrame(
            RawVideoFrameError::OddDimensions {
                width: frame.width,
                height: frame.height,
            },
        ));
    }

    if frame.width != config.width || frame.height != config.height {
        return Err(MediaError::InvalidRawFrame(
            RawVideoFrameError::ConfigDimensionMismatch {
                frame_width: frame.width,
                frame_height: frame.height,
                config_width: config.width,
                config_height: config.height,
            },
        ));
    }

    if frame.format != RawPixelFormat::Bgra8Unorm {
        return Err(MediaError::InvalidRawFrame(
            RawVideoFrameError::UnsupportedPixelFormat {
                format: frame.format,
            },
        ));
    }

    if frame.bytes.is_empty() {
        return Err(MediaError::InvalidRawFrame(
            RawVideoFrameError::EmptyPayload,
        ));
    }

    let min_row_pitch = frame
        .width
        .checked_mul(4)
        .ok_or(MediaError::InvalidRawFrame(
            RawVideoFrameError::RowPitchOverflow,
        ))?;
    if frame.row_pitch < min_row_pitch {
        return Err(MediaError::InvalidRawFrame(
            RawVideoFrameError::InvalidRowPitch {
                row_pitch: frame.row_pitch,
                min_row_pitch,
            },
        ));
    }

    let expected_len =
        frame
            .row_pitch
            .checked_mul(frame.height)
            .ok_or(MediaError::InvalidRawFrame(
                RawVideoFrameError::PayloadLengthOverflow,
            ))? as usize;
    if frame.bytes.len() != expected_len {
        return Err(MediaError::InvalidRawFrame(
            RawVideoFrameError::InvalidPayloadLength {
                actual: frame.bytes.len(),
                expected: expected_len,
            },
        ));
    }

    Ok(())
}

fn validate_config_dimensions(config: VideoPipelineConfig) -> MediaResult<()> {
    if !config.width.is_multiple_of(2) || !config.height.is_multiple_of(2) {
        return Err(MediaError::Config(crate::MediaConfigError::OddDimensions {
            width: config.width,
            height: config.height,
        }));
    }
    Ok(())
}

fn bgra_to_rgb(frame: RawVideoFrame<'_>, rgb: &mut Vec<u8>) -> MediaResult<()> {
    let width = frame.width as usize;
    let height = frame.height as usize;
    let row_pitch = frame.row_pitch as usize;
    rgb.reserve(width * height * 3);
    for row in 0..height {
        let start = row
            .checked_mul(row_pitch)
            .ok_or(MediaError::InvalidRawFrame(
                RawVideoFrameError::PayloadLengthOverflow,
            ))?;
        let end = start + width * 4;
        let source_row = frame
            .bytes
            .get(start..end)
            .ok_or(MediaError::InvalidRawFrame(
                RawVideoFrameError::InvalidPayloadLength {
                    actual: frame.bytes.len(),
                    expected: row_pitch * height,
                },
            ))?;
        for bgra in source_row.chunks_exact(4) {
            rgb.push(bgra[2]);
            rgb.push(bgra[1]);
            rgb.push(bgra[0]);
        }
    }
    Ok(())
}
