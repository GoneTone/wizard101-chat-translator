//! 服務清單分頁、服務商卡片與三種對話框（移植自 Python 的 test_service_list 與 test_provider_picker）。

mod common;

use eframe::egui::accesskit::Role;
use eframe::egui::{self, Event, Key, Modifiers, PointerButton, Pos2, Rect, Shape, TextureId, Ui};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use common::log_capture::{capture_logs, log_lines_with, log_text};
use common::settings_env::FakeEnv;
use wizard101_chat_translator::config::Config;
use wizard101_chat_translator::config::services::{ServiceSlot, new_service, providers};
use wizard101_chat_translator::i18n::{self, t};
use wizard101_chat_translator::runtime::resources::provider_icon;
use wizard101_chat_translator::translation::translator::TestOutcome;
use wizard101_chat_translator::ui::fonts_egui;
use wizard101_chat_translator::ui::settings::env::SettingsEvent;
use wizard101_chat_translator::ui::settings::provider_picker::{
    CardLayout, lighten_dark_monochrome, provider_cards,
};
use wizard101_chat_translator::ui::settings::service_list::{ServiceDraft, ServicePane};
use wizard101_chat_translator::ui::settings::theme;

/// 本檔所有測試都用同一個介面語言，不會互相干擾。
const LANGUAGE: &str = "en-US";
const TARGET: &str = "Japanese";

fn tr(key: &str) -> String {
    t(key, &[])
}

fn id(service: &Value) -> String {
    service["id"].as_str().unwrap().to_owned()
}

fn name(service: &Value) -> String {
    service["name"].as_str().unwrap().to_owned()
}

fn service(provider: &str, services: &[Value]) -> Value {
    let mut service = new_service(provider, services);
    service["model"] = json!("m");
    service["api_key"] = json!("k");
    service
}

fn draft(
    services: Vec<Value>,
    default: Option<&Value>,
    slots: [Option<&Value>; 3],
) -> ServiceDraft {
    i18n::set_language(LANGUAGE);
    ServiceDraft {
        services,
        default: default.map(id),
        slots: slots.map(|s| s.map(id)),
    }
}

/// 兩筆服務、第一筆是預設、用途都跟隨預設。
fn two() -> (ServiceDraft, Value, Value) {
    let a = service("openai", &[]);
    let b = service("claude", std::slice::from_ref(&a));
    let d = draft(vec![a.clone(), b.clone()], Some(&a), [None; 3]);
    (d, a, b)
}

// --- ServiceDraft ---

#[test]
fn values_round_trip_unchanged() {
    let (d, a, b) = two();
    let mut cfg = Config::new();
    cfg.insert("language".into(), json!("en-US"));
    cfg.insert("services".into(), json!([a, b]));
    cfg.insert("default_service".into(), json!(id(&a)));
    cfg.insert(
        "service_slots".into(),
        json!({"incoming": null, "outgoing": null, "region": null}),
    );
    assert_eq!(ServiceDraft::from_config(&cfg), d);
    let mut written = cfg.clone();
    ServiceDraft::from_config(&cfg).write_into(&mut written);
    assert_eq!(written, cfg);
}

#[test]
fn write_into_stores_the_edited_values() {
    let (mut d, a, b) = two();
    d.set_default(&id(&b));
    d.set_slot(ServiceSlot::Region, Some(id(&a)));
    let mut cfg = Config::new();
    d.write_into(&mut cfg);
    assert_eq!(cfg["services"], json!([a, b]));
    assert_eq!(cfg["default_service"], json!(id(&b)));
    assert_eq!(
        cfg["service_slots"],
        json!({"incoming": null, "outgoing": null, "region": id(&a)})
    );
}

#[test]
fn cards_are_listed_in_order_with_the_default_badged() {
    let (d, a, b) = two();
    let labels: Vec<String> = d.services.iter().map(|s| d.card_label(s)).collect();
    assert_eq!(
        labels,
        [
            format!("{}　{}", name(&a), tr("service.badge_default")),
            name(&b)
        ]
    );
}

#[test]
fn a_card_with_no_role_shows_its_name_alone() {
    // 沒有角色的服務不留前導空位：標記在名稱後面，沒什麼要對齊的
    let (d, _a, b) = two();
    assert_eq!(d.card_label(&b), name(&b));
}

#[test]
fn the_default_badge_comes_before_the_slot_badges() {
    let a = service("openai", &[]);
    let b = service("claude", std::slice::from_ref(&a));
    let d = draft(vec![a.clone(), b], Some(&a), [None, None, Some(&a)]);
    assert_eq!(
        d.card_label(&a),
        format!(
            "{}　{} · {}",
            name(&a),
            tr("service.badge_default"),
            tr("slot.region")
        )
    );
}

