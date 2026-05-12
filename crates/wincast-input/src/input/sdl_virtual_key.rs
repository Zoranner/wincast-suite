const SDL_KEY_A: u32 = b'a' as u32;
const SDL_KEY_Z: u32 = b'z' as u32;
const SDL_KEY_0: u32 = b'0' as u32;
const SDL_KEY_9: u32 = b'9' as u32;

pub(super) fn map_sdl_keycode_to_windows_virtual_key(code: u32) -> Option<u16> {
    match code {
        SDL_KEY_A..=SDL_KEY_Z => Some((code - 32) as u16),
        SDL_KEY_0..=SDL_KEY_9 => Some(code as u16),
        8 => Some(0x08),
        9 => Some(0x09),
        13 => Some(0x0D),
        27 => Some(0x1B),
        32 => Some(0x20),
        127 => Some(0x2E),
        1_073_741_897 => Some(0x2D),
        1_073_741_898 => Some(0x24),
        1_073_741_899 => Some(0x21),
        1_073_741_901 => Some(0x23),
        1_073_741_902 => Some(0x22),
        1_073_741_903 => Some(0x27),
        1_073_741_904 => Some(0x25),
        1_073_741_905 => Some(0x28),
        1_073_741_906 => Some(0x26),
        1_073_741_882..=1_073_741_893 => Some((0x70 + (code - 1_073_741_882)) as u16),
        1_073_742_048 => Some(0xA2),
        1_073_742_049 => Some(0xA0),
        1_073_742_050 => Some(0xA4),
        1_073_742_052 => Some(0xA3),
        1_073_742_053 => Some(0xA1),
        1_073_742_054 => Some(0xA5),
        _ => u16::try_from(code).ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_common_sdl_keycodes_to_windows_virtual_keys() {
        assert_eq!(
            map_sdl_keycode_to_windows_virtual_key(b'a' as u32),
            Some(0x41)
        );
        assert_eq!(
            map_sdl_keycode_to_windows_virtual_key(b'z' as u32),
            Some(0x5A)
        );
        assert_eq!(
            map_sdl_keycode_to_windows_virtual_key(b'7' as u32),
            Some(0x37)
        );
        assert_eq!(
            map_sdl_keycode_to_windows_virtual_key(1_073_741_904),
            Some(0x25)
        );
        assert_eq!(
            map_sdl_keycode_to_windows_virtual_key(1_073_741_906),
            Some(0x26)
        );
        assert_eq!(
            map_sdl_keycode_to_windows_virtual_key(1_073_741_882),
            Some(0x70)
        );
        assert_eq!(
            map_sdl_keycode_to_windows_virtual_key(1_073_742_048),
            Some(0xA2)
        );
    }
}
