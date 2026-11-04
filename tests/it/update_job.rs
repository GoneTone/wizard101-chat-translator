//! 更新下載狀態機（wiremock 伺服假 exe，不打真網路）。

use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::runtime::Handle;
use tokio::sync::oneshot;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use wizard101_chat_translator::net::download_client;
use wizard101_chat_translator::runtime::self_update::{new_path, part_path};
use wizard101_chat_translator::runtime::update_job::{Phase, UpdateJob};
use wizard101_chat_translator::runtime::updater::{Asset, EXE_ASSET, Release};

const BODY: &[u8] = b"fake exe";

fn sha(bytes: &[u8]) -> String {
    wizard101_chat_translator::util::hex(&Sha256::digest(bytes))
}

fn release(version: &str, url: &str, sha256: String) -> Release {
    Release {
        version: version.to_owned(),
        asset: Some(Asset {
            url: url.to_owned(),
            size: BODY.len() as u64,
            sha256,
        }),
        ..Release::default()
    }
}

fn offline_release(version: &str) -> Release {
    release(version, "http://127.0.0.1:1/app.exe", sha(BODY))
}

fn exe_in(dir: &TempDir) -> PathBuf {
    dir.path().join("app.exe")
}

fn touch(file: &Path) {
    std::fs::write(file, b"x").unwrap();
}

async fn serve(delay: Duration) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/app.exe"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(BODY)
                .set_delay(delay),
        )
        .mount(&server)
        .await;
    server
}

fn start(job: &mut UpdateJob, server: &MockServer) -> oneshot::Receiver<(u64, Result<(), String>)> {
    let (tx, rx) = oneshot::channel();
    job.start(
        &Handle::current(),
        download_client(&server.uri()),
        Box::new(move |run, result| {
            let _ = tx.send((run, result));
        }),
    );
    rx
}

#[tokio::test(flavor = "multi_thread")]
async fn download_ends_ready_with_the_new_file() {
    let server = serve(Duration::ZERO).await;
    let dir = TempDir::new().unwrap();
    let exe = exe_in(&dir);
    let url = format!("{}/app.exe", server.uri());
    let mut job = UpdateJob::new(exe.clone());
    job.set_releases(vec![release("1.2.0", &url, sha(BODY))]);

    let done = start(&mut job, &server);
    assert_eq!(job.phase(), &Phase::Downloading);
    let (run, result) = done.await.unwrap();
    job.finish(run, result);

    assert_eq!(job.phase(), &Phase::Ready);
    assert_eq!(std::fs::read(new_path(&exe)).unwrap(), BODY);
}

