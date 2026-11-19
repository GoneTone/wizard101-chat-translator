# hook 跟著遊戲改版自己調整 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 掛入時從比對到的位元組推導暫存器與位移，並改用自配的 stub 區，讓「只換暫存器／堆疊大小／開頭寫法」的遊戲改版不必再改程式。

**Architecture:** 定位（掃描＋檢查，純讀取）與掛入（配置＋寫入）分開。`attach` 先定位 root window 與 AddLine 兩處，都通過後才配置 stub 區、寫入跳轉。stub 區由 `ProcessMemory::alloc_near` 配置在遊戲模組 ±2 GB 內，取代 autobot cave。

**Tech Stack:** Rust、`regex::bytes`（既有 `game::pattern::scan`）、`windows-sys`（`VirtualQueryEx`／`VirtualAllocEx`）。

**Spec:** `docs/superpowers/specs/2026-11-19-adaptive-hooks-design.md`

## Global Constraints

- **分支與 commit**
  - 在原目錄開分支 `fix/adaptive-hooks`，不建 worktree。第一個 commit 放 spec 與本計畫。
  - commit message 用英文 conventional commits，結尾加 `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`。
  - 不 push。
- **每次 commit 前**：依序跑以下指令，全部零警告、全綠才 commit。
  - `cargo fmt --check`
  - `cargo clippy -j 4 --all-targets -- -D warnings`
  - `cargo test -j 4 -- --test-threads=4`
  - 全套測試每個 task 只跑一次，開發中只跑相關測試。
- **log**
  - 一律用 `log(...)`，訊息用英文，前綴 `[reader]`。
  - 要帶 slot、pid、位址、推導出的暫存器或 push 數、失敗的檢查項目與比對到的位元組（hex）。
- **註解**
  - 中文全形標點，預設一行，只寫 WHY。
  - 公開項目寫一行 `///`，模組頂端寫 `//!`。
  - 不寫 pattern 取自哪裡。
- **測試**：整合測試放 `tests/it/`，共用的假遊戲放 `tests/common/fake_game.rs`。
- **文件同步**：CLAUDE.md 與 AGENTS.md 的「記憶體掛入須知」逐字同步。
- **反目標**：任何一項檢查不確定，就掛不上（`HookError::Mismatch`），不寫入遊戲。

## Review Focus

1. **root window 檢查通過、AddLine 檢查失敗**：預期遊戲記憶體完全沒有寫入，也沒有配置，並顯示版本不符。由 Task 4 的 `mismatch_in_either_site_writes_nothing` 驗證。
2. **附近沒有空位可配置**：預期不顯示版本不符橫幅，走一般失敗與 log 去重，遊戲沒有被寫入。由 Task 4 的 `no_room_near_module_is_not_a_version_mismatch` 驗證。
3. **root window 逾時重連**：預期第一次的 stub 區與 export 都被釋放，下一次配置新的一塊，不累積。由 Task 4 的 `root_timeout_frees_arena_before_retry` 驗證。
4. **關閉時遊戲執行緒還在 stub 內**：預期先寫回所有跳轉，`STUB_EXIT_WAIT` 之後才釋放 export 與 stub 區。由 Task 4 的 `uninstall_frees_arena_after_wait` 驗證。
5. **新 regex 在 57 MB 模組上的掃描時間**：預期定位兩處合計不超過約 1 秒。Task 4 在 attach 記下 `located in {ms}ms`，Task 5 實機確認。

---

### Task 1: `alloc_near`：在模組附近配置記憶體

**Files:**
- Modify: `src/game/memory.rs`（trait、`WinProcess`、`FakeMemory`、新純函式 `pick_near`）
- Modify: `tests/common/spy_memory.rs`（轉呼叫）
- Test: `src/game/memory.rs` 內的 `mod tests`

