use wincast_media::{
    OpenH264Decoder, OpenH264Encoder, RawPixelFormat, RawVideoFrame, VideoDecoder, VideoEncoder,
    VideoLatencyMode, VideoPipelineConfig,
};
use wincast_protocol::config::VideoCodec;

#[test]
fn openh264_backend_encodes_and_decodes_bgra_frame() {
    let config = VideoPipelineConfig {
        codec: VideoCodec::H264,
        width: 16,
        height: 16,
        fps: 30,
        bitrate_kbps: 500,
        max_bitrate_kbps: 1_000,
        latency_mode: VideoLatencyMode::LowLatency,
    };
    let mut encoder = OpenH264Encoder::new(config).expect("OpenH264 encoder should initialize");
    let mut decoder = OpenH264Decoder::new().expect("OpenH264 decoder should initialize");
    let bgra = test_bgra_frame(16, 16);

    let encoded = encoder
        .encode(RawVideoFrame {
            width: 16,
            height: 16,
            row_pitch: 64,
            format: RawPixelFormat::Bgra8Unorm,
            sequence_number: 7,
            timestamp_ns: 123_000,
            bytes: &bgra,
        })
        .expect("OpenH264 should encode BGRA input");
    let decoded = decoder
        .decode(&encoded)
        .expect("OpenH264 should decode encoded frame");

    assert_eq!(encoded.codec, VideoCodec::H264);
    assert_eq!(encoded.sequence_number, 7);
    assert!(encoded.keyframe);
    assert!(!encoded.bytes.starts_with(b"WINCAST_FAKE_H264"));
    assert_eq!(decoded.width, 16);
    assert_eq!(decoded.height, 16);
    assert_eq!(decoded.row_pitch(), 64);
    assert_eq!(decoded.bytes.len(), 16 * 16 * 4);
}

fn test_bgra_frame(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        for x in 0..width {
            bytes.push((x * 11) as u8);
            bytes.push((y * 13) as u8);
            bytes.push(((x + y) * 7) as u8);
            bytes.push(0xff);
        }
    }
    bytes
}
