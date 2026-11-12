# 遊戲 2026-11-12 改版 hook 更新 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓 autobot cave、root window、AddLine 三個 hook 在 2026-11-12 改版後的 `WizardGraphicalClient.exe` 正確掛上。

**Architecture:** 只改 `src/reader/hook/` 裡的 pattern、stub 與 cave 大小，`SimpleHook`／`AutobotCave` 的機制不動；測試的假遊戲依 pattern 自動產生命中位元組，golden stub 依新 stub 手動更新。

**Tech Stack:** Rust（cargo）；驗證 stub 用 scratchpad 內的 Python（`capstone`、`pefile` 已安裝）。

**Spec:** `docs/superpowers/specs/2026-11-12-game-update-hooks-design.md`

## Global Constraints

- 新版 exe：`D:\Program Files (x86)\Steam\steamapps\common\Wizard101\Bin\WizardGraphicalClient.exe`（只讀，不可修改）。
- 每個新 pattern 在新版 exe 必須**恰好命中一次**。
- `AUTOBOT_SIZE = 3900`（autobot 函式 3975 bytes，下一個函式在 +3984）。
- 保存的 cave 原始位元組數維持「pattern 字串長度」慣例；新 pattern 長度為 **308**。
- 註解規則照 CLAUDE.md：中文全形、預設一行、只寫 WHY；過時的註解（「wizwalker 原文」、「與 Python 版逐一相同」、「5 bytes」、「208」、「4100」）要跟著改或刪。
- 每個 task 提交前跑：`cargo fmt --check`、`cargo clippy --all-targets -j 4 -- -D warnings`、`cargo test -j 4 -- --test-threads=4`（全套每個 task 只跑一次）。commit message 用英文 conventional commits，結尾帶 `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`。不 push。
- 測試名稱沿用專案的 `*_matches_python` 慣例，不改名。

## Review Focus

1. **AddLine stub 搬移的位元組或順序錯誤 → 遊戲當掉**：Task 3 以 capstone 反組譯產生的 stub，確認結尾是 `push rbp; push rsi; push rdi; push r12`，緊接 `jmp` 到 `入口+6`，且 `mov r9,[rsp+0x38]` 前沒有任何會動到 rsp 的原指令。
2. **跳轉蓋到半條指令**：AddLine 的跳轉是 `E9 rel32` 加 1 個 `0x90`，共 6 bytes，剛好蓋住 `40 55 56 57 41 54`；Task 3 的 golden `jump` 要以 `90` 結尾。
3. **解除 hook 時寫回的長度**：寫回 AddLine 的原碼要 6 bytes、cave 要 308 bytes；Task 1／3 把測試裡寫死的 `[..5]`／`[..208]` 改成新長度。
4. **cave 清零越界**：`AUTOBOT_SIZE` 必須是 3900；Task 1 的測試用 `AUTOBOT_SIZE` 常數斷言寫入長度。
5. **root window 換暫存器**：pattern 開頭 7 bytes 要寫死成 `\x49\x8B\x8F\xD8\x00\x00\x00`；Task 2 驗證新版 exe 唯一命中，stub 讀 r15。

---

### Task 1: autobot cave 換新 pattern 與大小

**Files:**
- Modify: `src/reader/hook/cave.rs:14-24`（pattern、size、doc 註解）、`:139`、`:162`、`:173`（寫 208／4100 的 doc 註解）、`:241-244`（長度測試）
- Modify: `tests/it/hooks_golden.rs`（`vec![0; 4100]`、`[..208]`、測試名 `cave_saves_208_and_zeroes_4100`）
- Modify: `tests/it/mem_reader.rs`（`cave_bytes[..208]` 共 6 處）

**Interfaces:**
- Produces: `AUTOBOT_PATTERN: &str`（長度 308）、`AUTOBOT_SIZE: usize = 3900`，名稱不變。

- [ ] **Step 1: 改長度測試**：`cave.rs` 的 `autobot_pattern_is_208_chars` 改名 `autobot_pattern_is_308_chars`，斷言 `AUTOBOT_PATTERN.len() == 308`。
- [ ] **Step 2: 跑 `cargo test -j 4 --lib autobot_pattern` 確認 FAIL**（目前是 208）。
- [ ] **Step 3: 換 pattern 與大小**，`AUTOBOT_PATTERN` 改為（PR #36 版本，逐字照抄）：

