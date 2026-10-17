# Rust 重寫第 2 段（發話：快捷鍵、輸入框、Ctrl+V 攔截與逐字輸入）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在第 1 段的 Rust 版上補齊發話：全域快捷鍵呼出翻譯輸入框、譯文逐字送回遊戲、遊戲內 Ctrl+V 攔截貼上，行為與 Python v0.6.0 一致。

**Architecture:**
- 鍵盤：自己的 `WH_KEYBOARD_LL` hook 執行緒取代 Python 的 `keyboard` 套件。按鍵身分沿用 `keyboard` 的「scan code 集合」模型，名稱表逐行移植自 `keyboard/_winkeyboard.py`，以 golden 對照。
- 每顆按鍵事件交給 `Dispatcher` 決定吞不吞（純邏輯，可測），hook 執行緒只負責接 Win32。
- 發話分三層：`input`（按鍵與分派）、`composer`（SendInput 逐字輸入、剪貼簿貼上）、`ui::input_box`（egui immediate viewport）。
- 跨執行緒一律走既有的 `UiEvent` channel；hook 回呼裡只投遞事件或起執行緒，不做重活。

**Tech Stack:** 沿用第 1 段（eframe/egui 0.37、windows-sys 0.61、tokio、egui_kittest）。windows-sys 新增 feature `Win32_UI_Input_KeyboardAndMouse`、`Win32_System_Console`；不新增 crate。

**Spec:** `docs/superpowers/specs/2026-10-17-rust-rewrite-design.md`，§E 是本段的驗收依據。每個 task 的行為以「對應的 Python 原始碼」為準：
- `src/input/hotkeys.py`、`src/input/dispatch.py`
- `src/composer/paste.py`
- `src/ui/input_box.py`
- `src/main.py:277-341`（接線）

`keyboard` 套件的行為以 `.venv/Lib/site-packages/keyboard/` 的原始碼為準。另外要納入 `docs/superpowers/plans/2026-10-17-rust-rewrite-followups.md` 的「第 2 段」兩項。

## Global Constraints

- **提交前檢查依改動範圍**（使用者 2026-10-17 決定，取代第 1 段「五項全跑」）：只動 `rust/` 時跑三項 cargo 檢查（`fmt --check`、`clippy --all-targets -D warnings`、`test`）；有動到 `src/`、`tests/`、`tools/` 或 `pyproject.toml` 時另跑 `uv run ruff check src tests tools` 與 `uv run pytest`；只動文件不必跑。
- 第 1 段計畫的 Global Constraints 其餘全部沿用：log 規則、註解規則、i18n 不新增 key、測試不干擾桌面、移植測試、commit 格式。
- **測試絕不可安裝真的鍵盤 hook，也不可呼叫 `SendInput`／`keybd_event`。**
  - 測試會攔到使用者的實體鍵盤，或把字打進使用者的前景視窗。
  - 所有 Win32 動作都經可注入的介面（trait 或 fn 欄位），測試換成假的。
  - 真的 hook 與 SendInput 只列入實機驗收。
- hook 回呼（含 `Dispatcher::process` 呼叫到的所有 handler）必須立刻返回：
  - 只能投遞 `UiEvent`、起執行緒、讀共享狀態；
  - 不可 sleep、不可做 I/O（`foreground_exe` 的 OpenProcess 除外，與 Python 相同）；
  - 不可呼叫會回頭鎖 `Dispatcher` 的函式。

  否則 Windows 判定 hook 逾時，會把它整個拔掉。
- log 前綴對齊 Python：`[hotkey]`、`[composer]`、`[input]`、`[app]`。字串盡量與 Python 同一行 log 相同，`{x!r}` 用 `crate::py::repr_str` 重現。
- 新的 egui viewport：
  - 一律無邊框，建好就呼叫 `ui::win32::remove_system_frame`；
  - **不可用視窗標題找 HWND**，要用建立前後的 `thread_windows()` 比對（followups 第 2 段第 2 項）。

## Review Focus

1. **輸入法選字時按 Enter**：中日韓使用者要靠 IME 打字。Enter 確認候選字時，不可觸發翻譯；Esc 取消組字時，不可關掉輸入框。測試放在 Task 7：`ime_enter_commits_composition_without_submitting`。
2. **hook 回呼裡的 handler panic**：事件要照常放行，不可拖垮 hook 執行緒或讓整個鍵盤卡住。測試放在 Task 2：`panicking_handler_passes_the_event_through`。
3. **剪貼簿或譯文含 emoji 等 BMP 外字元**：要送出一組 surrogate pair（兩個 down 再兩個 up，與 `keyboard.type_unicode` 相同），不可拆成兩個字、也不可丟字。測試放在 Task 4：`astral_char_is_sent_as_one_surrogate_pair`。
4. **遊戲輸入框在 150% 縮放的螢幕上**：輸入框寬度要等於錨點的實體寬度；最小寬度與預設寬度以 DIP 計，乘上縮放。測試放在 Task 7：`width_follows_the_anchor_in_physical_pixels_at_150_percent`。
5. **overlay 縮成泡泡時按快捷鍵**：輸入框照樣出現，泡泡與 overlay 狀態不變。測試放在 Task 8：`hotkey_opens_the_input_box_while_minimized`。

---

### Task 1: `keyboard` 套件的按鍵名稱表與快捷鍵解析

**Files:**
- Create: `rust/src/input/mod.rs`, `rust/src/input/keys.rs`
- Modify: `rust/src/lib.rs`（`pub mod input;`）、`rust/Cargo.toml`（windows-sys feature `Win32_UI_Input_KeyboardAndMouse`）
- Modify: `tools/golden/export.py`（新增 `keyboard_tables`、`keyboard_names`、`keyboard_parse`）
- Test: `rust/tests/keys_golden.rs`

**Interfaces:**
- Produces（`crate::input::keys`）：
  - `pub type Code = i32;`：與 `keyboard` 的事件 `scan_code` 相同。值是原始 scan code，scan code 為 0 時是 `-vk`。
  - `pub type Key = Vec<Code>;`：一顆鍵的所有替代 code，順序與 `key_to_scan_codes` 相同。
  - `pub fn normalize_name(name: &str) -> Result<String, String>`
  - `pub fn key_to_scan_codes(name: &str) -> Result<Key, String>`：錯誤字串為 `Key 'foo' is not mapped to any known key.`。
  - `pub fn key_codes_or_empty(name: &str) -> Key`：對應 `error_if_missing=False`。
  - `pub fn parse_hotkey(hotkey: &str) -> Result<Vec<Vec<Key>>, String>`：回傳 steps → keys → codes。
  - `pub fn is_modifier(code: Code) -> bool`
  - `pub fn scan_code_to_vk(code: Code) -> u16`：給注入按鍵用，對應 `_winkeyboard.scan_code_to_vk`；`code < 0` 時回 `-code`。

