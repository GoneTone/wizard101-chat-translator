//! 術語表檔案解析（含建立者）與內建術語表註冊。

use wizard101_chat_translator::glossary::{
    CREATORS_KEY, Conflict, Entry, Fold, Glossary, GlossaryFile, ParseError, RowIssue, apply_merge,
    builtin, file_to_json, hits_key, overridden, parse_file, plan_merge, review,
    without_builtin_duplicates,
};

fn e(source: &str, target: &str) -> Entry {
    Entry {
        source: source.into(),
        target: target.into(),
    }
}

#[test]
fn parse_keeps_file_order_and_trims() {
    let got = parse_file(r#"{" Hollo ": "哈樓", "Fire Elf": "火精靈"}"#).unwrap();
    assert_eq!(
        got.entries,
        vec![e("Hollo", "哈樓"), e("Fire Elf", "火精靈")]
    );
    assert_eq!(got.creators, "");
}

#[test]
fn parse_skips_blank_sides() {
    assert_eq!(
        parse_file(r#"{"": "x", "a": "  "}"#),
        Ok(GlossaryFile::default())
    );
}

#[test]
fn parse_reads_creators_apart_from_entries() {
    let got =
        parse_file(r#"{"glossary.creators": " [A](https://a.example), B ", "Hollo": "哈樓"}"#)
            .unwrap();
    assert_eq!(got.creators, "[A](https://a.example), B");
    assert_eq!(got.entries, vec![e("Hollo", "哈樓")]);
}

#[test]
fn parse_names_what_is_wrong() {
    let not_text = |key: &str| Err(ParseError::NotText { key: key.into() });
    assert_eq!(parse_file(r#"{"a": 1}"#), not_text("a"));
    assert_eq!(
        parse_file(r#"{"glossary.creators": ["A"]}"#),
        not_text(CREATORS_KEY)
    );
    assert_eq!(parse_file("[]"), Err(ParseError::NotObject));
    assert_eq!(parse_file("not json"), Err(ParseError::Json { line: 1 }));
    assert_eq!(
        parse_file("{\n\"a\": \"b\",\n}"),
        Err(ParseError::Json { line: 3 })
    );
}

#[test]
fn export_round_trips_with_creators_first() {
    let file = GlossaryFile {
        creators: "[A](https://a.example), B".into(),
        entries: vec![e("Hollo", "哈樓"), e("Fire Elf", "火精靈")],
    };
    let json = file_to_json(&file);
    assert!(
        json.trim_start_matches(['{', '\n', ' '])
            .starts_with(&format!("\"{CREATORS_KEY}\""))
    );
    assert_eq!(parse_file(&json), Ok(file));
}

#[test]
fn export_without_creators_has_no_creators_key() {
    let file = GlossaryFile {
        creators: String::new(),
        entries: vec![e("Hollo", "哈樓")],
    };
    assert!(!file_to_json(&file).contains(CREATORS_KEY));
}

#[test]
fn builtin_load_drops_untranslated_pairs() {
    let got = builtin::load(r#"{"Fire Elf": "fire elf", "Hollo": "哈樓"}"#);
    assert_eq!(got, vec![e("Hollo", "哈樓")]);
}

#[test]
fn every_builtin_loads_with_entries() {
    let codes = builtin::builtin_codes();
    assert!(codes.contains(&"zh-CN"), "{codes:?}");
    for code in codes {
        assert!(
            builtin::builtin_entries(code).is_some_and(|e| !e.is_empty()),
            "{code}"
        );
    }
}

/// 真實大小的內建表下，設定頁檢查與翻譯端比對都要夠快。
#[test]
fn real_builtin_stays_fast() {
    let builtin = builtin::builtin_entries("zh-CN").unwrap();
    let custom: Vec<Entry> = (0..2_000)
        .map(|i| e(&format!("custom {i}"), &format!("自訂 {i}")))
        .collect();
    let start = std::time::Instant::now();
    review(&custom, &builtin, Fold::default());
    overridden(&custom, &builtin, Fold::default());
    let g = Glossary::new(custom, builtin, Fold::default());
    assert!(!g.hits("anyone going to Tower of the Helephant?").is_empty());
    let elapsed = start.elapsed();
    assert!(elapsed < std::time::Duration::from_secs(2), "{elapsed:?}");
}

#[test]
fn builtin_creators_come_from_the_file() {
    assert_eq!(
        builtin::builtin_creators(builtin::TEST_BUILTIN),
        "[Tester](https://example.com)"
    );
    assert_eq!(builtin::builtin_creators("xx-XX"), "");
}

fn hits(entries: Vec<Entry>, text: &str) -> Vec<Entry> {
    Glossary::new(entries, vec![], Fold::default()).hits(text)
}

fn layered(custom: Vec<Entry>, builtin: Vec<Entry>, text: &str) -> Vec<Entry> {
    Glossary::new(custom, builtin, Fold::default()).hits(text)
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
        review(&custom, &builtin, Fold::default()),
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
        review(
            &[e("哈樓", "Hollo")],
            &[e("Hollo", "哈羅")],
            Fold::default()
        ),
        vec![RowIssue::BuiltinDiffers {
            other: "哈羅".into()
        }]
    );
}

#[test]
fn overridden_marks_only_shadowed_builtins() {
    let (custom, builtin) = review_fixture();
    assert_eq!(
        overridden(&custom, &builtin, Fold::default()),
        vec![true, true, false]
    );
}

#[test]
fn without_builtin_duplicates_drops_same_rows() {
    let (custom, builtin) = review_fixture();
    assert_eq!(
        without_builtin_duplicates(&custom, &builtin, Fold::default()),
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
        review(
            &[e("Hollo", "哈樓"), e("Hollo ", "哈囉")],
            &[],
            Fold::default()
        ),
        vec![RowIssue::Ok, RowIssue::DuplicateSource]
    );
    assert_eq!(
        review(&[e(" ", "哈樓")], &[], Fold::default()),
        vec![RowIssue::Incomplete]
    );
}

#[test]
fn reversed_identical_entry_is_same_as_builtin() {
    let (c, b) = (vec![e("哈樓", "Hollo")], vec![e("Hollo", "哈樓")]);
    assert_eq!(
        review(&c, &b, Fold::default()),
        vec![RowIssue::SameAsBuiltin]
    );
    assert!(without_builtin_duplicates(&c, &b, Fold::default()).is_empty());
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
    assert!(
        review(&custom, &builtin, Fold::default())
            .iter()
            .all(|i| *i == RowIssue::Ok)
    );
    assert!(!overridden(&custom, &builtin, Fold::default()).contains(&true));
    let elapsed = start.elapsed();
    assert!(elapsed < std::time::Duration::from_secs(1), "{elapsed:?}");
}

#[test]
fn merge_plan_counts_added_identical_and_conflicts() {
    let plan = plan_merge(
        &[e("A", "甲"), e("B", "乙")],
        &[e("a", "甲"), e("B", "丙"), e("C", "丁")],
        Fold::default(),
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
    let plan = plan_merge(
        &current,
        &[e("a", "甲"), e("B", "丙"), e("C", "丁")],
        Fold::default(),
    );
    let mut kept = current.clone();
    apply_merge(&mut kept, &plan, &[false]);
    assert_eq!(kept, vec![e("A", "甲"), e("B", "乙"), e("C", "丁")]);
    let mut taken = current;
    apply_merge(&mut taken, &plan, &[true]);
    assert_eq!(taken, vec![e("A", "甲"), e("B", "丙"), e("C", "丁")]);
}

#[test]
fn merge_plan_keeps_the_first_of_case_variant_duplicates() {
    let plan = plan_merge(&[], &[e("Hollo", "甲"), e("hollo", "乙")], Fold::default());
    assert_eq!(plan.added, vec![e("Hollo", "甲")]);
    assert_eq!(plan.identical, 0);
    assert!(plan.conflicts.is_empty());
}

const TW: Fold = Fold { traditional: true };

#[test]
fn traditional_fold_matches_simplified_entries() {
    let g = Glossary::new(vec![], vec![e("Krokotopia", "克洛克托比亚")], TW);
    assert_eq!(g.hits("要去克洛克托比亞嗎").len(), 1);
    let plain = Glossary::new(
        vec![],
        vec![e("Krokotopia", "克洛克托比亚")],
        Fold::default(),
    );
    assert!(plain.hits("要去克洛克托比亞嗎").is_empty());
}

#[test]
fn traditional_fold_keeps_other_scripts_matching() {
    let g = Glossary::new(
        vec![e("Tokyo Tower", "東京タワー")],
        vec![e("Krokotopia", "克洛克托比亚")],
        TW,
    );
    assert_eq!(g.hits("要去克洛克托比亚吗").len(), 1);
    assert_eq!(g.hits("going to krokotopia").len(), 1);
    assert_eq!(g.hits("東京タワーに行こう").len(), 1);
}

/// 依現有規則，內建的譯名側只會被自訂的「來源側」蓋掉，所以自訂把繁體寫在來源。
#[test]
fn traditional_custom_overrides_simplified_builtin() {
    let custom = vec![e("龍之谷", "Dspyre")];
    let builtin = vec![e("Dragonspyre", "龙之谷")];
    assert_eq!(overridden(&custom, &builtin, TW), vec![true]);
    assert_eq!(overridden(&custom, &builtin, Fold::default()), vec![false]);
    let g = Glossary::new(custom.clone(), builtin, TW);
    assert_eq!(g.hits("去龍之谷"), custom);
}

#[test]
fn traditional_fold_applies_to_review_and_merge() {
    let custom = vec![e("學院", "School"), e("学院", "Academy")];
    assert_eq!(review(&custom, &[], TW)[1], RowIssue::DuplicateSource);
    assert_eq!(review(&custom, &[], Fold::default())[1], RowIssue::Ok);
    let plan = plan_merge(&[e("学院", "School")], &[e("學院", "School")], TW);
    assert_eq!(plan.identical, 1);
    let plan = plan_merge(
        &[e("学院", "School")],
        &[e("學院", "School")],
        Fold::default(),
    );
    assert_eq!(plan.added.len(), 1);
}

/// 只差在繁簡的自訂條目不算「與內建相同」，存檔時不可被刪。
#[test]
fn traditional_custom_is_not_same_as_simplified_builtin() {
    let custom = vec![e("Dragonspyre", "龍之谷")];
    let builtin = vec![e("Dragonspyre", "龙之谷")];
    assert_eq!(
        review(&custom, &builtin, TW)[0],
        RowIssue::BuiltinDiffers {
            other: "龙之谷".into()
        }
    );
    assert_eq!(without_builtin_duplicates(&custom, &builtin, TW), custom);
    assert_eq!(overridden(&custom, &builtin, TW), vec![true]);
}

/// 用真實內建表驗證：繁體寫法單獨出現與夾在句中，命中結果都要跟簡體寫法一樣（防詞組轉換前後不一致）。
#[test]
fn real_zh_cn_traditional_matches_in_context() {
    let builtin = builtin::builtin_entries("zh-CN").unwrap();
    let g = Glossary::new(vec![], builtin.clone(), TW);
    let mut misses = Vec::new();
    for entry in &builtin {
        let tw = hanconv::s2tw(&entry.target);
        for (simplified, traditional) in [
            (entry.target.clone(), tw.clone()),
            (format!("我们去{}吧", entry.target), format!("我們去{tw}吧")),
        ] {
            if g.hits(&traditional) != g.hits(&simplified) {
                misses.push(traditional);
            }
        }
    }
    assert!(
        misses.is_empty(),
        "{} misses, e.g. {:?}",
        misses.len(),
        &misses[..misses.len().min(20)]
    );
}

mod config_glossaries {
    use serde_json::{Value, json};
    use wizard101_chat_translator::config::glossaries::StoredGlossary;
    use wizard101_chat_translator::config::{Config, default_config, glossaries};
    use wizard101_chat_translator::glossary::builtin;

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
                {"id": "a", "name": "X", "creators": 5, "builtin": 5, "match_traditional": "no",
                 "entries": {"k": "v", "n": 1}},
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
        assert!(list[0].get("creators").is_none());
        assert!(list[0].get("match_traditional").is_none());
        assert_eq!(list[0]["entries"], json!({"k": "v"}));
        assert_eq!(list[1]["entries"], json!({}));
        assert!(cfg["active_glossary"].is_null());
    }

    #[test]
    fn incomplete_entries_are_dropped_at_load() {
        let mut cfg = cfg_with(
            json!([{"id": "a", "name": "X", "builtin": null,
                    "entries": {"k": "v", "half": "  ", "": "x"}}]),
            json!("a"),
        );
        assert!(glossaries::normalize(&mut cfg));
        assert_eq!(cfg["glossaries"][0]["entries"], json!({"k": "v"}));
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
    fn active_glossary_matches_traditional_input() {
        let glossary = json!({"id": "a", "name": "X", "builtin": "zh-CN", "entries": {}});
        let cfg = cfg_with(json!([glossary.clone()]), json!("a"));
        assert!(
            !glossaries::active(&cfg)
                .unwrap()
                .hits("去龍之谷")
                .is_empty()
        );
        let mut off = glossary;
        off["match_traditional"] = json!(false);
        let cfg = cfg_with(json!([off]), json!("a"));
        assert!(
            glossaries::active(&cfg)
                .unwrap()
                .hits("去龍之谷")
                .is_empty()
        );
    }

    #[test]
    fn match_traditional_round_trips() {
        let raw = json!({"id": "a", "name": "X", "builtin": "zh-CN", "entries": {}});
        let g = StoredGlossary::from_value(&raw).unwrap();
        assert!(g.match_traditional);
        assert!(g.to_value().get("match_traditional").is_none());
        let off = StoredGlossary {
            match_traditional: false,
            ..g
        };
        let back = StoredGlossary::from_value(&off.to_value()).unwrap();
        assert!(!back.match_traditional);
    }

    #[test]
    fn fold_only_applies_to_zh_cn() {
        let mut g = StoredGlossary {
            builtin: Some("zh-CN".into()),
            ..Default::default()
        };
        assert!(g.match_traditional);
        assert!(g.fold().traditional);
        g.match_traditional = false;
        assert!(!g.fold().traditional);
        g.match_traditional = true;
        g.builtin = Some(builtin::TEST_BUILTIN.into());
        assert!(!g.fold().traditional);
        g.builtin = None;
        assert!(!g.fold().traditional);
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
    use wizard101_chat_translator::glossary::{Entry, Fold, Glossary};
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
        Some(Arc::new(Glossary::new(
            vec![entry],
            Vec::new(),
            Fold::default(),
        )))
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
