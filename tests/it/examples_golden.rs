//! 範例集的 repr／digest／解析／快取檔對照 Python 匯出的 golden fixture。

use crate::common;

use serde_json::Value;
use wizard101_chat_translator::config::ServiceSpec;
use wizard101_chat_translator::py::pyrepr::repr_str;
use wizard101_chat_translator::translation::Turn;
use wizard101_chat_translator::translation::examples::{
    EXAMPLE_REVISION, ExampleSet, ExampleStore, GAME_LANGUAGE_EXAMPLES, MAX_ENTRIES, SOURCE_LINES,
    examples_fingerprint, generation_request, parse_generated,
};

fn s(v: &Value) -> &str {
    v.as_str().unwrap()
}

fn pair(v: &Value) -> Option<(String, String)> {
    v.as_array()
        .map(|p| (s(&p[0]).to_string(), s(&p[1]).to_string()))
}

fn set_of(v: &Value) -> ExampleSet {
    ExampleSet {
        incoming: pair(&v["incoming"]),
        system: pair(&v["system"]),
        region: pair(&v["region"]),
    }
}

fn pair_set(a: &str) -> ExampleSet {
    ExampleSet {
        incoming: Some((a.into(), "o".into())),
        ..ExampleSet::default()
    }
}

#[test]
fn repr_matches_python() {
    let cases = common::golden("pyrepr");
    assert_eq!(cases.len(), 200);
    for c in cases {
        assert_eq!(repr_str(s(&c["input"])), s(&c["output"]), "{}", c["input"]);
    }
}

#[test]
fn digest_matches_python() {
    let cases = common::golden("examples_digest");
    assert!(cases.len() >= 7);
    for c in cases {
        let set = set_of(&c["input"]);
        assert_eq!(set.digest(), s(&c["output"]["digest"]), "{}", c["input"]);
    }
}

#[test]
fn parse_matches_python() {
    let cases = common::golden("examples_parse");
    assert!(cases.len() >= 20);
    for c in cases {
        let (set, failures) = parse_generated(s(&c["input"]));
        let out = &c["output"];
        assert_eq!(set, set_of(out), "{}", c["input"]);
        let expected: Vec<&str> = out["failures"].as_array().unwrap().iter().map(s).collect();
        assert_eq!(failures, expected, "{}", c["input"]);
    }
}

#[test]
fn store_writes_the_golden_content() {
    let cases = common::golden("examples_store_bytes");
    assert!(cases.len() >= 5);
    for c in cases {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("translation-examples.json");
        let store = ExampleStore::new(path.clone());
        for put in c["input"].as_array().unwrap() {
            store.put(s(&put["fingerprint"]), &set_of(&put["examples"]));
        }
        let actual = std::fs::read_to_string(&path)
            .ok()
            .map(|t| common::json_content(&t));
        let expected = c["output"].as_str().map(common::json_content);
        assert_eq!(actual, expected, "{}", c["input"]);
    }
}

#[test]
fn reads_an_old_format_store() {
    for c in common::golden("examples_store_bytes") {
        let Some(text) = c["output"].as_str() else {
            continue;
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("translation-examples.json");
        std::fs::write(&path, text).unwrap();
        let store = ExampleStore::new(path);
        let mut last = std::collections::HashMap::new();
        for put in c["input"].as_array().unwrap() {
            last.insert(s(&put["fingerprint"]), set_of(&put["examples"]));
        }
        let kept = last.len().min(MAX_ENTRIES);
        let found = last
            .iter()
            .filter(|(fp, set)| store.get(fp).as_ref() == Some(set))
            .count();
        assert_eq!(found, kept, "{}", c["input"]);
    }
}

#[test]
fn store_evicts_lru_beyond_32() {
    let dir = tempfile::tempdir().unwrap();
    let store = ExampleStore::new(dir.path().join("ex.json"));
    for i in 0..=MAX_ENTRIES {
        store.put(&format!("fp-{i}"), &pair_set("s"));
    }
    assert!(store.get("fp-0").is_none() && store.get(&format!("fp-{MAX_ENTRIES}")).is_some());
}

#[test]
fn re_putting_moves_to_the_end() {
    let dir = tempfile::tempdir().unwrap();
    let store = ExampleStore::new(dir.path().join("ex.json"));
    for i in 0..MAX_ENTRIES {
        store.put(&format!("fp-{i}"), &pair_set("s"));
    }
    store.put("fp-0", &pair_set("s"));
    store.put("fp-new", &pair_set("s"));
    assert!(store.get("fp-0").is_some() && store.get("fp-1").is_none());
}

#[test]
fn store_round_trips_and_ignores_bad_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ex.json");
    let store = ExampleStore::new(path.clone());
    let (a, b) = (
        pair_set("s"),
        ExampleSet {
            system: Some(("s".into(), "o".into())),
            ..ExampleSet::default()
        },
    );
    store.put("fp-a", &a);
    store.put("fp-b", &b);
    assert_eq!(ExampleStore::new(path.clone()).get("fp-a"), Some(a));
    assert_eq!(store.get("fp-b"), Some(b));
    assert!(store.get("fp-c").is_none());

    let bad = r#"{"entries": {"fp": {"incoming": "ab", "system": null, "region": null}}}"#;
    for text in [bad, "{not json"] {
        std::fs::write(&path, text).unwrap();
        assert!(store.get("fp").is_none());
    }
}

