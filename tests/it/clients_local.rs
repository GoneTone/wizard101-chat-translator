//! 本地服務 client：經 hub 確保 llama-server 在跑再轉送（假 Launcher、wiremock 當 llama-server）。

use crate::common::local::{GlobalHub, Launches, add_engine, add_model, fake_hub_with, global_hub};
use crate::stream_server::{bodies_at, request};

use std::sync::{Arc, Mutex};

use serde_json::json;
use tempfile::TempDir;
use tokio::runtime::Handle;
use wizard101_chat_translator::config::{Config, ServiceSpec};
use wizard101_chat_translator::local::{self, Local, LocalNotice, store};
use wizard101_chat_translator::translation::Turn;
use wizard101_chat_translator::translation::clients::{
    ChatBackend, Client, LocalClient, build_client,
};
use wizard101_chat_translator::translation::errors::TranslateError;

struct Hub {
    _dir: TempDir,
    local: Arc<Local>,
    launches: Launches,
    notices: Arc<Mutex<Vec<LocalNotice>>>,
    _global: GlobalHub,
}

impl Hub {
    fn loading_notices(&self) -> Vec<LocalNotice> {
        let notices = self.notices.lock().unwrap();
        notices
            .iter()
            .filter(|n| matches!(n, LocalNotice::Loading(_)))
            .cloned()
            .collect()
    }
}

/// 建一個有引擎、下載完成的 m1 與下載到一半的 m2 的 hub，裝成全程序共用的那個。
async fn hub(fail: bool) -> Hub {
    let global = global_hub().await;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    add_engine(&root, "b11871");
    add_model(&root, "m1", false);
    add_model(&root, "m2", false);
    std::fs::remove_file(store::models_dir(&root).join("m2/model-Q4_K_M.gguf")).unwrap();
    let notices = Arc::new(Mutex::new(Vec::new()));
    let sink = notices.clone();
    let notify = Arc::new(move |n| sink.lock().unwrap().push(n));
    let (local, launches) = fake_hub_with(root, Handle::current(), fail, notify);
    local::install(local.clone());
    Hub {
        _dir: dir,
        local,
        launches,
        notices,
        _global: global,
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
    let mut cfg = Config::default();
    cfg.insert("local_models".into(), json!({"preload": true}));
    h.local.apply_config(&cfg);
    assert!(h.local.settings().preload);
    local_client("m1").warm_up("test").await;
    assert_eq!(h.launches.started(), 0);
}

/// 閒置睡著的模型：請求前查 `/props`，重新載入期間送載入橫幅，譯文回來後收起。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_sleeping_server_shows_the_loading_banner_while_it_reloads() {
    let h = hub(false).await;
    *h.launches.sleeping.lock().unwrap() = Some(true);
    h.local.ensure("m1").await.unwrap();
    h.notices.lock().unwrap().clear();
    assert_eq!(chat(&local_client("m1")).await, Ok("hola".into()));
    assert_eq!(
        h.loading_notices(),
        [
            LocalNotice::Loading(Some("org/Model-GGUF · Q4_K_M".into())),
            LocalNotice::Loading(None)
        ]
    );
}

/// 沒睡著，或 `/props` 查不到（404）時不送橫幅，翻譯照常。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_awake_or_unknown_server_shows_no_banner() {
    for sleeping in [Some(false), None] {
        let h = hub(false).await;
        *h.launches.sleeping.lock().unwrap() = sleeping;
        h.local.ensure("m1").await.unwrap();
        h.notices.lock().unwrap().clear();
        let translated = chat(&local_client("m1")).await;
        assert_eq!(translated, Ok("hola".into()), "{sleeping:?}");
        assert_eq!(h.loading_notices(), [], "{sleeping:?}");
    }
}

/// 測試連線的載入計時：沒在跑時要啟動、睡著時送 1 個 token 的請求叫醒，都回傳花的時間；醒著時不算載入。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn load_times_a_start_or_a_wake_but_not_an_awake_server() {
    let h = hub(false).await;
    *h.launches.sleeping.lock().unwrap() = Some(false);
    let client = LocalClient::new("m1", false);
    assert!(client.load(None).await.unwrap().is_some(), "started");
    assert_eq!(h.launches.started(), 1);
    assert_eq!(client.load(None).await, Ok(None), "awake");
    drop(h);

    let h = hub(false).await;
    *h.launches.sleeping.lock().unwrap() = Some(true);
    h.local.ensure("m1").await.unwrap();
    h.notices.lock().unwrap().clear();
    assert!(client.load(None).await.unwrap().is_some(), "woken");
    let servers = std::mem::take(&mut *h.launches.servers.lock().unwrap());
    let bodies = bodies_at(&servers[0], "/v1/chat/completions").await;
    assert_eq!(bodies.len(), 1);
    assert_eq!(bodies[0]["max_tokens"], 1);
    assert_eq!(h.loading_notices().len(), 2, "banner while waking");
}

/// 測試結束放掉 hub 後，全程序共用的 hub 也一併卸下。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dropping_the_test_hub_uninstalls_it() {
    let h = hub(false).await;
    assert!(local::get().is_some());
    drop(h);
    let _global = global_hub().await;
    assert!(local::get().is_none());
}