#[test]
fn several_slots_on_one_service_are_badged_in_slot_order() {
    let a = service("openai", &[]);
    let b = service("claude", std::slice::from_ref(&a));
    let d = draft(
        vec![a.clone(), b.clone()],
        Some(&a),
        [Some(&b), None, Some(&b)],
    );
    assert_eq!(
        d.card_label(&b),
        format!(
            "{}　{} · {}",
            name(&b),
            tr("slot.incoming"),
            tr("slot.region")
        )
    );
}

#[test]
fn deleting_the_default_hands_it_to_the_first_remaining() {
    let (mut d, a, b) = two();
    d.delete(&id(&a));
    assert_eq!(d.default, Some(id(&b)));
    assert_eq!(d.services, [b]);
}

#[test]
fn deleting_an_assigned_service_frees_that_slot() {
    let a = service("openai", &[]);
    let b = service("claude", std::slice::from_ref(&a));
    let mut d = draft(vec![a.clone(), b.clone()], Some(&a), [None, None, Some(&b)]);
    d.delete(&id(&b));
    assert_eq!(d.slots, [None, None, None]);
}

#[test]
fn the_last_service_cannot_be_deleted() {
    let a = service("openai", &[]);
    let mut d = draft(vec![a.clone()], Some(&a), [None; 3]);
    assert!(!d.can_delete());
    d.delete(&id(&a));
    assert_eq!(d.services.len(), 1);
}

#[test]
fn adding_to_an_empty_list_makes_it_the_default() {
    capture_logs();
    let mut d = draft(vec![], None, [None; 3]);
    let added = service("openai", &[]);
    d.apply_dialog_result(added.clone());
    assert_eq!(d.default, Some(id(&added)));
    let line = format!("[settings] first service added; default={}", id(&added));
    assert_eq!(log_lines_with(&line).len(), 1);
}

#[test]
fn a_saved_name_never_collides() {
    let (mut d, a, b) = two();
    let mut renamed = b.clone();
    renamed["name"] = json!(name(&a));
    d.apply_dialog_result(renamed);
    assert_eq!(d.services[1]["name"], json!(format!("{} (2)", name(&a))));
}

// --- ServicePane ---

struct State {
    pane: ServicePane,
    env: FakeEnv,
}

fn harness(draft: ServiceDraft) -> Harness<'static, State> {
    harness_sized(draft, egui::vec2(620.0, 660.0))
}

fn harness_sized(draft: ServiceDraft, size: egui::Vec2) -> Harness<'static, State> {
    let state = State {
        pane: ServicePane::new(draft),
        env: FakeEnv::default(),
    };
    let mut h = Harness::builder()
        .with_size(size)
        .with_context_setup(|ctx| fonts_egui::install(ctx, LANGUAGE))
        .build_ui_state(
            |ui: &mut Ui, s: &mut State| {
                theme::apply(ui.style_mut());
                s.pane.ui(ui, &s.env, TARGET);
            },
            state,
        );
    h.run_steps(3);
    h
}

fn run<S>(h: &mut Harness<S>) {
    h.run_steps(3);
}

fn press<S>(h: &mut Harness<S>, key: Key) {
    h.key_press(key);
    run(h);
}

fn click_at<S>(h: &mut Harness<S>, pos: Pos2) {
    h.event(Event::PointerMoved(pos));
    for pressed in [true, false] {
        h.event(Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::default(),
        });
    }
    run(h);
}

fn type_text<S>(h: &mut Harness<S>, text: &str) {
    h.event(Event::Text(text.into()));
    run(h);
}

fn has_dialog<S>(h: &Harness<S>, title: &str) -> bool {
    h.query_by_role_and_label(Role::Dialog, title).is_some()
}

/// 對話框裡的按鈕（背後的卡片也有同名按鈕）。
fn click_in_dialog<S>(h: &mut Harness<S>, title: &str, button: &str) {
    h.get_by_role_and_label(Role::Dialog, title)
        .get_by_role_and_label(Role::Button, button)
        .click();
    run(h);
}

fn card_buttons<S>(h: &Harness<S>, label: &str) -> Vec<Rect> {
    let mut rects: Vec<Rect> = h
        .query_all_by_role_and_label(Role::Button, label)
        .map(|n| n.rect())
        .collect();
    rects.sort_by(|a, b| a.top().total_cmp(&b.top()));
    rects
}

/// 第 `index` 張服務卡片的〔編輯〕或〔刪除〕（依畫面由上而下）。
fn click_card<S>(h: &mut Harness<S>, label: &str, index: usize) {
    let mut nodes: Vec<_> = h.query_all_by_role_and_label(Role::Button, label).collect();
    nodes.sort_by(|a, b| a.rect().top().total_cmp(&b.rect().top()));
    nodes[index].click();
    run(h);
}

fn painted_texts<S>(h: &Harness<S>) -> Vec<String> {
    h.output()
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            Shape::Text(shape) => Some(shape.galley.text().to_owned()),
            _ => None,
        })
        .collect()
}