#[test]
fn store_does_not_write_an_empty_set() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ex.json");
    ExampleStore::new(path.clone()).put("fp", &ExampleSet::default());
    assert!(!path.exists());
}

#[test]
fn a_failed_write_leaves_no_temp_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ex.json");
    std::fs::create_dir(&path).unwrap();
    ExampleStore::new(path).put("fp", &pair_set("s"));
    let names: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
    assert_eq!(names.len(), 1);
}

#[test]
fn fingerprint_has_service_target_and_revision_but_no_key() {
    let spec = ServiceSpec {
        provider: "custom".into(),
        base_url: "https://x".into(),
        model: "m".into(),
        api_key: "secret".into(),
        ..ServiceSpec::default()
    };
    let fp = examples_fingerprint(&spec, " 日本語 ");
    assert_eq!(fp, format!("custom|https://x|m|日本語|e{EXAMPLE_REVISION}"));
}

#[test]
fn generation_request_names_the_target_and_numbers_the_sources() {
    let (system, turns) = generation_request("Deutsch");
    assert!(system.contains("Deutsch") && system.contains("1. [Amy]") && system.contains("{0}"));
    let numbered: Vec<String> = SOURCE_LINES
        .iter()
        .enumerate()
        .map(|(i, l)| format!("{}. {l}", i + 1))
        .collect();
    assert_eq!(turns, vec![Turn::user(numbered.join("\n"))]);
}

#[test]
fn game_language_examples_are_complete_and_bare_label() {
    let g = &*GAME_LANGUAGE_EXAMPLES;
    assert!(g.complete());
    let (source, output) = g.region.as_ref().unwrap();
    assert_eq!(source.lines().nth(2), Some("选项"));
    assert_eq!(output.lines().nth(2), Some("Options"));
    assert!(source == source.trim_matches('\n') && output == output.trim_matches('\n'));
}

#[test]
fn merge_summary_and_digest() {
    let a = pair_set("s");
    let b = ExampleSet {
        system: Some(("s2".into(), "o2".into())),
        ..pair_set("x")
    };
    let merged = a.merge(&b);
    assert_eq!(merged.incoming, a.incoming);
    assert_eq!(merged.system, b.system);
    assert_eq!(a.summary(), "incoming=ok system=failed region=failed");
    assert_ne!(a.digest(), ExampleSet::default().digest());
}

#[test]
fn game_language_examples_match_python() {
    let cases = common::golden("examples_game_language");
    let out = &cases[0]["output"];
    assert_eq!(*GAME_LANGUAGE_EXAMPLES, set_of(out));
    // digest 是 repr 的 sha1，相等即 repr 逐字相同；它會進使用者的譯文快取指紋
    assert_eq!(GAME_LANGUAGE_EXAMPLES.digest(), s(&out["digest"]));
}

#[test]
fn generation_request_matches_python() {
    let cases = common::golden("examples_generation_request");
    assert_eq!(cases.len(), 5);
    for c in cases {
        let (system, turns) = generation_request(s(&c["input"]));
        assert_eq!(system, s(&c["output"]["system"]), "{}", c["input"]);
        let expected: Vec<Turn> = c["output"]["turns"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| {
                assert_eq!(s(&t["role"]), "user");
                Turn::user(s(&t["content"]))
            })
            .collect();
        assert_eq!(turns, expected, "{}", c["input"]);
    }
}
