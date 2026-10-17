//! 首次設定精靈的內容：介面語言 → 建立翻譯服務（選服務商 → 填表單 → 測試連線）→ 偏好設定；視窗本身由 App 托管。
//!
//! 第二步還沒有服務可編輯時先出服務商卡片，選過之後才建出表單：`form` 是 `None` 就是還在卡片。

mod steps;

use eframe::egui::{
    Align, Button, Id, Layout, Rect, Response, RichText, Role, Stroke, TextureHandle, Ui,
    UiBuilder, WidgetInfo, pos2, vec2,
};
use serde_json::Value;

use super::env::{SettingsEnv, SettingsEvent};
use super::hotkeys::{HotkeyEditor, HotkeyKind};
use super::service_form::ServiceForm;
use super::service_list::{Choice, ServiceDraft, picker_dialog};
use super::theme::{self, icon};
use super::widgets::{focus_ring, primary_button, right_aligned, secondary_icon_button};
use super::window::{flag_setting, text_setting};
use crate::config::Config;
use crate::config::services::{
    find, is_auto_name, new_service, provider, str_field, unique_name, validate_service,
};
use crate::i18n::{current_language, language_name, set_language, t};
use crate::logs::log;
use crate::py::pystr;

/// 步驟條高：上內距 22、圓點 24、下內距 18。
const STEPS_HEIGHT: f32 = 64.0;

/// 精靈的三個步驟，依顯示順序。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Step {
    Language,
    Service,
    Prefs,
}

impl Step {
    const ALL: [Step; 3] = [Self::Language, Self::Service, Self::Prefs];

