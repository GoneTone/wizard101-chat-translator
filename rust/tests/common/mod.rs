//! 整合測試共用工具。

use std::path::Path;

/// 讀取 `tests/golden/<name>.json`，回傳 Python 匯出的案例清單。
pub fn golden(name: &str) -> Vec<serde_json::Value> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.json"));
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read golden fixture {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("invalid golden fixture {name}: {e}"))
}
