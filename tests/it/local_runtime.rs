//! 本地模型 hub：子程序管理與下載（假 Launcher、wiremock 當 `/health` 與 Hugging Face，不啟動真程序）。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use crate::common::local::{add_engine, add_model, serve_on};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::runtime::Handle;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use wizard101_chat_translator::config::local::LocalSettings;
use wizard101_chat_translator::config::{Config, default_config};
use wizard101_chat_translator::local::engine::{self, EngineInstall, Variant};
use wizard101_chat_translator::local::fit::Basis;
use wizard101_chat_translator::local::hardware::{self, Gpu, GpuKind};
use wizard101_chat_translator::local::server::{Launcher, Mtp, Process, ServerPlan};
use wizard101_chat_translator::local::store::{self, Measured};
use wizard101_chat_translator::local::usage::{Reading, Usage};
use wizard101_chat_translator::local::{
    Command, DownloadState, Endpoints, EngineState, Local, LocalNotice, RepoInfo, RepoState,
    Snapshot,
};

type Events = Arc<Mutex<Vec<String>>>;
type FailWhen = Box<dyn Fn(&ServerPlan) -> bool + Send + Sync>;
type Exit = Arc<Mutex<Option<i32>>>;

struct FakeProcess {
    exit: Exit,
    events: Events,
    model_dir: PathBuf,
    exe: PathBuf,
    exe_at_kill: Arc<Mutex<Vec<bool>>>,
    kill_delay: Arc<Mutex<Duration>>,
    pid: u32,
    all_on_gpu: Arc<Mutex<Option<bool>>>,
    mapped: Arc<Mutex<u64>>,
    unloaded: Arc<Mutex<bool>>,
    _health: Option<MockServer>,
}

impl Process for FakeProcess {
    fn exited(&mut self) -> Option<i32> {
        *self.exit.lock().unwrap()
    }
    fn kill(&mut self) {
        let delay = *self.kill_delay.lock().unwrap();
        std::thread::sleep(delay);
        self.exe_at_kill.lock().unwrap().push(self.exe.exists());
        let exists = self.model_dir.exists();
        self.events
            .lock()
            .unwrap()
            .push(format!("kill model_dir_exists={exists}"));
    }
    fn tail(&self) -> Vec<String> {
        vec!["error: fake failure".into()]
    }
    fn pid(&self) -> Option<u32> {
        Some(self.pid)
    }
    fn all_on_gpu(&self) -> Option<bool> {
        *self.all_on_gpu.lock().unwrap()
    }
    fn mapped(&self) -> u64 {
        *self.mapped.lock().unwrap()
    }
    fn unloaded(&self) -> bool {
        *self.unloaded.lock().unwrap()
    }
}

#[derive(Clone, Default)]
struct Shared {
    plans: Arc<Mutex<Vec<ServerPlan>>>,
    launched_at: Arc<Mutex<Vec<Instant>>>,
    /// 每個啟動過的子程序的結束碼，測試可從外面改成已結束。
    exits: Arc<Mutex<Vec<Exit>>>,
    events: Events,
    health_delay: Arc<Mutex<Duration>>,
    /// `--list-devices` 的假輸出；`None` 表示量測失敗。
    devices: Arc<Mutex<Option<String>>>,
    /// `launch` 本身卡住的時間（模擬啟動途中、取消不到的那一段）。
    launch_delay: Arc<Mutex<Duration>>,
    /// 每次 kill 當下引擎 exe 是否還在。
    exe_at_kill: Arc<Mutex<Vec<bool>>>,
    /// `--list-devices` 卡住的時間。
    measure_delay: Arc<Mutex<Duration>>,
    /// 每次 `--list-devices` 結束的時間與當下 exe 是否還在。
    measured: Arc<Mutex<Vec<(Instant, bool)>>>,
    /// 偵測 GPU 的次數。
    gpu_detections: Arc<Mutex<usize>>,
    /// 子程序 kill 要等它退出的時間。
    kill_delay: Arc<Mutex<Duration>>,
    /// 每台假 llama-server 回報的實際佔用；`process_usage` 收到的 PID 也記在這。
    usage_per_server: Arc<Mutex<Usage>>,
    usage_pids: Arc<Mutex<Vec<u32>>>,
    /// 假 llama-server 回報的「每層都在 GPU」；`None` 是還沒印 `load_tensors: offloaded`。
    all_on_gpu: Arc<Mutex<Option<bool>>>,
    /// 假 llama-server 回報的對映權重（`CPU_Mapped` buffer）。
    mapped: Arc<Mutex<u64>>,
    /// 每台假 llama-server 的 working set 比私有部分多出的（對映的模型檔）。
    shared_pages: Arc<Mutex<u64>>,
    /// 假 llama-server 睡眠中（模型不在記憶體裡）。
    unloaded: Arc<Mutex<bool>>,
    /// GPU 計數器讀不到。
    gpu_unreadable: Arc<Mutex<bool>>,
}

struct FakeLauncher {
    shared: Shared,
    fail_when: FailWhen,
    model_dir: PathBuf,
}

impl Launcher for FakeLauncher {
    fn launch(&self, plan: &ServerPlan, port: u16) -> io::Result<Box<dyn Process>> {
        let s = &self.shared;
        s.plans.lock().unwrap().push(plan.clone());
        s.launched_at.lock().unwrap().push(Instant::now());
        let fail = (self.fail_when)(plan);
        let exit = Arc::new(Mutex::new(fail.then_some(1)));
        s.exits.lock().unwrap().push(exit.clone());
        let delay = *s.health_delay.lock().unwrap();
        let launch_delay = *s.launch_delay.lock().unwrap();
        tokio::task::block_in_place(|| std::thread::sleep(launch_delay));
        Ok(Box::new(FakeProcess {
            exit,
            events: s.events.clone(),
            model_dir: self.model_dir.clone(),
            exe: plan.exe.clone(),
            exe_at_kill: s.exe_at_kill.clone(),
            kill_delay: s.kill_delay.clone(),
            pid: u32::from(port),
            all_on_gpu: s.all_on_gpu.clone(),
            mapped: s.mapped.clone(),
            unloaded: s.unloaded.clone(),
            _health: (!fail).then(|| health_server(port, delay)),
        }))
    }

    fn detect_gpu(&self) -> Option<Gpu> {
        *self.shared.gpu_detections.lock().unwrap() += 1;
        Some(Gpu {
            name: "Fake GPU".into(),
            vendor: 0x10de,
            kind: GpuKind::Discrete,
            nvidia_driver: None,
            device: 0,
        })
    }

    fn list_devices(&self, exe: &Path) -> Result<String, String> {
        let delay = *self.shared.measure_delay.lock().unwrap();
        std::thread::sleep(delay);
        let done = (Instant::now(), exe.exists());
        self.shared.measured.lock().unwrap().push(done);
        let devices = self.shared.devices.lock().unwrap().clone();
        devices.ok_or_else(|| "fake measurement failure".into())
    }

    fn process_usage(&self, pids: &[u32], _integrated: bool) -> Reading {
        *self.shared.usage_pids.lock().unwrap() = pids.to_vec();
        let each = *self.shared.usage_per_server.lock().unwrap();
        let n = pids.len() as u64;
        let shared_pages = *self.shared.shared_pages.lock().unwrap();
        Reading {
            usage: Usage {
                vram: each.vram * n,
                ram: each.ram * n,
            },
            resident: (each.ram + shared_pages) * n,
            gpu_read: !*self.shared.gpu_unreadable.lock().unwrap(),
        }
    }
}

/// 在 llama-server 該聽的埠上開一個 `/health` 回 200 的 wiremock。
fn health_server(port: u16, delay: Duration) -> MockServer {
    serve_on(
        port,
        vec![
            Mock::given(method("GET"))
                .and(path("/health"))
                .respond_with(ResponseTemplate::new(200).set_delay(delay)),
        ],
    )
}

struct Fixture {
    _dir: TempDir,
    root: PathBuf,
    hf: MockServer,
    local: Arc<Local>,
    shared: Shared,
    notices: Arc<Mutex<Vec<LocalNotice>>>,
}

impl Fixture {
    fn plans(&self) -> Vec<ServerPlan> {
        self.shared.plans.lock().unwrap().clone()
    }

