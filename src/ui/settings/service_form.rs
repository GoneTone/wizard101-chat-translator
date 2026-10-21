//! 單筆翻譯服務的編輯表單：名稱、服務商、該家的欄位與測試連線；設定視窗的對話框與首次設定精靈共用。

use std::time::Duration;

use eframe::egui::{Align, Hyperlink, Id, Label, Layout, RichText, Separator, Ui, vec2};
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
    EFFORT_AUTO, EFFORTS, Provider, ServiceSpec, is_auto_name, provider, unique_name,
    validate_endpoint_fields, validate_service,
};
use crate::i18n::t;
use crate::logs::log;
use crate::py::pystr;
use crate::translation::errors::{TranslateError, friendly_text};
use crate::translation::translator::{TEST_SAMPLE, TestOutcome};

const LABEL_WIDTH: f32 = 96.0;
const COLUMN_GAP: f32 = 12.0;
const ROW_GAP: f32 = 10.0;
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
    models: Vec<String>,
    model_status: ModelStatus,
    models_round: Round,
    /// 抓清單那一刻的服務商與網址，log 用。
    models_request: (String, String),
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
        let effort = service
            .get("effort")
            .and_then(Value::as_str)
            .filter(|effort| EFFORTS.contains(effort))
            .unwrap_or(EFFORT_AUTO)
            .to_owned();
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
            model_status: ModelStatus::Idle,
            models_round: Round::new("model list"),
            models_request: Default::default(),
            test_round: Round::new("test connection"),
            test_request: Default::default(),
            test_result: None,
            pending_change: false,
        }
    }

    /// 完整的一筆服務：id、名稱、服務商與該家的欄位；名稱空白時退回去重後的短名。
    pub fn values(&self) -> Value {
        let mut service = Map::new();
        service.insert("id".into(), self.id.clone().into());
        service.insert("name".into(), self.resolved_name().into());
        service.extend(self.api_map());
        Value::Object(service)
    }

    /// 測試連線與翻譯用的形狀：去掉 id 與名稱。
    pub fn api_values(&self) -> Value {
        Value::Object(self.api_map())
    }

    /// 按確定與測試連線前要擋下的原因（錯誤文案 key）：必填欄位，加上伺服器網址的格式。
    ///
    /// 網址格式只在表單擋：`validate_service` 也決定啟動時要不要進精靈，不跟著加嚴。
    pub fn errors(&self) -> Vec<String> {
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
        self.models.clear();
        self.model_status = ModelStatus::Idle;
        self.models_round.invalidate();
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
        ui.scope(|ui| self.body(ui, env, target_language)).inner
    }

    fn body(&mut self, ui: &mut Ui, env: &dyn SettingsEnv, target_language: &str) -> FormResponse {
        let before = self.api_values();
        let mut response = FormResponse {
            changed: std::mem::take(&mut self.pending_change),
            change_provider: false,
        };
        ui.spacing_mut().item_spacing.y = ROW_GAP;

        form_row(ui, &t("field.service_name", &[]), |ui, label| {
            text_field(ui, &mut self.name).labelled_by(label);
        });
        response.change_provider = self.provider_row(ui);
        if self.provider.needs_base_url() {
            form_row(ui, &t("field.base_url", &[]), |ui, label| {
                text_field(ui, &mut self.base_url).labelled_by(label);
                hint(ui, &t("hint.custom_endpoint", &[]));
            });
        }
        self.key_row(ui);
        if self.model_row(ui) {
            self.start_refresh(env);
        }
        if self.provider.has_field("thinking") {
            self.thinking_row(ui);
        }
        if self.provider.has_field("effort") {
            self.effort_row(ui);
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
        response
    }

    fn resolved_name(&self) -> String {
        match pystr::strip(&self.name) {
            "" => unique_name(&self.provider.short_name(), &self.services, Some(&self.id)),
            name => name.to_owned(),
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
                "thinking" => self.thinking.into(),
                "effort" => self.effort.clone().into(),
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
                ui.add(Label::new(label).truncate());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    secondary_button(ui, &t("button.change", &[])).clicked()
                })
                .inner
            })
            .inner
        })
    }

    fn key_row(&mut self, ui: &mut Ui) {
        let label = if self.provider.needs_base_url() {
            "field.api_key_optional"
        } else {
            "field.api_key"
        };
        form_row(ui, &t(label, &[]), |ui, label| {
            let (show, hide) = (t("button.show", &[]), t("button.hide", &[]));
            secret_field(ui, &mut self.api_key, &mut self.key_revealed, &show, &hide)
                .labelled_by(label);
            if let Some(url) = self.provider.key_url {
                let text = RichText::new(t("link.get_key", &[])).size(13.0);
                if ui.add(Hyperlink::from_label_and_url(text, url)).clicked() {
                    log(format!("[ui] opening API key page: {url}"));
                }
            }
        });
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
                    editable_combo(ui, combo, &mut self.model, &self.models, true)
                        .labelled_by(label);
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
                        ui.add_enabled_ui(!busy, |ui| {
                            secondary_icon_button(ui, icon::REFRESH, &text)
                        })
                        .inner
                        .clicked()
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

    fn thinking_row(&mut self, ui: &mut Ui) {
        form_row(ui, "", |ui, _| {
            let label = t("field.thinking", &[]);
            ui.horizontal(|ui| {
                ui.set_min_height(theme::CONTROL_HEIGHT);
                let switch = toggle(ui, &mut self.thinking, &label);
                switch.labelled_by(ui.add(Label::new(label.as_str()).wrap()).id);
            });
            hint(ui, &t("hint.thinking", &[]));
        });
    }

    fn effort_row(&mut self, ui: &mut Ui) {
        form_row(ui, &t("field.effort", &[]), |ui, label| {
            let name = |code: &str| t(&format!("effort.{code}"), &[]);
            let combo = combo_box(ui, "effort")
                .selected_text(name(&self.effort))
                .width(EFFORT_WIDTH)
                .show_ui(ui, |ui| {
                    for code in EFFORTS {
                        if ui
                            .selectable_label(code == self.effort, name(code))
                            .clicked()
                        {
                            code.clone_into(&mut self.effort);
                        }
                    }
                });
            let response = combo.response.labelled_by(label);
            focus_ring(ui, &response, theme::CONTROL_RADIUS);
            hint(ui, &t("hint.effort", &[]));
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
            let clicked = ui
                .add_enabled_ui(!busy, |ui| secondary_button(ui, &text))
                .inner
                .clicked();
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
        let errors = self.errors();
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

    fn start_refresh(&mut self, env: &dyn SettingsEnv) {
        let api = self.api_values();
        let errors = validate_endpoint_fields(&api);
        if !errors.is_empty() {
            self.model_status = ModelStatus::Error(join_errors(&errors));
            return;
        }
        self.model_status = ModelStatus::Idle;
        let spec = ServiceSpec::from_service(&api);
        self.models_request = (spec.provider.clone(), spec.base_url.clone());
        let round = self.models_round.start();
        env.list_models(round, spec);
    }

    fn listed(&mut self, result: &Result<Vec<String>, TranslateError>) {
        let (provider, base_url) = &self.models_request;
        match result {
            Ok(models) => {
                log(format!(
                    "[settings] model list fetched (provider={provider}, count={})",
                    models.len()
                ));
                self.models.clone_from(models);
                self.model_status = ModelStatus::Found(models.len());
            }
            Err(e) => {
                let unsupported = matches!(e, TranslateError::NoModelList);
                let reason = if unsupported { "unsupported" } else { "failed" };
                log(format!(
                    "[settings] model list {reason} (provider={provider}, base_url={base_url}): {e}"
                ));
                self.models.clear();
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
fn form_row<R>(ui: &mut Ui, label: &str, field: impl FnOnce(&mut Ui, Id) -> R) -> R {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = COLUMN_GAP;
        let size = vec2(LABEL_WIDTH, theme::CONTROL_HEIGHT);
        let label_id = ui
            .allocate_ui_with_layout(size, Layout::left_to_right(Align::Center), |ui| {
                ui.set_min_size(size);
                let text = RichText::new(label).size(theme::CONTROL_TEXT_SIZE);
                ui.add(Label::new(text).wrap()).id
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
    ui.add(Label::new(text).wrap());
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
