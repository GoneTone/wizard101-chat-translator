//! 設定介面共用元件、背景工作輪次與 RealEnv 不碰桌面的部分。

use std::sync::{Arc, Mutex};

use crate::common::kittest::{click_at, themed_harness};
use eframe::egui::accesskit::Role;
use eframe::egui::{self, Align, Color32, Id, Key, Layout, Rect, Shape, Ui};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use wizard101_chat_translator::input::dispatcher::{Dispatcher, testing::key};
use wizard101_chat_translator::translation::cache::TranslationCache;
use wizard101_chat_translator::ui::events::{UiEvent, UiPoster};
use wizard101_chat_translator::ui::fonts_egui;
use wizard101_chat_translator::ui::settings::env::{RealEnv, Round, SettingsEnv, SettingsEvent};
use wizard101_chat_translator::ui::settings::fields::{
    editable_combo, number_field, secret_field, text_field,
};
use wizard101_chat_translator::ui::settings::language::ui_language_combo;
use wizard101_chat_translator::ui::settings::theme;
use wizard101_chat_translator::ui::settings::theme::icon;
use wizard101_chat_translator::ui::settings::widgets::{
    INLINE_ICON_INSET, callout, card, collapsible, dashed_button, filter_models, icon_button,
    keycaps, outcome, primary_button, secondary_button, secondary_icon_button, setting_row, toggle,
};
use wizard101_chat_translator::ui::text_select::find_text_shape;