    fn kills(&self) -> Vec<String> {
        self.shared.events.lock().unwrap().clone()
    }
}

/// 建 hub；`prepare` 在建立前寫好引擎與模型。
async fn fixture(
    prepare: impl FnOnce(&Path),
    fail_when: impl Fn(&ServerPlan) -> bool + Send + Sync + 'static,
) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    prepare(&root);
    let hf = MockServer::start().await;
    let shared = Shared::default();
    let notices = Arc::new(Mutex::new(Vec::new()));
    let sink = notices.clone();
    let local = Local::new(
        root.clone(),
        Handle::current(),
        Arc::new(FakeLauncher {
            shared: shared.clone(),
            fail_when: Box::new(fail_when),
            model_dir: store::models_dir(&root).join("m1"),
        }),
        Endpoints {
            hf: hf.uri(),
            releases: format!("{}/releases", hf.uri()),
        },
        Arc::new(move |n| sink.lock().unwrap().push(n)),
    );
    Fixture {
        _dir: dir,
        root,
        hf,
        local,
        shared,
        notices,
    }
}

async fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn engine_and_model(with_mtp: bool) -> impl FnOnce(&Path) {
    move |root| {
        add_engine(root, "b11871");
        add_model(root, "m1", with_mtp);
    }
}

/// 同一模型的並發 ensure 只啟動一次，回同一個 base URL。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ensure_starts_once_for_concurrent_requests() {
    let f = fixture(engine_and_model(false), |_| false).await;
    let (a, b) = tokio::join!(f.local.ensure("m1"), f.local.ensure("m1"));
    let a = a.unwrap();
    assert_eq!(Ok(a.clone()), b);
    assert!(a.starts_with("http://127.0.0.1:"), "{a}");
    assert_eq!(f.plans().len(), 1);
}

/// 帶 MTP 啟動失敗時關掉 MTP 重試一次。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mtp_failure_retries_without_mtp() {
    let f = fixture(engine_and_model(true), |p| p.mtp != Mtp::Off).await;
    f.local.ensure("m1").await.unwrap();
    let plans = f.plans();
    assert_eq!(plans.len(), 2);
    assert!(matches!(plans[0].mtp, Mtp::Draft(_)), "{:?}", plans[0].mtp);
    assert_eq!(plans[1].mtp, Mtp::Off);
}

/// 新啟用、尚未確認的引擎啟動失敗時退回上一版再試。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unconfirmed_engine_rolls_back_on_failure() {
    let f = fixture(
        |root| {
            add_engine(root, "b11871");
            add_engine(root, "b11872");
            add_model(root, "m1", false);
        },
        |p| p.exe.to_string_lossy().contains("b11872"),
    )
    .await;
    f.local.ensure("m1").await.unwrap();
    let active = engine::read_active(&f.root);
    let previous = active.current.unwrap();
    assert_eq!(previous.build, "b11871");
    assert_eq!(f.local.snapshot().engine, EngineState::Ready(previous));
    assert_eq!(active.previous, None);
    let plans = f.plans();
    assert_eq!(plans.len(), 2);
    assert!(plans[1].exe.to_string_lossy().contains("b11871"));
}

/// 啟動一直失敗時回傳原文錯誤，連續兩次後不再嘗試。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn repeated_failures_stop_retrying() {
    let f = fixture(engine_and_model(false), |_| true).await;
    let first = f.local.ensure("m1").await.unwrap_err();
    assert!(first.contains("error: fake failure"), "{first}");
    f.local.ensure("m1").await.unwrap_err();
    assert_eq!(f.local.ensure("m1").await.unwrap_err(), first);
    assert_eq!(f.plans().len(), 2);
}

/// 沒有任何用途用到的模型會被停掉。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn apply_config_stops_models_no_slot_uses() {
    let f = fixture(engine_and_model(false), |_| false).await;
    f.local.ensure("m1").await.unwrap();
    f.local.apply_config(&default_config());
    assert_eq!(f.kills(), ["kill model_dir_exists=true".to_string()]);
}

/// 刪除模型前先停掉它的子程序。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn delete_stops_the_server_before_removing_files() {
    let f = fixture(engine_and_model(false), |_| false).await;
    f.local.ensure("m1").await.unwrap();
    f.local.command(Command::Delete("m1".into()));
    let dir = store::models_dir(&f.root).join("m1");
    wait_for("model folder removed", || !dir.exists()).await;
    assert_eq!(f.kills(), ["kill model_dir_exists=true".to_string()]);
    wait_for("models refreshed", || f.local.snapshot().models.is_empty()).await;
}

/// 啟動前後各送一次載入通知。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn loading_notices_wrap_the_start() {
    let f = fixture(engine_and_model(false), |_| false).await;
    f.local.ensure("m1").await.unwrap();
    let loading: Vec<LocalNotice> = f
        .notices
        .lock()
        .unwrap()
        .iter()
        .filter(|n| matches!(n, LocalNotice::Loading(_)))
        .cloned()
        .collect();
    assert_eq!(
        loading,
        [
            LocalNotice::Loading(Some("Model-GGUF · Q4_K_M".into())),
            LocalNotice::Loading(None)
        ]
    );
}

/// 最小的 GGUF v3 檔頭：只有架構名稱。
fn tiny_gguf() -> Vec<u8> {
    let mut out = b"GGUF".to_vec();
    out.extend(3u32.to_le_bytes());
    out.extend(0u64.to_le_bytes());
    out.extend(1u64.to_le_bytes());
    let key = "general.architecture";
    out.extend((key.len() as u64).to_le_bytes());
    out.extend(key.as_bytes());
    out.extend(8u32.to_le_bytes());
    out.extend(5u64.to_le_bytes());
    out.extend(b"llama");
    out
}

/// 查詢 repo 後下載：先寫清單、下載完成後列為完整模型。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn query_then_download_installs_the_model() {
    let f = fixture(|root| drop(add_engine(root, "b11871")), |_| false).await;
    let body = tiny_gguf();
    Mock::given(method("GET"))
        .and(path("/api/models/org/Model-GGUF/tree/main"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"type": "file", "path": "model-Q4_K_M.gguf", "size": body.len()},
            {"type": "file", "path": "README.md", "size": 10}
        ])))
        .mount(&f.hf)
        .await;
    Mock::given(method("GET"))
        .and(path("/org/Model-GGUF/resolve/main/model-Q4_K_M.gguf"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
        .mount(&f.hf)
        .await;

    f.local.command(Command::QueryRepo("org/Model-GGUF".into()));
    wait_for("repo ready", || {
        matches!(
            f.local.snapshot().repos.get("org/Model-GGUF"),
            Some(RepoState::Ready(_) | RepoState::Failed(_))
        )
    })
    .await;
    let Some(RepoState::Ready(info)) = f.local.snapshot().repos.remove("org/Model-GGUF") else {
        panic!("{:?}", f.local.snapshot().repos);
    };
    assert_eq!(info.fits.len(), 1);
    assert_eq!(info.fits[0][0].fit, None, "VRAM not measured yet");

    f.local.command(Command::Download {
        repo: "org/Model-GGUF".into(),
        choice: 0,
        mtp: None,
        auto: true,
    });
    let id = store::model_id("org/Model-GGUF", "Q4_K_M");
    wait_for("model complete", || {
        let s = f.local.snapshot();
        s.models.iter().any(|m| m.id == id && m.complete) && s.downloads.is_empty()
            || matches!(s.downloads.get(&id), Some(DownloadState::Failed(_)))
    })
    .await;
    let s = f.local.snapshot();
    assert!(s.downloads.is_empty(), "{:?}", s.downloads);
    assert_eq!(s.models[0].manifest.quant, "Q4_K_M");
}

/// 每個用途各用一個本地模型（`models[0]` 給收訊、`models[1]` 給發話）的設定。
fn local_config(models: &[&str], local_models: Value) -> Config {
    let mut cfg = default_config();
    let services: Vec<Value> = models
        .iter()
        .enumerate()
        .map(|(i, m)| json!({"id": format!("s{i}"), "name": format!("s{i}"), "provider": "local", "model": m}))
        .collect();
    cfg.insert("services".into(), services.into());
    cfg.insert("default_service".into(), "s0".into());
    let outgoing = if models.len() > 1 { "s1" } else { "s0" };
    cfg.insert(
        "service_slots".into(),
        json!({"incoming": "s0", "outgoing": outgoing, "region": null}),
    );
    cfg.insert("local_models".into(), local_models);
    cfg
}

/// 子程序意外結束時重啟一次；第二次結束後停在失敗狀態並回傳結束原因。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unexpected_exit_restarts_once_then_stays_failed() {
    let f = fixture(engine_and_model(false), |_| false).await;
    f.local.ensure("m1").await.unwrap();
    *f.shared.exits.lock().unwrap()[0].lock().unwrap() = Some(3);
    f.local.ensure("m1").await.unwrap();
    assert_eq!(f.plans().len(), 2);
    *f.shared.exits.lock().unwrap()[1].lock().unwrap() = Some(4);
    let err = f.local.ensure("m1").await.unwrap_err();
    assert!(err.contains("code 4"), "{err}");
    assert_eq!(f.local.ensure("m1").await.unwrap_err(), err);
    assert_eq!(f.plans().len(), 2);
}

