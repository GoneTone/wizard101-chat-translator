//! 設定視窗與首次設定精靈的配色、尺寸、字型與圖示碼位。

use eframe::egui::epaint::Shadow;
use eframe::egui::{
    Color32, CornerRadius, FontFamily, FontId, Margin, Stroke, Style, TextStyle, Vec2, Visuals,
    vec2,
};

use crate::ui::fonts_egui::{BOLD, ICONS};
use crate::ui::palette::{self, hex};

/// 內容區底色。
pub const WINDOW_BG: Color32 = hex(0x15151f);
/// 側欄、精靈步驟條以外的深底。
pub const SIDEBAR_BG: Color32 = palette::BG;
/// 選中的導覽項。
pub const NAV_SELECTED: Color32 = palette::BAR;
/// 未選中的導覽項文字、關著的開關圓點。
pub const NAV_TEXT: Color32 = hex(0xb8bccb);
/// 卡片、對話框。
pub const CARD_BG: Color32 = hex(0x1c1c2a);
/// 卡片外框、分隔線。
pub const CARD_BORDER: Color32 = hex(0x2a2a3e);
/// 按鈕、輸入框、下拉。
pub const CONTROL_BG: Color32 = hex(0x262638);
/// 控制項外框、捲軸。
pub const CONTROL_BORDER: Color32 = hex(0x3a3a55);
/// 輸入框底線（聚焦時改 `ACCENT`、2 寬）。
pub const CONTROL_UNDERLINE: Color32 = hex(0x6a6a88);
/// 主要按鈕、開關、選中指示、焦點框。
pub const ACCENT: Color32 = palette::TAB_ACCENT;
/// 主要按鈕與開關上的字。
pub const ON_ACCENT: Color32 = hex(0x0b1220);
/// 一般文字。
pub const TEXT: Color32 = hex(0xf2f2f7);
/// 說明文字。
pub const TEXT_SECONDARY: Color32 = hex(0xa3a7b8);
/// 版本號、步驟未到。
pub const TEXT_MUTED: Color32 = hex(0x8a8aa0);
/// 連結。
pub const LINK: Color32 = hex(0x7fb2ff);
/// 成功。
pub const OK: Color32 = hex(0x6fd39a);
/// 失敗。
pub const ERROR: Color32 = hex(0xff8a8a);
/// 刪除類按鈕的紅：確認按鈕的底色、卡片〔刪除〕與刪列 ✕ 的字色。
pub const DANGER: Color32 = hex(0xff7a7a);
/// 刪除確認按鈕上的字。
pub const ON_DANGER: Color32 = hex(0x1a0b0b);
/// 儲存被擋下的提示底色。
pub const WARN_BG: Color32 = hex(0x3a2a14);
/// 儲存被擋下的提示外框。
pub const WARN_BORDER: Color32 = hex(0x6b4a1c);
/// 儲存被擋下的提示文字。
pub const WARN_TEXT: Color32 = hex(0xffd9a3);
/// 服務卡片角色標記的底色。
pub const PILL_BG: Color32 = hex(0x22314a);
/// 服務卡片角色標記的文字。
pub const PILL_TEXT: Color32 = hex(0xa9cdf5);
/// 熱鍵按鍵帽底色。
pub const KEYCAP_BG: Color32 = hex(0x2b2b42);
/// 熱鍵按鍵帽外框。
pub const KEYCAP_BORDER: Color32 = hex(0x40405c);
/// hover／按下的底色：服務商卡片（外框改 `ACCENT`）與按鈕、輸入框、下拉等元件。
pub const HOT_BG: Color32 = hex(0x24304a);
/// 未選中的導覽項 hover。
pub const NAV_HOVER: Color32 = hex(0x1c1c2c);
/// 對話框按鈕列底、精靈步驟條底。
pub const MODAL_FOOT: Color32 = hex(0x181826);
/// 對話框背後的遮罩：rgba(6,6,12,0.62)。
pub const SCRIM: Color32 = Color32::from_rgba_unmultiplied_const(6, 6, 12, 158);

