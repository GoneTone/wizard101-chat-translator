//! 本地模型的對外入口：狀態快照，以及在背景執行的引擎、硬體、repo 查詢與下載指令。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;

use super::download::Progress;
use super::engine::{self, Build, EngineInstall, Variant};
use super::fit::{self, Fit, Pick};
use super::gguf::{self, ModelMeta};
use super::hardware::{self, Hardware};
use super::hf::{self, Catalog};
use super::runtime::Servers;
use super::server::Launcher;
use super::store::{self, Installed};
use super::{LocalNotice, Notify};
use crate::config::local::LocalSettings;
use crate::logs::log;
use crate::util::lock;

pub(super) const PROGRESS_TICK: Duration = Duration::from_millis(250);

/// 外部服務根網址；正式為 [`hf::HF_BASE`] 與 [`engine::RELEASES_API`]。
#[derive(Clone, Debug)]
pub struct Endpoints {
    pub hf: String,
    pub releases: String,
}

/// llama.cpp 引擎的狀態。
#[derive(Clone, Debug, Default, PartialEq)]
pub enum EngineState {
    #[default]
    Missing,
    Installing {
        build: String,
        done: u64,
        total: u64,
    },
    Ready(EngineInstall),
    Failed(String),
}

/// 一個模型下載的狀態；下載完成後從快照移除。
#[derive(Clone, Debug, PartialEq)]
pub enum DownloadState {
    Running { done: u64, total: u64 },
    Paused { done: u64, total: u64 },
    Failed(String),
}

/// 一個 repo 的查詢狀態。
#[derive(Clone, Debug, PartialEq)]
pub enum RepoState {
    Loading,
    Ready(RepoInfo),
    Failed(String),
}

/// 一個 repo 的檔案分類、檔頭與相容判斷。
#[derive(Clone, Debug, PartialEq)]
pub struct RepoInfo {
    pub catalog: Catalog,
    pub meta: ModelMeta,
    /// 與 `catalog.choices` 對應的（所需位元組, 等級）；VRAM 還沒量到時等級為 `None`。
    pub fits: Vec<(u64, Option<Fit>)>,
    /// 與 `catalog.mtp` 對應：加上該 MTP 檔多出的所需位元組。
    pub mtp_required: Vec<u64>,
    pub auto: Option<Pick>,
}

/// UI 每幀讀取的狀態快照。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub engine: EngineState,
    pub builds: Option<Result<Vec<Build>, String>>,
    pub hardware: Hardware,
    pub models: Vec<Installed>,
    pub downloads: BTreeMap<String, DownloadState>,
    pub repos: BTreeMap<String, RepoState>,
}

/// UI 送給 hub 的指令，全部在背景執行。
#[derive(Clone, Debug)]
pub enum Command {
    RefreshHardware,
    ListBuilds,
    /// `build`／`variant` 為 `None` 時取最新 build／依硬體自動挑。
    InstallEngine {
        build: Option<String>,
        variant: Option<Variant>,
    },
    RemoveEngine,
    QueryRepo(String),
    /// `choice`／`mtp` 為 `RepoInfo.catalog` 內的索引。
    Download {
        repo: String,
        choice: usize,
        mtp: Option<usize>,
    },
    Resume(String),
    CancelDownload(String),
    Delete(String),
}

/// 本地模型 hub：持有快照、下載與 llama-server 子程序。
pub struct Local {
    pub(super) root: PathBuf,
    pub(super) rt: Handle,
    pub(super) launcher: Arc<dyn Launcher>,
    pub(super) notify: Notify,
    pub(super) settings: Mutex<LocalSettings>,
    pub(super) servers: Servers,
    pub(super) endpoints: Endpoints,
    pub(super) hf_http: reqwest::Client,
    gh_http: reqwest::Client,
    pub(super) state: Mutex<Snapshot>,
    pub(super) vram_measured: AtomicBool,
    /// 引擎安裝與移除一次只做一件；等引擎的下載也排在這裡。
    engine_lock: tokio::sync::Mutex<()>,
    pub(super) cancels: Mutex<BTreeMap<String, CancellationToken>>,
    me: Weak<Local>,
}

