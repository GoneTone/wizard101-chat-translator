# 簡體中文術語表的繁體配對 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 引入 `zh-CN` 內建表的術語表多一個預設打開的開關，打開時整份術語表的比對先把繁體字轉成簡體字。

**Architecture:** `glossary` 模組新增 `Fold`（正規化方式），所有「整份術語表」層級的比對函式都改收 `Fold`；`StoredGlossary` 多存一個開關並算出自己的 `Fold`；設定畫面在引入 `zh-CN` 時多一列開關。

**Tech Stack:** Rust、`hanconv`（`tw2s`）、egui／kittest。

**Spec:** `docs/superpowers/specs/2026-11-16-glossary-traditional-matching-design.md`

## Global Constraints

- 只有引入的內建表代碼是 `zh-CN` 時才有開關與繁簡轉換；代碼寫成常數，旁邊一行註解說明這是例外。
- 開關預設打開；config 裡沒有這個欄位就是打開。
- 轉換只用於比對，不改送給 AI 的原文或條目。
- 編輯畫面的搜尋框、內建檔載入時的「原文與譯名相同」過濾維持用 `fold()`，不受開關影響。
- 匯出檔案不帶開關。
- log 訊息用英文；註解、UI 文字用中文全形標點。
- 每次 commit 前：`cargo fmt --check`、`cargo clippy -j 4 --all-targets -- -D warnings`、`cargo test -j 4 -- --test-threads=4`。

## Review Focus

- `tw2s` 的詞組轉換：術語單獨轉與夾在句中轉結果不同 → 要配得上（Task 1 的 `real_zh_cn_traditional_matches_in_context`）。
- 開關打開後簡體、英文、日文輸入照常配對（Task 1 的 `traditional_fold_keeps_other_scripts_matching`）。
- 編輯畫面快取：只切開關、條目不變時，「被自訂覆蓋」標記也要更新（Task 3 的 `toggle_refreshes_overridden_marks`）。
- 舊設定沒有欄位 → 打開；關掉後存檔再讀回 → 還是關（Task 2 的 `match_traditional_round_trips`）。
- 引入的內建表從 `zh-CN` 改成別的或不引入 → 不轉換、開關不顯示（Task 2 的 `fold_only_applies_to_zh_cn`、Task 3 的 `toggle_only_shows_for_zh_cn`）。

---

### Task 1: `Fold` 與帶開關的比對函式

**Files:**
- Modify: `Cargo.toml`（加 `hanconv = "0.6"`）
- Modify: `src/glossary/mod.rs`（新增 `Fold`，re-export）
- Modify: `src/glossary/matcher.rs`、`src/glossary/review.rs`
- Modify（呼叫端暫時帶 `Fold::default()`）：`src/config/glossaries.rs`、`src/ui/settings/glossary_draft.rs`、`src/ui/settings/glossary_editor.rs`、`src/ui/settings/glossary_pane.rs`
- Test: `tests/it/glossary.rs`

**Interfaces:**
- Produces:
  - `pub struct Fold { pub traditional: bool }`（`Clone, Copy, Debug, Default, PartialEq, Eq`），`impl Fold { pub fn apply(self, s: &str) -> String }`：`traditional` 時先 `hanconv::tw2s`，再做與 `fold()` 相同的去空白＋casefold。`fold(s)` 保留，等於 `Fold::default().apply(s)`。
  - `Glossary::new(custom: Vec<Entry>, builtin: Vec<Entry>, fold: Fold) -> Self`；`hits` 用同一個 `fold` 正規化輸入（存在 struct 裡）。
  - `review(custom, builtin, fold: Fold)`、`overridden(custom, builtin, fold: Fold)`、`without_builtin_duplicates(custom, builtin, fold: Fold)`、`plan_merge(current, incoming, fold: Fold)`；`CustomWords::new(custom, fold)`、`BuiltinIndex::new(builtin, fold)` 內部跟著帶。

- [ ] **Step 1: 寫失敗的測試**（`tests/it/glossary.rs`，`const TW: Fold = Fold { traditional: true };`）

