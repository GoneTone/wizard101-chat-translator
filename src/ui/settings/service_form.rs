//! 單筆翻譯服務的編輯表單：名稱、服務商、該家的欄位與測試連線；設定視窗的對話框與首次設定精靈共用。

use std::time::Duration;

use eframe::egui::{
    Align, CornerRadius, Hyperlink, Id, Label, Layout, Rect, RichText, Role, Sense, Separator, Ui,
    WidgetInfo, pos2, vec2,
};
use serde_json::{Map, Value};

use super::env::{Round, SettingsEnv, SettingsEvent};
use super::fields::{editable_combo, secret_field, text_field};
use super::provider_picker::provider_icon;
use super::theme::{self, icon};
use super::widgets::{
    combo_box, focus_ring, outcome, rich, right_aligned, right_aligned_width, secondary_button,
    secondary_icon_button, toggle,
};
use crate::config::py::py_bool;
use crate::config::services::{
    EFFORT_AUTO, Provider, ServiceSpec, is_auto_name, known_effort, provider, str_field,
    unique_name, validate_endpoint_fields, validate_service,
};
use crate::i18n::t;
use crate::logs::log;
use crate::py::pystr;
use crate::translation::clients::{ModelInfo, ThinkingCaps};
use crate::translation::errors::{TranslateError, friendly_text};
use crate::translation::translator::{TEST_SAMPLE, TestOutcome};
use crate::ui::settings::thinking::{
    ThinkingSwitch, effective_thinking, effort_options, switch_state,
};

const LABEL_WIDTH: f32 = 96.0;
const COLUMN_GAP: f32 = 12.0;
const ROW_GAP: f32 = 10.0;
/// 開啟表單時的載入佔位最多顯示多久（秒）；超過就先給使用者操作，清單回來再更新。
const PLACEHOLDER_SECONDS: f64 = 1.5;
const EFFORT_WIDTH: f32 = 180.0;

/// `ServiceForm::ui` 這一幀的回報。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FormResponse {
    /// API 欄位（含換服務商）有變動；精靈用來作廢「略過測試」。
    pub changed: bool,
    /// 按了〔更改〕，由外層開服務商選擇。
    pub change_provider: bool,
}

enum ModelStatus {
    Idle,
    Found(usize),
    NoList,
    Error(String),
}

enum TestResult {
    Passed { seconds: String, translated: String },
    Failed(String),
}

