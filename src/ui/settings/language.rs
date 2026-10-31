//! 介面語言下拉與翻譯目標語言的常用選項（設定視窗與首次設定精靈共用）。

use eframe::egui::Ui;

use super::theme;
use super::widgets::{combo_box, focus_ring, full_text_tip, name_by_row};
use crate::i18n::{available_languages, current_language, language_name, t, translators};
use crate::runtime::updater::CROWDIN_URL;

/// 翻譯目標語言的常用選項：各語言的自稱，任何介面語言下都不翻譯。
pub const COMMON_LANGUAGES: [&str; 9] = [
    "繁體中文（台灣）",
    "简体中文（中国）",
    "English",
    "日本語",
    "한국어",
    "Español",
    "Português",
    "Deutsch",
    "Français",
];

/// `COMMON_LANGUAGES` 轉成 `editable_combo` 要的清單。
pub fn common_languages() -> Vec<String> {
    COMMON_LANGUAGES.map(str::to_owned).to_vec()
}

/// 介面語言下拉：唯讀、顯示自稱；回傳這一幀選到的另一個語言碼（選回目前的語言不算）。
pub fn ui_language_combo(ui: &mut Ui) -> Option<String> {
    let current = current_language();
    let mut chosen = None;
    let selected = language_name(&current);
    let response = combo_box(ui, "ui_language")
        .selected_text(selected.clone())
        .show_ui(ui, |ui| {
            for (code, name) in available_languages() {
                if ui.selectable_label(code == current, name).clicked() {
                    chosen = Some(code);
                }
            }
        })
        .response;
    let response = name_by_row(ui, response);
    full_text_tip(ui, &response, &selected, 0.0);
    focus_ring(ui, &response, theme::CONTROL_RADIUS);
    chosen.filter(|code| *code != current)
}

/// 介面語言那一列的說明：目前語言的譯者掛名（有的話），以及協助翻譯的連結（沒人翻的語言更需要）。
pub fn ui_language_hint() -> String {
    let link = format!("[{} ↗]({CROWDIN_URL})", t("credit.help_translate", &[]));
    let credit = translators(&current_language());
    if credit.is_empty() {
        link
    } else {
        format!("{} {credit} · {link}", t("credit.translators", &[]))
    }
}
