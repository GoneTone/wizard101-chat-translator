//! 設定視窗的「術語表」分頁：草稿讀寫、清單上的新增／複製／刪除、啟用與儲存檢查。

use crate::common;

use std::path::Path;

use eframe::egui::accesskit::{Role, Toggled};
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
use wizard101_chat_translator::glossary::{Entry, parse_entries};
use wizard101_chat_translator::i18n::{self, t};
use wizard101_chat_translator::translation::errors::localized;
use wizard101_chat_translator::ui::settings::env::SettingsEvent;
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
        ..Default::default()
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
    env: FakeEnv,
}

fn harness(draft: GlossaryDraft) -> Harness<'static, State> {
    harness_with(draft, FakeEnv::default())
}

fn harness_with(draft: GlossaryDraft, env: FakeEnv) -> Harness<'static, State> {
    let state = State {
        pane: GlossaryPane::new(draft),
        env,
    };
    themed_harness(
        egui::vec2(620.0, 660.0),
        LANGUAGE,
        |ui: &mut Ui, s: &mut State| s.pane.ui(ui, &s.env),
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
    assert!(has_label(
        &h,
        &t("error.glossary_name_blank", &[("index", "1")])
    ));
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
    editing_with(entries, builtin, FakeEnv::default())
}

fn editing_with(
    entries: Vec<Entry>,
    builtin: Option<&str>,
    env: FakeEnv,
) -> Harness<'static, State> {
    let d = draft(vec![glossary("a", "Mine", builtin, entries)], None);
    let mut h = harness_with(d, env);
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

fn numbered(count: usize) -> Vec<Entry> {
    (0..count)
        .map(|i| entry(&format!("S{i}"), &format!("T{i}")))
        .collect()
}

#[test]
fn long_custom_list_only_draws_rows_near_the_view() {
    let h = editing(numbered(2000), None);
    let drawn = fields_named(&h, &tr("glossary.source")).len();
    assert!((1..200).contains(&drawn), "drew {drawn} rows");
    let label = tr("button.add_entry");
    let add = h.get_by_role_and_label(Role::Button, &label);
    assert!(add.rect().top() > 2000.0 * theme::CONTROL_HEIGHT);
}

/// 包在捲動區裡、已進入編輯畫面的 `entries`；`to_bottom` 時每幀捲到底。
fn scrollable(entries: Vec<Entry>, to_bottom: bool) -> Harness<'static, State> {
    let d = draft(vec![glossary("a", "Mine", None, entries)], None);
    let state = State {
        pane: GlossaryPane::new(d),
        env: FakeEnv::default(),
    };
    let mut h = themed_harness(
        egui::vec2(620.0, 660.0),
        LANGUAGE,
        move |ui: &mut Ui, s: &mut State| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                s.pane.ui(ui, &s.env);
                if to_bottom {
                    ui.scroll_to_cursor(Some(egui::Align::Max));
                }
            });
        },
        state,
    );
    click(&mut h, Role::Button, &tr("button.edit"));
    h
}

#[test]
fn scrolling_to_the_bottom_draws_the_last_rows() {
    let mut h = scrollable(numbered(2000), true);
    for _ in 0..5 {
        run(&mut h);
    }
    let painted = painted_texts(&h);
    assert!(painted.contains(&"S1999".to_owned()));
    assert!(!painted.contains(&"S0".to_owned()));
}

#[test]
fn tabbing_past_the_view_scrolls_the_focused_field_into_view() {
    let mut h = scrollable(numbered(200), false);
    let label = tr("glossary.source");
    let last = fields_named(&h, &label)
        .into_iter()
        .rfind(|n| n.rect().bottom() <= 660.0)
        .unwrap();
    last.click();
    run(&mut h);
    for _ in 0..12 {
        h.key_press(egui::Key::Tab);
        run(&mut h);
    }
    let focused = h.query_by(|n| n.is_focused()).unwrap().rect();
    assert!(
        focused.top() >= 0.0 && focused.bottom() <= 660.0,
        "{focused:?}"
    );
}

