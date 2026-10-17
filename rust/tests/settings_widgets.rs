//! 設定介面共用元件、背景工作輪次與 RealEnv 不碰桌面的部分。

use std::sync::{Arc, Mutex};

use eframe::egui::{self, Id, Key, Ui};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use wizard101_chat_translator::input::dispatcher::{Dispatcher, KeyEvent, RawKey};
use wizard101_chat_translator::translation::cache::TranslationCache;
use wizard101_chat_translator::ui::events::{UiEvent, UiPoster};
use wizard101_chat_translator::ui::fonts_egui;
use wizard101_chat_translator::ui::settings::env::{RealEnv, Round, SettingsEnv, SettingsEvent};
use wizard101_chat_translator::ui::settings::theme;
use wizard101_chat_translator::ui::settings::widgets::{
    card, collapsible, editable_combo, filter_models, keycaps, number_field, outcome, setting_row,
    toggle,
};

fn harness<S: 'static>(
    state: S,
    content: impl FnMut(&mut Ui, &mut S) + 'static,
) -> Harness<'static, S> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(480.0, 360.0))
        .with_context_setup(|ctx| {
            fonts_egui::install(ctx, "en-US");
            ctx.all_styles_mut(theme::apply);
        })
        .build_ui_state(content, state);
    harness.run_steps(2);
    harness
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn keycaps_splits_steps_and_keys() {
    assert_eq!(keycaps("ctrl+space"), [strings(&["Ctrl", "Space"])]);
    assert_eq!(
        keycaps("ctrl+shift+a, b"),
        [strings(&["Ctrl", "Shift", "A"]), strings(&["B"])]
    );
    assert_eq!(keycaps("page up"), [strings(&["Page Up"])]);
    assert!(keycaps("").is_empty());
}

#[test]
fn toggle_is_reachable_by_tab_and_flips_on_space() {
    let mut h = harness(false, |ui, on: &mut bool| {
        toggle(ui, on, "Auto open");
    });
    assert!(!h.get_by_label("Auto open").is_focused());
    h.key_press(Key::Tab);
    h.run_steps(2);
    assert!(h.get_by_label("Auto open").is_focused());
    h.key_press(Key::Space);
    h.run_steps(2);
    assert!(*h.state());
    h.key_press(Key::Enter);
    h.run_steps(2);
    assert!(!*h.state());
    h.get_by_label("Auto open").click();
    h.run_steps(2);
    assert!(*h.state());
    assert_eq!(
        h.get_by_label("Auto open").accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::True)
    );
}

#[test]
fn controls_in_a_setting_row_are_named_by_the_row() {
    struct Rows {
        on: bool,
        language: String,
    }
    let state = Rows {
        on: false,
        language: "English".into(),
    };
    let mut h = harness(state, |ui, s: &mut Rows| {
        card(ui, |ui| {
            setting_row(ui, "Follow the game input box", None, |ui| {
                toggle(ui, &mut s.on, "Follow the game input box");
            });
            setting_row(ui, "Target language", Some("Type any language"), |ui| {
                let options = ["English".to_owned()];
                editable_combo(ui, Id::unique("target"), &mut s.language, &options, false);
            });
        });
    });
    let switch = h.get_by_label("Follow the game input box");
    assert_eq!(
        switch.accesskit_node().role(),
        egui::accesskit::Role::Switch
    );
    let text = h
        .get_by(|n| {
            n.role() == egui::accesskit::Role::Label
                && n.value().as_deref() == Some("Follow the game input box")
        })
        .rect();
    let (switch_rect, card_right) = (switch.rect(), h.ctx.content_rect().right());
    assert!(
        (switch_rect.center().y - text.center().y).abs() < 1.0,
        "{switch_rect:?} vs {text:?}"
    );
    assert!(
        card_right - switch_rect.right() < 40.0,
        "control sits at the right: {switch_rect:?}"
    );
    switch.click();
    h.run_steps(2);
    assert!(h.state().on);
    let combo = h.get_by_label("Target language");
    assert_eq!(
        combo.accesskit_node().role(),
        egui::accesskit::Role::TextInput
    );
    h.get_by_label("Type any language");
}

