//! 基本分頁：介面語言、翻譯目標語言、預設服務摘要、兩把熱鍵與相關開關。

use eframe::egui::{Id, Label, Ui};

use super::{Page, SettingsWindow};
use crate::config::services::str_field;
use crate::i18n::t;
use crate::ui::settings::env::SettingsEnv;
use crate::ui::settings::fields::editable_combo;
use crate::ui::settings::language::{common_languages, ui_language_combo, ui_language_hint};
use crate::ui::settings::provider_picker::provider_icon;
use crate::ui::settings::widgets::{card, secondary_button, setting_row, toggle};

impl SettingsWindow {
    pub(super) fn basic_page(&mut self, ui: &mut Ui, env: &dyn SettingsEnv) {
        // 介面語言與翻譯目標語言最容易被搞混，同一張卡片相鄰擺放才看得出各管什麼
        let mut language = None;
        card(ui, |ui| {
            let hint = ui_language_hint();
            setting_row(ui, &t("field.ui_language", &[]), Some(&hint), |ui| {
                language = ui_language_combo(ui);
            });
            let hint = t("hint.language", &[]);
            setting_row(ui, &t("settings.target_language", &[]), Some(&hint), |ui| {
                let id = Id::unique("target_language");
                editable_combo(ui, id, &mut self.target_language, &common_languages());
            });
        });
        if let Some(code) = language {
            self.preview_language(&code);
        }

        let mut manage = false;
        card(ui, |ui| {
            setting_row(ui, &t("settings.default_service", &[]), None, |ui| {
                self.service_summary(ui);
                ui.add_space(8.0);
                manage = secondary_button(ui, &t("button.manage", &[])).clicked();
            });
        });
        if manage {
            self.switch_page(Page::Services, env);
        }

        self.hotkeys.cards(ui, env, &mut self.auto_show_input);
        card(ui, |ui| {
            let label = t("field.paste_hotkey", &[]);
            setting_row(ui, &label, None, |ui| {
                toggle(ui, &mut self.paste_hotkey, &label);
            });
        });
    }

    /// 草稿裡預設服務的圖示與名稱（未儲存的編輯也算）；沒有預設服務時顯示「—」。
    fn service_summary(&self, ui: &mut Ui) {
        let draft = self.services.draft();
        let Some(service) = draft.find(draft.default.as_deref()) else {
            ui.add(Label::new("—"));
            return;
        };
        let provider = str_field(service, "provider");
        provider_icon(ui, provider, 20.0);
        ui.add(Label::new(str_field(service, "name")));
    }
}
