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

#[test]
fn linkify_matches_python() {
    for c in cases("linkify") {
        let text = c["input"]["text"].as_str().unwrap();
        assert_eq!(linkify(text), c["output"].as_str().unwrap(), "{text:?}");
    }
}
