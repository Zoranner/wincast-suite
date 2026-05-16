use wincast_media::{
    EncodedVideoFrame, MediaConfigError, VideoDecoder, VideoEncoder, VideoLatencyMode,
    VideoPipelineConfig,
};
use wincast_protocol::config::VideoCodec;

#[test]
fn low_latency_h264_1080p_config_is_valid() {
    let config = VideoPipelineConfig {
        codec: VideoCodec::H264,
        width: 1920,
        height: 1080,
        fps: 30,
        bitrate_kbps: 8_000,
        max_bitrate_kbps: 12_000,
        latency_mode: VideoLatencyMode::LowLatency,
    };

    config
        .validate()
        .expect("1080p H.264 config should be valid");
}

#[test]
fn media_boundary_reuses_protocol_encoded_video_frame() {
    let frame = EncodedVideoFrame {
        codec: VideoCodec::H264,
        width: 1920,
        height: 1080,
        sequence_number: 7,
        timestamp_ns: 123_456,
        keyframe: true,
        bytes: vec![1, 2, 3],
    };

    frame
        .validate()
        .expect("protocol encoded frame should validate at media boundary");
}

#[test]
fn config_rejects_resolution_above_1080p() {
    let too_wide = VideoPipelineConfig {
        width: 1921,
        ..valid_config()
    };
    assert_eq!(
        too_wide.validate(),
        Err(MediaConfigError::ResolutionTooLarge {
            width: 1921,
            height: 1080,
            max_width: 1920,
            max_height: 1080,
        })
    );

    let too_tall = VideoPipelineConfig {
        height: 1081,
        ..valid_config()
    };
    assert_eq!(
        too_tall.validate(),
        Err(MediaConfigError::ResolutionTooLarge {
            width: 1920,
            height: 1081,
            max_width: 1920,
            max_height: 1080,
        })
    );
}

#[test]
fn config_rejects_invalid_fps_and_bitrate() {
    let zero_fps = VideoPipelineConfig {
        fps: 0,
        ..valid_config()
    };
    assert_eq!(
        zero_fps.validate(),
        Err(MediaConfigError::InvalidFps {
            fps: 0,
            max_fps: 60
        })
    );

    let high_fps = VideoPipelineConfig {
        fps: 61,
        ..valid_config()
    };
    assert_eq!(
        high_fps.validate(),
        Err(MediaConfigError::InvalidFps {
            fps: 61,
            max_fps: 60
        })
    );

    let zero_bitrate = VideoPipelineConfig {
        bitrate_kbps: 0,
        ..valid_config()
    };
    assert_eq!(
        zero_bitrate.validate(),
        Err(MediaConfigError::InvalidBitrate {
            bitrate_kbps: 0,
            max_bitrate_kbps: 12_000,
        })
    );

    let over_max_bitrate = VideoPipelineConfig {
        bitrate_kbps: 12_001,
        ..valid_config()
    };
    assert_eq!(
        over_max_bitrate.validate(),
        Err(MediaConfigError::InvalidBitrate {
            bitrate_kbps: 12_001,
            max_bitrate_kbps: 12_000,
        })
    );
}

#[test]
fn encoder_and_decoder_traits_use_protocol_frame_boundary() {
    fn assert_encoder<T: VideoEncoder>() {}
    fn assert_decoder<T: VideoDecoder>() {}

    struct MockEncoder;
    impl VideoEncoder for MockEncoder {
        fn encode(
            &mut self,
            _frame: wincast_media::RawVideoFrame<'_>,
        ) -> wincast_media::MediaResult<EncodedVideoFrame> {
            Ok(EncodedVideoFrame {
                codec: VideoCodec::H264,
                width: 1920,
                height: 1080,
                sequence_number: 1,
                timestamp_ns: 1,
                keyframe: true,
                bytes: vec![1],
            })
        }

        fn request_keyframe(&mut self) -> wincast_media::MediaResult<()> {
            Ok(())
        }
    }

    struct MockDecoder {
        bytes: Vec<u8>,
    }

    impl VideoDecoder for MockDecoder {
        fn decode<'a>(
            &'a mut self,
            frame: &EncodedVideoFrame,
        ) -> wincast_media::MediaResult<wincast_media::DecodedVideoFrame<'a>> {
            self.bytes.clear();
            self.bytes.extend_from_slice(frame.bytes.as_slice());

            Ok(wincast_media::DecodedVideoFrame {
                width: frame.width,
                height: frame.height,
                format: wincast_media::DecodedPixelFormat::Bgra8Unorm,
                bytes: self.bytes.as_slice(),
            })
        }
    }

    assert_encoder::<MockEncoder>();
    assert_decoder::<MockDecoder>();
}

fn valid_config() -> VideoPipelineConfig {
    VideoPipelineConfig {
        codec: VideoCodec::H264,
        width: 1920,
        height: 1080,
        fps: 30,
        bitrate_kbps: 8_000,
        max_bitrate_kbps: 12_000,
        latency_mode: VideoLatencyMode::LowLatency,
    }
}
