//! 提示詞與對話輪組裝對照 Python 匯出的 golden fixture（逐 byte 相同）。

use crate::common;

use serde_json::Value;
use wizard101_chat_translator::translation::Turn;
use wizard101_chat_translator::translation::prompts::{self as p, FEWSHOT_OUTGOING};

fn turns_of(v: &Value) -> Vec<Turn> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|t| match t["role"].as_str().unwrap() {
            "user" => Turn::user(t["content"].as_str().unwrap()),
            "assistant" => Turn::assistant(t["content"].as_str().unwrap()),
            other => panic!("unknown role {other}"),
        })
        .collect()
}

#[test]
fn system_prompts_match_python() {
    let cases = common::golden("prompts_systems");
    assert!(cases.len() >= 20);
    for case in cases {
        let input = &case["input"];
        let target = input["target"].as_str().unwrap();
        let actual = match input["kind"].as_str().unwrap() {
            "incoming" => p::build_incoming_system(target),
            "outgoing" => p::build_outgoing_system(target),
            "region" => p::build_region_system(target),
            "system_message" => {
                p::build_system_message_system(target, input["strict"].as_bool().unwrap())
            }
            other => panic!("unknown kind {other}"),
        };
        assert_eq!(actual, case["output"].as_str().unwrap(), "{input}");
    }
}

#[test]
fn turns_and_helpers_match_python() {
    for case in common::golden("prompts_turns") {
        let input = &case["input"];
        let out = &case["output"];
        match input["fn"].as_str().unwrap() {
            "build_turns" => {
                let context: Vec<String> = input["context"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|s| s.as_str().unwrap().to_string())
                    .collect();
                let actual = p::build_turns(
                    &context,
                    input["text"].as_str().unwrap(),
                    input["intro"].as_str().unwrap(),
                    &turns_of(&input["examples"]),
                );
                assert_eq!(actual, turns_of(out), "{input}");
            }
            "example_turns" => {
                let pair = input["pair"].as_array().map(|a| {
                    (
                        a[0].as_str().unwrap().to_string(),
                        a[1].as_str().unwrap().to_string(),
                    )
                });
                assert_eq!(p::example_turns(pair.as_ref()), turns_of(out), "{input}");
            }
            "is_game_language" => {
                let lang = input["lang"].as_str().unwrap();
                assert_eq!(
                    p::is_game_language(lang),
                    out.as_bool().unwrap(),
                    "{lang:?}"
                );
            }
            "constants" => {
                assert_eq!(
                    p::OUTGOING_LANGUAGE,
                    out["outgoing_language"].as_str().unwrap()
                );
                assert_eq!(
                    u64::from(p::PROMPT_REVISION),
                    out["prompt_revision"].as_u64().unwrap()
                );
                assert_eq!(
                    p::CONTEXT_INTRO_INCOMING,
                    out["intro_incoming"].as_str().unwrap()
                );
                assert_eq!(
                    p::CONTEXT_INTRO_OUTGOING,
                    out["intro_outgoing"].as_str().unwrap()
                );
                assert_eq!(p::CONTEXT_ACK, out["ack"].as_str().unwrap());
                assert_eq!(p::fewshot_outgoing_turns(), turns_of(&out["fewshot"]));
                assert_eq!(
                    FEWSHOT_OUTGOING.len() * 2,
                    out["fewshot"].as_array().unwrap().len()
                );
            }
            other => panic!("unknown fn {other}"),
        }
    }
}