**Interfaces:**
- Produces:
  - `ProcessMemory::alloc_near(&self, near: u64, size: usize) -> Result<u64, MemError>`：配置 `MEM_COMMIT`、可執行可讀寫的記憶體，整塊落在 `near ± NEAR_LIMIT` 內。找不到空位時回 `MemError::Alloc(size)`。用 `free` 釋放。
  - `pub const NEAR_LIMIT: u64 = 0x7FF0_0000;`
  - `pub fn pick_near(free: &[(u64, u64)], near: u64, size: u64) -> Option<u64>`：`free` 是（起點, 長度）的空閒區段。回傳 64 KB 對齊、整塊落在某個空閒區段內、首尾都離 `near` 不超過 `NEAR_LIMIT`、且離 `near` 最近的起點。
  - `FakeMemory::alloc_near`：不看空閒區段，從 `near - 0x1000_0000` 起每次往上 64 KB 配給。這個游標與 `alloc` 的游標分開，測試不必另外設定。
  - `FakeMemory::fail_alloc_near(&self)`：之後的 `alloc_near` 一律回 `Err(MemError::Alloc(size))`，不記 op。

- [ ] **Step 1: 開分支、commit spec 與計畫**

```bash
git switch -c fix/adaptive-hooks
git add docs/superpowers/specs/2026-11-19-adaptive-hooks-design.md docs/superpowers/plans/2026-11-19-adaptive-hooks.md
```

跑 Global Constraints 的三項檢查，然後 commit：`docs: add design and plan for adaptive game hooks`。

- [ ] **Step 2: 寫失敗的測試**

在 `memory.rs` 的 `mod tests` 加：

```rust
#[test]
fn pick_near_prefers_closest_aligned_slot() {
    let near = 0x1_4000_0000;
    // 模組下方有一段空位、上方較遠處也有
    let free = [(0x1_3000_0000, 0x0800_0000), (0x1_9000_0000, 0x1000_0000)];
    assert_eq!(pick_near(&free, near, 0x1000), Some(0x1_37FF_0000));
}

#[test]
fn pick_near_rejects_slots_beyond_limit_and_unaligned_tails() {
    let near = 0x1_4000_0000;
    assert_eq!(pick_near(&[(near + NEAR_LIMIT, 0x10_0000)], near, 0x1000), None);
    assert_eq!(pick_near(&[(0x1_3000_8000, 0x8000)], near, 0x1000), None); // 區段內沒有 64 KB 對齊點
}

#[test]
fn fake_alloc_near_records_alloc_and_can_fail() {
    let mem = FakeMemory::new();
    assert_eq!(mem.alloc_near(0x1_4000_0000, 0x1000).unwrap(), 0x1_3000_0000);
    assert_eq!(mem.alloc_near(0x1_4000_0000, 0x1000).unwrap(), 0x1_3001_0000);
    assert_eq!(mem.alloc(0x10).unwrap(), 0x2000_0000); // 與 alloc 的游標互不影響
    mem.fail_alloc_near();
    assert!(matches!(mem.alloc_near(0x1_4000_0000, 0x1000), Err(MemError::Alloc(0x1000))));
    assert_eq!(mem.ops().len(), 3);
}
```

在既有的 `win_process_works_on_own_process` 旁加 `win_process_alloc_near_lands_within_limit`：對自己的程序以本 exe 的模組基址當 `near` 呼叫 `alloc_near(near, 0x1000)`，斷言 `addr.abs_diff(near) <= NEAR_LIMIT`、`addr % 0x1_0000 == 0`，寫入再讀回 4 bytes 相同，最後 `free`。

- [ ] **Step 3: 確認測試失敗**

Run: `cargo test -j 4 --lib game::memory -- --test-threads=4`
Expected: 編譯失敗，訊息為 `alloc_near`／`pick_near`／`NEAR_LIMIT` 未定義。

- [ ] **Step 4: 實作**

- **`pick_near`**：對每個空閒區段，算出區段內離 `near` 最近、64 KB 對齊的候選起點：
  - 往下取 `align_down(min(end - size, near), 64K)`；
  - 往上取 `align_up(max(base, near), 64K)`。
  
  保留整塊落在區段內、且首尾都在範圍內的候選，回傳 `abs_diff` 最小者。
