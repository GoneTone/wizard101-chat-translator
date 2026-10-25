//! 本地模型 hub：子程序管理與下載（假 Launcher、wiremock 當 `/health` 與 Hugging Face，不啟動真程序）。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::json;
use tempfile::TempDir;
use tokio::runtime::Handle;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use wizard101_chat_translator::config::default_config;
use wizard101_chat_translator::local::engine::{self, EngineInstall, Variant};
use wizard101_chat_translator::local::server::{Launcher, Mtp, Process, ServerPlan};
use wizard101_chat_translator::local::store::{self, Manifest, ManifestFile};
use wizard101_chat_translator::local::{
    Command, DownloadState, Endpoints, Local, LocalNotice, RepoState,
};

type Events = Arc<Mutex<Vec<String>>>;
type FailWhen = Box<dyn Fn(&ServerPlan) -> bool + Send + Sync>;

struct FakeProcess {
    exit: Option<i32>,
    events: Events,
    model_dir: PathBuf,
    _health: Option<MockServer>,
}

impl Process for FakeProcess {
    fn exited(&mut self) -> Option<i32> {
        self.exit
    }
    fn kill(&mut self) {
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

struct FakeLauncher {
    plans: Arc<Mutex<Vec<ServerPlan>>>,
    events: Events,
    fail_when: FailWhen,
    model_dir: PathBuf,
}

impl Launcher for FakeLauncher {
    fn launch(&self, plan: &ServerPlan, port: u16) -> io::Result<Box<dyn Process>> {
        self.plans.lock().unwrap().push(plan.clone());
        let fail = (self.fail_when)(plan);
        Ok(Box::new(FakeProcess {
            exit: fail.then_some(1),
            events: self.events.clone(),
            model_dir: self.model_dir.clone(),
            _health: (!fail).then(|| health_server(port)),
        }))
    }
}

/// 在 llama-server 該聽的埠上開一個 `/health` 回 200 的 wiremock。
fn health_server(port: u16) -> MockServer {
    let listener = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
    let rt = Handle::current();
    tokio::task::block_in_place(|| {
        rt.block_on(async {
            let server = MockServer::builder().listener(listener).start().await;
            Mock::given(method("GET"))
                .and(path("/health"))
                .respond_with(ResponseTemplate::new(200))
                .mount(&server)
                .await;
            server
        })
    })
}

fn add_engine(root: &Path, build: &str) -> EngineInstall {
    let install = EngineInstall {
        build: build.into(),
        variant: Variant::Cpu,
    };
    let dir = engine::engine_dir(root, &install);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("llama-server.exe"), b"").unwrap();
    engine::activate(root, install.clone()).unwrap();
    install
}

fn add_model(root: &Path, id: &str, with_mtp: bool) {
    let file = |path: &str, size| ManifestFile {
        path: path.into(),
        size,
        sha256: None,
    };
    let manifest = Manifest {
        repo: "org/Model-GGUF".into(),
        quant: "Q4_K_M".into(),
        files: vec![file("model-Q4_K_M.gguf", 4)],
        mtp: with_mtp.then(|| file("mtp-Q8_0.gguf", 2)),
        moe: false,
        has_builtin_mtp: false,
        required: 1,
    };
    store::write_manifest(root, id, &manifest).unwrap();
    let dir = store::models_dir(root).join(id);
    fs::write(dir.join("model-Q4_K_M.gguf"), b"main").unwrap();
    if with_mtp {
        fs::write(dir.join("mtp-Q8_0.gguf"), b"mt").unwrap();
    }
}

struct Fixture {
    _dir: TempDir,
    root: PathBuf,
    hf: MockServer,
    local: Arc<Local>,
    plans: Arc<Mutex<Vec<ServerPlan>>>,
    events: Events,
    notices: Arc<Mutex<Vec<LocalNotice>>>,
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
    let plans = Arc::new(Mutex::new(Vec::new()));
    let events: Events = Arc::default();
    let notices = Arc::new(Mutex::new(Vec::new()));
    let sink = notices.clone();
    let local = Local::new(
        root.clone(),
        Handle::current(),
        Arc::new(FakeLauncher {
            plans: plans.clone(),
            events: events.clone(),
            fail_when: Box::new(fail_when),
            model_dir: store::models_dir(&root).join("m1"),
        }),
        Endpoints {
            hf: hf.uri(),
            releases: hf.uri(),
        },
        Arc::new(move |n| sink.lock().unwrap().push(n)),
    );
    Fixture {
        _dir: dir,
        root,
        hf,
        local,
        plans,
        events,
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
    assert_eq!(f.plans.lock().unwrap().len(), 1);
}

/// 帶 MTP 啟動失敗時關掉 MTP 重試一次。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mtp_failure_retries_without_mtp() {
    let f = fixture(engine_and_model(true), |p| p.mtp != Mtp::Off).await;
    f.local.ensure("m1").await.unwrap();
    let plans = f.plans.lock().unwrap();
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
    assert_eq!(active.current.unwrap().build, "b11871");
    assert_eq!(active.previous, None);
    let plans = f.plans.lock().unwrap();
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
    assert_eq!(f.plans.lock().unwrap().len(), 2);
}

/// 沒有任何用途用到的模型會被停掉。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn apply_config_stops_models_no_slot_uses() {
    let f = fixture(engine_and_model(false), |_| false).await;
    f.local.ensure("m1").await.unwrap();
    f.local.apply_config(&default_config());
    assert_eq!(
        *f.events.lock().unwrap(),
        ["kill model_dir_exists=true".to_string()]
    );
}

/// 刪除模型前先停掉它的子程序。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn delete_stops_the_server_before_removing_files() {
    let f = fixture(engine_and_model(false), |_| false).await;
    f.local.ensure("m1").await.unwrap();
    f.local.command(Command::Delete("m1".into()));
    let dir = store::models_dir(&f.root).join("m1");
    wait_for("model folder removed", || !dir.exists()).await;
    assert_eq!(
        *f.events.lock().unwrap(),
        ["kill model_dir_exists=true".to_string()]
    );
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
