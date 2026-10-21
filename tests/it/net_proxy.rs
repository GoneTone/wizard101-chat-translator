//! 代理設定：非本機網址走設定的代理、本機網址直連。
//!
//! 環境變數是整個程序共用的，改了會波及平行跑的其他測試；所以把真正的請求放到帶著代理變數的子程序裡跑。

use crate::common;
use crate::stream_server::request;

use common::log_capture::{capture_logs, log_text};

use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use wizard101_chat_translator::translation::Turn;
use wizard101_chat_translator::translation::clients::OpenAiCompatClient;

const CHILD_MARKER: &str = "W101_PROXY_TEST_CHILD";
const CHILD_TEST: &str = "net_proxy::child_requests_through_the_env_proxy";
const UPSTREAM_HOST: &str = "upstream.invalid";

async fn mount_reply(server: &MockServer, text: &str) {
    let chunk = json!({"choices": [{"delta": {"content": text}, "finish_reason": "stop"}]});
    let body = format!("data: {chunk}\n\ndata: [DONE]\n\n");
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
        .mount(server)
        .await;
}

async fn chat(base_url: &str) -> String {
    let client = OpenAiCompatClient::new(base_url, "", "m", true, false);
    let turns = [Turn::user("[A] hi")];
    client.chat(request(&turns), None).await.unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn non_loopback_requests_go_through_the_configured_proxy() {
    let proxy = MockServer::start().await;
    mount_reply(&proxy, "via proxy").await;
    let exe = std::env::current_exe().unwrap();
    let proxy_url = proxy.uri();
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new(exe)
            .args([CHILD_TEST, "--exact", "--ignored", "--nocapture"])
            .env(CHILD_MARKER, "1")
            .env("HTTP_PROXY", proxy_url)
            // 非空的 NO_PROXY 讓開發機登錄檔裡的代理例外清單不套用進來
            .env("NO_PROXY", "no-proxy.test")
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let received = proxy.received_requests().await.unwrap();
    let hosts: Vec<_> = received.iter().map(|r| r.url.host_str()).collect();
    assert_eq!(hosts, [Some(UPSTREAM_HOST)], "本機網址的請求不該經過代理");
}

#[tokio::test]
#[ignore = "run in a child process by non_loopback_requests_go_through_the_configured_proxy"]
async fn child_requests_through_the_env_proxy() {
    if std::env::var_os(CHILD_MARKER).is_none() {
        return;
    }
    assert_eq!(chat(&format!("http://{UPSTREAM_HOST}")).await, "via proxy");
    let local = MockServer::start().await;
    mount_reply(&local, "direct").await;
    assert_eq!(chat(&local.uri()).await, "direct");
}

#[tokio::test]
async fn loopback_clients_log_that_no_proxy_is_used() {
    capture_logs();
    let url = crate::stream_server::closed_url().await;
    let _client = OpenAiCompatClient::new(&url, "", "m", true, false);
    let line = format!("[net] {url} is a loopback address; not using a proxy");
    assert!(log_text().contains(&line), "{line}");
}
