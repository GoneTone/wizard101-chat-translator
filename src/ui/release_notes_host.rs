//! 托管本版說明視窗：背景取得目前版本的 release，結果快取，視窗的動作轉成重試、開網頁與關窗。

use std::panic::AssertUnwindSafe;

use eframe::egui::{self, OpenUrl};
use futures_util::FutureExt;
use tokio::runtime::Handle;

use crate::logs::log;
use crate::runtime::updater::{
    PROJECT_URL, Release, fetch_release, release_tag_url, update_client,
};
use crate::ui::events::{UiEvent, UiPoster};
use crate::ui::settings::release_notes_window::{NotesAction, NotesState, ReleaseNotesView};
use crate::ui::settings_host::{HostedWindow, Spec, WindowOps};
use crate::ui::update_host::UPDATE;
use crate::util::panic_message;

const RELEASE_NOTES: Spec = Spec {
    name: "release notes window",
    title_key: "release_notes.title",
    ..UPDATE
};

/// 本版說明視窗與它的背景請求；所有方法都在 UI 執行緒呼叫。
pub struct ReleaseNotesHost {
    /// 視窗開著才有。
    view: Option<ReleaseNotesView>,
    /// 沒請求過是 `None`；關窗後仍保留，下次開窗直接用。
    state: Option<NotesState>,
    win: HostedWindow,
    rt: Handle,
    ui: UiPoster,
    /// releases API 的網址（測試換成 wiremock）。
    api: String,
}

impl ReleaseNotesHost {
    pub fn new(rt: Handle, ui: UiPoster, api: String) -> Self {
        Self {
            view: None,
            state: None,
            win: HostedWindow::new(RELEASE_NOTES),
            rt,
            ui,
            api,
        }
    }

    /// 換掉視窗操作（測試用：不搶前景、假的工作區）。
    pub fn set_window_ops(&mut self, ops: WindowOps) {
        self.win.ops = ops;
    }

    /// 開窗；已開著就叫到前景，已有結果或請求中就不再發請求。
    pub fn open(&mut self) {
        if self.view.is_some() {
            log("[update] release notes window already open; bringing it to the front");
            self.win.bring_to_front();
            return;
        }
        self.view = Some(ReleaseNotesView::new());
        let state = match &self.state {
            None => "none",
            Some(NotesState::Loading) => "loading",
            Some(NotesState::Loaded(_)) => "loaded",
            Some(NotesState::NotFound) => "not_found",
            Some(NotesState::Failed(_)) => "failed",
        };
        log(format!(
            "[update] release notes window opened: state={state}"
        ));
        match self.state {
            Some(NotesState::Loaded(_)) => log("[update] using cached release notes"),
            Some(NotesState::Loading) => {}
            _ => self.request(),
        }
    }

    /// 視窗是否開著。
    pub fn is_open(&self) -> bool {
        self.view.is_some()
    }

    /// 視窗的 HWND；沒開或還沒找到是 0。
    pub fn hwnd(&self) -> isize {
        self.win.hwnd
    }

    /// 目前的載入狀態（測試用）。
    pub fn state(&self) -> Option<&NotesState> {
        self.state.as_ref()
    }

    /// 背景請求的結果（`UiEvent::ReleaseNotesLoaded`）；視窗關著也照樣存。
    pub fn loaded(&mut self, result: Result<Option<Release>, String>) {
        self.state = Some(match result {
            Ok(Some(release)) => NotesState::Loaded(release),
            Ok(None) => NotesState::NotFound,
            Err(error) => NotesState::Failed(error),
        });
    }

    /// 每幀呼叫：開著就畫出視窗並處理動作。
    pub fn show(&mut self, ctx: &egui::Context, known: &[isize]) {
        let Self {
            view, state, win, ..
        } = self;
        let (Some(view), Some(state)) = (view, state.as_ref()) else {
            return;
        };
        let actions = win.show(ctx, known, |ui, closing| {
            let mut actions = view.ui(ui, env!("CARGO_PKG_VERSION"), state);
            if closing {
                actions.push(NotesAction::Close);
            }
            actions
        });
        for action in actions {
            self.act(ctx, action);
        }
    }

    /// 處理視窗動作；公開給測試直接觸發。
    pub fn act(&mut self, ctx: &egui::Context, action: NotesAction) {
        match action {
            NotesAction::Retry => self.request(),
            NotesAction::OpenOnGitHub => {
                let url = github_url(self.state.as_ref());
                log(format!("[update] opening release notes page: {url}"));
                ctx.open_url(OpenUrl::new_tab(url));
            }
            NotesAction::Close => {
                if self.view.take().is_some() {
                    log("[update] release notes window closed");
                    self.win.reset();
                }
            }
        }
    }

    /// 發背景請求；請求中不重發，結果以 `ReleaseNotesLoaded` 投遞。
    fn request(&mut self) {
        if self.state == Some(NotesState::Loading) {
            return;
        }
        self.state = Some(NotesState::Loading);
        let url = release_tag_url(&self.api, env!("CARGO_PKG_VERSION"));
        let ui = self.ui.clone();
        self.rt.spawn(async move {
            let fetch = async {
                fetch_release(&update_client(&url), &url)
                    .await
                    .map_err(|e| e.0)
            };
            let result = match AssertUnwindSafe(fetch).catch_unwind().await {
                Ok(result) => result,
                Err(panic) => Err(panic_message(panic.as_ref())),
            };
            if let Err(error) = &result {
                log(format!("[update] release notes fetch failed: {error}"));
            }
            ui.post(UiEvent::ReleaseNotesLoaded(result));
        });
    }
}

/// 「在 GitHub 查看」要開的網址：已載入用該 release 的頁面，否則開專案的 releases 清單。
pub fn github_url(state: Option<&NotesState>) -> String {
    match state {
        Some(NotesState::Loaded(release)) => release.url.clone(),
        _ => format!("{PROJECT_URL}/releases"),
    }
}