fn harness<S: 'static>(
    state: S,
    content: impl FnMut(&mut Ui, &mut S) + 'static,
) -> Harness<'static, S> {
    themed_harness(egui::vec2(480.0, 360.0), "en-US", content, state)
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
fn keycaps_splits_like_the_hotkey_parser() {
    assert_eq!(
        keycaps("ctrl+a,b"),
        [strings(&["Ctrl", "A"]), strings(&["B"])]
    );
    assert_eq!(keycaps("+"), [strings(&["+"])]);
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
fn icon_button_tooltip_uses_the_settings_theme() {
    let mut h = Harness::builder()
        .with_size(egui::vec2(200.0, 100.0))
        .with_context_setup(|ctx| {
            fonts_egui::install(ctx, "en-US");
            ctx.all_styles_mut(|style| style.interaction.tooltip_delay = 0.0);
        })
        .build_ui(|ui| {
            theme::apply(ui.style_mut());
            icon_button(ui, icon::CLOSE, "Close tip");
        });
    h.run_steps(2);
    let button = h.get_by_label("Close tip").rect();
    h.hover_at(button.center());
    h.run_steps(4);
    let color = find_text_shape(&h.output().shapes, "Close tip").map(|shape| shape.fallback_color);
    assert_eq!(color, Some(theme::TEXT));
}

#[test]
fn editable_combo_picks_the_option_that_is_clicked() {
    let mut h = combo_harness("");
    type_into_combo(&mut h, "haiku");
    h.get_by_label("claude-haiku-4-5").click();
    h.run_steps(3);
    assert_eq!(h.state().text, "claude-haiku-4-5");
    assert!(h.query_by_label("claude-3-5-haiku-latest").is_none());
}

#[test]
fn editable_combo_closes_on_a_click_elsewhere_and_keeps_the_text() {
    let mut h = combo_harness("");
    type_into_combo(&mut h, "sonnet");
    h.get_by_label("claude-sonnet-4-5");
    click_at(&mut h, egui::pos2(460.0, 340.0));
    assert!(h.query_by_label("claude-sonnet-4-5").is_none());
    assert_eq!(h.state().text, "sonnet");
}

#[test]
fn editable_combo_chevron_opens_the_full_list() {
    let mut h = combo_harness("");
    let edit = h.get_by_label("Model").rect();
    assert!(h.query_by_label("claude-haiku-4-5").is_none());
    click_at(&mut h, egui::pos2(edit.right() + 10.0, edit.center().y));
    for option in [
        "claude-3-5-haiku-latest",
        "claude-haiku-4-5",
        "claude-sonnet-4-5",
    ] {
        h.get_by_label(option);
    }
}

#[test]
fn editable_combo_scrolls_a_long_list_and_keeps_the_hot_option_in_view() {
    let mut h = combo_harness("");
    h.state_mut().options = (0..60).map(|i| format!("model-{i:02}")).collect();
    type_into_combo(&mut h, "model");
    let edit = h.get_by_label("Model").rect();
    let limit = edit.bottom() + egui::Style::default().spacing.combo_height + 20.0;
    for _ in 0..30 {
        h.key_press(Key::ArrowDown);
    }
    h.run_steps(4);
    let hot = h.get_by_label("model-29").rect();
    assert!(hot.bottom() <= limit, "{} > {limit}", hot.bottom());
}

#[test]
fn editable_combo_grows_to_the_full_height_when_more_options_match() {
    let mut h = combo_harness("");
    h.state_mut().options = (0..14).map(|i| format!("model-{i:02}")).collect();
    type_into_combo(&mut h, "model-00");
    h.get_by_label("model-00");
    assert!(
        h.query_by_label("model-01").is_none(),
        "precondition: one match"
    );
    for _ in 0.."model-00".len() {
        h.key_press(Key::Backspace);
    }
    h.run_steps(4);
    assert_eq!(h.state().text, "");
    let shown = h
        .output()
        .shapes
        .iter()
        .find_map(|c| match &c.shape {
            Shape::Text(t) if t.galley.text() == "model-04" => {
                Some((c.clip_rect, Rect::from_min_size(t.pos, t.galley.size())))
            }
            _ => None,
        })
        .expect("model-04 painted");
    assert!(
        shown.0.contains_rect(shown.1),
        "model-04 clipped: {shown:?}"
    );
}

#[test]
fn dragging_the_dropdown_scrollbar_keeps_the_list_open_and_scrolls_it() {
    let mut h = combo_harness("");
    h.state_mut().options = (0..60).map(|i| format!("model-{i:02}")).collect();
    type_into_combo(&mut h, "model");
    let first = h.get_by_label("model-00").rect();
    let bar = egui::pos2(first.right() - 2.0, first.center().y + 20.0);
    h.event(egui::Event::PointerMoved(bar));
    h.run_steps(2);
    let button = |pressed| egui::Event::PointerButton {
        pos: bar,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    h.event(button(true));
    h.run_steps(2);
    h.event(egui::Event::PointerMoved(bar + egui::vec2(0.0, 60.0)));
    h.run_steps(2);
    h.event(button(false));
    h.run_steps(2);
    let after = h.get_by_label("model-00").rect();
    assert!(after.min.y < first.min.y, "{first:?} -> {after:?}");
}

/// 畫面上文字是 `text` 的那個字形的顏色。
fn painted_color(h: &Harness<Combo>, text: &str) -> Option<Color32> {
    find_text_shape(&h.output().shapes, text).map(|shape| shape.fallback_color)
}

#[test]
fn the_theme_stays_on_the_settings_ui_and_reaches_the_combo_popup() {
    let mut h = combo_harness("");
    type_into_combo(&mut h, "sonnet");
    assert_eq!(painted_color(&h, "claude-sonnet-4-5"), Some(theme::TEXT));
    let global = h.ctx.global_style();
    assert_ne!(global.visuals.override_text_color, Some(theme::TEXT));
    assert_ne!(global.spacing.interact_size.y, theme::CONTROL_HEIGHT);
    assert_ne!(global.visuals.panel_fill, theme::WINDOW_BG);
    assert_ne!(global.spacing.button_padding.x, 16.0);
    assert_ne!(
        global.text_styles[&egui::TextStyle::Body].size,
        theme::TEXT_SIZE
    );
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
fn number_field_with_a_reversed_range_does_not_panic() {
    let mut h = harness("3".to_owned(), |ui, text: &mut String| {
        card(ui, |ui| {
            setting_row(ui, "Reversed", None, |ui| {
                number_field(ui, text, 1.0, (5.0, 1.0));
            });
        });
    });
    h.get_by_label("+").click();
    h.run_steps(2);
    assert_eq!(h.state(), "1");
}

#[test]
fn pasting_into_a_text_field_inserts_the_clipboard_text() {
    let mut h = harness(String::new(), |ui, text: &mut String| {
        card(ui, |ui| {
            setting_row(ui, "Server URL", None, |ui| {
                text_field(ui, text);
            });
        });
    });
    h.get_by_label("Server URL").click();
    h.run_steps(2);
    h.event(egui::Event::Paste("http://127.0.0.1:8000".into()));
    h.run_steps(2);
    assert_eq!(h.state(), "http://127.0.0.1:8000");
}

#[test]
fn round_discards_invalidated_and_stale_results() {
    let mut r = Round::new("probe");
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
    let mut other = Round::new("other");
    assert!(other.start() > current);
}

#[test]
fn a_cancelled_round_is_idle_and_drops_its_late_result() {
    let mut r = Round::new("hotkey capture");
    let recording = r.start();
    r.cancel();
    assert!(!r.busy());
    assert!(!r.finish(recording));
    assert!(!r.busy());
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
    assert_eq!(none.clear_cache(), Some(0));

    let dir = tempfile::tempdir().unwrap();
    let cache = Arc::new(TranslationCache::new(
        dir.path().join("cache.json"),
        "fp".into(),
    ));
    assert!(cache.put("Hello", "你好", "fp"));
    let env = RealEnv::new(rt.handle().clone(), poster, dispatcher, Some(cache));
    assert!(env.has_cache());
    assert_eq!(env.clear_cache(), Some(1));
    assert_eq!(env.clear_cache(), Some(0));
}

/// 畫面上文字是 `text` 的字形範圍。
fn text_rect<S>(h: &Harness<S>, text: &str) -> Rect {
    find_text_shape(&h.output().shapes, text)
        .map(|shape| shape.galley.rect.translate(shape.pos.to_vec2()))
        .unwrap_or_else(|| panic!("{text} not painted"))
}

/// 外框正好是 `rect` 的那個方塊的底色。
fn fill_at<S>(h: &Harness<S>, rect: Rect) -> Option<Color32> {
    h.output()
        .shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            Shape::Rect(shape) if shape.rect == rect => Some(shape.fill),
            _ => None,
        })
}

#[test]
fn button_labels_are_centred_whatever_the_row_alignment() {
    let h = harness((), |ui, _| {
        // 設定視窗底部按鈕列靠下對齊、服務表單的測試列靠上對齊、精靈裡有直排
        ui.allocate_ui_with_layout(
            egui::vec2(400.0, theme::CONTROL_HEIGHT),
            Layout::left_to_right(Align::Max),
            |ui| {
                secondary_button(ui, "取消");
                primary_button(ui, "儲存");
            },
        );
        ui.horizontal_top(|ui| {
            secondary_button(ui, "測試連線");
            secondary_icon_button(ui, icon::REFRESH, "更新清單");
        });
        ui.vertical(|ui| {
            secondary_button(ui, "更改");
        });
    });
    for label in ["取消", "儲存", "測試連線", "更新清單", "更改"] {
        let button = h.get_by_role_and_label(Role::Button, label).rect();
        let text = text_rect(&h, label);
        assert!(
            (text.center().y - button.center().y).abs() <= 1.0,
            "{label}: text {text:?} in button {button:?}"
        );
        assert_eq!(button.height(), theme::CONTROL_HEIGHT, "{label}");
    }
    let button = h.get_by_role_and_label(Role::Button, "取消").rect();
    let text = text_rect(&h, "取消");
    assert!((text.center().x - button.center().x).abs() <= 1.0);
}

#[test]
fn buttons_light_up_while_hovered() {
    let mut h = harness((), |ui, _| {
        ui.horizontal(|ui| {
            secondary_button(ui, "Cancel");
            primary_button(ui, "Save");
        });
        dashed_button(ui, "Add service", 40.0);
    });
    let cancel = h.get_by_role_and_label(Role::Button, "Cancel").rect();
    let save = h.get_by_role_and_label(Role::Button, "Save").rect();
    assert_eq!(fill_at(&h, cancel), Some(theme::CONTROL_BG));
    assert_eq!(fill_at(&h, save), Some(theme::ACCENT));
    h.hover_at(cancel.center());
    h.run_steps(2);
    assert_eq!(fill_at(&h, cancel), Some(theme::HOT_BG));
    h.hover_at(save.center());
    h.run_steps(2);
    let hovered = fill_at(&h, save).unwrap();
    assert_ne!(hovered, theme::ACCENT, "primary button shows hover");
    let add = h.get_by_role_and_label(Role::Button, "Add service").rect();
    assert_ne!(fill_at(&h, add), Some(theme::HOT_BG));
    h.hover_at(add.center());
    h.run_steps(2);
    assert_eq!(
        fill_at(&h, add),
        Some(theme::HOT_BG),
        "dashed button shows hover"
    );
}

#[test]
fn a_control_after_a_field_starts_after_the_whole_field() {
    let h = harness(String::new(), |ui, text: &mut String| {
        setting_row(ui, "Model", None, |ui| {
            ui.spacing_mut().combo_width = 200.0;
            editable_combo(ui, Id::unique("model"), text, &[], true);
            secondary_icon_button(ui, icon::REFRESH, "Refresh");
        });
    });
    let edit = h.get_by_role(Role::TextInput).rect();
    let field_right = edit.right() + 2.0 * theme::CHEVRON_INSET;
    let button = h.get_by_role_and_label(Role::Button, "Refresh").rect();
    assert!(
        button.left() >= field_right + 1.0,
        "{button:?} overlaps the field ending at {field_right}"
    );
}

/// 打開介面語言下拉，回傳前兩個項目的間距（下一項頂端減上一項頂端）與它們之間的空隙。
fn popup_pitch(mut content: impl FnMut(&mut Ui) + 'static) -> (f32, f32) {
    let mut h = harness((), move |ui, _| content(ui));
    h.get_by_role(Role::ComboBox).click();
    h.run_steps(3);
    let mut items: Vec<Rect> = wizard101_chat_translator::i18n::available_languages()
        .iter()
        .map(|(_, name)| h.get_by_role_and_label(Role::Button, name).rect())
        .collect();
    items.sort_by(|a, b| a.top().total_cmp(&b.top()));
    (
        items[1].top() - items[0].top(),
        items[1].top() - items[0].bottom(),
    )
}

#[test]
fn dropdown_items_are_spaced_the_same_in_the_settings_window_and_the_wizard() {
    let settings = popup_pitch(|ui| {
        card(ui, |ui| {
            setting_row(ui, "Interface language", None, |ui| {
                ui_language_combo(ui);
            });
        });
    });
    // 精靈的下拉不在卡片裡，外層的列距是 12
    let wizard = popup_pitch(|ui| {
        ui.spacing_mut().item_spacing.y = 12.0;
        setting_row(ui, "Interface language", None, |ui| {
            ui_language_combo(ui);
        });
    });
    assert_eq!(settings, wizard);
    assert_eq!(settings.1, theme::POPUP_ITEM_GAP);
}

#[test]
fn both_language_dropdowns_draw_the_same_chevron() {
    let h = harness(String::new(), |ui, text: &mut String| {
        card(ui, |ui| {
            setting_row(ui, "Interface language", None, |ui| {
                ui_language_combo(ui);
            });
            setting_row(ui, "Target language", None, |ui| {
                editable_combo(ui, Id::unique("target"), text, &[], false);
            });
        });
    });
    let chevrons: Vec<egui::Pos2> = h
        .output()
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            Shape::Text(s) if s.galley.text() == icon::EXPAND.to_string() => {
                Some(s.galley.rect.translate(s.pos.to_vec2()).center())
            }
            _ => None,
        })
        .collect();
    assert_eq!(chevrons.len(), 2, "{chevrons:?}");
    assert!((chevrons[0].x - chevrons[1].x).abs() <= 1.0, "{chevrons:?}");
    let combo = h.get_by_role(Role::ComboBox).rect();
    assert!((combo.right() - theme::CHEVRON_INSET - chevrons[0].x).abs() <= 1.0);
}

