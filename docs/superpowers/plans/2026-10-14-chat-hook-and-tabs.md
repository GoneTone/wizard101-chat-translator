# 收訊改用 AddLine hook＋翻譯視窗分頁同步 實作計畫

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 以 hook `ChatBox::AddLine` 的事件流取代 chatLog 全文差分，並讓翻譯視窗依遊戲分頁分開顯示、自動跟著前景客戶端的目前分頁。

**Architecture:** 新 hook 把每次 `AddLine` 呼叫（序號、分頁、`this`、整行 markup）複製進遊戲程序內的環形緩衝，reader 依序號讀取，不做任何內容比對。reader 另外每輪讀 `ChatBox+0x7a0`（目前分頁）與各分頁按鈕文字，連同前景狀態回報給 overlay；overlay 依（客戶端, 分頁）分開保存訊息，只畫目前顯示的那一份。

**Tech Stack:** Python 3.11+、wizwalker（LaurenzNotHere fork）、tkinter、pytest、unicorn（新 dev 相依，模擬執行 hook 機器碼）。

**Spec:** `docs/superpowers/specs/2026-10-14-chat-hook-and-tabs-design.md`

## Global Constraints

- 分支 `feat/chat-hook-tabs`，直接在原目錄工作，不建 worktree；**不要 push**。
- 每個 task 結束前：`uv run ruff check src tests tools` 零錯誤、`uv run pytest` 全綠（全套每 task 只跑一次，其餘用 `-k`／指定檔案）。
- commit message 英文、conventional commits，結尾附：
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` 與 `Claude-Session: https://claude.ai/code/session_01Jd7ExtVDMHEKLpugpvrmxa`
- log 訊息一律英文、前綴 `[reader]`／`[ui]`，帶 slot、PID 等 context；不可 `print`。
- 註解預設一行、只寫 WHY；模組與公開函式一行 docstring；中文註解用全形標點。
- 不可寫死語言（`zh`／`en`），UI 文字走 i18n；分頁名稱是遊戲文字，不經 i18n。
- 讀遊戲的 `std::wstring` 一律用本案的 `read_wstring`（依 capacity 判斷 SSO），不用 wizwalker 的 `read_wide_string`／`maybe_text`。
- 動 `uv.lock` 的指令在 Bash 用 `/c/Users/p2902/AppData/Local/Microsoft/WinGet/Links/uv tool run --from "uv==0.12.15" uv ...`，事後 `git diff --ignore-cr-at-eol --numstat uv.lock` 確認只多出新套件。
- 測試不可干擾執行中的遊戲：會搶焦點或讀前景視窗的動作抽成可 patch 的函式。
- 原型參考：`.superpowers/sdd/chat-hook-prototypes/`（`probe_addline.py`、`test_stub.py`、`probe_tabs.py`）。原型的 `and eax, imm8` 只適用 64 槽，本案 256 槽必須改 `and eax, imm32`。

## Review Focus

1. **斷線重連後序號歸零**：遊戲關掉再開、reader 重新掛上，新 hook 的累計數從 0 開始；reader 若沿用舊的 `last_seq` 會永遠讀不到新訊息。→ Task 4 `test_teardown_resets_the_ring_cursor`。
2. **訊息抵達時顯示的是別的客戶端／分頁，譯文之後才回填**：切過去時必須看到譯文，不是佔位字。→ Task 5 `test_translation_fills_a_message_in_a_hidden_view`。
3. **重新登入後 ChatBox 位址改變**：不可把之後所有訊息當成其他聊天框丟掉。→ Task 4 `test_unknown_chatbox_triggers_one_refind_before_dropping`。
4. **訊息屬於目前沒顯示在分頁列的分頁**（例如好友分頁隱藏）：保存下來，分頁出現時看得到，不可遺失。→ Task 5 `test_messages_for_a_tab_not_in_the_bar_are_kept`。
5. **環形緩衝溢位**：一輪超過 256 行時記 log 遺失筆數並繼續讀剩下的，不可卡住或重讀。→ Task 1 `test_overflow_reports_lost_and_resumes_at_oldest_valid_slot`。

---

### Task 1：AddLine hook 與環形緩衝讀取

**Files:**
- Create: `src/reader/chat_hook.py`
- Create: `tests/test_chat_hook.py`
- Modify: `pyproject.toml`（dev 群組加 `unicorn`）、`uv.lock`

