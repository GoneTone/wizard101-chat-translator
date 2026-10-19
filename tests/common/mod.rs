//! 整合測試共用工具。

use std::path::Path;
use std::time::Duration;

pub mod fake_backend;
pub mod fake_game;
pub mod log_capture;
pub mod reader_fixture;
pub mod settings_env;
pub mod wizard;

/// 等背景執行緒或伺服器達成條件的上限：只有失敗時才等滿，放寬到負載重的 CI 也不誤判。
#[allow(dead_code)] // 每個測試執行檔各自編譯 common，不是每個都用到。
pub const WAIT: Duration = Duration::from_secs(30);

/// 讀取 `tests/golden/<name>.json`，回傳 Python 匯出的案例清單。
#[allow(dead_code)] // 每個測試執行檔各自編譯 common，不是每個都用到。
pub fn golden(name: &str) -> Vec<serde_json::Value> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.json"));
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read golden fixture {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("invalid golden fixture {name}: {e}"))
}

/// 把 golden fixture 裡的 hex 字串解回位元組。
#[allow(dead_code)] // 每個測試執行檔各自編譯 common，不是每個都用到。
pub fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
