//! 錯誤說明與 OpenAI 請求內容對照 Python 匯出的 golden fixture。

use crate::common;

use wizard101_chat_translator::translation::Turn;
use wizard101_chat_translator::translation::clients::OpenAiCompatClient;
use wizard101_chat_translator::translation::errors::{error_detail, rejected_parameter};

#[test]
fn error_detail_matches_python() {
    let cases = common::golden("clients_error_detail");
    assert!(cases.len() >= 15);
    for case in cases {
        let input = case["input"].as_str().unwrap();
        assert_eq!(
            error_detail(input),
            case["output"].as_str().unwrap(),
            "{input:?}"
        );
    }
}

#[test]
fn rejected_parameter_matches_python() {
    let cases = common::golden("clients_rejected_parameter");
    assert!(cases.len() >= 6);
    for case in cases {
        let input = case["input"].as_str().unwrap();
        assert_eq!(
            rejected_parameter(input).as_deref(),
            case["output"].as_str(),
            "{input:?}"
        );
    }
}

#[test]
fn request_body_matches_python() {
    let cases = common::golden("clients_openai_body");
    assert_eq!(cases.len(), 16);
    for case in cases {
        let input = &case["input"];
        let turns: Vec<Turn> = input["turns"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| match t["role"].as_str().unwrap() {
                "user" => Turn::user(t["content"].as_str().unwrap()),
                _ => Turn::assistant(t["content"].as_str().unwrap()),
            })
            .collect();
        let client = OpenAiCompatClient::new(
            "http://x",
            "",
            "m",
            input["thinking"].as_bool().unwrap(),
            "auto",
            input["official"].as_bool().unwrap(),
        );
        let body = client.request_body(
            input["system"].as_str().unwrap(),
            &turns,
            input["max_tokens"].as_u64().unwrap() as u32,
            input["deterministic"].as_bool().unwrap(),
        );
        assert_eq!(body, case["output"], "{input}");
    }
}