**Interfaces:**
- Produces（`src/reader/chat_hook.py`）：
  - 常數 `SLOTS = 256`、`SLOT_SIZE = 0x800`、`HEADER = 0x40`、`DATA_OFF = 0x30`、`MAX_CHARS = (SLOT_SIZE - DATA_OFF) // 2`（=1000）、`PATTERN: bytes`（spec 中的 pattern，原型 `AddLineHook.pattern` 逐字）。
  - `class HookRecord(NamedTuple)`: `seq: int, caller: int, this: int, tab: int, length: int, text: str`（`length`＝原始字元數，`len(text) < length` 代表截斷）。
  - `class RingRead(NamedTuple)`: `records: list[HookRecord], last_seq: int, lost: int`。
  - `async def read_ring(read_bytes, ring: int, last_seq: int) -> RingRead`：`read_bytes` 是 `async (addr: int, size: int) -> bytes`。
  - `def group_broadcasts(records: list[HookRecord]) -> list[tuple[tuple[int, ...], HookRecord]]`：回傳（分頁組, 代表記錄）。
  - `class AddLineHook(SimpleHook)`：`pattern = PATTERN`、`instruction_length = 5`、`exports = [("ring", HEADER + SLOTS * SLOT_SIZE)]`；`get_hook_address` 配 256 bytes；`unhook` 只還原跳躍點、**不釋放 ring**（遊戲執行緒可能還在 stub 內寫入）。
  - `def stub_bytes(ring: int) -> bytes`：stub 機器碼（不含 SimpleHook 附加的回跳），`AddLineHook.bytecode_generator` 呼叫它。

- [ ] **Step 1：加 dev 相依**

Run: `/c/Users/p2902/AppData/Local/Microsoft/WinGet/Links/uv tool run --from "uv==0.12.15" uv add --dev unicorn`
Expected：`pyproject.toml` dev 群組多 `unicorn>=…`；`git diff --ignore-cr-at-eol --numstat uv.lock` 只有 unicorn 相關行。

- [ ] **Step 2：寫失敗測試 `tests/test_chat_hook.py`**

以原型 `test_stub.py` 為底（unicorn 映射 RING／CODE／STACK／HEAP／STR，`emu_start(CODE, CODE + len(stub))`）：

```python
def test_stub_copies_sso_and_heap_strings_and_restores_registers(): ...
    # "hi"（SSO，cap 7）與 26 字堆積字串各跑一次；
    # rax rcx rdx rsi rdi r8 r9 r10 rbx rsp 值不變、DF 仍為 1、[rsp+0x20]==原 rbx、
    # 標頭累計數 == 預期序號、read_ring 讀回 (tab, this, caller=0x1407443B1, text)
def test_stub_truncates_long_lines_and_keeps_original_length():
    # 1500 字 → record.length == 1500、len(record.text) == MAX_CHARS
def test_stub_wraps_past_the_slot_count():
    # 起始累計數 SLOTS（序號 SLOTS+1）→ 寫入槽 0；且所有寫入位址落在 [RING, RING+HEADER+SLOTS*SLOT_SIZE)
    # （用 unicorn UC_HOOK_MEM_WRITE 收集寫入位址斷言）
def test_read_ring_returns_new_records_in_order(): ...      # 假記憶體 3 筆，last_seq 0→3
def test_read_ring_stops_at_a_slot_being_written():          # 第 2 槽序號 0 → 只回 1 筆、last_seq==1
def test_read_ring_counts_an_overwritten_slot_as_lost():     # 槽內序號 != 預期 → lost==1，跳過續讀
def test_overflow_reports_lost_and_resumes_at_oldest_valid_slot():
    # 累計數 = last_seq + SLOTS + 5 → lost==5，從 last_seq+6 起讀滿 SLOTS 筆
def test_read_ring_rereads_seq_to_detect_a_torn_read():      # 讀完內容後序號變了 → 視為 lost
def test_group_broadcasts_collapses_tabs_0_to_3_plus_channels():
    # 連號、同字、tab 0,1,2,3,5 → [((0,1,2,3,5), rec0)]
def test_group_broadcasts_keeps_ordinary_lines_apart():
    # 同字但 tab 0,1（不足 0..3）→ 兩組各自 (0,) (1,)；不連號的 0..3 也不合併
```

- [ ] **Step 3：跑測試確認失敗**

