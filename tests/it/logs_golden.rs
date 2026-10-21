//! logs 與 Python 版的 golden 對照。

use crate::common;

use chrono::{DateTime, Utc};
use wizard101_chat_translator::logs::{trim_log_sessions, utc_stamp};

#[test]
fn trim_matches_python() {
    let cases = common::golden("logs_trim");
    assert!(!cases.is_empty());
    for case in cases {
        let text = case["input"]["text"].as_str().unwrap();
        let now: DateTime<Utc> = case["input"]["now"].as_str().unwrap().parse().unwrap();
        assert_eq!(
            trim_log_sessions(text, now),
            case["output"].as_str().unwrap(),
            "input={text:?}"
        );
    }
}

#[test]
fn stamp_matches_python() {
    let cases = common::golden("logs_stamp");
    assert!(!cases.is_empty());
    for case in cases {
        let input = case["input"].as_str().unwrap();
        let dt = DateTime::parse_from_rfc3339(input)
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            utc_stamp(dt),
            case["output"].as_str().unwrap(),
            "input={input}"
        );
    }
}