- **`WinProcess::alloc_near`**：
  - 從 `near.saturating_sub(NEAR_LIMIT)` 到 `near + NEAR_LIMIT`，用 `VirtualQueryEx` 逐段走，收集 `State == MEM_FREE` 的區段，交給 `pick_near`。
  - 用 `VirtualAllocEx(addr, size, MEM_RESERVE | MEM_COMMIT, PAGE_EXECUTE_READWRITE)` 配置。
  - 失敗時照 `alloc` 經 `self.failure(MemError::Alloc(size))` 回傳。
- **`FakeMemory::alloc_near`**：依上面 Interfaces 的游標規則配給，記一筆 `MemOp::Alloc`，並放入 `size` 個 0；`fail_alloc_near` 打開時直接回錯誤。
- **`SpyMemory`**：轉呼叫給內部的 mem。

- [ ] **Step 5: 確認測試通過**

Run: `cargo test -j 4 --lib game::memory -- --test-threads=4`
Expected: PASS（含 `win_process_alloc_near_lands_within_limit`）。

- [ ] **Step 6: 跑三項檢查並 commit**

```bash
git add src/game/memory.rs tests/common/spy_memory.rs
git commit -m "feat(game): allocate executable memory near a module"
```

---

### Task 2: root window 定位與暫存器推導

**Files:**
- Create: `src/reader/hook/root_window.rs`（pattern、定位、檢查、stub 組裝；之後 `RootWindowHook` 也搬來這裡）
- Modify: `src/reader/hook/mod.rs`（`pub mod root_window;`）
- Modify: `src/reader/hook/cave.rs`（`HookError` 加 `Mismatch(String)`）
- Test: `src/reader/hook/root_window.rs` 內的 `mod tests`

**Interfaces:**
- Produces:
  - `HookError::Mismatch(String)`：Display 為 `hook site does not fit this game build: {detail}`。
  - `pub const ROOT_WINDOW_PATTERN: &str`，內容如下（之後刪掉 `cave.rs` 裡的舊常數）：

    ```rust
    pub const ROOT_WINDOW_PATTERN: &str = concat!(
        r"[\x48\x49]\x8B[\x88-\x8F]\xD8\x00\x00\x00\x48\x8B\x01",
        r"[\x48\x49]\x8B[\x90-\x97]\xC8\x00\x00\x00\xFF\x50\x70\x84",
    );
    ```

  - `pub const MATCH_LEN: usize = 21;`
  - `pub struct RootWindowSite { pub jump: u64, pub base: u8, pub original: [u8; 7] }`：`base` 是暫存器編號 0～15。
  - `pub fn decode(bytes: &[u8]) -> Result<(u8, [u8; 7]), String>`：輸入為比對到的 21 bytes。
  - `pub fn locate(mem: &dyn ProcessMemory) -> Result<RootWindowSite, HookError>`：scan 失敗時回 `HookError::Scan`；`decode` 失敗時回 `HookError::Mismatch(format!("root window at {addr:#x}: {why} (bytes={hex})"))`。
  - `pub fn stub_for(export: u64, site: &RootWindowSite) -> Vec<u8>`：不含回跳。
  - `pub fn reg_name(n: u8) -> &'static str`：`0..=15` 對應 `rax`…`r15`，log 用。

- [ ] **Step 1: 寫失敗的測試**