Run: `uv run pytest tests/test_chat_hook.py -n 0 -q`
Expected：FAIL（`ModuleNotFoundError: src.reader.chat_hook`）

- [ ] **Step 4：實作 `src/reader/chat_hook.py`**

- stub 以原型 `bytecode_generator` 為準，兩處改動：槽位取模改 `b"\x25" + struct.pack("<I", SLOTS - 1)`（`and eax, imm32`）；`SLOTS`／`MAX_CHARS`／`HEADER` 用本模組常數。
- `read_ring`：讀標頭累計數 `count`；`count - last_seq > SLOTS` 時 `lost += count - last_seq - SLOTS` 並把 `last_seq` 推到 `count - SLOTS`；逐序號讀槽：序號 0 → 停（下一輪再讀）；不等於預期 → `lost += 1` 跳過；讀完內容再讀一次序號，不同 → `lost += 1`。
- `group_broadcasts`：合併條件是連號、文字相同、分頁序列以 `0,1,2,3` 開頭且之後嚴格遞增（≥5）。分組跨輪被切開的情況不處理，加一行 `shortcut:` 註解（廣播極少且寫入在微秒內完成，被輪詢切開時會變成各分頁各一則；真的發生再改成保留尾段）。

- [ ] **Step 5：跑測試確認通過**

Run: `uv run pytest tests/test_chat_hook.py -n 0 -q`
Expected：PASS

- [ ] **Step 6：lint、全套測試、commit**

```bash
uv run ruff check src tests tools && uv run pytest
git add pyproject.toml uv.lock src/reader/chat_hook.py tests/test_chat_hook.py
git commit -m "feat(reader): add AddLine hook stub and ring buffer reader"
```

---

### Task 2：分頁狀態讀取的純函式

**Files:**
- Create: `src/reader/chat_tabs.py`
- Create: `tests/test_chat_tabs.py`

**Interfaces:**
- Produces（`src/reader/chat_tabs.py`）：
  - `CHATBOX_WINDOW = "WizardChatBox"`、`CUR_TAB_OFFSET = 0x7A0`、`LABEL_OFFSET = 0x2E0`
  - `TAB_BUTTONS: tuple[tuple[str, int], ...]`，依遊戲由左到右順序：`("MainFilterButton", 0), ("GroupFilterButton", 1), ("HouseFilterButton", 2), ("FriendFilterButton", 3), ("Channel1FilterButton", 5), ("Channel2FilterButton", 6), ("Channel3FilterButton", 7), ("Channel4FilterButton", 8), ("TeamUpFilterButton", 4)`（3、4、5–8 是靜態推論，註解標明待實機確認）
  - `class TabState(NamedTuple)`: `current: int, tabs: tuple[tuple[int, str], ...]`（`tabs`＝目前可見分頁的（編號, 按鈕文字），依 `TAB_BUTTONS` 順序）
  - `def visible_label(raw: str) -> str | None`：空字串或以 `<string;` 開頭（隱藏分頁）回 None，其餘原樣回傳（含未讀 `(n)`，使用者要照抄遊戲）
  - `async def read_wstring(read_bytes, addr: int) -> str`：MSVC `std::wstring`，size 在 `+0x10`、capacity 在 `+0x18`，capacity ≥ 8 時 `+0` 是指標，否則內嵌；size 0 或 > 4096 回 `""`

- [ ] **Step 1：寫失敗測試**

```python
def test_read_wstring_inline_short_string():       # "主要的"（cap 7）→ "主要的"
def test_read_wstring_five_chars_inline():         # "队伍(2)"（5 字、cap 7）→ "队伍(2)"（wizwalker 會讀壞的那種）
def test_read_wstring_heap_string():               # "Party 1 (3)"（cap 15、指標）→ 原文
def test_read_wstring_empty_and_absurd_sizes():    # size 0 → ""；size 10**7 → ""
def test_visible_label_hides_unresolved_tokens():  # "<string;GUI2_00000367>" → None；"" → None
def test_visible_label_keeps_unread_suffix():      # "队伍(1)" → "队伍(1)"
def test_tab_buttons_cover_the_verified_mapping(): # Main→0、Group→1、House→2
```

