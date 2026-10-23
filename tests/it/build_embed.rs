//! 建置時列出 JSON 檔的產生邏輯（`build_support/embed.rs`）的測試。

#[path = "../../build_support/embed.rs"]
mod embed;

use embed::embedded_json_list;

#[test]
fn lists_only_json_files_sorted_by_name() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["zh-TW.json", "af-ZA.json", "mod.rs", "notes.txt"] {
        std::fs::write(dir.path().join(name), "{}").unwrap();
    }
    let list = embedded_json_list(dir.path());
    let af = dir.path().join("af-ZA.json").display().to_string();
    let zh = dir.path().join("zh-TW.json").display().to_string();
    assert_eq!(
        list,
        format!(
            "&[\n    (\"af-ZA\", include_str!({af:?})),\n    (\"zh-TW\", include_str!({zh:?})),\n]\n"
        )
    );
}
