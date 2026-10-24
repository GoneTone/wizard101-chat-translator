//! 設定視窗的「術語表」分頁：草稿讀寫、清單上的新增／複製／刪除、啟用與儲存檢查。

use crate::common;

use eframe::egui::accesskit::Role;
use eframe::egui::{self, Ui};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use common::kittest::{click, click_in_dialog, click_nth, has_label, themed_harness, tr};
use common::settings_env::FakeEnv;
use common::settings_window::{State as WindowState, custom_cfg, harness as window_harness};
use wizard101_chat_translator::config::Config;
use wizard101_chat_translator::config::glossaries::StoredGlossary;
use wizard101_chat_translator::glossary::Entry;
use wizard101_chat_translator::i18n::{self, t};
use wizard101_chat_translator::ui::settings::glossary_draft::GlossaryDraft;
use wizard101_chat_translator::ui::settings::glossary_pane::GlossaryPane;
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