/// 畫面上貼圖的位置（服務商圖示；字形用的是預設貼圖，不算）。
fn painted_images<S>(h: &Harness<S>) -> Vec<Rect> {
    h.output()
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            Shape::Mesh(mesh) if mesh.texture_id != TextureId::default() => Some(
                Rect::from_points(&mesh.vertices.iter().map(|v| v.pos).collect::<Vec<_>>()),
            ),
            _ => None,
        })
        .collect()
}

fn group_labels<S>(h: &Harness<S>) -> Vec<String> {
    let mut groups: Vec<_> = h.query_all_by_role(Role::Group).collect();
    groups.sort_by(|a, b| a.rect().top().total_cmp(&b.rect().top()));
    groups
        .iter()
        .filter_map(|n| n.accesskit_node().label())
        .collect()
}

fn has_label<S>(h: &Harness<S>, label: &str) -> bool {
    h.query_all_by_label(label).next().is_some()
}

#[test]
fn the_pane_draws_one_card_per_service_with_its_badges() {
    let (d, a, b) = two();
    let h = harness(d.clone());
    assert_eq!(group_labels(&h), [d.card_label(&a), d.card_label(&b)]);
}

#[test]
fn deleting_needs_confirmation_and_cancel_changes_nothing() {
    let (d, _a, b) = two();
    let mut h = harness(d.clone());
    click_card(&mut h, &tr("button.delete"), 1);
    assert!(has_dialog(&h, &tr("dialog.confirm_title")));
    assert_eq!(h.state().pane.draft(), &d);
    click_in_dialog(&mut h, &tr("dialog.confirm_title"), &tr("button.cancel"));
    assert!(!has_dialog(&h, &tr("dialog.confirm_title")));
    assert_eq!(h.state().pane.draft(), &d);

    click_card(&mut h, &tr("button.delete"), 1);
    click_in_dialog(&mut h, &tr("dialog.confirm_title"), &tr("button.delete"));
    assert!(!has_dialog(&h, &tr("dialog.confirm_title")));
    let left: Vec<String> = h.state().pane.draft().services.iter().map(id).collect();
    assert!(!left.contains(&id(&b)));
}

/// 依序按 Tab，記下每次拿到焦點的按鈕名稱。
fn tab_through<S>(h: &mut Harness<S>, presses: usize) -> Vec<String> {
    (0..presses)
        .filter_map(|_| {
            press(h, Key::Tab);
            h.query_by(|n| n.is_focused())
                .and_then(|n| n.accesskit_node().label())
        })
        .collect()
}

#[test]
fn tab_order_follows_the_cards_left_to_right() {
    let (d, _a, _b) = two();
    let mut h = harness(d);
    let buttons: Vec<String> = tab_through(&mut h, 12)
        .into_iter()
        .filter(|l| *l == tr("button.edit") || *l == tr("button.delete"))
        .collect();
    assert_eq!(
        buttons[..2],
        [tr("button.edit"), tr("button.delete")],
        "{buttons:?}"
    );
}

#[test]
fn the_first_tab_in_the_confirm_dialog_lands_on_cancel() {
    // 刪除是破壞性操作，鍵盤第一站不可以是它
    let (d, _a, _b) = two();
    let mut h = harness(d);
    click_card(&mut h, &tr("button.delete"), 1);
    assert_eq!(
        tab_through(&mut h, 2),
        [tr("button.cancel"), tr("button.delete")]
    );
}

#[test]
fn the_form_footer_tabs_cancel_before_ok() {
    let (d, _a, _b) = two();
    let mut h = harness(d);
    click_card(&mut h, &tr("button.edit"), 0);
    let labels = tab_through(&mut h, 20);
    let at = |label: String| labels.iter().position(|l| *l == label);
    let (cancel, ok) = (at(tr("button.cancel")), at(tr("button.ok")));
    assert!(cancel.is_some() && cancel < ok, "{labels:?}");
}

#[test]
fn deleting_an_assigned_service_says_its_slots_fall_back() {
    // 指定它的用途會被悄悄退回預設，確認框要先講
    let a = service("openai", &[]);
    let b = service("claude", std::slice::from_ref(&a));
    let d = draft(vec![a.clone(), b.clone()], Some(&a), [Some(&b), None, None]);
    let mut h = harness(d);
    click_card(&mut h, &tr("button.delete"), 1);
    let question = t("service.confirm_delete_in_use", &[("name", &name(&b))]);
    assert!(painted_texts(&h).concat().contains(&question));
}

#[test]
fn deleting_an_unassigned_service_keeps_the_plain_question() {
    let (d, _a, b) = two();
    let mut h = harness(d);
    click_card(&mut h, &tr("button.delete"), 1);
    let question = t("service.confirm_delete", &[("name", &name(&b))]);
    let in_use = t("service.confirm_delete_in_use", &[("name", &name(&b))]);
    let texts = painted_texts(&h).concat();
    assert!(texts.contains(&question));
    assert!(!texts.contains(&in_use));
}

