# 多張顯示卡的 VRAM 估算 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 引擎會用到幾張獨顯，VRAM 總量、可用量與 `--fit` 預留就按那幾張加總，設定頁列出這些卡。

**Architecture:** `src/local/hardware.rs` 改成偵測所有獨顯、逐張讀 DXGI 計數器，`vram()` 依引擎 `--list-devices` 配對要算的卡並加總，結果連同卡名寫進 `Hardware.vram_cards`；`fit.rs` 依卡數乘預留量；設定頁把卡名合併成一行。`Hardware.gpu` 保留為第一張，`auto_variant` 與內顯判斷不動。

**Tech Stack:** Rust、`windows` crate（DXGI）、wgpu adapter 列舉、egui。

**Spec:** `docs/superpowers/specs/2026-11-14-multi-gpu-vram-estimate-design.md`

## Global Constraints

- 單卡時所有數字與行為不變；既有測試不改期望值。
- log 用 `log(...)`、英文、前綴 `[local]`；註解／docstring 用繁中全形、預設一行。
- 不新增 i18n 字串：卡名合併後塞進既有 `local.hw.gpu` 的 `{name}`。
- 同型號寫成 `名稱 ×2`，不同型號用 ` + ` 連接。
- cargo 一律限並行：`cargo build -j 4`、`cargo test -j 4 -- --test-threads=4`、`cargo clippy -j 4 --all-targets -- -D warnings`。
- 每次 commit 前跑 `cargo fmt --check`、clippy、`cargo test`（全套測試整個分支只在最後跑一次；task 內只跑相關測試）。commit 訊息英文 conventional commits。
- 在原目錄開分支 `feat/multi-gpu-vram`，不建 worktree；spec 與本計畫在第一個 commit 一起提交。

## Review Focus

- 兩張同型號獨顯：DXGI 配對必須對到兩個不同的 adapter（LUID），不能讀同一張兩次 —— Task 1 `pair_adapters` 測試。
- 一張卡的計數器讀不到：整組改用引擎數字，不混用 —— Task 1 測試。
- CUDA 版只列 NVIDIA、機器上還有 AMD 獨顯：只算 NVIDIA —— Task 1 測試。
- `Hardware` 沒有 `vram_cards`（內顯、還沒量、舊測試的 literal）：預留量仍是一份 `FIT_TARGET` —— Task 2 測試。
- 引擎是 CPU 版（列不出裝置）：VRAM 為 0、`vram_cards` 為空，設定頁照舊顯示偵測到的卡名 —— Task 1、Task 3 測試。

---

### Task 1: 偵測所有獨顯並加總 VRAM

**Files:**
- Modify: `src/local/hardware.rs`（`detect_gpu`、`adapter_memory`、`vram`、`Hardware`、tests）
- Modify: `src/local/server.rs:120-129`（`Launcher` trait）
- Modify: `src/local/hub.rs:205`、`:258`、`:425-490`、`:538`
- Modify: `tests/common/local.rs:118`、`tests/it/local_runtime.rs:149-162`（假 launcher）
- Modify: 所有列出全部欄位的 `Hardware { … }` literal（`src/local/fit.rs:381-400` 等，`cargo build --all-targets` 會列出），補 `vram_cards: Vec::new()`

**Interfaces:**
- Produces:
  - `pub fn detect_gpus() -> Vec<Gpu>`：DX12 列舉順序的所有獨顯；沒有獨顯時只回第一張內顯；都沒有回空。取代 `detect_gpu`。
  - `pub fn adapter_memory(gpus: &[Gpu]) -> Vec<Option<(u64, u64)>>`：與 `gpus` 等長、逐張 `(總量, 可用量)`。
  - `fn pair_adapters(gpus: &[Gpu], adapters: &[(u32, u32)]) -> Vec<Option<usize>>`：`adapters` 是 DXGI 列舉順序的 `(VendorId, DeviceId)`；每張 gpu 取第一個還沒被用過、vendor／device 相同的 adapter 索引。
  - `pub struct Vram { pub total: u64, pub free: u64, pub cards: Vec<String> }`
  - `pub fn vram(gpus: &[Gpu], counters: &[Option<(u64, u64)>], devices: Option<Vec<(String, u64, u64)>>) -> Option<Vram>`
  - `Hardware.vram_cards: Vec<String>`：算進 VRAM 的卡名（長度即卡數）；沒量到或 VRAM 為 0 時為空。
  - `Launcher::detect_gpus(&self) -> Vec<Gpu>`、`Launcher::adapter_memory(&self, gpus: &[Gpu]) -> Vec<Option<(u64, u64)>>`（預設實作呼叫上面兩個函式）。

- [ ] **Step 1: 開分支並提交 spec 與計畫**

```bash
git switch -c feat/multi-gpu-vram
git add docs/superpowers/specs/2026-11-14-multi-gpu-vram-estimate-design.md docs/superpowers/plans/2026-11-14-multi-gpu-vram-estimate.md
git commit -m "docs(spec): plan multi-GPU VRAM estimate"
```

