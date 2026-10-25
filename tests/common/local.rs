//! 本地模型測試共用：假的引擎與模型檔、在指定埠上開 wiremock 當 llama-server、用假 Launcher 的 hub。

// 每個測試執行檔各自編譯 common，不是每個都用到
#![allow(dead_code)]

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::json;
use tokio::runtime::Handle;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use wizard101_chat_translator::local::engine::{self, EngineInstall, Variant};
use wizard101_chat_translator::local::hardware::Gpu;
use wizard101_chat_translator::local::server::{Launcher, Process, ServerPlan};
use wizard101_chat_translator::local::store::{self, Manifest, ManifestFile};
use wizard101_chat_translator::local::{self, Endpoints, Local, Notify};

/// hub 是全程序共用的：裝上測試 hub（`local::install`）的測試一次只跑一個。
static GLOBAL_HUB: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 占住全程序共用 hub 的使用權；放掉時卸下測試裝的 hub，不留指向已結束 runtime 的 hub。
pub struct GlobalHub {
    _guard: tokio::sync::MutexGuard<'static, ()>,
}

impl Drop for GlobalHub {
    fn drop(&mut self) {
        local::uninstall_for_test();
    }
}

/// 等到可以裝全程序共用的 hub。
pub async fn global_hub() -> GlobalHub {
    GlobalHub {
        _guard: GLOBAL_HUB.lock().await,
    }
}

/// 假 llama-server 的紀錄：每次成功啟動的伺服器與 kill 次數。
#[derive(Clone, Default)]
pub struct Launches {
    /// kill 後也留著，測試才看得到每台收到的請求。
    pub servers: Arc<Mutex<Vec<MockServer>>>,
    pub kills: Arc<AtomicUsize>,
    /// 之後啟動的伺服器 `/props` 回的 `is_sleeping`；`None` 時不提供 `/props`（回 404）。
    pub sleeping: Arc<Mutex<Option<bool>>>,
}

impl Launches {
    pub fn started(&self) -> usize {
        self.servers.lock().unwrap().len()
    }

    pub fn killed(&self) -> usize {
        self.kills.load(Ordering::SeqCst)
    }
}

struct FakeProcess {
    exit: Option<i32>,
    kills: Arc<AtomicUsize>,
}

impl Process for FakeProcess {
    fn exited(&mut self) -> Option<i32> {
        self.exit
    }
    fn kill(&mut self) {
        self.kills.fetch_add(1, Ordering::SeqCst);
    }
    fn tail(&self) -> Vec<String> {
        vec!["boom".into()]
    }
}

/// `fail` 時子程序立刻以 1 結束（tail 為 `boom`）；否則在指定埠上開回 `/health` 與串流譯文 `hola` 的伺服器。
struct FakeLauncher {
    fail: bool,
    launches: Launches,
}

impl Launcher for FakeLauncher {
    fn launch(&self, _plan: &ServerPlan, port: u16) -> io::Result<Box<dyn Process>> {
        let kills = self.launches.kills.clone();
        if self.fail {
            return Ok(Box::new(FakeProcess {
                exit: Some(1),
                kills,
            }));
        }
        let stream = "data: {\"choices\":[{\"delta\":{\"content\":\"hola\"}}]}

data: [DONE]

";
        let mut mocks = vec![
            Mock::given(method("GET"))
                .and(path("/health"))
                .respond_with(ResponseTemplate::new(200)),
            Mock::given(method("POST"))
                .and(path("/v1/chat/completions"))
                .respond_with(ResponseTemplate::new(200).set_body_raw(stream, "text/event-stream")),
        ];
        if let Some(sleeping) = *self.launches.sleeping.lock().unwrap() {
            mocks.push(Mock::given(method("GET")).and(path("/props")).respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"is_sleeping": sleeping})),
            ));
        }
        let server = serve_on(port, mocks);
        self.launches.servers.lock().unwrap().push(server);
        Ok(Box::new(FakeProcess { exit: None, kills }))
    }

    fn detect_gpu(&self) -> Option<Gpu> {
        None
    }

    fn list_devices(&self, _exe: &Path) -> Result<String, String> {
        Err("fake launcher measures nothing".into())
    }
}

/// 以 `root` 建一個用假 Launcher 的 hub（不連外、不通知）；`ensure` 須在 multi-thread runtime 的 worker 上跑。
pub fn fake_hub(root: PathBuf, rt: Handle, fail: bool) -> (Arc<Local>, Launches) {
    fake_hub_with(root, rt, fail, Arc::new(|_| {}))
}

/// 同 `fake_hub`，通知送到 `notify`。
pub fn fake_hub_with(
    root: PathBuf,
    rt: Handle,
    fail: bool,
    notify: Notify,
) -> (Arc<Local>, Launches) {
    let launches = Launches::default();
    let unreachable = "http://127.0.0.1:9".to_owned();
    let local = Local::new(
        root,
        rt,
        Arc::new(FakeLauncher {
            fail,
            launches: launches.clone(),
        }),
        Endpoints {
            hf: unreachable.clone(),
            releases: unreachable,
        },
        notify,
    );
    (local, launches)
}

/// 在 llama-server 該聽的埠上開 wiremock 並掛上 `mocks`；須在 multi-thread runtime 裡呼叫。
pub fn serve_on(port: u16, mocks: Vec<Mock>) -> MockServer {
    let listener = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
    let rt = Handle::current();
    tokio::task::block_in_place(|| {
        rt.block_on(async {
            let server = MockServer::builder().listener(listener).start().await;
            for mock in mocks {
                mock.mount(&server).await;
            }
            server
        })
    })
}

/// 寫一個空的 `llama-server.exe` 並設為啟用中的引擎。
pub fn add_engine(root: &Path, build: &str) -> EngineInstall {
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

/// 寫一個下載完成的模型（`with_mtp` 時附 MTP 檔）。
pub fn add_model(root: &Path, id: &str, with_mtp: bool) {
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