/// 一筆服務的編輯表單；`values()` 回傳可直接存進清單的服務。
pub struct ServiceForm {
    id: String,
    provider: &'static Provider,
    services: Vec<Value>,
    name: String,
    api_key: String,
    model: String,
    base_url: String,
    thinking: bool,
    effort: String,
    key_revealed: bool,
    models: Vec<ModelInfo>,
    /// `models` 的 id，給模型下拉用。
    model_ids: Vec<String>,
    /// 上次記進 log 的（開關狀態、清單有無資訊、深度選項、生效深度），判定結果改變才再記。
    logged_caps: Option<(ThinkingSwitch, bool, Vec<&'static str>, String)>,
    model_status: ModelStatus,
    models_round: Round,
    /// 抓清單那一刻的觸發來源、服務商與網址，log 用。
    models_request: (&'static str, String, String),
    /// 上次自動載入清單時的（金鑰, 網址）；`None` 是表單開啟後還沒自動載入過。
    auto_endpoint: Option<(String, String)>,
    /// 表單已畫過第一幀（開啟時的自動載入只做一次）。
    opened: bool,
    /// 開啟時那次自動載入的佔位顯示到何時（egui 時間）；載入結束或換服務商就清掉。
    placeholder_until: Option<f64>,
    test_round: Round,
    /// 測試那一刻的服務商與模型，log 用。
    test_request: (String, String),
    test_result: Option<TestResult>,
    pending_change: bool,
}

impl ServiceForm {
    /// 編輯 `service`；`services` 是清單裡的全部服務，命名去重用（可含 `service` 自己）。
    ///
    /// `service` 的服務商必須是 `providers()` 裡的鍵。
    pub fn new(service: Value, services: Vec<Value>) -> Self {
        let text = |key: &str| {
            service
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned()
        };
        let key = text("provider");
        let provider = provider(&key).unwrap_or_else(|| panic!("unknown provider {key:?}"));
        let effort = known_effort(provider, str_field(&service, "effort")).to_owned();
        Self {
            id: text("id"),
            provider,
            services,
            name: text("name"),
            api_key: text("api_key"),
            model: text("model"),
            base_url: text("base_url"),
            thinking: service
                .get("thinking")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            effort,
            key_revealed: false,
            models: Vec::new(),
            model_ids: Vec::new(),
            logged_caps: None,
            model_status: ModelStatus::Idle,
            models_round: Round::new("model list"),
            models_request: Default::default(),
            auto_endpoint: None,
            opened: false,
            placeholder_until: None,
            test_round: Round::new("test connection"),
            test_request: Default::default(),
            test_result: None,
            pending_change: false,
        }
    }

    /// 完整的一筆服務：id、名稱、服務商與該家的欄位。
    pub fn values(&self) -> Value {
        let mut service = Map::new();
        service.insert("id".into(), self.id.clone().into());
        service.insert("name".into(), pystr::strip(&self.name).into());
        service.extend(self.api_map());
        Value::Object(service)
    }

    /// 測試連線與翻譯用的形狀：去掉 id 與名稱。
    pub fn api_values(&self) -> Value {
        Value::Object(self.api_map())
    }

    /// 按確定前要擋下的原因（錯誤文案 key）：名稱，加上測試連線也要擋的 [`Self::api_errors`]。
    pub fn errors(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if pystr::strip(&self.name).is_empty() {
            errors.push("error.need_name".to_string());
        }
        errors.extend(self.api_errors());
        errors
    }

    /// 測試連線前要擋下的原因：必填欄位，加上伺服器網址的格式；名稱不影響請求，不擋。
    ///
    /// 網址格式只在表單擋：`validate_service` 也決定啟動時要不要進精靈，不跟著加嚴。
    fn api_errors(&self) -> Vec<String> {
        let mut errors = validate_service(&self.api_values());
        let base_url = pystr::strip(&self.base_url);
        if self.provider.needs_base_url() && !base_url.is_empty() {
            errors.extend(base_url_error(base_url).map(String::from));
        }
        errors
    }

    /// 換服務商：名稱還是上一家的自動名才跟著換，欄位清空並套新服務商預設，測試結果與模型清單作廢。
    pub fn set_provider(&mut self, key: &str) {
        let next = provider(key).unwrap_or_else(|| panic!("unknown provider {key:?}"));
        if next.key == self.provider.key {
            return;
        }
        let previous = std::mem::replace(&mut self.provider, next);
        if is_auto_name(pystr::strip(&self.name), &previous.short_name()) {
            self.name = unique_name(&next.short_name(), &self.services, Some(&self.id));
        }
        let defaults = next.defaults();
        self.api_key.clear();
        self.model.clear();
        self.base_url.clear();
        self.thinking = defaults
            .get("thinking")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        self.effort = defaults
            .get("effort")
            .and_then(Value::as_str)
            .unwrap_or(EFFORT_AUTO)
            .to_owned();
        self.set_models(Vec::new());
        self.model_status = ModelStatus::Idle;
        self.models_round.invalidate();
        self.auto_endpoint = None;
        self.placeholder_until = None;
        self.void_test();
        self.pending_change = true;
        log(format!(
            "[settings] service form provider switched {} -> {}",
            previous.key, next.key
        ));
    }

    pub fn set_name(&mut self, name: &str) {
        name.clone_into(&mut self.name);
    }

    /// 目前顯示的是一次成功的測試，且之後沒改過 API 欄位。
    pub fn test_passed(&self) -> bool {
        matches!(self.test_result, Some(TestResult::Passed { .. }))
    }

    /// 收下測試連線與模型清單的結果；不是本輪或等待期間改過欄位的結果丟掉。
    pub fn apply(&mut self, event: &SettingsEvent) {
        match event {
            SettingsEvent::TestDone { round, result } if self.test_round.finish(*round) => {
                self.tested(result);
            }
            SettingsEvent::ModelsDone { round, result } if self.models_round.finish(*round) => {
                self.listed(result);
            }
            _ => {}
        }
    }

    /// 畫表單；測試連線翻成 `target_language`。
    pub fn ui(
        &mut self,
        ui: &mut Ui,
        env: &dyn SettingsEnv,
        target_language: &str,
    ) -> FormResponse {
        // 以服務 id 為作用域：換編輯另一筆服務時，開關動畫與下拉狀態不沿用上一筆的
        ui.push_id(self.id.clone(), |ui| self.body(ui, env, target_language))
            .inner
    }

    fn body(&mut self, ui: &mut Ui, env: &dyn SettingsEnv, target_language: &str) -> FormResponse {
        let before = self.api_values();
        let mut response = FormResponse {
            changed: std::mem::take(&mut self.pending_change),
            change_provider: false,
        };
        ui.spacing_mut().item_spacing.y = ROW_GAP;
        let now = ui.input(|i| i.time);
        if !std::mem::replace(&mut self.opened, true) {
            self.auto_refresh(env);
            if self.models_round.busy() {
                self.placeholder_until = Some(now + PLACEHOLDER_SECONDS);
            }
        }
        let placeholder =
            self.models_round.busy() && self.placeholder_until.is_some_and(|until| now < until);
        let form = ui.scope(|ui| {
            if placeholder {
                // 照常排版但不畫不收輸入，記下各元件的位置給佔位灰塊用，換成表單時位置不跳
                ui.set_invisible();
                ui.data_mut(|d| d.insert_temp(placeholder_id(), Vec::<(Rect, Block)>::new()));
            }
            self.rows(ui, env, target_language, before, &mut response);
        });
        if placeholder {
            paint_placeholder(ui, form.response.rect, &t("form.loading", &[]));
        }
        response
    }

    /// 表單的各列與測試連線；`before` 是這一幀開始時的 API 欄位，用來判斷這一幀有沒有改到。
    fn rows(
        &mut self,
        ui: &mut Ui,
        env: &dyn SettingsEnv,
        target_language: &str,
        before: Value,
        response: &mut FormResponse,
    ) {
        form_row(ui, &t("field.service_name", &[]), |ui, label| {
            let field = text_field(ui, &mut self.name).labelled_by(label);
            mark(ui, field.rect, Block::Control);
        });
        response.change_provider = self.provider_row(ui);
        let mut left_endpoint = false;
        if self.provider.needs_base_url() {
            form_row(ui, &t("field.base_url", &[]), |ui, label| {
                let field = text_field(ui, &mut self.base_url).labelled_by(label);
                mark(ui, field.rect, Block::Control);
                left_endpoint |= field.lost_focus();
                hint(ui, &t("hint.custom_endpoint", &[]));
            });
        }
        left_endpoint |= self.key_row(ui);
        if left_endpoint {
            self.auto_refresh(env);
        }
        if self.model_row(ui) {
            self.start_refresh(env, "manual");
        }
        let options = self.reconcile_thinking();
        let switch = self.switch();
        self.thinking_row(ui, switch);
        if effective_thinking(switch, self.thinking) {
            self.effort_row(ui, &options);
        }

        let after = self.api_values();
        if after != before {
            self.test_round.invalidate();
            // 失敗原因留著供對照；成功結果已不代表目前的欄位
            if self.test_passed() {
                self.test_result = None;
            }
            let endpoint =
                |api: &Value| (api.get("api_key").cloned(), api.get("base_url").cloned());
            if endpoint(&after) != endpoint(&before) {
                self.models_round.invalidate();
            }
            response.changed = true;
        }

        if self.test_row(ui) {
            self.start_test(env, target_language);
        }
    }

    fn api_map(&self) -> Map<String, Value> {
        let mut api = Map::new();
        api.insert("provider".into(), self.provider.key.into());
        for field in self.provider.defaults().into_iter().map(|(k, _)| k) {
            let value = match field.as_str() {
                "api_key" => pystr::strip(&self.api_key).into(),
                "model" => pystr::strip(&self.model).into(),
                "base_url" => pystr::strip(&self.base_url).into(),
                "thinking" => effective_thinking(self.switch(), self.thinking).into(),
                "effort" => self.effective_effort().into(),
                other => unreachable!("service field {other} has no form input"),
            };
            api.insert(field, value);
        }
        api
    }

    /// 作廢還沒回來與已顯示的測試結果。
    fn void_test(&mut self) {
        self.test_round.invalidate();
        self.test_result = None;
    }

    fn provider_row(&mut self, ui: &mut Ui) -> bool {
        let key = self.provider.key;
        let label = t(self.provider.label_key, &[]);
        form_row(ui, &t("field.provider", &[]), |ui, _| {
            ui.horizontal(|ui| {
                ui.set_min_height(theme::CONTROL_HEIGHT);
                provider_icon(ui, key, 20.0);
                mark(ui, ui.min_rect(), Block::Control);
                let name = ui.add(Label::new(label).truncate());
                mark(ui, name.rect, Block::Text);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let change = secondary_button(ui, &t("button.change", &[]));
                    mark(ui, change.rect, Block::Control);
                    change.clicked()
                })
                .inner
            })
            .inner
        })
    }

