//! 術語表條目解析與內建術語表註冊。

use wizard101_chat_translator::glossary::{
    Conflict, Entry, Glossary, RowIssue, apply_merge, builtin, entries_to_json, hits_key,
    overridden, parse_entries, plan_merge, review, without_builtin_duplicates,
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
fn whitespace_variant_sources_are_duplicates() {
    assert_eq!(
        review(&[e("Hollo", "哈樓"), e("Hollo ", "哈囉")], &[]),
        vec![RowIssue::Ok, RowIssue::DuplicateSource]
    );
    assert_eq!(review(&[e(" ", "哈樓")], &[]), vec![RowIssue::Incomplete]);
}

#[test]
fn reversed_identical_entry_is_same_as_builtin() {
    let (c, b) = (vec![e("哈樓", "Hollo")], vec![e("Hollo", "哈樓")]);
    assert_eq!(review(&c, &b), vec![RowIssue::SameAsBuiltin]);
    assert!(without_builtin_duplicates(&c, &b).is_empty());
    assert_eq!(layered(c.clone(), b.clone(), "Hollo"), c);
    assert_eq!(layered(c.clone(), b, "哈樓"), c);
}

#[test]
fn review_handles_large_glossaries() {
    let custom: Vec<Entry> = (0..2_000)
        .map(|i| e(&format!("custom {i}"), &format!("自訂 {i}")))
        .collect();
    let builtin: Vec<Entry> = (0..5_000)
        .map(|i| e(&format!("builtin {i}"), &format!("內建 {i}")))
        .collect();
    let start = std::time::Instant::now();
    assert!(review(&custom, &builtin).iter().all(|i| *i == RowIssue::Ok));
    assert!(!overridden(&custom, &builtin).contains(&true));
    let elapsed = start.elapsed();
    assert!(elapsed < std::time::Duration::from_secs(1), "{elapsed:?}");
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

mod config_glossaries {
    use serde_json::{Value, json};
    use wizard101_chat_translator::config::{Config, default_config, glossaries};

    fn cfg_with(glossaries: Value, active: Value) -> Config {
        let mut cfg = default_config();
        cfg.insert("glossaries".into(), glossaries);
        cfg.insert("active_glossary".into(), active);
        cfg
    }

    #[test]
    fn default_config_has_no_glossaries() {
        let cfg = default_config();
        assert_eq!(cfg["glossaries"], json!([]));
        assert!(cfg["active_glossary"].is_null());
    }

    #[test]
    fn malformed_glossaries_are_normalized() {
        let mut cfg = cfg_with(
            json!([
                {"id": "a", "name": "X", "builtin": 5, "entries": {"k": "v", "n": 1}},
                3,
                {"id": "a", "name": "X", "entries": []},
            ]),
            json!("zz"),
        );
        assert!(glossaries::normalize(&mut cfg));
        let list = cfg["glossaries"].as_array().unwrap();
        assert_eq!(list.len(), 2);
        assert_ne!(list[1]["id"], "a");
        assert_eq!(list[0]["name"], "X");
        assert_eq!(list[1]["name"], "X (2)");
        assert!(list[0]["builtin"].is_null());
        assert_eq!(list[0]["entries"], json!({"k": "v"}));
        assert_eq!(list[1]["entries"], json!({}));
        assert!(cfg["active_glossary"].is_null());
    }

    #[test]
    fn glossaries_not_a_list_becomes_empty() {
        let mut cfg = cfg_with(json!({}), Value::Null);
        assert!(glossaries::normalize(&mut cfg));
        assert_eq!(cfg["glossaries"], json!([]));
    }

    #[test]
    fn well_formed_glossaries_are_untouched() {
        let mut cfg = cfg_with(
            json!([{"id": "a", "name": "X", "builtin": null, "entries": {"k": "v"}}]),
            json!("a"),
        );
        let before = cfg.clone();
        assert!(!glossaries::normalize(&mut cfg));
        assert_eq!(cfg, before);
    }

    #[test]
    fn active_returns_the_selected_glossary() {
        let cfg = cfg_with(
            json!([{"id": "a", "name": "X", "builtin": null, "entries": {"Hollo": "哈樓"}}]),
            json!("a"),
        );
        assert!(!glossaries::active(&cfg).unwrap().hits("Hollo").is_empty());
        assert!(glossaries::active(&default_config()).is_none());
    }

    #[test]
    fn active_ignores_a_missing_builtin() {
        let cfg = cfg_with(
            json!([{"id": "a", "name": "X", "builtin": "xx-XX", "entries": {"Hollo": "哈樓"}}]),
            json!("a"),
        );
        assert!(!glossaries::active(&cfg).unwrap().hits("Hollo").is_empty());
    }

    #[test]
    fn missing_builtin_key_is_not_a_change() {
        let mut cfg = cfg_with(
            json!([{"id": "a", "name": "X", "entries": {"k": "v"}}]),
            json!("a"),
        );
        assert!(!glossaries::normalize(&mut cfg));
        assert!(cfg["glossaries"][0]["builtin"].is_null());
    }
}

mod system_cache {
    use std::sync::Arc;

    use crate::common::fake_backend::{Fake, spec};
    use wizard101_chat_translator::glossary::{Entry, Glossary};
    use wizard101_chat_translator::translation::cache::{
        TranslationCache, lookup, translate_and_cache,
    };
    use wizard101_chat_translator::translation::translator::Translator;

    const KEY: &str = "Hollo <=> 哈樓";

    fn hollo() -> Option<Arc<Glossary>> {
        let entry = Entry {
            source: "Hollo".into(),
            target: "哈樓".into(),
        };
        Some(Arc::new(Glossary::new(vec![entry], Vec::new())))
    }

    fn cache(dir: &tempfile::TempDir) -> TranslationCache {
        TranslationCache::new(dir.path().join("cache.json"), "fp".into())
    }

    #[test]
    fn glossary_hits_get_their_own_cache_entry() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache(&dir);
        assert!(cache.put("Hollo waves", "", "甲", "fp"));
        assert!(cache.put("Hollo waves", KEY, "乙", "fp"));
        assert_eq!(cache.get("Hollo waves", "").as_deref(), Some("甲"));
        assert_eq!(cache.get("Hollo waves", KEY).as_deref(), Some("乙"));
    }

    #[tokio::test]
    async fn lookup_and_store_use_the_same_template_key() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache(&dir);
        let fake = Fake::texts(&["哈樓獲得了 {0} 金幣"]);
        let fake2 = Arc::clone(&fake);
        let tr = Translator::with_client(move |_| fake2.clone(), spec("m"), "繁體中文".into());
        tr.set_glossary(hollo());
        translate_and_cache(&tr, &cache, "Hollo got 5 gold")
            .await
            .unwrap();
        assert_eq!(
            lookup(&tr, &cache, "Hollo got 7 gold").as_deref(),
            Some("哈樓獲得了 7 金幣")
        );
    }

    #[tokio::test]
    async fn system_translation_is_cached_under_the_key_it_was_made_with() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache(&dir);
        let fake = Fake::texts(&["哈樓獲得了金幣"]);
        let fake2 = Arc::clone(&fake);
        let tr = Translator::with_client(move |_| fake2.clone(), spec("m"), "繁體中文".into());
        tr.set_glossary(hollo());
        let tr2 = Arc::clone(&tr);
        fake.on_call(move |_| tr2.set_glossary(None));
        let text = "Hollo got gold";
        translate_and_cache(&tr, &cache, text).await.unwrap();
        assert!(cache.get(text, KEY).is_some());
        assert!(cache.get(text, "").is_none());
    }
}
