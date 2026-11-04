//! 更新視窗的托管：開關窗、下載中擋 ✕、驗證後的設定檢查與重新啟動時機（離屏 viewport，wiremock 伺服假 exe）。

use std::cell::Cell;
use std::path::PathBuf;

use eframe::egui::{
    Context, FullOutput, RawInput, ViewportCommand, ViewportEvent, ViewportId, ViewportInfo,
};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio::runtime::{Handle, Runtime};
use tokio::sync::oneshot;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use wizard101_chat_translator::net::download_client;
use wizard101_chat_translator::runtime::self_update::part_path;
use wizard101_chat_translator::runtime::update_job::{Phase, UpdateJob};
use wizard101_chat_translator::runtime::updater::{Asset, Release};
use wizard101_chat_translator::ui::events::UiPoster;
use wizard101_chat_translator::ui::fonts_egui;
use wizard101_chat_translator::ui::settings_host::WindowOps;
use wizard101_chat_translator::ui::update_host::{HostAction, UpdateHost};

const BODY: &[u8] = b"fake exe";

thread_local! {
    static CLOSE_NEXT: Cell<bool> = const { Cell::new(false) };
}

/// 同 eframe：immediate viewport 在自己的 `run_ui` 裡跑；`CLOSE_NEXT` 時那一幀帶關閉要求。
fn native_like_context() -> Context {
    let ctx = Context::default();
    fonts_egui::install(&ctx, "en-US");
    ctx.set_embed_viewports(false);
    Context::set_immediate_viewport_renderer(|ctx, mut viewport| {
        let mut info = ViewportInfo::default();
        if CLOSE_NEXT.take() {
            info.events.push(ViewportEvent::Close);
        }
        let mut input = RawInput {
            viewport_id: viewport.ids.this,
            ..Default::default()
        };
        input.viewports.insert(viewport.ids.this, info);
        ctx.run_ui(input, |ui| (viewport.viewport_ui_cb)(ui))
            .textures_delta
            .clear();
    });
    ctx
}

/// 一連串幀：上一幀登記過的視窗下一幀當成已建好。
struct Frames {
    ctx: Context,
    windows: Vec<ViewportId>,
}

impl Frames {
    fn new() -> Self {
        Self {
            ctx: native_like_context(),
            windows: Vec::new(),
        }
    }

    /// 跑一幀；回傳 host 的動作與這一幀所有 viewport 收到的指令。
    fn run(
        &mut self,
        host: &mut UpdateHost,
        unsaved: bool,
        close: bool,
    ) -> (Vec<HostAction>, Vec<ViewportCommand>) {
        let mut input = RawInput::default();
        input
            .viewports
            .insert(ViewportId::ROOT, ViewportInfo::default());
        for id in &self.windows {
            let window = ViewportInfo {
                native_pixels_per_point: Some(1.0),
                ..Default::default()
            };
            input.viewports.insert(*id, window);
        }
        CLOSE_NEXT.set(close);
        let mut got = Vec::new();
        let mut output: FullOutput = self.ctx.run_ui(input, |ui| {
            got = host.show(ui.ctx(), &[], unsaved);
        });
        CLOSE_NEXT.set(false);
        output.textures_delta.clear();
        self.windows = output
            .viewport_output
            .keys()
            .copied()
            .filter(|id| *id != ViewportId::ROOT)
            .collect();
        let commands = output
            .viewport_output
            .into_values()
            .flat_map(|v| v.commands)
            .collect();
        (got, commands)
    }
}

fn sha(bytes: &[u8]) -> String {
    wizard101_chat_translator::util::hex(&Sha256::digest(bytes))
}

fn release(url: &str) -> Release {
    Release {
        version: "1.2.0".to_owned(),
        url: "https://example.invalid/rel".to_owned(),
        asset: Some(Asset {
            url: url.to_owned(),
            size: BODY.len() as u64,
            sha256: sha(BODY),
        }),
        ..Release::default()
    }
}

fn exe_in(dir: &TempDir) -> PathBuf {
    dir.path().join("app.exe")
}

fn host_with(rt: Handle, job: UpdateJob) -> UpdateHost {
    let (poster, _) = UiPoster::for_test();
    let mut host = UpdateHost::with_job(rt, download_client("http://127.0.0.1"), poster, job);
    host.set_window_ops(WindowOps::offscreen());
    host
}

/// 停在 `phase` 的 host（不打網路）。
fn host_in(rt: &Runtime, dir: &TempDir, phase: Phase) -> UpdateHost {
    let releases = vec![release("http://127.0.0.1:1/app.exe")];
    host_with(
        rt.handle().clone(),
        UpdateJob::with_phase(exe_in(dir), releases, phase),
    )
}