#[test]
fn the_last_service_says_why_delete_is_greyed_out() {
    // 灰掉的按鈕自己不會說話：只剩一組時寫明刪不掉的原因
    let a = service("openai", &[]);
    let h = harness(draft(vec![a.clone()], Some(&a), [None; 3]));
    let label = tr("button.delete");
    let delete = h.get_by_role_and_label(Role::Button, &label);
    assert!(delete.accesskit_node().is_disabled());
    assert!(painted_texts(&h).contains(&tr("service.keep_one")));
}

#[test]
fn the_keep_one_hint_goes_away_once_there_are_two() {
    let (d, _a, _b) = two();
    let h = harness(d);
    assert!(!painted_texts(&h).contains(&tr("service.keep_one")));
    for delete in h.query_all_by_role_and_label(Role::Button, &tr("button.delete")) {
        assert!(!delete.accesskit_node().is_disabled());
    }
}

#[test]
fn slots_collapsed_when_everything_follows_the_default() {
    let (d, _a, _b) = two();
    let h = harness(d);
    assert!(
        h.query_by_role_and_label(Role::ComboBox, &tr("slot.incoming"))
            .is_none()
    );
}

#[test]
fn slots_expanded_when_one_is_assigned() {
    let a = service("openai", &[]);
    let b = service("claude", std::slice::from_ref(&a));
    let h = harness(draft(
        vec![a.clone(), b.clone()],
        Some(&a),
        [None, None, Some(&b)],
    ));
    for slot in ["slot.incoming", "slot.outgoing", "slot.region"] {
        h.get_by_role_and_label(Role::ComboBox, &tr(slot));
    }
}

#[test]
fn slot_options_start_with_follow_the_default() {
    let a = service("openai", &[]);
    let b = service("claude", std::slice::from_ref(&a));
    let mut h = harness(draft(
        vec![a.clone(), b.clone()],
        Some(&a),
        [None, None, Some(&b)],
    ));
    h.get_by_role_and_label(Role::ComboBox, &tr("slot.incoming"))
        .click();
    run(&mut h);
    let top = |label: &str| h.get_by_role_and_label(Role::Button, label).rect().top();
    let tops = [
        top(&tr("service.follow_default")),
        top(&name(&a)),
        top(&name(&b)),
    ];
    assert!(tops.is_sorted(), "{tops:?}");
}

#[test]
fn assigning_a_slot_rebadges_the_cards() {
    let (d, _a, b) = two();
    let mut h = harness(d);
    h.get_by_label(&tr("service.slots_title")).click();
    run(&mut h);
    h.get_by_role_and_label(Role::ComboBox, &tr("slot.incoming"))
        .click();
    run(&mut h);
    h.get_by_role_and_label(Role::Button, &name(&b)).click();
    run(&mut h);
    assert_eq!(h.state().pane.draft().slots[0], Some(id(&b)));
    assert_eq!(
        group_labels(&h)[1],
        format!("{}　{}", name(&b), tr("slot.incoming"))
    );
}

#[test]
fn choosing_the_default_moves_the_badge() {
    let (d, a, b) = two();
    let mut h = harness(d);
    h.get_by_role_and_label(Role::ComboBox, &tr("service.default"))
        .click();
    run(&mut h);
    h.get_by_role_and_label(Role::Button, &name(&b)).click();
    run(&mut h);
    assert_eq!(h.state().pane.draft().default, Some(id(&b)));
    assert_eq!(
        group_labels(&h),
        [
            name(&a),
            format!("{}　{}", name(&b), tr("service.badge_default"))
        ]
    );
}

#[test]
fn a_long_name_does_not_squeeze_the_card_buttons() {
    // 名稱長到撐滿卡片時，被壓縮的要是名稱而不是〔編輯〕〔刪除〕
    let mut a = service("openai", &[]);
    a["name"] = json!("My self-hosted DeepSeek endpoint with a very very long name indeed");
    let b = service("claude", std::slice::from_ref(&a));
    let d = draft(vec![a.clone(), b], Some(&a), [Some(&a); 3]);
    let h = harness_sized(d, egui::vec2(480.0, 660.0));
    let edit = card_buttons(&h, &tr("button.edit"))[0];
    let delete = card_buttons(&h, &tr("button.delete"))[0];
    let title = h.get_by_label(&name(&a)).rect();
    assert!(delete.right() <= 480.0, "{delete:?}");
    assert!(edit.width() > 40.0 && delete.width() > 40.0);
    assert!(title.right() <= edit.left(), "{title:?} vs {edit:?}");
}

#[test]
fn every_provider_ships_an_icon() {
    // 新增服務商時不會悄悄少一張圖
    let missing: Vec<&str> = providers()
        .iter()
        .map(|p| p.key)
        .filter(|key| provider_icon(key).is_none())
        .collect();
    assert!(missing.is_empty(), "{missing:?}");
}

