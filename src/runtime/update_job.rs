//! 更新下載狀態機：UI 無關，只管 phase、進度與 `.new`／`.new.part` 檔案。

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;

use super::self_update::{new_path, part_path};
use super::updater::{EXE_ASSET, Release, is_newer};
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

    /// 開始下載最新版；沒有可驗證的 exe 直接進 Failed。`done` 在下載任務結束時被呼叫（任何執行緒）。
    pub fn start(
        &mut self,
        rt: &Handle,
        http: reqwest::Client,
        done: Box<dyn FnOnce(Result<(), String>) + Send>,
    ) {
        if !matches!(
            self.phase,
            Phase::Idle | Phase::Cancelled | Phase::Failed { replace: false, .. }
        ) {
            return;
        }
        let Some(release) = self.releases.first() else {
            return self.fail("no release to download".to_owned());
        };
        let Some(asset) = release.asset.clone() else {
            let error = format!(
                "release {} has no verifiable {EXE_ASSET} asset",
                release.version
            );
            return self.fail(error);
        };
        log(format!(
            "[update] download start: version={} size={}",
            release.version, asset.size
        ));
        self.target = Some(release.version.clone());
        self.progress = Arc::new(Progress::default());
        let cancel = CancellationToken::new();
        self.cancel = Some(cancel.clone());
        self.phase = Phase::Downloading;

        let progress = self.progress.clone();
        let dest = new_path(&self.exe);
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
            done(result);
        });
    }

    /// 下載任務的結果；已取消時不改 phase，只刪 `.new.part`。
    pub fn finish(&mut self, result: Result<(), String>) {
        self.cancel = None;
        if self.phase == Phase::Cancelled {
            log("[update] download cancelled");
            self.remove(&part_path(&self.exe));
        } else {
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
