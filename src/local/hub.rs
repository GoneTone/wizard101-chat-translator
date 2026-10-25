//! 本地模型的對外入口：狀態快照，以及在背景執行的引擎、硬體、repo 查詢與下載指令。

use std::collections::BTreeMap;
use std::io::Read;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;

use super::download::{self, DownloadError, Progress};
use super::engine::{self, Build, EngineInstall, Variant};
use super::fit::{self, Fit, Pick};
use super::gguf::{self, ModelMeta};
use super::hardware::{self, Hardware};
use super::hf::{self, Catalog};
use super::runtime::Servers;
use super::server::{CREATE_NO_WINDOW, Launcher};
use super::store::{self, Installed, Manifest, ManifestFile};
use super::{LocalNotice, Notify};
use crate::config::local::LocalSettings;
use crate::logs::log;
use crate::util::{Clock, lock, poll_until};

const PROGRESS_TICK: Duration = Duration::from_millis(250);
const LIST_DEVICES_TIMEOUT: Duration = Duration::from_secs(15);

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
    endpoints: Endpoints,
    hf_http: reqwest::Client,
    gh_http: reqwest::Client,
    state: Mutex<Snapshot>,
    vram_measured: AtomicBool,
    /// 引擎安裝與移除一次只做一件；等引擎的下載也排在這裡。
    engine_lock: tokio::sync::Mutex<()>,
    cancels: Mutex<BTreeMap<String, CancellationToken>>,
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
                Command::ListBuilds => {
                    let builds = engine::list_builds(&me.gh_http, &me.endpoints.releases).await;
                    me.update(|s| s.builds = Some(builds));
                }
                Command::InstallEngine { build, variant } => {
                    let _engine = me.engine_lock.lock().await;
                    let _ = me.install_engine(build, variant).await;
                }
                Command::RemoveEngine => me.remove_engine().await,
                Command::QueryRepo(repo) => me.query_repo(&repo).await,
                Command::Download { repo, choice, mtp } => me.download(&repo, choice, mtp).await,
                Command::Resume(id) => {
                    if let Some(cancel) = me.claim(&id) {
                        me.fetch_model(&id, &cancel).await;
                        lock(&me.cancels).remove(&id);
                    }
                }
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
    async fn track<T>(
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
                gpu: hardware::detect_gpu(),
                ram_total,
                ram_free,
                ..Hardware::default()
            };
            let Some(exe) = exe else {
                return (hw, false);
            };
            match list_devices(&exe) {
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
            None => {
                let listed = engine::list_builds(&self.gh_http, &self.endpoints.releases).await;
                self.update(|s| s.builds = Some(listed.clone()));
                listed?
            }
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
            engine::auto_variant(tokio::task::block_in_place(hardware::detect_gpu).as_ref())
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
        self.restart_servers();
        self.refresh_hardware().await;
        Ok(())
    }

    async fn remove_engine(&self) {
        let _engine = self.engine_lock.lock().await;
        self.shutdown();
        let state = match engine::remove_all(&self.root) {
            Ok(()) => EngineState::Missing,
            Err(e) => {
                log(format!("[local] removing engines failed: {e}"));
                EngineState::Failed(e.to_string())
            }
        };
        self.update(|s| s.engine = state);
    }

    async fn query_repo(&self, repo: &str) {
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

    fn repo_info(&self, repo: &str) -> Option<RepoInfo> {
        match lock(&self.state).repos.get(repo) {
            Some(RepoState::Ready(info)) => Some(info.clone()),
            _ => None,
        }
    }

    async fn download(&self, repo: &str, choice: usize, mtp: Option<usize>) {
        if self.repo_info(repo).is_none() {
            self.query_repo(repo).await;
        }
        let Some(info) = self.repo_info(repo) else {
            log(format!("[local] download {repo}: repo info unavailable"));
            return;
        };
        let main = info.catalog.choices.get(choice);
        let mtp_choice = mtp.map(|i| info.catalog.mtp.get(i));
        let (Some(main), None | Some(Some(_))) = (main, mtp_choice) else {
            log(format!(
                "[local] download {repo}: no choice {choice} / mtp {mtp:?}"
            ));
            return;
        };
        let mtp_file = mtp_choice.flatten().map(|m| &m.file);
        let to_manifest = |f: &hf::RepoFile| ManifestFile {
            path: f.path.clone(),
            size: f.size,
            sha256: f.sha256.clone(),
        };
        let manifest = Manifest {
            repo: repo.to_owned(),
            quant: main.quant.label.clone(),
            files: main.files.iter().map(to_manifest).collect(),
            mtp: mtp_file.map(to_manifest),
            moe: info.meta.expert_count > 0,
            has_builtin_mtp: info.meta.has_mtp,
            required: fit::required(
                &info.meta,
                fit::total_size(main),
                mtp_file.map_or(0, |f| f.size),
            ),
        };
        let id = store::model_id(repo, &main.quant.label);
        let Some(cancel) = self.claim(&id) else {
            log(format!("[local] download {id} already running"));
            return;
        };
        let prepared = async {
            store::write_manifest(&self.root, &id, &manifest).map_err(|e| {
                log(format!("[local] writing manifest for {id} failed: {e}"));
                e.to_string()
            })?;
            self.update(|s| s.models = store::list(&self.root));
            let _engine = self.engine_lock.lock().await;
            if !matches!(lock(&self.state).engine, EngineState::Ready(_)) {
                log(format!(
                    "[local] download {id}: installing the engine first"
                ));
                self.install_engine(None, None).await?;
            }
            Ok::<_, String>(())
        };
        match prepared.await {
            Ok(()) => self.fetch_model(&id, &cancel).await,
            Err(e) => self.update(|s| {
                s.downloads.insert(id.clone(), DownloadState::Failed(e));
            }),
        }
        lock(&self.cancels).remove(&id);
    }

    /// 登記 `id` 的下載；已在下載中回 `None`。
    fn claim(&self, id: &str) -> Option<CancellationToken> {
        let mut cancels = lock(&self.cancels);
        if cancels.contains_key(id) {
            return None;
        }
        let token = CancellationToken::new();
        cancels.insert(id.to_owned(), token.clone());
        self.update(|s| {
            s.downloads
                .insert(id.to_owned(), DownloadState::Running { done: 0, total: 0 });
        });
        Some(token)
    }

    /// 依清單下載缺少的檔案；已完整的檔略過。
    async fn fetch_model(&self, id: &str, cancel: &CancellationToken) {
        let Some(model) = store::list(&self.root).into_iter().find(|m| m.id == id) else {
            log(format!("[local] download {id}: no manifest on disk"));
            self.update(|s| {
                s.downloads.remove(id);
            });
            return;
        };
        let m = &model.manifest;
        let files: Vec<&ManifestFile> = m.files.iter().chain(&m.mtp).collect();
        let total: u64 = files.iter().map(|f| f.size).sum();
        let dir = store::models_dir(&self.root).join(&model.id);
        log(format!(
            "[local] model download start: id={id} repo={} files={} bytes={total}",
            m.repo,
            files.len()
        ));
        let mut done = 0;
        for f in files {
            let Some(name) = f.local_name() else {
                return self.download_failed(id, format!("invalid file path {}", f.path));
            };
            let dest = dir.join(name);
            if std::fs::metadata(&dest).is_ok_and(|md| md.len() == f.size) {
                done += f.size;
                continue;
            }
            let url = hf::resolve_url(&self.endpoints.hf, &m.repo, &f.path);
            let progress = Progress::default();
            let fetch = download::download(
                &self.hf_http,
                &url,
                &dest,
                f.size,
                f.sha256.as_deref(),
                &progress,
                cancel,
            );
            let result = self
                .track(&progress, fetch, |s, d, _| {
                    s.downloads.insert(
                        id.to_owned(),
                        DownloadState::Running {
                            done: done + d,
                            total,
                        },
                    );
                })
                .await;
            match result {
                Ok(()) => done += f.size,
                Err(DownloadError::Cancelled) => {
                    let done = done + progress.done.load(Ordering::Relaxed);
                    log(format!(
                        "[local] model download paused: id={id} done={done}"
                    ));
                    return self.update(|s| {
                        s.downloads
                            .insert(id.to_owned(), DownloadState::Paused { done, total });
                    });
                }
                Err(e) => return self.download_failed(id, e.to_string()),
            }
        }
        log(format!("[local] model download finished: id={id}"));
        self.update(|s| {
            s.downloads.remove(id);
            s.models = store::list(&self.root);
        });
    }

    fn download_failed(&self, id: &str, error: String) {
        log(format!("[local] model download failed: id={id}: {error}"));
        self.update(|s| {
            s.downloads
                .insert(id.to_owned(), DownloadState::Failed(error));
        });
    }

    /// 先取消進行中的下載並等它放開 `.part`、停掉子程序，再刪資料夾。
    async fn delete(&self, id: &str) {
        if let Some(cancel) = lock(&self.cancels).get(id) {
            cancel.cancel();
        }
        while lock(&self.cancels).contains_key(id) {
            tokio::time::sleep(PROGRESS_TICK).await;
        }
        self.stop(id);
        match store::delete(&self.root, id) {
            Ok(()) => log(format!("[local] deleted model {id}")),
            Err(e) => log(format!("[local] deleting model {id} failed: {e}")),
        }
        self.update(|s| {
            s.downloads.remove(id);
            s.models = store::list(&self.root);
        });
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

/// 執行 `llama-server --list-devices`，回傳 stdout 與 stderr 的內容。
fn list_devices(exe: &Path) -> Result<String, String> {
    let mut child = std::process::Command::new(exe)
        .arg("--list-devices")
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let pipes: [Option<Box<dyn Read + Send>>; 2] = [
        child.stdout.take().map(|p| Box::new(p) as _),
        child.stderr.take().map(|p| Box::new(p) as _),
    ];
    let readers: Vec<_> = pipes
        .into_iter()
        .flatten()
        .map(|mut pipe| {
            std::thread::spawn(move || {
                let mut buf = Vec::new();
                let _ = pipe.read_to_end(&mut buf);
                String::from_utf8_lossy(&buf).into_owned()
            })
        })
        .collect();
    let deadline = Instant::now() + LIST_DEVICES_TIMEOUT;
    let status = poll_until(
        &Clock::default(),
        deadline,
        Duration::from_millis(100),
        || child.try_wait().ok().flatten(),
    );
    if status.is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let output: Vec<String> = readers.into_iter().filter_map(|r| r.join().ok()).collect();
    let output = output.join("\n");
    match status {
        None => Err(format!(
            "did not finish within {}s",
            LIST_DEVICES_TIMEOUT.as_secs()
        )),
        Some(s) if !s.success() => Err(format!("exited with {s}:\n{output}")),
        Some(_) => Ok(output),
    }
}