/// 模型還沒下載完時的 ensure 不算失敗，下載完就能直接啟動。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ensure_before_download_does_not_count_as_failure() {
    let f = fixture(|root| drop(add_engine(root, "b11871")), |_| false).await;
    for _ in 0..3 {
        let err = f.local.ensure("m1").await.unwrap_err();
        assert!(err.contains("not downloaded"), "{err}");
    }
    add_model(&f.root, "m1", false);
    f.local.ensure("m1").await.unwrap();
}

/// 設定改了閒置分鐘或該模型的 MTP 開關時重啟跑著的子程序。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn apply_config_restarts_servers_when_their_settings_change() {
    let f = fixture(engine_and_model(true), |_| false).await;
    let settings =
        |idle: u32, mtp: Value| json!({"preload": false, "idle_minutes": idle, "mtp": mtp});
    f.local
        .apply_config(&local_config(&["m1"], settings(15, json!({}))));
    f.local.ensure("m1").await.unwrap();
    f.local
        .apply_config(&local_config(&["m1"], settings(5, json!({}))));
    wait_for("restart after idle change", || f.plans().len() == 2).await;
    f.local
        .apply_config(&local_config(&["m1"], settings(5, json!({"m1": false}))));
    wait_for("restart after MTP change", || f.plans().len() == 3).await;
    let plans = f.plans();
    assert_eq!(plans[1].idle_minutes, 5);
    assert!(matches!(plans[1].mtp, Mtp::Draft(_)), "{:?}", plans[1].mtp);
    assert_eq!(plans[2].mtp, Mtp::Off);
    assert_eq!(f.kills().len(), 2);
}

/// 預先載入依序啟動：前一個就緒後才啟動下一個。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn preload_starts_models_one_after_another() {
    let f = fixture(
        |root| {
            add_engine(root, "b11871");
            add_model(root, "m1", false);
            add_model(root, "m2", false);
        },
        |_| false,
    )
    .await;
    let delay = Duration::from_millis(400);
    *f.shared.health_delay.lock().unwrap() = delay;
    let settings = json!({"preload": true, "idle_minutes": 15, "mtp": {}});
    f.local.apply_config(&local_config(&["m1", "m2"], settings));
    wait_for("both started", || f.plans().len() == 2).await;
    let ids: Vec<String> = f.plans().into_iter().map(|p| p.model_id).collect();
    assert_eq!(ids, ["m1", "m2"]);
    let at = f.shared.launched_at.lock().unwrap().clone();
    assert!(
        at[1] - at[0] >= delay,
        "second launch {:?} after the first",
        at[1] - at[0]
    );
}

/// 預先載入只載入用途用得到的模型：新增了但沒有用途（含預設）選用的本地服務不載入。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn preload_skips_local_services_no_slot_uses() {
    let f = fixture(
        |root| {
            add_engine(root, "b11871");
            add_model(root, "m1", false);
            add_model(root, "m2", false);
        },
        |_| false,
    )
    .await;
    let settings = json!({"preload": true, "idle_minutes": 0, "mtp": {}});
    let mut cfg = local_config(&["m1", "m2"], settings);
    cfg.insert(
        "service_slots".into(),
        json!({"incoming": "s0", "outgoing": null, "region": null}),
    );
    f.local.apply_config(&cfg);
    wait_for("m1 preloaded", || f.plans().len() == 1).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let ids: Vec<String> = f.plans().into_iter().map(|p| p.model_id).collect();
    assert_eq!(ids, ["m1"]);
}

/// shutdown 後 ensure 回錯誤、不啟動，也不算失敗。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ensure_after_shutdown_starts_nothing() {
    let f = fixture(engine_and_model(false), |_| false).await;
    f.local.ensure("m1").await.unwrap();
    f.local.shutdown();
    assert_eq!(f.kills().len(), 1);
    let err = f.local.ensure("m1").await.unwrap_err();
    assert!(err.contains("shut down"), "{err}");
    assert_eq!(f.plans().len(), 1);
}

/// 預先載入途中 shutdown：載入中的那個被取消，排在後面的不再啟動。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shutdown_stops_the_preload_queue() {
    let f = fixture(
        |root| {
            add_engine(root, "b11871");
            add_model(root, "m1", false);
            add_model(root, "m2", false);
        },
        |_| false,
    )
    .await;
    *f.shared.health_delay.lock().unwrap() = Duration::from_millis(400);
    let settings = json!({"preload": true, "idle_minutes": 15, "mtp": {}});
    f.local.apply_config(&local_config(&["m1", "m2"], settings));
    wait_for("first preload started", || f.plans().len() == 1).await;
    f.local.shutdown();
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(f.plans().len(), 1);
}

const REPO: &str = "org/Model-GGUF";

/// 掛上 repo 檔案列表與每個檔的內容。
async fn mount_repo(hf: &MockServer, files: &[(&str, Vec<u8>)]) {
    let tree: Vec<Value> = files
        .iter()
        .map(|(name, body)| json!({"type": "file", "path": name, "size": body.len()}))
        .collect();
    Mock::given(method("GET"))
        .and(path(format!("/api/models/{REPO}/tree/main")))
        .respond_with(ResponseTemplate::new(200).set_body_json(tree))
        .mount(hf)
        .await;
    for (name, body) in files {
        Mock::given(method("GET"))
            .and(path(format!("/{REPO}/resolve/main/{name}")))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone()))
            .mount(hf)
            .await;
    }
}

/// 下一次下載 `name` 時回應延遲很久，讓測試能在下載途中動作。
async fn stall_next_download(hf: &MockServer, name: &str) {
    Mock::given(method("GET"))
        .and(path(format!("/{REPO}/resolve/main/{name}")))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(60)))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(hf)
        .await;
}

async fn query(f: &Fixture) -> RepoInfo {
    f.local.command(Command::QueryRepo(REPO.into()));
    wait_for("repo queried", || {
        matches!(
            f.local.snapshot().repos.get(REPO),
            Some(RepoState::Ready(_) | RepoState::Failed(_))
        )
    })
    .await;
    match f.local.snapshot().repos.remove(REPO) {
        Some(RepoState::Ready(info)) => info,
        other => panic!("{other:?}"),
    }
}

fn model_dir(f: &Fixture, id: &str) -> PathBuf {
    store::models_dir(&f.root).join(id)
}

/// 自動挑選的下載（使用者沒動量化表）。
fn download(choice: usize) -> Command {
    Command::Download {
        repo: REPO.into(),
        choice,
        mtp: None,
        auto: true,
    }
}

/// 取消後為 Paused，按繼續下載可完成。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancel_pauses_and_resume_completes() {
    let f = fixture(|root| drop(add_engine(root, "b11871")), |_| false).await;
    mount_repo(&f.hf, &[("model-Q4_K_M.gguf", tiny_gguf())]).await;
    query(&f).await;
    stall_next_download(&f.hf, "model-Q4_K_M.gguf").await;
    let id = store::model_id(REPO, "Q4_K_M");
    f.local.command(download(0));
    let part = model_dir(&f, &id).join("model-Q4_K_M.gguf.part");
    wait_for("download started", || part.exists()).await;
    f.local.command(Command::CancelDownload(id.clone()));
    wait_for("paused", || {
        matches!(
            f.local.snapshot().downloads.get(&id),
            Some(DownloadState::Paused { .. })
        )
    })
    .await;
    f.local.command(Command::Resume(id.clone()));
    wait_for("complete", || {
        let s = f.local.snapshot();
        s.downloads.is_empty() && s.models.iter().any(|m| m.id == id && m.complete)
    })
    .await;
}