- [ ] **Step 1:** 在 `tools/golden/export.py` 新增三個 exporter（都要先呼叫 `keyboard._winkeyboard._setup_name_tables()`）：
  - `keyboard_tables`：Rust 端會 `include_str!` 這份當資料來源，不只是對照。只有一筆 `{"input": "tables", "output": {...}}`，output 包含：
    - `canonical_names`（dict）
    - `official_virtual_keys`（vk → 名稱 tuple 轉 list）
    - `keypad_keys`
    - `distinct_modifiers`
    - `sided_modifiers`
    - `all_modifiers`（排序後的 list）
  - `keyboard_names`：`from_name` 的每個 key（排序），input 為名稱，output 為 `list(keyboard.key_to_scan_codes(name))`。另外對 `canonical_names` 的每個 key 也匯出一筆，失敗時 output 為 `{"error": True}`。
  - `keyboard_parse`：固定清單。input 為字串，output 為 `[[list(codes) for key in step] for step in parse_hotkey(s)]`，`ValueError` 時為 `{"error": True}`。清單至少涵蓋：
    - `"ctrl+space"`、`"ctrl+shift+space"`
    - 大小寫與空白變化：`"Ctrl + Space"`、`"CTRL+SPACE"`
    - 別名：`"control+spacebar"`、`"win+a"`、`"left windows+e"`、`"right ctrl+a"`、`"alt gr+e"`
    - 單一字元：`"a"`、`"A"`、`"+"`、`"!"`、`"ctrl+5"`、`"ctrl+`"`
    - 功能鍵與小鍵盤：`"f1"`、`"shift+f12"`、`"num 1"`、`"ctrl+up"`
    - 多段：`"ctrl+a, s"`、`"ctrl+a,s"`
    - 只有修飾鍵：`"ctrl+shift"`、`"alt"`
    - 錯誤：`"ctrl+foo"`、`"ctrl++"`、`""`、`" "`

  然後執行 `uv run python -m tools.golden.export keyboard_tables keyboard_names keyboard_parse`。
- [ ] **Step 2:** 寫 `rust/tests/keys_golden.rs`：
  - `key_to_scan_codes_matches_python`：逐筆比對 `keyboard_names`。
  - `parse_hotkey_matches_python`：逐筆比對 `keyboard_parse`。
  - `modifier_codes_match_python`：`all_modifiers` 每個名稱的 code，`is_modifier` 都回 true；`"a"`、`"space"` 的 code 回 false。

  跑 `cargo test --manifest-path rust/Cargo.toml --test keys_golden`，預期編譯失敗。
- [ ] **Step 3:** 實作 `keys.rs`，逐行移植以下函式（只移植列出的函式，不移植事件名稱查詢）：
  - `_canonical_names.normalize_name`
  - `_winkeyboard.get_event_names`、`_setup_name_tables`（含 alt gr 的手動補項與 `order_key` 排序）、`map_name`
  - `__init__.key_to_scan_codes`、`parse_hotkey`、`is_modifier`

  實作細節：
  - 表格資料以 `include_str!("../../tests/golden/keyboard_tables.json")` 讀進 `LazyLock`。
  - 名稱表在第一次查詢時才建立，同樣放 `LazyLock`。
  - Win32 呼叫用 `MapVirtualKeyExW`、`ToUnicode`（呼叫兩次以清掉 dead key 狀態）、`GetKeyNameTextW`、`MapVirtualKeyW`。
  - `parse_hotkey` 的切分要重現 `re.split(r',\s?')` 與 `re.split(r'\s?\+\s?')`。字串長度為 1 時直接當單一鍵。
- [ ] **Step 4:** 再跑同一個測試，預期 PASS。
- [ ] **Step 5:** 若 CI 的 `--check` 與本機產生的 `keyboard_names` 不同（鍵盤配置差異），把差異寫進 commit body 並回報 controller，不要自行刪減清單。
- [ ] **Step 6:** 跑提交前檢查後 commit：`feat(rust): port keyboard key-name tables and hotkey parsing`。

### Task 2: 快捷鍵攔截器與按鍵分派器（純邏輯）

**Files:**
- Create: `rust/src/input/hotkeys.rs`, `rust/src/input/dispatcher.rs`
- Test: 同檔 `#[cfg(test)]`

**Interfaces:**
- Consumes：Task 1 的 `parse_hotkey`、`key_codes_or_empty`、`is_modifier`、`Code`、`Key`。
- Produces：
  - `crate::input::dispatcher`：
    - `pub struct KeyEvent { pub down: bool, pub code: Code }`
    - `pub type Pressed = std::collections::BTreeSet<Code>;`
    - `pub trait Blocking: Send { fn handle(&mut self, ev: KeyEvent, pressed: &Pressed) -> bool; }`：回 true 放行、false 吞掉。
    - `pub struct Dispatcher`
      - `new()`
      - `add_blocking(Box<dyn Blocking>) -> HandlerId`
      - `add_passive(steps: Vec<Vec<Key>>, on_trigger: Box<dyn FnMut() + Send>) -> HandlerId`
      - `remove(HandlerId)`
      - `process(ev: KeyEvent) -> bool`
      - `pressed() -> &Pressed`
    - `pub type SharedDispatcher = Arc<Mutex<Dispatcher>>;`
    - `pub fn is_pressed(pressed: &Pressed, codes: &[Code]) -> bool`
  - `crate::input::hotkeys`：
    - `pub const MASK_VK: u16 = 0xE8;`
    - `pub const MODIFIER_GROUPS: [&str; 4] = ["ctrl", "shift", "alt", "windows"];`
    - `pub enum CannotSwallow { MultiStep, OnlyModifiers }`
    - `pub struct HotkeyInterceptor`
      - `new(hotkey: &str, on_trigger: Box<dyn FnMut() -> bool + Send>, mask: Box<dyn Fn() + Send>) -> Result<Result<Self, CannotSwallow>, String>`：外層 `Err` 是名稱無效。
      - 實作 `Blocking`。
      - 提供 `pub fn swallow_override(self, f: Box<dyn Fn() -> bool + Send>) -> Self`：給 Task 4 的「鍵入中一律吞 V」用。
    - `pub fn register_hotkey(d: &SharedDispatcher, hotkey: &str, callback: Arc<dyn Fn() -> bool + Send + Sync>, fallback: &str, mask: Arc<dyn Fn() + Send + Sync>) -> (Registration, String)`：回傳實際生效的熱鍵。
    - `pub struct Registration`：`unregister(self)`。

