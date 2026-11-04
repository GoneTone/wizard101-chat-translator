//! 更新檢查：版本解析比較與 GitHub API 取用（wiremock，不打真 API）。

use crate::common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::log_capture::{capture_logs, log_text};
use serde_json::json;
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use wizard101_chat_translator::runtime::updater::{
    EXE_ASSET, Release, UpdateCheckError, check_for_update, is_newer, parse_releases,
    schedule_update_checks, update_client,
};

const SHA256: &str = "be76267b7f18060bab1fe7ca9d63822c84974f5aed16e4688b1c0de575e84d79";

type Found = Arc<dyn Fn(Vec<Release>) + Send + Sync>;
type Seen = Arc<Mutex<Vec<Vec<Release>>>>;

#[test]
fn is_newer_compares_numerically() {
    assert!(is_newer("0.2.0", "0.1.0"));
    assert!(is_newer("v0.10.0", "0.9.9"));
    assert!(!is_newer("0.1.0", "0.1.0"));
    assert!(!is_newer("0.1.0", "0.2.0"));
}

#[test]
fn is_newer_ranks_a_release_above_its_own_prereleases() {
    assert!(is_newer("0.2.0", "0.2.0-rc.1"));
    assert!(!is_newer("0.2.0-rc.1", "0.2.0"));
}

#[test]
fn is_newer_compares_numeric_prerelease_identifiers_as_numbers() {
    assert!(is_newer("0.2.0-rc.2", "0.2.0-rc.1"));
    assert!(is_newer("0.2.0-rc.10", "0.2.0-rc.9"));
}

#[test]
fn is_newer_ranks_alphanumeric_prerelease_identifiers_above_numeric_ones() {
    assert!(is_newer("0.2.0-alpha.beta", "0.2.0-alpha.1"));
    assert!(is_newer("0.2.0-beta", "0.2.0-alpha"));
    assert!(is_newer("0.2.0-rc.1", "0.2.0-rc"));
}

#[test]
fn build_metadata_does_not_affect_precedence() {
    assert!(!is_newer("0.2.0+build.2", "0.2.0+build.1"));
    assert!(!is_newer("0.2.0-rc.1+build.2", "0.2.0-rc.1"));
}

#[test]
fn is_newer_is_false_when_either_side_is_unparsable() {
    assert!(!is_newer("nightly", "0.1.0"));
    assert!(!is_newer("0.2.0", "unknown"));
}

fn release_item(tag: &str, extra: serde_json::Value) -> serde_json::Value {
    let mut item = json!({
        "tag_name": tag,
        "html_url": format!("https://github.com/GoneTone/wizard101-chat-translator/releases/tag/{tag}"),
        "published_at": "2026-11-03T11:58:24Z",
        "body": format!("notes for {tag}"),
        "draft": false,
        "prerelease": false,
        "assets": [{
            "name": EXE_ASSET,
            "browser_download_url": format!("https://example.invalid/{tag}/{EXE_ASSET}"),
            "size": 61598208,
            "digest": format!("sha256:{SHA256}"),
        }],
    });
    item.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    item
}

fn releases_json() -> serde_json::Value {
    json!([
        release_item("v1.1.0", json!({})),
        release_item("v9.0.0", json!({"draft": true})),
        release_item("v1.3.0-rc.1", json!({"prerelease": true})),
        release_item("v1.2.0", json!({})),
        release_item("v1.0.0", json!({})),
    ])
}

async fn server_with(response: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/releases"))
        .respond_with(response)
        .mount(&server)
        .await;
    server
}

fn releases_url(server: &MockServer) -> String {
    format!("{}/releases", server.uri())
}

async fn request_count(server: &MockServer) -> usize {
    server.received_requests().await.unwrap().len()
}

#[test]
fn parse_releases_keeps_newer_published_releases_newest_first() {
    let list = parse_releases(&releases_json(), "1.0.0").unwrap();
    let versions: Vec<_> = list.iter().map(|r| r.version.as_str()).collect();
    assert_eq!(versions, ["1.2.0", "1.1.0"]);
    let asset = list[0].asset.as_ref().unwrap();
    assert_eq!(asset.sha256, SHA256);
    assert_eq!(asset.size, 61598208);
    assert_eq!(
        asset.url,
        format!("https://example.invalid/v1.2.0/{EXE_ASSET}")
    );
    assert_eq!(list[0].published_at, "2026-11-03T11:58:24Z");
    assert_eq!(list[0].body, "notes for v1.2.0");
    assert!(list[0].url.ends_with("/releases/tag/v1.2.0"));
}