- [ ] **Step 2: 寫失敗的測試（`src/local/hardware.rs` tests）**

以 `G = 1 << 30`、`card(name, kind)` 建 `Gpu`（vendor `NVIDIA`、device 依名稱區分）：

```rust
/// 同型號兩張各自對到不同的 adapter。
#[test]
fn identical_cards_pair_with_distinct_adapters() {
    let a = card("RTX 4060", GpuKind::Discrete); // device 1
    assert_eq!(
        pair_adapters(&[a.clone(), a], &[(NVIDIA, 1), (0x8086, 9), (NVIDIA, 1)]),
        vec![Some(0), Some(2)]
    );
    assert_eq!(pair_adapters(&[card("X", GpuKind::Discrete)], &[]), vec![None]);
}

/// 兩張同型號：引擎列出兩張、計數器都讀得到，加總計數器。
#[test]
fn two_listed_cards_sum_their_counters() {
    let a = card("RTX 4060", GpuKind::Discrete);
    let devices = Some(vec![("RTX 4060".into(), 8 * G, 7 * G), ("RTX 4060".into(), 8 * G, 7 * G)]);
    let v = vram(&[a.clone(), a], &[Some((8 * G, 5 * G)), Some((8 * G, 6 * G))], devices).unwrap();
    assert_eq!((v.total, v.free, v.cards.len()), (16 * G, 11 * G, 2));
}

/// 一張計數器讀不到：整組用引擎數字。
#[test]
fn one_missing_counter_uses_engine_numbers_for_all() {
    // 同上兩張，counters = [Some((8G,5G)), None] → (16G, 14G, 2 張)
}

/// Vulkan 列出內顯、CUDA 只列 NVIDIA：只算對得上的獨顯。
#[test]
fn only_listed_discrete_cards_count() {
    // gpus = [NVIDIA "RTX 4060", AMD "RX 7600"]；devices 只有 "RTX 4060" 與 "Intel(R) Graphics"
    // → total/free 只有 RTX 4060 的計數器，cards == ["RTX 4060"]
}

/// 沒有任何名稱對得上：退回第一張的計數器，再不行取最大的裝置。
#[test]
fn unmatched_names_fall_back_to_a_single_device() {
    // devices = [("Other", 4G, 3G), ("Bigger", 6G, 5G)]
    // counters[0] = Some((8G, 2G)) → (8G, 2G, [gpus[0].name])
    // counters[0] = None → (6G, 5G, ["Bigger"])
}

/// 沒裝引擎：所有獨顯的計數器加總；有一張讀不到就還量不到。
#[test]
fn without_engine_all_discrete_cards_sum() {
    // 兩張、都 Some → 加總、2 張；其中一張 None → vram(...) == None
}
```

既有的 `vram_prefers_the_counters_and_needs_no_engine` 改成新簽名（`&[gpu]`、`&[counters]`），期望的 `(total, free)` 不變；VRAM 為 0 的情況 `cards` 為空。`the_detected_gpu_wins_over_a_larger_integrated_one` 保留不動。

- [ ] **Step 3: 確認測試失敗**

Run: `cargo test -j 4 --lib local::hardware -- --test-threads=4`
Expected: 編譯失敗（`pair_adapters`、`Vram` 未定義、`vram` 簽名不符）。

- [ ] **Step 4: 實作 `hardware.rs`**

- `detect_gpus`：同現在的 wgpu 列舉，收集所有 `DiscreteGpu`；空的話取第一個 `IntegratedGpu`。沒有可用 adapter 時沿用現有 log。
- `adapter_memory`：建一次 DXGI factory、列舉所有 adapter 的 `GetDesc1`，用 `pair_adapters` 配對後逐張讀 `usage::adapter_dedicated_usage`；讀失敗的那張回 `None`，沿用現有的 log 訊息。
- `vram` 的規則：
  1. `devices == Some(空)` → `Some(Vram { 0, 0, [] })`。
  2. `devices == Some(d)`：依 `d` 的順序，每個裝置找第一張還沒配對、名稱相同的**獨顯**。有配到時：配到的那幾張計數器都 `Some` 就加總計數器，否則加總這些裝置在 `d` 裡的數字；`cards` 為配到的卡名。一張都沒配到：`counters.first()` 是 `Some` 就用它（卡名 `gpus[0].name`），否則 `pick_device(d, gpus.first())`（卡名為該裝置名稱）。
  3. `devices == None`：`gpus` 為空或第一張是內顯 → `Some(Vram { 0, 0, [] })`；否則全部計數器都 `Some` 就加總（`cards` 為所有卡名），否則 `None`。
- `Hardware` 加 `pub vram_cards: Vec<String>`，docstring 一行。

- [ ] **Step 5: 接上 trait、hub 與假 launcher**

