//! 本地模型的對外入口：狀態快照，以及在背景執行的引擎、硬體、repo 查詢與下載指令。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use tokio::runtime::Handle;
use tokio::sync::OnceCell;
use tokio_util::sync::CancellationToken;

use super::download::Progress;
use super::engine::{self, Build, EngineInstall, Variant};
use super::fit::{self, Fit, Pick};
use super::gguf::{self, ModelMeta};
use super::hardware::{self, Gpu, Hardware};
use super::hf::{self, Catalog};
use super::runtime::{EngineUsers, Servers};
use super::server::Launcher;
use super::store::{self, Installed};
use super::{LocalNotice, Notify};
use crate::config::Config;
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
        /// 正在解壓剛下載完的 zip。
        extracting: bool,
    },
    Ready(EngineInstall),
    /// 正在停掉子程序並刪除引擎檔案。
    Removing,
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
    /// 目前跑著 llama-server 的模型 id。
    pub running: BTreeSet<String>,
    /// 硬體偵測中（啟動後第一次量完以前，或〔重新偵測〕進行中）；這時硬體與相容等級還不能信。
    pub detecting: bool,
}

impl Snapshot {
    /// 已下載完成的模型 `id`。
    pub fn ready_model(&self, id: &str) -> Option<&Installed> {
        self.models.iter().find(|m| m.id == id && m.complete)
    }
}

/// `Local::detecting` 登記的一筆硬體偵測。
struct Detecting<'a> {
    local: &'a Local,
}

impl Drop for Detecting<'_> {
    fn drop(&mut self) {
        self.local.update(|_| {
            self.local.refreshing.fetch_sub(1, Ordering::SeqCst);
        });
    }
}

/// 啟動後第一次量測：放掉時扣掉建立時就算進去的那一筆（只扣一次）。
struct FirstMeasure<'a> {
    local: &'a Local,
}

impl Drop for FirstMeasure<'_> {
    fn drop(&mut self) {
        if self.local.first_pending.swap(false, Ordering::SeqCst) {
            self.local.update(|_| {
                self.local.refreshing.fetch_sub(1, Ordering::SeqCst);
            });
        }
    }
}