```rust
pub const AUTOBOT_PATTERN: &str = concat!(
    r"\x48\x8B\xC4\x55\x41\x54\x41\x55\x41\x56\x41\x57",
    r"\x48\x8D\xA8\x18\xFE\xFF\xFF\x48\x81\xEC\xC0\x02\x00\x00",
    r"\x48\xC7\x45\xC8\xFE\xFF\xFF\xFF",
    r"\x48\x89\x58\x10\x48\x89\x70\x18\x48\x89\x78\x20",
    r"\x48\x8B\x05....\x48\x33\xC4\x48\x89\x85\xB0\x01\x00\x00",
    r"\x4C\x8B\xE9\x48\x8B\x05....\x80\xB8\xCE\x1D\x02\x00\x00\x0F\x84....",
);
```

  `AUTOBOT_SIZE` 改 3900。doc 註解：pattern 的出處改成「Deimos wizwalker PR #36」，「208 bytes」改成「pattern 字串長度」，「4100 個 0」改成「`AUTOBOT_SIZE` 個 0」，以後改版不用再改註解。
- [ ] **Step 4: 測試去掉寫死的數字**：`hooks_golden.rs`、`mem_reader.rs` 裡的 `vec![0; 4100]` 改成 `vec![0; AUTOBOT_SIZE]`，`cave_bytes[..208]` 改成 `cave_bytes[..AUTOBOT_PATTERN.len()]`（需要時補 import）；`cave_saves_208_and_zeroes_4100` 改名 `cave_saves_pattern_length_and_zeroes_cave`。`repairs_from_python_written_hookstate` 的 `208` 是舊 hookstate 檔的資料，**不要動**。
- [ ] **Step 5: 用 scratchpad 的 `scan.py` 確認新 pattern 在新版 exe 唯一命中**（`0x170bf10`），並跑全套檢查。預期全綠。
- [ ] **Step 6: Commit** `fix(reader): update autobot cave pattern for the 2026-11-12 game build`

### Task 2: root window hook 改成 r15

**Files:**
- Modify: `src/reader/hook/cave.rs:29-30`（pattern 與 doc）、`:208-219`（stub 兩條指令與註解）
- Modify: `tests/golden/hook_stubs.json`（3 個案例的 `output.root.stub`）

**Interfaces:**
- Produces: `ROOT_WINDOW_PATTERN`（名稱不變）；`ROOT_WINDOW` 的 `instruction_length: 7`、`noops: 2` 不變。

- [ ] **Step 1: 更新 golden**：在 3 個案例的 `output.root.stub` 中，把 `498b85d8000000` 換成 `498b87d8000000`，把 `498b8dd8000000` 換成 `498b8fd8000000`（長度不變，回跳 rel32 也不變）。用 scratchpad 腳本改，保留 JSON 原本的縮排格式。
- [ ] **Step 2: 跑 `cargo test -j 4 --test it root_window_stub` 確認 FAIL。**
- [ ] **Step 3: 改 pattern 與 stub**：pattern 改為 `concat!(r"\x49\x8B\x8F\xD8\x00\x00\x00\x48\x8B\x01", r"\x49\x8B\x97\xC8\x00\x00\x00\xFF\x50\x70\x84")`，doc 寫一行 WHY：暫存器要寫死，換暫存器時才會比對失敗，而不是讓遊戲當掉。stub 兩條改成 `0x49, 0x8B, 0x87, 0xD8, 0, 0, 0`（`mov rax,[r15+D8]`）與 `0x49, 0x8B, 0x8F, 0xD8, 0, 0, 0`（原指令），註解跟著改。
- [ ] **Step 4: 確認新 pattern 在新版 exe 唯一命中**（`0x17b3ed6`），並跑全套檢查，預期全綠。
- [ ] **Step 5: Commit** `fix(reader): hook the root window through r15 in the new game build`

### Task 3: AddLine 換新入口