/// 下載途中刪除：先停下載放開檔案，資料夾刪得掉。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn delete_during_download_removes_the_folder() {
    let f = fixture(|root| drop(add_engine(root, "b11871")), |_| false).await;
    mount_repo(&f.hf, &[("model-Q4_K_M.gguf", tiny_gguf())]).await;
    query(&f).await;
    stall_next_download(&f.hf, "model-Q4_K_M.gguf").await;
    let id = store::model_id(REPO, "Q4_K_M");
    f.local.command(download(0));
    let dir = model_dir(&f, &id);
    let part = dir.join("model-Q4_K_M.gguf.part");
    wait_for("download started", || part.exists()).await;
    f.local.command(Command::Delete(id.clone()));
    wait_for("folder removed", || !dir.exists()).await;
    wait_for("state cleared", || {
        let s = f.local.snapshot();
        s.downloads.is_empty() && s.models.is_empty()
    })
    .await;
}

fn engine_zip(exe: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(vec![]));
    w.start_file("llama-server.exe", zip::write::SimpleFileOptions::default())
        .unwrap();
    w.write_all(exe).unwrap();
    w.finish().unwrap().into_inner()
}

/// 掛上 b11871 的 releases 列表（延遲 `delay` 回應）與引擎 zip。
async fn mount_engine(hf: &MockServer, delay: Duration) {
    mount_engine_zip(hf, delay, engine_zip(b"")).await;
}

async fn mount_engine_zip(hf: &MockServer, delay: Duration, zip: Vec<u8>) {
    let variants = ["vulkan", "cpu"];
    let assets: Vec<Value> = variants
        .iter()
        .map(|v| {
            let name = format!("llama-b11871-bin-win-{v}-x64.zip");
            json!({"name": name, "size": zip.len(), "browser_download_url": format!("{}/{name}", hf.uri())})
        })
        .collect();
    Mock::given(method("GET"))
        .and(path("/releases"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!([
                    {"tag_name": "b11871", "published_at": "2026-10-25T00:00:00Z", "assets": assets}
                ]))
                .set_delay(delay),
        )
        .mount(hf)
        .await;
    for v in variants {
        Mock::given(method("GET"))
            .and(path(format!("/llama-b11871-bin-win-{v}-x64.zip")))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(zip.clone()))
            .mount(hf)
            .await;
    }
}

/// 掛上 Q4 與較大的 Q8 兩個量化，回傳兩者在 catalog 內的索引 `(q4, q8)`。
async fn two_quants(f: &Fixture) -> (usize, usize) {
    let mut big = tiny_gguf();
    big.extend([0u8; 64]);
    mount_repo(
        &f.hf,
        &[("model-Q4_K_M.gguf", tiny_gguf()), ("model-Q8_0.gguf", big)],
    )
    .await;
    let info = query(f).await;
    let index = |label: &str| {
        info.catalog
            .choices
            .iter()
            .position(|c| c.quant.label == label)
            .unwrap()
    };
    (index("Q4_K_M"), index("Q8_0"))
}

const FAKE_DEVICES: &str = "Available devices:\n  Vulkan0: Fake GPU (8192 MiB, 8000 MiB free)\n";

async fn wait_complete(f: &Fixture, id: &str) {
    wait_for("model complete", || {
        let s = f.local.snapshot();
        s.downloads.is_empty() && s.models.iter().any(|m| m.id == id && m.complete)
    })
    .await;
}

/// 引擎還沒裝時下載：裝好引擎、量到 VRAM 後改下載自動挑選的量化，原本選擇的清單不留。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn download_without_engine_uses_the_auto_pick_once_vram_is_measured() {
    let f = fixture(|_| {}, |_| false).await;
    *f.shared.devices.lock().unwrap() = Some(FAKE_DEVICES.into());
    mount_engine(&f.hf, Duration::ZERO).await;
    let (_, q8) = two_quants(&f).await;
    f.local.command(download(q8));
    let q4_id = store::model_id(REPO, "Q4_K_M");
    wait_complete(&f, &q4_id).await;
    let s = f.local.snapshot();
    assert!(matches!(s.engine, EngineState::Ready(_)), "{:?}", s.engine);
    assert_eq!(s.hardware.vram_free, 8000 << 20);
    assert_eq!(s.models.len(), 1, "{:?}", s.models);
    assert!(!model_dir(&f, &store::model_id(REPO, "Q8_0")).exists());
}

/// 量不到 VRAM 時維持使用者原本的選擇。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn download_without_engine_keeps_the_choice_when_vram_is_unmeasured() {
    let f = fixture(|_| {}, |_| false).await;
    mount_engine(&f.hf, Duration::ZERO).await;
    let (_, q8) = two_quants(&f).await;
    f.local.command(download(q8));
    wait_complete(&f, &store::model_id(REPO, "Q8_0")).await;
    assert_eq!(f.local.snapshot().models.len(), 1);
}

/// 引擎安裝失敗時清單已寫好，修好後「繼續下載」可完成；沒有清單的 id 繼續下載記為失敗。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resume_after_engine_install_failure_completes() {
    let f = fixture(|_| {}, |_| false).await;
    Mock::given(method("GET"))
        .and(path("/releases"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&f.hf)
        .await;
    mount_engine(&f.hf, Duration::ZERO).await;
    mount_repo(&f.hf, &[("model-Q4_K_M.gguf", tiny_gguf())]).await;
    query(&f).await;
    let id = store::model_id(REPO, "Q4_K_M");
    f.local.command(download(0));
    wait_for("download failed", || {
        matches!(
            f.local.snapshot().downloads.get(&id),
            Some(DownloadState::Failed(_))
        )
    })
    .await;
    assert!(
        f.local
            .snapshot()
            .models
            .iter()
            .any(|m| m.id == id && !m.complete)
    );
    f.local.command(Command::Resume(id.clone()));
    wait_complete(&f, &id).await;

    f.local.command(Command::Resume("missing".into()));
    wait_for("missing manifest reported", || {
        matches!(
            f.local.snapshot().downloads.get("missing"),
            Some(DownloadState::Failed(e)) if e.contains("no manifest")
        )
    })
    .await;
}

/// 等引擎安裝時刪除：下載中止、不改挑別的量化，也不留任何模型資料夾。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn delete_during_engine_install_leaves_nothing_behind() {
    let f = fixture(|_| {}, |_| false).await;
    *f.shared.devices.lock().unwrap() = Some(FAKE_DEVICES.into());
    mount_engine(&f.hf, Duration::from_millis(500)).await;
    let (q4, _) = two_quants(&f).await;
    let id = store::model_id(REPO, "Q4_K_M");
    f.local.command(download(q4));
    // releases 的回應被壓住 500 ms：請求一到，下載就確定正卡在等引擎安裝
    let deadline = Instant::now() + Duration::from_secs(10);
    while !f
        .hf
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .any(|r| r.url.path() == "/releases")
    {
        assert!(Instant::now() < deadline, "timed out waiting for /releases");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(matches!(
        f.local.snapshot().engine,
        EngineState::Missing | EngineState::Installing { .. }
    ));
    f.local.command(Command::Delete(id.clone()));
    wait_for("engine ready and download gone", || {
        let s = f.local.snapshot();
        matches!(s.engine, EngineState::Ready(_)) && s.downloads.is_empty() && s.models.is_empty()
    })
    .await;
    let left: Vec<_> = fs::read_dir(store::models_dir(&f.root))
        .map(|d| d.flatten().map(|e| e.file_name()).collect())
        .unwrap_or_default();
    assert!(left.is_empty(), "{left:?}");
}

