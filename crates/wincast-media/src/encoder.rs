use wincast_protocol::config::VideoCodec;

use crate::{
    EncodedVideoFrame, MAX_H264_HEIGHT, MAX_H264_WIDTH, MediaError, MediaResult, RawPixelFormat,
    RawVideoFrame, RawVideoFrameError, VideoEncoder, VideoPipelineConfig,
};

const FAKE_H264_MAGIC: &[u8] = b"WINCAST_FAKE_H264";

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