#[test]
fn each_card_shows_its_providers_icon() {
    let (d, _a, _b) = two();
    let h = harness(d);
    let cards: Vec<Rect> = {
        let mut groups: Vec<_> = h.query_all_by_role(Role::Group).map(|n| n.rect()).collect();
        groups.sort_by(|a, b| a.top().total_cmp(&b.top()));
        groups
    };
    for card in cards {
        let icons: Vec<Rect> = painted_images(&h)
            .into_iter()
            .filter(|r| card.contains_rect(*r))
            .collect();
        assert_eq!(icons.len(), 1, "{card:?}");
        // 圖示跨整張卡片垂直置中，不是跟標題那一行對齊
        assert!(
            (icons[0].center().y - card.center().y).abs() <= 2.0,
            "{:?} vs {card:?}",
            icons[0]
        );
    }
}

#[test]
fn a_card_without_its_icon_still_renders() {
    // 圖檔缺失只是少一個圖示，分頁照畫
    let mut a = service("openai", &[]);
    a["provider"] = json!("mystery");
    let d = draft(vec![a.clone()], Some(&a), [None; 3]);
    let h = harness(d.clone());
    assert_eq!(group_labels(&h), [d.card_label(&a)]);
    assert!(painted_images(&h).is_empty());
}

// --- 服務表單對話框 ---

fn invalid_draft() -> (ServiceDraft, Value) {
    let a = new_service("openai", &[]);
    (draft(vec![a.clone()], Some(&a), [None; 3]), a)
}

#[test]
fn dialog_ok_keeps_the_dialog_open_when_invalid() {
    let (d, _a) = invalid_draft();
    let mut h = harness(d.clone());
    click_card(&mut h, &tr("button.edit"), 0);
    click_in_dialog(&mut h, &tr("service.dialog_title"), &tr("button.ok"));
    assert!(has_dialog(&h, &tr("service.dialog_title")));
    assert!(has_label(&h, &tr("dialog.incomplete_title")));
    assert!(has_label(&h, &tr("error.need_model")));
    assert_eq!(h.state().pane.draft(), &d);
}

#[test]
fn dialog_ok_saves_the_full_service_when_valid() {
    capture_logs();
    let (d, a, _b) = two();
    let mut h = harness(d);
    click_card(&mut h, &tr("button.edit"), 0);
    h.get_by_label(&tr("field.service_name")).click();
    run(&mut h);
    type_text(&mut h, " 2");
    click_in_dialog(&mut h, &tr("service.dialog_title"), &tr("button.ok"));
    assert!(!has_dialog(&h, &tr("service.dialog_title")));
    let mut expected = a.clone();
    expected["name"] = json!(format!("{} 2", name(&a)));
    assert_eq!(h.state().pane.draft().services[0], expected);
    assert!(!log_lines_with("[settings] service saved (provider=openai, has_key=True)").is_empty());
}

#[test]
fn saving_never_logs_the_api_key() {
    capture_logs();
    let mut a = service("gemini", &[]);
    a["api_key"] = json!("secret-key-d41d8cd98f00b204");
    let mut h = harness(draft(vec![a.clone()], Some(&a), [None; 3]));
    click_card(&mut h, &tr("button.edit"), 0);
    click_in_dialog(&mut h, &tr("service.dialog_title"), &tr("button.ok"));
    assert!(!log_lines_with("[settings] service saved (provider=gemini, has_key=True)").is_empty());
    assert!(!log_text().contains("secret-key-d41d8cd98f00b204"));
}

#[test]
fn dialog_cancel_changes_nothing() {
    let (d, _a, _b) = two();
    let mut h = harness(d.clone());
    click_card(&mut h, &tr("button.edit"), 0);
    h.get_by_label(&tr("field.service_name")).click();
    run(&mut h);
    type_text(&mut h, "x");
    click_in_dialog(&mut h, &tr("service.dialog_title"), &tr("button.cancel"));
    assert!(!has_dialog(&h, &tr("service.dialog_title")));
    assert_eq!(h.state().pane.draft(), &d);
}

#[test]
fn dialog_escape_cancels() {
    let (d, _a, _b) = two();
    let mut h = harness(d.clone());
    click_card(&mut h, &tr("button.edit"), 0);
    h.get_by_label(&tr("field.service_name")).click();
    run(&mut h);
    type_text(&mut h, "x");
    press(&mut h, Key::Escape);
    assert!(!has_dialog(&h, &tr("service.dialog_title")));
    assert_eq!(h.state().pane.draft(), &d);
}

#[test]
fn the_close_button_cancels_the_dialog() {
    let (d, _a, _b) = two();
    let mut h = harness(d.clone());
    click_card(&mut h, &tr("button.edit"), 0);
    click_in_dialog(&mut h, &tr("service.dialog_title"), &tr("tooltip.close"));
    assert!(!has_dialog(&h, &tr("service.dialog_title")));
    assert_eq!(h.state().pane.draft(), &d);
}

#[test]
fn clicking_the_scrim_does_not_close_the_dialog() {
    let (d, _a, _b) = two();
    let mut h = harness(d);
    click_card(&mut h, &tr("button.edit"), 0);
    click_at(&mut h, Pos2::new(4.0, 4.0));
    assert!(has_dialog(&h, &tr("service.dialog_title")));
}

