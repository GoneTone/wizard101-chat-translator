//! 版面稽核：以離屏 wgpu 渲染設定視窗與首次設定精靈的各畫面與狀態，存成 PNG 供人工比對設計稿（不開視窗、不碰桌面）。
//!
//! 執行：`LAYOUT_AUDIT_DIR=<資料夾> cargo test --test layout_audit -- --ignored`；PNG 不進版控。
//!
//! 會切換全域介面語言，獨立成一個測試執行檔，不放進 `tests/it`。

mod common;

use std::path::PathBuf;
use std::time::Duration;

use eframe::egui::accesskit::Role;
use eframe::egui::{self, Key, Ui, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use common::kittest::{click, themed_harness_with, tr};
use common::settings_env::FakeEnv;
use common::wizard::{self as wiz, State as WizardState, cfg_with, filled};
use wizard101_chat_translator::config::Config;
use wizard101_chat_translator::i18n;
use wizard101_chat_translator::runtime::updater::Release;
use wizard101_chat_translator::translation::clients::ModelInfo;
use wizard101_chat_translator::translation::errors::TranslateError;
use wizard101_chat_translator::translation::translator::TestOutcome;
use wizard101_chat_translator::ui::settings::env::SettingsEvent;
use wizard101_chat_translator::ui::settings::theme;
use wizard101_chat_translator::ui::settings::window::{Page, SettingsWindow};
use wizard101_chat_translator::ui::settings::wizard::Wizard;

const PROVIDERS: [&str; 4] = ["claude", "openai", "gemini", "custom"];

fn out_dir() -> PathBuf {
    let dir = PathBuf::from(
        std::env::var_os("LAYOUT_AUDIT_DIR").expect("set LAYOUT_AUDIT_DIR to the output folder"),
    );
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn save<S>(h: &mut Harness<'_, S>, name: &str) {
    h.run_steps(3);
    let image = h.render().expect("offscreen render");
    image.save(out_dir().join(format!("{name}.png"))).unwrap();
}

struct Settings {
    win: SettingsWindow,
    env: FakeEnv,
}

/// 設定視窗，字型與語言都依 `language`。
fn settings(cfg: &Config, language: &'static str, size: Vec2) -> Harness<'static, Settings> {
    i18n::set_language(language);
    let state = Settings {
        win: SettingsWindow::new(cfg),
        env: FakeEnv {
            cache: Some(3),
            ..FakeEnv::default()
        },
    };
    themed_harness_with(
        Harness::builder().wgpu(),
        size,
        language,
        |ui: &mut Ui, s: &mut Settings| {
            paint_window_bg(ui);
            let _ = s.win.ui(ui, &s.env);
        },
        state,
    )
}

/// 由上而下第 `index` 個名稱是 `label` 的按鈕。
fn click_nth(h: &mut Harness<'_, Settings>, label: &str, index: usize) {
    let mut nodes: Vec<_> = h.query_all_by_role_and_label(Role::Button, label).collect();
    nodes.sort_by(|a, b| a.rect().top().total_cmp(&b.rect().top()));
    nodes[index].scroll_to_me();
    nodes[index].click();
    h.run_steps(3);
}

fn all_providers() -> Config {
    let mut services: Vec<Value> = Vec::new();
    for provider in PROVIDERS {
        let service = filled(provider, &services);
        services.push(service);
    }
    let default = services[0].clone();
    cfg_with(services, Some(&default))
}

fn page_name(page: Page) -> &'static str {
    match page {
        Page::Basic => "basic",
        Page::Services => "services",
        Page::Advanced => "advanced",
        Page::About => "about",
    }
}

fn settings_pages(language: &'static str, size: Vec2, tag: &str) {
    for page in [Page::Basic, Page::Services, Page::Advanced, Page::About] {
        let mut h = settings(&all_providers(), language, size);
        h.state_mut().win.set_page(page);
        save(&mut h, &format!("settings_{tag}_{}", page_name(page)));
    }
}

/// 服務分頁打開第 `index` 張卡片的編輯對話框。
fn service_form(index: usize) -> Harness<'static, Settings> {
    let mut h = settings(&all_providers(), "zh-TW", theme::SETTINGS_SIZE);
    h.state_mut().win.set_page(Page::Services);
    h.run_steps(3);
    click_nth(&mut h, &tr("button.edit"), index);
    h
}

fn last_test_round(h: &Harness<'_, Settings>) -> u64 {
    h.state().env.tests.borrow().last().expect("test started").0
}

fn service_states() {
    for (index, provider) in PROVIDERS.iter().enumerate() {
        let mut h = service_form(index);
        save(&mut h, &format!("service_form_{provider}"));
    }

    let mut h = service_form(0);
    click(&mut h, Role::Button, &tr("button.test"));
    let round = last_test_round(&h);
    h.state_mut().win.apply(&SettingsEvent::TestDone {
        round,
        result: Ok(TestOutcome {
            translated: "你好，今天要一起去打王嗎？".into(),
            elapsed: Duration::from_millis(1250),
        }),
    });
    save(&mut h, "service_form_test_passed");

    let mut h = service_form(0);
    click(&mut h, Role::Button, &tr("button.test"));
    let round = last_test_round(&h);
    h.state_mut().win.apply(&SettingsEvent::TestDone {
        round,
        result: Err(TranslateError::Config {
            status: Some(401),
            detail: "invalid x-api-key, see https://docs.anthropic.com/en/api/errors".into(),
        }),
    });
    save(&mut h, "service_form_test_failed");

    let mut h = service_form(0);
    click(&mut h, Role::Button, &tr("button.refresh"));
    let round = h.state().env.lists.borrow().last().unwrap().0;
    let models = [
        "claude-3-5-haiku-latest",
        "claude-haiku-4-5",
        "claude-haiku-4-5-20251001",
        "claude-sonnet-4-5",
        "claude-opus-4-1",
    ];
    h.state_mut().win.apply(&SettingsEvent::ModelsDone {
        round,
        result: Ok(models.map(ModelInfo::bare).to_vec()),
    });
    h.run_steps(3);
    save(&mut h, "service_form_models_fetched");
    let model = tr("field.model");
    h.get_by_role_and_label(Role::TextInput, &model).click();
    h.run_steps(3);
    h.key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
    h.key_press(Key::Backspace);
    h.event(egui::Event::Text("haiku".into()));
    save(&mut h, "service_form_models_filtered");

    let mut h = service_form(0);
    click(&mut h, Role::Button, &tr("button.change"));
    save(&mut h, "service_provider_picker");

    let mut h = settings(&all_providers(), "zh-TW", theme::SETTINGS_SIZE);
    h.state_mut().win.set_page(Page::Services);
    h.run_steps(3);
    click_nth(&mut h, &tr("button.delete"), 1);
    save(&mut h, "service_delete_confirm");
}

fn basic_states() {
    let mut h = settings(&all_providers(), "zh-TW", theme::SETTINGS_SIZE);
    click_nth(&mut h, &tr("button.change"), 0);
    save(&mut h, "settings_hotkey_recording");

    let mut cfg = all_providers();
    cfg.insert("region_hotkey".into(), json!("ctrl+space"));
    cfg.insert("target_language".into(), json!(""));
    let mut h = settings(&cfg, "zh-TW", theme::SETTINGS_SIZE);
    click(&mut h, Role::Button, &tr("button.save"));
    save(&mut h, "settings_save_blocked");
}

fn update_states() {
    let results: [(&str, Result<Option<Release>, String>); 3] = [
        ("latest", Ok(None)),
        (
            "failed",
            Err("HTTP 403: API rate limit exceeded for 203.0.113.7".into()),
        ),
        (
            "available",
            Ok(Some(Release {
                version: "0.7.0".into(),
                url: "https://github.com/GoneTone/wizard101-chat-translator/releases/latest".into(),
            })),
        ),
    ];
    for (name, result) in results {
        let mut h = settings(&all_providers(), "zh-TW", theme::SETTINGS_SIZE);
        h.state_mut().win.set_page(Page::About);
        h.run_steps(3);
        click(&mut h, Role::Button, &tr("button.check_update"));
        let round = *h.state().env.checks.borrow().last().unwrap();
        h.state_mut()
            .win
            .apply(&SettingsEvent::UpdateChecked { round, result });
        save(&mut h, &format!("about_update_{name}"));
    }
}

/// 精靈，字型與語言都依 `language`。
fn wizard(cfg: Config, language: &'static str, size: Vec2) -> Harness<'static, WizardState> {
    i18n::set_language(language);
    let state = WizardState {
        wizard: Wizard::new(cfg),
        env: FakeEnv::default(),
        actions: Vec::new(),
    };
    themed_harness_with(
        Harness::builder().wgpu(),
        size,
        language,
        |ui: &mut Ui, s: &mut WizardState| {
            paint_window_bg(ui);
            if let Some(action) = s.wizard.ui(ui, &s.env) {
                s.actions.push(action);
            }
        },
        state,
    )
}

/// 離屏渲染沒有視窗底色，先自己填。
fn paint_window_bg(ui: &Ui) {
    ui.painter()
        .rect_filled(ui.max_rect(), 0.0, theme::WINDOW_BG);
}

fn empty_cfg() -> Config {
    cfg_with(Vec::new(), None)
}

fn wizard_states(language: &'static str, tag: &str) {
    let mut h = wizard(empty_cfg(), language, theme::WIZARD_SIZE);
    save(&mut h, &format!("wizard_{tag}_1_language"));
    wiz::next(&mut h);
    save(&mut h, &format!("wizard_{tag}_2a_providers"));
    wiz::pick_provider(&mut h, "provider.claude");
    save(&mut h, &format!("wizard_{tag}_2b_form"));
    wiz::type_into(&mut h, &tr("field.api_key"), "sk-ant-test");
    wiz::type_into(&mut h, &tr("field.model"), "claude-haiku-4-5");
    wiz::pass_test(&mut h);
    save(&mut h, &format!("wizard_{tag}_2b_passed"));
    wiz::next(&mut h);
    save(&mut h, &format!("wizard_{tag}_3_prefs"));
    wiz::record(&mut h, 1, "ctrl+space");
    click(&mut h, Role::Button, &tr("button.finish"));
    save(&mut h, &format!("wizard_{tag}_3_hotkeys_same"));
}

fn small_windows() {
    settings_pages("zh-TW", egui::vec2(640.0, 360.0), "small");
    let mut h = wizard(empty_cfg(), "zh-TW", egui::vec2(600.0, 380.0));
    save(&mut h, "wizard_small_1_language");
    wiz::next(&mut h);
    wiz::pick_provider(&mut h, "provider.custom");
    save(&mut h, "wizard_small_2b_form");
}

#[test]
#[ignore = "renders PNGs for a manual layout review; needs LAYOUT_AUDIT_DIR"]
fn render_every_settings_screen() {
    // 全域介面語言只有一份：所有畫面在同一個測試裡依序渲染
    settings_pages("zh-TW", theme::SETTINGS_SIZE, "zh-TW");
    settings_pages("en-US", theme::SETTINGS_SIZE, "en-US");
    basic_states();
    update_states();
    service_states();
    wizard_states("zh-TW", "zh-TW");
    wizard_states("en-US", "en-US");
    small_windows();
}