```rust
const NEW_BUILD: &str = "498b8dd8000000488b01498b95c8000000ff507084db"; // r13
const NOV12_BUILD: &str = "498b8fd8000000488b01498b97c8000000ff507084c0"; // r15

#[test]
fn decode_reads_base_register() {
    assert_eq!(decode(&unhex(NEW_BUILD)).unwrap().0, 13);
    assert_eq!(decode(&unhex(NOV12_BUILD)).unwrap().0, 15);
    let rbx = unhex("488b8bd8000000488b01488b93c8000000ff507084c0");
    assert_eq!(decode(&rbx).unwrap(), (3, rbx[..7].try_into().unwrap()));
}

#[test]
fn decode_rejects_unsafe_or_inconsistent_bases() {
    for bad in [
        "498b8dd8000000488b01498b97c8000000ff507084c0", // r13 / r15 不一致
        "488b8dd8000000488b01498b95c8000000ff507084c0", // rbp / r13：REX.B 不一致
        "488b8cd8000000488b01488b94c8000000ff507084c0", // rm=100（rsp）
        "498b8cd8000000488b01498b94c8000000ff507084c0", // rm=100（r12）
        "488b89d8000000488b01488b91c8000000ff507084c0", // rcx
    ] {
        assert!(decode(&unhex(bad)).is_err(), "{bad}");
    }
    let r9 = unhex("498b89d8000000488b01498b91c8000000ff507084c0");
    assert_eq!(decode(&r9).unwrap().0, 9);
}

#[test]
fn pattern_matches_both_builds() {
    let re = pattern::compile(ROOT_WINDOW_PATTERN).unwrap();
    assert!(re.is_match(&unhex(NEW_BUILD)) && re.is_match(&unhex(NOV12_BUILD)));
}

#[test]
fn stub_reads_derived_register_then_replays_original() {
    let site = RootWindowSite { jump: 0, base: 13, original: unhex(NEW_BUILD)[..7].try_into().unwrap() };
    let export = 0x2000_0000u64;
    let expected = [
        &[0x50][..],
        &[0x49, 0x8B, 0x85, 0xD8, 0, 0, 0],
        &[0x48, 0xA3], &export.to_le_bytes(),
        &[0x58],
        &site.original,
    ].concat();
    assert_eq!(stub_for(export, &site), expected);
}

#[test]
fn locate_reports_mismatch_with_address_and_bytes() {
    // FakeMemory 放一段 r13／r15 不一致的命中
    // 斷言 Err(HookError::Mismatch(msg))，且 msg 含位址的 `{:#x}` 與 bytes 的 hex
}
```

`unhex` 用既有的 `crate::util::unhex`。

- [ ] **Step 2: 確認測試失敗**

Run: `cargo test -j 4 --lib reader::hook::root_window -- --test-threads=4`
Expected: 編譯失敗，訊息為模組或函式不存在。

- [ ] **Step 3: 實作 `decode`／`locate`／`stub_for`／`reg_name`，並在 `HookError` 加 `Mismatch`（含 `cave.rs` 的 Display 分支）**

- **`decode` 的檢查**：
  - 兩條指令的 `(REX & 1) << 3 | (ModRM & 7)` 必須相同；
  - `ModRM & 7 != 4`；
  - 編號不可為 1（rcx）。
  
  錯誤字串用英文，寫明是哪一項不符。
- **`locate`**：`pattern::scan` 之後讀 `MATCH_LEN` bytes，再交給 `decode`。
- **stub 的 `mov rax,[base+D8]`**：位元組為 `0x48 | (base >> 3)`、`0x8B`、`0x80 | (base & 7)`、`D8 00 00 00`。

- [ ] **Step 4: 確認測試通過**

Run: `cargo test -j 4 --lib reader::hook::root_window -- --test-threads=4`
Expected: PASS

- [ ] **Step 5: 跑三項檢查並 commit**

`feat(reader): derive the root window base register from the matched bytes`

---

### Task 3: AddLine 定位、開頭解析與 stub 參數化

**Files:**
- Modify: `src/reader/hook/add_line.rs`（新增 `ANCHOR_PATTERN`、`AddLineSite`、`decode_prologue`、`locate`、`stub_for`；舊的 `PATTERN`、`stub_bytes` 先留著，Task 4 換掉呼叫端時再刪，所以本 task 的 commit 仍然全綠）
- Test: `src/reader/hook/add_line.rs` 內的 `mod tests`

**Interfaces:**
- Consumes: `HookError::Mismatch`（Task 2）。
- Produces:
  - `ANCHOR_PATTERN`：

    ```rust
    pub const ANCHOR_PATTERN: &str = concat!(
        r"(?:[\x48\x4C]\x89[\x44\x4C\x54\x5C\x6C\x74\x7C]\x24.)*",
        r"(?:[\x40\x41]?[\x50-\x57]){1,8}",
        r"\x48\x8D\xAC\x24....",
        r".{0,96}?\x83[\xF8-\xFF]\x05\x7C",
        r".{0,40}?\x48\x08\x00\x00",
    );
    ```

    兩段長度上限 96、40 是初始值。實作者用 Step 5 的實檔檢查確認新版唯一命中，必要時調整。
  - `pub const PROLOGUE_READ: usize = 48;`
  - `pub struct AddLineSite { pub jump: u64, pub pushes: u8, pub original: [u8; 8] }`：`jump` 是 `lea` 的位址。
  - `pub fn decode_prologue(bytes: &[u8]) -> Result<(u8, usize), String>`：回傳（push 數, `lea` 在 bytes 內的位移）。
  - `pub fn locate(mem: &dyn ProcessMemory) -> Result<AddLineSite, HookError>`。
  - `pub fn stub_for(ring: u64, site: &AddLineSite) -> Vec<u8>`：讀回傳位址的 disp8 為 `8 * pushes + 0x38`，開頭照抄 `site.original`，其餘位元組與舊的 `stub_bytes` 相同。

- [ ] **Step 1: 寫失敗的測試**

```rust
// 新版 0x140758cf0 起，到 add rsi,[rcx+848] 為止
const NEW_BUILD: &str = concat!(
    "48895c2420", "5556574154415541564157", "488dac24e0feffff",
    "4881ec20020000", "488b051240c302", "4833c4", "48898518010000",
    "4489442458", "48895588", "488bf9", "48895590", "4183f805", "7c14",
    "4963f0", "4883ee05", "48c1e605", "4803b148080000",
);
// 11/12 版的開頭形狀；0x848 那段為合成
const NOV12_BUILD: &str = concat!(
    "40555657415441554156415748", "8dac24d0feffff", "4881ecc0020000",
    "48c7442440feffffff", "48899c2410030000", "488b050a0a0a0a", "4833c4",
    "4889850a0a0a0a", "4d63f0", "4c8bea", "4889542440", "488bf9",
    "4889542448", "33db", "895c2420", "4183fe057c0a", "90909090", "4803b148080000",
);

