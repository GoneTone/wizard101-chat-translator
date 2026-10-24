//! 設定視窗的「術語表」分頁：草稿讀寫、清單上的新增／複製／刪除、啟用與儲存檢查。

use crate::common;

use eframe::egui::accesskit::Role;
use eframe::egui::{self, Ui};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use common::kittest::{
    click, click_in_dialog, click_nth, has_label, painted_texts, run, text_sections,
    themed_harness, tr,
};
use common::settings_env::FakeEnv;
use common::settings_window::{State as WindowState, custom_cfg, harness as window_harness};
use wizard101_chat_translator::config::Config;
use wizard101_chat_translator::config::glossaries::StoredGlossary;
use wizard101_chat_translator::glossary::Entry;
use wizard101_chat_translator::i18n::{self, t};
use wizard101_chat_translator::ui::settings::glossary_draft::GlossaryDraft;
use wizard101_chat_translator::ui::settings::glossary_pane::GlossaryPane;
use wizard101_chat_translator::ui::settings::theme;
use wizard101_chat_translator::ui::settings::window::SettingsAction;

/// 本檔所有測試都用同一個介面語言，不會互相干擾。
const LANGUAGE: &str = "en-US";

fn entry(source: &str, target: &str) -> Entry {
    Entry {
        source: source.into(),
        target: target.into(),
    }
}

fn glossary(id: &str, name: &str, builtin: Option<&str>, entries: Vec<Entry>) -> StoredGlossary {
    StoredGlossary {
        id: id.into(),
        name: name.into(),
        builtin: builtin.map(Into::into),
        entries,
    }
}

fn draft(glossaries: Vec<StoredGlossary>, active: Option<&str>) -> GlossaryDraft {
    i18n::set_language(LANGUAGE);
    GlossaryDraft {
        glossaries,
        active: active.map(Into::into),
    }
}

fn names(d: &GlossaryDraft) -> Vec<String> {
    d.glossaries.iter().map(|g| g.name.clone()).collect()
}

struct State {
    pane: GlossaryPane,
}

fn harness(draft: GlossaryDraft) -> Harness<'static, State> {
    let state = State {
        pane: GlossaryPane::new(draft),
    };
    themed_harness(
        egui::vec2(620.0, 660.0),
        LANGUAGE,
        |ui: &mut Ui, s: &mut State| s.pane.ui(ui),
        state,
    )
}

fn add_blank(h: &mut Harness<State>) {
    click(h, Role::Button, &tr("button.add_glossary"));
    click_in_dialog(h, &tr("glossary.new_title"), &tr("glossary.start_blank"));
}

#[test]
fn draft_round_trips_config() {
    i18n::set_language(LANGUAGE);
    let mut cfg = Config::new();
    let glossaries = json!([
        {"id": "a", "name": "Mine", "builtin": null,
         "entries": {"Ravenwood": "R", "Krokotopia": "K"}},
        {"id": "b", "name": "Other", "builtin": null, "entries": {}}
    ]);
    cfg.insert("glossaries".into(), glossaries.clone());
    cfg.insert("active_glossary".into(), json!("a"));
    let mut written = Config::new();
    GlossaryDraft::from_config(&cfg).write_into(&mut written);
    assert_eq!(written["glossaries"], glossaries);
    assert_eq!(written["active_glossary"], json!("a"));
}

#[test]
fn write_into_trims_nothing_without_a_builtin() {
    let entries = vec![entry("b", "2"), entry("a", "1"), entry("c", "3")];
    let d = draft(vec![glossary("a", "Mine", None, entries)], None);
    let mut cfg = Config::new();
    d.write_into(&mut cfg);
    assert_eq!(
        serde_json::to_string(&cfg["glossaries"][0]["entries"]).unwrap(),
        r#"{"b":"2","a":"1","c":"3"}"#
    );
}

#[test]
fn new_glossary_names_dedupe() {
    let mut h = harness(draft(Vec::new(), None));
    add_blank(&mut h);
    assert!(has_label(&h, &tr("glossary.back_to_list")));
    click(&mut h, Role::Button, &tr("glossary.back_to_list"));
    add_blank(&mut h);
    let base = tr("glossary.default_name");
    assert_eq!(
        names(h.state().pane.draft()),
        [base.clone(), format!("{base} (2)")]
    );
}

#[test]
fn duplicate_copies_entries_and_builtin() {
    let original = glossary("a", "Mine", Some("xx"), vec![entry("Ravenwood", "R")]);
    let mut h = harness(draft(vec![original.clone()], None));
    click_nth(&mut h, &tr("button.copy"), 0);
    click_nth(&mut h, &tr("button.copy"), 0);
    let d = h.state().pane.draft();
    let copy = t("glossary.copy_name", &[("name", "Mine")]);
    assert_eq!(
        names(d),
        ["Mine".into(), copy.clone(), format!("{copy} (2)")]
    );
    for duplicate in &d.glossaries[1..] {
        assert_eq!(duplicate.entries, original.entries);
        assert_eq!(duplicate.builtin, original.builtin);
        assert_ne!(duplicate.id, original.id);
    }
    assert_ne!(d.glossaries[1].id, d.glossaries[2].id);
}

