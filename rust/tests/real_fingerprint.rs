//! 手動驗證：以使用者真實檔案的副本算出的收訊快取指紋，須等於 Python 算出的值與快取檔裡存的值。
//!
//! 只讀環境變數指向的副本：`W101_CONFIG`、`W101_EXAMPLES`、`W101_CACHE`、`W101_PYTHON_FINGERPRINT`。
//! 執行：`cargo test --test real_fingerprint -- --ignored`。

use std::path::PathBuf;

use wizard101_chat_translator::app_main::incoming_fingerprint;
use wizard101_chat_translator::config::services::resolve;
use wizard101_chat_translator::config::{ServiceSlot, Settings, load_config};
use wizard101_chat_translator::translation::examples::{ExampleStore, examples_fingerprint};

fn env_path(key: &str) -> PathBuf {
    PathBuf::from(std::env::var(key).unwrap_or_else(|_| panic!("{key} is not set")))
}

#[test]
#[ignore = "reads copies of the user's real files named by env vars"]
fn incoming_fingerprint_matches_python_and_the_stored_cache() {
    let cfg = load_config(&env_path("W101_CONFIG"));
    let target = Settings::from_config(&cfg).target_language;
    let store = ExampleStore::new(env_path("W101_EXAMPLES"));
    let stored = store.get(&examples_fingerprint(
        &resolve(&cfg, ServiceSlot::Incoming),
        &target,
    ));
    let rust = incoming_fingerprint(&cfg, stored.as_ref());

    let python =
        std::env::var("W101_PYTHON_FINGERPRINT").expect("W101_PYTHON_FINGERPRINT is not set");
    assert_eq!(rust, python, "mismatch: rust vs python");
    let cache: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(env_path("W101_CACHE")).unwrap()).unwrap();
    assert_eq!(
        Some(rust.as_str()),
        cache["fingerprint"].as_str(),
        "mismatch: rust vs stored cache"
    );
}
