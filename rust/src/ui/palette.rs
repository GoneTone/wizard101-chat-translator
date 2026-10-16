//! 疊加視窗（overlay、泡泡、細捲軸）共用的深色配色。

use eframe::egui::Color32;

const fn hex(rgb: u32) -> Color32 {
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
/// 沒有遊戲色時的譯文。
pub const FG_TRANSLATED: Color32 = hex(0xf2f2f7);
/// 原文對譯文的調暗係數：原文是輔助資訊，壓暗到譯文之下讓視線先落在譯文上。
pub const DIM_FACTOR: f64 = 0.71;
/// 佔位中的譯文。
pub const FG_PENDING: Color32 = hex(0x9398a8);
/// 錯誤文字。
pub const FG_ERROR: Color32 = hex(0xff5f5f);
/// 更新橫幅的連結藍（深色底上要比設定視窗的連結亮一階）。
pub const FG_UPDATE: Color32 = hex(0x6fa8ff);
/// 標題列文字與按鈕。
pub const FG_BAR: Color32 = hex(0xc8c8d8);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_matches_python_color_strings() {
        assert_eq!(BG, Color32::from_hex("#101018").unwrap());
        assert_eq!(TAB_ACCENT, Color32::from_hex("#6fa8dc").unwrap());
    }
}
