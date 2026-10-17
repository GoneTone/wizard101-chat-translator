//! 版本比較對照 Python 匯出的 golden fixture。

#[allow(dead_code)]
mod common;

use wizard101_chat_translator::runtime::updater::is_newer;

#[test]
fn is_newer_matches_python() {
    let cases = common::golden("updater_is_newer");
    assert!(cases.len() >= 25);
    for case in cases {
        let pair = case["input"].as_array().unwrap();
        let (latest, current) = (pair[0].as_str().unwrap(), pair[1].as_str().unwrap());
        assert_eq!(
            is_newer(latest, current),
            case["output"].as_bool().unwrap(),
            "{latest:?} vs {current:?}"
        );
    }
}