- [ ] **Step 1:** 寫測試。

  `HotkeyInterceptor` 的測試逐一移植 `tests/input/test_hotkeys.py`，pressed 由測試直接組（例如 ctrl 的 code 29、space 的 code 57）：
  - `swallows_the_trigger_key_when_the_combo_is_handled`
  - `passes_the_key_through_when_the_callback_does_not_handle_it`
  - `masks_modifiers_only_when_swallowing`
  - `passes_the_bare_trigger_key_through`
  - `extra_modifier_does_not_match`
  - `extra_non_modifier_key_still_matches`
  - `unrelated_keys_pass_without_calling_back`
  - `key_repeat_is_swallowed_until_release_even_after_the_modifier_is_up`
  - `combo_without_modifiers_swallows_whichever_key_completes_it`
  - `rejects_combos_it_cannot_swallow`：`"ctrl"`、`"ctrl+shift"`、`"ctrl+a, s"`。

  `Dispatcher` 的測試：
  - `swallowed_keys_do_not_count_as_pressed`：吞掉的 down 不進 `pressed`，其 up 也不影響 `pressed`。
  - `first_swallowing_handler_short_circuits_later_ones`：handler 依註冊順序呼叫，有一個吞掉就停，與 `all(...)` 相同。
  - `passive_single_step_fires_on_key_down_when_pressed_equals_the_combo`：按住時的重複 down 會重複觸發；多按一顆鍵就不觸發。
  - `passive_multi_step_fires_after_each_step_in_order`
  - `passive_multi_step_resets_after_one_second`
  - `panicking_handler_passes_the_event_through`（Review Focus 2）：handler panic 時 `process` 回 true，記 `[hotkey] handler panicked`，之後的事件照常處理。

  `register_hotkey` 的測試移植 `test_hotkeys.py` 後 4 個：
  - `register_hotkey_installs_a_swallowing_handler`
  - `register_hotkey_falls_back_to_passive_when_it_cannot_swallow`
  - `register_hotkey_falls_back_to_the_default_on_an_unknown_key`
  - `register_hotkey_falls_back_to_the_given_fallback_on_an_unknown_key`
- [ ] **Step 2:** 跑 `cargo test --manifest-path rust/Cargo.toml --lib input::`，預期 FAIL。
- [ ] **Step 3:** 實作。

  `HotkeyInterceptor` 照 `hotkeys.py` 的 `HotkeyInterceptor`：
  - `_trigger_codes`、`_unwanted_modifiers`、`_held` 都照抄；
  - 吞鍵後呼叫 `mask()`，並記 `[hotkey] {hotkey!r} swallowed; injected mask key vk=0xe8`。

  `Dispatcher::process` 依序做：
  1. 依序呼叫 blocking handler，第一個回 false 就停，整體回 false。每次呼叫包 `catch_unwind`，panic 時當成放行。
  2. 只有全部放行時才更新 `pressed`：down 時加入，up 時移除。
  3. 對每個 passive 熱鍵做比對。

  passive 單段的觸發條件與 `keyboard.add_hotkey`（`suppress=False`）相同：這次是 down，且「排序後的 pressed」等於某一組 `itertools.product` 組合。

  **裁定：多段 passive 的狀態機簡化如下**，因為只有手改 config 才會出現多段：
  - 第 i 段湊齊時：若是最後一段就觸發並歸零，否則前進到 i+1，並記下時間。
  - 下列情況歸零：距上次前進滿 1 秒；或在 i>0 時按下一顆不屬於第 i 段的鍵。
  - 不重播被吃掉的事件（passive 本來就不吃）。

  `register_hotkey` 的 log 三種文字照 `hotkeys.py`：
  - `[hotkey] {hotkey!r} registered; its trigger key is swallowed when handled`
  - `[hotkey] {exc}; registered without swallowing, its keys still reach the game`
  - `[hotkey] {hotkey!r} is not a valid key combination ({err}); using {fallback!r}`

  其中 `{exc}` 是 `{hotkey!r} is a multi-step hotkey` 或 `{hotkey!r} has only modifier keys`。
- [ ] **Step 4:** 跑測試，預期 PASS。
- [ ] **Step 5:** 跑提交前檢查後 commit：`feat(rust): add hotkey interceptor and key event dispatcher`。

### Task 3: 低階鍵盤 hook 執行緒與按鍵注入

**Files:**
- Create: `rust/src/input/hook.rs`, `rust/src/input/inject.rs`
- Modify: `rust/src/app_main.rs`（`shutdown_steps` 加卸 hook 一步）、`rust/tests/app_main.rs`

**Interfaces:**
- Consumes：Task 2 的 `SharedDispatcher`、`KeyEvent`；Task 1 的 `scan_code_to_vk`。
- Produces：
  - `crate::input::hook`：
    - `pub struct KeyboardHook`
    - `pub fn install(d: SharedDispatcher) -> Result<KeyboardHook, String>`：起名為 `keyboard-hook` 的執行緒，在裡面呼叫 `SetWindowsHookExW(WH_KEYBOARD_LL)`，並跑 `GetMessageW` 迴圈。
    - `impl KeyboardHook { pub fn uninstall(self) }`：`PostThreadMessageW(WM_QUIT)` 後 join，最多等 1 秒。
    - `pub fn to_event(vk: u32, scan: u32, flags: u32, msg: u32) -> Option<KeyEvent>`：純函式。
  - `crate::input::inject`：
    - `pub trait Injector: Send + Sync { fn press(&self, code: Code); fn release(&self, code: Code); fn unicode(&self, ch: char); fn mask(&self); }`
    - `pub struct SendInputInjector;`
  - `shutdown_steps(stop, close_pools, unhook_keyboard: impl FnOnce(), supervisor, join_budget, flush_cache)`：新增的參數夾在 `close_pools` 之後。

- [ ] **Step 1:** 寫測試。

  `to_event` 的測試（同檔 `#[cfg(test)]`）：
  - `vk_packet_events_are_ignored`：vk `0xE7` 回 `None`。
  - `injected_alt_down_events_are_ignored`：`flags & 0x30 == 0x30` 回 `None`；只有 `LLKHF_INJECTED`（0x10）的照常處理。
  - `zero_scan_code_becomes_negative_vk`
  - `syskey_messages_map_like_key_messages`：`WM_SYSKEYDOWN` 對應 down，`WM_SYSKEYUP` 對應 up。

  `rust/tests/app_main.rs` 擴充既有的關閉順序測試：`shutdown_unhooks_the_keyboard_after_pools_and_before_joining_readers`，驗證順序為 stop → pools → unhook → supervisor join → flush。