    /// 金鑰欄與取得金鑰連結；回傳金鑰欄是否剛失去焦點。
    fn key_row(&mut self, ui: &mut Ui) -> bool {
        let label = if self.provider.needs_base_url() {
            "field.api_key_optional"
        } else {
            "field.api_key"
        };
        form_row(ui, &t(label, &[]), |ui, label| {
            let (show, hide) = (t("button.show", &[]), t("button.hide", &[]));
            let field = secret_field(ui, &mut self.api_key, &mut self.key_revealed, &show, &hide)
                .labelled_by(label);
            mark(ui, full_width(ui, field.rect), Block::Control);
            let left = field.lost_focus();
            if let Some(url) = self.provider.key_url {
                let text = RichText::new(t("link.get_key", &[]))
                    .size(13.0)
                    .color(theme::LINK);
                let link = ui.add(Hyperlink::from_label_and_url(text, url));
                mark(ui, link.rect, Block::Text);
                if link.clicked() {
                    log(format!("[ui] opening API key page: {url}"));
                }
            }
            left
        })
    }

    /// 模型欄與〔更新清單〕；回傳是否按了更新。
    fn model_row(&mut self, ui: &mut Ui) -> bool {
        form_row(ui, &t("field.model", &[]), |ui, label| {
            let refresh = ui
                .horizontal(|ui| {
                    // 按鈕寬取上一幀量到的值，讓下拉吃掉剩下的寬度
                    let width_id = ui.scope_id().with("refresh_width");
                    let reserved = right_aligned_width(ui, width_id);
                    let gap = ui.spacing().item_spacing.x;
                    ui.spacing_mut().combo_width =
                        (ui.available_width() - reserved - gap).max(80.0);
                    let combo = ui.make_persistent_id("model");
                    let field = editable_combo(ui, combo, &mut self.model, &self.model_ids)
                        .labelled_by(label);
                    mark(ui, field.rect, Block::Control);
                    let busy = self.models_round.busy();
                    let text = t(
                        if busy {
                            "button.loading"
                        } else {
                            "button.refresh"
                        },
                        &[],
                    );
                    right_aligned(ui, width_id, |ui| {
                        let refresh = ui
                            .add_enabled_ui(!busy, |ui| {
                                secondary_icon_button(ui, icon::REFRESH, &text)
                            })
                            .inner;
                        mark(ui, refresh.rect, Block::Control);
                        refresh.clicked()
                    })
                })
                .inner;
            match &self.model_status {
                ModelStatus::Idle => hint(ui, &t("hint.model_idle", &[])),
                ModelStatus::Found(0) => hint(ui, &t("hint.model_empty", &[])),
                ModelStatus::Found(count) => {
                    hint(ui, &t("hint.model_found", &[("count", &count.to_string())]));
                }
                ModelStatus::NoList => hint(ui, &t("hint.model_no_list", &[])),
                ModelStatus::Error(text) => rich(ui, text, theme::HINT_SIZE, theme::ERROR),
            }
            refresh
        })
    }

