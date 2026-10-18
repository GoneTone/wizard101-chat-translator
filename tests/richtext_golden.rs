//! 連結語法切段與裸網址轉連結對照 Python 匯出的 golden fixture。

mod common;

use serde_json::Value;
use wizard101_chat_translator::ui::richtext::{linkify, parse_link_markup};

fn cases(function: &str) -> Vec<Value> {
    let cases: Vec<Value> = common::golden("richtext")
        .into_iter()
        .filter(|c| c["input"]["fn"] == function)
        .collect();
    assert!(
        cases.len() >= 30,
        "{function} has only {} cases",
        cases.len()
    );
    cases
}

#[test]
fn link_markup_matches_python() {
    for c in cases("parse_link_markup") {
        let text = c["input"]["text"].as_str().unwrap();
        let expected: Vec<(String, Option<String>)> =
            serde_json::from_value(c["output"].clone()).unwrap();
        assert_eq!(parse_link_markup(text), expected, "{text:?}");
    }
}

/// 刻意保留的分歧：Python `re.I` 讓 `[A-Za-z]` 也收 `ı`／`İ`，Rust 的網址只收到 ASCII 為止。
const DIVERGENT: (&str, &str) = (
    "https://a.example/x\u{131}y",
    "[https://a.example/x](https://a.example/x)\u{131}y",
);

#[test]
fn linkify_matches_python() {
    let mut divergent = 0;
    for c in cases("linkify") {
        let text = c["input"]["text"].as_str().unwrap();
        let python = c["output"].as_str().unwrap();
        if text == DIVERGENT.0 {
            divergent += 1;
            assert_ne!(python, DIVERGENT.1, "Python now agrees; drop DIVERGENT");
            assert_eq!(linkify(text), DIVERGENT.1);
        } else {
            assert_eq!(linkify(text), python, "{text:?}");
        }
    }
    assert_eq!(
        divergent, 1,
        "the pinned divergent case is missing from the fixture"
    );
}
