//! 更新下載狀態機：UI 無關，只管 phase、進度與 `.new`／`.new.part` 檔案。

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;

use super::self_update::{new_path, part_path};
use super::updater::{EXE_ASSET, Release, is_newer};
use crate::i18n::t;
use crate::local::download::{Progress, download};
use crate::logs::log;

/// 更新視窗所處的階段（對應設計文件的狀態 ①～⑧）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    Idle,
    /// `progress()` 的 total > 0 且 done == total 時 UI 顯示「驗證中」。
    Downloading,
    /// 驗證通過、`.new` 在磁碟上。
    Ready,
    Restarting,
    /// `replace` 為替換那步失敗：重試不重新下載。
    Failed {
        error: String,
        replace: bool,
    },
    Cancelled,
}

/// 下載任務結束時的回呼：帶上該次下載的 run id 與結果。
pub type DoneCallback = Box<dyn FnOnce(u64, Result<(), String>) + Send>;

/// 舊下載還沒結束時被擋下的 `start`，等舊任務的 `finish` 進來再跑。
struct QueuedStart {
    rt: Handle,
    http: reqwest::Client,
    done: DoneCallback,
}

/// 一次更新流程的狀態：release 清單、階段、下載進度與取消。
pub struct UpdateJob {
    releases: Vec<Release>,
    pending: Option<Vec<Release>>,
    phase: Phase,
    progress: Arc<Progress>,
    cancel: Option<CancellationToken>,
    exe: PathBuf,
    /// 下載中或已下載的版本。
    target: Option<String>,
    run: u64,
    /// 目前那次下載的任務還沒結束（保證 `.new.part` 只有一個寫入者）。
    running: bool,
    queued: Option<QueuedStart>,
}

impl UpdateJob {
    pub fn new(exe: PathBuf) -> Self {
        Self {
            releases: Vec::new(),
            pending: None,
            phase: Phase::Idle,
            progress: Arc::new(Progress::default()),
            cancel: None,
            exe,
            target: None,
            run: 0,
            running: false,
            queued: None,
        }
    }

    /// 直接進到指定 phase 的建構子，測試用。
    #[cfg(feature = "test-support")]
    pub fn with_phase(exe: PathBuf, releases: Vec<Release>, phase: Phase) -> Self {
        let target = releases.first().map(|r| r.version.clone());
        Self {
            releases,
            target,
            phase,
            ..Self::new(exe)
        }
    }

    /// 新到舊的 release 清單。
    pub fn releases(&self) -> &[Release] {
        &self.releases
    }

    /// 要被替換的執行檔。
    pub fn exe(&self) -> &Path {
        &self.exe
    }

    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    /// `(已下載, 總計)` 位元組。
    pub fn progress(&self) -> (u64, u64) {
        (
            self.progress.done.load(Ordering::Relaxed),
            self.progress.total.load(Ordering::Relaxed),
        )
    }

    /// 換上新清單；下載中先暫存到 `finish`，已下載的 `.new` 比最新版舊就刪掉回 Idle。
    pub fn set_releases(&mut self, releases: Vec<Release>) {
        if self.phase == Phase::Downloading {
            self.pending = Some(releases);
            return;
        }
        self.releases = releases;
        let stale = matches!(
            self.phase,
            Phase::Ready | Phase::Failed { replace: true, .. }
        ) && match (self.releases.first(), &self.target) {
            (Some(latest), Some(target)) => is_newer(&latest.version, target),
            _ => false,
        };
        if stale {
            log(format!(
                "[update] discarding stale download: downloaded={:?} latest={:?}",
                self.target,
                self.releases.first().map(|r| &r.version)
            ));
            self.remove(&new_path(&self.exe));
            self.phase = Phase::Idle;
        }
    }

    /// 開始下載最新版；沒有可驗證的 exe 直接進 Failed。舊下載的任務還沒結束時排隊，等它的 `finish` 再開始。
    pub fn start(&mut self, rt: &Handle, http: reqwest::Client, done: DoneCallback) {
        if !matches!(
            self.phase,
            Phase::Idle | Phase::Cancelled | Phase::Failed { replace: false, .. }
        ) {
            return;
        }
        let Some(release) = self.releases.first() else {
            log("[update] start ignored: no release in the list");
            return;
        };
        if release.asset.is_none() {
            return self.fail_no_asset();
        }
        self.target = Some(release.version.clone());
        self.phase = Phase::Downloading;
        let job = QueuedStart {
            rt: rt.clone(),
            http,
            done,
        };
        if self.running {
            log("[update] previous download still running; start queued");
            self.queued = Some(job);
        } else {
            self.spawn(job);
        }
    }