- [ ] **Step 2:** 跑測試，預期 FAIL。
- [ ] **Step 3:** 實作。

  hook 程序：
  - 經 `static DISPATCHER: OnceLock<SharedDispatcher>` 取得分派器，呼叫 `process`；回 false 時回傳 1（吞掉），否則 `CallNextHookEx`。
  - `nCode < 0` 一律直接 `CallNextHookEx`。
  - 鎖中毒時用 `into_inner`。

  `SendInputInjector` 對應 `_winkeyboard` 的各函式：

  | 方法 | 送出的事件 |
  |---|---|
  | `press(code)` | 等同 `keybd_event(vk, code, 0)`：`code > 0` 時 vk 用 `scan_code_to_vk`；`code < 0` 時送 vk `-code`、scan 0 |
  | `release(code)` | 同 `press`，加 `KEYEVENTF_KEYUP` |
  | `unicode(ch)` | 把字元編成 UTF-16，所有單元先送 `KEYEVENTF_UNICODE` down，再全部送 up，一次 `SendInput`，與 `type_unicode` 相同 |
  | `mask()` | vk `0xE8` 的 down 與 up |

  用 `SendInput` 而不是 `keybd_event`，兩者送出的事件相同。不處理 alt gr（541）的特例。

  `Running` 新增 `pub keyboard: Option<KeyboardHook>`：
  - 本 task 先一律設為 `None`，Task 9 才真的安裝。
  - 裝不上時為 `None`，並記 `[hotkey] cannot install keyboard hook: {err}`。
  - `shutdown()` 傳入的卸除閉包在有 hook 時呼叫 `uninstall()`，`None` 時什麼都不做。
- [ ] **Step 4:** 跑測試，預期 PASS。
- [ ] **Step 5:** 跑提交前檢查後 commit：`feat(rust): add low-level keyboard hook thread and key injection`。

### Task 4: composer（逐字鍵入、Ctrl+V 貼上）

**Files:**
- Create: `rust/src/composer/mod.rs`, `rust/src/composer/paste.rs`
- Modify: `rust/src/lib.rs`、`rust/src/ui/clipboard.rs`（新增讀取）、`rust/src/reader/process.rs`（`process_exe_path` 改成 `pub`）
- Test: 同檔 `#[cfg(test)]`

**Interfaces:**
- Consumes：Task 3 的 `Injector`；Task 2 的 `HotkeyInterceptor`、`is_pressed`、`Pressed`；`ui::win32::force_foreground`、`reader::process::{foreground_window, window_exists, is_game_process_path}`。
- Produces：
  - `crate::ui::clipboard::get_text() -> Result<Option<String>, String>`：`Err` 是開不了剪貼簿；`Ok(None)` 是剪貼簿裡沒有文字。
  - `crate::composer::paste`：
    - 常數：
      - `pub const FOCUS_DELAY: Duration = 150 ms;`
      - `pub const PASTE_KEY: &str = "v";`
      - `pub const MODIFIER_RELEASE_TIMEOUT: Duration = 3 s;`
      - `pub const KEY_RELEASE_POLL: Duration = 10 ms;`
    - `pub fn foreground_exe() -> Option<String>`
    - `pub fn typing_in_progress() -> bool`：以全域 `static TYPING: Mutex<()>` 實作，`try_lock` 失敗即為進行中。
    - `pub struct TypingEnv`：所有欄位都可注入，`TypingEnv::real(pressed: Arc<dyn Fn() -> Pressed + Send + Sync>)` 接真的 Win32。
      - `inject: Arc<dyn Injector>`
      - `foreground: fn() -> isize`
      - `is_window: fn(isize) -> bool`
      - `force_foreground: fn(isize) -> bool`
      - `sleep: fn(Duration)`
      - `now: fn() -> Instant`
      - `pressed: Arc<dyn Fn() -> Pressed + Send + Sync>`
      - `clipboard: fn() -> Result<Option<String>, String>`
    - `pub fn type_into_window(env: &TypingEnv, hwnd: isize, text: &str, delay: Duration)`
    - `pub fn paste_clipboard(env: &TypingEnv, hwnd: isize, delay: Duration, single_line: bool) -> bool`
    - `pub fn paste_interceptor(should_intercept: Box<dyn Fn() -> bool + Send>, on_paste: Box<dyn FnMut() + Send>, mask: Box<dyn Fn() + Send>) -> HotkeyInterceptor`

- [ ] **Step 1:** 寫測試，逐一移植 `tests/composer/test_paste.py`。

  鍵入：
  - `typing_never_re_presses_the_modifiers_the_user_held`：每個字送出前，對 `pressed()` 裡的每個 code 呼叫 `release`，之後從不呼叫 `press` 把它們按回去。
  - `type_into_window_types_text_one_char_at_a_time`
  - `type_into_window_flattens_newlines`
  - `type_into_window_stops_when_foreground_leaves_target`
  - `type_into_window_never_sends_enter`
  - `foreground_exe_returns_none_when_lookup_fails`：可改測 `exe_of_window(0)`。

  攔截器：
  - `interceptor_passes_plain_v_through`
  - `interceptor_passes_ctrl_v_through_outside_the_game`
  - `interceptor_swallows_ctrl_v_in_the_game_and_pastes_once`
  - `interceptor_ignores_other_keys`
  - `interceptor_ignores_key_repeat_until_released`
  - `interceptor_keeps_swallowing_v_until_released_even_after_ctrl_is_up`
  - `interceptor_swallows_v_while_typing_is_in_progress`

  剪貼簿與貼上：
  - `clipboard_text_reads_unicode_text` 與 `clipboard_text_returns_none_when_clipboard_has_no_text`：改成測 `paste_clipboard` 對 `clipboard` 回傳值的處理。真的剪貼簿不碰。
  - `paste_clipboard_types_the_clipboard_with_the_configured_delay`
  - `paste_clipboard_skips_empty_clipboard`
  - `paste_clipboard_types_the_text_verbatim_like_native_paste`：CRLF 與 CR 都折成 LF，`\n` 送 Enter，即 `press(28)` 再 `release(28)`。
  - `paste_clipboard_turns_line_breaks_into_spaces_for_the_chat_box`
  - `paste_clipboard_waits_for_ctrl_and_v_to_be_released_before_typing`
  - `paste_clipboard_gives_up_waiting_after_the_timeout`
  - `paste_clipboard_stops_when_foreground_leaves_target`
  - `paste_clipboard_ignores_a_second_paste_while_typing`
  - `typing_in_progress_is_true_only_while_chars_are_being_typed`

  另外加兩項：
  - `backspace_in_clipboard_is_sent_as_the_backspace_key`：`\b` 走 `press`／`release` 送 backspace 的 code，與 `keyboard.write` 相同。
  - `astral_char_is_sent_as_one_surrogate_pair`（Review Focus 3）：`"a😀"` 產生 `unicode('a')`、`unicode('😀')` 兩次呼叫。另以 `encode_units('😀')` 驗證送出 `[D83D down, DE00 down, D83D up, DE00 up]`；`encode_units` 是 `inject.rs` 裡給 `SendInputInjector` 用的純函式。