    /// 目前模型欄對到清單那一筆的思考能力。
    fn caps(&self) -> Option<&ThinkingCaps> {
        let model = pystr::strip(&self.model);
        self.models
            .iter()
            .find(|m| m.id == model)
            .map(|m| &m.thinking)
    }

    fn switch(&self) -> ThinkingSwitch {
        switch_state(self.caps())
    }

    /// 實際生效的深度：使用者選的深度目前模型不支援時用自動，不覆蓋選擇，換回支援的模型時才能恢復。
    fn effective_effort(&self) -> &str {
        let options = effort_options(self.provider, self.caps());
        if options.contains(&self.effort.as_str()) {
            &self.effort
        } else {
            EFFORT_AUTO
        }
    }

    /// 算出目前模型的深度選項；開關狀態、選項或生效深度改變時記 log（打字途中的每個字不算）。
    fn reconcile_thinking(&mut self) -> Vec<&'static str> {
        let options = effort_options(self.provider, self.caps());
        let switch = self.switch();
        let known = self.caps().is_some_and(ThinkingCaps::is_known);
        let effort = self.effective_effort().to_owned();
        let resolved = (switch, known, options.clone(), effort);
        if self.logged_caps.as_ref() != Some(&resolved) {
            let name = match (switch, known) {
                (ThinkingSwitch::Unsupported, _) => "unsupported",
                (ThinkingSwitch::Always, _) => "always",
                (ThinkingSwitch::Toggle, true) => "toggle",
                (ThinkingSwitch::Toggle, false) => "unknown",
            };
            log(format!(
                "[settings] thinking caps resolved (provider={}, model={}, switch={name}, \
                 efforts=[{}], effort={} chosen={})",
                self.provider.key,
                pystr::strip(&self.model),
                options.join(","),
                resolved.3,
                self.effort
            ));
            self.logged_caps = Some(resolved);
        }
        options
    }

    fn thinking_row(&mut self, ui: &mut Ui, switch: ThinkingSwitch) {
        form_row(ui, "", |ui, _| {
            let label = t("field.thinking", &[]);
            ui.horizontal(|ui| {
                ui.set_min_height(theme::CONTROL_HEIGHT);
                // 鎖定時畫生效值，不動使用者自己的選擇，換回可切換的模型時才能恢復
                let mut shown = effective_thinking(switch, self.thinking);
                let value = if switch == ThinkingSwitch::Toggle {
                    &mut self.thinking
                } else {
                    &mut shown
                };
                let switch_response = ui
                    .add_enabled_ui(switch == ThinkingSwitch::Toggle, |ui| {
                        toggle(ui, value, &label)
                    })
                    .inner;
                mark(ui, switch_response.rect, Block::Control);
                let caption = ui.add(Label::new(label.as_str()).wrap());
                mark(ui, caption.rect, Block::Text);
                switch_response.labelled_by(caption.id);
            });
            let key = match switch {
                ThinkingSwitch::Unsupported => "hint.thinking_unsupported",
                ThinkingSwitch::Always => "hint.thinking_always",
                ThinkingSwitch::Toggle => "hint.thinking",
            };
            hint(ui, &t(key, &[]));
        });
    }

    fn effort_row(&mut self, ui: &mut Ui, options: &[&'static str]) {
        form_row(ui, &t("field.effort", &[]), |ui, label| {
            let name = |code: &str| t(&format!("effort.{code}"), &[]);
            let combo = combo_box(ui, "effort")
                .selected_text(name(self.effective_effort()))
                .width(EFFORT_WIDTH)
                .show_ui(ui, |ui| {
                    for &code in options {
                        if ui
                            .selectable_label(code == self.effective_effort(), name(code))
                            .clicked()
                        {
                            code.clone_into(&mut self.effort);
                        }
                    }
                });
            let response = combo.response.labelled_by(label);
            mark(ui, response.rect, Block::Control);
            focus_ring(ui, &response, theme::CONTROL_RADIUS);
            // 清單列了支援的等級時選單已濾掉不支援的，不必再警告會選錯
            let known = self.caps().is_some_and(|c| c.efforts.is_some());
            hint(
                ui,
                &t(
                    if known {
                        "hint.effort_known"
                    } else {
                        "hint.effort"
                    },
                    &[],
                ),
            );
        });
    }

    /// 分隔線下的〔測試連線〕與結果；回傳是否按了測試。
    fn test_row(&mut self, ui: &mut Ui) -> bool {
        ui.add(Separator::default().horizontal().spacing(8.0));
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = COLUMN_GAP;
            let busy = self.test_round.busy();
            let text = t(
                if busy {
                    "button.testing"
                } else {
                    "button.test"
                },
                &[],
            );
            let button = ui
                .add_enabled_ui(!busy, |ui| secondary_button(ui, &text))
                .inner;
            mark(ui, button.rect, Block::Control);
            let clicked = button.clicked();
            if let Some(result) = &self.test_result {
                ui.vertical(|ui| {
                    ui.add_space(6.0);
                    ui.spacing_mut().item_spacing.y = 3.0;
                    match result {
                        TestResult::Passed {
                            seconds,
                            translated,
                        } => {
                            outcome(ui, true, &t("test.connected", &[("seconds", seconds)]));
                            pair(ui, &t("test.source", &[]), TEST_SAMPLE);
                            pair(ui, &t("test.translation", &[]), translated);
                        }
                        TestResult::Failed(text) => outcome(ui, false, text),
                    }
                });
            }
            clicked
        })
        .inner
    }

    fn start_test(&mut self, env: &dyn SettingsEnv, target_language: &str) {
        let errors = self.api_errors();
        if !errors.is_empty() {
            self.test_result = Some(TestResult::Failed(join_errors(&errors)));
            return;
        }
        self.test_result = None;
        let api = self.api_values();
        let spec = ServiceSpec::from_service(&api);
        log(format!(
            "[settings] test connection started (provider={}, model={}, has_key={})",
            spec.provider,
            spec.model,
            py_bool(!spec.api_key.is_empty())
        ));
        self.test_request = (spec.provider.clone(), spec.model.clone());
        let round = self.test_round.start();
        env.test_connection(round, spec, target_language.to_owned());
    }

    fn tested(&mut self, result: &Result<TestOutcome, TranslateError>) {
        let (provider, model) = &self.test_request;
        self.test_result = Some(match result {
            Ok(done) => {
                log(format!(
                    "[settings] test connection ok (provider={provider}, model={model})"
                ));
                TestResult::Passed {
                    seconds: format_seconds(done.elapsed),
                    translated: done.translated.clone(),
                }
            }
            Err(e) => {
                log(format!(
                    "[settings] test connection failed (provider={provider}, model={model}): {e}"
                ));
                TestResult::Failed(friendly_text(e))
            }
        });
    }

    /// 連線欄位填妥且與上次自動載入時不同，就自動載入清單；沒填妥不顯示錯誤。
    fn auto_refresh(&mut self, env: &dyn SettingsEnv) {
        let endpoint = (
            pystr::strip(&self.api_key).to_owned(),
            pystr::strip(&self.base_url).to_owned(),
        );
        if self.auto_endpoint.as_ref() == Some(&endpoint)
            || !validate_endpoint_fields(&self.api_values()).is_empty()
        {
            return;
        }
        self.auto_endpoint = Some(endpoint);
        self.start_refresh(env, "auto");
    }

    /// 載入模型清單；`trigger` 是 `auto`／`manual`，log 用。
    fn start_refresh(&mut self, env: &dyn SettingsEnv, trigger: &'static str) {
        let api = self.api_values();
        let errors = validate_endpoint_fields(&api);
        if !errors.is_empty() {
            self.model_status = ModelStatus::Error(join_errors(&errors));
            return;
        }
        self.model_status = ModelStatus::Idle;
        let spec = ServiceSpec::from_service(&api);
        self.models_request = (trigger, spec.provider.clone(), spec.base_url.clone());
        let round = self.models_round.start();
        env.list_models(round, spec);
    }

    fn set_models(&mut self, models: Vec<ModelInfo>) {
        self.model_ids = models.iter().map(|m| m.id.clone()).collect();
        self.models = models;
    }

    fn listed(&mut self, result: &Result<Vec<ModelInfo>, TranslateError>) {
        let before = self.api_values();
        self.apply_listing(result);
        // 清單帶來的能力可能鎖住開關或改掉生效深度，通過的測試就不再代表會存下的值
        if self.api_values() != before && self.test_passed() {
            self.void_test();
        }
    }

    fn apply_listing(&mut self, result: &Result<Vec<ModelInfo>, TranslateError>) {
        self.placeholder_until = None;
        let (trigger, provider, base_url) = &self.models_request;
        match result {
            Ok(models) => {
                let with_caps = models.iter().filter(|m| m.thinking.is_known()).count();
                log(format!(
                    "[settings] model list fetched (trigger={trigger}, provider={provider}, \
                     count={}, with_thinking_caps={with_caps})",
                    models.len()
                ));
                self.set_models(models.clone());
                self.model_status = ModelStatus::Found(models.len());
            }
            Err(e) => {
                let unsupported = matches!(e, TranslateError::NoModelList);
                let reason = if unsupported { "unsupported" } else { "failed" };
                log(format!(
                    "[settings] model list {reason} (trigger={trigger}, provider={provider}, \
                     base_url={base_url}): {e}"
                ));
                self.set_models(Vec::new());
                self.model_status = if unsupported {
                    ModelStatus::NoList
                } else {
                    ModelStatus::Error(friendly_text(e))
                };
            }
        }
    }
}

