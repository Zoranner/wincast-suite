#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClientRenderMode {
    SdlWindow,
    ProtocolOnly,
}

impl ClientRenderMode {
    pub(crate) fn for_current_platform() -> Self {
        if cfg!(target_os = "linux") {
            Self::SdlWindow
        } else {
            Self::ProtocolOnly
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_platform_uses_protocol_only_render_mode_outside_linux() {
        if !cfg!(target_os = "linux") {
            assert_eq!(
                ClientRenderMode::for_current_platform(),
                ClientRenderMode::ProtocolOnly
            );
        }
    }
}
