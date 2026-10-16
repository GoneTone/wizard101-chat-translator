//! 內嵌唯讀資源（icon、服務商標誌），與 `src/assets/` 共用同一份檔案。

/// 應用程式 icon（多尺寸 ICO）。
pub const ICON_ICO: &[u8] = include_bytes!("../../../src/assets/icon.ico");

/// 指定尺寸的 PNG icon；只提供 16 與 48，其他尺寸視為程式錯誤。
pub fn icon_png(size: u32) -> &'static [u8] {
    match size {
        16 => include_bytes!("../../../src/assets/icon_16.png"),
        48 => include_bytes!("../../../src/assets/icon_48.png"),
        _ => panic!("no embedded icon PNG for size {size}"),
    }
}

/// 服務商標誌（原生 32px PNG）；未知的服務商回 `None`。
pub fn provider_icon(key: &str) -> Option<&'static [u8]> {
    Some(match key {
        "openai" => include_bytes!("../../../src/assets/provider_openai_32.png"),
        "claude" => include_bytes!("../../../src/assets/provider_claude_32.png"),
        "gemini" => include_bytes!("../../../src/assets/provider_gemini_32.png"),
        "custom" => include_bytes!("../../../src/assets/provider_custom_32.png"),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";

    #[test]
    fn provider_icons_exist_for_all_providers() {
        for key in ["openai", "claude", "gemini", "custom"] {
            assert!(provider_icon(key).unwrap().starts_with(PNG_MAGIC), "{key}");
        }
        assert!(provider_icon("nope").is_none());
    }

    #[test]
    fn icon_pngs_have_native_sizes() {
        for size in [16u32, 48] {
            let d = icon_png(size);
            assert!(d.starts_with(PNG_MAGIC));
            assert_eq!(u32::from_be_bytes(d[16..20].try_into().unwrap()), size);
            assert_eq!(u32::from_be_bytes(d[20..24].try_into().unwrap()), size);
        }
    }

    #[test]
    fn ico_has_expected_sizes() {
        assert_eq!(&ICON_ICO[..4], b"\x00\x00\x01\x00");
        let count = u16::from_le_bytes([ICON_ICO[4], ICON_ICO[5]]) as usize;
        let sizes: Vec<u32> = (0..count)
            .map(|i| match ICON_ICO[6 + i * 16] {
                0 => 256,
                n => n as u32,
            })
            .collect();
        assert_eq!(sizes, [16, 20, 24, 32, 40, 48, 64, 128]);
    }
}