/// 伺服器網址格式不對的原因：沒有 `http(s)://` 會建不出請求，結尾 `/v1` 會被用戶端再接一次 `/v1`。
fn base_url_error(base_url: &str) -> Option<&'static str> {
    let url = base_url.to_ascii_lowercase();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        Some("error.base_url_scheme")
    } else if url.trim_end_matches('/').ends_with("/v1") {
        Some("error.base_url_v1")
    } else {
        None
    }
}

/// 測試連線花的秒數，四捨五入到十分位（`1250ms` → `"1.3"`）。
pub fn format_seconds(elapsed: Duration) -> String {
    let tenths = (elapsed.as_millis() + 50) / 100;
    format!("{}.{}", tenths / 10, tenths % 10)
}

/// 表單的一列：左邊固定寬的名稱，右邊欄位；`field` 拿到名稱的 Id，供欄位 `labelled_by`。
/// 佔位灰塊的樣式：輸入框、按鈕等整塊畫滿；文字逐行畫成細條。
#[derive(Clone, Copy)]
enum Block {
    Control,
    Text,
}

fn placeholder_id() -> Id {
    Id::unique("service_form_placeholder")
}

/// 佔位期間（表單不可見）記下一個元件的位置；畫面可見時什麼都不做。
fn mark(ui: &Ui, rect: Rect, block: Block) {
    if !ui.is_visible() {
        ui.data_mut(|d| {
            d.get_temp_mut_or_default::<Vec<(Rect, Block)>>(placeholder_id())
                .push((rect, block));
        });
    }
}

