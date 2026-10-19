//! 系統語言配對對照 langcodes 3.5.1 的 `closest_match`（產生方式見 Python 測試對照文件附錄）。

use crate::common;

use serde_json::Value;
use wizard101_chat_translator::i18n;

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn closest_regional_catalog_matches_python() {
    let mut mismatches = Vec::new();
    for case in common::golden("i18n_closest_match") {
        let offered = strings(&case["input"]["offered"]);
        let tag_lists = case["input"]["tags"].as_array().unwrap();
        for (tags, expected) in tag_lists.iter().zip(strings(&case["output"])) {
            let tags = strings(tags);
            let actual = i18n::best_match_among(&tags, &offered);
            if actual != expected {
                mismatches.push(format!("{tags:?} in {offered:?}: {actual} != {expected}"));
            }
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}