#[test]
fn column_captions_line_up_with_their_fields() {
    let h = editing(vec![entry("Hollo", "哈樓")], None);
    let left = |key| {
        let label = tr(key);
        let caption = h.get_by_role_and_label(Role::Label, &label).rect().left();
        (caption, fields_named(&h, &label)[0].rect().left())
    };
    let (source, target) = (left("glossary.source"), left("glossary.target"));
    let (captions, fields) = (target.0 - source.0, target.1 - source.1);
    assert!((captions - fields).abs() < 1.0, "{captions} vs {fields}");
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
fn deleting_a_row_does_not_hand_its_undo_history_to_the_next_row() {
    let mut h = editing(vec![entry("A", "甲"), entry("B", "乙")], None);
    type_into_nth(&mut h, &tr("glossary.source"), 0, "x");
    click_nth(&mut h, &tr("glossary.delete_entry"), 0);
    fields_named(&h, &tr("glossary.source"))[0].click();
    run(&mut h);
    h.event(egui::Event::Key {
        key: egui::Key::Z,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    });
    run(&mut h);
    assert_eq!(custom_entries(&h), [entry("B", "乙")]);
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

#[test]
fn typed_entries_are_trimmed_on_save() {
    let entries = vec![entry(" Hollo ", " 哈樓	")];
    let d = draft(vec![glossary("a", "Mine", None, entries)], None);
    let mut cfg = Config::new();
    d.write_into(&mut cfg);
    assert_eq!(cfg["glossaries"][0]["entries"], json!({"Hollo": "哈樓"}));
}

#[test]
fn glossary_names_are_trimmed_on_save() {
    let d = draft(
        vec![
            glossary("a", "X ", None, Vec::new()),
            glossary("b", "X", None, Vec::new()),
        ],
        None,
    );
    let mut cfg = Config::new();
    d.write_into(&mut cfg);
    assert_eq!(cfg["glossaries"][0]["name"], "X");
    assert_eq!(cfg["glossaries"][1]["name"], "X (2)");
}

#[test]
fn unknown_glossary_keys_survive_a_round_trip() {
    i18n::set_language(LANGUAGE);
    let mut cfg = Config::new();
    let glossaries = json!([
        {"id": "a", "name": "Mine", "builtin": null, "entries": {"k": "v"},
         "note": "kept", "extra": {"n": 1}}
    ]);
    cfg.insert("glossaries".into(), glossaries.clone());
    let mut written = Config::new();
    GlossaryDraft::from_config(&cfg).write_into(&mut written);
    assert_eq!(written["glossaries"], glossaries);
}

#[test]
fn glossary_errors_name_the_glossary() {
    let d = draft(
        vec![
            glossary("a", "Mine", None, vec![entry("A", "甲"), entry("a", "乙")]),
            glossary("b", " ", None, Vec::new()),
            glossary("c", "Other", None, vec![entry("B", "")]),
        ],
        None,
    );
    let shown: Vec<String> = d
        .errors()
        .iter()
        .map(|(key, args)| localized(key, args))
        .collect();
    assert_eq!(
        shown,
        [
            t("error.glossary_name_blank", &[("index", "2")]),
            t("error.glossary_duplicate_source", &[("name", "Mine")]),
            t("error.glossary_incomplete_entry", &[("name", "Other")]),
        ]
    );
}

#[test]
fn editor_flags_a_blank_name() {
    let d = draft(vec![glossary("a", "  ", None, Vec::new())], None);
    let mut h = harness(d);
    click(&mut h, Role::Button, &tr("button.edit"));
    let hint = tr("error.need_name");
    assert!(
        text_sections(&h, &hint)
            .iter()
            .any(|(_, color)| *color == theme::ERROR)
    );
}

/// 選檔對話框會「選到」`dir` 裡的 `name`；`content` 不是 `None` 時先寫好檔案。
fn env_picking(dir: &Path, name: &str, content: Option<&str>) -> FakeEnv {
    let path = dir.join(name);
    if let Some(content) = content {
        std::fs::write(&path, content).unwrap();
    }
    FakeEnv {
        picked_path: Some(path),
        ..FakeEnv::default()
    }
}

/// 把預設路徑當成第 `index` 次選檔的結果送回分頁。
fn deliver_file_pick(h: &mut Harness<State>, index: usize) {
    let round = h.state().env.file_picks.borrow()[index];
    let path = h.state().env.picked_path.clone();
    h.state_mut()
        .pane
        .apply(&SettingsEvent::GlossaryFilePicked { round, path });
    run(h);
}

fn import_new(h: &mut Harness<State>) {
    click(h, Role::Button, &tr("button.add_glossary"));
    click_in_dialog(h, &tr("glossary.new_title"), &tr("glossary.start_import"));
}

#[test]
fn import_from_file_creates_a_glossary_named_after_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let env = env_picking(dir.path(), "friends.json", Some(r#"{"Hollo":"哈樓"}"#));
    let mut h = harness_with(draft(Vec::new(), None), env);
    import_new(&mut h);
    deliver_file_pick(&mut h, 0);
    let d = h.state().pane.draft();
    assert_eq!(names(d), ["friends"]);
    assert_eq!(d.glossaries[0].entries, [entry("Hollo", "哈樓")]);
    assert!(has_label(&h, &tr("glossary.back_to_list")));
}

#[test]
fn import_failure_shows_the_parse_error() {
    let dir = tempfile::tempdir().unwrap();
    let content = r#"{"a":1}"#;
    let env = env_picking(dir.path(), "bad.json", Some(content));
    let mut h = harness_with(draft(two(), None), env);
    import_new(&mut h);
    deliver_file_pick(&mut h, 0);
    assert!(has_label(&h, &tr("glossary.import_failed")));
    assert!(has_label(&h, &parse_entries(content).unwrap_err()));
    assert_eq!(names(h.state().pane.draft()), ["Mine", "Other"]);
    assert!(has_label(&h, &tr("button.add_glossary")));
}

/// 在 `Mine` 的編輯畫面按〔匯入檔案〕，選到內容是 `content` 的檔案。
fn merging(current: Vec<Entry>, content: &str) -> (tempfile::TempDir, Harness<'static, State>) {
    let dir = tempfile::tempdir().unwrap();
    let env = env_picking(dir.path(), "more.json", Some(content));
    let mut h = editing_with(current, None, env);
    click(&mut h, Role::Button, &tr("glossary.import"));
    deliver_file_pick(&mut h, 0);
    (dir, h)
}

fn checked(h: &Harness<State>, label: &str) -> bool {
    h.get_by_role_and_label(Role::Dialog, &tr("glossary.merge_title"))
        .get_by_role_and_label(Role::CheckBox, label)
        .accesskit_node()
        .toggled()
        == Some(Toggled::True)
}

fn toggle(h: &mut Harness<State>, label: &str) {
    h.get_by_role_and_label(Role::Dialog, &tr("glossary.merge_title"))
        .get_by_role_and_label(Role::CheckBox, label)
        .click();
    run(h);
}

#[test]
fn merge_dialog_lists_conflicts_and_applies_checked_rows() {
    let (_dir, mut h) = merging(vec![entry("B", "乙")], r#"{"B":"丙","C":"丁"}"#);
    assert!(has_label(&h, &t("glossary.merge_added", &[("added", "1")])));
    assert!(has_label(&h, &t("glossary.merge_total", &[("total", "2")])));
    let title = tr("glossary.merge_title");
    let dialog = h.get_by_role_and_label(Role::Dialog, &title);
    assert_eq!(dialog.query_all_by_role(Role::CheckBox).count(), 2);
    toggle(&mut h, "B");
    click_in_dialog(&mut h, &tr("glossary.merge_title"), &tr("button.merge"));
    assert!(
        h.query_by_role_and_label(Role::Dialog, &tr("glossary.merge_title"))
            .is_none()
    );
    assert_eq!(custom_entries(&h), [entry("B", "丙"), entry("C", "丁")]);
}

#[test]
fn merge_rows_line_up_with_their_headers_and_sit_close() {
    let current = vec![entry("Bob", "乙"), entry("Amy", "甲")];
    let (_dir, h) = merging(current, r#"{"Bob":"丙","Amy":"丁"}"#);
    let title = tr("glossary.merge_title");
    let dialog = h.get_by_role_and_label(Role::Dialog, &title);
    let rect = |label: &str| dialog.get_by_label(label).rect();
    assert_eq!(
        rect(&tr("glossary.current_target")).left(),
        rect("乙").left()
    );
    assert_eq!(rect(&tr("glossary.file_target")).left(), rect("丙").left());
    let (bob, amy) = (rect("Bob"), rect("Amy"));
    assert!(amy.top() - bob.bottom() < 10.0, "{bob:?} {amy:?}");
    assert!((rect("乙").center().y - bob.center().y).abs() < 1.0);
}

#[test]
fn merge_cancel_changes_nothing() {
    let (_dir, mut h) = merging(vec![entry("B", "乙")], r#"{"B":"丙","C":"丁"}"#);
    toggle(&mut h, "B");
    click_in_dialog(&mut h, &tr("glossary.merge_title"), &tr("button.cancel"));
    assert!(
        h.query_by_role_and_label(Role::Dialog, &tr("glossary.merge_title"))
            .is_none()
    );
    assert_eq!(custom_entries(&h), [entry("B", "乙")]);
}

#[test]
fn select_all_toggles_every_conflict() {
    let current = vec![entry("A", "1"), entry("B", "2")];
    let (_dir, mut h) = merging(current, r#"{"A":"x","B":"y"}"#);
    let all = tr("glossary.select_all");
    toggle(&mut h, "A");
    assert!(!checked(&h, &all));
    toggle(&mut h, &all);
    assert!(checked(&h, &all) && checked(&h, "A") && checked(&h, "B"));
    toggle(&mut h, &all);
    assert!(!checked(&h, &all) && !checked(&h, "A") && !checked(&h, "B"));
}

#[test]
fn export_writes_only_custom_entries() {
    let dir = tempfile::tempdir().unwrap();
    let env = env_picking(dir.path(), "out.json", None);
    let custom = vec![entry("Hollo", "哈樓"), entry("Fire Elf", "火精靈")];
    let mut h = editing_with(custom.clone(), Some("xx-XX"), env);
    click(&mut h, Role::Button, &tr("glossary.export"));
    let (round, name) = h.state().env.save_picks.borrow()[0].clone();
    assert_eq!(name, "Mine.json");
    let path = h.state().env.picked_path.clone();
    h.state_mut()
        .pane
        .apply(&SettingsEvent::GlossarySavePathPicked { round, path });
    run(&mut h);
    let written = std::fs::read_to_string(dir.path().join("out.json")).unwrap();
    assert_eq!(parse_entries(&written).unwrap(), custom);
}

#[test]
fn stale_file_pick_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let env = env_picking(dir.path(), "friends.json", Some(r#"{"Hollo":"哈樓"}"#));
    let mut h = harness_with(draft(Vec::new(), None), env);
    import_new(&mut h);
    import_new(&mut h);
    deliver_file_pick(&mut h, 0);
    assert!(names(h.state().pane.draft()).is_empty());
    deliver_file_pick(&mut h, 1);
    assert_eq!(names(h.state().pane.draft()), ["friends"]);
}

#[test]
fn import_accepts_a_utf8_bom_file() {
    let dir = tempfile::tempdir().unwrap();
    let env = env_picking(dir.path(), "bom.json", Some("\u{feff}{\"Hollo\":\"哈樓\"}"));
    let mut h = harness_with(draft(Vec::new(), None), env);
    import_new(&mut h);
    deliver_file_pick(&mut h, 0);
    let d = h.state().pane.draft();
    assert_eq!(names(d), ["bom"]);
    assert_eq!(d.glossaries[0].entries, [entry("Hollo", "哈樓")]);
}

#[test]
fn export_file_name_is_sanitized() {
    for (name, expected) in [
        (
            r#" a\b/c:d*e?f"g<h>i|j"#,
            "a_b_c_d_e_f_g_h_i_j.json".to_owned(),
        ),
        ("x\u{7}y", "x_y.json".to_owned()),
        ("  ", format!("{}.json", tr("glossary.default_name"))),
    ] {
        let d = draft(vec![glossary("a", name, None, Vec::new())], None);
        let mut h = harness_with(d, FakeEnv::default());
        click(&mut h, Role::Button, &tr("button.edit"));
        click(&mut h, Role::Button, &tr("glossary.export"));
        assert_eq!(h.state().env.save_picks.borrow()[0].1, expected);
    }
}