fn collapsible_harness(default_open: bool) -> Harness<'static, ()> {
    harness((), move |ui, _| {
        collapsible(
            ui,
            Id::unique("per-slot"),
            "Per-use services",
            Some("Unassigned uses follow the default."),
            default_open,
            |ui| {
                ui.label("Inside");
            },
        );
    })
}

#[test]
fn collapsible_starts_as_asked_and_toggles_on_click() {
    let mut h = collapsible_harness(false);
    assert!(h.query_by_label("Inside").is_none());
    h.get_by_label("Per-use services").click();
    h.run_steps(2);
    h.get_by_label("Inside");
    h.get_by_label("Per-use services").click();
    h.run_steps(2);
    assert!(h.query_by_label("Inside").is_none());

    let h = collapsible_harness(true);
    h.get_by_label("Inside");
}

#[test]
fn collapsible_header_is_keyboard_activatable() {
    let mut h = collapsible_harness(false);
    h.key_press(Key::Tab);
    h.run_steps(2);
    assert!(h.get_by_label("Per-use services").is_focused());
    h.key_press(Key::Enter);
    h.run_steps(2);
    h.get_by_label("Inside");
    h.key_press(Key::Space);
    h.run_steps(2);
    assert!(h.query_by_label("Inside").is_none());
}

struct Combo {
    text: String,
    options: Vec<String>,
}

fn combo_harness(text: &str) -> Harness<'static, Combo> {
    let state = Combo {
        text: text.to_owned(),
        options: strings(&[
            "claude-3-5-haiku-latest",
            "claude-haiku-4-5",
            "claude-sonnet-4-5",
        ]),
    };
    harness(state, |ui, s: &mut Combo| {
        card(ui, |ui| {
            setting_row(ui, "Model", None, |ui| {
                editable_combo(ui, Id::unique("model"), &mut s.text, &s.options, true);
            });
        });
    })
}

fn type_into_combo(h: &mut Harness<Combo>, text: &str) {
    h.get_by_label("Model").click();
    h.run_steps(2);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(2);
}

#[test]
fn editable_combo_filters_while_typing_and_picks_with_enter() {
    let mut h = combo_harness("");
    type_into_combo(&mut h, "HAIKU");
    h.get_by_label("claude-3-5-haiku-latest");
    h.get_by_label("claude-haiku-4-5");
    assert!(h.query_by_label("claude-sonnet-4-5").is_none());

    h.key_press(Key::ArrowDown);
    h.key_press(Key::ArrowDown);
    h.run_steps(2);
    h.key_press(Key::Enter);
    h.run_steps(2);
    assert_eq!(h.state().text, "claude-haiku-4-5");
    assert!(h.query_by_label("claude-3-5-haiku-latest").is_none());
}

#[test]
fn editable_combo_escape_closes_and_keeps_the_typed_text() {
    let mut h = combo_harness("");
    type_into_combo(&mut h, "sonnet");
    h.get_by_label("claude-sonnet-4-5");
    h.key_press(Key::Escape);
    h.run_steps(2);
    assert!(h.query_by_label("claude-sonnet-4-5").is_none());
    assert_eq!(h.state().text, "sonnet");

    h.key_press(Key::Enter);
    h.run_steps(2);
    assert_eq!(
        h.state().text,
        "sonnet",
        "Enter without a pick keeps the text"
    );
}

#[test]
fn editable_combo_stays_closed_when_nothing_matches() {
    let mut h = combo_harness("");
    type_into_combo(&mut h, "my-own-model");
    for option in [
        "claude-3-5-haiku-latest",
        "claude-haiku-4-5",
        "claude-sonnet-4-5",
    ] {
        assert!(h.query_by_label(option).is_none(), "{option}");
    }
    h.key_press(Key::ArrowDown);
    h.key_press(Key::Enter);
    h.run_steps(2);
    assert_eq!(h.state().text, "my-own-model");
}

#[test]
fn filter_models_is_case_insensitive_substring() {
    let models = strings(&["GPT-4o", "gpt-4o-mini", "o3"]);
    assert_eq!(
        filter_models(&models, " 4O "),
        strings(&["GPT-4o", "gpt-4o-mini"])
    );
    assert!(filter_models(&models, "claude").is_empty());
}

#[test]
fn filter_models_blank_query_returns_all() {
    let models = strings(&["a", "b"]);
    assert_eq!(filter_models(&models, ""), models);
    assert_eq!(filter_models(&models, "   "), models);
}