**Files:**
- Modify: `src/reader/hook/add_line.rs:1-3`（模組 doc）、`:20-26`（pattern）、`:29-77`（`stub_bytes`）、`:79-88`（`ADD_LINE`）
- Modify: `tests/golden/hook_stubs.json`（`output.addline.stub`、`output.addline.jump`、`output.stub_bytes`）
- Modify: `tests/it/hooks_golden.rs`、`tests/it/mem_reader.rs`（`addline_bytes[..5]` 共 5 處改成 `[..6]`）

**Interfaces:**
- Produces: `add_line::PATTERN`、`stub_bytes(ring: u64) -> Vec<u8>`（簽名不變）；`ADD_LINE` 改為 `instruction_length: 6`、`noops: 1`，其餘不變。

- [ ] **Step 1: 更新 golden**，用 scratchpad 腳本對每個案例處理：
  - `stub_bytes`：去掉開頭的 `48895c2420`，在結尾補上 `405556574154`。
  - `addline.stub`：等於新的 `stub_bytes` 加上 `E9 rel32`，rel32 為 `(addline_jump + 6) - (addline.stub_addr + len(stub_bytes) + 5)`，以 i32 小端序編碼。
  - `addline.jump`：舊值（`E9 rel32`，rel32 不變）後面補 `90`。
- [ ] **Step 2: 測試的長度跟著改**：把 `addline_bytes[..5]` 改成 `[..6]`，然後跑 `cargo test -j 4 --test it hooks_golden`，預期 FAIL。
- [ ] **Step 3: 改 pattern**，改為：

```rust
pub const PATTERN: &str = concat!(
    r"\x40\x55\x56\x57\x41\x54\x41\x55\x41\x56\x41\x57",
    r"\x48\x8D\xAC\x24....\x48\x81\xEC....\x48\xC7\x44\x24.\xFE\xFF\xFF\xFF",
    r"\x48\x89\x9C\x24....\x48\x8B\x05....\x48\x33\xC4\x48\x89\x85....",
    r"\x4D\x63\xF0\x4C\x8B\xEA\x48\x89\x54\x24.\x48\x8B\xF9\x48\x89\x54\x24.",
    r"\x33\xDB\x89\x5C\x24.\x41\x83\xFE\x05\x7C",
);
```

  doc 寫一行：開頭 6 bytes 是 4 個 push，stub 會搬到結尾執行。
- [ ] **Step 4: 改 `stub_bytes`**：拿掉第一行 `mov [rsp+20],rbx`，在 `popfq` 那行之後加上 `&[0x40, 0x55, 0x56, 0x57, 0x41, 0x54]`，註解為 `// push rbp,rsi,rdi,r12（原指令，搬到擷取之後才不會移動 rsp）`。`ADD_LINE` 改成 `instruction_length: 6, noops: 1`。模組 doc 的「與 Python 版逐一相同」那句刪掉。
- [ ] **Step 5: 跑全套檢查**，預期全綠。
- [ ] **Step 6: 對照新版 exe 驗證 stub**：在 scratchpad 寫腳本，用 golden 第一個案例的 `addline.stub` 與 `stub_addr` 做 capstone 反組譯，確認以下三點：
  - 第一條指令是 `pushfq`。
  - 結尾依序是 `push rbp; push rsi; push rdi; push r12; jmp <addline_jump+6>`。
  - 從 exe 的 `0x14073fa80` 讀 6 bytes，結果等於 `405556574154`；從 `0x14073fa86` 讀 2 bytes，結果是 `41 55`（下一條是完整的 `push r13`）。

  另外確認新 pattern 在新版 exe 唯一命中（`0x73ee80`）。
- [ ] **Step 7: Commit** `fix(reader): hook ChatBox::AddLine at its new prologue`

### Task 4: 文件與實機驗證

**Files:**
- Add: `docs/superpowers/specs/2026-11-12-game-update-hooks-design.md`、`docs/superpowers/plans/2026-11-12-game-update-hooks.md`

- [ ] **Step 1: 跑全套檢查**（即使只改文件也要跑），然後 commit spec 與 plan：`docs: add design and plan for the 2026-11-12 game build hooks`。
- [ ] **Step 2: 請使用者實機驗證**：先問使用者，再請對方登入進世界內，執行 `cargo run`。確認 log 沒有 `hook does not match this game build`、收訊正常、分頁名稱讀得到（`LABEL_OFFSET` 0x2E0 的結果）。發版由使用者處理。