/// 已排隊的啟動不會在刪除途中跑起來：刪完才輪到它，得到「沒下載」的錯誤。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn queued_start_waits_until_delete_finishes() {
    let f = fixture(engine_and_model(false), |_| false).await;
    *f.shared.health_delay.lock().unwrap() = Duration::from_secs(60);
    let (a, b) = (f.local.clone(), f.local.clone());
    let first = tokio::spawn(async move { a.ensure("m1").await });
    wait_for("first launched", || f.plans().len() == 1).await;
    let queued = tokio::spawn(async move { b.ensure("m1").await });
    tokio::time::sleep(Duration::from_millis(100)).await;
    f.local.command(Command::Delete("m1".into()));
    let first = first.await.unwrap().unwrap_err();
    assert!(first.contains("unloaded while starting"), "{first}");
    let queued = queued.await.unwrap().unwrap_err();
    assert!(queued.contains("not downloaded"), "{queued}");
    assert!(!store::models_dir(&f.root).join("m1").exists());
    assert_eq!(f.plans().len(), 1);
    assert_eq!(f.kills(), ["kill model_dir_exists=true".to_string()]);
}

/// 啟動等待就緒途中刪除：立刻殺掉子程序再刪資料夾。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn delete_while_starting_kills_the_process_first() {
    let f = fixture(engine_and_model(false), |_| false).await;
    *f.shared.health_delay.lock().unwrap() = Duration::from_secs(60);
    let local = f.local.clone();
    let start = tokio::spawn(async move { local.ensure("m1").await });
    wait_for("launched", || f.plans().len() == 1).await;
    f.local.command(Command::Delete("m1".into()));
    let dir = store::models_dir(&f.root).join("m1");
    wait_for("model folder removed", || !dir.exists()).await;
    assert_eq!(f.kills(), ["kill model_dir_exists=true".to_string()]);
    let err = start.await.unwrap().unwrap_err();
    assert!(err.contains("unloaded while starting"), "{err}");
}

/// 移除引擎途中排隊的啟動（連從沒啟動過的模型）也被停掉：等引擎檔案時已登記的取消鈕不會漏掉，且不算失敗。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ensure_during_engine_removal_is_stopped() {
    let f = fixture(
        |root| {
            add_engine(root, "b11871");
            add_model(root, "m0", false);
            add_model(root, "m1", false);
        },
        |_| false,
    )
    .await;
    *f.shared.launch_delay.lock().unwrap() = Duration::from_secs(1);
    let a = f.local.clone();
    let first = tokio::spawn(async move { a.ensure("m0").await });
    wait_for("m0 launching", || f.plans().len() == 1).await;
    f.local.command(Command::RemoveEngine);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(f.local.snapshot().engine, EngineState::Removing);
    let err = f.local.ensure("m1").await.unwrap_err();
    assert!(err.contains("unloaded while starting"), "{err}");
    assert!(first.await.unwrap().is_err());
    // 卡在啟動中的 m0 要先被殺掉，引擎檔案才能刪
    assert_eq!(*f.shared.exe_at_kill.lock().unwrap(), [true]);
    let ids: Vec<String> = f.plans().into_iter().map(|p| p.model_id).collect();
    assert_eq!(ids, ["m0"]);
    let install = EngineInstall {
        build: "b11871".into(),
        variant: Variant::Cpu,
    };
    assert!(!engine::engine_dir(&f.root, &install).exists());
    wait_for("engine missing", || {
        f.local.snapshot().engine == EngineState::Missing
    })
    .await;
    *f.shared.launch_delay.lock().unwrap() = Duration::ZERO;
    add_engine(&f.root, "b11871");
    f.local.ensure("m1").await.unwrap();
}

fn loading_notices(f: &Fixture) -> Vec<LocalNotice> {
    let notices = f.notices.lock().unwrap();
    notices
        .iter()
        .filter(|n| matches!(n, LocalNotice::Loading(_)))
        .cloned()
        .collect()
}

/// 下載與繼續下載一送出快照就是下載中：舊的失敗紀錄當場被蓋掉，UI 不必猜它是不是舊的。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn download_and_resume_replace_an_old_failure_before_returning() {
    let f = fixture(|_| {}, |_| false).await;
    Mock::given(method("GET"))
        .and(path("/releases"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&f.hf)
        .await;
    mount_repo(&f.hf, &[("model-Q4_K_M.gguf", tiny_gguf())]).await;
    query(&f).await;
    let id = store::model_id(REPO, "Q4_K_M");
    let failed = |f: &Fixture| {
        matches!(
            f.local.snapshot().downloads.get(&id),
            Some(DownloadState::Failed(_))
        )
    };
    let running = |f: &Fixture| {
        matches!(
            f.local.snapshot().downloads.get(&id),
            Some(DownloadState::Running { .. })
        )
    };
    f.local.command(download(0));
    assert!(running(&f));
    wait_for("download failed", || failed(&f)).await;
    f.local.command(download(0));
    assert!(running(&f), "{:?}", f.local.snapshot().downloads);
    wait_for("download failed again", || failed(&f)).await;
    f.local.command(Command::Resume(id.clone()));
    assert!(running(&f), "{:?}", f.local.snapshot().downloads);
    wait_for("resume failed", || failed(&f)).await;
}

/// 手動選的量化：就算這次才裝好引擎、量到 VRAM，也照使用者的選擇下載。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn download_without_engine_keeps_a_manual_pick_even_when_vram_is_measured() {
    let f = fixture(|_| {}, |_| false).await;
    *f.shared.devices.lock().unwrap() = Some(FAKE_DEVICES.into());
    mount_engine(&f.hf, Duration::ZERO).await;
    let (q4, _) = two_quants(&f).await;
    f.local.command(Command::Download {
        repo: REPO.into(),
        choice: q4,
        mtp: None,
        auto: false,
    });
    let q4_id = store::model_id(REPO, "Q4_K_M");
    wait_complete(&f, &q4_id).await;
    let s = f.local.snapshot();
    assert!(s.hardware.measured);
    assert_eq!(s.models.len(), 1, "{:?}", s.models);
}

/// 一次啟動失敗之後成功啟動，失敗就清零：之後意外結束仍會重啟，不會被當成連續失敗。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_good_start_forgets_an_earlier_start_failure() {
    let failing = Arc::new(AtomicBool::new(true));
    let flag = failing.clone();
    let f = fixture(engine_and_model(false), move |_| {
        flag.load(Ordering::SeqCst)
    })
    .await;
    f.local.ensure("m1").await.unwrap_err();
    failing.store(false, Ordering::SeqCst);
    f.local.ensure("m1").await.unwrap();
    *f.shared.exits.lock().unwrap()[1].lock().unwrap() = Some(3);
    f.local.ensure("m1").await.unwrap();
    assert_eq!(f.plans().len(), 3);
}

/// 兩個模型同時載入：先好的那個不收起橫幅，最後一個好了才收。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn overlapping_starts_keep_the_banner_until_the_last_one() {
    let f = fixture(
        |root| {
            add_engine(root, "b11871");
            add_model(root, "m1", false);
            add_model(root, "m2", false);
        },
        |_| false,
    )
    .await;
    *f.shared.health_delay.lock().unwrap() = Duration::from_millis(300);
    let (a, b) = tokio::join!(f.local.ensure("m1"), f.local.ensure("m2"));
    a.unwrap();
    b.unwrap();
    let loading = loading_notices(&f);
    let ends = loading
        .iter()
        .filter(|n| **n == LocalNotice::Loading(None))
        .count();
    assert_eq!(ends, 1, "{loading:?}");
    assert_eq!(loading.last(), Some(&LocalNotice::Loading(None)));
}

/// 程式啟動：先量完硬體才預先載入，量到的可用量不含自己要載入的模型。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn start_measures_the_hardware_before_preloading() {
    let f = fixture(engine_and_model(false), |_| false).await;
    *f.shared.devices.lock().unwrap() = Some(FAKE_DEVICES.into());
    *f.shared.measure_delay.lock().unwrap() = Duration::from_millis(300);
    let settings = json!({"preload": true, "idle_minutes": 15, "mtp": {}});
    f.local.start(&local_config(&["m1"], settings));
    wait_for("preloaded", || f.plans().len() == 1).await;
    let measured = f.shared.measured.lock().unwrap().clone();
    let launched = f.shared.launched_at.lock().unwrap()[0];
    assert_eq!(measured.len(), 1);
    assert!(
        launched >= measured[0].0,
        "launched before the measurement ended"
    );
    assert!(f.local.snapshot().hardware.measured);
}

