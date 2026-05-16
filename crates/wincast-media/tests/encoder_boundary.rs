use wincast_media::{
    MediaError, RawPixelFormat, RawVideoFrame, RawVideoFrameError, VideoEncoder, VideoLatencyMode,
    VideoPipelineConfig, test_support::FakeH264Encoder,
};
use wincast_protocol::config::VideoCodec;

#[test]
fn fake_h264_encoder_outputs_protocol_frame_boundary() {
    let mut encoder = FakeH264Encoder::new(valid_config()).expect("config should be valid");
    let bytes = vec![0x11; 640 * 360 * 4];
    let raw = raw_frame(640, 360, 9, 123_456, &bytes);

    let encoded = encoder
        .encode(raw)
        .expect("fake encoder should produce frame");

    assert_eq!(encoded.codec, VideoCodec::H264);
    assert_eq!(encoded.width, 640);
    assert_eq!(encoded.height, 360);
    assert_eq!(encoded.sequence_number, 9);
    assert_eq!(encoded.timestamp_ns, 123_456);
    assert!(encoded.keyframe);
    assert!(!encoded.bytes.is_empty());
    encoded
        .validate()
        .expect("fake encoded frame should satisfy protocol validation");
}

#[test]
fn fake_h264_encoder_marks_next_frame_keyframe_after_request() {
    let mut encoder = FakeH264Encoder::new(valid_config()).expect("config should be valid");
    let bytes = vec![0x22; 640 * 360 * 4];

    let first = encoder
        .encode(raw_frame(640, 360, 1, 100, &bytes))
        .expect("first frame should encode");
    let second = encoder
        .encode(raw_frame(640, 360, 2, 200, &bytes))
        .expect("second frame should encode");
    encoder
        .request_keyframe()
        .expect("fake encoder should accept keyframe request");
    let third = encoder
        .encode(raw_frame(640, 360, 3, 300, &bytes))
        .expect("requested frame should encode");

    assert!(first.keyframe);
    assert!(!second.keyframe);
    assert!(third.keyframe);
}

#[test]
fn fake_h264_encoder_rejects_invalid_or_oversized_dimensions() {
    let zero_width = FakeH264Encoder::new(VideoPipelineConfig {
        width: 0,
        ..valid_config()
    });
    assert!(matches!(zero_width, Err(MediaError::Config(_))));

    let too_large = FakeH264Encoder::new(VideoPipelineConfig {
        width: 1921,
        ..valid_config()
    });
    assert!(matches!(too_large, Err(MediaError::Config(_))));
}

#[test]
fn fake_h264_encoder_rejects_empty_raw_payload() {
    let mut encoder = FakeH264Encoder::new(valid_config()).expect("config should be valid");
    let result = encoder.encode(RawVideoFrame {
        bytes: &[],
        ..raw_frame(640, 360, 1, 100, &[1, 2, 3, 4])
    });

    assert!(matches!(result, Err(MediaError::InvalidRawFrame(_))));
}

#[test]
fn fake_h264_encoder_rejects_raw_frame_size_mismatched_with_config() {
    let mut encoder = FakeH264Encoder::new(VideoPipelineConfig {
        width: 1920,
        height: 1080,
        ..valid_config()
    })
    .expect("config should be valid");
    let bytes = vec![0x22; 640 * 360 * 4];

    let result = encoder.encode(raw_frame(640, 360, 1, 100, &bytes));

    assert!(matches!(
        result,
        Err(MediaError::InvalidRawFrame(
            RawVideoFrameError::ConfigDimensionMismatch {
                frame_width: 640,
                frame_height: 360,
                config_width: 1920,
                config_height: 1080,
            }
        ))
    ));
}

#[test]
fn fake_h264_encoder_rejects_frames_that_cannot_produce_payload() {
    let mut encoder = FakeH264Encoder::new(VideoPipelineConfig {
        width: 640,
        height: 360,
        ..valid_config()
    })
    .expect("config should be valid");
    let result = encoder.encode(RawVideoFrame {
        row_pitch: 4,
        bytes: &[1, 2, 3, 4],
        ..raw_frame(640, 360, 1, 100, &[1, 2, 3, 4])
    });

    assert!(matches!(result, Err(MediaError::InvalidRawFrame(_))));
}

fn valid_config() -> VideoPipelineConfig {
    VideoPipelineConfig {
        codec: VideoCodec::H264,
        width: 640,
        height: 360,
        fps: 30,
        bitrate_kbps: 8_000,
        max_bitrate_kbps: 12_000,
        latency_mode: VideoLatencyMode::LowLatency,
    }
}

fn raw_frame<'a>(
    width: u32,
    height: u32,
    sequence_number: u64,
    timestamp_ns: u64,
    bytes: &'a [u8],
) -> RawVideoFrame<'a> {
    RawVideoFrame {
        width,
        height,
        row_pitch: width * 4,
        format: RawPixelFormat::Bgra8Unorm,
        sequence_number,
        timestamp_ns,
        bytes,
    }
}
