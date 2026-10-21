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
    RELEASES_URL, Release, UpdateCheckError, check_for_update, fetch_latest_release, is_newer,
    schedule_update_checks, update_client,
};

const TAG_URL: &str = "https://github.com/GoneTone/wizard101-chat-translator/releases/tag/v0.2.0";

type Found = Arc<dyn Fn(Release) + Send + Sync>;

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

fn release_json() -> serde_json::Value {
    json!({"tag_name": "v0.2.0", "html_url": TAG_URL})
}

async fn server_with(response: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/latest"))
        .respond_with(response)
        .mount(&server)
        .await;
    server
}

fn latest_url(server: &MockServer) -> String {
    format!("{}/latest", server.uri())
}

async fn request_count(server: &MockServer) -> usize {
    server.received_requests().await.unwrap().len()
}

#[tokio::test]
async fn fetch_latest_release_reads_tag_and_url() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/latest"))
        .and(header("Accept", "application/vnd.github+json"))
        .and(header(
            "User-Agent",
            format!("wizard101-chat-translator/{}", env!("CARGO_PKG_VERSION")).as_str(),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(release_json()))
        .expect(1)
        .mount(&server)
        .await;
    let release =
        fetch_latest_release(&update_client("http://127.0.0.1"), &latest_url(&server)).await;
    assert_eq!(
        release,
        Ok(Some(Release {
            version: "0.2.0".into(),
            url: TAG_URL.into()
        }))
    );
}

#[tokio::test]
async fn fetch_latest_release_returns_none_when_no_release_exists() {
    let server = server_with(ResponseTemplate::new(404)).await;
    assert_eq!(
        fetch_latest_release(&update_client("http://127.0.0.1"), &latest_url(&server)).await,
        Ok(None)
    );
}

#[tokio::test]
async fn fetch_latest_release_errors_on_other_status_codes() {
    for status in [403u16, 429, 500] {
        let server = server_with(ResponseTemplate::new(status)).await;
        let err = fetch_latest_release(&update_client("http://127.0.0.1"), &latest_url(&server))
            .await
            .unwrap_err();
        assert_eq!(err, UpdateCheckError(format!("HTTP {status}")));
        assert!(err.to_string().contains(&status.to_string()));
    }
}

#[tokio::test]
async fn fetch_latest_release_errors_when_connection_fails() {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let url = format!("http://127.0.0.1:{port}/latest");
    let err = fetch_latest_release(&update_client("http://127.0.0.1"), &url)
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
async fn fetch_latest_release_errors_on_unusable_payload() {
    let server = server_with(ResponseTemplate::new(200).set_body_json(json!({}))).await;
    assert!(
        fetch_latest_release(&update_client("http://127.0.0.1"), &latest_url(&server))
            .await
            .is_err()
    );
    let server = server_with(ResponseTemplate::new(200).set_body_string("not json")).await;
    let err = fetch_latest_release(&update_client("http://127.0.0.1"), &latest_url(&server))
        .await
        .unwrap_err();
    assert!(err.0.contains("line 1 column"), "{err}");
}

#[tokio::test]
async fn fetch_latest_release_falls_back_to_releases_page_without_html_url() {
    let server =
        server_with(ResponseTemplate::new(200).set_body_json(json!({"tag_name": "v0.3.0"}))).await;
    assert_eq!(
        fetch_latest_release(&update_client("http://127.0.0.1"), &latest_url(&server)).await,
        Ok(Some(Release {
            version: "0.3.0".into(),
            url: RELEASES_URL.into()
        }))
    );
}

#[tokio::test]
async fn fetch_latest_release_follows_redirects() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/old"))
        .respond_with(ResponseTemplate::new(301).insert_header("Location", "/latest"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/latest"))
        .respond_with(ResponseTemplate::new(200).set_body_json(release_json()))
        .mount(&server)
        .await;
    let url = format!("{}/old", server.uri());
    let release = fetch_latest_release(&update_client("http://127.0.0.1"), &url)
        .await
        .unwrap();
    assert_eq!(release.unwrap().version, "0.2.0");
}

#[tokio::test]
async fn check_for_update_returns_release_only_when_newer() {
    let server = server_with(ResponseTemplate::new(200).set_body_json(release_json())).await;
    let (client, url) = (update_client("http://127.0.0.1"), latest_url(&server));
    let found = check_for_update(&client, &url, "0.1.0").await.unwrap();
    assert_eq!(found.unwrap().version, "0.2.0");
    assert_eq!(check_for_update(&client, &url, "0.2.0").await, Ok(None));
    assert_eq!(check_for_update(&client, &url, "9.9.9").await, Ok(None));
}

#[tokio::test]
async fn check_for_update_returns_none_when_no_release_exists() {
    let server = server_with(ResponseTemplate::new(404)).await;
    assert_eq!(
        check_for_update(
            &update_client("http://127.0.0.1"),
            &latest_url(&server),
            "0.1.0"
        )
        .await,
        Ok(None)
    );
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

fn collecting_callback() -> (Arc<AtomicUsize>, Arc<Mutex<Vec<Release>>>, Found) {
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
        latest_url(server),
        interval,
        found,
    );
    (stop, task)
}

#[tokio::test(flavor = "multi_thread")]
async fn schedule_update_checks_checks_now_and_again_after_the_interval() {
    capture_logs();
    let body = json!({"tag_name": "v99.0.0", "html_url": TAG_URL});
    let server = server_with(ResponseTemplate::new(200).set_body_json(body)).await;
    let (count, seen, found) = collecting_callback();
    let (stop, task) = start(&server, Duration::from_millis(50), found);

    wait_until("two announcements", async || {
        count.load(Ordering::SeqCst) >= 2
    })
    .await;
    stop.cancel();
    task.await.unwrap();
    assert_eq!(
        seen.lock().unwrap()[0],
        Release {
            version: "99.0.0".into(),
            url: TAG_URL.into()
        }
    );
    assert!(log_text().contains("[update] next automatic check in 0 min"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_panicking_callback_is_logged_and_checks_go_on() {
    capture_logs();
    let body = json!({"tag_name": "v99.0.0", "html_url": TAG_URL});
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
            ResponseTemplate::new(200).set_body_json(json!({"tag_name": "v99.0.0"})),
            true,
        ),
        (3, ResponseTemplate::new(404), false),
    ] {
        let mock = Mock::given(method("GET"))
            .and(path("/latest"))
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
    assert_eq!(seen.lock().unwrap()[0].version, "99.0.0");
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