- [ ] **Step 2:** 跑測試，預期 FAIL。
- [ ] **Step 3:** 照 `paste.py` 實作。

  `_type_chars` 的逐字流程對應 `keyboard.write(char, delay, restore_state_after=False)`：
  1. 釋放 `pressed()` 的所有 code（排序後）。
  2. 字元是 `\n` 或 `\b` 時，送該鍵的 press+release；其他字元呼叫 `unicode`。
  3. `delay > 0` 時 sleep。

  其他細節：
  - 前景守衛與 log 文字照 Python。
  - `TYPING` 鎖在整段鍵入期間持有，譯文與貼上兩條路互斥，後到的那條等待。
  - `paste_clipboard` 另有自己的 `PASTE_IN_PROGRESS` 鎖，用 `try_lock`，拿不到就記 `[composer] paste ignored: previous paste still typing`。
  - 等待放開時，判斷 `ctrl` 群組與 `v` 的 code 是否在 `pressed()` 裡（用 `key_codes_or_empty`）。
  - `get_text` 失敗的 log 沿用 `[composer] clipboard open failed: ...` 與 `[composer] clipboard has no text: ...`，由 `paste_clipboard` 記。
  - `get_text` 與既有的 `set_text` 共用開關剪貼簿的流程，不重複寫。
- [ ] **Step 4:** 跑測試，預期 PASS。
- [ ] **Step 5:** 跑提交前檢查後 commit：`feat(rust): add composer typing and clipboard paste`。

### Task 5: 熱鍵與遊戲輸入框事件的分派

**Files:**
- Create: `rust/src/input/dispatch.rs`
- Modify: `rust/src/ui/events.rs`（新增事件）
- Test: 同檔 `#[cfg(test)]`

**Interfaces:**
- Consumes：Task 4 的 `foreground_exe`；`reader::process::is_game_process_path`；`ui::events::UiPoster`；`ui::geometry::Rect`。
- Produces：
  - `UiEvent` 新增以下變體：
    - `ShowInputBox { retarget: Option<isize> }`
    - `GameInputOpened { hwnd: isize, anchor: Option<Rect> }`
    - `GameInputClosed { hwnd: isize }`
    - `OutgoingDone { session: u64, result: Result<String, String> }`：`Err` 是已在地化的錯誤訊息。
    - `RegionHotkey { hwnd: isize }`：第 3 段換成真的框選。
    - `Quit`
  - `crate::input::dispatch`：
    - `pub struct Foreground { pub hwnd: fn() -> isize, pub exe: fn() -> Option<String> }`：`Foreground::real()` 接 Win32。
    - `pub fn on_hotkey(fg: &Foreground, box_open: &AtomicBool, ui: &UiPoster) -> bool`
    - `pub fn foreground_game_hwnd(fg: &Foreground, prefix: &str) -> Option<isize>`
    - `pub fn on_region_hotkey(fg: &Foreground, selecting: &AtomicBool, ui: &UiPoster) -> bool`：選取層開著時投遞 `RegionHotkey { hwnd: 0 }`，等同 Python 的 `toggle(0)`；第 2 段的 `selecting` 恆為 false。
    - `pub fn region_hotkey_to_register(cfg: &Config) -> Option<String>`
    - `pub fn should_intercept_paste(cfg: &Config, fg: &Foreground) -> bool`
    - `pub struct ChatInputTracker`：`opened`、`closed`、`is_open`，內部用 `Mutex<HashSet<isize>>`。
    - `pub trait InputBoxControl { fn set_anchor(&mut self, r: Rect); fn clear_anchor(&mut self); fn retarget(&mut self, hwnd: isize); fn show(&mut self); fn hide(&mut self); }`
    - `pub struct GameInputEvents`：`new(tracker: Arc<ChatInputTracker>, foreground: fn() -> isize)`，另有 `opened(&mut self, hwnd, anchor, auto_show: bool, b: &mut impl InputBoxControl)` 與 `closed(&mut self, hwnd, auto_show: bool, b: &mut impl InputBoxControl)`。

- [ ] **Step 1:** 寫測試，逐一移植 `tests/input/test_dispatch.py`。

  遊戲輸入框事件（假的 `InputBoxControl` 記錄呼叫順序）：
  - `open_in_the_foreground_window_anchors_and_shows`
  - `open_in_the_foreground_window_retargets_before_showing`
  - `open_in_a_background_window_is_tracked_but_not_shown`
  - `close_from_another_window_does_not_hide_the_box`
  - `auto_show_off_still_tracks_and_anchors`

  `on_hotkey`：
  - `on_hotkey_retargets_before_showing_when_game_is_foreground`
  - `on_hotkey_only_refocuses_when_the_box_itself_is_foreground`
  - `on_hotkey_ignored_when_box_closed_and_game_not_foreground`
  - `on_hotkey_does_not_log_ignored_when_box_open_and_foreground_is_not_the_game`
  - `hotkey_opens_input_box_when_game_is_foreground`
  - `hotkey_ignored_when_other_window_is_foreground`
  - `hotkey_ignored_when_foreground_unknown`
  - `hotkey_refocuses_open_input_box_regardless_of_foreground`

  框選熱鍵：
  - `region_hotkey_to_register_returns_none_on_collision`
  - `region_hotkey_to_register_returns_the_hotkey_when_different`
  - `region_hotkey_cancels_when_the_selector_is_already_open`
  - `region_hotkey_starts_a_selection_when_game_is_foreground`
  - `region_hotkey_ignored_when_other_window_is_foreground`

  貼上：
  - `paste_intercepted_only_when_enabled_and_game_is_foreground`
  - `paste_single_line_follows_the_foreground_window`：測 `ChatInputTracker` 依 hwnd 各自記。

  `test_paste_hotkey_types_single_line_only_while_the_game_chat_box_is_open` 在 Task 9 以 `on_paste_hotkey` 的接線測試涵蓋。
- [ ] **Step 2:** 跑測試，預期 FAIL。
- [ ] **Step 3:** 照 `dispatch.py` 實作，log 文字照抄：
  - `[app] hotkey ignored: foreground is not the game window (exe={exe!r})`：`None` 印 `None`。
  - `[{prefix}] hotkey ignored: ...`
  - `[app] region_hotkey ... collides with hotkey; ...`
  - `[app] game chat opened in a background client (hwnd=0x..); ignoring`
- [ ] **Step 4:** 跑測試，預期 PASS。
- [ ] **Step 5:** 跑提交前檢查後 commit：`feat(rust): add hotkey and game chat input dispatch`。

### Task 6: reader 回報遊戲輸入框開關、上下文改為可共用

**Files:**
- Modify: `rust/src/reader/run_loop.rs`（`poll_input`、`force_input_closed`）、`rust/src/app_main.rs`（spawn 時把 `ClientContexts` 存進共用表）
- Test: `rust/tests/reader_loop.rs`

