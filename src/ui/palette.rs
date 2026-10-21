//! 疊加視窗（overlay、泡泡、細捲軸）共用的深色配色。

use eframe::egui::Color32;

/// `0xRRGGBB` 轉成不透明的 `Color32`。
pub const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// 底色（半透明底板）。
pub const BG: Color32 = hex(0x101018);
/// 標題列。
pub const BAR: Color32 = hex(0x23233a);
/// 縮放把手與泡泡外框。
pub const GRIP: Color32 = hex(0x3a3a55);
/// 更新橫幅底色。
pub const BG_UPDATE: Color32 = hex(0x1b2740);
/// 沒有遊戲色時的原文。
pub const FG_ORIGINAL: Color32 = hex(0xc0c0cd);
/// 沒有遊戲色時的譯文；也是輸入框的文字色。
pub const FG_TRANSLATED: Color32 = hex(0xf2f2f7);
/// 原文對譯文的調暗係數：原文是輔助資訊，壓暗到譯文之下讓視線先落在譯文上。
pub const DIM_FACTOR: f64 = 0.71;
/// 佔位中的譯文。
pub const FG_PENDING: Color32 = hex(0x9398a8);
/// 提示與等待中的次要文字（輸入框提示、等待遊戲的狀態）。
pub const FG_HINT: Color32 = hex(0x9a9aa8);
/// 錯誤文字。
pub const FG_ERROR: Color32 = hex(0xff5f5f);
/// 更新橫幅的連結藍（深色底上要比設定視窗的連結亮一階）。
pub const FG_UPDATE: Color32 = hex(0x6fa8ff);
/// 標題列文字與按鈕。
pub const FG_BAR: Color32 = hex(0xc8c8d8);
/// 滑過標題列 ✕ 的底色（Windows 關閉鈕的紅）。
pub const CLOSE_HOVER: Color32 = hex(0xc42b1c);
/// 字幕描邊色。
pub const OUTLINE: Color32 = hex(0x0a0a10);
/// 框選反白底色。
pub const SELECT_BG: Color32 = hex(0x2d4a7a);
/// 分頁列底色。
pub const TAB_BG: Color32 = hex(0x181828);
/// 分頁文字。
pub const TAB_FG: Color32 = hex(0x8a8aa0);
/// 目前分頁的文字。
pub const TAB_FG_ACTIVE: Color32 = hex(0xf2f2f7);
/// 目前分頁的底線。
pub const TAB_ACCENT: Color32 = hex(0x6fa8dc);
/// 標題列、分頁列與更新橫幅的固定不透明度：不跟著 overlay 不透明度設定變。
pub const BAR_ALPHA: f32 = 0.8;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_color_matches_its_expected_hex() {
        let expected = [
            (BG, "#101018"),
            (BAR, "#23233a"),
            (GRIP, "#3a3a55"),
            (BG_UPDATE, "#1b2740"),
            (FG_ORIGINAL, "#c0c0cd"),
            (FG_TRANSLATED, "#f2f2f7"),
            (FG_PENDING, "#9398a8"),
            (FG_ERROR, "#ff5f5f"),
            (FG_UPDATE, "#6fa8ff"),
            (FG_BAR, "#c8c8d8"),
            // 只有 Rust 版有這個名字（Python 的 palette.py 沒有）
            (CLOSE_HOVER, "#c42b1c"),
            (OUTLINE, "#0a0a10"),
            (SELECT_BG, "#2d4a7a"),
            (TAB_BG, "#181828"),
            (TAB_FG, "#8a8aa0"),
            (TAB_FG_ACTIVE, "#f2f2f7"),
            (TAB_ACCENT, "#6fa8dc"),
        ];
        for (color, hex_string) in expected {
            assert_eq!(
                color,
                Color32::from_hex(hex_string).unwrap(),
                "{hex_string}"
            );
        }
        assert_eq!(DIM_FACTOR, 0.71);
    }
}