/// 設定視窗預設大小。
pub const SETTINGS_SIZE: Vec2 = vec2(840.0, 660.0);
/// 設定視窗最小大小。
pub const SETTINGS_MIN_SIZE: Vec2 = vec2(640.0, 360.0);
/// 精靈預設大小。
pub const WIZARD_SIZE: Vec2 = vec2(600.0, 760.0);
/// 精靈最小大小。
pub const WIZARD_MIN_SIZE: Vec2 = vec2(600.0, 380.0);
/// 側欄寬。
pub const SIDEBAR_WIDTH: f32 = 200.0;
/// 導覽項高。
pub const NAV_HEIGHT: f32 = 40.0;
/// 內容區內距。
pub const CONTENT_MARGIN: Margin = Margin {
    left: 24,
    right: 28,
    top: 20,
    bottom: 16,
};
/// 頁標題字級（粗體）。
pub const TITLE_SIZE: f32 = 22.0;
/// 卡片圓角。
pub const CARD_RADIUS: u8 = 8;
/// 卡片間距。
pub const CARD_GAP: f32 = 10.0;
/// 設定列最小高。
pub const ROW_MIN_HEIGHT: f32 = 48.0;
/// 設定列內距（左右、上下）。
pub const ROW_PADDING: (i8, i8) = (16, 10);
/// 控制項高。
pub const CONTROL_HEIGHT: f32 = 32.0;
/// 控制項圓角。
pub const CONTROL_RADIUS: u8 = 6;
/// 下拉寬。
pub const COMBO_WIDTH: f32 = 240.0;
/// 下拉清單項目之間的間距。
pub const POPUP_ITEM_GAP: f32 = 2.0;
/// 下拉箭頭中心到控制項右緣的距離。
pub const CHEVRON_INSET: f32 = 14.0;
/// 數值欄寬（含 −／＋）。
pub const NUMBER_WIDTH: f32 = 132.0;
/// 滑桿寬。
pub const SLIDER_WIDTH: f32 = 200.0;
/// 開關大小。
pub const TOGGLE_SIZE: Vec2 = vec2(40.0, 20.0);
/// 設定視窗底部按鈕列高。
pub const FOOTER_HEIGHT: f32 = 60.0;
/// 精靈底部按鈕列高。
pub const WIZARD_FOOTER_HEIGHT: f32 = 64.0;
/// 對話框圓角。
pub const DIALOG_RADIUS: u8 = 10;
/// 服務表單對話框寬。
pub const SERVICE_DIALOG_WIDTH: f32 = 540.0;
/// 服務商選擇對話框寬。
pub const PICKER_DIALOG_WIDTH: f32 = 460.0;
/// 下載模型對話框寬。
pub const DOWNLOAD_DIALOG_WIDTH: f32 = 680.0;
/// 確認對話框寬。
pub const CONFIRM_DIALOG_WIDTH: f32 = 280.0;
/// 內文字級。
pub const TEXT_SIZE: f32 = 14.0;
/// 精靈格狀服務商卡片的標題字級（粗體）。
pub const PROVIDER_TITLE_SIZE: f32 = 14.5;
/// 說明字級。
pub const HINT_SIZE: f32 = 12.5;
/// 按鈕與輸入框字級。
pub const CONTROL_TEXT_SIZE: f32 = 13.5;

/// Segoe Fluent Icons／MDL2 Assets 共用的圖示碼位。
pub mod icon {
    pub const BASIC: char = '\u{E713}';
    pub const SERVICES: char = '\u{E774}';
    pub const GLOSSARIES: char = '\u{E82D}';
    pub const ADVANCED: char = '\u{E9E9}';
    pub const ABOUT: char = '\u{E946}';
    pub const EXPAND: char = '\u{E70D}';
    pub const COLLAPSE: char = '\u{E70E}';
    pub const NEXT: char = '\u{E76C}';
    pub const BACK: char = '\u{E76B}';
    pub const SUCCESS: char = '\u{E73E}';
    pub const REFRESH: char = '\u{E72C}';
    pub const EXTERNAL_LINK: char = '\u{E8A7}';
    pub const OPEN_FOLDER: char = '\u{E838}';
    pub const CHECK_UPDATE: char = '\u{E895}';
    pub const SHOW_KEY: char = '\u{E890}';
    pub const HIDE_KEY: char = '\u{ED1A}';
    pub const FAILURE: char = '\u{E783}';
    pub const WARNING: char = '\u{E7BA}';
    pub const DECREASE: char = '\u{E738}';
    pub const INCREASE: char = '\u{E710}';
    pub const CLOSE: char = '\u{E8BB}';
    pub const NEW_DOCUMENT: char = '\u{E8A5}';
    pub const IMPORT: char = '\u{E8B5}';
}