/// UI 送給 hub 的指令，全部在背景執行。
#[derive(Clone, Debug)]
pub enum Command {
    /// 重新量硬體；`redetect_gpu` 時也重新列舉顯示卡（〔重新偵測〕），否則只重量可用量（例如打開本地模型頁）。
    RefreshHardware {
        redetect_gpu: bool,
    },
    ListBuilds,
    /// `build`／`variant` 為 `None` 時取最新 build／依硬體自動挑。
    InstallEngine {
        build: Option<String>,
        variant: Option<Variant>,
    },
    RemoveEngine,
    QueryRepo(String),
    /// `choice`／`mtp` 為 `RepoInfo.catalog` 內的索引；`auto` 表示來自自動挑選（使用者沒手動選量化）。
    Download {
        repo: String,
        choice: usize,
        mtp: Option<usize>,
        auto: bool,
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
    /// 偵測過的 GPU（外層 `None` 是還沒偵測）；只有使用者按〔重新偵測〕才重新列舉。
    gpu: Mutex<Option<Option<Gpu>>>,
    /// 程式啟動後的第一次量硬體；預先載入等它量完才開始。
    first_measure: OnceCell<()>,
    /// 進行中的硬體偵測數；啟動後第一次量測從建立起就算一筆，量完（或中途失敗）才扣掉。
    refreshing: AtomicUsize,
    /// 第一次量測那一筆還沒扣掉。
    first_pending: AtomicBool,
    /// 載入中的模型顯示名（依開始順序）。
    pub(super) loading: Mutex<Vec<String>>,
    /// `shutdown` 之後不再啟動任何模型。
    pub(super) closed: AtomicBool,
    /// 引擎安裝與移除一次只做一件；等引擎的下載也排在這裡。
    engine_lock: tokio::sync::Mutex<()>,
    /// 每次 `ensure` 從檢查前置條件到啟動完成都持有讀鎖；移除引擎時取寫鎖，期間任何模型都啟動不了。
    pub(super) engine_files: tokio::sync::RwLock<()>,
    pub(super) engine_users: EngineUsers,
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
            gpu: Mutex::default(),
            first_measure: OnceCell::new(),
            refreshing: AtomicUsize::new(1),
            first_pending: AtomicBool::new(true),
            loading: Mutex::default(),
            closed: AtomicBool::new(false),
            engine_lock: tokio::sync::Mutex::new(()),
            engine_files: tokio::sync::RwLock::new(()),
            engine_users: EngineUsers::default(),
            cancels: Mutex::default(),
            me: me.clone(),
        })
    }

    pub fn snapshot(&self) -> Snapshot {
        let mut snapshot = {
            let state = lock(&self.state);
            // 在狀態鎖內讀：計數只在 `update` 裡變，偵測中與硬體值是同一刻的
            let mut snapshot = state.clone();
            snapshot.detecting = self.refreshing.load(Ordering::SeqCst) > 0;
            snapshot
        };
        snapshot.running = self.running_ids();
        snapshot
    }

    /// 程式啟動時呼叫一次：背景量硬體並套用設定；預先載入等量完才開始，量到的可用量才不含自己的模型。
    pub fn start(&self, cfg: &Config) {
        let me = self.arc();
        self.rt.spawn(async move { me.measured_once().await });
        self.apply_config(cfg);
    }

    /// 等程式啟動後的第一次量硬體完成（還沒開始就由這裡量）。
    pub(super) async fn measured_once(&self) {
        self.first_measure
            .get_or_init(|| async {
                let _first = FirstMeasure { local: self };
                self.refresh_hardware(false).await;
            })
            .await;
    }

    /// 最近一次 `apply_config` 收到的設定（之前為預設值）。
    pub fn settings(&self) -> LocalSettings {
        lock(&self.settings).clone()
    }

    /// 在背景執行指令；狀態變了就送 `LocalNotice::Changed`。
    ///
    /// 下載與繼續下載在回傳前就登記好，快照立刻是下載中、蓋掉舊的失敗紀錄（repo 還沒查過的下載除外）。
    pub fn command(&self, cmd: Command) {
        log(format!("[local] command {cmd:?}"));
        let me = self.arc();
        let task: std::pin::Pin<Box<dyn Future<Output = ()> + Send>> = match cmd {
            Command::RefreshHardware { redetect_gpu } => {
                Box::pin(async move { me.refresh_hardware(redetect_gpu).await })
            }
            Command::ListBuilds => Box::pin(async move { drop(me.list_builds().await) }),
            Command::InstallEngine { build, variant } => Box::pin(async move {
                let _engine = me.engine_lock.lock().await;
                let _ = me.install_engine(build, variant).await;
            }),
            Command::RemoveEngine => Box::pin(async move { me.remove_engine().await }),
            Command::QueryRepo(repo) => Box::pin(async move { me.query_repo(&repo).await }),
            Command::Download {
                repo,
                choice,
                mtp,
                auto,
            } => {
                let claimed = match self.repo_info(&repo) {
                    Some(_) => match self.claim_download(&repo, choice, mtp) {
                        Some(claimed) => Some(claimed),
                        None => return,
                    },
                    None => None,
                };
                Box::pin(async move {
                    let claimed = match claimed {
                        Some(claimed) => Some(claimed),
                        None => {
                            me.query_repo(&repo).await;
                            me.claim_download(&repo, choice, mtp)
                        }
                    };
                    if let Some(claimed) = claimed {
                        me.download(&repo, auto, claimed).await;
                    }
                })
            }
            Command::Resume(id) => {
                let Some(cancel) = self.claim(&id) else {
                    log(format!("[local] download {id} already running"));
                    return;
                };
                Box::pin(async move { me.resume(&id, cancel).await })
            }
            Command::CancelDownload(id) => {
                if let Some(cancel) = lock(&self.cancels).get(&id) {
                    cancel.cancel();
                }
                return;
            }
            Command::Delete(id) => Box::pin(async move { me.delete(&id).await }),
        };
        self.rt.spawn(task);
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

    /// 偵測 GPU；偵測過且不要求 `redetect` 時沿用上次的結果。
    fn gpu(&self, redetect: bool) -> Option<Gpu> {
        let mut cached = lock(&self.gpu);
        if redetect || cached.is_none() {
            *cached = Some(tokio::task::block_in_place(|| self.launcher.detect_gpu()));
        }
        cached.clone().flatten()
    }

    /// 量 GPU（`redetect_gpu` 時重新列舉）、VRAM 與 RAM，以及本程式跑著的 llama-server 實際佔用多少。
    async fn refresh_hardware(&self, redetect_gpu: bool) {
        let _detecting = self.detecting();
        let gpu = self.gpu(redetect_gpu);
        // 持讀鎖跑 `--list-devices`：移除引擎要等它結束，才不會刪到執行中的 exe
        let files = self.engine_files.read().await;
        let pids = self.running_pids();
        let exe = current_exe(&self.root);
        let mut hw = tokio::task::block_in_place(|| {
            let (ram_total, ram_free) = hardware::system_memory();
            let mut hw = Hardware {
                gpu,
                ram_total,
                ram_free,
                ..Hardware::default()
            };
            let Some(exe) = exe else {
                return hw;
            };
            match self.launcher.list_devices(&exe) {
                Ok(out) => {
                    // 有獨顯又有內顯時兩個都會列出，取記憶體最大的那個
                    let devices = hardware::parse_list_devices(&out);
                    if let Some((_, total, free)) = devices.into_iter().max_by_key(|d| d.1) {
                        (hw.vram_total, hw.vram_free) = (total, free);
                    }
                    hw.measured = true;
                }
                Err(e) => log(format!("[local] llama-server --list-devices failed: {e}")),
            }
            hw
        });
        drop(files);
        hw.own = tokio::task::block_in_place(|| self.launcher.process_usage(&pids));
        log(format!(
            "[local] hardware: gpu={:?} vram_total={} vram_free={} ram_total={} ram_free={} vram_measured={} own_pids={pids:?} own_vram={} own_ram={}",
            hw.gpu,
            hw.vram_total,
            hw.vram_free,
            hw.ram_total,
            hw.ram_free,
            hw.measured,
            hw.own.vram,
            hw.own.ram
        ));
        self.update(|s| {
            for repo in s.repos.values_mut() {
                if let RepoState::Ready(info) = repo {
                    rate(info, &hw);
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
        let variant =
            variant.unwrap_or_else(|| engine::auto_variant(self.gpu(false).as_ref(), Some(build)));
        let total = build
            .variants
            .iter()
            .find(|(v, _)| *v == variant)
            .map_or(0, |(_, assets)| assets.iter().map(|a| a.size).sum());
        let progress = Progress::default();
        let installing = |done, total| EngineState::Installing {
            build: build.tag.clone(),
            done,
            total,
            extracting: progress.extracting.load(Ordering::Relaxed),
        };
        self.update(|s| s.engine = installing(0, total));
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
        // 先登記偵測中再設 Ready，UI 才不會在量測開始前看到「Ready 但沒量過」
        let detecting = self.detecting();
        self.update(|s| s.engine = EngineState::Ready(installed));
        // 先量 VRAM 再重啟子程序，量到的才是模型還沒載入時的可用量
        self.refresh_hardware(false).await;
        drop(detecting);
        self.restart_servers();
        Ok(())
    }

    /// 登記一筆硬體偵測；放掉時（含 panic）扣掉並通知 UI。
    fn detecting(&self) -> Detecting<'_> {
        self.update(|_| {
            self.refreshing.fetch_add(1, Ordering::SeqCst);
        });
        Detecting { local: self }
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
        self.update(|s| s.engine = EngineState::Removing);
        self.stop_all();
        let _files = self.engine_files.write().await;
        // 取寫鎖前剛起跑的啟動不在第一次 stop_all 之內，拿到鎖後再停一次
        self.stop_all();
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
        self.update(|s| {
            if let RepoState::Ready(info) = &mut state {
                rate(info, &s.hardware);
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

/// 依硬體重算各量化的所需量、等級與自動挑選；VRAM 沒量過時等級為 `None`。
fn rate(info: &mut RepoInfo, hw: &Hardware) {
    let moe = info.meta.expert_count > 0;
    info.fits = info
        .catalog
        .choices
        .iter()
        .map(|c| {
            let required = fit::required(&info.meta, fit::total_size(c), 0);
            (required, hw.measured.then(|| fit::fit(required, moe, hw)))
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