- [ ] **Step 2：跑測試確認失敗** — `uv run pytest tests/test_chat_tabs.py -q`，Expected：FAIL（模組不存在）
- [ ] **Step 3：實作 `src/reader/chat_tabs.py`**
- [ ] **Step 4：跑測試確認通過** — 同上指令，Expected：PASS
- [ ] **Step 5：lint、全套測試、commit**

```bash
git add src/reader/chat_tabs.py tests/test_chat_tabs.py
git commit -m "feat(reader): add chat tab state helpers"
```

---

### Task 3：單行 markup 解析

**Files:**
- Modify: `src/reader/markup.py`
- Modify: `tests/test_chatlog_parse.py`、`tests/test_translator.py:1197-1202`

**Interfaces:**
- Produces：`def parse_line(raw: str) -> ChatLine | None`（分類規則與現行 `lines_from_chatlog` 對單行的處理完全相同，含 `_warn_unknown_icon`）。`lines_from_chatlog` 暫時保留、改為逐行呼叫 `parse_line`（`mem_reader.py` 還在用，Task 4 才刪）。

- [ ] **Step 1：改寫測試**：`test_chatlog_parse.py` 中單行輸入的 `lines_from_chatlog(x)` 改成 `parse_line(x)`（斷言改為單一 `ChatLine` 或 None）；多行輸入的測試（`_log(...)` 串多行）改成 `[l for r in raw.split("\n") if (l := parse_line(r))]`，斷言值不變。`test_translator.py` 那段改用 `parse_line(raw).text`。
- [ ] **Step 2：跑測試確認失敗** — `uv run pytest tests/test_chatlog_parse.py -q`，Expected：FAIL（`ImportError: parse_line`）
- [ ] **Step 3：實作**：把 `lines_from_chatlog` 迴圈體抽成 `parse_line`。
- [ ] **Step 4：跑測試確認通過** — `uv run pytest tests/test_chatlog_parse.py tests/test_translator.py -q`，Expected：PASS
- [ ] **Step 5：lint、全套測試、commit**

```bash
git add src/reader/markup.py tests/test_chatlog_parse.py tests/test_translator.py
git commit -m "refactor(reader): parse chat markup one line at a time"
```

---

### Task 4：reader 重寫——hook 事件流取代差分

**Files:**
- Modify（重寫）：`src/reader/mem_reader.py`、`src/reader/message_log.py`
- Modify: `src/reader/markup.py`、`tests/test_chatlog_parse.py`（刪舊函式）
- Delete: `src/reader/diff.py`、`tests/test_chatlog_diff.py`、`tests/test_system_track.py`
- Modify: `tests/test_reader_connect.py`、`tests/test_message_log.py`
- Create: `tests/test_reader_events.py`

**Interfaces:**
- Consumes：Task 1 `read_ring`、`group_broadcasts`、`AddLineHook`、`HookRecord`；Task 2 `TabState`、`TAB_BUTTONS`、`CUR_TAB_OFFSET`、`LABEL_OFFSET`、`CHATBOX_WINDOW`、`visible_label`、`read_wstring`；Task 3 `parse_line`。
- Produces（`src/reader/mem_reader.py`）：
  - `class ChatEvent(NamedTuple)`: `tabs: tuple[int, ...], line: ChatLine`
  - `WizChatReader.read_new() -> list[ChatEvent]`（emit_system False 時不含系統行，但序號照推）
  - `WizChatReader.tab_state() -> TabState | None`（未連上、找不到 ChatBox 或讀取失敗回 None）
  - 保留：`anchored`、`emit_system`、`input_open()`、`input_box_screen_rect()`、`close()`、`GameNotRunning`／`GameAccessDenied`／`GameVersionMismatch`、`is_version_mismatch`
  - 測試接縫（方法，測試 monkeypatch）：`_install_chat_hook() -> int`（回傳 ring 位址）、`_read_records() -> RingRead`、`_find_chatbox() -> int | None`
- Produces（`src/reader/message_log.py`）：`MessageLog(stream, slot=None)`，方法 `record(seq: int, tab: int, raw: str, verdict: str) -> None`、`note(text: str) -> None`；保留 `is_debug_line`（`record` 對除錯行不寫）。verdict 取值：`player`、`system`、`system-off`、`dropped`、`broadcast`、`broadcast-dup`、`foreign`。

- [ ] **Step 1：寫失敗測試 `tests/test_reader_events.py`**（以 monkeypatch 接縫，不碰 wizwalker；`_connected=True` 直接造出已連上的 reader）