**Interfaces:**
- Consumes：Task 5 的 `UiEvent::GameInputOpened`、`UiEvent::GameInputClosed`。
- Produces：
  - `pub type ContextMap = Arc<Mutex<HashMap<isize, Arc<ClientContexts>>>>;`，放在 `crate::translation::context`。
  - `Running` 新增 `pub contexts: ContextMap`。
  - spawn 時插入或覆寫 `contexts[hwnd]`，與 Python `contexts[hwnd] = ClientContexts()` 相同。

- [ ] **Step 1:** 在 `reader_loop.rs` 加測試：
  - `input_open_and_close_are_posted_with_the_anchor`：開時投遞 `GameInputOpened { hwnd, anchor: Some(rect) }`；讀不到矩形時 anchor 為 `None`；關時投遞 `GameInputClosed`。
  - `game_window_gone_posts_input_closed_when_it_was_open`
  - `input_events_are_edge_triggered`：狀態沒翻轉就不投遞。
- [ ] **Step 2:** 跑測試，預期 FAIL。
- [ ] **Step 3:** 在既有的 log 旁邊投遞事件，並刪掉「開關事件在第 2 段接上 UI」的註解；`app_main.rs` 的 spawn 改用 `ContextMap`。
- [ ] **Step 4:** 跑測試，預期 PASS。
- [ ] **Step 5:** 跑提交前檢查後 commit：`feat(rust): report game chat input open and close to the UI`。

### Task 7: 翻譯輸入框（狀態、內容、幾何）

**Files:**
- Create: `rust/src/ui/input_box.rs`
- Modify: `rust/src/ui/mod.rs`
- Test: 同檔 `#[cfg(test)]`（純邏輯與 `egui_kittest`）

**Interfaces:**
- Consumes：
  - `ui::geometry::{anchored_position, Rect}`
  - `ui::richtext::show`
  - `ui::palette::{FG_ERROR, FG_UPDATE}`
  - `i18n::t`
  - Task 5 的 `InputBoxControl`
- Produces（`crate::ui::input_box`）：
  - 常數：
    - `BG = #1a1a24`、`FG = #f2f2f7`、`HINT_FG = #9a9aa8`、`ENTRY_BG = #262636`
    - `GAME_INPUT_MAX_CHARS = 80`
    - `DEFAULT_WIDTH = 460.0`、`MIN_WIDTH = 320.0`、`ANCHOR_GAP = 4.0`：皆為 DIP。
  - `pub struct OutgoingRequest { pub text: String, pub target: isize, pub session: u64, pub cancel: CancellationToken }`
  - `pub struct InputBox`：
    - `new(translate: Box<dyn Fn(OutgoingRequest)>, send: Box<dyn Fn(String, isize)>, open_flag: Arc<AtomicBool>, foreground: fn() -> isize, cursor: fn() -> (i32, i32))`
    - 實作 `InputBoxControl`。
    - `is_open() -> bool`、`target_hwnd() -> isize`
    - `close()`：主動關閉，丟掉草稿。
    - `finish(session: u64, result: Result<String, String>)`
    - `content(ui: &mut egui::Ui) -> Option<BoxAction>`：只畫內容，給 kittest 測。
    - `pub enum BoxAction { Submit, Close }`
    - 給 Task 8 宿主用的方法：
      - `take_focus_request(&mut self) -> bool`：`show()` 之後回一次 true。
      - `take_restore_foreground(&mut self) -> Option<isize>`：關閉後回一次 target。
      - `placement_anchor(&self) -> (Option<Rect>, (i32, i32))`：`show()` 當下的錨點與游標位置。
  - `pub fn open_rect(anchor: Option<Rect>, cursor: (i32, i32), content_h: i32, scale: f32, area: Rect) -> (Rect, bool)`：回傳外框（實體像素）與「是否在錨點上方」。
  - `pub fn refit(current: Rect, content_h: i32, above: bool) -> Rect`：在上方時固定底邊往上長，否則固定頂邊。

- [ ] **Step 1:** 寫測試。

  狀態與流程，移植 `tests/ui/test_input_box.py` 的 Tk 無關部分（`translate`／`send` 用記錄呼叫的假函式）：
  - `stale_session_discarded_on_cancel`
  - `finish_over_limit_keeps_window_and_blocks_send`：字數以 `chars().count()` 計，與 Python `len` 相同；錯誤文字為 `input.too_long`。
  - `finish_within_limit_sends_and_closes`
  - `enter_on_empty_input_closes_window`：只有空白也算空。
  - `non_cancelled_translation_succeeds`
  - `stale_error_discarded_on_cancel`
  - `current_error_shown_on_error`
  - `error_message_urls_are_clickable`：用 kittest 查到 Hyperlink。
  - `is_open_tracks_window_lifecycle`：`open_flag` 同步。
  - `retarget_changes_target_hwnd_while_open`
  - `retarget_is_a_no_op_for_none_or_same_hwnd`：0 或相同 hwnd 都不動。
  - `hide_keeps_the_unsent_draft_for_the_next_show`
  - `manual_close_discards_the_draft`
  - `sent_text_is_not_restored`
  - `hide_while_translating_restores_an_editable_draft`
  - `closing_the_box_while_translating_cancels_the_request`：`cancel` token 被 cancel。
  - `the_failure_log_names_the_service`：此項移到 Task 9，因為 log 由接線端的 translate 閉包記。
  - `submit_collapses_whitespace`：`"  a \n b "` 送出 `"a b"`。
  - `show_records_the_foreground_as_target_and_logs_the_anchor`：log 為 `[input] box opened (target_hwnd=0x.., anchor=(x, y, w, h))`，沒有錨點時印 `None`。

  幾何，移植 Python 的幾何測試：
  - `anchored_position_sits_below_the_anchor`
  - `anchored_position_flips_above_when_no_room_below`
  - `anchored_position_clamps_into_the_work_area`
  - `show_uses_the_anchor_width`
  - `show_clamps_the_anchor_width_to_minimum`
  - `show_without_any_anchor_uses_the_default_width`
  - `show_without_anchor_sits_at_the_cursor`
  - `box_placed_above_the_anchor_grows_upward`
  - `growth_after_dragging_pins_the_dragged_bottom`：`refit` 依目前外框計算，所以拖過之後也成立。
  - `width_follows_the_anchor_in_physical_pixels_at_150_percent`（Review Focus 4）：`scale=1.5` 時，錨點寬 900 回 900，錨點寬 300 回 480，沒有錨點回 690。

  內容，用 kittest：
  - `enter_submits_and_escape_closes`
  - `entry_is_disabled_while_translating`
  - `ime_enter_commits_composition_without_submitting`（Review Focus 1）：
    - 送 `Event::Ime(ImeEvent::Enabled)` 與 `Preedit("ㄋㄧˇ")`，接著 Enter，斷言沒有 `Submit`。
    - 再送 `Commit("你")` 後按 Enter，斷言 `Submit` 且文字為 `你`。
    - 組字中按 Esc，不回 `Close`。

  `test_error_message_follows_language` 不必移植：文字每幀以 `t()` 重譯。Tk 專屬的測試寫在 commit body，例如 `visible_chrome`、`_pin_bottom` 迴圈、`not_resizable`。