/// 有沒有畫出底色是 `fill`、與 `area` 相交的方塊。
fn painted_fill<S>(h: &Harness<S>, fill: Color32, area: Rect) -> bool {
    h.output().shapes.iter().any(|clipped| {
        matches!(&clipped.shape, Shape::Rect(s) if s.fill == fill && s.rect.intersects(area))
    })
}

#[test]
fn toggles_steppers_and_section_headers_react_to_hover() {
    struct Hover {
        on: bool,
        number: String,
    }
    let state = Hover {
        on: false,
        number: "1".into(),
    };
    let mut h = harness(state, |ui, s: &mut Hover| {
        card(ui, |ui| {
            setting_row(ui, "Auto open", None, |ui| {
                toggle(ui, &mut s.on, "Auto open");
            });
            setting_row(ui, "Parallel", None, |ui| {
                number_field(ui, &mut s.number, 1.0, (1.0, 8.0));
            });
            collapsible(
                ui,
                Id::unique("section"),
                "Per-use services",
                None,
                false,
                |_| {},
            );
        });
    });
    let switch = h.get_by_label("Auto open").rect();
    assert_eq!(fill_at(&h, switch), Some(Color32::TRANSPARENT));
    h.hover_at(switch.center());
    h.run_steps(2);
    assert_eq!(fill_at(&h, switch), Some(theme::CONTROL_BG));

    let plus = h.get_by_label("+").rect();
    assert_eq!(fill_at(&h, plus.shrink(1.0)), None);
    h.hover_at(plus.center());
    h.run_steps(2);
    assert_eq!(fill_at(&h, plus.shrink(1.0)), Some(theme::HOT_BG));

    let header = h.get_by_label("Per-use services").rect();
    assert!(!painted_fill(&h, theme::CONTROL_BG, header.shrink(4.0)));
    h.hover_at(header.center());
    h.run_steps(2);
    assert!(painted_fill(&h, theme::CONTROL_BG, header.shrink(4.0)));
}

