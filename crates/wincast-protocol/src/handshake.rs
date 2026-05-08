use std::io::{Read, Write};

use thiserror::Error;

use crate::{
    frame::{FrameError, read_message, write_message},
    message::{ControlMessage, ErrorCode},
};

pub const PROTOCOL_VERSION: u16 = 1;

#[derive(Debug, Error)]
pub enum HandshakeError {
    #[error("控制消息读写失败: {0}")]
    Frame(#[from] FrameError),
    #[error("宿主端拒绝连接: {code:?}: {message}")]
    HostRejected { code: ErrorCode, message: String },
    #[error("协议版本不兼容")]
    UnsupportedVersion,
    #[error("握手消息无效")]
    InvalidMessage,
}

pub fn send_client_hello(writer: &mut impl Write) -> Result<(), HandshakeError> {
    write_message(
        writer,
        &ControlMessage::Hello {
            version: PROTOCOL_VERSION,
        },
    )?;
    Ok(())
}

pub fn accept_client_hello(
    reader: &mut impl Read,
    writer: &mut impl Write,
) -> Result<(), HandshakeError> {
    match read_message(reader)? {
        ControlMessage::Hello {
            version: PROTOCOL_VERSION,
        } => {
            write_message(
                writer,
                &ControlMessage::Hello {
                    version: PROTOCOL_VERSION,
                },
            )?;
            Ok(())
        }
        ControlMessage::Hello { .. } => {
            write_message(
                writer,
                &ControlMessage::Error {
                    code: ErrorCode::UnsupportedVersion,
                    message: "协议版本不兼容".to_owned(),
                },
            )?;
            Err(HandshakeError::UnsupportedVersion)
        }
        _ => Err(HandshakeError::InvalidMessage),
    }
}

pub fn read_host_hello(reader: &mut impl Read) -> Result<(), HandshakeError> {
    match read_message(reader)? {
        ControlMessage::Hello {
            version: PROTOCOL_VERSION,
        } => Ok(()),
        ControlMessage::Hello { .. } => Err(HandshakeError::UnsupportedVersion),
        ControlMessage::Error { code, message } => {
            Err(HandshakeError::HostRejected { code, message })
        }
        _ => Err(HandshakeError::InvalidMessage),
    }
}

pub fn reject_busy_client(writer: &mut impl Write) -> Result<(), HandshakeError> {
    write_message(
        writer,
        &ControlMessage::Error {
            code: ErrorCode::Busy,
            message: "宿主端已有客户端连接".to_owned(),
        },
    )?;
    Ok(())
}

pub fn send_start_session(writer: &mut impl Write) -> Result<(), HandshakeError> {
    write_message(writer, &ControlMessage::StartSession)?;
    Ok(())
}

pub fn read_start_session(reader: &mut impl Read) -> Result<(), HandshakeError> {
    match read_message(reader)? {
        ControlMessage::StartSession => Ok(()),
        _ => Err(HandshakeError::InvalidMessage),
    }
}

pub fn send_session_ready(
    writer: &mut impl Write,
    width: u32,
    height: u32,
) -> Result<(), HandshakeError> {
    write_message(writer, &ControlMessage::SessionReady { width, height })?;
    Ok(())
}

pub fn send_video_ready(writer: &mut impl Write) -> Result<(), HandshakeError> {
    write_message(writer, &ControlMessage::VideoReady)?;
    Ok(())
}

pub fn send_goodbye(writer: &mut impl Write) -> Result<(), HandshakeError> {
    write_message(writer, &ControlMessage::Goodbye)?;
    Ok(())
}
