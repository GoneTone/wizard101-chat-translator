//! 術語表條目解析與內建術語表註冊。

use wizard101_chat_translator::glossary::{
    Conflict, Entry, Glossary, RowIssue, apply_merge, blocks_saving, builtin, entries_to_json,
    hits_key, overridden, parse_entries, plan_merge, review, without_builtin_duplicates,
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

fn review_fixture() -> (Vec<Entry>, Vec<Entry>) {
    (
        vec![
            e("Fire Elf", "火妖精"),
            e("Wizard City", "巫師城"),
            e("hollo", ""),
            e("A", "甲"),
            e("a", "乙"),
            e("RW", "拉文伍德"),
        ],
        vec![
            e("Fire Elf", "火精靈"),
            e("Wizard City", "巫師城"),
            e("Ravenwood", "拉文伍德"),
        ],
    )
}

#[test]
fn review_flags_each_issue() {
    let (custom, builtin) = review_fixture();
    assert_eq!(
        review(&custom, &builtin),
        vec![
            RowIssue::BuiltinDiffers {
                other: "火精靈".into()
            },
            RowIssue::SameAsBuiltin,
            RowIssue::Incomplete,
            RowIssue::Ok,
            RowIssue::DuplicateSource,
            RowIssue::Ok,
        ]
    );
}

#[test]
fn cross_side_conflict_is_flagged() {
    assert_eq!(
        review(&[e("哈樓", "Hollo")], &[e("Hollo", "哈羅")]),
        vec![RowIssue::BuiltinDiffers {
            other: "哈羅".into()
        }]
    );
}

#[test]
fn overridden_marks_only_shadowed_builtins() {
    let (custom, builtin) = review_fixture();
    assert_eq!(overridden(&custom, &builtin), vec![true, true, false]);
}

#[test]
fn blocks_saving_on_duplicates_and_incomplete_rows() {
    assert!(blocks_saving(&[RowIssue::Ok, RowIssue::DuplicateSource]));
    assert!(blocks_saving(&[RowIssue::Incomplete]));
    assert!(!blocks_saving(&[
        RowIssue::Ok,
        RowIssue::SameAsBuiltin,
        RowIssue::BuiltinDiffers { other: "x".into() }
    ]));
}

#[test]
fn without_builtin_duplicates_drops_same_rows() {
    let (custom, builtin) = review_fixture();
    assert_eq!(
        without_builtin_duplicates(&custom, &builtin),
        vec![
            e("Fire Elf", "火妖精"),
            e("hollo", ""),
            e("A", "甲"),
            e("a", "乙"),
            e("RW", "拉文伍德")
        ]
    );
}

#[test]
fn merge_plan_counts_added_identical_and_conflicts() {
    let plan = plan_merge(
        &[e("A", "甲"), e("B", "乙")],
        &[e("a", "甲"), e("B", "丙"), e("C", "丁")],
    );
    assert_eq!(plan.identical, 1);
    assert_eq!(plan.added, vec![e("C", "丁")]);
    assert_eq!(
        plan.conflicts,
        vec![Conflict {
            index: 1,
            current: e("B", "乙"),
            incoming: e("B", "丙")
        }]
    );
}

#[test]
fn apply_merge_takes_only_checked_conflicts() {
    let current = vec![e("A", "甲"), e("B", "乙")];
    let plan = plan_merge(&current, &[e("a", "甲"), e("B", "丙"), e("C", "丁")]);
    let mut kept = current.clone();
    apply_merge(&mut kept, &plan, &[false]);
    assert_eq!(kept, vec![e("A", "甲"), e("B", "乙"), e("C", "丁")]);
    let mut taken = current;
    apply_merge(&mut taken, &plan, &[true]);
    assert_eq!(taken, vec![e("A", "甲"), e("B", "丙"), e("C", "丁")]);
}

#[test]
fn merge_plan_keeps_the_first_of_case_variant_duplicates() {
    let plan = plan_merge(&[], &[e("Hollo", "甲"), e("hollo", "乙")]);
    assert_eq!(plan.added, vec![e("Hollo", "甲")]);
    assert_eq!(plan.identical, 0);
    assert!(plan.conflicts.is_empty());
}
