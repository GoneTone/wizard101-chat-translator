//! 譯文後處理對照 Python 匯出的 golden fixture。

mod common;

use serde_json::Value;
use wizard101_chat_translator::reader::markup::parse_line;
use wizard101_chat_translator::translation::postprocess as pp;

fn s(v: &Value) -> &str {
    v.as_str().unwrap()
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| s(x).to_string())
        .collect()
}

fn cases(name: &str) -> Vec<Value> {
    let cases = common::golden(name);
    assert!(cases.len() >= 15, "{name} has only {} cases", cases.len());
    cases
}

#[test]
fn strip_think_matches_python() {
    for c in cases("postprocess_strip_think") {
        assert_eq!(
            pp::strip_think(s(&c["input"])),
            s(&c["output"]),
            "{}",
            c["input"]
        );
    }
}

#[test]
fn tidy_parentheses_matches_python() {
    for c in cases("postprocess_tidy") {
        let i = &c["input"];
        assert_eq!(
            pp::tidy_parentheses(s(&i["source"]), s(&i["translated"])),
            s(&c["output"]),
            "{i}"
        );
    }
}

#[test]
fn sender_of_matches_python() {
    for c in cases("postprocess_sender") {
        let expected = c["output"].as_str().map(str::to_string);
        assert_eq!(pp::sender_of(s(&c["input"])), expected, "{}", c["input"]);
    }
}

#[test]
fn restore_sender_matches_python() {
    for c in cases("postprocess_restore") {
        let i = &c["input"];
        assert_eq!(
            pp::restore_sender(s(&i["source"]), s(&i["translated"])),
            s(&c["output"]),
            "{i}"
        );
    }
}

#[test]
fn uses_latin_script_matches_python() {
    for c in cases("postprocess_latin") {
        assert_eq!(
            pp::uses_latin_script(s(&c["input"])),
            c["output"].as_bool().unwrap(),
            "{}",
            c["input"]
        );
    }
}

#[test]
fn has_stray_latin_matches_python() {
    for c in cases("postprocess_stray") {
        let i = &c["input"];
        assert_eq!(
            pp::has_stray_latin(s(&i["source"]), s(&i["translated"]), s(&i["target"])),
            c["output"].as_bool().unwrap(),
            "{i}"
        );
    }
}

#[test]
fn line_helpers_match_python() {
    for c in cases("postprocess_lines") {
        let i = &c["input"];
        let text = s(&i["text"]);
        match s(&i["fn"]) {
            "nonblank_lines" => assert_eq!(pp::nonblank_lines(text), strings(&c["output"]), "{i}"),
            "number_lines" => {
                let (lines, numbered) = pp::number_lines(text);
                assert_eq!(lines, strings(&c["output"][0]), "{i}");
                assert_eq!(numbered, s(&c["output"][1]), "{i}");
            }
            other => panic!("unknown fn {other}"),
        }
    }
}

#[test]
fn numbering_matches_python() {
    for c in cases("postprocess_numbering") {
        let i = &c["input"];
        let output = s(&i["output"]);
        match s(&i["fn"]) {
            "numbered_entries" => {
                let actual: Vec<(String, String)> = pp::numbered_entries(output)
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v))
                    .collect();
                let mut expected: Vec<(String, String)> = c["output"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|p| (s(&p[0]).to_string(), s(&p[1]).to_string()))
                    .collect();
                expected.sort_by_key(|(k, _)| k.parse::<u128>().unwrap_or(u128::MAX));
                assert_eq!(actual, expected, "{i}");
            }
            "unnumber_lines" => assert_eq!(
                pp::unnumber_lines(output, &strings(&i["originals"])),
                s(&c["output"]),
                "{i}"
            ),
            other => panic!("unknown fn {other}"),
        }
    }
}

#[test]
fn sender_prefix_limit_is_shared_with_the_chat_parser() {
    let longest = "A".repeat(40);
    let raw = format!(
        "<color;FFFFFF><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> \
         <link;GID:1,{longest},2>[{longest}]</link> 你好 </color>"
    );
    let line = parse_line(&raw).unwrap();
    assert_eq!(line.text, format!("[{longest}] 你好"));
    assert!(!pp::has_stray_latin(
        &line.text,
        &format!("[{longest}] 哈囉"),
        "繁體中文（台灣）"
    ));
}
