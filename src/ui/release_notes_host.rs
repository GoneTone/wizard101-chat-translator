//! 托管本版說明視窗：背景取得目前版本的 release，結果快取，視窗的動作轉成重試、開網頁與關窗。

use eframe::egui::{self, OpenUrl, vec2};
use tokio::runtime::Handle;

use crate::logs::log;
use crate::runtime::updater::{
    PROJECT_URL, Release, fetch_release, release_tag_url, update_client,
};
use crate::ui::events::{UiEvent, UiPoster};
use crate::ui::settings::release_notes_window::{NotesAction, NotesState, ReleaseNotesView};
use crate::ui::settings_host::{HostedWindow, Spec, WindowOps};

const RELEASE_NOTES: Spec = Spec {
    name: "release notes window",
    title_key: "release_notes.title",
    size: vec2(620.0, 600.0),
    min_size: vec2(480.0, 360.0),
    topmost_once: true,
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
        match self.state {
            Some(NotesState::Loaded(_)) => log("[update] using cached release notes"),
            Some(NotesState::Loading) => {}
            _ => self.request(),
        }
    }

    pub fn is_open(&self) -> bool {
        self.view.is_some()
    }

    /// 視窗的 HWND；沒開或還沒找到是 0。
    pub fn hwnd(&self) -> isize {
        self.win.hwnd
    }

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

    /// 處理使用者在視窗按下的動作。
    pub fn act(&mut self, ctx: &egui::Context, action: NotesAction) {
        match action {
            NotesAction::Retry => self.request(),
            NotesAction::OpenOnGitHub => {
                let url = match &self.state {
                    Some(NotesState::Loaded(release)) => release.url.clone(),
                    _ => format!("{PROJECT_URL}/releases"),
                };
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

    fn request(&mut self) {
        self.state = Some(NotesState::Loading);
        let url = release_tag_url(&self.api, env!("CARGO_PKG_VERSION"));
        let ui = self.ui.clone();
        self.rt.spawn(async move {
            let result = fetch_release(&update_client(&url), &url)
                .await
                .map_err(|e| {
                    log(format!("[update] release notes fetch failed: {}", e.0));
                    e.0
                });
            ui.post(UiEvent::ReleaseNotesLoaded(result));
        });
    }
}
