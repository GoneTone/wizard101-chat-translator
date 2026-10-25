//! 可續傳、sha256 驗證的下載（wiremock，不打真網路）。

use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::Duration;

use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use wizard101_chat_translator::local::download::{DownloadError, Progress, download};
use wizard101_chat_translator::local::hf::hf_client;

const BODY: &[u8] = b"0123456789abcdefghij";

fn sha(bytes: &[u8]) -> String {
    wizard101_chat_translator::util::hex(&Sha256::digest(bytes))
}

async fn run(
    server: &MockServer,
    url_path: &str,
    dest: &Path,
    sha256: Option<&str>,
    progress: &Progress,
) -> Result<(), DownloadError> {
    let url = format!("{}{url_path}", server.uri());
    download(
        &hf_client(&server.uri()),
        &url,
        dest,
        BODY.len() as u64,
        sha256,
        progress,
        &CancellationToken::new(),
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn resumes_with_range_and_verifies_sha256() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/f"))
        .and(header("range", "bytes=4-"))
        .respond_with(
            ResponseTemplate::new(206)
                .insert_header("Content-Range", "bytes 4-19/20")
                .set_body_bytes(&BODY[4..]),
        )
        .expect(1)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("sub").join("a.gguf");
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    let part = dir.path().join("sub").join("a.gguf.part");
    std::fs::write(&part, &BODY[..4]).unwrap();
    let progress = Progress::default();

    run(
        &server,
        "/f",
        &dest,
        Some(&sha(BODY).to_uppercase()),
        &progress,
    )
    .await
    .unwrap();

    assert_eq!(std::fs::read(&dest).unwrap(), BODY);
    assert!(!part.exists());
    assert_eq!(progress.total.load(Ordering::Relaxed), BODY.len() as u64);
    assert_eq!(progress.done.load(Ordering::Relaxed), BODY.len() as u64);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_200_reply_to_a_range_request_restarts_from_zero() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/f"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(BODY))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("a.gguf");
    std::fs::write(dir.path().join("a.gguf.part"), b"junk").unwrap();
    let progress = Progress::default();

    run(&server, "/f", &dest, None, &progress).await.unwrap();

    assert_eq!(std::fs::read(&dest).unwrap(), BODY);
    assert_eq!(progress.done.load(Ordering::Relaxed), BODY.len() as u64);
}

#[tokio::test(flavor = "multi_thread")]
async fn checksum_mismatch_deletes_the_part_file() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(BODY))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("a.gguf");

    let err = run(
        &server,
        "/f",
        &dest,
        Some(&"0".repeat(64)),
        &Progress::default(),
    )
    .await
    .unwrap_err();

    assert!(matches!(err, DownloadError::Checksum { .. }), "{err}");
    assert!(!dir.path().join("a.gguf.part").exists());
    assert!(!dest.exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_keeps_the_part_file() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(BODY)
                .set_delay(Duration::from_secs(30)),
        )
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("a.gguf");
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        trigger.cancel();
    });

    let url = format!("{}/f", server.uri());
    let err = download(
        &hf_client(&server.uri()),
        &url,
        &dest,
        BODY.len() as u64,
        None,
        &Progress::default(),
        &cancel,
    )
    .await
    .unwrap_err();

    assert!(matches!(err, DownloadError::Cancelled), "{err}");
    assert!(dir.path().join("a.gguf.part").exists());
    assert!(!dest.exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn follows_redirects_to_the_cdn() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/resolve"))
        .respond_with(ResponseTemplate::new(302).insert_header("Location", "/cdn/x?sig=secret"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/cdn/x"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(BODY))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("a.gguf");

    run(
        &server,
        "/resolve",
        &dest,
        Some(&sha(BODY)),
        &Progress::default(),
    )
    .await
    .unwrap();

    assert_eq!(std::fs::read(&dest).unwrap(), BODY);
}

#[tokio::test(flavor = "multi_thread")]
async fn http_errors_and_short_bodies_keep_the_part_file() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/missing"))
        .respond_with(ResponseTemplate::new(404).set_body_string("nope"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/short"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(&BODY[..5]))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("a.gguf");

    let err = run(&server, "/missing", &dest, None, &Progress::default())
        .await
        .unwrap_err();
    assert!(
        matches!(&err, DownloadError::Http(m) if m == "HTTP 404 Not Found: nope"),
        "{err}"
    );

    let err = run(&server, "/short", &dest, None, &Progress::default())
        .await
        .unwrap_err();
    assert!(matches!(err, DownloadError::Http(_)), "{err}");
    assert!(!dest.exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn refuses_when_the_disk_is_too_small() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let url = format!("{}/f", server.uri());
    let err = download(
        &hf_client(&server.uri()),
        &url,
        &dir.path().join("a.gguf"),
        u64::MAX / 2,
        None,
        &Progress::default(),
        &CancellationToken::new(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, DownloadError::NoSpace { .. }), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_mismatched_content_range_restarts_from_zero() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(header("range", "bytes=4-"))
        .respond_with(
            ResponseTemplate::new(206)
                .insert_header("Content-Range", "bytes 0-19/20")
                .set_body_bytes(BODY),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(BODY))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("a.gguf");
    std::fs::write(dir.path().join("a.gguf.part"), &BODY[..4]).unwrap();
    let progress = Progress::default();

    run(&server, "/f", &dest, Some(&sha(BODY)), &progress)
        .await
        .unwrap();

    assert_eq!(std::fs::read(&dest).unwrap(), BODY);
    assert_eq!(progress.done.load(Ordering::Relaxed), BODY.len() as u64);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_oversized_body_is_rejected_by_content_length() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes([BODY, b"extra"].concat()))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();

    let err = run(
        &server,
        "/f",
        &dir.path().join("a.gguf"),
        None,
        &Progress::default(),
    )
    .await
    .unwrap_err();

    assert!(
        matches!(&err, DownloadError::Http(m) if m.contains("Content-Length")),
        "{err}"
    );
}
