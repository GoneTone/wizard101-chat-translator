//! 本地服務 client：經 hub 確保 llama-server 在跑再轉送（假 Launcher、wiremock 當 llama-server）。

use crate::common::local::{add_engine, add_model, serve_on};
use crate::stream_server::request;

use std::io;
use std::sync::{Arc, Mutex};

use serde_json::json;
use tempfile::TempDir;
use tokio::runtime::Handle;
use tokio::sync::MutexGuard;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use wizard101_chat_translator::config::{Config, ServiceSpec};
use wizard101_chat_translator::local::server::{Launcher, Process, ServerPlan};
use wizard101_chat_translator::local::{self, Endpoints, Local, store};
use wizard101_chat_translator::translation::Turn;
use wizard101_chat_translator::translation::clients::{ChatBackend, Client, build_client};
use wizard101_chat_translator::translation::errors::TranslateError;

/// hub 是全程序共用的：裝上測試 hub 的測試一次只跑一個。
static HUB: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct FakeProcess {
    exit: Option<i32>,
    _server: Option<MockServer>,
}

impl Process for FakeProcess {
    fn exited(&mut self) -> Option<i32> {
        self.exit
    }
    fn kill(&mut self) {}
    fn tail(&self) -> Vec<String> {
        vec!["boom".into()]
    }
}

/// `fail` 時子程序立刻以 1 結束；否則在指定埠上回 `/health` 與 `model` 為 m1 的串流譯文。
struct FakeLauncher {
    fail: bool,
    ports: Arc<Mutex<Vec<u16>>>,
}

impl Launcher for FakeLauncher {
    fn launch(&self, _plan: &ServerPlan, port: u16) -> io::Result<Box<dyn Process>> {
        self.ports.lock().unwrap().push(port);
        if self.fail {
            return Ok(Box::new(FakeProcess {
                exit: Some(1),
                _server: None,
            }));
        }
        let stream = "data: {\"choices\":[{\"delta\":{\"content\":\"hola\"}}]}\n\ndata: [DONE]\n\n";
        let server = serve_on(
            port,
            vec![
                Mock::given(method("GET"))
                    .and(path("/health"))
                    .respond_with(ResponseTemplate::new(200)),
                Mock::given(method("POST"))
                    .and(path("/v1/chat/completions"))
                    .and(body_partial_json(json!({"model": "m1"})))
                    .respond_with(
                        ResponseTemplate::new(200).set_body_raw(stream, "text/event-stream"),
                    ),
            ],
        );
        Ok(Box::new(FakeProcess {
            exit: None,
            _server: Some(server),
        }))
    }
}

struct Hub {
    _dir: TempDir,
    local: Arc<Local>,
    ports: Arc<Mutex<Vec<u16>>>,
    _guard: MutexGuard<'static, ()>,
}

/// 建一個有引擎、下載完成的 m1 與下載到一半的 m2 的 hub，裝成全程序共用的那個。
async fn hub(fail: bool) -> Hub {
    let guard = HUB.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    add_engine(&root, "b11871");
    add_model(&root, "m1", false);
    add_model(&root, "m2", false);
    std::fs::remove_file(store::models_dir(&root).join("m2/model-Q4_K_M.gguf")).unwrap();
    let ports = Arc::new(Mutex::new(Vec::new()));
    let local = Local::new(
        root,
        Handle::current(),
        Arc::new(FakeLauncher {
            fail,
            ports: ports.clone(),
        }),
        Endpoints {
            hf: "http://127.0.0.1:9".into(),
            releases: "http://127.0.0.1:9".into(),
        },
        Arc::new(|_| {}),
    );
    local::replace_for_test(local.clone());
    Hub {
        _dir: dir,
        local,
        ports,
        _guard: guard,
    }
}

fn local_client(model: &str) -> Client {
    build_client(&ServiceSpec {
        provider: "local".into(),
        model: model.into(),
        ..Default::default()
    })
}

async fn chat(client: &Client) -> Result<String, TranslateError> {
    client.chat(request(&[Turn::user("[A] hi")]), None).await
}

#[test]
fn build_client_maps_local_provider() {
    let client = local_client("m1");
    assert!(matches!(client, Client::Local(_)));
    assert_eq!(client.model(), "m1");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn local_client_forwards_to_the_ensured_server() {
    let h = hub(false).await;
    let client = local_client("m1");
    assert_eq!(chat(&client).await, Ok("hola".into()));
    // 子程序換埠重啟後要改連新的位址
    h.local.stop("m1");
    assert_eq!(chat(&client).await, Ok("hola".into()));
    let ports = h.ports.lock().unwrap().clone();
    assert_eq!(ports.len(), 2);
    assert_ne!(ports[0], ports[1]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ensure_failure_is_a_config_error_with_the_original_text() {
    let _h = hub(true).await;
    match chat(&local_client("m1")).await {
        Err(TranslateError::Config {
            status: None,
            detail,
        }) => assert!(detail.contains("boom"), "{detail}"),
        other => panic!("expected a config error, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn list_models_returns_only_complete_downloads() {
    let _h = hub(false).await;
    let ids: Vec<String> = local_client("m1")
        .list_models()
        .await
        .unwrap()
        .into_iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(ids, ["m1"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn warm_up_launches_nothing_even_when_preloading() {
    let h = hub(false).await;
    h.local.apply_config(&Config::default());
    assert!(h.local.settings().preload);
    local_client("m1").warm_up("test").await;
    assert!(h.ports.lock().unwrap().is_empty());
}