#[test]
fn glyphs_sit_on_the_first_line_of_the_text_they_lead() {
    const FAILED: &str = "伺服器回應 HTTP 401：invalid x-api-key";
    let h = harness((), |ui, _| {
        outcome(ui, false, FAILED);
        outcome(ui, true, "已是最新版本");
        callout(ui, "設定不完整", &["兩個熱鍵不可相同".to_owned()]);
    });
    let cases = [
        (icon::FAILURE, FAILED),
        (icon::SUCCESS, "已是最新版本"),
        (icon::WARNING, "設定不完整"),
    ];
    for (glyph, text) in cases {
        let icon = text_rect(&h, &glyph.to_string());
        let line = text_rect(&h, text);
        assert!(
            (icon.center().y - line.center().y).abs() <= 1.0,
            "{text}: icon {icon:?} vs line {line:?}"
        );
        assert!(icon.right() < line.left(), "{text}: same row");
    }
}

#[test]
fn dropdowns_give_way_to_the_label_in_a_narrow_row() {
    let h = harness(String::new(), |ui, text: &mut String| {
        card(ui, |ui| {
            setting_row(ui, "Interface language", Some("Help us translate"), |ui| {
                ui_language_combo(ui);
            });
            setting_row(ui, "Target language", None, |ui| {
                editable_combo(ui, Id::unique("target"), text, &[], false);
            });
        });
    });
    let row = h
        .get_by(|n| n.role() == Role::Label && n.value().as_deref() == Some("Interface language"))
        .rect();
    let combo = h.get_by_role(Role::ComboBox).rect();
    let room = combo.right() - row.left();
    assert!(combo.width() < theme::COMBO_WIDTH, "{combo:?}");
    assert!(combo.width() <= 0.5 * room, "{combo:?} in a row of {room}");
    let edit = h.get_by_role(Role::TextInput).rect();
    assert!(
        edit.left() >= combo.left() - 1.0,
        "both dropdowns shrink alike"
    );
}