/// 快照的「偵測中」：啟動後第一次量完以前、以及〔重新偵測〕進行中為真。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_snapshot_says_while_hardware_is_being_detected() {
    let f = fixture(engine_and_model(false), |_| false).await;
    *f.shared.devices.lock().unwrap() = Some(FAKE_DEVICES.into());
    *f.shared.measure_delay.lock().unwrap() = Duration::from_millis(200);
    assert!(f.local.snapshot().detecting);
    f.local.start(&local_config(&[], json!({})));
    wait_for("first measurement", || !f.local.snapshot().detecting).await;
    assert!(f.local.snapshot().hardware.measured);
    f.local
        .command(Command::RefreshHardware { redetect_gpu: true });
    wait_for("re-detect started", || f.local.snapshot().detecting).await;
    wait_for("re-detect finished", || !f.local.snapshot().detecting).await;
    assert_eq!(f.shared.measured.lock().unwrap().len(), 2);
}

/// 模型跑著時重新偵測：可用量照實記，量它的 llama-server 實際佔用另外記下、相容判斷時才加回。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn redetecting_adds_running_models_back_to_free_memory() {
    const G: u64 = 1 << 30;
    let f = fixture(engine_and_model(false), |_| false).await;
    *f.shared.usage_per_server.lock().unwrap() = Usage {
        vram: 3 * G,
        ram: G,
    };
    f.local.ensure("m1").await.unwrap();
    *f.shared.devices.lock().unwrap() =
        Some("Available devices:\n  Vulkan0: Fake GPU (8192 MiB, 4096 MiB free)\n".into());
    f.local
        .command(Command::RefreshHardware { redetect_gpu: true });
    wait_for("measured", || f.local.snapshot().hardware.measured).await;
    let s = f.local.snapshot();
    assert!(s.running.contains("m1"), "{:?}", s.running);
    assert_eq!(s.hardware.vram_free, 4096 << 20, "shown as measured");
    assert_eq!(
        f.shared.usage_pids.lock().unwrap().len(),
        1,
        "the running server"
    );
    assert_eq!(
        s.hardware.own,
        Usage {
            vram: 3 * G,
            ram: G
        }
    );
    assert_eq!(hardware::effective(&s.hardware).0, (4096 << 20) + 3 * G);
}

/// 就緒後量到的佔用記進模型清單（含那次有沒有開 MTP）；還不知道有沒有全放 GPU 時不記。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_ready_server_records_what_it_used() {
    const G: u64 = 1 << 30;
    let f = fixture(engine_and_model(true), |_| false).await;
    *f.shared.usage_per_server.lock().unwrap() = Usage {
        vram: 3 * G,
        ram: G,
    };
    *f.shared.all_on_gpu.lock().unwrap() = Some(true);
    f.local.ensure("m1").await.unwrap();
    assert_eq!(
        store::list(&f.root)[0].manifest.measured,
        None,
        "GPU not detected yet"
    );
    f.local.stop("m1");
    *f.shared.all_on_gpu.lock().unwrap() = None;
    detect(&f).await;
    f.local.ensure("m1").await.unwrap();
    assert_eq!(
        store::list(&f.root)[0].manifest.measured,
        None,
        "unknown offload"
    );
    f.local.stop("m1");
    *f.shared.all_on_gpu.lock().unwrap() = Some(true);
    f.local.ensure("m1").await.unwrap();
    let want = Some(Measured {
        vram: 3 * G,
        ram: G,
        all_on_gpu: true,
        mtp: true,
        engine: "b11871-cpu".into(),
    });
    assert_eq!(store::list(&f.root)[0].manifest.measured, want);
    assert_eq!(f.local.snapshot().models[0].manifest.measured, want);
}

/// 實測的 RAM 是私有的 working set 加上對映留在系統記憶體的權重；加回可用量的是整個 working set（停掉時都會空出來）。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ram_need_excludes_mapped_pages_the_gpu_already_holds() {
    const G: u64 = 1 << 30;
    let f = fixture(engine_and_model(false), |_| false).await;
    detect(&f).await;
    *f.shared.all_on_gpu.lock().unwrap() = Some(true);
    *f.shared.usage_per_server.lock().unwrap() = Usage {
        vram: 5 * G,
        ram: G / 2,
    };
    *f.shared.mapped.lock().unwrap() = G / 2;
    *f.shared.shared_pages.lock().unwrap() = 5 * G;
    f.local.ensure("m1").await.unwrap();
    let measured = store::list(&f.root)[0].manifest.measured.clone().unwrap();
    assert_eq!((measured.vram, measured.ram), (5 * G, G));
    redetect_until(
        &f,
        Usage {
            vram: 5 * G,
            ram: G / 2 + 5 * G,
        },
    )
    .await;
}

/// 重新偵測時再量跑著的模型，保留量過的最大值（第一次請求後佔用會再長）。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn redetecting_keeps_the_largest_measurement() {
    const G: u64 = 1 << 30;
    let f = fixture(engine_and_model(false), |_| false).await;
    detect(&f).await;
    *f.shared.all_on_gpu.lock().unwrap() = Some(true);
    *f.shared.usage_per_server.lock().unwrap() = Usage {
        vram: 3 * G,
        ram: G,
    };
    f.local.ensure("m1").await.unwrap();
    *f.shared.usage_per_server.lock().unwrap() = Usage {
        vram: 2 * G,
        ram: 2 * G,
    };
    f.local.command(Command::RefreshHardware {
        redetect_gpu: false,
    });
    let measured = || f.local.snapshot().models[0].manifest.measured.clone();
    wait_for("measured again", || {
        measured().is_some_and(|m| m.ram == 2 * G)
    })
    .await;
    assert_eq!(measured().unwrap().vram, 3 * G);
    assert_eq!(store::list(&f.root)[0].manifest.measured, measured());
}

/// 啟動後第一次量完硬體（GPU 偵測過）才記實測。
async fn detect(f: &Fixture) {
    f.local.start(&local_config(&[], json!({})));
    wait_for("first measurement", || !f.local.snapshot().detecting).await;
}

/// 重新偵測並等它量完（`own` 變成 `want`）。
async fn redetect_until(f: &Fixture, want: Usage) {
    f.local.command(Command::RefreshHardware {
        redetect_gpu: false,
    });
    wait_for("redetected", || f.local.snapshot().hardware.own == want).await;
}

/// GPU 計數器讀不到、working set 讀不到、獨顯全放 GPU 卻量到 0 VRAM、模型睡眠中：都只量不記。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn incomplete_readings_are_not_recorded() {
    const G: u64 = 1 << 30;
    let f = fixture(engine_and_model(false), |_| false).await;
    detect(&f).await;
    *f.shared.all_on_gpu.lock().unwrap() = Some(true);
    *f.shared.gpu_unreadable.lock().unwrap() = true;
    let usage = Usage {
        vram: 3 * G,
        ram: G,
    };
    *f.shared.usage_per_server.lock().unwrap() = usage;
    f.local.ensure("m1").await.unwrap();
    let measured = || store::list(&f.root)[0].manifest.measured.clone();
    assert_eq!(measured(), None, "GPU counters unreadable");
    *f.shared.gpu_unreadable.lock().unwrap() = false;
    for usage in [
        Usage { vram: 0, ram: G },
        Usage {
            vram: 3 * G,
            ram: 0,
        },
    ] {
        *f.shared.usage_per_server.lock().unwrap() = usage;
        redetect_until(&f, usage).await;
        assert_eq!(measured(), None, "{usage:?}");
    }
    *f.shared.unloaded.lock().unwrap() = true;
    let usage = Usage {
        vram: G / 2,
        ram: G,
    };
    *f.shared.usage_per_server.lock().unwrap() = usage;
    redetect_until(&f, usage).await;
    assert_eq!(measured(), None, "sleeping");
}

