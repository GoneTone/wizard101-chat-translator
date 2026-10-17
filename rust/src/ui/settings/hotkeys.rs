//! 兩把熱鍵（呼出輸入框、框選翻譯）的設定列與錄製（設定視窗與首次設定精靈共用）。

use eframe::egui::{Label, Rect, RichText, Sense, Stroke, StrokeKind, TextStyle, TextWrapMode, Ui};
use eframe::egui::{WidgetText, vec2};

use super::env::{Round, SettingsEnv, SettingsEvent};
use super::theme;
use super::widgets::{keycap_row, secondary_button, setting_row};
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
/// 錄製結果被收下（`apply` 回 `true`）或錄製被放棄（`stop` 回 `true`）後，呼叫端要在 UI 執行緒呼叫
/// `env.cancel_recording()`，錄製器才會從鍵盤分派器移除。
pub struct HotkeyEditor {
    values: [String; 2],
    recording: Round,
    target: Option<HotkeyKind>,
}

impl HotkeyEditor {
    pub fn new(input: &str, region: &str) -> Self {
        Self {
            values: [input.to_owned(), region.to_owned()],
            recording: Round::new("hotkey capture"),
            target: None,
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
        if self.stop() {
            env.cancel_recording();
        }
        // 錄製時按的空白鍵／Enter 不可再觸發聚焦中的按鈕
        change.surrender_focus();
        let round = self.recording.start();
        self.target = Some(kind);
        env.record_hotkey(round);
    }

    /// 收下錄製結果：`esc` 或空字串不改值。回傳是否收下了本輪的結果。
    pub fn apply(&mut self, event: &SettingsEvent) -> bool {
        let SettingsEvent::HotkeyRecorded { round, combo } = event else {
            return false;
        };
        if !self.recording.finish(*round) {
            return false;
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
        true
    }

    /// 放棄進行中的錄製（之後回來的結果一律丟掉）；回傳原本是否在錄。
    pub fn stop(&mut self) -> bool {
        let was = self.recording.busy();
        self.recording.cancel();
        self.target = None;
        was
    }
}

/// 錄製中的提示：`ACCENT` 外框的框，不可聚焦也不可點，免得錄製時按的鍵觸發它。
fn press_key_prompt(ui: &mut Ui) {
    let text = RichText::new(t("button.press_key", &[]))
        .size(theme::CONTROL_TEXT_SIZE)
        .color(theme::TEXT);
    let galley = WidgetText::from(text.clone()).into_galley(
        ui,
        Some(TextWrapMode::Extend),
        f32::INFINITY,
        TextStyle::Button,
    );
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