```python
def test_records_become_events_in_order():                 # 2 筆玩家行 → 2 個 ChatEvent，tabs=(0,)、(1,)
def test_identical_consecutive_lines_both_emit():          # 兩筆同字掉寶（系統行，emit_system=True）→ 2 個事件
def test_system_lines_are_skipped_when_emit_system_off():  # 序號仍推進：下一輪不重吐
def test_broadcast_becomes_one_event_for_all_tabs():       # tab 0,1,2,3 同字 → 1 個事件 tabs=(0,1,2,3)
def test_unparseable_lines_are_dropped_and_logged():       # 除錯行 → 無事件，messages.log 不寫；未知圖示 → verdict dropped
def test_unknown_chatbox_triggers_one_refind_before_dropping():
    # 記錄 this=B、目前 chatbox=A：_find_chatbox 回 B → 照收並更新；回 A → 丟棄、log 一次，
    # 同一個 B 第二次出現不再重找
def test_lost_records_are_logged_with_the_count():          # RingRead(lost=3) → app.log 有 "lost 3"
def test_teardown_resets_the_ring_cursor():
    # 讀到 seq 5 後 _teardown()；重連後 ring 累計數 1 → 讀得到 seq 1
def test_tab_state_reads_current_tab_and_visible_labels():
    # 假記憶體：+0x7a0=1、Main "主要的"、Group "队伍(2)"、其餘 "<string;...>" → TabState(1, ((0,"主要的"),(1,"队伍(2)")))
def test_tab_state_is_none_without_a_chatbox():
```

`tests/test_reader_connect.py` 增改：
```python
def test_chat_hook_pattern_failure_raises_version_mismatch():  # _install_chat_hook 丟 PatternFailed
def test_hook_state_saves_hook_jumps_before_autobot():
    # 存檔 ops 順序：各 hook 的 (jump_address, 原碼) 在前，autobot 在最後；含 AddLineHook
```
刪除只測差分的既有測試；`test_message_log.py` 改測 `record`／`note` 的輸出格式（含 `[slot=N]` 前綴、除錯行不寫）。

- [ ] **Step 2：跑測試確認失敗** — `uv run pytest tests/test_reader_events.py tests/test_reader_connect.py tests/test_message_log.py -n 0 -q`，Expected：FAIL
- [ ] **Step 3：刪除 `src/reader/diff.py`、`tests/test_chatlog_diff.py`、`tests/test_system_track.py`；`markup.py` 刪 `lines_from_chatlog`、`lines_from_nodes`、`_mirrors`、`node_sizes` 與模組 docstring 的多節點描述，`test_chatlog_parse.py` 刪掉 `lines_from_nodes` 的測試與剩下的 `lines_from_chatlog` 引用**
- [ ] **Step 4：重寫 `mem_reader.py`**
  - 刪掉所有差分常數、`Track`、`_Outcome`、`_diff_*`／`_align_player`／`_filter_*`／`_drop_resurfaced`／`_guard_burst`／`_track_*`／`_read_snapshot`／`_grab_texts`，以及模組 docstring 的差分敘述。
  - `_connect`：root window hook（`wait_for_ready=False`）→ `_install_chat_hook()`（`AddLineHook(handler).hook()` 後登記到 `handler._active_hooks[AddLineHook]`，回傳 `hook.ring`）→ `_save_hook_state` → `_wait_root_window_ready`。成功 log 帶 ring 位址與跳躍點換算的靜態位址（`jump - module_base + 0x140000000`）。
  - `_save_hook_state`：ops 先放各 hook 的跳躍點，最後才放 autobot（spec「hook_state」段）。
  - `read_new`：`_read_records()` → 記 lost → `group_broadcasts` → 每組：`this` 檢查（不符先 `_find_chatbox()` 重找一次，仍不符記入 `_foreign` 集合、log 一次、verdict `foreign`）→ `parse_line` → `system` 且 `not emit_system` 記 `system-off` → 產生 `ChatEvent`；每筆記錄都寫 `messages.log`。
  - `tab_state`：ChatBox 位址快取（`_find_chatbox`：root window 找 `CHATBOX_WINDOW`），按鈕節點快取（失效清空重找），讀 `CUR_TAB_OFFSET` 的 int32 與各按鈕 `LABEL_OFFSET` 的 wstring；有變化才 log。
  - `_teardown`：除現有清理外，`_last_seq = 0`、ChatBox／按鈕快取與 `_foreign` 清空。
