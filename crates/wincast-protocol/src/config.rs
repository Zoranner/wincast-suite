use std::{net::SocketAddr, path::Path};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("配置文件格式无效: {0}")]
    InvalidToml(String),
    #[error("缺少必填配置: {0}")]
    MissingField(&'static str),
    #[error("配置项 {field} 无效: {reason}")]
    InvalidValue {
        field: &'static str,
        reason: &'static str,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostConfig {
    pub listen: String,
    pub program: ProgramConfig,
    pub video: VideoConfig,
    pub capture: CaptureConfig,
}

impl HostConfig {
    pub fn from_toml_str(source: &str) -> Result<Self, ConfigError> {
        let config: Self =
            toml::from_str(source).map_err(|err| ConfigError::InvalidToml(err.to_string()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        validate_required("listen", &self.listen)?;
        self.listen
            .parse::<SocketAddr>()
            .map_err(|_| ConfigError::InvalidValue {
                field: "listen",
                reason: "必须是 host:port 格式的监听地址",
            })?;

        self.program.validate()?;
        self.video.validate()?;
        self.capture.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgramConfig {
    pub path: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub work_dir: String,
    pub startup_delay_ms: u64,
}

impl ProgramConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        validate_required("program.path", &self.path)?;
        validate_required("program.work_dir", &self.work_dir)?;

        if Path::new(&self.path).as_os_str().is_empty() {
            return Err(ConfigError::MissingField("program.path"));
        }

        if Path::new(&self.work_dir).as_os_str().is_empty() {
            return Err(ConfigError::MissingField("program.work_dir"));
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoConfig {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub codec: VideoCodec,
    pub bitrate_kbps: u32,
    pub max_bitrate_kbps: u32,
}

impl VideoConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        validate_non_zero("video.width", self.width)?;
        validate_non_zero("video.height", self.height)?;
        validate_non_zero("video.fps", self.fps)?;
        validate_non_zero("video.bitrate_kbps", self.bitrate_kbps)?;
        validate_non_zero("video.max_bitrate_kbps", self.max_bitrate_kbps)?;
        if self.bitrate_kbps > self.max_bitrate_kbps {
            return Err(ConfigError::InvalidValue {
                field: "video.bitrate_kbps",
                reason: "不能大于 video.max_bitrate_kbps",
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VideoCodec {
    #[serde(rename = "h264")]
    H264,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureConfig {
    pub first_frame_timeout_ms: u64,
}

impl CaptureConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        validate_non_zero_u64(
            "capture.first_frame_timeout_ms",
            self.first_frame_timeout_ms,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientConfig {
    pub host: String,
    pub port: u16,
}

impl ClientConfig {
    pub fn from_toml_str(source: &str) -> Result<Self, ConfigError> {
        let config: Self =
            toml::from_str(source).map_err(|err| ConfigError::InvalidToml(err.to_string()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        validate_required("host", &self.host)?;
        validate_non_zero_u16("port", self.port)?;
        Ok(())
    }

    pub fn endpoint(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

fn validate_required(field: &'static str, value: &str) -> Result<(), ConfigError> {
    if value.trim().is_empty() {
        return Err(ConfigError::MissingField(field));
    }

    Ok(())
}

fn validate_non_zero(field: &'static str, value: u32) -> Result<(), ConfigError> {
    if value == 0 {
        return Err(ConfigError::InvalidValue {
            field,
            reason: "必须大于 0",
        });
    }

    Ok(())
}

fn validate_non_zero_u16(field: &'static str, value: u16) -> Result<(), ConfigError> {
    if value == 0 {
        return Err(ConfigError::InvalidValue {
            field,
            reason: "必须大于 0",
        });
    }

    Ok(())
}

fn validate_non_zero_u64(field: &'static str, value: u64) -> Result<(), ConfigError> {
    if value == 0 {
        return Err(ConfigError::InvalidValue {
            field,
            reason: "必须大于 0",
        });
    }

    Ok(())
}