- [ ] **Step 2:** 跑 `cargo test --manifest-path rust/Cargo.toml --lib ui::input_box`，預期 FAIL。
- [ ] **Step 3:** 照 `input_box.py` 實作。

  寬度：
  - 有錨點時取 `max(round(MIN_WIDTH*scale), anchor.w)`；沒有錨點時取 `round(DEFAULT_WIDTH*scale)`。
  - 沒有錨點時把游標當成 0×0 錨點，間距為 `round(ANCHOR_GAP*scale)`。

  Enter 送出流程：
  - 壓縮空白（`split_whitespace` 再以空格連接）。
  - 把輸入欄設為 disabled，狀態顯示 `input.translating`。
  - 建立新的 `CancellationToken`，呼叫 `translate(OutgoingRequest{..})`。

  `finish(session, ..)`：session 對不上就丟掉；`Err(msg)` 時恢復可編輯並以紅字顯示；成功時先檢查 80 字上限，接著 `close()` 再 `send(text, target)`。

  `close()` 與 `hide()` 共同的收尾：
  - session +1；
  - cancel 進行中的 token；
  - 設定 `open_flag=false`；
  - 讓 `take_restore_foreground()` 回傳 `target`，由 Task 8 的宿主把前景還給它。

  IME：若 egui 的 `TextEdit` 在組字中仍回報 Enter，就在 `content` 裡自行追蹤 `ImeEvent` 的 Preedit 狀態擋掉。
- [ ] **Step 4:** 跑測試，預期 PASS。
- [ ] **Step 5:** 跑提交前檢查後 commit：`feat(rust): add the outgoing translation input box`。

### Task 8: 在 App 裡托管輸入框視窗

**Files:**
- Modify: `rust/src/ui/app.rs`、`rust/src/ui/win32.rs`、`rust/src/ui/overlay/bubble.rs`
- Test: `rust/src/ui/app.rs` 的 `#[cfg(test)]`、`rust/tests/ui_probe.rs`

**Interfaces:**
- Consumes：Task 7 的 `InputBox`；Task 5 的 `GameInputEvents`、`UiEvent` 新變體。
- Produces：
  - `pub fn pick_new_window(before: &[isize], after: &[isize], known: &[isize], by_title: impl FnOnce() -> isize) -> isize`：從 `bubble.rs` 的 `pick_bubble_window` 搬到 `ui::win32` 並改名；`known` 取代原本單一的 `overlay`。
  - `win32::find_thread_window(title: &str, except: &[isize])`：參數改成清單。
  - `App::with_input(self, input_box: InputBox, events: GameInputEvents, settings: Arc<RwLock<Config>>) -> Self`：`settings` 用來讀 `auto_show_input`。

- [ ] **Step 1:** 寫測試。
  - `pick_new_window_skips_known_windows`：由泡泡原本的測試改寫，涵蓋「同標題的輸入框不會被當成泡泡」。
  - App 的事件處理：
    - `game_input_events_reach_the_input_box`：`GameInputOpened` 後 `ShowInputBox` 依序作用。
    - `outgoing_done_is_forwarded_with_its_session`
    - `quit_closes_the_root_viewport`：kittest 收到 `ViewportCommand::Close`。
    - `region_hotkey_logs_until_phase_3`：記 `[ui] region hotkey pressed (hwnd=0x..); region capture arrives in phase 3`。
  - `hotkey_opens_the_input_box_while_minimized`（Review Focus 5）：minimized 時收到 `ShowInputBox`，輸入框開啟，`minimized` 仍為 true。
  - 在 `rust/tests/ui_probe.rs` 新增 ignored 的 probe 檢查：以 `InputBox` 開一個無錨點的框，用 WGC 量四邊沒有 Win11 灰邊，背景色為 `BG`。這是螢幕外擷取，不搶前景（不呼叫 force_foreground）。
- [ ] **Step 2:** 跑測試，預期 FAIL。
- [ ] **Step 3:** 實作。

  `App::ui` 每幀在泡泡之後呼叫 `show_input_box(&ctx)`，用 `show_viewport_immediate(ViewportId::from_hash_of("input_box"))`。builder 設定：
  - `with_title(app_name())`、`with_decorations(false)`、`with_resizable(false)`、`with_always_on_top()`、`with_taskbar(false)`
  - 第一幀加 `with_visible(false)`

  輸入框內容的組成：
  - 頂端是 20 DIP 的拖曳條，顯示 app 名稱與 ✕。拖曳用 `ViewportCommand::StartDrag`，✕ 等同 `close()`。
  - 下方是 `InputBox::content`。
  - 這個 viewport 收到 `close_requested`（Alt+F4）時，同樣呼叫 `close()`。

  建立那一幀與之後的處理：
  - 建立那一幀以 `pick_new_window(before, after, &[overlay, bubble], || find_thread_window(&app_name(), &[overlay, bubble]))` 找到 HWND，然後依序：
    1. `remove_system_frame`
    2. 依量到的內容高度，以 `open_rect` 呼叫 `set_window_rect`
    3. 下一幀送 `Visible(true)`，接著 `force_foreground(hwnd)` 並讓 TextEdit 取得焦點
  - 之後每幀若內容高度改變，就用 `refit(window_rect(hwnd), ..)` 調整高度。
  - 泡泡的 `pick_bubble_window` 呼叫改成 `pick_new_window(.., &[overlay, input_box_hwnd], ..)`。

  事件處理：
  - `apply` 處理新事件：`GameInputOpened`／`Closed` 交給 `GameInputEvents`，`auto_show` 每次從 settings 讀；`ShowInputBox { retarget }` 時先 `retarget` 再 `show`；`OutgoingDone` 交給 `finish`。
  - `Quit` 在 `logic` 裡處理，送 `ViewportCommand::Close` 給 ROOT。
  - 輸入框關閉後的 `take_restore_foreground()` 在下一幀呼叫 `force_foreground`。
- [ ] **Step 4:** 跑測試，並跑 probe：`cargo test --release --manifest-path rust/Cargo.toml --test ui_probe -- --ignored`，預期全部 PASS。
- [ ] **Step 5:** 跑提交前檢查後 commit：`feat(rust): host the input box viewport in the app`。

### Task 9: 主程式接線、Ctrl+C 關閉、啟動 log