#[test]
fn an_open_dialog_blocks_the_pane_behind_it() {
    // 對話框開著時背後的卡片按不到，否則能在編輯途中刪掉正在編輯的服務
    let (d, _a, _b) = two();
    let mut h = harness(d.clone());
    click_card(&mut h, &tr("button.delete"), 1);
    let behind = card_buttons(&h, &tr("button.edit"))[0];
    click_at(&mut h, behind.center());
    assert!(!has_dialog(&h, &tr("service.dialog_title")));
    assert!(has_dialog(&h, &tr("dialog.confirm_title")));
}

#[test]
fn test_results_reach_the_open_dialog() {
    let (d, _a, _b) = two();
    let mut h = harness(d);
    click_card(&mut h, &tr("button.edit"), 0);
    click_in_dialog(&mut h, &tr("service.dialog_title"), &tr("button.test"));
    let round = h.state().env.tests.borrow().last().expect("test started").0;
    let result = Ok(TestOutcome {
        translated: "x".into(),
        elapsed: std::time::Duration::from_millis(1000),
    });
    h.state_mut()
        .pane
        .apply(&SettingsEvent::TestDone { round, result });
    run(&mut h);
    assert!(has_label(&h, &t("test.connected", &[("seconds", "1.0")])));
}

#[test]
fn the_service_dialog_stays_put_when_its_content_grows() {
    // 測試結果出現時對話框若重新置中，按鈕會在游標下跳開
    let (d, _a, _b) = two();
    let mut h = harness(d);
    click_card(&mut h, &tr("button.edit"), 0);
    let close = || tr("tooltip.close");
    let before = h.get_by_role_and_label(Role::Button, &close()).rect();
    click_in_dialog(&mut h, &tr("service.dialog_title"), &tr("button.test"));
    let round = h.state().env.tests.borrow().last().expect("test started").0;
    let result = Ok(TestOutcome {
        translated: "x".into(),
        elapsed: std::time::Duration::from_millis(1000),
    });
    h.state_mut()
        .pane
        .apply(&SettingsEvent::TestDone { round, result });
    run(&mut h);
    assert_eq!(
        h.get_by_role_and_label(Role::Button, &close()).rect(),
        before
    );
}

#[test]
fn the_dialog_buttons_stay_visible_in_a_small_window() {
    // 1366×768、150% 縮放時工作區約 910×485 point，內容要捲動、〔確定〕要看得到
    let mut custom = service("custom", &[]);
    custom["base_url"] = json!("http://localhost:1234");
    let d = draft(vec![custom.clone()], Some(&custom), [None; 3]);
    let mut h = harness_sized(d, egui::vec2(620.0, 485.0));
    click_card(&mut h, &tr("button.edit"), 0);
    let ok = h
        .get_by_role_and_label(Role::Dialog, &tr("service.dialog_title"))
        .get_by_role_and_label(Role::Button, &tr("button.ok"))
        .rect();
    assert!(ok.bottom() <= 485.0, "{ok:?}");
}

// --- 新增流程與服務商選擇 ---

#[test]
fn add_service_opens_nothing_when_the_pick_is_cancelled() {
    // 沒選服務商就不該開一張空表單：那時還不知道要畫哪家的欄位
    let (d, _a, _b) = two();
    let mut h = harness(d.clone());
    h.get_by_label(&tr("button.add_service")).click();
    run(&mut h);
    assert!(has_dialog(&h, &tr("service.pick_provider")));
    click_in_dialog(&mut h, &tr("service.pick_provider"), &tr("button.cancel"));
    assert!(!has_dialog(&h, &tr("service.pick_provider")));
    assert!(!has_dialog(&h, &tr("service.dialog_title")));
    assert_eq!(h.state().pane.draft(), &d);
}

#[test]
fn escape_cancels_the_provider_pick() {
    let (d, _a, _b) = two();
    let mut h = harness(d.clone());
    h.get_by_label(&tr("button.add_service")).click();
    run(&mut h);
    press(&mut h, Key::Escape);
    assert!(!has_dialog(&h, &tr("service.pick_provider")));
    assert!(!has_dialog(&h, &tr("service.dialog_title")));
}

#[test]
fn the_cancel_button_closes_the_picker() {
    // 重新設計後選擇框以標題列的關閉鈕取消，沒有獨立的取消鈕
    let (d, _a, _b) = two();
    let mut h = harness(d.clone());
    h.get_by_label(&tr("button.add_service")).click();
    run(&mut h);
    click_in_dialog(&mut h, &tr("service.pick_provider"), &tr("tooltip.close"));
    assert!(!has_dialog(&h, &tr("service.pick_provider")));
    assert!(!has_dialog(&h, &tr("service.dialog_title")));
    assert_eq!(h.state().pane.draft(), &d);
}