```rust
#[test]
fn traditional_fold_matches_simplified_entries() {
    let g = Glossary::new(vec![], vec![e("Krokotopia", "克洛克托比亚")], TW);
    assert_eq!(g.hits("要去克洛克托比亞嗎").len(), 1);
    let plain = Glossary::new(vec![], vec![e("Krokotopia", "克洛克托比亚")], Fold::default());
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
    let plan = plan_merge(&[e("学院", "School")], &[e("學院", "School")], Fold::default());
    assert_eq!(plan.added.len(), 1);
}

/// 只差在繁簡的自訂條目不算「與內建相同」，存檔時不可被刪。
#[test]
fn traditional_custom_is_not_same_as_simplified_builtin() {
    let custom = vec![e("Dragonspyre", "龍之谷")];
    let builtin = vec![e("Dragonspyre", "龙之谷")];
    assert_eq!(
        review(&custom, &builtin, TW)[0],
        RowIssue::BuiltinDiffers { other: "龙之谷".into() }
    );
    assert_eq!(without_builtin_duplicates(&custom, &builtin, TW), custom);
    assert_eq!(overridden(&custom, &builtin, TW), vec![true]);
}

/// 用真實內建表驗證：繁體寫法單獨出現與夾在句中都配得上（防詞組轉換前後不一致）。
#[test]
fn real_zh_cn_traditional_matches_in_context() {
    let builtin = builtin::builtin_entries("zh-CN").unwrap();
    let g = Glossary::new(vec![], builtin.clone(), TW);
    let mut misses = Vec::new();
    for entry in &builtin {
        let tw = hanconv::s2tw(&entry.target);
        for text in [tw.clone(), format!("我們去{tw}吧")] {
            if !g.hits(&text).iter().any(|h| h.target == entry.target) {
                misses.push(text);
            }
        }
    }
    assert!(misses.is_empty(), "{} misses, e.g. {:?}", misses.len(), &misses[..misses.len().min(20)]);
}
```

- [ ] **Step 2: 執行，確認因簽名不符而編譯失敗**

Run: `cargo test -j 4 --test it glossary:: -- --test-threads=4`
Expected: 編譯錯誤（`Fold` 不存在、參數數量不符）

- [ ] **Step 3: 實作**：加 `hanconv` 相依、`Fold`，改上列函式簽名。`review` 裡判斷 `SameAsBuiltin` 的 `same_words` 一律用 `fold()`（字完全相同才算），重複來源、`BuiltinDiffers` 與共用詞索引才用傳入的 `Fold`；既有呼叫端與既有測試一律帶 `Fold::default()`（Task 2 再換成術語表自己的）。`real_zh_cn_traditional_matches_in_context` 若出現 misses，先看是否為多對一合併（例如「后／後」）導致反向 `s2tw` 本身不是使用者會打的字，再決定是放寬測試（排除 `tw2s(s2tw(x)) != tw2s(x)` 的條目）還是換轉換函式，不可直接刪測試。

- [ ] **Step 4: 執行全套測試確認通過**

Run: `cargo test -j 4 -- --test-threads=4`
Expected: 全部通過

- [ ] **Step 5: 跑 fmt／clippy 後 commit**

```bash
git commit -m "feat(glossary): fold Traditional Chinese to Simplified when asked"
```

### Task 2: 每份術語表的開關與啟用流程

**Files:**
- Modify: `src/glossary/builtin.rs`（常數）
- Modify: `src/config/glossaries.rs`（欄位、讀寫、`active`）
- Modify: `src/ui/settings/glossary_draft.rs`、`src/ui/settings/glossary_editor.rs`、`src/ui/settings/glossary_pane.rs`（改用 `glossary.fold()`）
- Test: `tests/it/glossary.rs`（config 讀寫與 `active`）、`tests/it/glossary_settings.rs`（複製）

**Interfaces:**
- Consumes: Task 1 的 `Fold` 與各函式簽名。
- Produces:
  - `pub const TRADITIONAL_MATCH_BUILTIN: &str = "zh-CN";`（`glossary::builtin`，註解：只有簡體中文表需要繁體配對，例外直接判斷語系）
  - `StoredGlossary.match_traditional: bool`；config 鍵 `"match_traditional"`，加入 `KNOWN_KEYS`；讀取時缺欄位或不是布林 → `true`；寫回時只在 `false` 時寫出（舊設定原封不動）。
  - `impl StoredGlossary { pub fn fold(&self) -> Fold }`：`traditional = match_traditional && builtin == Some(TRADITIONAL_MATCH_BUILTIN)`。
  - `active()` 用 `chosen.fold()` 建 `Glossary`，log 增加 `traditional={bool}` 欄位。

- [ ] **Step 1: 寫失敗的測試**

```rust
#[test]
fn match_traditional_round_trips() {
    let g = StoredGlossary::from_value(&json!({"id": "a", "name": "X", "builtin": "zh-CN", "entries": {}})).unwrap();
    assert!(g.match_traditional);
    assert!(g.to_value().get("match_traditional").is_none());
    let off = StoredGlossary { match_traditional: false, ..g };
    let back = StoredGlossary::from_value(&off.to_value()).unwrap();
    assert!(!back.match_traditional);
}

#[test]
fn fold_only_applies_to_zh_cn() {
    let mut g = StoredGlossary { builtin: Some("zh-CN".into()), match_traditional: true, ..Default::default() };
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
fn active_glossary_matches_traditional_input() {
    // 用 tests/it/glossary.rs 既有的 active() 測試寫法建 cfg：builtin "zh-CN"、無 match_traditional 欄位
    // 斷言 active(&cfg).unwrap().hits("去龍之谷") 非空；加上 "match_traditional": false 後為空。
}
```