/// 粗體字型；要先 `fonts_egui::install`，否則 egui 找不到字族會 panic。
pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(BOLD.into()))
}

/// 圖示字型；要先 `fonts_egui::install`，否則 egui 找不到字族會 panic。
pub fn icons(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(ICONS.into()))
}

/// 把設定介面的深色樣式套到設定視窗／精靈根 Ui：`theme::apply(ui.style_mut())`。
///
/// 不可套到 `Context` 全域樣式：overlay、泡泡、輸入框與框選卡片共用同一個 `Context`。
pub fn apply(style: &mut Style) {
    let control = CornerRadius::same(CONTROL_RADIUS);
    let mut v = Visuals::dark();
    v.override_text_color = Some(TEXT);
    v.panel_fill = WINDOW_BG;
    v.window_fill = CARD_BG;
    v.window_stroke = Stroke::new(1.0, CONTROL_BORDER);
    v.window_corner_radius = CornerRadius::same(DIALOG_RADIUS);
    v.menu_corner_radius = CornerRadius::same(CARD_RADIUS);
    v.extreme_bg_color = CONTROL_BG;
    v.faint_bg_color = CARD_BG;
    v.hyperlink_color = LINK;
    v.selection.bg_fill = palette::SELECT_BG;
    v.selection.stroke = Stroke::new(2.0, ACCENT);
    v.text_cursor.stroke = Stroke::new(2.0, TEXT);
    let shadow = Shadow {
        offset: [0, 10],
        blur: 28,
        spread: 0,
        color: Color32::from_black_alpha(128),
    };
    v.window_shadow = shadow;
    v.popup_shadow = shadow;
    v.widgets.noninteractive.bg_fill = CARD_BG;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, CARD_BORDER);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    for (state, fill, border) in [
        (&mut v.widgets.inactive, CONTROL_BG, CONTROL_BORDER),
        (&mut v.widgets.hovered, HOT_BG, CONTROL_UNDERLINE),
        (&mut v.widgets.active, HOT_BG, ACCENT),
        (&mut v.widgets.open, CONTROL_BG, ACCENT),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::new(1.0, border);
        state.fg_stroke = Stroke::new(1.0, TEXT);
        state.corner_radius = control;
        state.expansion = 0.0;
    }
    style.visuals = v;

    let text = [
        (TextStyle::Body, FontId::proportional(TEXT_SIZE)),
        (TextStyle::Button, FontId::proportional(CONTROL_TEXT_SIZE)),
        (TextStyle::Small, FontId::proportional(HINT_SIZE)),
        (TextStyle::Monospace, FontId::monospace(CONTROL_TEXT_SIZE)),
        (TextStyle::Heading, bold(TITLE_SIZE)),
    ];
    style.text_styles.extend(text);
    style.spacing.interact_size.y = CONTROL_HEIGHT;
    style.spacing.button_padding = vec2(16.0, 0.0);
    style.spacing.combo_width = COMBO_WIDTH;
    style.spacing.item_spacing = vec2(8.0, 8.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_colours_come_from_the_overlay_palette() {
        assert_eq!(SIDEBAR_BG, Color32::from_hex("#101018").unwrap());
        assert_eq!(NAV_SELECTED, Color32::from_hex("#23233a").unwrap());
        assert_eq!(ACCENT, Color32::from_hex("#6fa8dc").unwrap());
    }
}
