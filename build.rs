//! 把 per-monitor DPI 與 Common Controls v6 的 manifest、app 圖示（工作列讀 exe 資源）與版本資訊編進 exe，
//! 並列出要編進 exe 的介面語言檔。

include!("build_support/embed.rs");
include!("build_support/version.rs");

use std::path::PathBuf;
use std::{env, fs};

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let manifest = manifest_dir.join("app.manifest");
    let icon = manifest_dir.join("assets/icon.ico");

    println!("cargo:rerun-if-changed=build_support/version.rs");
    println!("cargo:rerun-if-changed=app.manifest");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=build_support/embed.rs");
    println!("cargo:rerun-if-changed=src/i18n");
    println!("cargo:rerun-if-changed=src/glossary");

    fs::write(
        out_dir.join("i18n_catalogs.rs"),
        embedded_json_list(&manifest_dir.join("src/i18n")),
    )
    .unwrap();

    fs::write(
        out_dir.join("glossary_builtins.rs"),
        embedded_json_list(&manifest_dir.join("src/glossary")),
    )
    .unwrap();

    let rc = version_rc(
        &env::var("CARGO_PKG_VERSION").unwrap(),
        &manifest.display().to_string(),
        &icon.display().to_string(),
    );
    let rc_path = out_dir.join("app.rc");
    fs::write(&rc_path, rc).unwrap();
    embed_resource::compile(rc_path, embed_resource::NONE)
        .manifest_required()
        .unwrap();
}