#[tokio::test(flavor = "multi_thread")]
async fn checksum_mismatch_fails_without_a_new_file() {
    let server = serve(Duration::ZERO).await;
    let dir = TempDir::new().unwrap();
    let exe = exe_in(&dir);
    let url = format!("{}/app.exe", server.uri());
    let mut job = UpdateJob::new(exe.clone());
    job.set_releases(vec![release("1.2.0", &url, sha(b"other"))]);

    let done = start(&mut job, &server);
    let (run, result) = done.await.unwrap();
    job.finish(run, result);

    match job.phase() {
        Phase::Failed { error, replace } => {
            assert!(error.contains("sha256 mismatch"), "{error}");
            assert!(!replace);
        }
        other => panic!("unexpected phase {other:?}"),
    }
    assert!(!new_path(&exe).exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_deletes_the_partial_file() {
    let server = serve(Duration::from_secs(30)).await;
    let dir = TempDir::new().unwrap();
    let exe = exe_in(&dir);
    let url = format!("{}/app.exe", server.uri());
    let mut job = UpdateJob::new(exe.clone());
    job.set_releases(vec![release("1.2.0", &url, sha(BODY))]);

    let done = start(&mut job, &server);
    tokio::time::sleep(Duration::from_millis(300)).await;
    job.cancel();
    assert_eq!(job.phase(), &Phase::Cancelled);
    let (run, result) = done.await.unwrap();
    assert!(result.is_err());
    job.finish(run, result);

    assert_eq!(job.phase(), &Phase::Cancelled);
    assert!(!part_path(&exe).exists());
}

#[test]
fn start_without_verifiable_asset_fails_immediately() {
    let dir = TempDir::new().unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut job = UpdateJob::new(exe_in(&dir));
    job.set_releases(vec![Release {
        version: "1.2.0".to_owned(),
        ..Release::default()
    }]);

    job.start(
        rt.handle(),
        download_client("http://127.0.0.1:1"),
        Box::new(|_, _| panic!("no download should run")),
    );

    match job.phase() {
        Phase::Failed { error, replace } => {
            assert!(
                error.contains("1.2.0") && error.contains(EXE_ASSET),
                "{error}"
            );
            assert!(!replace);
        }
        other => panic!("unexpected phase {other:?}"),
    }
}

#[test]
fn closing_after_a_failed_download_deletes_the_partial_file() {
    let dir = TempDir::new().unwrap();
    let exe = exe_in(&dir);
    touch(&part_path(&exe));
    let mut job = UpdateJob::with_phase(
        exe.clone(),
        vec![offline_release("1.2.0")],
        Phase::Failed {
            error: "x".to_owned(),
            replace: false,
        },
    );

    job.window_closed();

    assert!(!part_path(&exe).exists());
    assert_eq!(job.phase(), &Phase::Idle);
}

#[test]
fn closing_when_ready_keeps_the_new_file() {
    let dir = TempDir::new().unwrap();
    let exe = exe_in(&dir);
    touch(&new_path(&exe));
    let mut job = UpdateJob::with_phase(exe.clone(), vec![offline_release("1.2.0")], Phase::Ready);

    job.window_closed();

    assert!(new_path(&exe).exists());
    assert_eq!(job.phase(), &Phase::Ready);
}

#[test]
fn newer_release_discards_ready_download() {
    let dir = TempDir::new().unwrap();
    let exe = exe_in(&dir);
    touch(&new_path(&exe));
    let mut job = UpdateJob::with_phase(exe.clone(), vec![offline_release("1.2.0")], Phase::Ready);

    job.set_releases(vec![offline_release("1.3.0"), offline_release("1.2.0")]);

    assert!(!new_path(&exe).exists());
    assert_eq!(job.phase(), &Phase::Idle);
    assert_eq!(job.releases()[0].version, "1.3.0");
}

#[test]
fn same_release_list_keeps_ready_download() {
    let dir = TempDir::new().unwrap();
    let exe = exe_in(&dir);
    touch(&new_path(&exe));
    let mut job = UpdateJob::with_phase(exe.clone(), vec![offline_release("1.2.0")], Phase::Ready);

    job.set_releases(vec![offline_release("1.2.0")]);

    assert!(new_path(&exe).exists());
    assert_eq!(job.phase(), &Phase::Ready);
}

#[tokio::test(flavor = "multi_thread")]
async fn newer_release_during_download_discards_on_finish() {
    let server = serve(Duration::ZERO).await;
    let dir = TempDir::new().unwrap();
    let exe = exe_in(&dir);
    let url = format!("{}/app.exe", server.uri());
    let mut job = UpdateJob::new(exe.clone());
    job.set_releases(vec![release("1.2.0", &url, sha(BODY))]);

    let done = start(&mut job, &server);
    job.set_releases(vec![release("1.3.0", &url, sha(BODY))]);
    assert_eq!(job.releases()[0].version, "1.2.0");
    let (run, result) = done.await.unwrap();
    job.finish(run, result);

    assert_eq!(job.releases()[0].version, "1.3.0");
    assert!(!new_path(&exe).exists());
    assert_eq!(job.phase(), &Phase::Idle);
}

#[test]
fn replace_failure_retry_goes_straight_to_restart() {
    let dir = TempDir::new().unwrap();
    let mut job = UpdateJob::with_phase(exe_in(&dir), vec![offline_release("1.2.0")], Phase::Ready);

    job.begin_restart();
    job.restart_failed("x".to_owned());
    assert_eq!(
        job.phase(),
        &Phase::Failed {
            error: "x".to_owned(),
            replace: true
        }
    );
    job.begin_restart();

    assert_eq!(job.phase(), &Phase::Restarting);
}

#[test]
fn open_does_not_reset_phase() {
    let dir = TempDir::new().unwrap();
    let mut job = UpdateJob::with_phase(
        exe_in(&dir),
        vec![offline_release("1.2.0")],
        Phase::Restarting,
    );

    job.set_releases(vec![offline_release("1.2.0")]);

    assert_eq!(job.phase(), &Phase::Restarting);
}

fn failed(replace: bool) -> Phase {
    Phase::Failed {
        error: "x".to_owned(),
        replace,
    }
}

#[test]
fn closing_when_cancelled_deletes_the_partial_file() {
    let dir = TempDir::new().unwrap();
    let exe = exe_in(&dir);
    touch(&part_path(&exe));
    let mut job = UpdateJob::with_phase(
        exe.clone(),
        vec![offline_release("1.2.0")],
        Phase::Cancelled,
    );

    job.window_closed();

    assert!(!part_path(&exe).exists());
    assert_eq!(job.phase(), &Phase::Idle);
}

#[tokio::test(flavor = "multi_thread")]
async fn closing_while_downloading_is_a_no_op() {
    let server = serve(Duration::from_secs(30)).await;
    let dir = TempDir::new().unwrap();
    let url = format!("{}/app.exe", server.uri());
    let mut job = UpdateJob::new(exe_in(&dir));
    job.set_releases(vec![release("1.2.0", &url, sha(BODY))]);
    let _done = start(&mut job, &server);

    job.window_closed();

    assert_eq!(job.phase(), &Phase::Downloading);
    job.cancel();
}

#[test]
fn newer_release_discards_a_download_waiting_for_replace_retry() {
    let dir = TempDir::new().unwrap();
    let exe = exe_in(&dir);
    touch(&new_path(&exe));
    let mut job = UpdateJob::with_phase(exe.clone(), vec![offline_release("1.2.0")], failed(true));

    job.set_releases(vec![offline_release("1.3.0")]);

    assert!(!new_path(&exe).exists());
    assert_eq!(job.phase(), &Phase::Idle);
}

#[tokio::test(flavor = "multi_thread")]
async fn late_ok_after_cancel_deletes_the_new_file() {
    let server = serve(Duration::ZERO).await;
    let dir = TempDir::new().unwrap();
    let exe = exe_in(&dir);
    let url = format!("{}/app.exe", server.uri());
    let mut job = UpdateJob::new(exe.clone());
    job.set_releases(vec![release("1.2.0", &url, sha(BODY))]);

    let done = start(&mut job, &server);
    let (run, result) = done.await.unwrap();
    assert!(result.is_ok());
    job.cancel();
    job.finish(run, result);

    assert_eq!(job.phase(), &Phase::Cancelled);
    assert!(!new_path(&exe).exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn retry_after_cancel_waits_for_the_old_task() {
    let server = serve(Duration::from_millis(500)).await;
    let dir = TempDir::new().unwrap();
    let exe = exe_in(&dir);
    let url = format!("{}/app.exe", server.uri());
    let mut job = UpdateJob::new(exe.clone());
    job.set_releases(vec![release("1.2.0", &url, sha(BODY))]);

    let old = start(&mut job, &server);
    tokio::time::sleep(Duration::from_millis(100)).await;
    job.cancel();
    let new = start(&mut job, &server);
    assert_eq!(job.phase(), &Phase::Downloading);

    let (run, result) = old.await.unwrap();
    assert!(result.is_err());
    job.finish(run, result);
    assert_eq!(job.phase(), &Phase::Downloading);

    let (run, result) = new.await.unwrap();
    job.finish(run, result);
    assert_eq!(job.phase(), &Phase::Ready);
    assert_eq!(std::fs::read(new_path(&exe)).unwrap(), BODY);
}

#[tokio::test(flavor = "multi_thread")]
async fn late_result_after_cancel_and_close_keeps_idle() {
    let server = serve(Duration::from_millis(500)).await;
    let dir = TempDir::new().unwrap();
    let exe = exe_in(&dir);
    let url = format!("{}/app.exe", server.uri());
    let mut job = UpdateJob::new(exe.clone());
    job.set_releases(vec![release("1.2.0", &url, sha(BODY))]);

    let done = start(&mut job, &server);
    tokio::time::sleep(Duration::from_millis(100)).await;
    job.cancel();
    job.window_closed();
    assert_eq!(job.phase(), &Phase::Idle);
    let (run, result) = done.await.unwrap();
    job.finish(run, result);

    assert_eq!(job.phase(), &Phase::Idle);
    assert!(!part_path(&exe).exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_drops_a_queued_start() {
    let server = serve(Duration::from_millis(500)).await;
    let dir = TempDir::new().unwrap();
    let exe = exe_in(&dir);
    let url = format!("{}/app.exe", server.uri());
    let mut job = UpdateJob::new(exe.clone());
    job.set_releases(vec![release("1.2.0", &url, sha(BODY))]);

    let old = start(&mut job, &server);
    tokio::time::sleep(Duration::from_millis(100)).await;
    job.cancel();
    let queued = start(&mut job, &server);
    job.cancel();
    let (run, result) = old.await.unwrap();
    job.finish(run, result);

    assert_eq!(job.phase(), &Phase::Cancelled);
    assert!(queued.await.is_err());
    assert!(!part_path(&exe).exists());
    assert!(!new_path(&exe).exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn newer_release_during_queued_start_discards_the_queued_download() {
    let server = serve(Duration::from_millis(300)).await;
    let dir = TempDir::new().unwrap();
    let exe = exe_in(&dir);
    let url = format!("{}/app.exe", server.uri());
    let mut job = UpdateJob::new(exe.clone());
    job.set_releases(vec![release("1.2.0", &url, sha(BODY))]);

    let old = start(&mut job, &server);
    tokio::time::sleep(Duration::from_millis(100)).await;
    job.cancel();
    let queued = start(&mut job, &server);
    job.set_releases(vec![release("1.3.0", &url, sha(BODY))]);
    let (run, result) = old.await.unwrap();
    job.finish(run, result);
    let (run, result) = queued.await.unwrap();
    job.finish(run, result);

    assert_eq!(job.releases()[0].version, "1.3.0");
    assert!(!new_path(&exe).exists());
    assert_eq!(job.phase(), &Phase::Idle);
}
