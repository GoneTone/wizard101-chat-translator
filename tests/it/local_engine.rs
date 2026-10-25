//! llama.cpp 引擎的列表解析、安裝與切換（wiremock，不打真網路）。

use std::io::Write;
use std::path::Path;

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use wizard101_chat_translator::local::download::Progress;
use wizard101_chat_translator::local::engine::*;
use wizard101_chat_translator::local::hf::hf_client;

fn asset(name: &str) -> Value {
    json!({"name": name, "size": 10, "browser_download_url": format!("http://x/{name}")})
}

fn fixture() -> Value {
    json!([
        {"tag_name": "b11871", "published_at": "2026-10-01T00:00:00Z", "prerelease": true, "assets": [
            asset("llama-b11871-bin-win-cuda-13.4-x64.zip"),
            asset("llama-b11871-bin-win-cpu-x64.zip"),
            asset("cudart-llama-bin-win-cuda-13.4-x64.zip"),
            asset("llama-b11871-bin-win-vulkan-x64.zip"),
            asset("llama-b11871-bin-macos-arm64.zip")]},
        {"tag_name": "v0.7.0", "assets": [asset("source.zip")]},
        {"tag_name": "b11000", "assets": [asset("llama-b11000-bin-win-cpu-x64.zip")]}
    ])
}

#[test]
fn parse_releases_keeps_windows_builds_only() {
    let builds = parse_releases(&fixture());
    assert_eq!(builds.len(), 1);
    let kinds: Vec<_> = builds[0]
        .variants
        .iter()
        .map(|(v, _)| v.to_string())
        .collect();
    assert_eq!(kinds, ["vulkan", "cpu", "cuda-13.4"]);
    assert_eq!(builds[0].variants[2].1.len(), 2);
    assert_eq!(builds[0].variants[0].1.len(), 1);
}

#[test]
fn builds_older_than_the_minimum_are_hidden() {
    assert!(parse_releases(&fixture()).iter().all(|b| b.tag != "b11000"));
}

#[test]
fn variant_serializes_as_a_string() {
    let v = Variant::Cuda("13.4".into());
    assert_eq!(serde_json::to_string(&v).unwrap(), "\"cuda-13.4\"");
    assert_eq!(
        serde_json::from_str::<Variant>("\"vulkan\"").unwrap(),
        Variant::Vulkan
    );
    assert!(serde_json::from_str::<Variant>("\"metal\"").is_err());
}

fn inst(build: &str) -> EngineInstall {
    EngineInstall {
        build: build.into(),
        variant: Variant::Cpu,
    }
}

fn make_dir(root: &Path, i: &EngineInstall) {
    std::fs::create_dir_all(engine_dir(root, i)).unwrap();
}

#[test]
fn rollback_restores_the_previous_build() {
    let root = tempfile::tempdir().unwrap();
    let (a, b) = (inst("b11871"), inst("b11872"));
    make_dir(root.path(), &a);
    make_dir(root.path(), &b);
    activate(root.path(), a.clone()).unwrap();
    confirm(root.path()).unwrap();
    activate(root.path(), b.clone()).unwrap();
    assert_eq!(rollback(root.path()).unwrap(), Some(a.clone()));
    assert!(!engine_dir(root.path(), &b).exists());
    assert!(engine_dir(root.path(), &a).exists());
    assert_eq!(
        read_active(root.path()),
        ActiveFile {
            current: Some(a),
            previous: None
        }
    );
}

#[test]
fn confirm_deletes_the_previous_build() {
    let root = tempfile::tempdir().unwrap();
    let (a, b) = (inst("b11871"), inst("b11872"));
    make_dir(root.path(), &a);
    make_dir(root.path(), &b);
    activate(root.path(), a.clone()).unwrap();
    activate(root.path(), b.clone()).unwrap();
    confirm(root.path()).unwrap();
    assert!(!engine_dir(root.path(), &a).exists());
    assert_eq!(
        read_active(root.path()),
        ActiveFile {
            current: Some(b),
            previous: None
        }
    );
}

#[test]
fn corrupt_active_file_reads_as_empty() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("llama")).unwrap();
    std::fs::write(root.path().join("llama/active.json"), "{not json").unwrap();
    assert_eq!(read_active(root.path()), ActiveFile::default());
}

fn zip_bytes(entries: &[&str]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(vec![]));
    for name in entries {
        w.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        w.write_all(b"x").unwrap();
    }
    w.finish().unwrap().into_inner()
}

async fn serve(server: &MockServer, name: &str, body: Vec<u8>) -> Asset {
    Mock::given(method("GET"))
        .and(path(format!("/{name}")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone()))
        .mount(server)
        .await;
    Asset {
        name: name.into(),
        url: format!("{}/{name}", server.uri()),
        size: body.len() as u64,
    }
}

async fn install_cpu(
    server: &MockServer,
    root: &Path,
    entries: &[&str],
) -> Result<EngineInstall, String> {
    let a = serve(
        server,
        "llama-b11871-bin-win-cpu-x64.zip",
        zip_bytes(entries),
    )
    .await;
    let build = Build {
        tag: "b11871".into(),
        published: String::new(),
        variants: vec![(Variant::Cpu, vec![a])],
    };
    install(
        &hf_client(&server.uri()),
        root,
        &build,
        &Variant::Cpu,
        &Progress::default(),
        &CancellationToken::new(),
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn install_extracts_the_zip() {
    let server = MockServer::start().await;
    let root = tempfile::tempdir().unwrap();
    let got = install_cpu(&server, root.path(), &["bin/llama-server.exe"])
        .await
        .unwrap();
    assert!(server_exe(&engine_dir(root.path(), &got)).is_some());
    assert_eq!(read_active(root.path()), ActiveFile::default());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_zip_slip_entry_fails_the_install_and_leaves_nothing() {
    let server = MockServer::start().await;
    let root = tempfile::tempdir().unwrap();
    let r = install_cpu(&server, root.path(), &["../evil.txt"]).await;
    assert!(r.is_err(), "{r:?}");
    assert!(!root.path().join("llama/evil.txt").exists());
    assert!(!root.path().join("evil.txt").exists());
    assert!(!root.path().join("llama/b11871-cpu").exists());
    assert!(!root.path().join("llama/b11871-cpu.tmp").exists());
}