    fn spawn(&mut self, job: QueuedStart) {
        let Some(asset) = self.releases.first().and_then(|r| r.asset.clone()) else {
            return self.fail_no_asset();
        };
        log(format!(
            "[update] download start: version={:?} size={}",
            self.target, asset.size
        ));
        self.run += 1;
        self.running = true;
        self.progress = Arc::new(Progress::default());
        let cancel = CancellationToken::new();
        self.cancel = Some(cancel.clone());
        let (run, progress, dest) = (self.run, self.progress.clone(), new_path(&self.exe));
        let QueuedStart { rt, http, done } = job;
        rt.spawn(async move {
            let result = download(
                &http,
                &asset.url,
                &dest,
                asset.size,
                Some(&asset.sha256),
                &progress,
                &cancel,
            )
            .await
            .map_err(|e| e.to_string());
            done(run, result);
        });
    }

    /// 下載任務的結果；已取消或視窗已關時不改 phase，只刪 `.new.part` 與 `.new`。
    pub fn finish(&mut self, run: u64, result: Result<(), String>) {
        if run != self.run || !self.running {
            log(format!("[update] ignoring result of superseded run {run}"));
            return;
        }
        self.running = false;
        self.cancel = None;
        if let Some(job) = self.queued.take() {
            self.remove(&part_path(&self.exe));
            self.remove(&new_path(&self.exe));
            return self.spawn(job);
        }
        if self.phase == Phase::Downloading {
            match result {
                Ok(()) => {
                    log(format!(
                        "[update] download verified: version={:?}",
                        self.target
                    ));
                    self.phase = Phase::Ready;
                }
                Err(error) => self.fail(error),
            }
        } else {
            log("[update] download cancelled");
            self.remove(&part_path(&self.exe));
            self.remove(&new_path(&self.exe));
        }
        if let Some(releases) = self.pending.take() {
            self.set_releases(releases);
        }
    }

    /// 取消下載；`.new.part` 等任務結束、`finish` 進來時才刪（任務可能還開著那個檔）。
    pub fn cancel(&mut self) {
        if self.phase != Phase::Downloading {
            return;
        }
        log("[update] cancel requested");
        self.queued = None;
        if let Some(cancel) = &self.cancel {
            cancel.cancel();
        }
        self.phase = Phase::Cancelled;
    }

    /// 視窗被關：沒下載成功的 Failed／Cancelled 刪 `.new.part` 回 Idle，Ready 保留 `.new`。
    pub fn window_closed(&mut self) {
        if matches!(
            self.phase,
            Phase::Cancelled | Phase::Failed { replace: false, .. }
        ) {
            self.remove(&part_path(&self.exe));
            self.phase = Phase::Idle;
        }
    }

    /// Ready（或替換失敗後重試）進入 Restarting。
    pub fn begin_restart(&mut self) {
        if matches!(
            self.phase,
            Phase::Ready | Phase::Failed { replace: true, .. }
        ) {
            log("[update] restarting to apply the update");
            self.phase = Phase::Restarting;
        }
    }

    /// 替換執行檔失敗。
    pub fn restart_failed(&mut self, error: String) {
        log(format!("[update] replace failed: {error}"));
        self.phase = Phase::Failed {
            error,
            replace: true,
        };
    }

    fn fail_no_asset(&mut self) {
        let version = self.releases.first().map_or("", |r| r.version.as_str());
        let error = t(
            "update.no_verifiable_asset",
            &[("version", version), ("asset", EXE_ASSET)],
        );
        log(format!(
            "[update] release {version} has no verifiable {EXE_ASSET} asset"
        ));
        self.fail(error);
    }

    fn fail(&mut self, error: String) {
        log(format!("[update] download failed: {error}"));
        self.phase = Phase::Failed {
            error,
            replace: false,
        };
    }

    fn remove(&self, file: &Path) {
        match std::fs::remove_file(file) {
            Ok(()) => log(format!("[update] removed {}", file.display())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => log(format!("[update] cannot remove {}: {e}", file.display())),
        }
    }
}