    fn title_key(self) -> &'static str {
        match self {
            Self::Language => "wizard.step.language",
            Self::Service => "wizard.step.service",
            Self::Prefs => "wizard.step.prefs",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// 該步驟是否允許按〔下一步〕；`test_passed` 已含使用者明示略過測試的情形。
pub fn can_advance(step: Step, test_passed: bool, api_errors: &[String]) -> bool {
    match step {
        Step::Service => api_errors.is_empty() && test_passed,
        Step::Language | Step::Prefs => true,
    }
}

/// 精靈要 App 做的事。
#[derive(Clone, Debug, PartialEq)]
pub enum WizardAction {
    /// 精靈已呼叫 `set_language`，App 只需 relabel（如視窗標題）。
    PreviewLanguage,
    /// 走完了：整份設定（服務已依 id 併回清單並設為預設），由 App 存檔。精靈該關了。
    Finish(Config),
    /// 取消，不留半套設定。精靈該關了。
    Cancel,
}

/// 精靈的內容與編輯中的設定；`ui` 每幀畫一次並回傳這一幀要 App 做的事。
pub struct Wizard {
    cfg: Config,
    step: Step,
    /// 第二步編輯中的服務；`None` 是還在選服務商。
    form: Option<Box<ServiceForm>>,
    /// 表單上疊著服務商選擇（按了〔更改〕）。
    picking: bool,
    /// 使用者明示略過測試；改任何 API 欄位就作廢。
    skip_test: bool,
    target_language: String,
    hotkeys: HotkeyEditor,
    auto_show_input: bool,
    /// 按〔完成〕時兩把熱鍵相同；提示顯示到兩把不再相同為止。
    hotkeys_blocked: bool,
    /// 提示剛出現，下一次畫的時候捲到它。
    reveal_callout: bool,
    /// 下一幀要呼叫 `env.cancel_recording()`（`apply` 收下錄製結果時手上沒有 env）。
    release_recorder: bool,
    icon: Option<TextureHandle>,
}

impl Wizard {
    /// 從 `cfg` 開始；有服務可編輯（預設服務，沒有就第一筆）時直接建表單，否則第二步先出服務商卡片。
    pub fn new(cfg: Config) -> Self {
        let mut wizard = Self {
            step: Step::Language,
            form: None,
            picking: false,
            skip_test: false,
            target_language: text_setting(&cfg, "target_language"),
            hotkeys: HotkeyEditor::new(
                &text_setting(&cfg, HotkeyKind::Input.config_key()),
                &text_setting(&cfg, HotkeyKind::Region.config_key()),
            ),
            auto_show_input: flag_setting(&cfg, "auto_show_input"),
            hotkeys_blocked: false,
            reveal_callout: false,
            release_recorder: false,
            icon: None,
            cfg,
        };
        let default = wizard.cfg.get("default_service").and_then(Value::as_str);
        let existing = find(&wizard.cfg, default).map(|s| str_field(s, "id").to_owned());
        let services = ServiceDraft::from_config(&wizard.cfg).services;
        let first = services.first().map(|s| str_field(s, "id").to_owned());
        if let Some(id) = existing.or(first) {
            wizard.edit_service(&id);
        }
        wizard
    }

    pub fn step(&self) -> Step {
        self.step
    }

    /// 背景工作的結果：測試連線與模型清單轉給服務表單，熱鍵錄製結果給熱鍵列。
    pub fn apply(&mut self, event: &SettingsEvent) {
        match event {
            SettingsEvent::TestDone { .. } | SettingsEvent::ModelsDone { .. } => {
                if let Some(form) = &mut self.form {
                    form.apply(event);
                }
            }
            SettingsEvent::HotkeyRecorded { .. } => {
                self.release_recorder |= self.hotkeys.apply(event);
            }
            SettingsEvent::UpdateChecked { .. } => {}
        }
    }

    /// 取消（視窗 ✕ 或 Alt+F4）：停掉進行中的熱鍵錄製，回傳 `Cancel`。
    pub fn cancel(&mut self, env: &dyn SettingsEnv) -> WizardAction {
        self.release_recorder |= self.hotkeys.stop();
        self.release_recorder(env);
        log(format!("[ui] wizard cancelled at step {:?}", self.step));
        WizardAction::Cancel
    }

    /// 畫整個精靈：上方步驟條、中間可捲動的內容、底部按鈕列；回傳這一幀要 App 做的事。
    pub fn ui(&mut self, ui: &mut Ui, env: &dyn SettingsEnv) -> Option<WizardAction> {
        self.release_recorder(env);
        let full = ui.available_rect_before_wrap();
        ui.painter().rect_filled(full, 0, theme::WINDOW_BG);
        let bar_bottom = (full.top() + STEPS_HEIGHT).min(full.bottom());
        let footer_top = (full.bottom() - theme::WIZARD_FOOTER_HEIGHT).max(bar_bottom);
        let bar = Rect::from_min_max(full.min, pos2(full.right(), bar_bottom));
        let body = Rect::from_min_max(
            pos2(full.left(), bar_bottom),
            pos2(full.right(), footer_top),
        );
        let footer = Rect::from_min_max(pos2(full.left(), footer_top), full.max);

        self.step_bar(ui, bar);
        // 內容先畫：表單這一幀的編輯要先作廢「略過測試」，按鈕列才算得對
        let mut action = ui
            .scope_builder(UiBuilder::new().max_rect(body), |ui| {
                ui.set_clip_rect(body);
                self.body(ui, env)
            })
            .inner;
        action = action.or(self.footer(ui, env, footer));
        self.show_picker(ui);
        ui.advance_cursor_after_rect(full);
        action
    }

    fn release_recorder(&mut self, env: &dyn SettingsEnv) {
        if std::mem::take(&mut self.release_recorder) {
            env.cancel_recording();
        }
    }

    /// 以 `cfg` 裡 `id` 那一筆服務建表單；命名去重對照整份清單。
    fn edit_service(&mut self, id: &str) {
        let services = ServiceDraft::from_config(&self.cfg).services;
        self.form = services
            .iter()
            .find(|s| str_field(s, "id") == id)
            .map(|s| Box::new(ServiceForm::new(s.clone(), services.clone())));
    }

    fn pick_provider(&mut self, key: &str) {
        let services = ServiceDraft::from_config(&self.cfg).services;
        let service = new_service(key, &services);
        self.form = Some(Box::new(ServiceForm::new(service, services)));
        log(format!("[ui] wizard provider picked: {key}"));
    }

    fn can_go_next(&self) -> bool {
        match &self.form {
            // 沒有表單只可能停在第二步（第三步只能經由被擋住的〔下一步〕抵達）
            None => self.step != Step::Service,
            Some(form) => {
                let errors = validate_service(&form.api_values());
                can_advance(self.step, form.test_passed() || self.skip_test, &errors)
            }
        }
    }

    fn go_to(&mut self, step: Step, env: &dyn SettingsEnv) {
        if self.step == Step::Prefs && self.hotkeys.stop() {
            env.cancel_recording();
        }
        log(format!("[ui] wizard step {:?} -> {step:?}", self.step));
        self.step = step;
    }

    /// 介面語言換了：欄位先寫回設定，換語言後讓還沒動過的目標語言與服務名稱跟著換，表單重建（測試結果作廢）。
    fn change_language(&mut self, code: &str) {
        let old_default = language_name(&current_language());
        self.collect_into_cfg();
        // 依 id 認人：清單裡不只精靈編輯的這一筆，位置不可假設
        let edited = self
            .form
            .as_ref()
            .map(|f| str_field(&f.values(), "id").to_owned());
        let old_short_name = edited
            .as_deref()
            .and_then(|id| find(&self.cfg, Some(id)))
            .and_then(|s| provider(str_field(s, "provider")))
            .map(|p| p.short_name());
        set_language(code);
        self.cfg.insert("ui_language".into(), code.into());
        if self.cfg.get("target_language").and_then(Value::as_str) == Some(old_default.as_str()) {
            self.cfg
                .insert("target_language".into(), language_name(code).into());
        }
        if let (Some(id), Some(old_short_name)) = (&edited, old_short_name) {
            self.retitle(id, &old_short_name);
        }
        log(format!("[ui] wizard restarting with language {code}"));
        self.target_language = text_setting(&self.cfg, "target_language");
        self.skip_test = false;
        self.picking = false;
        if let Some(id) = edited {
            self.edit_service(&id);
        }
    }

    /// 服務名稱還是舊語言的自動短名就換成新語言的，一樣要去重。
    fn retitle(&mut self, id: &str, old_short_name: &str) {
        let services = ServiceDraft::from_config(&self.cfg).services;
        let Some(service) = services.iter().find(|s| str_field(s, "id") == id) else {
            return;
        };
        let Some(provider) = provider(str_field(service, "provider")) else {
            return;
        };
        if !is_auto_name(str_field(service, "name"), old_short_name) {
            return;
        }
        let name = unique_name(&provider.short_name(), &services, Some(id));
        let list = self.cfg.get_mut("services").and_then(Value::as_array_mut);
        if let Some(service) = list.and_then(|l| l.iter_mut().find(|s| str_field(s, "id") == id)) {
            service["name"] = name.into();
        }
    }

    /// 把欄位寫回設定（換語言與完成共用）；還在選服務商時沒有服務可寫，其餘欄位照寫。
    fn collect_into_cfg(&mut self) {
        if let Some(form) = &self.form {
            let service = form.values();
            let id = str_field(&service, "id").to_owned();
            let mut draft = ServiceDraft::from_config(&self.cfg);
            if draft.services.is_empty() {
                draft.slots = Default::default();
            }
            // 依 id 併回清單而不是整份取代：精靈也是設定壞掉時的救援路徑，不能刪掉其他服務與金鑰
            draft.apply_dialog_result(service);
            draft.default = Some(id);
            draft.write_into(&mut self.cfg);
        }
        let target = pystr::strip(&self.target_language);
        if !target.is_empty() {
            self.cfg.insert("target_language".into(), target.into());
        }
        let values = [
            (
                HotkeyKind::Input.config_key(),
                Value::from(self.hotkeys.value(HotkeyKind::Input)),
            ),
            ("auto_show_input", self.auto_show_input.into()),
            (
                HotkeyKind::Region.config_key(),
                self.hotkeys.value(HotkeyKind::Region).into(),
            ),
        ];
        for (key, value) in values {
            self.cfg.insert(key.into(), value);
        }
    }

    fn finish(&mut self, env: &dyn SettingsEnv) -> Option<WizardAction> {
        // 與設定視窗同一條驗證：兩把熱鍵相同會一起觸發
        if self.hotkeys.value(HotkeyKind::Input) == self.hotkeys.value(HotkeyKind::Region) {
            log("[ui] wizard finish blocked: error.hotkeys_same");
            self.hotkeys_blocked = true;
            self.reveal_callout = true;
            return None;
        }
        self.release_recorder |= self.hotkeys.stop();
        self.release_recorder(env);
        self.collect_into_cfg();
        self.cfg
            .insert("ui_language".into(), current_language().into());
        let default = self.cfg.get("default_service").and_then(Value::as_str);
        log(format!(
            "[ui] wizard finished (default_service={})",
            default.unwrap_or("None")
        ));
        Some(WizardAction::Finish(self.cfg.clone()))
    }

    /// 按鈕列：〔上一步〕靠左（第一步停用），〔下一步〕／〔完成〕靠右。
    fn footer(&mut self, ui: &mut Ui, env: &dyn SettingsEnv, rect: Rect) -> Option<WizardAction> {
        ui.painter().hline(
            rect.x_range(),
            rect.top(),
            Stroke::new(1.0, theme::CARD_BORDER),
        );
        let row = rect.shrink2(vec2(24.0, 0.0));
        let last = self.step == Step::Prefs;
        let can_back = self.step != Step::Language;
        let can_next = self.can_go_next();
        let (back, next) = ui
            .scope_builder(
                UiBuilder::new()
                    .max_rect(row)
                    .layout(Layout::left_to_right(Align::Center)),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let back = ui
                        .add_enabled_ui(can_back, |ui| {
                            secondary_icon_button(ui, icon::BACK, &t("button.back", &[]))
                        })
                        .inner
                        .clicked();
                    let next = right_aligned(ui, Id::unique("wizard_next"), |ui| {
                        ui.add_enabled_ui(can_next, |ui| next_button(ui, last))
                            .inner
                            .clicked()
                    });
                    (back, next)
                },
            )
            .inner;
        let index = self.step.index();
        if back && can_back {
            self.go_to(Step::ALL[index - 1], env);
        } else if next && can_next {
            if last {
                return self.finish(env);
            }
            self.go_to(Step::ALL[index + 1], env);
        }
        None
    }

    /// 表單上按了〔更改〕：疊上服務商選擇。
    fn show_picker(&mut self, ui: &Ui) {
        if !self.picking {
            return;
        }
        match picker_dialog(ui) {
            Choice::Open => {}
            Choice::Cancel => self.picking = false,
            Choice::Done(key) => {
                if let Some(form) = &mut self.form {
                    form.set_provider(key);
                }
                self.picking = false;
            }
        }
    }
}

/// 〔下一步〕（文字後接箭頭）或最後一步的〔完成〕。
fn next_button(ui: &mut Ui, finish: bool) -> Response {
    if finish {
        return primary_button(ui, &t("button.finish", &[]));
    }
    let text = t("button.next", &[]);
    let atoms = (
        RichText::new(&text)
            .font(theme::bold(theme::CONTROL_TEXT_SIZE))
            .color(theme::ON_ACCENT),
        RichText::new(icon::NEXT)
            .font(theme::icons(11.0))
            .color(theme::ON_ACCENT),
    );
    let response = ui.add(
        Button::new(atoms)
            .fill(theme::ACCENT)
            .stroke(Stroke::new(1.0, theme::ACCENT))
            .corner_radius(theme::CONTROL_RADIUS)
            .min_size(vec2(0.0, theme::CONTROL_HEIGHT)),
    );
    response.widget_info(|| WidgetInfo::labeled(Role::Button, ui.is_enabled(), &text));
    focus_ring(ui, &response, theme::CONTROL_RADIUS);
    response
}