`glossary_settings.rs` 的 `duplicate_copies_entries_and_builtin` 加一條：原本 `match_traditional: false` 的術語表複製後仍為 `false`。

- [ ] **Step 2: 執行確認失敗**

Run: `cargo test -j 4 --test it glossary -- --test-threads=4`
Expected: 編譯錯誤（欄位與方法不存在）

- [ ] **Step 3: 實作**：`Default` 的 `match_traditional` 也要是 `true`（手寫 `Default` 或建構處補上）；把 Task 1 暫帶的 `Fold::default()` 換成 `glossary.fold()`（`write_into`、`glossary_errors`、編輯畫面 `refresh`、匯入合併）。

- [ ] **Step 4: 執行全套測試確認通過**

Run: `cargo test -j 4 -- --test-threads=4`
Expected: 全部通過

- [ ] **Step 5: 跑 fmt／clippy 後 commit**

```bash
git commit -m "feat(glossary): store per-glossary Traditional matching for zh-CN"
```

### Task 3: 設定畫面的開關

**Files:**
- Modify: `src/ui/settings/glossary_editor.rs`（`builtin_row` 下方加一列；`refresh` 的快取鍵加上 `fold()`）
- Modify: `src/i18n/locales/zh-TW.json`、`zh-CN.json`、`en-US.json`
- Test: `tests/it/glossary_settings.rs`

**Interfaces:**
- Consumes: Task 2 的 `match_traditional`、`fold()`、`TRADITIONAL_MATCH_BUILTIN`。
- Produces: 語系鍵 `glossary.match_traditional`、`glossary.match_traditional_hint`。

- [ ] **Step 1: 寫失敗的測試**（`editing(entries, Some("zh-CN"))` 用真實內建表）

```rust
#[test]
fn toggle_only_shows_for_zh_cn() {
    let h = editing(Vec::new(), Some("zh-CN"));
    assert!(h.query_by_role_and_label(Role::Switch, &tr("glossary.match_traditional")).is_some());
    let h = editing(Vec::new(), Some(TEST_BUILTIN));
    assert!(h.query_by_role_and_label(Role::Switch, &tr("glossary.match_traditional")).is_none());
}

#[test]
fn toggle_turns_matching_off() {
    let mut h = editing(Vec::new(), Some("zh-CN"));
    click(&mut h, Role::Switch, &tr("glossary.match_traditional"));
    assert!(!copy(&h).match_traditional);
}

#[test]
fn toggle_refreshes_overridden_marks() {
    // 自訂 e("龍之谷", "Dspyre")，引入 zh-CN：搜尋 Dragonspyre、展開內建區塊時那列標「被自訂覆蓋」；
    // 點開關關掉後同一列不再標示。沿用 expand_builtin／builtin_rows_stay_one_line_tall_and_mark_overridden_entries 的查法。
}
```

- [ ] **Step 2: 執行確認失敗**

Run: `cargo test -j 4 --test it glossary_settings -- --test-threads=4`
Expected: FAIL（找不到開關）

- [ ] **Step 3: 實作**：`setting_row(ui, &label, Some(&hint), |ui| toggle(...))`，條件 `glossary.builtin.as_deref() == Some(TRADITIONAL_MATCH_BUILTIN)`；切換時記 log `[settings] glossary {id} match_traditional set to {bool}`。`refresh` 的 `checked` 快取要納入 `fold()`，否則只切開關不會重算。語系文字：

| 鍵 | zh-TW | zh-CN | en-US |
|---|---|---|---|
| `glossary.match_traditional` | 繁體也能配對 | 繁体也能匹配 | Match Traditional Chinese |
| `glossary.match_traditional_hint` | 打繁體字也能配對到這份簡體術語表的術語。 | 输入繁体字时也能匹配到这份简体术语表的术语。 | Terms in this Simplified Chinese glossary also match text typed in Traditional Chinese. |

- [ ] **Step 4: 執行全套測試確認通過**

Run: `cargo test -j 4 -- --test-threads=4`
Expected: 全部通過

- [ ] **Step 5: 跑 fmt／clippy 後 commit，再打包量大小**

```bash
git commit -m "feat(settings): add the Traditional matching switch for zh-CN glossaries"
```

`cargo build --release -j 4`，與 `master` 打包的 exe 比較大小，數字寫進 PR 描述；exe 複製到 `_exe_test`。
