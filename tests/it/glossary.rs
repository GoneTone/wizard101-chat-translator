//! 術語表條目解析與內建術語表註冊。

use wizard101_chat_translator::glossary::{Entry, builtin, entries_to_json, parse_entries};

fn e(source: &str, target: &str) -> Entry {
    Entry {
        source: source.into(),
        target: target.into(),
    }
}

#[test]
fn parse_keeps_file_order_and_trims() {
    let got = parse_entries(r#"{" Hollo ": "哈樓", "Fire Elf": "火精靈"}"#).unwrap();
    assert_eq!(got, vec![e("Hollo", "哈樓"), e("Fire Elf", "火精靈")]);
}

#[test]
fn parse_skips_blank_sides() {
    assert_eq!(parse_entries(r#"{"": "x", "a": "  "}"#), Ok(vec![]));
}

#[test]
fn parse_rejects_non_string_values() {
    assert!(parse_entries(r#"{"a": 1}"#).is_err());
    assert!(parse_entries("[]").is_err());
    assert!(parse_entries("not json").is_err());
}

#[test]
fn export_round_trips() {
    let entries = vec![e("Hollo", "哈樓"), e("Fire Elf", "火精靈")];
    assert_eq!(parse_entries(&entries_to_json(&entries)), Ok(entries));
}

#[test]
fn builtin_load_drops_untranslated_pairs() {
    let got = builtin::load(r#"{"Fire Elf": "fire elf", "Hollo": "哈樓"}"#);
    assert_eq!(got, vec![e("Hollo", "哈樓")]);
}

#[test]
fn no_builtins_ship_yet() {
    assert!(builtin::builtin_codes().is_empty());
    assert!(builtin::builtin_entries("zh-TW").is_none());
}