#[test]
fn the_secret_eye_stays_inside_the_field_and_flips_its_icon() {
    let mut h = harness(
        (String::from("sk-secret"), false),
        |ui, (text, revealed): &mut (String, bool)| {
            ui.set_width(300.0);
            let label = ui.label("API key").id;
            secret_field(ui, text, revealed, "Show", "Hide").labelled_by(label);
        },
    );
    let edit = h.get_by_role(Role::PasswordInput).rect();
    let eye = h.get_by_role_and_label(Role::Button, "Show").rect();
    // 按鈕縮在框內一圈，hover 底色碰不到外框
    let side = theme::CONTROL_HEIGHT - 2.0 * INLINE_ICON_INSET;
    assert_eq!(eye.size(), egui::vec2(side, side), "{eye:?}");
    assert!(edit.right() <= eye.left(), "{edit:?} {eye:?}");
    h.hover_at(eye.center());
    h.run_steps(2);
    assert_eq!(fill_at(&h, eye), Some(theme::CONTROL_BORDER));
    h.get_by_role_and_label(Role::Button, "Show").click();
    h.run_steps(2);
    assert!(h.state().1, "click reveals the key");
    assert!(h.query_by_label("Show").is_none());
    h.get_by_role_and_label(Role::Button, "Hide");
}