#[test]
fn number_field_steps_within_the_range() {
    let mut h = harness("0.48".to_owned(), |ui, text: &mut String| {
        card(ui, |ui| {
            setting_row(
                ui,
                "Typing delay",
                Some("Raise it if the game drops keys"),
                |ui| {
                    number_field(ui, text, 0.01, (0.0, 0.5));
                },
            );
        });
    });
    h.get_by_label("+").click();
    h.run_steps(2);
    assert_eq!(h.state(), "0.49");
    for _ in 0..3 {
        h.get_by_label("+").click();
        h.run_steps(2);
    }
    assert_eq!(h.state(), "0.50");
    *h.state_mut() = "abc".into();
    h.get_by_label("−").click();
    h.run_steps(2);
    assert_eq!(
        h.state(),
        "0.00",
        "unparsable text restarts from the minimum"
    );
}

#[test]
fn round_discards_invalidated_and_stale_results() {
    let mut r = Round::default();
    assert!(!r.busy());

    // 正常結果送達
    let normal = r.start();
    assert!(r.busy());
    assert!(r.finish(normal));
    assert!(!r.busy());
    assert!(!r.finish(normal), "a round is delivered once");

    // 作廢後的結果丟掉，但按鈕恢復
    let edited = r.start();
    r.invalidate();
    assert!(r.busy());
    assert!(!r.finish(edited));
    assert!(!r.busy());

    // 慢的一輪不會落到後一輪
    let slow = r.start();
    let current = r.start();
    assert!(!r.finish(slow));
    assert!(r.busy(), "the newer round is still running");
    assert!(r.finish(current));

    // 序號全域遞增：別的元件的輪次不會撞號
    let mut other = Round::default();
    assert!(other.start() > current);
}

#[test]
fn outcome_makes_urls_clickable() {
    let h = harness((), |ui, _| {
        outcome(
            ui,
            false,
            "HTTP 429: quota exceeded, read https://platform.example/docs/errors.",
        );
        outcome(ui, true, "Connected");
    });
    let link = h.get_by_label("https://platform.example/docs/errors");
    assert_eq!(link.accesskit_node().role(), egui::accesskit::Role::Link);
    h.get_by_label("Connected");
}

fn key(down: bool, code: i32, vk: u32) -> KeyEvent {
    KeyEvent {
        down,
        code,
        raw: Some(RawKey {
            vk,
            extended: false,
        }),
    }
}

#[test]
fn real_env_records_into_a_ui_event_and_cancel_removes_the_recorder() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (poster, inbox) = UiPoster::for_test();
    let dispatcher = Arc::new(Mutex::new(Dispatcher::new()));
    let env = RealEnv::new(rt.handle().clone(), poster, dispatcher.clone(), None);

    env.record_hotkey(7);
    env.record_hotkey(8);
    assert_eq!(
        dispatcher.lock().unwrap().handler_count(),
        1,
        "a new recording replaces the old one"
    );
    for ev in [key(true, 57, 0x20), key(false, 57, 0x20)] {
        dispatcher.lock().unwrap().process(ev);
    }
    let got: Vec<UiEvent> = inbox.try_iter().collect();
    assert_eq!(
        got,
        [UiEvent::Settings(SettingsEvent::HotkeyRecorded {
            round: 8,
            combo: "space".into(),
        })]
    );
    env.cancel_recording();
    assert_eq!(dispatcher.lock().unwrap().handler_count(), 0);
}

#[test]
fn real_env_clears_the_cache_and_reports_the_count() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (poster, _inbox) = UiPoster::for_test();
    let dispatcher = Arc::new(Mutex::new(Dispatcher::new()));
    let none = RealEnv::new(
        rt.handle().clone(),
        poster.clone(),
        dispatcher.clone(),
        None,
    );
    assert!(!none.has_cache());
    assert_eq!(none.clear_cache(), 0);

    let dir = tempfile::tempdir().unwrap();
    let cache = Arc::new(TranslationCache::new(
        dir.path().join("cache.json"),
        "fp".into(),
    ));
    assert!(cache.put("Hello", "你好", "fp"));
    let env = RealEnv::new(rt.handle().clone(), poster, dispatcher, Some(cache));
    assert!(env.has_cache());
    assert_eq!(env.clear_cache(), 1);
    assert_eq!(env.clear_cache(), 0);
}