/// 從 `rect` 的左緣延伸到這一欄的右緣：自訂外框的輸入框回應的範圍只有文字區。
fn full_width(ui: &Ui, rect: Rect) -> Rect {
    Rect::from_x_y_ranges(rect.left()..=ui.max_rect().right(), rect.y_range())
}

/// 在記下的位置畫明暗呼吸的灰塊；`area` 是表單範圍，給無障礙的「讀取中」節點用。畫著時持續重繪。
fn paint_placeholder(ui: &mut Ui, area: Rect, label: &str) {
    const LINE: f32 = 18.0;
    let blocks: Vec<(Rect, Block)> = ui
        .data(|d| d.get_temp(placeholder_id()))
        .unwrap_or_default();
    let progress = ui.interact(area, placeholder_id(), Sense::hover());
    progress.widget_info(|| WidgetInfo::labeled(Role::ProgressIndicator, true, label));
    let pulse = 0.5 + 0.5 * (ui.input(|i| i.time) * std::f64::consts::TAU / 1.4).sin() as f32;
    let fill = theme::CONTROL_BG.lerp_to_gamma(theme::TEXT_MUTED, 0.08 + 0.14 * pulse);
    let painter = ui.painter();
    for (rect, block) in blocks {
        match block {
            Block::Control => {
                painter.rect_filled(rect, CornerRadius::same(theme::CONTROL_RADIUS), fill);
            }
            Block::Text => {
                let lines = (rect.height() / LINE).round().max(1.0) as usize;
                let pitch = rect.height() / lines as f32;
                for line in 0..lines {
                    let last = line + 1 == lines && lines > 1;
                    let width = rect.width() * if last { 0.6 } else { 1.0 };
                    let mid = rect.top() + pitch * (line as f32 + 0.5);
                    let bar = Rect::from_min_size(pos2(rect.left(), mid - 5.0), vec2(width, 10.0));
                    painter.rect_filled(bar, CornerRadius::same(5), fill);
                }
            }
        }
    }
    ui.ctx().request_repaint();
}

