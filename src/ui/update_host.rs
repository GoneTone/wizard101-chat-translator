//! 托管更新內容視窗：把 `UpdateView` 的動作轉給 `UpdateJob`，下載驗證完後決定要不要直接重新啟動。

use std::path::{Path, PathBuf};
use std::time::Duration;

use eframe::egui::{self, OpenUrl, ViewportCommand, vec2};
use tokio::runtime::Handle;

use crate::logs::log;
use crate::runtime::update_job::{Phase, UpdateJob};
use crate::runtime::updater::Release;
use crate::ui::events::{UiEvent, UiPoster};
use crate::ui::settings::update_window::{UpdateAction, UpdateView, can_close};
use crate::ui::settings_host::{HostedWindow, Spec, WindowOps};

const UPDATE: Spec = Spec {
    name: "update window",
    title_key: "update.title",
    size: vec2(620.0, 600.0),
    min_size: vec2(480.0, 360.0),
    topmost_once: true,
};

/// 下載中多久重繪一次，進度與速度才會跟著動。
const PROGRESS_REPAINT: Duration = Duration::from_millis(100);

/// 要 App 做的事。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostAction {
    /// 替換執行檔並關閉程式（⑤ 已畫出）。
    Restart,
}

/// 更新流程與它的視窗；所有方法都在 UI 執行緒呼叫。
pub struct UpdateHost {
    job: UpdateJob,
    /// 視窗開著才有。
    view: Option<UpdateView>,
    win: HostedWindow,
    rt: Handle,
    http: reqwest::Client,
    ui: UiPoster,
    /// 剛下載驗證完、還沒做設定檢查；每次下載完成只判斷一次。
    auto_restart_pending: bool,
    /// 進入 Restarting 的那一幀；之後的一幀才交給 App 替換。
    restart_at: Option<u64>,
}

impl UpdateHost {
    /// `http` 下載 exe 用，`exe` 是要被替換的執行檔。
    pub fn new(rt: Handle, http: reqwest::Client, ui: UiPoster, exe: PathBuf) -> Self {
        Self::build(rt, http, ui, UpdateJob::new(exe))
    }

    /// 以現成的 `job`（例如 `UpdateJob::with_phase`）建立，測試用。
    #[cfg(feature = "test-support")]
    pub fn with_job(rt: Handle, http: reqwest::Client, ui: UiPoster, job: UpdateJob) -> Self {
        Self::build(rt, http, ui, job)
    }

    fn build(rt: Handle, http: reqwest::Client, ui: UiPoster, job: UpdateJob) -> Self {
        Self {
            job,
            view: None,
            win: HostedWindow::new(UPDATE),
            rt,
            http,
            ui,
            auto_restart_pending: false,
            restart_at: None,
        }
    }

    /// 換掉視窗操作（測試用：不搶前景、假的工作區）。
    pub fn set_window_ops(&mut self, ops: WindowOps) {
        self.win.ops = ops;
    }

    /// 新到舊的 release 清單（轉給 `UpdateJob`）。
    pub fn set_releases(&mut self, releases: Vec<Release>) {
        self.job.set_releases(releases);
    }

    /// 開窗；已開著就叫到前景、不重設狀態，清單是空的就不開。
    pub fn open(&mut self) {
        if self.job.releases().is_empty() {
            log("[update] update window not opened: no release in the list");
            return;
        }
        if self.view.is_some() {
            log("[update] update window already open; bringing it to the front");
            self.win.bring_to_front();
            return;
        }
        log(format!(
            "[update] update window opened: phase={:?}",
            self.job.phase()
        ));
        self.view = Some(UpdateView::new());
    }

    pub fn is_open(&self) -> bool {
        self.view.is_some()
    }

    /// 視窗的 HWND；沒開或還沒找到是 0。
    pub fn hwnd(&self) -> isize {
        self.win.hwnd
    }

    pub fn phase(&self) -> &Phase {
        self.job.phase()
    }

