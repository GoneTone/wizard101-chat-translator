//! 翻譯錯誤的使用者文案對照 Python 匯出的 golden fixture。

use crate::common;

use wizard101_chat_translator::translation::errors::{TranslateError, friendly_error};

fn error_of(kind: &str, status: Option<u16>, detail: &str) -> TranslateError {
    let detail = detail.to_owned();
    match kind {
        "config" => TranslateError::Config { status, detail },
        "offline" => TranslateError::Offline { status, detail },
        "bad_output" => TranslateError::BadOutput(detail),
        "unexpected" => TranslateError::Unexpected(detail),
        other => panic!("unknown kind {other}"),
    }
}

#[test]
fn friendly_error_matches_python() {
    let cases = common::golden("errors_friendly");
    assert!(cases.len() >= 30);
    for case in cases {
        let input = &case["input"];
        let error = error_of(
            input["kind"].as_str().unwrap(),
            input["status"].as_u64().map(|s| s as u16),
            input["detail"].as_str().unwrap(),
        );
        let (key, args) = friendly_error(&error);
        let expected = case["output"].as_array().unwrap();
        assert_eq!(key, expected[0].as_str().unwrap(), "{input}");
        let expected_args = expected[1].as_object().unwrap();
        assert_eq!(args.len(), expected_args.len(), "{input}");
        for (name, value) in args {
            assert_eq!(value, expected_args[name].as_str().unwrap(), "{input}");
        }
    }
}
