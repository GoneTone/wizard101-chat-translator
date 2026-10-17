//! 把 per-monitor DPI 與 Common Controls v6 的 manifest、以及 app 圖示（工作列讀 exe 資源）編進 exe。

fn main() {
    println!("cargo:rerun-if-changed=app.rc");
    println!("cargo:rerun-if-changed=app.manifest");
    println!("cargo:rerun-if-changed=../src/assets/icon.ico");
    embed_resource::compile("app.rc", embed_resource::NONE)
        .manifest_required()
        .unwrap();
}
