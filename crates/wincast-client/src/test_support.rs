use std::sync::mpsc;

use wincast_protocol::{message::RawBgraReadbackFrame, raw_frame::RawBgraFrame};

use crate::stream::RawBgraStreamEvent;

pub(crate) fn raw_bgra_frame() -> RawBgraReadbackFrame {
    RawBgraReadbackFrame {
        width: 2,
        height: 2,
        stride_bytes: 8,
        texture_width: 2,
        texture_height: 2,
        row_pitch: 8,
        sequence_number: 0,
        timestamp_ns: 0,
        bytes: vec![0; 16],
    }
}

pub(crate) fn raw_binary_frame() -> RawBgraFrame {
    raw_binary_frame_with_sequence(0)
}

pub(crate) fn raw_binary_frame_with_sequence(sequence_number: u64) -> RawBgraFrame {
    RawBgraFrame {
        width: 2,
        height: 2,
        row_pitch: 8,
        sequence_number,
        timestamp_ns: sequence_number * 1_000_000,
        bytes: vec![0; 16],
    }
}

pub(crate) fn queued_raw_bgra_frames(
    frames: impl IntoIterator<Item = RawBgraFrame>,
) -> mpsc::Receiver<RawBgraStreamEvent> {
    let (sender, receiver) = mpsc::channel();
    for frame in frames {
        sender
            .send(RawBgraStreamEvent::Frame(frame))
            .expect("test frame should queue");
    }
    receiver
}

pub(crate) struct DuplexBuffer {
    read: std::io::Cursor<Vec<u8>>,
    pub(crate) written: Vec<u8>,
}

impl DuplexBuffer {
    pub(crate) fn new(read: Vec<u8>) -> Self {
        Self {
            read: std::io::Cursor::new(read),
            written: Vec::new(),
        }
    }
}

impl std::io::Read for DuplexBuffer {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.read.read(buf)
    }
}

impl std::io::Write for DuplexBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.written.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub(crate) struct FailingWriteStream {
    read: std::io::Cursor<Vec<u8>>,
}

impl FailingWriteStream {
    pub(crate) fn new(read: Vec<u8>) -> Self {
        Self {
            read: std::io::Cursor::new(read),
        }
    }
}

impl std::io::Read for FailingWriteStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.read.read(buf)
    }
}

impl std::io::Write for FailingWriteStream {
    fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "write failed",
        ))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