- [ ] **Step 5：重寫 `message_log.py`**（刪 `new_raw_lines`、`snapshot`、`decision`）
- [ ] **Step 6：跑測試確認通過** — 同 Step 2 指令，Expected：PASS
- [ ] **Step 7：`loop.py` 的暫時相容**：`reader_loop` 改為迭代 `ChatEvent`（`line = event.line`），其餘留給 Task 6；`tests/test_reader_loop.py` 的 `FakeReader.read_new` 改回傳 `ChatEvent((0,), line)`，並加 `tab_state()` 回 None。
- [ ] **Step 8：lint、全套測試**
- [ ] **Step 9：commit**

```bash
git add -A src/reader tests
git commit -m "feat(reader): read chat through the AddLine hook instead of diffing chatLog"
```

---

### Task 5：overlay 分頁列與依（客戶端, 分頁）分開的訊息清單

**Files:**
- Create: `src/ui/tab_bar.py`
- Modify: `src/ui/message_list.py`、`src/ui/overlay.py`、`src/ui/palette.py`、`src/reader/supervisor.py`、`src/main.py:642`
- Modify: `tests/test_overlay.py`、`tests/test_supervisor.py`、`tests/test_main.py:587-592`

**Interfaces:**
- Consumes：Task 2 `TabState`（只當型別，`TYPE_CHECKING` import）。
- Produces：
  - `src/ui/tab_bar.py`：`class TabBar`：`__init__(parent: tk.Misc)`；`set(tabs: tuple[tuple[int, str], ...], current: int | None) -> None`（tabs 為空時 `pack_forget` 隱藏）；`labels() -> list[tuple[str, bool]]`（測試用：（文字, 是否目前））。不可點擊、不經 i18n。顏色新增到 `palette.py`：`TAB_BG`、`TAB_FG`、`TAB_FG_ACTIVE`、`TAB_ACCENT`（取值對齊 brainstorm 示意 A：`#181828`、`#8a8aa0`、`#f2f2f7`、`#6fa8dc`）。
  - `MessageList`：訊息依 `key: tuple[int | None, int]`（slot, tab）分開保存資料記錄，只有 `shown_key` 那份有畫面元件。
    - `add_message(key, original, translated, now=None, msg_id=None, pending=False, color=None)`
    - `update_message(msg_id, translated, failed=False)`：更新所有 key 中該 msg_id 的記錄，正在顯示的才改畫面
    - `show(key) -> None`：換顯示清單（銷毀舊列、依記錄重建、回到跟隨底部）
    - `shown_key` 屬性；`set_limits`／`prune` 套用到每個 key；`visible_messages()` 只回顯示中的那份
    - 刪除 `slot_marker`、`set_multi_client`、`_display_original`、`_Message.slot`
  - `OverlayWindow`：
    - `add_message(original, translated, now=None, msg_id=None, pending=False, color=None, slot=None, tabs=(0,))`：加到 `(slot, t)` for t in tabs；顯示的客戶端尚未決定時，第一個帶訊息或分頁狀態的 slot 成為顯示中客戶端；**未讀數只在縮小中且 `slot` 是顯示中客戶端時 +1，每次呼叫最多 +1**（廣播一則）
    - `set_tabs(slot: int, state: TabState | None) -> None`：記下該客戶端分頁狀態；是顯示中客戶端就更新分頁列並 `show((slot, state.current))`
    - `set_foreground_slot(slot: int) -> None`：顯示中客戶端改為 slot，分頁列與清單跟著換
    - `tab_labels() -> list[tuple[str, bool]]`（測試用）
    - 刪除 `set_multi_client`
  - `supervise(stop, spawn, enumerate_windows, board, interval=SCAN_INTERVAL)`：移除 `on_multi_client` 參數與相關邏輯；`main.py` 對應移除 `overlay.set_multi_client` 的傳入。

- [ ] **Step 1：寫失敗測試（`tests/test_overlay.py`，沿用 `root` fixture）**

