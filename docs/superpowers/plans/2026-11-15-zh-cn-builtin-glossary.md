# 簡體中文內建術語表 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 從遊戲英文原文與 Subata 簡中漢化包產生 `src/glossary/builtin/zh-CN.json`，並更新假設「沒有正式內建表」的測試。

**Architecture:** 暫存區的一次性 Python 腳本讀兩個 KIWAD，依「檔名＋鍵」對照 `.lang`，照設計文件的規則挑出名稱、輸出 JSON。程式碼不動：`build.rs` 會自動把 `builtin/` 裡的新檔編進 exe。

**Tech Stack:** Python 3（只用標準函式庫，`python -I` 執行）、Rust／cargo 測試。

**Spec:** `docs/superpowers/specs/2026-11-15-zh-cn-builtin-glossary-design.md`

## Global Constraints

- 腳本放 scratchpad（`C:\Users\p2902\AppData\Local\Temp\claude\664c7238a8b7db3a\1d2554e7-ade9-4b7e-ba74-1d0833201a6f\scratchpad\gl\`），不進 repo；已有 `wad.py`（`entries(path)`、`read(entry)`）可直接用。
- 英文：`D:\Program Files (x86)\Steam\steamapps\common\Wizard101\Data\GameData\Root.wad`；簡中：同目錄 `Locale_en-US-root.wad`（漢化包 6.1.2）。只讀，不可寫入遊戲目錄。
- `.lang`：UTF-16、`\r\n` 分行，第 1 行標頭，之後每三行「鍵、註解、文字」。
- 輸出：UTF-8、兩格縮排、依英文原文字母序；第一個鍵 `"glossary.creators": "Subata"`。
- cargo 一律 `-j 4`，測試加 `-- --test-threads=4`。
- Commit message 英文、conventional commits；不 push。

## Review Focus

- 英文原文只差大小寫或頭尾空白（`Fire Elf`／`Fire elf `）：比對器會兩條都命中、都送進提示詞 → 依 `fold`（strip＋casefold）去重，只留一條。（Task 1 自檢）
- 譯名頭尾空白、全形空白或結尾括號原文：要清乾淨，不能讓 `暗夜沼泽城堡(Castle Darkmoor)` 進表。（Task 1 自檢）
- 中文譯名本身很常見（例如兩字的 `火焰`、`风暴`），中文聊天也會命中：被擋清單外另列「最短的 50 條譯名」給維護者看。（Task 1 報告）
- 約 2～3 萬條真實內建表的效能：設定頁的 `review`／`overridden` 與翻譯端 `Glossary::new`＋`hits` 不可明顯變慢。（Task 2 測試）
- 同一個英文跨類別有不同譯名（`Malistaire`：NPC `思泰尔`、法術 `玛丽思泰尔`）：取類別優先序最前的（地名 → 怪物 → 法術 → NPC → 寵物 → 寵物天賦 → 坐騎 → 徽章；怪物檔有它，所以是 `马利斯泰尔`），不是任意一個。（Task 1 自檢）

---

### Task 1: 產生 zh-CN.json 與審查報告

**Files:**
- Create: `scratchpad\gl\build_glossary.py`
- Create: `src/glossary/builtin/zh-CN.json`
- Create: `scratchpad\gl\report.md`（不進 repo）

**Interfaces:**
- Consumes: `wad.py` 的 `entries(path) -> dict[name, (is_zip, raw)]`、`read(entry) -> bytes`。
- Produces: `zh-CN.json`（Task 2 的測試讀它的條數）。

- [ ] **Step 1: 寫 `build_glossary.py`**

類別與主檔順序（類別優先序由上到下，跨類別同一英文取最前面類別的譯名；同類別內取出現最多的譯名，同票依檔案順序）：

```python
CATEGORIES = [
    ("place",  ["WizardZone", "ZoneLocName", "Zone", "WorldNames", "WizardPOI"]),
    ("mob",    ["WizardMobs"]),
    ("spell",  ["Spells"]),
    ("npc",    ["NPCs", "WC-NPCs", "WizardNPC"]),
    ("pet",    ["WizardPets"]),
    ("talent", ["PetTalents", "PetTalent"]),
    ("mount",  ["WizardMounts"]),
    ("badge",  ["Badges", "WizardBadges", "ClientBadgeNames"]),
]
```

清理與過濾，依設計文件「抽取規則」3、4：
- 英文 strip；含 `< > { } % . ! ?` 或空字串就丟。
- 譯名 strip（含全形空白），去掉結尾 `(...)`／`（...）` 且括號內含拉丁字母的部分；之後仍含 `[A-Za-z]` 或 `fold` 後等於原文就丟。
- 一字英文、而且這個字以全小寫出現在任何 `WizQst*`／`ConsoleWizQst*` 檔的英文文字裡（以 `\b` 單字比對），就丟並記為「常見單字」。
- 譯名只有一個字就丟。
- 依 `fold(英文)` 去重，保留大小寫形式出現最多的那個寫法。

結尾的自檢 assert：每個 value 不含拉丁字母、長度 ≥ 2、不含 `(`／`（`；key 沒有 fold 後重複；`"Tower of the Helephant" → "神象之塔"`、`"Storm Beetle" → "风暴甲虫"`（法術檔）存在；`"Darkmoor"` 的譯名來自地名類別。

- [ ] **Step 2: 執行，確認自檢全過**

Run: `python -I build_glossary.py`（在 `scratchpad\gl\` 內）
Expected: 印出各類別收錄／丟棄數與總數，沒有 AssertionError。

- [ ] **Step 3: 產出 `report.md` 給維護者**

內容：各類別收錄數、「常見單字」被擋清單（全列）、「一字譯名」被擋清單、最短的 50 條收錄譯名、跨類別衝突取捨的前 30 例。

- [ ] **Step 4: 停下來，請維護者看 report.md**

依維護者意見調整規則後回到 Step 2。維護者同意前不進 Task 2。

---

### Task 2: 更新測試並提交

**Files:**
- Modify: `tests/it/glossary.rs:88-93`（`only_the_test_builtin_loads_until_real_ones_ship`）
- Modify: `tests/it/glossary.rs`（新增效能測試）
- Commit: spec、plan、`zh-CN.json`、測試

**Interfaces:**
- Consumes: `builtin::builtin_codes() -> Vec<&'static str>`、`builtin::builtin_entries(&str) -> Option<Vec<Entry>>`、`review`、`overridden`、`Glossary::new(custom, builtin).hits(&str)`。

- [ ] **Step 1: 改寫舊測試**

```rust
#[test]
fn every_builtin_loads_with_entries() {
    let codes = builtin::builtin_codes();
    assert!(codes.contains(&"zh-CN"), "{codes:?}");
    for code in codes {
        assert!(builtin::builtin_entries(code).is_some_and(|e| !e.is_empty()), "{code}");
    }
    assert_eq!(builtin::builtin_creators("zh-CN"), "Subata");
}
```

刪掉原本那條；`builtin_creators_come_from_the_file` 裡 `builtin_creators("zh-TW") == ""` 保留（仍沒有 zh-TW）。

- [ ] **Step 2: 新增真實內建表的效能測試**

```rust
#[test]
fn real_builtin_stays_fast() {
    let builtin = builtin::builtin_entries("zh-CN").unwrap();
    let custom: Vec<Entry> = (0..2_000)
        .map(|i| e(&format!("custom {i}"), &format!("自訂 {i}")))
        .collect();
    let start = std::time::Instant::now();
    review(&custom, &builtin);
    overridden(&custom, &builtin);
    let g = Glossary::new(custom, builtin);
    assert!(!g.hits("anyone going to Tower of the Helephant?").is_empty());
    let elapsed = start.elapsed();
    assert!(elapsed < std::time::Duration::from_secs(2), "{elapsed:?}");
}
```

若超時，先回報量測數字再決定，不要直接放寬門檻。

- [ ] **Step 3: 跑檢查**

Run: `cargo fmt --check`、`cargo clippy -j 4 --all-targets -- -D warnings`、`cargo test -j 4 -- --test-threads=4`
Expected: 全部零錯誤、零警告、全綠（設定頁測試若因多了 zh-CN 而失敗，修測試的前提，不改產品程式碼）。

- [ ] **Step 4: Commit**

```bash
git add docs/superpowers/specs/2026-11-15-zh-cn-builtin-glossary-design.md docs/superpowers/plans/2026-11-15-zh-cn-builtin-glossary.md src/glossary/builtin/zh-CN.json tests/it/glossary.rs
git commit -m "feat(glossary): add the zh-CN built-in glossary"
```
