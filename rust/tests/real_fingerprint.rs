//! 手動驗證：以使用者真實檔案的副本算出的收訊快取指紋，須等於 Python 算出的值與快取檔裡存的值。
//!
//! 環境變數 `W101_CONFIG`、`W101_EXAMPLES`、`W101_CACHE` 指向的檔案，測試會先複製到暫存資料夾再讀（`load_config` 可能寫回），原檔不動。
//!
//! `W101_PYTHON_FINGERPRINT` 是 Python 算出的值。
//! 執行：`cargo test --test real_fingerprint -- --ignored`。

use std::path::{Path, PathBuf};

use wizard101_chat_translator::app_main::incoming_fingerprint;
use wizard101_chat_translator::config::services::resolve;
use wizard101_chat_translator::config::{ServiceSlot, Settings, load_config};
use wizard101_chat_translator::translation::examples::{ExampleStore, examples_fingerprint};

/// 把環境變數指向的檔案複製進暫存資料夾，回傳副本路徑。
fn copy_of(key: &str, dir: &Path) -> PathBuf {
    let source = PathBuf::from(std::env::var(key).unwrap_or_else(|_| panic!("{key} is not set")));
    let copy = dir.join(source.file_name().expect("env path has a file name"));
    std::fs::copy(&source, &copy).unwrap_or_else(|e| panic!("copy {key}: {e}"));
    copy
}

#[test]
#[ignore = "reads copies of the user's real files named by env vars"]
fn incoming_fingerprint_matches_python_and_the_stored_cache() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = load_config(&copy_of("W101_CONFIG", dir.path()));
    let target = Settings::from_config(&cfg).target_language;
    let store = ExampleStore::new(copy_of("W101_EXAMPLES", dir.path()));
    let stored = store.get(&examples_fingerprint(
        &resolve(&cfg, ServiceSlot::Incoming),
        &target,
    ));
    let rust = incoming_fingerprint(&cfg, stored.as_ref());

    let python =
        std::env::var("W101_PYTHON_FINGERPRINT").expect("W101_PYTHON_FINGERPRINT is not set");
    assert_eq!(rust, python, "mismatch: rust vs python");
    let cache: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(copy_of("W101_CACHE", dir.path())).unwrap())
            .unwrap();
    assert_eq!(
        Some(rust.as_str()),
        cache["fingerprint"].as_str(),
        "mismatch: rust vs stored cache"
    );
}
