//! 兩把熱鍵（呼出輸入框、框選翻譯）的設定列與錄製（設定視窗與首次設定精靈共用）。

use eframe::egui::vec2;
use eframe::egui::{Label, Rect, RichText, Sense, Stroke, StrokeKind, TextStyle, Ui};

use super::env::{Round, SettingsEnv, SettingsEvent};
use super::text_setting;
use super::theme;
use super::widgets::{card, extended_galley, keycap_row, secondary_button, setting_row, toggle};
use crate::config::Config;
use crate::i18n::t;
use crate::logs::log;

/// 哪一把熱鍵。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotkeyKind {
    /// 呼出翻譯輸入框。
    Input,
    /// 框選翻譯。
    Region,
}

impl HotkeyKind {
    /// 設定檔裡的鍵。
    pub fn config_key(self) -> &'static str {
        match self {
            Self::Input => "hotkey",
            Self::Region => "region_hotkey",
        }
    }

    fn label_key(self) -> &'static str {
        match self {
            Self::Input => "settings.hotkey",
            Self::Region => "settings.region_hotkey",
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Input => 0,
            Self::Region => 1,
        }
    }
}

/// 兩把熱鍵的值與錄製狀態；同一時間只錄一格。
///
/// 錄製結果被收下（`apply`）或錄製被放棄（`stop`）後，錄製器還留在鍵盤分派器裡；呼叫端要在每幀（或
/// 手上有 env 時）呼叫 `release`，在 UI 執行緒把它移除。
pub struct HotkeyEditor {
    values: [String; 2],
    recording: Round,
    target: Option<HotkeyKind>,
    /// 錄製器待移除；`apply`／`stop` 手上沒有 env，留到 `release` 才呼叫 `env.cancel_recording()`。
    release_pending: bool,
}

impl HotkeyEditor {
    /// 取設定裡兩把熱鍵目前的值。
    pub fn from_config(cfg: &Config) -> Self {
        Self {
            values: [HotkeyKind::Input, HotkeyKind::Region]
                .map(|kind| text_setting(cfg, kind.config_key())),
            recording: Round::new("hotkey capture"),
            target: None,
            release_pending: false,
        }
    }

    /// 兩把熱鍵寫回設定。
    pub fn write_into(&self, cfg: &mut Config) {
        for kind in [HotkeyKind::Input, HotkeyKind::Region] {
            cfg.insert(kind.config_key().into(), self.value(kind).into());
        }
    }

    /// 兩把熱鍵相同會一起觸發，儲存前要擋下。
    pub fn conflict(&self) -> bool {
        self.value(HotkeyKind::Input) == self.value(HotkeyKind::Region)
    }

    /// 錄製器待移除就從鍵盤分派器拿掉。
    pub fn release(&mut self, env: &dyn SettingsEnv) {
        if std::mem::take(&mut self.release_pending) {
            env.cancel_recording();
        }
    }

    pub fn value(&self, kind: HotkeyKind) -> &str {
        &self.values[kind.index()]
    }

    /// 熱鍵那一列：按鍵帽加〔更改〕；錄製中換成「請按鍵」提示。按〔更改〕會先停掉另一格的錄製。
    pub fn row(&mut self, ui: &mut Ui, env: &dyn SettingsEnv, kind: HotkeyKind) {
        let recording = self.target == Some(kind) && self.recording.busy();
        let mut change = None;
        setting_row(ui, &t(kind.label_key(), &[]), None, |ui| {
            if recording {
                press_key_prompt(ui);
            } else {
                keycap_row(ui, &self.values[kind.index()]);
                ui.add_space(8.0);
                change = Some(secondary_button(ui, &t("button.change", &[])));
            }
        });
        let Some(change) = change.filter(|r| r.clicked()) else {
            return;
        };
        self.stop();
        self.release(env);
        // 錄製時按的空白鍵／Enter 不可再觸發聚焦中的按鈕
        change.surrender_focus();
        let round = self.recording.start();
        self.target = Some(kind);
        env.record_hotkey(round);
    }

    /// 兩張熱鍵卡片：輸入框熱鍵底下接 `field.auto_input` 開關，框選熱鍵自成一張（設定基本頁與精靈共用）。
    pub fn cards(&mut self, ui: &mut Ui, env: &dyn SettingsEnv, auto_show_input: &mut bool) {
        // 每把熱鍵底下緊接著它自己的選項，看得出哪個開關管哪個功能
        card(ui, |ui| {
            self.row(ui, env, HotkeyKind::Input);
            let label = t("field.auto_input", &[]);
            setting_row(ui, &label, None, |ui| {
                toggle(ui, auto_show_input, &label);
            });
        });
        card(ui, |ui| self.row(ui, env, HotkeyKind::Region));
    }

    /// 收下錄製結果：`esc` 或空字串不改值。
    pub fn apply(&mut self, event: &SettingsEvent) {
        let SettingsEvent::HotkeyRecorded { round, combo } = event else {
            return;
        };
        if !self.recording.finish(*round) {
            return;
        }
        if let Some(kind) = self.target.take()
            && !combo.is_empty()
            && combo != "esc"
        {
            log(format!(
                "[settings] {} recorded: {combo}",
                kind.config_key()
            ));
            self.values[kind.index()] = combo.clone();
        }
        self.release_pending = true;
    }

    /// 放棄進行中的錄製（之後回來的結果一律丟掉）。
    pub fn stop(&mut self) {
        self.release_pending |= self.recording.busy();
        self.recording.cancel();
        self.target = None;
    }
}

/// 錄製中的提示：`ACCENT` 外框的框，不可聚焦也不可點，免得錄製時按的鍵觸發它。
fn press_key_prompt(ui: &mut Ui) {
    let text = RichText::new(t("button.press_key", &[]))
        .size(theme::CONTROL_TEXT_SIZE)
        .color(theme::TEXT);
    let galley = extended_galley(ui, text.clone(), TextStyle::Button);
    let size = vec2(galley.size().x + 28.0, theme::CONTROL_HEIGHT);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect(
        rect,
        theme::CONTROL_RADIUS,
        theme::HOT_BG,
        Stroke::new(2.0, theme::ACCENT),
        StrokeKind::Inside,
    );
    let inner: Rect = rect.shrink2(vec2(14.0, 0.0));
    ui.put(inner, Label::new(text).selectable(false));
}
