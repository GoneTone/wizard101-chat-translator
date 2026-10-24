//! 術語表條目解析與內建術語表註冊。

use wizard101_chat_translator::glossary::{
    Entry, Glossary, builtin, entries_to_json, hits_key, parse_entries,
};

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

fn hits(entries: Vec<Entry>, text: &str) -> Vec<Entry> {
    Glossary::new(entries, vec![]).hits(text)
}

fn layered(custom: Vec<Entry>, builtin: Vec<Entry>, text: &str) -> Vec<Entry> {
    Glossary::new(custom, builtin).hits(text)
}

#[test]
fn matches_either_side_case_insensitively() {
    let g = vec![e("Hollo", "哈樓")];
    assert_eq!(hits(g.clone(), "I met HOLLO"), g);
    assert_eq!(hits(g.clone(), "我遇到哈樓了"), g);
}

#[test]
fn latin_terms_need_word_boundaries() {
    let g = vec![e("Hollo", "哈樓")];
    assert_eq!(hits(g.clone(), "Hollow Tower"), vec![]);
    assert_eq!(hits(g.clone(), "我在Hollo旁邊"), g);
}

#[test]
fn unspaced_scripts_use_substring() {
    let g = vec![e("Fire Elf", "파이어엘프")];
    assert_eq!(hits(g.clone(), "파이어엘프가 왔다"), g);
}

#[test]
fn longest_overlap_wins() {
    let g = vec![e("Fire", "火"), e("Fire Elf", "火精靈")];
    assert_eq!(hits(g.clone(), "a Fire Elf"), vec![e("Fire Elf", "火精靈")]);
    assert_eq!(
        hits(g, "Fire Elf and Fire"),
        vec![e("Fire Elf", "火精靈"), e("Fire", "火")]
    );
}

#[test]
fn shorter_term_fills_in_when_longest_fails_boundary() {
    let g = vec![e("Fire Elfx", "甲"), e("Fire", "火")];
    assert_eq!(hits(g, "Fire Elfxy"), vec![e("Fire", "火")]);
}

#[test]
fn custom_overrides_builtin_on_the_shared_source() {
    let (c, b) = (vec![e("Fire Elf", "火妖精")], vec![e("Fire Elf", "火精靈")]);
    assert_eq!(layered(c.clone(), b.clone(), "Fire Elf"), c);
    assert_eq!(layered(c, b.clone(), "火精靈"), b);
}

#[test]
fn shared_target_sends_both() {
    let (c, b) = (vec![e("RW", "拉文伍德")], vec![e("Ravenwood", "拉文伍德")]);
    assert_eq!(
        layered(c.clone(), b.clone(), "拉文伍德"),
        vec![c[0].clone(), b[0].clone()]
    );
    assert_eq!(layered(c, b.clone(), "Ravenwood"), b);
}

#[test]
fn cross_side_shared_word_prefers_custom() {
    let (c, b) = (vec![e("哈樓", "Hollo")], vec![e("Hollo", "哈羅")]);
    assert_eq!(layered(c.clone(), b.clone(), "Hollo"), c);
    assert_eq!(layered(c, b.clone(), "哈羅"), b);
}

#[test]
fn casefold_that_changes_length_does_not_panic() {
    let g = vec![e("STRASSE", "街"), e("İstanbul", "伊斯坦堡")];
    assert!(hits(g, "Straße İstanbul").contains(&e("STRASSE", "街")));
}

#[test]
fn hits_key_is_order_independent() {
    let (a, b) = (e("A", "甲"), e("B", "乙"));
    assert_eq!(hits_key(&[a.clone(), b.clone()]), hits_key(&[b, a]));
    assert_eq!(hits_key(&[]), "");
}
