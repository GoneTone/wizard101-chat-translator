//! 本版說明視窗：載入中、內文、找不到、失敗與底部按鈕。

use crate::common::kittest::{click, themed_harness, tr};
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use wizard101_chat_translator::i18n::t;
use wizard101_chat_translator::runtime::updater::Release;
use wizard101_chat_translator::ui::settings::release_notes_window::{
    NotesAction, NotesState, ReleaseNotesView,
};

const VERSION: &str = "1.3.1";

struct Window {
    view: ReleaseNotesView,
    state: NotesState,
    actions: Vec<NotesAction>,
}

fn release(published_at: &str, body: &str) -> Release {
    Release {
        version: VERSION.into(),
        url: format!("https://example.com/v{VERSION}"),
        published_at: published_at.into(),
        body: body.into(),
        asset: None,
    }
}

fn window(state: NotesState) -> Harness<'static, Window> {
    let window = Window {
        view: ReleaseNotesView::new(),
        state,
        actions: Vec::new(),
    };
    themed_harness(
        eframe::egui::vec2(620.0, 600.0),
        "en-US",
        |ui, w: &mut Window| {
            let actions = w.view.ui(ui, VERSION, &w.state);
            w.actions.extend(actions);
        },
        window,
    )
}

fn press(h: &mut Harness<'static, Window>, key: &str) -> Vec<NotesAction> {
    h.state_mut().actions.clear();
    click(h, Role::Button, &tr(key));
    std::mem::take(&mut h.state_mut().actions)
}

fn has_retry(h: &Harness<'static, Window>) -> bool {
    h.query_by_role_and_label(Role::Button, &tr("local.dialog.retry"))
        .is_some()
}

/// 載入中：標題與版本號照常顯示，內文位置是佔位條，沒有重試按鈕。
#[test]
fn loading_shows_placeholders_not_text() {
    let h = window(NotesState::Loading);
    h.get_by_label(&t("release_notes.heading", &[("version", VERSION)]));
    h.get_by_label("v1.3.1");
    assert!(h.get_all_by_label(&tr("local.loading")).count() >= 1);
    assert!(!has_retry(&h));
}

/// 載入完成：顯示 Markdown 內文，不是最新版所以沒有「最新」標籤。
#[test]
fn loaded_shows_the_markdown_body() {
    let h = window(NotesState::Loaded(release(
        "2026-11-03T08:00:00Z",
        "## Fixed\n- a bug",
    )));
    h.get_by_label("Fixed");
    h.get_by_label("a bug");
    assert!(h.query_by_label(&tr("update.latest_tag")).is_none());
}

/// 發布日期解析不了就原樣顯示。
#[test]
fn unparsable_date_is_shown_as_is() {
    let h = window(NotesState::Loaded(release("someday", "notes")));
    h.get_by_label(&t("update.published", &[("date", "someday")]));
}

/// 找不到：顯示說明文字，不提供重試。
#[test]
fn not_found_has_no_retry() {
    let h = window(NotesState::NotFound);
    h.get_by_label(&t("release_notes.not_found", &[("version", VERSION)]));
    assert!(!has_retry(&h));
}

/// 失敗：顯示原文錯誤，按重試送出 `Retry`。
#[test]
fn failed_offers_retry() {
    let mut h = window(NotesState::Failed("HTTP 403".into()));
    h.get_by_label(&t("release_notes.failed", &[("error", "HTTP 403")]));
    assert_eq!(press(&mut h, "local.dialog.retry"), [NotesAction::Retry]);
}

/// 底部列〔在 GitHub 上查看〕與〔關閉〕各送出對應動作。
#[test]
fn footer_buttons_send_actions() {
    let mut h = window(NotesState::Loading);
    assert_eq!(
        press(&mut h, "release_notes.open_github"),
        [NotesAction::OpenOnGitHub]
    );
    assert_eq!(press(&mut h, "update.close"), [NotesAction::Close]);
}