impl Local {
    /// 以 `root` 為資料夾建立 hub；引擎與模型清單從磁碟讀出。
    pub fn new(
        root: PathBuf,
        rt: Handle,
        launcher: Arc<dyn Launcher>,
        endpoints: Endpoints,
        notify: Notify,
    ) -> Arc<Self> {
        let snapshot = Snapshot {
            engine: engine_on_disk(&root),
            models: store::list(&root),
            ..Snapshot::default()
        };
        log(format!(
            "[local] hub created: engine={:?} models={}",
            snapshot.engine,
            snapshot.models.len()
        ));
        Arc::new_cyclic(|me| Self {
            hf_http: hf::hf_client(&endpoints.hf),
            gh_http: hf::hf_client(&endpoints.releases),
            root,
            rt,
            launcher,
            notify,
            settings: Mutex::default(),
            servers: Servers::default(),
            endpoints,
            state: Mutex::new(snapshot),
            vram_measured: AtomicBool::new(false),
            engine_lock: tokio::sync::Mutex::new(()),
            cancels: Mutex::default(),
            me: me.clone(),
        })
    }

    pub fn snapshot(&self) -> Snapshot {
        lock(&self.state).clone()
    }

    /// 最近一次 `apply_config` 收到的設定（之前為預設值）。
    pub fn settings(&self) -> LocalSettings {
        lock(&self.settings).clone()
    }

    /// 在背景執行指令；狀態變了就送 `LocalNotice::Changed`。
    pub fn command(&self, cmd: Command) {
        log(format!("[local] command {cmd:?}"));
        let me = self.arc();
        self.rt.spawn(async move {
            match cmd {
                Command::RefreshHardware => me.refresh_hardware().await,
                Command::ListBuilds => drop(me.list_builds().await),
                Command::InstallEngine { build, variant } => {
                    let _engine = me.engine_lock.lock().await;
                    let _ = me.install_engine(build, variant).await;
                }
                Command::RemoveEngine => me.remove_engine().await,
                Command::QueryRepo(repo) => me.query_repo(&repo).await,
                Command::Download { repo, choice, mtp } => me.download(&repo, choice, mtp).await,
                Command::Resume(id) => me.resume(&id).await,
                Command::CancelDownload(id) => {
                    if let Some(cancel) = lock(&me.cancels).get(&id) {
                        cancel.cancel();
                    }
                }
                Command::Delete(id) => me.delete(&id).await,
            }
        });
    }

    pub(super) fn arc(&self) -> Arc<Self> {
        self.me.upgrade().expect("Local is alive while in use")
    }

    pub(super) fn update(&self, f: impl FnOnce(&mut Snapshot)) {
        f(&mut lock(&self.state));
        (self.notify)(LocalNotice::Changed);
    }

    /// 執行 `fut`，期間定時把 `progress` 寫進快照。
    pub(super) async fn track<T>(
        &self,
        progress: &Progress,
        fut: impl Future<Output = T>,
        set: impl Fn(&mut Snapshot, u64, u64),
    ) -> T {
        tokio::pin!(fut);
        let mut tick = tokio::time::interval(PROGRESS_TICK);
        loop {
            tokio::select! {
                out = &mut fut => return out,
                _ = tick.tick() => self.update(|s| set(
                    s,
                    progress.done.load(Ordering::Relaxed),
                    progress.total.load(Ordering::Relaxed),
                )),
            }
        }
    }

