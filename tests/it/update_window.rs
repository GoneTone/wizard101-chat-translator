//! 更新內容視窗：卡片清單、各下載階段的底部列與按鈕、Markdown 內文。

use crate::common::kittest::{click, themed_harness, tr};
use eframe::egui::accesskit::Role;
use eframe::egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use wizard101_chat_translator::i18n::t;
use wizard101_chat_translator::runtime::update_job::Phase;
use wizard101_chat_translator::runtime::updater::Release;
use wizard101_chat_translator::ui::settings::update_window::{UpdateAction, UpdateView, can_close};

const MIB: u64 = 1 << 20;

struct Window {
    view: UpdateView,
    releases: Vec<Release>,
    phase: Phase,
    progress: (u64, u64),
    unsaved: bool,
    actions: Vec<UpdateAction>,
}

fn release(version: &str, body: &str) -> Release {
    Release {
        version: version.into(),
        url: format!("https://example.com/v{version}"),
        published_at: "2026-11-03T08:00:00Z".into(),
        body: body.into(),
        asset: None,
    }
}

fn window_with(
    releases: Vec<Release>,
    phase: Phase,
    progress: (u64, u64),
    unsaved: bool,
) -> Harness<'static, Window> {
    let state = Window {
        view: UpdateView::new(),
        releases,
        phase,
        progress,
        unsaved,
        actions: Vec::new(),
    };
    themed_harness(
        vec2(620.0, 600.0),
        "en-US",
        |ui, w: &mut Window| {
            let actions = w.view.ui(ui, &w.releases, &w.phase, w.progress, w.unsaved);
            w.actions.extend(actions);
        },
        state,
    )
}

fn window(phase: Phase, progress: (u64, u64), unsaved: bool) -> Harness<'static, Window> {
    let releases = vec![
        release("1.2.0", "Newest notes"),
        release("1.1.0", "Older notes"),
    ];
    window_with(releases, phase, progress, unsaved)
}

/// 點名稱是 `key` 譯文的按鈕，回傳這次點擊產生的動作。
fn press(h: &mut Harness<'static, Window>, key: &str) -> Vec<UpdateAction> {
    h.state_mut().actions.clear();
    click(h, Role::Button, &tr(key));
    std::mem::take(&mut h.state_mut().actions)
}

fn disabled(h: &Harness<'static, Window>, key: &str) -> bool {
    h.get_by_role_and_label(Role::Button, &tr(key))
        .accesskit_node()
        .is_disabled()
}

/// 卡片由新到舊，最新那張有「最新」標籤；〔立即更新〕開始下載、〔稍後〕關閉。
#[test]
fn idle_lists_every_release_newest_first() {
    let mut h = window(Phase::Idle, (0, 0), false);
    let newest = h.get_by_label("v1.2.0").rect().top();
    let older = h.get_by_label("v1.1.0").rect().top();
    assert!(newest < older);
    h.get_by_label(&tr("update.latest_tag"));
    assert_eq!(press(&mut h, "update.now"), [UpdateAction::Start]);
    assert_eq!(press(&mut h, "update.later"), [UpdateAction::Close]);
}

/// 下載中有進度條與 MB 進度文字，只剩〔取消下載〕可按。
#[test]
fn downloading_shows_progress_and_only_cancel() {
    let mut h = window(Phase::Downloading, (25 * MIB, 61 * MIB), false);
    h.get_by_role(Role::ProgressIndicator);
    let text = t("update.downloading", &[("done", "25.0"), ("total", "61.0")]);
    h.get_by_label(&text);
    assert!(h.query_by_label(&tr("update.now")).is_none());
    assert!(!disabled(&h, "update.cancel"));
    assert_eq!(press(&mut h, "update.cancel"), [UpdateAction::Cancel]);
}

/// 下載量到總量時是驗證中，不能取消。
#[test]
fn verifying_disables_cancel() {
    let h = window(Phase::Downloading, (61 * MIB, 61 * MIB), false);
    h.get_by_label(&tr("update.verifying"));
    assert!(disabled(&h, "update.cancel"));
}

/// 還沒拿到總量（0／0）時仍是下載中，可以取消。
#[test]
fn downloading_at_zero_bytes_is_not_verifying() {
    let h = window(Phase::Downloading, (0, 0), false);
    assert!(h.query_by_label(&tr("update.verifying")).is_none());
    assert!(!disabled(&h, "update.cancel"));
}

/// 重新啟動中不能取消。
#[test]
fn restarting_disables_cancel() {
    let h = window(Phase::Restarting, (61 * MIB, 61 * MIB), false);
    h.get_by_label(&tr("update.restarting"));
    assert!(disabled(&h, "update.cancel"));
}

/// 失敗時顯示原始錯誤，可重試或前往 GitHub。
#[test]
fn failed_offers_github_and_retry() {
    let phase = Phase::Failed {
        error: "boom".into(),
        replace: false,
    };
    let mut h = window(phase, (0, 0), false);
    h.get_by_label_contains("boom");
    assert_eq!(press(&mut h, "update.retry"), [UpdateAction::Retry]);
    assert_eq!(
        press(&mut h, "update.open_github"),
        [UpdateAction::OpenReleasePage]
    );
}

/// 取消後可關閉或重新下載。
#[test]
fn cancelled_offers_close_and_redownload() {
    let mut h = window(Phase::Cancelled, (0, 0), false);
    h.get_by_label(&tr("update.cancelled"));
    assert_eq!(press(&mut h, "update.close"), [UpdateAction::Close]);
    assert_eq!(press(&mut h, "update.redownload"), [UpdateAction::Start]);
}

/// 下載完成但設定有沒儲存的變更：警告並提供放棄變更重新啟動。
#[test]
fn ready_with_unsaved_settings_asks_to_discard() {
    let mut h = window(Phase::Ready, (61 * MIB, 61 * MIB), true);
    h.get_by_label(&tr("update.unsaved"));
    assert!(h.query_by_label(&tr("update.restart")).is_none());
    assert_eq!(
        press(&mut h, "update.discard_restart"),
        [UpdateAction::Restart]
    );
    assert_eq!(press(&mut h, "update.later"), [UpdateAction::Close]);
}

/// 下載完成且設定都存好了：直接提供重新啟動。
#[test]
fn ready_without_unsaved_settings_offers_restart() {
    let mut h = window(Phase::Ready, (61 * MIB, 61 * MIB), false);
    h.get_by_label(&tr("update.ready"));
    assert!(h.query_by_label(&tr("update.discard_restart")).is_none());
    assert_eq!(press(&mut h, "update.restart"), [UpdateAction::Restart]);
}

/// 內文以 Markdown 渲染，不露出原始標記。
#[test]
fn markdown_body_is_rendered() {
    let h = window_with(
        vec![release("1.2.0", "## What's New\n\n- item")],
        Phase::Idle,
        (0, 0),
        false,
    );
    h.get_by_label("What's New");
    h.get_by_label("item");
    assert!(h.query_by_label_contains("##").is_none());
}

/// 只有下載中（含驗證）與重新啟動中不能關視窗。
#[test]
fn can_close_only_outside_download_and_restart() {
    assert!(!can_close(&Phase::Downloading));
    assert!(!can_close(&Phase::Restarting));
    for phase in [
        Phase::Idle,
        Phase::Ready,
        Phase::Cancelled,
        Phase::Failed {
            error: String::new(),
            replace: true,
        },
    ] {
        assert!(can_close(&phase), "{phase:?}");
    }
}