#[test]
fn pattern_matches_both_builds_from_function_start() {
    let re = pattern::compile(ANCHOR_PATTERN).unwrap();
    for hex in [NEW_BUILD, NOV12_BUILD] {
        let bytes = [&[0xCC; 16][..], &unhex(hex)].concat();
        assert_eq!(re.find(&bytes).unwrap().start(), 16, "{hex}");
    }
}

#[test]
fn decode_prologue_counts_pushes_and_finds_lea() {
    assert_eq!(decode_prologue(&unhex(NEW_BUILD)).unwrap(), (7, 16));
    assert_eq!(decode_prologue(&unhex(NOV12_BUILD)).unwrap(), (7, 12));
}

#[test]
fn decode_prologue_rejects_other_instructions_before_lea() {
    // push rbp; mov rbp,rcx; lea rbp,[rsp-120]：開頭混入會改暫存器的指令
    assert!(decode_prologue(&unhex("554889cd488dac24e0feffff")).is_err());
    // 沒有 push
    assert!(decode_prologue(&unhex("48895c2420488dac24e0feffff")).is_err());
}

#[test]
fn stub_return_offset_follows_push_count() {
    let mut site = AddLineSite { jump: 0, pushes: 7, original: unhex("488dac24e0feffff").try_into().unwrap() };
    let stub = stub_for(0x3000_0000, &site);
    assert_eq!(&stub[..8], &site.original);
    assert_eq!(&stub[17..22], &[0x4C, 0x8B, 0x4C, 0x24, 0x70]);
    assert_eq!(stub[8..], stub_bytes(0x3000_0000)[8..]); // 其餘與舊 stub 相同
    site.pushes = 5;
    assert_eq!(stub_for(0x3000_0000, &site)[21], 0x60);
}

#[test]
fn locate_rejects_unaligned_match() {
    // FakeMemory 放 NEW_BUILD，起點為 16 的倍數 + 8
    // 斷言 Err(HookError::Mismatch(msg))，msg 含 "aligned"
}

