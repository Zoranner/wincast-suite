use wincast_media::{
    EncodedVideoFrame, MediaError, VideoDecoder,
    test_support::{FAKE_H264_DECODED_PAYLOAD_LIMIT, FakeH264Decoder},
};
use wincast_protocol::config::VideoCodec;
use wincast_protocol::message::MAX_ENCODED_VIDEO_FRAME_BYTES;

#[test]
fn fake_h264_decoder_decodes_valid_protocol_frame_with_traceable_metadata() {
    let mut decoder = FakeH264Decoder::new();
    let encoded = h264_frame(640, 360, 42, 987_654_321, vec![0x65, 0x88, 0x21]);

    let decoded = decoder
        .decode(&encoded)
        .expect("valid H.264 frame should decode through VideoDecoder boundary");

    assert_eq!(decoded.width, 640);
    assert_eq!(decoded.height, 360);
    assert_eq!(decoded.row_pitch(), 640 * 4);
    assert_eq!(decoded.bytes.len(), decoded.row_pitch() as usize * 360);
    assert_eq!(
        &decoded.bytes[..12],
        &[
            0x65, 0x65, 0x65, 0xff, 0x88, 0x88, 0x88, 0xff, 0x21, 0x21, 0x21, 0xff
        ]
    );
}

#[test]
fn fake_h264_decoder_reuses_internal_output_buffer_for_same_size_frames() {
    let mut decoder = FakeH264Decoder::new();
    let first = h264_frame(2, 2, 1, 10, vec![0x11]);
    let second = h264_frame(2, 2, 2, 20, vec![0x22]);

    let first_ptr = {
        let decoded = decoder
            .decode(&first)
            .expect("first valid H.264 frame should decode");
        assert_eq!(decoded.bytes.len(), decoded.row_pitch() as usize * 2);
        decoded.bytes.as_ptr()
    };

    let second_ptr = {
        let decoded = decoder
            .decode(&second)
            .expect("second valid H.264 frame should decode");
        assert_eq!(decoded.bytes.len(), decoded.row_pitch() as usize * 2);
        assert_eq!(
            &decoded.bytes[..8],
            &[0x22, 0x22, 0x22, 0xff, 0x22, 0x22, 0x22, 0xff]
        );
        decoded.bytes.as_ptr()
    };

    assert_eq!(
        first_ptr, second_ptr,
        "fake decoder should reuse its owned output buffer instead of leaking a new slice"
    );
}

#[test]
fn fake_h264_decoder_rejects_non_h264_frame() {
    let mut decoder = FakeH264Decoder::new();
    let encoded = EncodedVideoFrame {
        codec: VideoCodec::RawBgra,
        ..h264_frame(640, 360, 1, 2, vec![1])
    };

    assert!(matches!(
        decoder.decode(&encoded),
        Err(MediaError::UnsupportedEncodedCodec {
            codec: VideoCodec::RawBgra
        })
    ));
}

#[test]
fn fake_h264_decoder_rejects_empty_payload() {
    let mut decoder = FakeH264Decoder::new();
    let encoded = h264_frame(640, 360, 1, 2, Vec::new());

    assert!(matches!(
        decoder.decode(&encoded),
        Err(MediaError::InvalidEncodedFrame(_))
    ));
}

#[test]
fn fake_h264_decoder_rejects_invalid_dimensions() {
    let mut decoder = FakeH264Decoder::new();
    let encoded = h264_frame(0, 360, 1, 2, vec![1]);

    assert!(matches!(
        decoder.decode(&encoded),
        Err(MediaError::InvalidEncodedFrame(_))
    ));
}

#[test]
fn fake_h264_decoder_rejects_encoded_payload_above_protocol_limit() {
    let mut decoder = FakeH264Decoder::new();
    let encoded = h264_frame(
        640,
        360,
        1,
        2,
        vec![0x55; MAX_ENCODED_VIDEO_FRAME_BYTES + 1],
    );

    assert!(matches!(
        decoder.decode(&encoded),
        Err(MediaError::InvalidEncodedFrame(_))
    ));
}

#[test]
fn fake_h264_decoder_rejects_decoded_payload_above_fake_limit() {
    let mut decoder = FakeH264Decoder::new();
    let encoded = h264_frame(1920, 1081, 1, 2, vec![0x55]);

    assert!(matches!(
        decoder.decode(&encoded),
        Err(MediaError::DecodedPayloadTooLarge {
            actual,
            max: FAKE_H264_DECODED_PAYLOAD_LIMIT
        }) if actual == 1920 * 1081 * 4
    ));
}

fn h264_frame(
    width: u32,
    height: u32,
    sequence_number: u64,
    timestamp_ns: u64,
    bytes: Vec<u8>,
) -> EncodedVideoFrame {
    EncodedVideoFrame {
        codec: VideoCodec::H264,
        width,
        height,
        sequence_number,
        timestamp_ns,
        keyframe: true,
        bytes,
    }
}
