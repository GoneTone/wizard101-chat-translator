//! 本版說明視窗的托管：背景請求只發一次、結果快取、失敗重試與關窗重設（離屏 viewport，wiremock 假 GitHub）。

use std::sync::mpsc::Receiver;
use std::time::Duration;

use crate::common::frames::Frames;
use eframe::egui::Context;
use tokio::runtime::{Handle, Runtime};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use wizard101_chat_translator::runtime::updater::{PROJECT_URL, Release};
use wizard101_chat_translator::ui::events::{UiEvent, UiPoster};
use wizard101_chat_translator::ui::release_notes_host::{ReleaseNotesHost, github_url};
use wizard101_chat_translator::ui::settings::release_notes_window::{NotesAction, NotesState};
use wizard101_chat_translator::ui::settings_host::WindowOps;

fn tag_path() -> String {
    format!("/tags/v{}", env!("CARGO_PKG_VERSION"))
}

fn release_json() -> serde_json::Value {
    serde_json::json!({
        "tag_name": format!("v{}", env!("CARGO_PKG_VERSION")),
        "html_url": "https://example.invalid/rel",
        "body": "notes",
    })
}

fn notes() -> Release {
    Release {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        url: "https://example.invalid/rel".to_owned(),
        body: "notes".to_owned(),
        ..Release::default()
    }
}

fn step(frames: &mut Frames, host: &mut ReleaseNotesHost, close: bool) {
    frames.run(close, |ctx| host.show(ctx, &[]));
}

fn host_for(rt: Handle, api: &str) -> (ReleaseNotesHost, Receiver<UiEvent>) {
    let (poster, rx) = UiPoster::for_test();
    let mut host = ReleaseNotesHost::new(rt, poster, api.to_owned());
    host.set_window_ops(WindowOps::offscreen());
    (host, rx)
}

/// 等背景請求投遞結果，交給 host。
fn deliver(host: &mut ReleaseNotesHost, rx: &Receiver<UiEvent>) {
    match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
        UiEvent::ReleaseNotesLoaded(result) => host.loaded(result),
        other => panic!("unexpected event: {other:?}"),
    }
}

async fn mock_tag(server: &MockServer, status: u16, expected: u64) {
    let template = if status == 200 {
        ResponseTemplate::new(200).set_body_json(release_json())
    } else {
        ResponseTemplate::new(status)
    };
    Mock::given(method("GET"))
        .and(path(tag_path()))
        .respond_with(template)
        .expect(expected)
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn open_fetches_and_shows_the_notes() {
    let server = MockServer::start().await;
    mock_tag(&server, 200, 1).await;
    let (mut host, rx) = host_for(Handle::current(), &server.uri());

    host.open();
    assert_eq!(host.state(), Some(&NotesState::Loading));
    deliver(&mut host, &rx);

    assert!(matches!(host.state(), Some(NotesState::Loaded(r)) if r.body == "notes"));
}

#[tokio::test(flavor = "multi_thread")]
async fn open_while_loading_does_not_request_again() {
    let server = MockServer::start().await;
    mock_tag(&server, 200, 1).await;
    let (mut host, rx) = host_for(Handle::current(), &server.uri());

    host.open();
    host.open();
    deliver(&mut host, &rx);

    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn reopening_uses_the_cache() {
    let server = MockServer::start().await;
    mock_tag(&server, 200, 1).await;
    let (mut host, rx) = host_for(Handle::current(), &server.uri());
    let mut frames = Frames::new();

    host.open();
    deliver(&mut host, &rx);
    step(&mut frames, &mut host, false);
    step(&mut frames, &mut host, true);
    assert!(!host.is_open());
    host.open();

    assert!(host.is_open());
    assert!(matches!(host.state(), Some(NotesState::Loaded(_))));
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn result_after_close_is_cached() {
    let server = MockServer::start().await;
    mock_tag(&server, 200, 1).await;
    let (mut host, rx) = host_for(Handle::current(), &server.uri());
    let mut frames = Frames::new();

    host.open();
    step(&mut frames, &mut host, false);
    step(&mut frames, &mut host, true);
    assert!(!host.is_open());
    host.loaded(Ok(Some(notes())));
    host.open();

    assert_eq!(host.state(), Some(&NotesState::Loaded(notes())));
    // 吃掉仍在途的請求事件，wiremock 的 expect(1) 才算得準
    deliver(&mut host, &rx);
}

#[tokio::test(flavor = "multi_thread")]
async fn reopening_while_loading_does_not_request_again() {
    let server = MockServer::start().await;
    mock_tag(&server, 200, 1).await;
    let (mut host, rx) = host_for(Handle::current(), &server.uri());
    let mut frames = Frames::new();

    host.open();
    step(&mut frames, &mut host, false);
    step(&mut frames, &mut host, true);
    assert!(!host.is_open());
    host.open();
    assert_eq!(host.state(), Some(&NotesState::Loading));
    deliver(&mut host, &rx);

    assert!(matches!(host.state(), Some(NotesState::Loaded(_))));
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
}

#[test]
fn github_url_prefers_the_loaded_release() {
    assert_eq!(
        github_url(Some(&NotesState::Loaded(notes()))),
        "https://example.invalid/rel"
    );
    for state in [None, Some(NotesState::Loading), Some(NotesState::NotFound)] {
        assert_eq!(
            github_url(state.as_ref()),
            format!("{PROJECT_URL}/releases")
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn not_found_is_retried_on_next_open() {
    let server = MockServer::start().await;
    mock_tag(&server, 404, 2).await;
    let (mut host, rx) = host_for(Handle::current(), &server.uri());
    let mut frames = Frames::new();

    host.open();
    deliver(&mut host, &rx);
    assert_eq!(host.state(), Some(&NotesState::NotFound));
    step(&mut frames, &mut host, false);
    step(&mut frames, &mut host, true);
    host.open();
    deliver(&mut host, &rx);

    assert_eq!(host.state(), Some(&NotesState::NotFound));
}

#[tokio::test(flavor = "multi_thread")]
async fn retry_after_failure_shows_the_notes() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(tag_path()))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    mock_tag(&server, 200, 1).await;
    let (mut host, rx) = host_for(Handle::current(), &server.uri());
    let ctx = Context::default();

    host.open();
    deliver(&mut host, &rx);
    assert!(matches!(host.state(), Some(NotesState::Failed(_))));
    host.act(&ctx, NotesAction::Retry);
    assert_eq!(host.state(), Some(&NotesState::Loading));
    deliver(&mut host, &rx);

    assert!(matches!(host.state(), Some(NotesState::Loaded(_))));
}

#[tokio::test(flavor = "multi_thread")]
async fn retry_while_loading_does_not_request_again() {
    let server = MockServer::start().await;
    mock_tag(&server, 200, 1).await;
    let (mut host, rx) = host_for(Handle::current(), &server.uri());
    let ctx = Context::default();

    host.open();
    host.act(&ctx, NotesAction::Retry);
    deliver(&mut host, &rx);

    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
}

#[test]
fn closing_resets_the_window() {
    let rt = Runtime::new().unwrap();
    let (mut host, _rx) = host_for(rt.handle().clone(), "http://127.0.0.1:1");
    let mut frames = Frames::new();

    host.open();
    step(&mut frames, &mut host, false);
    assert!(host.is_open());
    step(&mut frames, &mut host, true);

    assert!(!host.is_open());
}