```python
def test_tab_bar_shows_game_labels_and_highlights_current(root):
    # set_tabs(1, TabState(1, ((0,"主要的"),(1,"队伍"),(2,"房间(1)")))) → tab_labels() == [("主要的",False),("队伍",True),("房间(1)",False)]
def test_tab_bar_hidden_until_a_state_arrives(root):              # 新建 overlay：tab_labels() == []
def test_messages_are_kept_per_tab_and_follow_the_game_tab(root):
    # tab 0 與 tab 1 各加一則；current=0 只看到 tab 0 那則；set_tabs 換 current=1 後只看到 tab 1 那則
def test_messages_for_a_tab_not_in_the_bar_are_kept(root):
    # 分頁列只有 0、1；加一則 tabs=(3,)；之後 set_tabs 出現 3 且 current=3 → 看得到那則
def test_broadcast_lands_in_every_listed_tab(root):                # tabs=(0,1,2,3) → 每個分頁都看得到
def test_foreground_slot_switches_the_whole_view(root):
    # slot 1 與 slot 2 各有分頁狀態與訊息；set_foreground_slot(2) → 分頁列與訊息換成 slot 2 的
def test_translation_fills_a_message_in_a_hidden_view(root):
    # 佔位訊息加在 tab 1，顯示 tab 0 時 update_message → 切到 tab 1 看到譯文而非佔位字
def test_unread_counts_only_the_foreground_client(root):
    # 縮小後 slot 2（非顯示中）加 2 則、slot 1 加 1 則廣播 tabs=(0,1,2,3) → unread_count() == 1
def test_max_messages_applies_per_tab(root):                         # max 2：tab 0 加 3 則只留 2，tab 1 那則不受影響
def test_prune_drops_faded_messages_in_hidden_views_too(root):
```
既有測試：刪除 `slot_marker` 與多客戶端標記的四個測試；`test_minimize_counts_unread_and_expand_resets` 照舊（slot None 即顯示中客戶端）。`tests/test_supervisor.py` 刪 multi-client 三個測試、呼叫改新簽名；`tests/test_main.py:592` 斷言改為不再含 `set_multi_client`。

- [ ] **Step 2：跑測試確認失敗** — `uv run pytest tests/test_overlay.py tests/test_supervisor.py -q`，Expected：FAIL
- [ ] **Step 3：實作 `tab_bar.py`、palette 色值**
- [ ] **Step 4：改寫 `MessageList` 為 keyed store**（資料記錄與畫面元件分開；`_drop_row` 與 `Selection.forget` 成對的規則不變；`show` 時清掉選取）
- [ ] **Step 5：改 `OverlayWindow`**：分頁列 pack 在標題列之下、訊息區之上；新增 `set_tabs`、`set_foreground_slot`、`tab_labels`；`add_message` 依上方規則；刪 `set_multi_client`；顯示中的（客戶端, 分頁）改變時記 `[ui] view switched (slot=…, tab=…, messages=…)`
- [ ] **Step 6：改 `supervisor.py`、`main.py:642` 移除 multi-client 通知**（supervisor 模組 docstring 同步）
- [ ] **Step 7：跑測試確認通過** — 同 Step 2 指令＋`tests/test_main.py`，Expected：PASS
- [ ] **Step 8：lint、全套測試、commit**

```bash
git add src/ui src/reader/supervisor.py src/main.py tests
git commit -m "feat(ui): per-tab message views with a game-synced tab bar"
```

---

### Task 6：收訊迴圈接線——分頁、前景客戶端、分頁上下文

**Files:**
- Modify: `src/translation/context.py`、`src/reader/loop.py`、`src/main.py`（`contexts`、`context_for`、`spawn`）
- Modify: `tests/test_reader_loop.py`、`tests/test_context.py`

**Interfaces:**
- Consumes：Task 4 `ChatEvent`、`WizChatReader.tab_state()`；Task 5 `OverlayWindow.add_message(..., slot, tabs)`、`set_tabs`、`set_foreground_slot`。
- Produces：
  - `src/translation/context.py`：`class ClientContexts`：`for_tab(tab: int) -> ChatContext`（不存在就建）、`current() -> ChatContext`（`for_tab(self.current_tab)`）、屬性 `current_tab: int = 0`；執行緒安全。
  - `reader_loop(..., contexts: ClientContexts | None = None)`：取代原 `context` 參數。
  - `src/reader/loop.py`：模組層 `def foreground_window() -> int`（包 `win32gui.GetForegroundWindow`，例外回 0；測試 patch 它）。

- [ ] **Step 1：寫失敗測試**