**Files:**
- Modify: `rust/src/app_main.rs`、`rust/src/main.rs`、`rust/Cargo.toml`（windows-sys feature `Win32_System_Console`）
- Test: `rust/tests/app_main.rs`

**Interfaces:**
- Consumes：Task 1～8 的全部產出。
- Produces（`crate::app_main`）：
  - `pub struct InputWiring { pub tracker: Arc<ChatInputTracker>, pub box_open: Arc<AtomicBool> }`：`Running` 新增 `pub input: InputWiring`。
  - `pub fn install_input(settings: &Arc<RwLock<Config>>, ui: &UiPoster, dispatcher: &SharedDispatcher, wiring: &InputWiring, typing: Arc<TypingEnv>) -> (Registration, Option<Registration>)`：依序註冊 hotkey、region hotkey、paste 攔截器（與 Python 順序相同），實際生效的鍵寫回 settings（只改記憶體，不存檔，與 Python 相同）。
  - `pub fn outgoing_translate(translator: Arc<Translator>, contexts: ContextMap, rt: Handle, ui: UiPoster) -> Box<dyn Fn(OutgoingRequest)>`
  - `pub fn on_paste_hotkey(settings: &RwLock<Config>, tracker: &ChatInputTracker, typing: Arc<TypingEnv>, foreground: fn() -> isize) -> std::thread::JoinHandle<()>`
  - `pub fn install_console_quit(ui: UiPoster)`：`SetConsoleCtrlHandler`，只攔 `CTRL_C_EVENT`。

- [ ] **Step 1:** 寫測試。
  - `install_input_writes_the_effective_hotkeys_back`：無效的 `hotkey` 退回 `ctrl+space`；與 hotkey 相同的 `region_hotkey` 不註冊，回 `None`，cfg 保持原值。
  - `install_input_registers_paste_last`：Dispatcher 的 handler 順序依序為 hotkey、region、paste。
  - `outgoing_translate_uses_the_target_clients_current_tab_context`：用假 translator（`Translator::with_client` 配假 client）。上下文取 `contexts[target].current().snapshot()`，沒有該客戶端時為空。
  - `outgoing_translate_posts_the_localized_error_and_logs_the_service`：取代 `the_failure_log_names_the_service`。log 為 `[input] outgoing translation failed (chars=N, provider=.., model=..): {class}: {err}`，事件為 `OutgoingDone { result: Err(t("input.failed", error)) }`。
  - `outgoing_translate_cancel_logs_and_posts_nothing`：log 為 `[input] outgoing translation cancelled (chars=N)`。
  - `paste_hotkey_types_single_line_only_while_the_game_chat_box_is_open`：移植 Python 同名測試，用假的 `TypingEnv`。
- [ ] **Step 2:** 跑測試，預期 FAIL。
- [ ] **Step 3:** 實作。

  `build_app` 的接線：
  - 建立 Dispatcher、`InputWiring`、`ContextMap`。
  - 呼叫 `install_input`，並 `hook::install`。`mask` 閉包用 `SendInputInjector::mask`。`TypingEnv::real` 的 `pressed` 讀 Dispatcher 的 pressed，複製出來後立刻放鎖。

  `main.rs` 的調整：
  - 建立 `InputBox` 時，`translate` 用 `outgoing_translate`；`send` 起執行緒跑 `type_into_window(env, hwnd, text, type_delay)`。
  - **裁定：** Python 在 UI 執行緒同步鍵入，Rust 改在背景執行緒，UI 不會凍結；`TYPING` 鎖保證不與貼上交錯。`type_delay` 在送出當下從 settings 讀。
  - 用 `with_input` 接上輸入框；`set_region_hotkey` 改用生效值。
  - `[app] running; ...` 那行 log 移到註冊之後，印生效值。
  - 在 UI 起來前呼叫 `install_console_quit(poster.clone())`：Ctrl+C 投遞 `UiEvent::Quit`，走與 ✕ 相同的關閉流程（followups 第 2 段第 1 項）。handler 回 TRUE 吃掉預設終止。

  `shutdown()` 依 Task 3 的順序呼叫 `keyboard.uninstall()`。
- [ ] **Step 4:** 跑測試，預期 PASS。另外跑 `cargo build --release --manifest-path rust/Cargo.toml`；確認沒有執行中的 `_test\Wizard101ChatTranslator.exe` 後，把 exe 複製到 `rust/target/release/_test/`。
- [ ] **Step 5:** 跑提交前檢查後 commit：`feat(rust): wire hotkeys, input box and paste into the app`。

---

## 實機驗收清單（第 2 段完成後交給使用者）

依 spec §E 與第 1 段的遺留項目：

1. **斷網橫幅**（第 1 段遺留）：斷網後等新訊息進來，overlay 出現離線橫幅；恢復網路後自動補翻。
2. **快捷鍵呼出**：
   - 遊戲在前景時按 Ctrl+Space，輸入框出現；在其他程式裡按，什麼都不發生、按鍵照常送給那個程式。
   - 按住 Ctrl+Space 不會連開，也不會切換輸入法；放開 Ctrl 後不會跳出輸入法切換或選單。
3. **位置與自動顯示**：
   - 遊戲聊天框開著時，輸入框貼在它正下方、同寬；沒開時貼在游標處。下方放不下就翻到上方，錯誤訊息換行時往上長。
   - 打開遊戲聊天框時，輸入框自動出現；關掉時自動收起，並保留打到一半的字。
   - 雙開時只跟前景客戶端。
4. **發話**：
   - 中文（含注音選字的 Enter）、日文、emoji 都能輸入。
   - Enter 送出翻譯，譯文逐字打進遊戲輸入框，不會自動送出。
   - 超過 80 字顯示錯誤並保留原文。
   - Esc、空白 Enter、✕ 都會關閉並回到遊戲；翻譯中關閉不會晚到打字。
   - 打字途中切走視窗，打字會停下。
   - 注音組字到一半時切到別的視窗再切回來，Enter 與 Esc 仍然有效。
   - 輸入框沒有工作列按鈕；關掉輸入框後，前景回到遊戲，不會停在 overlay。
   - 若有兩個縮放比例不同的螢幕：在每個螢幕上開輸入框，寬度與位置都正確。
5. **Ctrl+V**：
   - 遊戲聊天框開著時，多行剪貼簿變成一行。
   - 在遊戲其他輸入欄位，換行照打 Enter。
   - 其他程式的 Ctrl+V 不受影響；連按兩次不會交錯。
6. **框選快捷鍵**：遊戲在前景時按 Ctrl+Shift+Space，app.log 出現 region hotkey 的 log，按鍵不會傳進遊戲。
7. **關閉**：
   - 輸入框開著時從 ✕ 關閉程式，遊戲不當，鍵盤正常。
   - 開發版從主控台按 Ctrl+C，log 出現完整的 shutdown，下次啟動不需要修復 hook。