- `Launcher`：`detect_gpu` → `detect_gpus`，`adapter_memory` 改成新簽名。
- `hub.rs`：快取改成 `Mutex<Option<Vec<Gpu>>>`，`fn gpu(&self, redetect)` 改為 `fn gpus(&self, redetect: bool) -> Vec<Gpu>`；`integrated_gpu`、`auto_variant` 呼叫處取 `.first()`。`refresh_hardware` 中 `hw.gpu = gpus.first().cloned()`，計數器用 `self.launcher.adapter_memory(&gpus)`，把 `Vram` 寫進 `vram_total`／`vram_free`／`vram_cards`。
- log：`VRAM sources:` 改記逐張的 counters 與 devices；`hardware:` 那行改記 `gpus=<所有偵測到的卡名>` 與 `vram_cards=<算數的卡名>`。
- 假 launcher：`detect_gpus` 回 `vec![原本那張]`（`tests/common/local.rs` 回 `Vec::new()`）；`tests/it/local_runtime.rs` 的 `adapter_memory` 對每張卡回傳 `*self.shared.counters`。
- 依 `cargo build -j 4 --all-targets` 的錯誤，替每個列出全部欄位的 `Hardware { … }` 補 `vram_cards: Vec::new()`。

- [ ] **Step 6: 確認測試通過**

Run: `cargo test -j 4 --lib local:: -- --test-threads=4` 與 `cargo test -j 4 --test it local_runtime -- --test-threads=4`
Expected: 全部 PASS。

- [ ] **Step 7: Commit**

```bash
git add -A src tests
git commit -m "feat(local): sum VRAM across every GPU the engine uses"
```

### Task 2: `--fit` 預留量依卡數計算

**Files:**
- Modify: `src/local/fit.rs:104-150`（`usable`、`need` 的 `room`）

**Interfaces:**
- Consumes: `Hardware.vram_cards`（Task 1）
- Produces: `fn fit_margin(hw: &Hardware) -> u64`（`FIT_TARGET` × `max(1, vram_cards.len())`），`usable` 與 `need` 都改用它。

- [ ] **Step 1: 寫失敗的測試（`src/local/fit.rs` tests）**

```rust
/// 兩張卡各留一份 `FIT_TARGET`；沒記卡名時仍留一份。
#[test]
fn fit_margin_is_per_card() {
    let mut h = hw(16 * G, 32 * G);
    assert_eq!(fit_margin(&h), FIT_TARGET);
    h.vram_cards = vec!["A".into(), "A".into()];
    assert_eq!(fit_margin(&h), 2 * FIT_TARGET);
    assert_eq!(usable(&h).0, 16 * G - 2 * FIT_TARGET);
}
```

- [ ] **Step 2: 確認測試失敗**

Run: `cargo test -j 4 --lib local::fit -- --test-threads=4`
Expected: 編譯失敗（`fit_margin` 未定義）。

- [ ] **Step 3: 實作 `fit_margin` 並讓 `usable`、`need` 使用；`usable` 與 `need` 的 docstring 改成「每張卡各扣一份 `FIT_TARGET`」**

- [ ] **Step 4: 確認測試通過**

Run: `cargo test -j 4 --lib local::fit -- --test-threads=4`
Expected: 全部 PASS，既有測試期望值不變。

- [ ] **Step 5: Commit**

```bash
git add src/local/fit.rs
git commit -m "feat(local): reserve the fit margin on each GPU"
```

### Task 3: 設定頁列出所有算數的卡

**Files:**
- Modify: `src/ui/settings/local_engine.rs:372-377`
- Test: 同檔 `#[cfg(test)]`（沒有 tests 模組就新增）

**Interfaces:**
- Consumes: `Hardware.vram_cards`（Task 1）
- Produces: `fn card_names(names: &[String]) -> String`

- [ ] **Step 1: 寫失敗的測試**

```rust
/// 同型號合併成 ×N，不同型號用 + 連接，保留第一次出現的順序。
#[test]
fn card_names_group_identical_cards() {
    let n = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(card_names(&n(&["RTX 4060"])), "RTX 4060");
    assert_eq!(card_names(&n(&["RTX 4060", "RTX 4060"])), "RTX 4060 ×2");
    assert_eq!(card_names(&n(&["A", "B", "A"])), "A ×2 + B");
}
```

- [ ] **Step 2: 確認測試失敗**

Run: `cargo test -j 4 --lib ui::settings::local_engine -- --test-threads=4`
Expected: 編譯失敗（`card_names` 未定義）。

- [ ] **Step 3: 實作 `card_names`；顯示卡那行在 `hw.vram_cards` 非空時用 `card_names(&hw.vram_cards)`，否則照舊用 `hw.gpu` 的名稱或 `local.hw.none`**

- [ ] **Step 4: 確認測試通過**

Run: `cargo test -j 4 --lib ui::settings::local_engine -- --test-threads=4`
Expected: PASS。

- [ ] **Step 5: 全套檢查**

Run: `cargo fmt --check`、`cargo clippy -j 4 --all-targets -- -D warnings`、`cargo test -j 4 -- --test-threads=4`
Expected: 零警告、全部通過。

- [ ] **Step 6: Commit**

```bash
git add src/ui/settings/local_engine.rs
git commit -m "feat(settings): list every GPU counted for VRAM"
```
