//! 本地服務 client：經 hub 確保 llama-server 在跑再轉送（假 Launcher、wiremock 當 llama-server）。

use crate::common::local::{Launches, add_engine, add_model, fake_hub};
use crate::stream_server::{bodies_at, request};

use std::sync::Arc;

use serde_json::json;
use tempfile::TempDir;
use tokio::runtime::Handle;
use tokio::sync::MutexGuard;
use wizard101_chat_translator::config::{Config, ServiceSpec};
use wizard101_chat_translator::local::{self, Local, store};
use wizard101_chat_translator::translation::Turn;
use wizard101_chat_translator::translation::clients::{ChatBackend, Client, build_client};
use wizard101_chat_translator::translation::errors::TranslateError;

/// hub 是全程序共用的：裝上測試 hub 的測試一次只跑一個。
static HUB: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct Hub {
    _dir: TempDir,
    local: Arc<Local>,
    launches: Launches,
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
    let (local, launches) = fake_hub(root, Handle::current(), fail);
    local::replace_for_test(local.clone());
    Hub {
        _dir: dir,
        local,
        launches,
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
    let servers = std::mem::take(&mut *h.launches.servers.lock().unwrap());
    assert_eq!(servers.len(), 2);
    for server in &servers {
        // 兩台各收到一次：第二句沒打到舊位址
        let bodies = bodies_at(server, "/v1/chat/completions").await;
        assert_eq!(bodies.len(), 1);
        assert_eq!(bodies[0]["model"], "m1");
        assert_eq!(bodies[0]["stream_options"], json!({"include_usage": true}));
    }
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
    assert_eq!(h.launches.started(), 0);
}
