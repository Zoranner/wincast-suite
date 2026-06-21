use serde::Deserialize;

use wincast_media::{VideoLatencyMode, VideoPipelineConfig};
use wincast_protocol::config::VideoCodec;

use crate::error::{UnityNativeError, UnityNativeResult};

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct UnityNativeConfig {
    pub(crate) listen_addr: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) fps: u32,
    #[serde(default = "default_bitrate_kbps")]
    pub(crate) bitrate_kbps: u32,
    #[serde(default)]
    pub(crate) max_bitrate_kbps: u32,
}

impl UnityNativeConfig {
    pub(crate) fn parse(config_json: &str) -> UnityNativeResult<Self> {
        let mut config: Self = serde_json::from_str(config_json)?;
        config.apply_defaults();
        config.validate()?;
        Ok(config)
    }

    pub(crate) fn video_pipeline_config(&self) -> VideoPipelineConfig {
        VideoPipelineConfig {
            codec: VideoCodec::H264,
            width: self.width,
            height: self.height,
            fps: self.fps,
            bitrate_kbps: self.bitrate_kbps,
            max_bitrate_kbps: self.max_bitrate_kbps,
            latency_mode: VideoLatencyMode::LowLatency,
        }
    }

    fn apply_defaults(&mut self) {
        if self.max_bitrate_kbps == 0 {
            self.max_bitrate_kbps = self.bitrate_kbps.saturating_mul(2);
        }
    }

    fn validate(&self) -> UnityNativeResult<()> {
        if self.listen_addr.trim().is_empty() {
            return Err(UnityNativeError::EmptyConfigField("listen_addr"));
        }
        if self.width == 0 {
            return Err(UnityNativeError::ZeroConfigField("width"));
        }
        if self.height == 0 {
            return Err(UnityNativeError::ZeroConfigField("height"));
        }
        if self.fps == 0 {
            return Err(UnityNativeError::ZeroConfigField("fps"));
        }
        if self.bitrate_kbps == 0 {
            return Err(UnityNativeError::ZeroConfigField("bitrate_kbps"));
        }
        if self.max_bitrate_kbps == 0 {
            return Err(UnityNativeError::ZeroConfigField("max_bitrate_kbps"));
        }
        if self.bitrate_kbps > self.max_bitrate_kbps {
            return Err(UnityNativeError::InvalidConfigField {
                field: "bitrate_kbps",
                reason: "must not exceed max_bitrate_kbps",
            });
        }

        Ok(())
    }
}

fn default_bitrate_kbps() -> u32 {
    4_000
}