#[test]
fn deleting_the_active_glossary_disables_it() {
    let glossaries = vec![
        glossary("a", "Mine", None, Vec::new()),
        glossary("b", "Other", None, Vec::new()),
    ];
    let mut h = harness(draft(glossaries, Some("a")));
    click_nth(&mut h, &tr("button.delete"), 0);
    click_in_dialog(&mut h, &tr("dialog.confirm_title"), &tr("button.delete"));
    let d = h.state().pane.draft();
    assert_eq!(names(d), ["Other"]);
    assert_eq!(d.active, None);
}

#[test]
fn save_blocks_on_glossary_errors() {
    i18n::set_language(LANGUAGE);
    let mut cfg = custom_cfg();
    cfg.insert(
        "glossaries".into(),
        json!([{"id": "a", "name": "  ", "builtin": null, "entries": {}}]),
    );
    let mut h: Harness<WindowState> =
        window_harness(&cfg, FakeEnv::default(), egui::vec2(840.0, 660.0), LANGUAGE);
    click(&mut h, Role::Button, &tr("button.save"));
    assert!(
        !h.state()
            .actions
            .iter()
            .any(|a| matches!(a, SettingsAction::Save(_)))
    );
    assert!(has_label(&h, &tr("error.glossary_name_blank")));
}

#[test]
fn glossaries_tab_is_listed_after_services() {
    i18n::set_language(LANGUAGE);
    let h = window_harness(
        &custom_cfg(),
        FakeEnv::default(),
        egui::vec2(840.0, 660.0),
        LANGUAGE,
    );
    let mut tabs: Vec<_> = h.query_all_by_role(Role::Tab).collect();
    tabs.sort_by(|a, b| a.rect().top().total_cmp(&b.rect().top()));
    let labels: Vec<String> = tabs
        .iter()
        .filter_map(|n| n.accesskit_node().label())
        .collect();
    assert_eq!(labels[2], tr("settings.tab.glossaries"));
}

fn group_labels(h: &Harness<State>) -> Vec<String> {
    let mut groups: Vec<_> = h.query_all_by_role(Role::Group).collect();
    groups.sort_by(|a, b| a.rect().top().total_cmp(&b.rect().top()));
    groups
        .iter()
        .filter_map(|n| n.accesskit_node().label())
        .collect()
}

fn two() -> Vec<StoredGlossary> {
    vec![
        glossary("a", "Mine", None, vec![entry("x", "1"), entry("y", "2")]),
        glossary("b", "Other", None, Vec::new()),
    ]
}

#[test]
fn card_shows_counts_and_active_badge() {
    let h = harness(draft(two(), Some("a")));
    assert_eq!(
        group_labels(&h),
        [
            format!("Mine\u{3000}{}", tr("glossary.badge_active")),
            "Other".into()
        ]
    );
    assert!(painted_texts(&h).contains(&tr("glossary.badge_active")));
    let summary = format!(
        "{} · {}",
        t("glossary.custom_count", &[("count", "2")]),
        tr("glossary.no_builtin")
    );
    assert!(painted_texts(&h).contains(&summary));
}

#[test]
fn choosing_from_the_combo_sets_the_active_glossary() {
    let mut h = harness(draft(two(), None));
    for (option, expected) in [("Other".to_owned(), Some("b")), (tr("glossary.none"), None)] {
        h.get_by_role_and_label(Role::ComboBox, &tr("glossary.active"))
            .click();
        run(&mut h);
        h.get_by_role_and_label(Role::Button, &option).click();
        run(&mut h);
        assert_eq!(h.state().pane.draft().active.as_deref(), expected);
    }
}

#[test]
fn cancelling_delete_keeps_the_glossary() {
    let mut h = harness(draft(two(), Some("a")));
    click_nth(&mut h, &tr("button.delete"), 0);
    click_in_dialog(&mut h, &tr("dialog.confirm_title"), &tr("button.cancel"));
    assert!(
        h.query_by_role_and_label(Role::Dialog, &tr("dialog.confirm_title"))
            .is_none()
    );
    let d = h.state().pane.draft();
    assert_eq!(names(d), ["Mine", "Other"]);
    assert_eq!(d.active.as_deref(), Some("a"));
}

/// 打開只有一份術語表 `Mine` 的編輯畫面。
fn editing(entries: Vec<Entry>, builtin: Option<&str>) -> Harness<'static, State> {
    let mut h = harness(draft(vec![glossary("a", "Mine", builtin, entries)], None));
    click(&mut h, Role::Button, &tr("button.edit"));
    h
}