#[test]
fn locate_returns_lea_address_and_pushes() {
    // FakeMemory 在 0x1_4075_8CF0 放 NEW_BUILD
    // 斷言 jump == 0x1_4075_8D00、pushes == 7、original == lea 那 8 bytes
}
```

- [ ] **Step 2: 確認測試失敗**

Run: `cargo test -j 4 --lib reader::hook::add_line -- --test-threads=4`
Expected: 編譯失敗。

- [ ] **Step 3: 實作 `decode_prologue`／`locate`／`stub_for`**

- **`decode_prologue`**：從 bytes 開頭依序吃掉以下指令：
  1. 「存暫存器」：`[48|4C] 89 <ModRM ∈ {44,4C,54,5C,6C,74,7C}> 24 <disp8>`，0 次以上。
  2. push：`[40|41]? 5x`，1～8 次。
  3. 接著必須是 `48 8D AC 24` 加 4 bytes 的 `lea`。
  
  其他任何位元組都回錯誤（英文說明哪個位移遇到什麼）。
- **`locate`**：
  - `scan` 之後先檢查 `addr % 16 == 0`，不符時回 `Mismatch`，訊息含 `not 16-byte aligned`。
  - 讀 `PROLOGUE_READ` bytes 給 `decode_prologue`。
  - `jump = addr + lea 位移`，`original` 取那 8 bytes。

- [ ] **Step 4: 確認測試通過**

Run: `cargo test -j 4 --lib reader::hook::add_line -- --test-threads=4`
Expected: PASS

- [ ] **Step 5: 用本機已安裝的新版 exe 驗證兩個 pattern**

寫一支 scratchpad Python 腳本（不進版控），讀 `D:/Program Files (x86)/Steam/steamapps/common/Wizard101/Bin/WizardGraphicalClient.exe` 的記憶體映像（`pefile` 的 `get_memory_mapped_image()`），用 `re`（`re.S`）以 `root_window::ROOT_WINDOW_PATTERN`、`ANCHOR_PATTERN` 的字串原樣掃描。

Expected：
- root window 唯一命中 `0x14189400b`，推導出 r13。
- AddLine 唯一命中 `0x140758cf0`，`jump` 為 `0x140758d00`，push 數 7。

不符就調整 pattern 的長度上限，再重跑 Step 1～4。

- [ ] **Step 6: 跑三項檢查並 commit**

`feat(reader): locate AddLine by stable anchors and derive the push count`

---

### Task 4: 換上 stub 區並接進 reader

**Files:**
- Modify: `src/reader/hook/cave.rs`
  - `AutobotCave` 改為 `StubArena`；
  - 刪 `AUTOBOT_PATTERN`、`AUTOBOT_SIZE`、舊的 `ROOT_WINDOW_PATTERN`；
  - `SimpleHook` 改為接收 `jump`；
  - `RootWindowHook` 搬到 `root_window.rs`；
  - 模組文件改寫，拿掉「與 wizwalker 逐一相同」。
- Modify: `src/reader/hook/add_line.rs`（`AddLineHook::install` 改收 site；刪掉舊的 `PATTERN` 與 `stub_bytes`）
- Modify: `src/reader/hook/state.rs`（`repair` 去掉 cave 特例與 `wait` 參數）
- Modify: `src/reader/mem_reader.rs`（`Attached.cave` 改為 `arena`、先定位後掛入、`is_pattern_failure`、log）
- Modify: `tests/common/fake_game.rs`、`tests/it/hooks_golden.rs`、`tests/it/mem_reader.rs`、`tests/golden/hook_stubs.json`、`tests/golden/hookstate_bytes.json`（若內容含 cave）
- Modify: `CLAUDE.md`、`AGENTS.md`

**Interfaces:**
- Consumes: Task 1 的 `alloc_near`、`fail_alloc_near`；Task 2 的 `root_window::{locate, stub_for, RootWindowSite, reg_name}`；Task 3 的 `add_line::{locate, stub_for, AddLineSite, ANCHOR_PATTERN}`。
- Produces:
  - `pub const ARENA_SIZE: usize = 0x1000;`
  - `StubArena`：
    - `install(mem: &dyn ProcessMemory) -> Result<Self, HookError>`：`alloc_near(mem.module_base(MODULE)?, ARENA_SIZE)`。
    - `addr(&self) -> u64`。
    - `alloc_stub(&mut self, size: usize) -> Result<u64, HookError>`。
    - `restore_ops(&self) -> Vec<(u64, Vec<u8>)>`：只有跳轉點。
    - `uninstall_with(self, mem, wait)`：依序寫回跳轉，然後 `wait(STUB_EXIT_WAIT)`，再釋放 export，最後釋放 stub 區；程序已結束時直接回 `Ok`。
  - `HookError::CaveFull` 的 Display 改為 `stub arena is full`。
  - `SimpleHook { instruction_length, noops, stub_size, export_size, free_export }`。
    - `install(&self, mem, arena: &mut StubArena, jump: u64, body: impl FnOnce(u64) -> Vec<u8>) -> Result<u64, HookError>`，回傳 export 位址。
  - `RootWindowHook::install(mem, arena: &mut StubArena, site: &RootWindowSite) -> Result<Self, HookError>`。
  - `AddLineHook::install(mem, arena: &mut StubArena, site: &AddLineSite) -> Result<Self, HookError>`。
  - `hook::state::repair(dir, mem, pid, slot) -> usize`：全部依序寫回，不等待。
  - `fake_game` 提供：
    - `place_root_window(mem, addr, hex: &str) -> Vec<u8>`；
    - `place_addline(mem, addr, hex: &str) -> Vec<u8>`；
    - 預設內容 `ROOT_NEW_BUILD`、`ADDLINE_NEW_BUILD` 兩個常數，與 Task 2、3 的測試資料相同；
    - 刪掉 `place_cave` 和 `pattern_instance`（如果沒有別的地方用到）。

- [ ] **Step 1: 改測試以反映新行為（先讓它們失敗）**

- **`hooks_golden.rs`**
  - `install_case` 改用 `StubArena`。
  - golden 案例的 input 改為 `root_jump`、`root_hex`、`addline_jump`、`addline_hex`、`export`、`ring`。stub 區位址由假記憶體的 `alloc_near` 決定（`BASE - 0x1000_0000`），記在 output 裡。
  - 預期的 op 順序是：
    1. 配置 stub 區；
    2. 配置 export；
    3. 寫 root stub；
    4. 寫 root 跳轉；
    5. 配置 ring；
    6. 寫 AddLine stub；
    7. 寫 AddLine 跳轉。
  
  `hook_stubs.json` 至少兩個案例：新版（r13、7 個 push）和 11/12 版（r15、7 個 push）；另外再加一個 rbx 加 5 個 push 的合成案例。
- **`mem_reader.rs`** 的 fixture 改用 `place_root_window`／`place_addline` 和 `set_next_alloc`。凡是斷言 `CAVE` 寫入、或還原清單含 cave 的測試，都改成斷言 stub 區的 `Alloc`／`Free` 與跳轉的寫回。另外新增四個測試：
  - `mismatch_in_either_site_writes_nothing`：分兩次，放不一致的 root window 或沒對齊的 AddLine。斷言 `ops()` 裡沒有 `Write` 也沒有 `Alloc`，而且回報的是版本不符。
  - `no_room_near_module_is_not_a_version_mismatch`：呼叫 `fail_alloc_near()`。斷言沒有 `Write`，錯誤不是版本不符。
  - `root_timeout_frees_arena_before_retry`：root window 不觸發、等到逾時。斷言 stub 區與 export 都有 `Free`，重連時配置新的一塊。
  - `uninstall_frees_arena_after_wait`：斷言順序是「所有跳轉寫回 → 等待 → `Free(export)` → `Free(arena)`」。
- **`state.rs`**：`repair` 的測試拿掉等待與 cave 特例，斷言全部 op 依序寫回。

- [ ] **Step 2: 確認測試失敗**

Run: `cargo test -j 4 --test it -- hooks_golden mem_reader --test-threads=4`
Expected: 編譯失敗或斷言失敗，原因是舊 API 和 autobot 寫入。

- [ ] **Step 3: 實作 `StubArena`、`SimpleHook`、兩個 `install`、`repair`**

- **`SimpleHook::install`**：除了不再自己 scan，順序照舊：
  1. 切 stub；
  2. 配置 export；
  3. 組 stub 加回跳；
  4. 讀原碼；
  5. 寫 stub；
  6. 寫跳轉；
  7. 登記。
- **`RootWindowHook`**：沿用 `instruction_length: 7, noops: 2, stub_size: 50, export_size: 8, free_export: true`。
- **`AddLineHook`**：沿用 `instruction_length: 8, noops: 3, stub_size: 256, free_export: false`。

- [ ] **Step 4: 改 `mem_reader::attach`**

依序：
1. `root_window::locate` 和 `add_line::locate` 兩處都定位完。
2. 記一行 log，帶 slot、pid、兩處跳轉點（換算成 `IMAGE_BASE` 位址）、`reg_name(base)`、push 數，以及 `located in {ms}ms`。
3. `StubArena::install`，然後 `save_hook_state`。
4. `RootWindowHook::install`，然後 `save_hook_state`。
5. 等 root ready。
6. `install_chat_hook(&site)`。

其他改動：
- `is_pattern_failure` 加上 `HookError::Mismatch(_)`。
- `install_chat_hook` 的失敗 log 拿掉 `needs hook::add_line::PATTERN updated`，改為 `a mismatch here means the game changed more than registers or stack size`。
- stub 區配置失敗走既有的一般失敗路徑，不視為版本不符。

- [ ] **Step 5: 重新推導 golden stub，用 capstone 核對**

依 Task 2、3 的 `stub_for` 規則，手動推出 `hook_stubs.json` 每個案例的 stub 與跳轉位元組。然後用 scratchpad 的 Python capstone 反組譯每段 stub，確認：
- root stub 第二條是 `mov rax, qword ptr [<推導的暫存器> + 0xd8]`，最後一條是原指令；
- AddLine stub 第一條是原本的 `lea`，讀回傳位址的是 `mov r9, qword ptr [rsp + <8×push+0x38>]`；
- 跳轉的 rel32 指到 stub 區。

- [ ] **Step 6: 確認測試通過**

Run: `cargo test -j 4 --test it -- hooks_golden mem_reader --test-threads=4`，接著跑 `cargo test -j 4 --lib reader -- --test-threads=4`。
Expected: PASS

- [ ] **Step 7: 文件與殘留**

- **CLAUDE.md 與 AGENTS.md** 的「記憶體掛入須知」改成以下內容，兩份逐字相同：
  - AddLine 和 root window 的 pattern 只寫死指令形狀與結構偏移，暫存器、push 數、堆疊位移在掛入時從比對到的位元組推導；
  - stub 放在掛入時於模組附近配置的記憶體；
  - `app.log` 出現 `does not fit this game build` 或 pattern 比對失敗，代表指令形狀或偏移變了，要更新 `root_window.rs` 的 `ROOT_WINDOW_PATTERN` 或 `add_line.rs` 的 `ANCHOR_PATTERN`；
  - 不再提 `AUTOBOT_PATTERN`。
- **全專案搜尋殘留字樣**：`grep -rn "autobot\|AUTOBOT\|AutobotCave\|wizwalker 寫入" src tests CLAUDE.md AGENTS.md`，只能剩下與 wizwalker 慣例無關的說明。

- [ ] **Step 8: 跑三項檢查並 commit**

`refactor(reader): replace the autobot cave with an arena near the module`

---

### Task 5: 實機驗證（使用者執行）

- [ ] **Step 1**：請使用者登入進世界內後 `cargo run -j 4`，並貼回終端機輸出。原始碼執行時 log 印在 stderr。
- [ ] **Step 2**：從輸出確認以下幾點：
  - `located in …ms` 約 1 秒以內；
  - 推導出 `r13`、7 個 push；
  - 兩個 hook 都掛上，沒有出現版本不符。
- [ ] **Step 3**：請使用者實際確認：
  - 收訊有翻譯；
  - 切分頁時分頁名稱正確；
  - 輸入框偵測正常；
  - 關閉翻譯器時遊戲沒有當掉。
  
  `+0x7A0`、`LABEL_OFFSET`、視窗樹偏移讀不到的話，就停下來回報，不要猜改偏移。
- [ ] **Step 4**：都通過後，依使用者指示開 PR。PR 標題用英文，描述寫明支援的遊戲版本字串 `W.1.620.01`。
