//! 首次設定精靈測試共用的 harness 與操作（`wizard.rs` 與 `wizard_language.rs` 共用）。
#![allow(dead_code)]

use std::time::Duration;

use eframe::egui::accesskit::Role;
use eframe::egui::{self, Key, Modifiers, Ui};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use super::settings_env::FakeEnv;
use wizard101_chat_translator::config::services::new_service;
use wizard101_chat_translator::config::{Config, default_config};
use wizard101_chat_translator::i18n::t;
use wizard101_chat_translator::translation::translator::TestOutcome;
use wizard101_chat_translator::ui::fonts_egui;
use wizard101_chat_translator::ui::settings::env::SettingsEvent;
use wizard101_chat_translator::ui::settings::theme;
use wizard101_chat_translator::ui::settings::wizard::{Step, Wizard, WizardAction};

pub struct State {
    pub wizard: Wizard,
    pub env: FakeEnv,
    pub actions: Vec<WizardAction>,
}

pub type WizardHarness = Harness<'static, State>;

pub fn tr(key: &str) -> String {
    t(key, &[])
}

/// 以目前的介面語言開精靈（呼叫端先設好語言）。
pub fn harness(cfg: Config) -> WizardHarness {
    let state = State {
        wizard: Wizard::new(cfg),
        env: FakeEnv::default(),
        actions: Vec::new(),
    };
    let mut h = Harness::builder()
        .with_size(theme::WIZARD_SIZE)
        .with_context_setup(|ctx| fonts_egui::install(ctx, "zh-TW"))
        .build_ui_state(
            |ui: &mut Ui, s: &mut State| {
                theme::apply(ui.style_mut());
                if let Some(action) = s.wizard.ui(ui, &s.env) {
                    s.actions.push(action);
                }
            },
            state,
        );
    h.run_steps(3);
    h
}

pub fn run(h: &mut WizardHarness) {
    h.run_steps(3);
}

/// 先捲到元件再點：內容區可能捲動。
pub fn click(h: &mut WizardHarness, role: Role, label: &str) {
    h.get_by_role_and_label(role, label).scroll_to_me();
    run(h);
    h.get_by_role_and_label(role, label).click();
    run(h);
}

pub fn has_label(h: &WizardHarness, label: &str) -> bool {
    h.query_all_by_label(label).next().is_some()
}

pub fn has(h: &WizardHarness, role: Role, label: &str) -> bool {
    h.query_by_role_and_label(role, label).is_some()
}

pub fn step(h: &WizardHarness) -> Step {
    h.state().wizard.step()
}

pub fn next(h: &mut WizardHarness) {
    click(h, Role::Button, &tr("button.next"));
}

pub fn back(h: &mut WizardHarness) {
    click(h, Role::Button, &tr("button.back"));
}

pub fn skip(h: &mut WizardHarness) {
    click(h, Role::Link, &tr("wizard.skip_test"));
}

pub fn enabled(h: &WizardHarness, label: &str) -> bool {
    !h.get_by_role_and_label(Role::Button, label)
        .accesskit_node()
        .is_disabled()
}

pub fn next_enabled(h: &WizardHarness) -> bool {
    enabled(h, &tr("button.next"))
}

/// 點服務商卡片（第二步還沒選服務商時）。
pub fn pick_provider(h: &mut WizardHarness, label_key: &str) {
    click(h, Role::Button, &tr(label_key));
}

/// 在 `label` 這一欄的文字後面接著打字。
pub fn type_into(h: &mut WizardHarness, label: &str, text: &str) {
    h.get_by_label(label).scroll_to_me();
    run(h);
    h.get_by_label(label).click();
    run(h);
    h.event(egui::Event::Text(text.into()));
    run(h);
}

/// 把 `label` 這一欄整個換成 `text`（空字串就是清空）。
pub fn replace_text(h: &mut WizardHarness, label: &str, text: &str) {
    h.get_by_label(label).scroll_to_me();
    run(h);
    h.get_by_label(label).click();
    run(h);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.key_press(Key::Backspace);
    run(h);
    if !text.is_empty() {
        h.event(egui::Event::Text(text.into()));
        run(h);
    }
}

pub fn field_value(h: &WizardHarness, label: &str) -> Option<String> {
    h.get_by_role_and_label(Role::TextInput, label).value()
}

/// 依畫面由上而下的第 `index` 個〔更改〕：0 是輸入框熱鍵，1 是框選熱鍵。
pub fn click_change(h: &mut WizardHarness, index: usize) {
    let label = tr("button.change");
    let mut nodes: Vec<_> = h
        .query_all_by_role_and_label(Role::Button, &label)
        .collect();
    nodes.sort_by(|a, b| a.rect().top().total_cmp(&b.rect().top()));
    nodes[index].scroll_to_me();
    nodes[index].click();
    run(h);
}

/// 第三步：錄下 `combo` 交給第 `index` 把熱鍵。
pub fn record(h: &mut WizardHarness, index: usize, combo: &str) {
    click_change(h, index);
    let round = *h.state().env.recordings.borrow().last().unwrap();
    h.state_mut().wizard.apply(&SettingsEvent::HotkeyRecorded {
        round,
        combo: combo.into(),
    });
    run(h);
}

/// 按〔測試連線〕並讓它成功。
pub fn pass_test(h: &mut WizardHarness) {
    click(h, Role::Button, &tr("button.test"));
    let round = h.state().env.tests.borrow().last().expect("test started").0;
    h.state_mut().wizard.apply(&SettingsEvent::TestDone {
        round,
        result: Ok(TestOutcome {
            translated: "ok".into(),
            elapsed: Duration::from_millis(500),
        }),
    });
    run(h);
}

pub fn finished(h: &WizardHarness) -> Option<Config> {
    h.state().actions.iter().find_map(|a| match a {
        WizardAction::Finish(cfg) => Some(cfg.clone()),
        _ => None,
    })
}

/// 從目前的步驟一路走到〔完成〕：第二步按「略過測試」，回傳送出的設定。
pub fn walk_to_finish(h: &mut WizardHarness) -> Config {
    for _ in 0..4 {
        match step(h) {
            Step::Language => next(h),
            Step::Service => {
                skip(h);
                next(h);
            }
            Step::Prefs => {
                click(h, Role::Button, &tr("button.finish"));
                break;
            }
        }
    }
    finished(h).expect("wizard finished")
}

/// 一筆欄位填好的服務（以目前的介面語言命名）。
pub fn filled(provider: &str, services: &[Value]) -> Value {
    let mut service = new_service(provider, services);
    service["model"] = json!("m");
    if provider == "custom" {
        service["base_url"] = json!("http://x");
    } else {
        service["api_key"] = json!(format!("sk-{provider}"));
    }
    service
}

/// 以 `services` 為服務清單、`default` 為預設服務的設定。
pub fn cfg_with(services: Vec<Value>, default: Option<&Value>) -> Config {
    let mut cfg = default_config();
    cfg.insert(
        "default_service".into(),
        default.map_or(Value::Null, |s| s["id"].clone()),
    );
    cfg.insert("services".into(), Value::Array(services));
    cfg
}

/// 單一筆填好的 Claude 服務、設為預設。
pub fn configured_cfg() -> Config {
    let service = filled("claude", &[]);
    cfg_with(vec![service.clone()], Some(&service))
}

pub fn names(cfg: &Config) -> Vec<String> {
    cfg["services"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap().to_owned())
        .collect()
}