fn custom_entries(h: &Harness<State>) -> Vec<Entry> {
    h.state().pane.draft().glossaries[0].entries.clone()
}

/// 名稱是 `label` 的輸入欄，由上往下排。
fn fields_named<'a>(h: &'a Harness<State>, label: &'a str) -> Vec<egui_kittest::Node<'a>> {
    let mut nodes: Vec<_> = h
        .query_all_by_role_and_label(Role::TextInput, label)
        .collect();
    nodes.sort_by(|a, b| a.rect().top().total_cmp(&b.rect().top()));
    nodes
}

fn type_into_nth(h: &mut Harness<State>, label: &str, index: usize, text: &str) {
    fields_named(h, label)[index].click();
    run(h);
    h.event(egui::Event::Text(text.into()));
    run(h);
}

#[test]
fn editor_shows_duplicate_source_hint() {
    let h = editing(vec![entry("A", "甲"), entry("a", "乙")], None);
    let hint = tr("glossary.row_duplicate_source");
    let painted = painted_texts(&h).iter().filter(|t| **t == hint).count();
    assert_eq!(painted, 1);
    assert!(
        text_sections(&h, &hint)
            .iter()
            .all(|(_, color)| *color == theme::ERROR)
    );
    let second_row = fields_named(&h, &tr("glossary.source"))[1].rect().top();
    assert!(h.get_by_label(&hint).rect().top() > second_row);
}

#[test]
fn adding_and_deleting_rows_updates_the_draft() {
    let mut h = editing(vec![entry("Fire Elf", "火精靈")], None);
    click(&mut h, Role::Button, &tr("button.add_entry"));
    assert!(!painted_texts(&h).contains(&tr("glossary.row_incomplete")));
    type_into_nth(&mut h, &tr("glossary.source"), 1, "Hollo");
    type_into_nth(&mut h, &tr("glossary.target"), 1, "哈樓");
    assert_eq!(
        custom_entries(&h),
        [entry("Fire Elf", "火精靈"), entry("Hollo", "哈樓")]
    );
    click_nth(&mut h, &tr("glossary.delete_entry"), 1);
    assert_eq!(custom_entries(&h), [entry("Fire Elf", "火精靈")]);
}

#[test]
fn search_filters_custom_rows() {
    let mut h = editing(
        vec![entry("Hollo", "哈樓"), entry("Fire Elf", "火精靈")],
        None,
    );
    type_into_nth(&mut h, &tr("glossary.search"), 0, "hol");
    let shown: Vec<_> = fields_named(&h, &tr("glossary.source"))
        .iter()
        .filter_map(|n| n.value())
        .collect();
    assert_eq!(shown, ["Hollo"]);
}

#[test]
fn back_returns_to_the_list() {
    let mut h = editing(Vec::new(), None);
    click(&mut h, Role::Button, &tr("glossary.back_to_list"));
    assert_eq!(group_labels(&h), ["Mine"]);
}

#[test]
fn missing_builtin_shows_a_notice() {
    let h = editing(Vec::new(), Some("xx-XX"));
    assert!(has_label(&h, &tr("glossary.builtin_missing")));
    assert!(painted_texts(&h).contains(&"xx-XX".to_owned()));
}

fn shown_sources(h: &Harness<State>) -> Vec<String> {
    fields_named(h, &tr("glossary.source"))
        .iter()
        .map(|n| n.value().unwrap_or_default())
        .collect()
}

#[test]
fn search_matches_the_target_side() {
    let mut h = editing(
        vec![entry("Hollo", "哈樓"), entry("Fire Elf", "火精靈")],
        None,
    );
    type_into_nth(&mut h, &tr("glossary.search"), 0, "精靈");
    assert_eq!(shown_sources(&h), ["Fire Elf"]);
}

#[test]
fn adding_a_row_while_searching_shows_it() {
    let mut h = editing(vec![entry("Hollo", "哈樓")], None);
    type_into_nth(&mut h, &tr("glossary.search"), 0, "zzz");
    assert!(shown_sources(&h).is_empty());
    click(&mut h, Role::Button, &tr("button.add_entry"));
    assert_eq!(shown_sources(&h), [""]);
    type_into_nth(&mut h, &tr("glossary.source"), 0, "Fire Elf");
    assert_eq!(shown_sources(&h), ["Fire Elf"]);
    assert!(painted_texts(&h).contains(&tr("glossary.row_incomplete")));
}

#[test]
fn fully_blank_rows_are_dropped_on_save() {
    let entries = vec![entry("Hollo", "哈樓"), entry("", ""), entry("  ", "")];
    let d = draft(vec![glossary("a", "Mine", None, entries)], None);
    assert!(d.errors().is_empty());
    let mut cfg = Config::new();
    d.write_into(&mut cfg);
    assert_eq!(cfg["glossaries"][0]["entries"], json!({"Hollo": "哈樓"}));
}
