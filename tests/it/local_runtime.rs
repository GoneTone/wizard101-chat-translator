//! 本地模型 hub：子程序管理與下載（假 Launcher、wiremock 當 `/health` 與 Hugging Face，不啟動真程序）。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::common::local::{add_engine, add_model, serve_on};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::runtime::Handle;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use wizard101_chat_translator::config::{Config, default_config};
use wizard101_chat_translator::local::engine::{self, EngineInstall, Variant};
use wizard101_chat_translator::local::hardware::{Gpu, GpuKind};
use wizard101_chat_translator::local::server::{Launcher, Mtp, Process, ServerPlan};
use wizard101_chat_translator::local::store;
use wizard101_chat_translator::local::{
    Command, DownloadState, Endpoints, EngineState, Local, LocalNotice, RepoInfo, RepoState,
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
            _health: (!fail).then(|| health_server(port, delay)),
        }))
    }

    fn detect_gpu(&self) -> Option<Gpu> {
        *self.shared.gpu_detections.lock().unwrap() += 1;
        Some(Gpu {
            name: "Fake GPU".into(),
            vendor: 0x10de,
            kind: GpuKind::Discrete,
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
    assert_eq!(info.fits[0].1, None, "VRAM not measured yet");

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

fn engine_zip() -> Vec<u8> {
    use std::io::Write;
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(vec![]));
    w.start_file("llama-server.exe", zip::write::SimpleFileOptions::default())
        .unwrap();
    w.write_all(b"").unwrap();
    w.finish().unwrap().into_inner()
}

/// 掛上 b11871 的 releases 列表（延遲 `delay` 回應）與引擎 zip。
async fn mount_engine(hf: &MockServer, delay: Duration) {
    let zip = engine_zip();
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
    let (q4, _) = two_quants(&f).await;
    f.local.command(download(q4));
    let q8_id = store::model_id(REPO, "Q8_0");
    wait_complete(&f, &q8_id).await;
    let s = f.local.snapshot();
    assert!(matches!(s.engine, EngineState::Ready(_)), "{:?}", s.engine);
    assert_eq!(s.hardware.vram_free, 8000 << 20);
    assert_eq!(s.models.len(), 1, "{:?}", s.models);
    assert!(!model_dir(&f, &store::model_id(REPO, "Q4_K_M")).exists());
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

/// 模型跑著時重新偵測：它佔的記憶體加回可用量，快照也列出它在跑。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn redetecting_adds_running_models_back_to_free_memory() {
    const G: u64 = 1 << 30;
    let f = fixture(
        |root| {
            engine_and_model(false)(root);
            let mut m = store::list(root).remove(0).manifest;
            m.required = 2 * G;
            store::write_manifest(root, "m1", &m).unwrap();
        },
        |_| false,
    )
    .await;
    f.local.ensure("m1").await.unwrap();
    *f.shared.devices.lock().unwrap() =
        Some("Available devices:\n  Vulkan0: Fake GPU (8192 MiB, 4096 MiB free)\n".into());
    f.local.command(Command::RefreshHardware);
    wait_for("measured", || f.local.snapshot().hardware.measured).await;
    let s = f.local.snapshot();
    assert!(s.running.contains("m1"), "{:?}", s.running);
    assert_eq!(s.hardware.vram_free, (4096 << 20) + 2 * G);
}

/// 量硬體途中移除引擎：等 `--list-devices` 結束才刪，不會刪到執行中的 exe。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn engine_removal_waits_for_a_running_measurement() {
    let f = fixture(engine_and_model(false), |_| false).await;
    *f.shared.devices.lock().unwrap() = Some(FAKE_DEVICES.into());
    *f.shared.measure_delay.lock().unwrap() = Duration::from_millis(400);
    f.local.command(Command::RefreshHardware);
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
    f.local.command(Command::RefreshHardware);
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