/// 真的下載完一次（wiremock）、把結果交給 host：host 進 Ready、下載完成的設定檢查還沒做。
async fn downloaded_host(dir: &TempDir) -> (UpdateHost, MockServer) {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/app.exe"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(BODY))
        .mount(&server)
        .await;
    let mut job = UpdateJob::new(exe_in(dir));
    job.set_releases(vec![release(&format!("{}/app.exe", server.uri()))]);
    let (tx, rx) = oneshot::channel();
    job.start(
        &Handle::current(),
        download_client(&server.uri()),
        Box::new(move |run, result| {
            let _ = tx.send((run, result));
        }),
    );
    let mut host = host_with(Handle::current(), job);
    host.open();
    let (run, result) = rx.await.unwrap();
    host.downloaded(run, result);
    assert_eq!(host.phase(), &Phase::Ready);
    (host, server)
}

#[test]
fn open_without_releases_stays_closed() {
    let rt = Runtime::new().unwrap();
    let dir = TempDir::new().unwrap();
    let mut host = host_with(rt.handle().clone(), UpdateJob::new(exe_in(&dir)));

    host.open();

    assert!(!host.is_open());
}

#[test]
fn open_twice_keeps_one_window() {
    let rt = Runtime::new().unwrap();
    let dir = TempDir::new().unwrap();
    let mut host = host_in(&rt, &dir, Phase::Restarting);
    let mut frames = Frames::new();

    host.open();
    frames.run(&mut host, false, false);
    host.open();
    frames.run(&mut host, false, false);

    assert!(host.is_open());
    assert_eq!(host.phase(), &Phase::Restarting);
    assert_eq!(frames.windows.len(), 1);
}

#[test]
fn close_request_is_cancelled_while_downloading() {
    let rt = Runtime::new().unwrap();
    let dir = TempDir::new().unwrap();
    let mut host = host_in(&rt, &dir, Phase::Downloading);
    let mut frames = Frames::new();
    host.open();
    frames.run(&mut host, false, false);

    let (_, commands) = frames.run(&mut host, false, true);

    assert!(host.is_open());
    assert!(
        commands.contains(&ViewportCommand::CancelClose),
        "{commands:?}"
    );
    assert_eq!(host.phase(), &Phase::Downloading);
}

#[test]
fn closing_when_failed_deletes_the_partial_file() {
    let rt = Runtime::new().unwrap();
    let dir = TempDir::new().unwrap();
    let failed = Phase::Failed {
        error: "x".to_owned(),
        replace: false,
    };
    let mut host = host_in(&rt, &dir, failed);
    std::fs::write(part_path(&exe_in(&dir)), b"x").unwrap();
    let mut frames = Frames::new();
    host.open();
    frames.run(&mut host, false, false);

    let (_, commands) = frames.run(&mut host, false, true);

    assert!(!host.is_open());
    assert!(!commands.contains(&ViewportCommand::CancelClose));
    assert!(!part_path(&exe_in(&dir)).exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn ready_without_unsaved_restarts_on_the_next_frame() {
    let dir = TempDir::new().unwrap();
    let (mut host, _server) = downloaded_host(&dir).await;
    let mut frames = Frames::new();

    let (first, _) = frames.run(&mut host, false, false);
    assert_eq!(host.phase(), &Phase::Restarting);
    assert!(first.is_empty());
    let (second, _) = frames.run(&mut host, false, false);
    assert_eq!(second, [HostAction::Restart]);
    let (third, _) = frames.run(&mut host, false, false);
    assert!(third.is_empty(), "restart is handed over only once");
}

#[tokio::test(flavor = "multi_thread")]
async fn ready_with_unsaved_waits_for_the_user() {
    let dir = TempDir::new().unwrap();
    let (mut host, _server) = downloaded_host(&dir).await;
    let mut frames = Frames::new();

    for _ in 0..3 {
        let (got, _) = frames.run(&mut host, true, false);
        assert!(got.is_empty());
    }

    assert_eq!(host.phase(), &Phase::Ready);
}

#[tokio::test(flavor = "multi_thread")]
async fn reopening_when_ready_does_not_auto_restart() {
    let dir = TempDir::new().unwrap();
    let (mut host, _server) = downloaded_host(&dir).await;
    let mut frames = Frames::new();
    frames.run(&mut host, true, false);
    frames.run(&mut host, true, true);
    assert!(!host.is_open());

    host.open();
    for _ in 0..2 {
        let (got, _) = frames.run(&mut host, false, false);
        assert!(got.is_empty());
    }

    assert_eq!(host.phase(), &Phase::Ready);
}