`tests/test_context.py`：
```python
def test_client_contexts_keep_tabs_apart():          # for_tab(0).push("a") 不出現在 for_tab(1).snapshot()
def test_client_contexts_current_follows_current_tab(): # current_tab=1 → current() is for_tab(1)
```
`tests/test_reader_loop.py`（`FakeOverlay` 加 `set_tabs`／`set_foreground_slot` 記錄呼叫、`add_message` 收 `tabs`）：
```python
def test_events_carry_their_tabs_to_the_overlay():         # ChatEvent((1,), line) → add_message(..., tabs=(1,))
def test_player_lines_use_their_own_tab_context():          # tab 1 的行 → pool.submit 的 context 只含 tab 1 先前的行
def test_tab_state_is_sent_only_when_it_changes():          # 同一 TabState 連三輪 → set_tabs 只呼叫一次；current_tab 同步到 contexts
def test_tab_state_is_cleared_when_the_game_goes_away():    # GameNotRunning 後 set_tabs(slot, None)
def test_becoming_foreground_reports_the_slot_once():       # foreground_window 連續回 hwnd 三輪 → set_foreground_slot 一次；離開再回來 → 再一次
```
既有 `test_context_advances_by_read_order_not_by_completion` 等改用 `ClientContexts`。

- [ ] **Step 2：跑測試確認失敗** — `uv run pytest tests/test_reader_loop.py tests/test_context.py -q`，Expected：FAIL
- [ ] **Step 3：實作 `ClientContexts`**
- [ ] **Step 4：改 `reader_loop`**：玩家行用 `contexts.for_tab(event.tabs[0])`；系統行流程不變但帶 `tabs`；每輪 `tab_state()` 有變化才排 `set_tabs`，並設 `contexts.current_tab`；`foreground_window() == hwnd` 由否轉是時排 `set_foreground_slot(slot)`；`GameNotRunning` 分支在先前有狀態時排 `set_tabs(slot, None)`。
- [ ] **Step 5：改 `main.py`**：`contexts: dict[int, ClientContexts]`；`context_for` 回 `contexts[hwnd].current().snapshot()`；`spawn` 傳 `contexts=`。
- [ ] **Step 6：跑測試確認通過** — 同 Step 2 指令＋`tests/test_main.py`，Expected：PASS
- [ ] **Step 7：lint、全套測試、commit**

```bash
git add src/translation/context.py src/reader/loop.py src/main.py tests
git commit -m "feat(reader): route chat by game tab and follow the foreground client"
```

---

### Task 7：規則與文件同步、實機驗收

**Files:**
- Modify: `CLAUDE.md`、`AGENTS.md`（兩份逐字相同）、`README.md`、`README_ZH-TW.md`、`README_ZH-CN.md`

- [ ] **Step 1：`CLAUDE.md` 與 `AGENTS.md` 的「wizwalker 掛入須知」**：改為「收訊靠自訂 AddLine hook（`src/reader/chat_hook.py`）讀聊天事件，另掛 root_window hook 供輸入框偵測與分頁狀態；不啟 player/duel/quest 等其他 hook」，其餘句子不動。改完 `diff CLAUDE.md AGENTS.md` 無輸出。
- [ ] **Step 2：三份 README**：多開段落改成「翻譯視窗跟著前景的遊戲客戶端」，刪除「訊息前面標示來自哪一個」；功能清單加一條分頁同步（翻譯視窗的分頁與遊戲聊天分頁同步，名稱與未讀數照遊戲顯示）。不寫設定鍵名與選項文字。
- [ ] **Step 3：grep 殘留**：`grep -rn "差分\|chatLog 全文\|mirrored\|multi-client mode\|slot_marker\|lines_from_nodes" src tests README*.md CLAUDE.md AGENTS.md`，每個命中都確認是否為過期描述並修正。
- [ ] **Step 4：lint、全套測試、commit**

```bash
git add CLAUDE.md AGENTS.md README.md README_ZH-TW.md README_ZH-CN.md
git commit -m "docs: describe the AddLine hook reader and tab sync"
```

- [ ] **Step 5：實機驗收（交給使用者執行，依 spec「實機驗收」1–9 項）**：請使用者關閉其他翻譯器實例後 `uv run run.py`，逐項回報；`app.log`（終端機輸出）與 `messages.log` 用來核對。第 5 項若對應表有誤，修正 `TAB_BUTTONS` 並補測試後另一個 commit。