    async fn refresh_hardware(&self) {
        let exe = current_exe(&self.root);
        let (hw, measured) = tokio::task::block_in_place(|| {
            let (ram_total, ram_free) = hardware::system_memory();
            let mut hw = Hardware {
                gpu: self.launcher.detect_gpu(),
                ram_total,
                ram_free,
                ..Hardware::default()
            };
            let Some(exe) = exe else {
                return (hw, false);
            };
            match self.launcher.list_devices(&exe) {
                Ok(out) => {
                    // 有獨顯又有內顯時兩個都會列出，取記憶體最大的那個
                    let devices = hardware::parse_list_devices(&out);
                    if let Some((_, total, free)) = devices.into_iter().max_by_key(|d| d.1) {
                        (hw.vram_total, hw.vram_free) = (total, free);
                    }
                    (hw, true)
                }
                Err(e) => {
                    log(format!("[local] llama-server --list-devices failed: {e}"));
                    (hw, false)
                }
            }
        });
        log(format!(
            "[local] hardware: gpu={:?} vram_total={} vram_free={} ram_total={} ram_free={} vram_measured={measured}",
            hw.gpu, hw.vram_total, hw.vram_free, hw.ram_total, hw.ram_free
        ));
        self.vram_measured.store(measured, Ordering::Relaxed);
        self.update(|s| {
            for repo in s.repos.values_mut() {
                if let RepoState::Ready(info) = repo {
                    rate(info, &hw, measured);
                }
            }
            s.hardware = hw;
        });
    }

    /// 呼叫端須持有 `engine_lock`；失敗時引擎狀態改為 `Failed`。
    async fn install_engine(
        &self,
        tag: Option<String>,
        variant: Option<Variant>,
    ) -> Result<(), String> {
        let result = self.try_install_engine(tag, variant).await;
        if let Err(e) = &result {
            log(format!("[local] engine install failed: {e}"));
            self.update(|s| s.engine = EngineState::Failed(e.clone()));
        }
        result
    }

    async fn try_install_engine(
        &self,
        tag: Option<String>,
        variant: Option<Variant>,
    ) -> Result<(), String> {
        let cached = match &lock(&self.state).builds {
            Some(Ok(builds)) => Some(builds.clone()),
            _ => None,
        };
        let builds = match cached {
            Some(builds) => builds,
            None => self.list_builds().await?,
        };
        let build = match &tag {
            None => builds.first(),
            Some(t) => builds.iter().find(|b| &b.tag == t),
        }
        .ok_or_else(|| {
            format!(
                "llama.cpp build {} is not available",
                tag.as_deref().unwrap_or("latest")
            )
        })?;
        let variant = variant.unwrap_or_else(|| {
            engine::auto_variant(
                tokio::task::block_in_place(|| self.launcher.detect_gpu()).as_ref(),
            )
        });
        let total = build
            .variants
            .iter()
            .find(|(v, _)| *v == variant)
            .map_or(0, |(_, assets)| assets.iter().map(|a| a.size).sum());
        let installing = |done, total| EngineState::Installing {
            build: build.tag.clone(),
            done,
            total,
        };
        self.update(|s| s.engine = installing(0, total));
        let progress = Progress::default();
        let never = CancellationToken::new();
        let install = engine::install(
            &self.gh_http,
            &self.root,
            build,
            &variant,
            &progress,
            &never,
        );
        let installed = self
            .track(&progress, install, |s, done, total| {
                s.engine = installing(done, total)
            })
            .await?;
        engine::activate(&self.root, installed.clone()).map_err(|e| {
            log(format!("[local] activating engine failed: {e}"));
            e.to_string()
        })?;
        self.update(|s| s.engine = EngineState::Ready(installed));
        // 先量 VRAM 再重啟子程序，量到的才是模型還沒載入時的可用量
        self.refresh_hardware().await;
        self.restart_servers();
        Ok(())
    }

    /// 引擎沒就緒就安裝（有安裝進行中則等它）；回傳這次是否新裝了引擎。
    pub(super) async fn ensure_engine(&self) -> Result<bool, String> {
        let _engine = self.engine_lock.lock().await;
        if matches!(lock(&self.state).engine, EngineState::Ready(_)) {
            return Ok(false);
        }
        log("[local] installing the engine before downloading a model");
        self.install_engine(None, None).await.map(|()| true)
    }

