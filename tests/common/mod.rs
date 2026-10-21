//! 整合測試共用工具。

use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use wizard101_chat_translator::util::{self, Clock};

pub mod fake_backend;
pub mod fake_game;
pub mod kittest;
pub mod log_capture;
pub mod reader_fixture;
pub mod settings_env;
pub mod settings_window;
pub mod spy_memory;
pub mod wizard;

/// 等背景執行緒或伺服器達成條件的上限：只有失敗時才等滿，放寬到負載重的 CI 也不誤判。
#[allow(dead_code)] // 每個測試執行檔各自編譯 common，不是每個都用到。
pub const WAIT: Duration = Duration::from_secs(30);

/// 輪詢 `condition` 到成立為止，最多等 [`WAIT`]，逾時以 `what` 為訊息 panic。
#[allow(dead_code)] // 每個測試執行檔各自編譯 common，不是每個都用到。
pub fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT;
    let poll = || condition().then_some(());
    let done = util::poll_until(&Clock::default(), deadline, Duration::from_millis(5), poll);
    assert!(done.is_some(), "timed out waiting for {what}");
}

/// 全域介面語言同一時間只給一個測試用（會切換語言的測試檔各自排隊）。
#[allow(dead_code)] // 每個測試執行檔各自編譯 common，不是每個都用到。
pub fn language_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    util::lock(&LOCK)
}

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

/// JSON 文字的內容（含鍵順序與數字字面值）正規化成緊湊字串，比內容時不受排版影響。
#[allow(dead_code)] // 每個測試執行檔各自編譯 common，不是每個都用到。
pub fn json_content(text: &str) -> String {
    let value: serde_json::Value = serde_json::from_str(text).expect("valid json");
    serde_json::to_string(&value).unwrap()
}