/// 拿不到啟動鎖（啟動中或正要刪檔）時只量不記。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_busy_start_lock_measures_without_recording() {
    const G: u64 = 1 << 30;
    let f = fixture(engine_and_model(false), |_| false).await;
    detect(&f).await;
    *f.shared.all_on_gpu.lock().unwrap() = Some(true);
    *f.shared.usage_per_server.lock().unwrap() = Usage {
        vram: 3 * G,
        ram: G,
    };
    f.local.ensure("m1").await.unwrap();
    let bigger = Usage {
        vram: 4 * G,
        ram: 2 * G,
    };
    *f.shared.usage_per_server.lock().unwrap() = bigger;
    let held = f.local.hold_start_for_test("m1").await;
    redetect_until(&f, bigger).await;
    drop(held);
    let measured = store::list(&f.root)[0].manifest.measured.clone().unwrap();
    assert_eq!((measured.vram, measured.ram), (3 * G, G));
}

/// 換了載入方式（這裡是從全放 GPU 變部分卸載）就整筆重記，不跟上一筆各取最大值。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_mode_change_replaces_the_whole_record() {
    const G: u64 = 1 << 30;
    let f = fixture(engine_and_model(false), |_| false).await;
    detect(&f).await;
    *f.shared.all_on_gpu.lock().unwrap() = Some(true);
    *f.shared.usage_per_server.lock().unwrap() = Usage {
        vram: 3 * G,
        ram: G,
    };
    f.local.ensure("m1").await.unwrap();
    f.local.stop("m1");
    *f.shared.all_on_gpu.lock().unwrap() = Some(false);
    *f.shared.usage_per_server.lock().unwrap() = Usage {
        vram: 2 * G,
        ram: 4 * G,
    };
    f.local.ensure("m1").await.unwrap();
    let measured = store::list(&f.root)[0].manifest.measured.clone().unwrap();
    assert_eq!(
        (measured.vram, measured.ram, measured.all_on_gpu),
        (2 * G, 4 * G, false)
    );
}

/// repo 的量化表跟著已下載模型的實測走：同一個 MTP 檔的那欄是實測，其他量化由它推算；記下實測與刪除模型後都重算。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn repo_ratings_follow_measurements() {
    const G: u64 = 1 << 30;
    const REPO: &str = "org/Model-GGUF";
    let id = store::model_id(REPO, "Q4_K_M");
    let f = fixture(
        |root| {
            add_engine(root, "b11871");
            add_model(root, &store::model_id(REPO, "Q4_K_M"), true);
        },
        |_| false,
    )
    .await;
    let body = tiny_gguf();
    Mock::given(method("GET"))
        .and(path("/api/models/org/Model-GGUF/tree/main"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"type": "file", "path": "model-Q4_K_M.gguf", "size": body.len()},
            {"type": "file", "path": "model-Q8_0.gguf", "size": 8},
            {"type": "file", "path": "mtp-Q8_0.gguf", "size": 2}
        ])))
        .mount(&f.hf)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
        .mount(&f.hf)
        .await;
    detect(&f).await;
    f.local.command(Command::QueryRepo(REPO.into()));
    let info = || match f.local.snapshot().repos.remove(REPO) {
        Some(RepoState::Ready(info)) => Some(info),
        _ => None,
    };
    wait_for("repo ready", || info().is_some()).await;
    let quant = |label: &str| {
        let info = info().unwrap();
        let i = info
            .catalog
            .choices
            .iter()
            .position(|c| c.quant.label == label);
        info.fits[i.unwrap()].clone()
    };
    assert_eq!(quant("Q4_K_M")[1].need.basis, Basis::Estimate);
    *f.shared.all_on_gpu.lock().unwrap() = Some(true);
    *f.shared.usage_per_server.lock().unwrap() = Usage {
        vram: 3 * G,
        ram: G,
    };
    f.local.ensure(&id).await.unwrap();
    wait_for("re-rated", || {
        quant("Q4_K_M")[1].need.basis == Basis::Measured
    })
    .await;
    let q4 = quant("Q4_K_M");
    assert_eq!((q4[1].need.vram, q4[1].need.ram), (3 * G, G));
    assert_eq!(
        q4[0].need.basis,
        Basis::Derived,
        "measured with MTP; the column without it is derived"
    );
    let q8 = quant("Q8_0");
    assert_eq!(q8[1].need.basis, Basis::Derived);
    // 權重：Q4 主檔 4＋MTP 2 位元組；Q8 主檔 8＋MTP 2
    assert_eq!(q8[1].need.vram, 3 * G + 4);
    f.local.command(Command::Delete(id.clone()));
    wait_for("re-rated after delete", || {
        quant("Q8_0")[1].need.basis == Basis::Estimate
    })
    .await;
}

/// 開 MTP 啟動失敗、退回不帶 MTP 跑時，相容判斷照 runtime 實際的方式（不帶 MTP）比對實測。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_rating_follows_the_mtp_mode_actually_running() {
    const G: u64 = 1 << 30;
    let f = fixture(engine_and_model(true), |p| p.mtp != Mtp::Off).await;
    detect(&f).await;
    *f.shared.all_on_gpu.lock().unwrap() = Some(true);
    *f.shared.usage_per_server.lock().unwrap() = Usage {
        vram: 3 * G,
        ram: G,
    };
    f.local.ensure("m1").await.unwrap();
    let s = f.local.snapshot();
    assert!(s.mtp_fallback.contains("m1"), "{s:?}");
    let need = s.model_need(&s.models[0], &LocalSettings::default());
    assert_eq!((need.basis, need.vram), (Basis::Measured, 3 * G));
}

/// 設定關掉 MTP 跑的不算退回：設定改回開著時，關 MTP 時量的實測只拿來推算。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mtp_turned_off_by_the_setting_is_not_a_fallback() {
    const G: u64 = 1 << 30;
    let f = fixture(engine_and_model(true), |_| false).await;
    detect(&f).await;
    f.local
        .apply_config(&local_config(&["m1"], json!({"mtp": {"m1": false}})));
    *f.shared.all_on_gpu.lock().unwrap() = Some(true);
    *f.shared.usage_per_server.lock().unwrap() = Usage {
        vram: 3 * G,
        ram: G,
    };
    f.local.ensure("m1").await.unwrap();
    let s = f.local.snapshot();
    assert!(
        s.running.contains("m1") && s.mtp_fallback.is_empty(),
        "{s:?}"
    );
    assert_eq!(
        s.models[0].manifest.measured.as_ref().map(|m| m.mtp),
        Some(false)
    );
    let need = s.model_need(&s.models[0], &LocalSettings::default());
    assert_eq!(need.basis, Basis::Derived, "MTP back on");
}

/// 加上實測欄位前寫的清單照樣讀得到，只是沒有實測；RAM 含對映分頁的舊實測（`measured` 鍵）作廢。
#[test]
fn manifests_without_a_measurement_still_load() {
    let dir = tempfile::tempdir().unwrap();
    let folder = store::models_dir(dir.path()).join("old");
    fs::create_dir_all(&folder).unwrap();
    let old = json!({
        "repo": "org/Model-GGUF",
        "quant": "Q4_K_M",
        "files": [{"path": "m.gguf", "size": 1, "sha256": null}],
        "mtp": null,
        "moe": false,
        "has_builtin_mtp": false,
        "required": 1,
        "measured": {"vram": 1, "ram": 9, "all_on_gpu": true, "mtp": false, "engine": ""}
    });
    fs::write(folder.join("manifest.json"), old.to_string()).unwrap();
    let list = store::list(dir.path());
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].manifest.measured, None);
}

/// 量硬體途中移除引擎：等 `--list-devices` 結束才刪，不會刪到執行中的 exe。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn engine_removal_waits_for_a_running_measurement() {
    let f = fixture(engine_and_model(false), |_| false).await;
    *f.shared.devices.lock().unwrap() = Some(FAKE_DEVICES.into());
    *f.shared.measure_delay.lock().unwrap() = Duration::from_millis(400);
    f.local
        .command(Command::RefreshHardware { redetect_gpu: true });
    tokio::time::sleep(Duration::from_millis(100)).await;
    f.local.command(Command::RemoveEngine);
    wait_for("engine removed", || {
        f.local.snapshot().engine == EngineState::Missing
    })
    .await;
    let measured = f.shared.measured.lock().unwrap().clone();
    assert_eq!(measured.len(), 1);
    assert!(measured[0].1, "the exe was deleted during --list-devices");
}