    async fn list_builds(&self) -> Result<Vec<Build>, String> {
        let listed = engine::list_builds(&self.gh_http, &self.endpoints.releases).await;
        if let Err(e) = &listed {
            log(format!("[local] listing llama.cpp builds failed: {e}"));
        }
        self.update(|s| s.builds = Some(listed.clone()));
        listed
    }

    async fn remove_engine(&self) {
        let _engine = self.engine_lock.lock().await;
        let _holds = self.shutdown_and_wait().await;
        let state = match engine::remove_all(&self.root) {
            Ok(()) => EngineState::Missing,
            Err(e) => {
                log(format!("[local] removing engines failed: {e}"));
                EngineState::Failed(e.to_string())
            }
        };
        self.update(|s| s.engine = state);
    }

    pub(super) async fn query_repo(&self, repo: &str) {
        self.update(|s| {
            s.repos.insert(repo.to_owned(), RepoState::Loading);
        });
        let mut state = match self.fetch_repo(repo).await {
            Ok((catalog, meta)) => RepoState::Ready(RepoInfo {
                catalog,
                meta,
                fits: vec![],
                mtp_required: vec![],
                auto: None,
            }),
            Err(e) => RepoState::Failed(e),
        };
        let measured = self.vram_measured.load(Ordering::Relaxed);
        self.update(|s| {
            if let RepoState::Ready(info) = &mut state {
                rate(info, &s.hardware, measured);
                log(format!(
                    "[local] {repo}: {} choices, fits={:?} auto={:?}",
                    info.catalog.choices.len(),
                    info.fits,
                    info.auto
                ));
            }
            s.repos.insert(repo.to_owned(), state);
        });
    }

    /// 列出 repo 檔案並讀第一個主模型檔的檔頭（同 repo 的量化共用模型形狀）。
    async fn fetch_repo(&self, repo: &str) -> Result<(Catalog, ModelMeta), String> {
        let files = hf::list_files(&self.hf_http, &self.endpoints.hf, repo).await?;
        let catalog = hf::classify(&files);
        let first = catalog
            .choices
            .first()
            .and_then(|c| c.files.first())
            .ok_or_else(|| format!("{repo} has no GGUF model files"))?;
        let url = hf::resolve_url(&self.endpoints.hf, repo, &first.path);
        let meta = gguf::fetch(&self.hf_http, &url).await?;
        Ok((catalog, meta))
    }

    pub(super) fn repo_info(&self, repo: &str) -> Option<RepoInfo> {
        match lock(&self.state).repos.get(repo) {
            Some(RepoState::Ready(info)) => Some(info.clone()),
            _ => None,
        }
    }
}

/// 依硬體重算各量化的所需量、等級與自動挑選。
fn rate(info: &mut RepoInfo, hw: &Hardware, measured: bool) {
    let moe = info.meta.expert_count > 0;
    info.fits = info
        .catalog
        .choices
        .iter()
        .map(|c| {
            let required = fit::required(&info.meta, fit::total_size(c), 0);
            (required, measured.then(|| fit::fit(required, moe, hw)))
        })
        .collect();
    info.mtp_required = info.catalog.mtp.iter().map(|m| m.file.size).collect();
    info.auto = fit::auto_pick(&info.catalog, &info.meta, hw);
}

fn engine_on_disk(root: &Path) -> EngineState {
    match engine::read_active(root).current {
        Some(i) if engine::server_exe(&engine::engine_dir(root, &i)).is_some() => {
            EngineState::Ready(i)
        }
        _ => EngineState::Missing,
    }
}

/// 目前啟用引擎的 `llama-server.exe`。
fn current_exe(root: &Path) -> Option<PathBuf> {
    let current = engine::read_active(root).current?;
    engine::server_exe(&engine::engine_dir(root, &current))
}
