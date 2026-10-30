//! llama.cpp 引擎的列表解析、安裝與切換（wiremock，不打真網路）。

use std::io::Write;
use std::path::Path;
use std::sync::atomic::Ordering;

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use wizard101_chat_translator::local::download::Progress;
use wizard101_chat_translator::local::engine::*;
use wizard101_chat_translator::local::hardware::{Gpu, GpuKind, NVIDIA};
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

/// NVIDIA 獨顯挑卡與驅動撐得起的最新 CUDA 版；舊卡不用 CUDA 13，驅動太舊、讀不到或別家 GPU 用 Vulkan，沒 GPU 用 CPU。
#[test]
fn auto_variant_prefers_cuda_the_nvidia_driver_supports() {
    let mut json = fixture();
    let assets = json[0]["assets"].as_array_mut().unwrap();
    assets.push(asset("llama-b11871-bin-win-cuda-12.4-x64.zip"));
    assets.push(asset("cudart-llama-bin-win-cuda-12.4-x64.zip"));
    let builds = parse_releases(&json);
    let build = Some(&builds[0]);
    const RTX_4080_SUPER: u32 = 0x2702;
    const GTX_1080: u32 = 0x1b80;
    let gpu = |vendor, device, driver| Gpu {
        name: "GPU".into(),
        vendor,
        device,
        kind: GpuKind::Discrete,
        nvidia_driver: driver,
    };
    let pick = |g: Option<Gpu>| auto_variant(g.as_ref(), build).to_string();
    let rtx = |driver| pick(Some(gpu(NVIDIA, RTX_4080_SUPER, driver)));
    assert_eq!(rtx(Some(58157)), "cuda-13.4");
    assert_eq!(rtx(Some(56094)), "cuda-12.4");
    assert_eq!(rtx(Some(52000)), "vulkan");
    assert_eq!(rtx(None), "vulkan");
    assert_eq!(pick(Some(gpu(NVIDIA, GTX_1080, Some(58157)))), "cuda-12.4");
    assert_eq!(pick(Some(gpu(0x1002, 0x744c, None))), "vulkan");
    assert_eq!(pick(None), "cpu");
    let no_build = auto_variant(Some(&gpu(NVIDIA, RTX_4080_SUPER, Some(58157))), None);
    assert_eq!(no_build, Variant::Vulkan);
    // 少了 cudart 的 CUDA 版不自動選
    let mut bare = fixture();
    let assets = bare[0]["assets"].as_array_mut().unwrap();
    assets.retain(|a| !a["name"].as_str().unwrap().starts_with("cudart-"));
    let builds = parse_releases(&bare);
    let picked = auto_variant(
        Some(&gpu(NVIDIA, RTX_4080_SUPER, Some(58157))),
        Some(&builds[0]),
    );
    assert_eq!(picked, Variant::Vulkan);
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

/// 新版還沒確認就又換版：刪掉沒確認過的那版，備援留確認過的，不留孤兒資料夾。
#[test]
fn switching_again_before_confirming_keeps_the_confirmed_fallback() {
    let root = tempfile::tempdir().unwrap();
    let (a, b, c) = (inst("b11871"), inst("b11872"), inst("b11873"));
    for i in [&a, &b, &c] {
        make_dir(root.path(), i);
    }
    activate(root.path(), a.clone()).unwrap();
    confirm(root.path()).unwrap();
    activate(root.path(), b.clone()).unwrap();
    activate(root.path(), c.clone()).unwrap();
    assert!(!engine_dir(root.path(), &b).exists());
    assert_eq!(
        read_active(root.path()),
        ActiveFile {
            current: Some(c.clone()),
            previous: Some(a.clone())
        }
    );
    confirm(root.path()).unwrap();
    assert!(!engine_dir(root.path(), &a).exists());
    assert!(engine_dir(root.path(), &c).exists());
}

/// 沒確認過的版本重裝自己：資料夾就是剛裝好的，不能刪。
#[test]
fn reinstalling_the_unconfirmed_build_keeps_it() {
    let root = tempfile::tempdir().unwrap();
    let (a, b) = (inst("b11871"), inst("b11872"));
    make_dir(root.path(), &a);
    make_dir(root.path(), &b);
    activate(root.path(), a.clone()).unwrap();
    confirm(root.path()).unwrap();
    activate(root.path(), b.clone()).unwrap();
    activate(root.path(), b.clone()).unwrap();
    assert!(engine_dir(root.path(), &b).exists());
    assert_eq!(read_active(root.path()).previous, Some(a));
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

/// CUDA 版分成兩個 zip：進度是整體的，第二個下載途中也不會歸零或倒退。
#[tokio::test(flavor = "multi_thread")]
async fn progress_covers_every_zip_of_the_variant() {
    let server = MockServer::start().await;
    let root = tempfile::tempdir().unwrap();
    let main = serve(
        &server,
        "llama-b11871-bin-win-cuda-13.4-x64.zip",
        zip_bytes(&["llama-server.exe"]),
    )
    .await;
    let name = "cudart-llama-bin-win-cuda-13.4-x64.zip";
    let body = zip_bytes(&["cudart64_13.dll", "cublas64_13.dll"]);
    Mock::given(method("GET"))
        .and(path(format!("/{name}")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(body.clone())
                .set_delay(std::time::Duration::from_millis(400)),
        )
        .mount(&server)
        .await;
    let runtime = Asset {
        name: name.into(),
        url: format!("{}/{name}", server.uri()),
        size: body.len() as u64,
    };
    let first = main.size;
    let total = main.size + runtime.size;
    let cuda = Variant::Cuda("13.4".into());
    let build = Build {
        tag: "b11871".into(),
        published: String::new(),
        variants: vec![(cuda.clone(), vec![main, runtime])],
    };
    let progress = std::sync::Arc::new(Progress::default());
    let watched = progress.clone();
    let sampler = tokio::spawn(async move {
        let mut samples = vec![];
        while samples.len() < 40 {
            samples.push(watched.done.load(Ordering::Relaxed));
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        samples
    });
    install(
        &hf_client(&server.uri()),
        root.path(),
        &build,
        &cuda,
        &progress,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let samples = sampler.await.unwrap();
    assert!(samples.windows(2).all(|w| w[0] <= w[1]), "{samples:?}");
    assert!(
        samples.iter().any(|&d| d >= first && d < total),
        "never sampled the second zip: {samples:?}"
    );
    assert_eq!(progress.total.load(Ordering::Relaxed), total);
    assert_eq!(progress.done.load(Ordering::Relaxed), total);
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

#[test]
fn rollback_without_a_different_previous_keeps_everything() {
    let root = tempfile::tempdir().unwrap();
    let a = inst("b11871");
    make_dir(root.path(), &a);
    activate(root.path(), a.clone()).unwrap();
    let want = ActiveFile {
        current: Some(a.clone()),
        previous: None,
    };
    assert_eq!(rollback(root.path()).unwrap(), None);
    assert!(engine_dir(root.path(), &a).exists());
    assert_eq!(read_active(root.path()), want);

    activate(root.path(), a.clone()).unwrap();
    assert_eq!(rollback(root.path()).unwrap(), None);
    assert!(engine_dir(root.path(), &a).exists());
    assert_eq!(read_active(root.path()).current, Some(a));
}

#[tokio::test(flavor = "multi_thread")]
async fn install_skips_the_download_when_already_installed() {
    let server = MockServer::start().await;
    let root = tempfile::tempdir().unwrap();
    install_cpu(&server, root.path(), &["bin/llama-server.exe"])
        .await
        .unwrap();
    let before = server.received_requests().await.unwrap().len();
    install_cpu(&server, root.path(), &["bin/llama-server.exe"])
        .await
        .unwrap();
    assert_eq!(server.received_requests().await.unwrap().len(), before);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancelled_install_leaves_nothing() {
    let server = MockServer::start().await;
    let root = tempfile::tempdir().unwrap();
    let a = serve(
        &server,
        "llama-b11871-bin-win-cpu-x64.zip",
        zip_bytes(&["bin/llama-server.exe"]),
    )
    .await;
    let build = Build {
        tag: "b11871".into(),
        published: String::new(),
        variants: vec![(Variant::Cpu, vec![a])],
    };
    let cancel = CancellationToken::new();
    cancel.cancel();
    let r = install(
        &hf_client(&server.uri()),
        root.path(),
        &build,
        &Variant::Cpu,
        &Progress::default(),
        &cancel,
    )
    .await;
    assert!(r.is_err());
    assert!(!root.path().join("llama/b11871-cpu").exists());
    assert!(!root.path().join("llama/b11871-cpu.tmp").exists());
}