/// 新引擎上起了模型，但舊引擎上還有模型在跑：先不刪舊資料夾；新引擎失敗時也不回退刪掉別人在用的新資料夾。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn engine_folders_in_use_are_not_deleted() {
    let f = fixture(
        |root| {
            add_engine(root, "b11871");
            for id in ["m0", "m1", "m2"] {
                add_model(root, id, false);
            }
        },
        |p| p.model_id == "m1",
    )
    .await;
    let old = engine::read_active(&f.root).current.unwrap();
    f.local.ensure("m0").await.unwrap();
    let new = add_engine(&f.root, "b11872");
    f.local.ensure("m2").await.unwrap();
    assert!(engine::engine_dir(&f.root, &old).exists());
    assert_eq!(engine::read_active(&f.root).previous, Some(old.clone()));

    f.local.ensure("m1").await.unwrap_err();
    let active = engine::read_active(&f.root);
    assert_eq!(active.current, Some(new.clone()));
    assert!(engine::engine_dir(&f.root, &new).exists());

    f.local.stop("m0");
    f.local.stop("m2");
    f.local.ensure("m2").await.unwrap();
    assert!(!engine::engine_dir(&f.root, &old).exists());
    assert_eq!(engine::read_active(&f.root).previous, None);
}

/// GPU 只在啟動時偵測一次：裝引擎（挑變體、裝完重量）沿用它，使用者按〔重新偵測〕才重新列舉。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_gpu_is_detected_once_until_the_user_redetects() {
    let f = fixture(|_| {}, |_| false).await;
    *f.shared.devices.lock().unwrap() = Some(FAKE_DEVICES.into());
    mount_engine(&f.hf, Duration::ZERO).await;
    let detections = || *f.shared.gpu_detections.lock().unwrap();
    f.local.start(&default_config());
    wait_for("startup measurement", || {
        f.local.snapshot().hardware.gpu.is_some()
    })
    .await;
    f.local.command(Command::InstallEngine {
        build: None,
        variant: None,
    });
    wait_for("engine installed and measured", || {
        let s = f.local.snapshot();
        matches!(s.engine, EngineState::Ready(_)) && s.hardware.measured
    })
    .await;
    assert_eq!(detections(), 1);
    f.local
        .command(Command::RefreshHardware { redetect_gpu: true });
    wait_for("re-detected", || detections() == 2).await;
}

/// 停掉一個要等很久才結束的子程序時，讀快照不會被它卡住。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_slow_stop_does_not_block_snapshots() {
    let f = fixture(engine_and_model(false), |_| false).await;
    f.local.ensure("m1").await.unwrap();
    *f.shared.kill_delay.lock().unwrap() = Duration::from_millis(800);
    let local = f.local.clone();
    let stop = tokio::task::spawn_blocking(move || local.stop("m1"));
    tokio::time::sleep(Duration::from_millis(200)).await;
    let started = Instant::now();
    let s = f.local.snapshot();
    let waited = started.elapsed();
    assert!(
        waited < Duration::from_millis(400),
        "snapshot waited {waited:?}"
    );
    assert!(s.running.is_empty(), "{:?}", s.running);
    stop.await.unwrap();
    assert_eq!(f.kills().len(), 1);
}

/// 舊引擎上還有一個啟動卡在 launch 途中：新引擎上的模型就緒時不刪舊資料夾。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_start_still_launching_keeps_its_engine_folder() {
    let f = fixture(
        |root| {
            add_engine(root, "b11871");
            add_model(root, "m0", false);
            add_model(root, "m2", false);
        },
        |_| false,
    )
    .await;
    let old = engine::read_active(&f.root).current.unwrap();
    *f.shared.launch_delay.lock().unwrap() = Duration::from_millis(800);
    let local = f.local.clone();
    let slow = tokio::spawn(async move { local.ensure("m0").await });
    wait_for("m0 launching", || f.plans().len() == 1).await;
    *f.shared.launch_delay.lock().unwrap() = Duration::ZERO;
    add_engine(&f.root, "b11872");
    f.local.ensure("m2").await.unwrap();
    assert!(engine::engine_dir(&f.root, &old).exists());
    assert_eq!(engine::read_active(&f.root).previous, Some(old));
    slow.await.unwrap().unwrap();
}

/// 停掉的子程序還沒結束前不會啟動新的：舊程序的 VRAM 要先放掉。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_new_start_waits_for_the_stopped_server_to_exit() {
    let f = fixture(engine_and_model(false), |_| false).await;
    f.local.ensure("m1").await.unwrap();
    *f.shared.kill_delay.lock().unwrap() = Duration::from_millis(500);
    let local = f.local.clone();
    let stop = tokio::task::spawn_blocking(move || local.stop("m1"));
    tokio::time::sleep(Duration::from_millis(100)).await;
    f.local.ensure("m1").await.unwrap();
    stop.await.unwrap();
    assert_eq!(f.kills().len(), 1);
    let launched = f.shared.launched_at.lock().unwrap().clone();
    assert_eq!(launched.len(), 2);
    assert!(
        launched[1] - launched[0] >= Duration::from_millis(500),
        "restarted {:?} after the first start",
        launched[1] - launched[0]
    );
}

/// 每次通知當下讀一次快照：UI 收到通知時看到的就是這些。
struct Observed {
    _dir: TempDir,
    hf: MockServer,
    local: Arc<Local>,
    seen: Arc<Mutex<Vec<Snapshot>>>,
}

async fn observed(prepare: impl FnOnce(&Path), shared: Shared) -> Observed {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    prepare(&root);
    let hf = MockServer::start().await;
    let hub: Arc<OnceLock<Weak<Local>>> = Arc::default();
    let seen: Arc<Mutex<Vec<Snapshot>>> = Arc::default();
    let (cell, record) = (hub.clone(), seen.clone());
    let local = Local::new(
        root.clone(),
        Handle::current(),
        Arc::new(FakeLauncher {
            shared,
            fail_when: Box::new(|_| false),
            model_dir: store::models_dir(&root).join("m1"),
        }),
        Endpoints {
            hf: hf.uri(),
            releases: format!("{}/releases", hf.uri()),
        },
        Arc::new(move |_| {
            if let Some(local) = cell.get().and_then(Weak::upgrade) {
                record.lock().unwrap().push(local.snapshot());
            }
        }),
    );
    hub.set(Arc::downgrade(&local)).unwrap();
    Observed {
        _dir: dir,
        hf,
        local,
        seen,
    }
}

/// 偵測結束時一定再通知一次：最後一則通知當下讀到的已經不是偵測中，UI 的佔位才會收起。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_end_of_detection_is_notified() {
    let shared = Shared::default();
    *shared.devices.lock().unwrap() = Some(FAKE_DEVICES.into());
    let o = observed(engine_and_model(false), shared).await;
    o.local.start(&local_config(&[], json!({})));
    wait_for("first measurement", || !o.local.snapshot().detecting).await;
    let seen: Vec<bool> = o.seen.lock().unwrap().iter().map(|s| s.detecting).collect();
    assert!(seen.contains(&true), "{seen:?}");
    assert_eq!(seen.last(), Some(&false), "{seen:?}");
}

/// 解壓得久時快照會報「解壓中」：解壓不能卡住回報進度的那個 task。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_snapshot_reports_extraction() {
    let o = observed(|_| {}, Shared::default()).await;
    // 解得出 256 MiB：解壓要跨好幾個回報進度的間隔（64 MiB 約 0.25 秒，剛好一個間隔，偶爾整段漏掉）
    mount_engine_zip(&o.hf, Duration::ZERO, engine_zip(&vec![0u8; 256 << 20])).await;
    o.local.command(Command::InstallEngine {
        build: None,
        variant: None,
    });
    wait_for("engine ready", || {
        matches!(o.local.snapshot().engine, EngineState::Ready(_))
    })
    .await;
    let extracting = o.seen.lock().unwrap().iter().any(|s| {
        matches!(
            s.engine,
            EngineState::Installing {
                extracting: true,
                ..
            }
        )
    });
    assert!(extracting, "no notice showed the extraction");
}
