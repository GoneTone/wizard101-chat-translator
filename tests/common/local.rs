//! 本地模型測試共用：假的引擎與模型檔、在指定埠上開 wiremock 當 llama-server。

// 每個測試執行檔各自編譯 common，不是每個都用到
#![allow(dead_code)]

use std::fs;
use std::path::Path;

use tokio::runtime::Handle;
use wiremock::{Mock, MockServer};
use wizard101_chat_translator::local::engine::{self, EngineInstall, Variant};
use wizard101_chat_translator::local::store::{self, Manifest, ManifestFile};

/// 在 llama-server 該聽的埠上開 wiremock 並掛上 `mocks`；須在 multi-thread runtime 裡呼叫。
pub fn serve_on(port: u16, mocks: Vec<Mock>) -> MockServer {
    let listener = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
    let rt = Handle::current();
    tokio::task::block_in_place(|| {
        rt.block_on(async {
            let server = MockServer::builder().listener(listener).start().await;
            for mock in mocks {
                mock.mount(&server).await;
            }
            server
        })
    })
}

/// 寫一個空的 `llama-server.exe` 並設為啟用中的引擎。
pub fn add_engine(root: &Path, build: &str) -> EngineInstall {
    let install = EngineInstall {
        build: build.into(),
        variant: Variant::Cpu,
    };
    let dir = engine::engine_dir(root, &install);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("llama-server.exe"), b"").unwrap();
    engine::activate(root, install.clone()).unwrap();
    install
}

/// 寫一個下載完成的模型（`with_mtp` 時附 MTP 檔）。
pub fn add_model(root: &Path, id: &str, with_mtp: bool) {
    let file = |path: &str, size| ManifestFile {
        path: path.into(),
        size,
        sha256: None,
    };
    let manifest = Manifest {
        repo: "org/Model-GGUF".into(),
        quant: "Q4_K_M".into(),
        files: vec![file("model-Q4_K_M.gguf", 4)],
        mtp: with_mtp.then(|| file("mtp-Q8_0.gguf", 2)),
        moe: false,
        has_builtin_mtp: false,
        required: 1,
    };
    store::write_manifest(root, id, &manifest).unwrap();
    let dir = store::models_dir(root).join(id);
    fs::write(dir.join("model-Q4_K_M.gguf"), b"main").unwrap();
    if with_mtp {
        fs::write(dir.join("mtp-Q8_0.gguf"), b"mt").unwrap();
    }
}