#[test]
fn cancelling_the_provider_pick_changes_nothing() {
    let (d, _a, _b) = two();
    let mut h = harness(d.clone());
    click_card(&mut h, &tr("button.edit"), 0);
    click_in_dialog(&mut h, &tr("service.dialog_title"), &tr("button.change"));
    press(&mut h, Key::Escape);
    assert!(!has_dialog(&h, &tr("service.pick_provider")));
    assert!(has_dialog(&h, &tr("service.dialog_title")));
    assert!(!has_label(&h, &tr("field.base_url")));
    click_in_dialog(&mut h, &tr("service.dialog_title"), &tr("button.ok"));
    assert_eq!(h.state().pane.draft(), &d);
}

#[test]
fn add_service_drafts_the_picked_provider() {
    let (d, _a, _b) = two();
    let mut h = harness(d);
    h.get_by_label(&tr("button.add_service")).click();
    run(&mut h);
    click_in_dialog(&mut h, &tr("service.pick_provider"), &tr("provider.custom"));
    assert!(!has_dialog(&h, &tr("service.pick_provider")));
    assert!(has_dialog(&h, &tr("service.dialog_title")));
    h.get_by_label(&tr("field.base_url")).click();
    run(&mut h);
    type_text(&mut h, "http://localhost:11434/v1");
    h.get_by_label(&tr("field.model")).click();
    run(&mut h);
    type_text(&mut h, "qwen3");
    click_in_dialog(&mut h, &tr("service.dialog_title"), &tr("button.ok"));
    let services = &h.state().pane.draft().services;
    assert_eq!(services.len(), 3);
    assert_eq!(services[2]["provider"], json!("custom"));
    assert_eq!(services[2]["base_url"], json!("http://localhost:11434/v1"));
}

#[test]
fn escape_closes_only_the_top_dialog() {
    let (d, _a, _b) = two();
    let mut h = harness(d);
    click_card(&mut h, &tr("button.edit"), 0);
    click_in_dialog(&mut h, &tr("service.dialog_title"), &tr("button.change"));
    assert!(has_dialog(&h, &tr("service.pick_provider")));
    press(&mut h, Key::Escape);
    assert!(!has_dialog(&h, &tr("service.pick_provider")));
    assert!(has_dialog(&h, &tr("service.dialog_title")));
    press(&mut h, Key::Escape);
    assert!(!has_dialog(&h, &tr("service.dialog_title")));
}

#[test]
fn picking_from_the_form_switches_its_provider_and_keeps_it_modal() {
    // 關掉疊在上面的選擇框後，服務表單仍擋住背後的分頁
    let (d, _a, _b) = two();
    let mut h = harness(d);
    click_card(&mut h, &tr("button.edit"), 0);
    click_in_dialog(&mut h, &tr("service.dialog_title"), &tr("button.change"));
    click_in_dialog(&mut h, &tr("service.pick_provider"), &tr("provider.custom"));
    assert!(!has_dialog(&h, &tr("service.pick_provider")));
    assert!(has_label(&h, &tr("field.base_url")));
    let behind = h.get_by_label(&tr("button.add_service")).rect();
    click_at(&mut h, behind.center());
    assert!(!has_dialog(&h, &tr("service.pick_provider")));
    assert!(has_dialog(&h, &tr("service.dialog_title")));
}

#[test]
fn the_picker_cards_keep_a_margin_below_the_header() {
    // 第一張卡片不可貼著標題列；標題列底＝關閉鈕底加 8 的下內距
    let (d, _a, _b) = two();
    let mut h = harness(d);
    h.get_by_label(&tr("button.add_service")).click();
    run(&mut h);
    let title = tr("service.pick_provider");
    let dialog = h.get_by_role_and_label(Role::Dialog, &title);
    let header_bottom = dialog
        .get_by_role_and_label(Role::Button, &tr("tooltip.close"))
        .rect()
        .bottom()
        + 8.0;
    let first = dialog
        .get_by_role_and_label(Role::Button, &tr(providers()[0].label_key))
        .rect();
    assert!(
        first.top() >= header_bottom + 8.0,
        "{first:?} vs {header_bottom}"
    );
}

#[test]
fn the_picker_title_appears_once() {
    // 同一句話不畫兩次：對話框標題已經在講這件事
    let (d, _a, _b) = two();
    let mut h = harness(d);
    h.get_by_label(&tr("button.add_service")).click();
    run(&mut h);
    let title = tr("service.pick_provider");
    let count = painted_texts(&h).iter().filter(|t| **t == title).count();
    assert_eq!(count, 1);
}

// --- provider_cards ---

struct Cards {
    layout: CardLayout,
    picked: Vec<&'static str>,
}