    /// 要被替換的執行檔。
    pub fn exe(&self) -> &Path {
        self.job.exe()
    }

    /// 下載任務的結果（`UiEvent::UpdateDownloaded`）。
    pub fn downloaded(&mut self, run: u64, result: Result<(), String>) {
        let was_downloading = *self.job.phase() == Phase::Downloading;
        self.job.finish(run, result);
        if was_downloading && *self.job.phase() == Phase::Ready {
            self.auto_restart_pending = true;
        }
    }

    /// App 替換執行檔失敗。
    pub fn restart_failed(&mut self, error: String) {
        self.job.restart_failed(error);
    }

    /// 每幀呼叫：開著就畫出視窗並處理動作；`unsaved` 是設定視窗有沒有尚未存下的變更。
    pub fn show(&mut self, ctx: &egui::Context, known: &[isize], unsaved: bool) -> Vec<HostAction> {
        if self.view.is_none() {
            return Vec::new();
        }
        let frame = ctx.cumulative_frame_nr();
        let mut out = Vec::new();
        if self.restart_at.is_some_and(|at| frame > at) {
            self.restart_at = None;
            out.push(HostAction::Restart);
        }
        if std::mem::take(&mut self.auto_restart_pending) {
            log(format!("[update] ready; settings unsaved={unsaved}"));
            if !unsaved {
                self.begin_restart(ctx);
            }
        }
        let Self { job, view, win, .. } = self;
        let Some(view) = view else {
            return out;
        };
        let actions = win.show(ctx, known, |ui, closing| {
            let mut actions = view.ui(ui, job.releases(), job.phase(), job.progress(), unsaved);
            if closing {
                if can_close(job.phase()) {
                    actions.push(UpdateAction::Close);
                } else {
                    ui.ctx().send_viewport_cmd(ViewportCommand::CancelClose);
                }
            }
            actions
        });
        for action in actions {
            self.act(ctx, action, unsaved);
        }
        if *self.job.phase() == Phase::Downloading {
            ctx.request_repaint_after(PROGRESS_REPAINT);
        }
        out
    }

    fn act(&mut self, ctx: &egui::Context, action: UpdateAction, unsaved: bool) {
        match action {
            UpdateAction::Start => self.start(),
            UpdateAction::Cancel => self.job.cancel(),
            UpdateAction::Retry
                if matches!(self.job.phase(), Phase::Failed { replace: true, .. }) =>
            {
                self.begin_restart(ctx);
            }
            UpdateAction::Retry => self.start(),
            UpdateAction::OpenReleasePage => {
                if let Some(latest) = self.job.releases().first() {
                    log(format!("[update] opening release page: {}", latest.url));
                    ctx.open_url(OpenUrl::new_tab(&latest.url));
                }
            }
            UpdateAction::Close => self.close(),
            UpdateAction::Restart => {
                if unsaved {
                    log("[update] user chose to discard unsaved settings and restart");
                }
                self.begin_restart(ctx);
            }
        }
    }

    fn start(&mut self) {
        let ui = self.ui.clone();
        self.job.start(
            &self.rt,
            self.http.clone(),
            Box::new(move |run, result| ui.post(UiEvent::UpdateDownloaded { run, result })),
        );
    }

    /// 進入 Restarting 並記下這一幀，下一幀（⑤ 已畫出）才交給 App。
    fn begin_restart(&mut self, ctx: &egui::Context) {
        if *self.job.phase() == Phase::Restarting {
            return;
        }
        self.job.begin_restart();
        if *self.job.phase() == Phase::Restarting {
            self.restart_at = Some(ctx.cumulative_frame_nr());
            ctx.request_repaint();
        }
    }

    fn close(&mut self) {
        if !can_close(self.job.phase()) {
            return;
        }
        self.job.window_closed();
        if self.view.take().is_some() {
            log(format!(
                "[update] update window closed: phase={:?}",
                self.job.phase()
            ));
            self.win.reset();
        }
    }
}