#[test]
fn parse_releases_drops_the_asset_without_a_digest() {
    let mut item = release_item("v1.2.0", json!({}));
    item["assets"][0].as_object_mut().unwrap().remove("digest");
    let list = parse_releases(&json!([item]), "1.0.0").unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].asset, None);
}

#[test]
fn parse_releases_ignores_other_assets() {
    let mut item = release_item("v1.2.0", json!({}));
    item["assets"].as_array_mut().unwrap().insert(
        0,
        json!({"name": "notes.txt", "browser_download_url": "https://example.invalid/notes.txt", "size": 1, "digest": "sha256:00"}),
    );
    let list = parse_releases(&json!([item]), "1.0.0").unwrap();
    assert!(list[0].asset.as_ref().unwrap().url.ends_with(EXE_ASSET));
}

#[test]
fn parse_releases_is_empty_when_current_is_latest() {
    assert!(
        parse_releases(&releases_json(), "1.2.0")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn parse_releases_rejects_a_non_array() {
    let err = parse_releases(&json!({}), "1.0.0").unwrap_err();
    assert!(err.0.contains("unexpected response"), "{err}");
}

#[test]
fn parse_releases_skips_an_item_without_tag_name() {
    let items = json!([{"body": "x"}, release_item("v1.2.0", json!({}))]);
    assert_eq!(parse_releases(&items, "1.0.0").unwrap().len(), 1);
}

#[tokio::test]
async fn check_for_update_reads_the_list() {
    capture_logs();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/releases"))
        .and(header("Accept", "application/vnd.github+json"))
        .and(header(
            "User-Agent",
            format!("wizard101-chat-translator/{}", env!("CARGO_PKG_VERSION")).as_str(),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(releases_json()))
        .expect(1)
        .mount(&server)
        .await;
    let client = update_client("http://127.0.0.1");
    let list = check_for_update(&client, &releases_url(&server), "1.0.0")
        .await
        .unwrap();
    assert_eq!(list.len(), 2);
    assert!(log_text().contains("[update] newer releases: count=2 latest=1.2.0"));
}

#[tokio::test]
async fn check_for_update_is_empty_when_current_is_latest() {
    let server = server_with(ResponseTemplate::new(200).set_body_json(releases_json())).await;
    let (client, url) = (update_client("http://127.0.0.1"), releases_url(&server));
    assert_eq!(check_for_update(&client, &url, "1.2.0").await, Ok(vec![]));
    assert_eq!(check_for_update(&client, &url, "9.9.9").await, Ok(vec![]));
}

#[tokio::test]
async fn check_for_update_is_empty_when_no_release_exists() {
    let server = server_with(ResponseTemplate::new(404)).await;
    let client = update_client("http://127.0.0.1");
    assert_eq!(
        check_for_update(&client, &releases_url(&server), "1.0.0").await,
        Ok(vec![])
    );
}

#[tokio::test]
async fn check_for_update_errors_on_other_status_codes() {
    let client = update_client("http://127.0.0.1");
    for status in [403u16, 429, 500] {
        let server = server_with(ResponseTemplate::new(status)).await;
        let err = check_for_update(&client, &releases_url(&server), "1.0.0")
            .await
            .unwrap_err();
        assert_eq!(err, UpdateCheckError(format!("HTTP {status}")));
    }
}

#[tokio::test]
async fn check_for_update_errors_when_connection_fails() {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let url = format!("http://127.0.0.1:{port}/releases");
    let err = check_for_update(&update_client("http://127.0.0.1"), &url, "1.0.0")
        .await
        .expect_err("a refused connection must be an error");
    // reqwest 頂層訊息只有 "error sending request"，原因（被拒絕的 OS 錯誤）在原因鏈裡
    assert!(err.0.contains("os error"), "{err}");
    assert!(
        !err.0.contains(&port.to_string()),
        "URL must be stripped: {err}"
    );
}

#[tokio::test]
async fn check_for_update_errors_on_unusable_payload() {
    let client = update_client("http://127.0.0.1");
    let server = server_with(ResponseTemplate::new(200).set_body_json(json!({}))).await;
    let err = check_for_update(&client, &releases_url(&server), "1.0.0")
        .await
        .unwrap_err();
    assert!(err.0.contains("unexpected response"), "{err}");
    let server = server_with(ResponseTemplate::new(200).set_body_string("not json")).await;
    let err = check_for_update(&client, &releases_url(&server), "1.0.0")
        .await
        .unwrap_err();
    assert!(err.0.contains("line 1 column"), "{err}");
}

#[tokio::test]
async fn check_for_update_follows_redirects() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/old"))
        .respond_with(ResponseTemplate::new(301).insert_header("Location", "/releases"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/releases"))
        .respond_with(ResponseTemplate::new(200).set_body_json(releases_json()))
        .mount(&server)
        .await;
    let url = format!("{}/old", server.uri());
    let list = check_for_update(&update_client("http://127.0.0.1"), &url, "1.0.0")
        .await
        .unwrap();
    assert_eq!(list[0].version, "1.2.0");
}

async fn wait_until(what: &str, mut condition: impl AsyncFnMut() -> bool) {
    let deadline = std::time::Instant::now() + common::WAIT;
    while !condition().await {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn collecting_callback() -> (Arc<AtomicUsize>, Seen, Found) {
    let count = Arc::new(AtomicUsize::new(0));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (c, s) = (Arc::clone(&count), Arc::clone(&seen));
    let found: Found = Arc::new(move |release| {
        c.fetch_add(1, Ordering::SeqCst);
        s.lock().unwrap().push(release);
    });
    (count, seen, found)
}

fn start(
    server: &MockServer,
    interval: Duration,
    found: Found,
) -> (CancellationToken, tokio::task::JoinHandle<()>) {
    let stop = CancellationToken::new();
    let task = schedule_update_checks(
        &Handle::current(),
        stop.clone(),
        releases_url(server),
        interval,
        found,
    );
    (stop, task)
}

#[tokio::test(flavor = "multi_thread")]
async fn schedule_update_checks_checks_now_and_again_after_the_interval() {
    capture_logs();
    let body = json!([release_item("v99.0.0", json!({}))]);
    let server = server_with(ResponseTemplate::new(200).set_body_json(body)).await;
    let (count, seen, found) = collecting_callback();
    let (stop, task) = start(&server, Duration::from_millis(50), found);

    wait_until("two announcements", async || {
        count.load(Ordering::SeqCst) >= 2
    })
    .await;
    stop.cancel();
    task.await.unwrap();
    assert_eq!(seen.lock().unwrap()[0][0].version, "99.0.0");
    assert!(log_text().contains("[update] next automatic check in 0 min"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_panicking_callback_is_logged_and_checks_go_on() {
    capture_logs();
    let body = json!([release_item("v99.0.0", json!({}))]);
    let server = server_with(ResponseTemplate::new(200).set_body_json(body)).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    let found: Found = Arc::new(move |_| {
        counter.fetch_add(1, Ordering::SeqCst);
        panic!("callback boom");
    });
    let (stop, task) = start(&server, Duration::from_millis(30), found);
    wait_until("a second call after the panic", async || {
        calls.load(Ordering::SeqCst) >= 2
    })
    .await;
    stop.cancel();
    task.await.unwrap();
    assert!(log_text().contains("[update] announce callback panicked: callback boom"));
}

#[tokio::test(flavor = "multi_thread")]
async fn found_is_not_called_without_a_newer_release() {
    let server = server_with(ResponseTemplate::new(404)).await;
    let (count, _seen, found) = collecting_callback();
    let (stop, task) = start(&server, Duration::from_millis(30), found);
    wait_until("three polls", async || request_count(&server).await >= 3).await;
    stop.cancel();
    task.await.unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn scheduled_check_survives_failures_and_runs_again() {
    capture_logs();
    let server = MockServer::start().await;
    for (priority, response, once) in [
        (1, ResponseTemplate::new(403), true),
        (
            2,
            ResponseTemplate::new(200).set_body_json(json!([release_item("v99.0.0", json!({}))])),
            true,
        ),
        (3, ResponseTemplate::new(404), false),
    ] {
        let mock = Mock::given(method("GET"))
            .and(path("/releases"))
            .respond_with(response)
            .with_priority(priority);
        let mock = if once { mock.up_to_n_times(1) } else { mock };
        mock.mount(&server).await;
    }
    let (count, seen, found) = collecting_callback();
    let (stop, task) = start(&server, Duration::from_millis(50), found);

    wait_until("the announcement after the failure", async || {
        count.load(Ordering::SeqCst) >= 1
    })
    .await;
    wait_until("a further poll", async || request_count(&server).await >= 4).await;
    stop.cancel();
    task.await.unwrap();

    assert_eq!(count.load(Ordering::SeqCst), 1, "failures must stay silent");
    assert_eq!(seen.lock().unwrap()[0][0].version, "99.0.0");
    assert!(log_text().contains("[update] check failed: HTTP 403"));
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_ends_the_schedule() {
    let server = server_with(ResponseTemplate::new(404)).await;
    let (_count, _seen, found) = collecting_callback();
    let (stop, task) = start(&server, Duration::from_secs(3600), found);
    wait_until("the first check", async || {
        request_count(&server).await >= 1
    })
    .await;
    stop.cancel();
    tokio::time::timeout(common::WAIT, task)
        .await
        .expect("schedule must end after stop")
        .unwrap();
}