fn cards_harness(layout: CardLayout) -> Harness<'static, Cards> {
    i18n::set_language(LANGUAGE);
    let mut h = Harness::builder()
        .with_size(egui::vec2(460.0, 520.0))
        .with_context_setup(|ctx| fonts_egui::install(ctx, LANGUAGE))
        .build_ui_state(
            |ui: &mut Ui, s: &mut Cards| {
                theme::apply(ui.style_mut());
                if let Some(key) = provider_cards(ui, s.layout) {
                    s.picked.push(key);
                }
            },
            Cards {
                layout,
                picked: Vec::new(),
            },
        );
    h.run_steps(2);
    h
}

fn card_rects(h: &Harness<Cards>) -> Vec<Rect> {
    providers()
        .iter()
        .map(|p| {
            h.get_by_role_and_label(Role::Button, &tr(p.label_key))
                .rect()
        })
        .collect()
}

#[test]
fn one_titled_card_per_provider() {
    for layout in [CardLayout::List, CardLayout::Grid] {
        let h = cards_harness(layout);
        assert_eq!(h.query_all_by_role(Role::Button).count(), providers().len());
        card_rects(&h);
    }
}

#[test]
fn each_card_describes_its_provider() {
    let h = cards_harness(CardLayout::List);
    let texts = painted_texts(&h).concat();
    for p in providers() {
        assert!(texts.contains(&tr(p.desc_key)), "{}", p.key);
    }
}

#[test]
fn the_list_stacks_and_the_grid_has_two_columns() {
    let list = card_rects(&cards_harness(CardLayout::List));
    assert!(list.windows(2).all(|w| w[0].bottom() < w[1].top()));
    let grid = card_rects(&cards_harness(CardLayout::Grid));
    assert_eq!(grid[0].top(), grid[1].top());
    assert!(grid[0].right() < grid[1].left());
    assert!(grid[1].bottom() < grid[2].top());
    assert!(grid.iter().all(|r| r.height() >= 150.0));
}

#[test]
fn clicking_anywhere_inside_a_card_picks_its_provider() {
    // 整張卡都可點：標題、說明、圖示與空白處
    for layout in [CardLayout::List, CardLayout::Grid] {
        let mut h = cards_harness(layout);
        for (rect, provider) in card_rects(&h).into_iter().zip(providers()) {
            let inset = rect.shrink(3.0);
            let icon = match layout {
                CardLayout::List => Pos2::new(rect.left() + 30.0, rect.center().y),
                CardLayout::Grid => rect.left_top() + egui::vec2(32.0, 32.0),
            };
            for pos in [inset.left_top(), inset.right_bottom(), rect.center(), icon] {
                h.state_mut().picked.clear();
                click_at(&mut h, pos);
                assert_eq!(h.state().picked, [provider.key], "{layout:?} {pos:?}");
            }
        }
    }
}

#[test]
fn each_provider_card_shows_its_icon() {
    let h = cards_harness(CardLayout::List);
    let images = painted_images(&h);
    for rect in card_rects(&h) {
        assert_eq!(images.iter().filter(|r| rect.contains_rect(**r)).count(), 1);
    }
}

#[test]
fn cards_are_reachable_and_activatable_by_keyboard() {
    // 純鍵盤使用者在精靈這一步沒有別的出路：〔下一步〕要選到服務商才會亮
    for layout in [CardLayout::List, CardLayout::Grid] {
        let mut h = cards_harness(layout);
        press(&mut h, Key::Tab);
        let first = tr(providers()[0].label_key);
        assert!(h.get_by_role_and_label(Role::Button, &first).is_focused());
        press(&mut h, Key::Enter);
        assert_eq!(h.state().picked, [providers()[0].key]);
        press(&mut h, Key::Tab);
        let second = tr(providers()[1].label_key);
        assert!(h.get_by_role_and_label(Role::Button, &second).is_focused());
        press(&mut h, Key::Space);
        assert_eq!(h.state().picked, [providers()[0].key, providers()[1].key]);
    }
}

#[test]
fn only_dark_single_colour_provider_icons_are_lightened_for_the_dark_theme() {
    for (key, expect) in [
        ("openai", true),
        ("custom", true),
        ("claude", false),
        ("gemini", false),
    ] {
        let data = eframe::icon_data::from_png_bytes(provider_icon(key).unwrap()).unwrap();
        let mut rgba = data.rgba.clone();
        assert_eq!(
            lighten_dark_monochrome(&mut rgba, theme::TEXT),
            expect,
            "{key}"
        );
        let alpha = |px: &[u8]| {
            px.as_chunks::<4>()
                .0
                .iter()
                .map(|p| p[3])
                .collect::<Vec<_>>()
        };
        assert_eq!(
            alpha(&rgba),
            alpha(&data.rgba),
            "{key}: shape and transparency kept"
        );
        if expect {
            let lit = rgba.as_chunks::<4>().0.iter().find(|p| p[3] > 0).unwrap();
            assert_eq!(
                &lit[..3],
                &[theme::TEXT.r(), theme::TEXT.g(), theme::TEXT.b()]
            );
        } else {
            assert_eq!(rgba, data.rgba, "{key}: coloured logo untouched");
        }
    }
}