fn form_row<R>(ui: &mut Ui, label: &str, field: impl FnOnce(&mut Ui, Id) -> R) -> R {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = COLUMN_GAP;
        let size = vec2(LABEL_WIDTH, theme::CONTROL_HEIGHT);
        let label_id = ui
            .allocate_ui_with_layout(size, Layout::left_to_right(Align::Center), |ui| {
                ui.set_min_size(size);
                let text = RichText::new(label).size(theme::CONTROL_TEXT_SIZE);
                let caption = ui.add(Label::new(text).wrap());
                if !label.is_empty() {
                    mark(ui, caption.rect, Block::Text);
                }
                caption.id
            })
            .inner;
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            field(ui, label_id)
        })
        .inner
    })
    .inner
}

/// 欄位下方的說明；純文字（自訂端點的範例網址不該變成連結）。
fn hint(ui: &mut Ui, text: &str) {
    let text = RichText::new(text)
        .size(theme::HINT_SIZE)
        .color(theme::TEXT_SECONDARY);
    let label = ui.add(Label::new(text).wrap());
    mark(ui, label.rect, Block::Text);
}

/// 測試結果的一行「名稱　內容」。
fn pair(ui: &mut Ui, key: &str, value: &str) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        ui.add(Label::new(
            RichText::new(key).size(13.0).color(theme::TEXT_SECONDARY),
        ));
        ui.add(Label::new(RichText::new(value).size(13.0)).wrap());
    });
}

fn join_errors(keys: &[String]) -> String {
    let texts: Vec<String> = keys.iter().map(|key| t(key, &[])).collect();
    texts.join(&t("sep.errors", &[]))
}
