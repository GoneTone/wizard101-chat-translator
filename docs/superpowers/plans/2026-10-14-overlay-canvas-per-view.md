# 翻譯視窗訊息清單改為每分頁一個畫布 實作計畫

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 切換翻譯視窗分頁的時間與訊息數無關，外觀與操作完全不變。

**Architecture:** 新模組 `outlined_text.py` 負責在任意畫布上畫一行描邊文字並回傳「行把手」`Line`；`selection.py` 改以 `Line` 為單位做命中、反白與取字；`message_list.py` 為每個（客戶端, 分頁）建一個畫布，訊息直接畫在上面，切分頁只換 pack 的畫布。

**Tech Stack:** Python 3.11+、tkinter、pytest（xdist）。

**Spec:** `docs/superpowers/specs/2026-10-14-overlay-canvas-per-view-design.md`

## Global Constraints

- 分支 `perf/overlay-canvas-views`，原目錄工作、不建 worktree；**不要 push**。
- 每個 task 結束前：`uv run ruff check src tests tools` 零錯誤、`uv run pytest` 全綠（全套每 task 只跑一次）。
- commit message 英文 conventional commits，結尾附 `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` 與 `Claude-Session: https://claude.ai/code/session_01Jd7ExtVDMHEKLpugpvrmxa`。
- 外觀與操作完全不變；既有測試只能改「取得畫面元件的方式」，不得刪除或放寬斷言（唯一例外見 Task 2 Step 1 的 `test_expand_realigns_the_message_container`）。
- log 英文、前綴 `[ui]`；註解一行 WHY、全形標點；docstring 一行。
- 測試不可搶焦點、不可依賴真實前景視窗；視窗沿用 conftest 的螢幕外停放。
- 原型參考：`.superpowers/sdd/overlay-canvas-prototypes/`（`bench_canvas_per_view.py` 是選定做法）。

## Review Focus

1. **縮成泡泡期間收到訊息**，展開後最新一則要完整落在視口內。→ Task 2 `test_messages_received_as_a_bubble_are_visible_after_expand`。
2. **隱藏分頁裡的訊息被回填譯文、行高改變**，切過去時下方訊息不重疊。→ Task 2 `test_translation_growing_a_hidden_line_pushes_later_messages_down`。
3. **隱藏分頁時縮放視窗**，切過去時換行寬度是新的。→ Task 2 `test_resize_while_hidden_rewraps_on_show`。
4. **框選中最舊的訊息被淡出移除**，選取的文字與反白位置不變。→ Task 2 `test_pruning_above_a_selection_keeps_text_and_highlight`。
5. **新建分頁畫布的滾輪**要能捲動。→ Task 2 `test_wheel_scrolls_a_newly_created_view`。

---

### Task 1：描邊文字行與以行把手為單位的框選

**Files:**
- Create: `src/ui/outlined_text.py`、`tests/test_outlined_text.py`
- Modify: `src/ui/selection.py`、`src/ui/message_list.py`（只改 `_outlined_line` 與 `register` 的呼叫）、`tests/test_selection.py`

**Interfaces:**
- Produces（`src/ui/outlined_text.py`）：
  - `TEXT_ORIGIN = 2`、`OUTLINE_OFFSETS`（8 個方向，與現行 `_OUTLINE_OFFSETS` 相同）
  - `class Line(NamedTuple)`: `canvas: tk.Canvas`, `fg: int`（本色文字物件 id）, `tag: str`（該行 9 個文字物件共用的 tag）
  - `def draw_outlined(canvas, x: int, y: int, text: str, *, fill: str, font: tuple, wrap: int, tag: str) -> Line`：在 `(x, y)` 為行原點畫一行；8 個描邊副本畫在 `(x+TEXT_ORIGIN+dx, y+TEXT_ORIGIN+dy)`、`fill=OUTLINE`，本色畫在 `(x+TEXT_ORIGIN, y+TEXT_ORIGIN)`；全部 `anchor="nw"`、`width=wrap`、tags 含 `tag` 與 `"txt"`，本色另含 `"fg"`。
  - `def line_origin(line: Line) -> tuple[float, float]`：本色物件座標減 `TEXT_ORIGIN`。
  - `def line_height(line: Line) -> int`：`bbox(line.tag)` 底緣 − 原點 y ＋ 2（與現行 `_fit_line_height` 的畫布高度一致）。
- Produces（`src/ui/selection.py`，公開名稱不變、參數改吃 `Line`）：`line_text(line)`、`line_font(line)`、`caret_at(line, x, y)`、`visual_lines(line, font)`、`highlight_rects(line, font, start, end)`（座標皆為「行內座標」＝以行原點為 (0, 0)，與舊版單行畫布的本地座標相同）；`Selection.register(row, lines: Sequence[Line])`（`row` 任意可雜湊物件）、`forget(row)`、`holds(row)`，新增 `use_rows(rows: dict) -> None`（清除選取、直接改用傳入的 dict 當登記表，呼叫端持有它）。`TEXT_ORIGIN` 仍可自 `src.ui.selection` 匯入。
- 行的命中範圍：水平是整個畫布寬（舊版行畫布 `fill="x"`），垂直是 `[原點 y, 原點 y + line_height)`；螢幕座標換算要扣掉畫布的捲動偏移（`canvasx(0)`／`canvasy(0)`）。反白矩形畫在 `line.canvas`、tag `"sel"`，壓在該行文字之下（`tag_lower(rect, line.tag)`）。

- [ ] **Step 1：寫失敗測試**
  - `tests/test_outlined_text.py`：
    - `test_draw_outlined_matches_the_old_per_line_canvas`：在 `(0, 0)` 畫一行，9 個物件的座標、`fill`、`font`、`width`、`anchor` 等於舊 `_outlined_line` 的值，期望值寫成常數（描邊在 `(TEXT_ORIGIN+dx, TEXT_ORIGIN+dy)`、`fill=OUTLINE`；本色在 `(TEXT_ORIGIN, TEXT_ORIGIN)`；全部 `anchor="nw"`、`width=wrap`）——不可拿 Step 4 改寫後的 `_outlined_line` 對照自己。
    - `test_line_height_matches_fit_line_height`：同文字同寬度，`line_height` 等於舊畫布 `_fit_line_height` 後的 `height`。
    - `test_draw_outlined_at_an_offset_moves_every_item`：在 `(0, 50)` 畫，`line_origin == (0, 50)`。
  - `tests/test_selection.py`：fixture 改成在**同一個畫布**上用 `draw_outlined` 畫行（`line`、`message`、`three_messages` 都改，其中一則故意畫在 y≠0 處，並讓畫布可捲動），既有斷言全部保留。新增 `test_hit_accounts_for_canvas_scroll`：畫布捲動後點同一個螢幕位置，命中的是捲到該處的那一行。
- [ ] **Step 2：跑測試確認失敗** — `uv run pytest tests/test_outlined_text.py tests/test_selection.py -q -n 0`，Expected：FAIL（模組不存在）
- [ ] **Step 3：實作 `outlined_text.py`，改寫 `selection.py` 以 `Line` 為單位**
- [ ] **Step 4：`message_list.py` 過渡改動**：`_outlined_line` 改用 `draw_outlined(c, 0, 0, ..., tag="txt")` 畫在自己的單行畫布上，`register` 改傳 `(Line, Line)`；其餘不動（Task 2 才換架構）。
- [ ] **Step 5：跑測試確認通過** — `uv run pytest tests/test_outlined_text.py tests/test_selection.py tests/test_overlay.py -q -n 0`，Expected：PASS
- [ ] **Step 6：lint、全套測試、commit**

```bash
git add src/ui/outlined_text.py src/ui/selection.py src/ui/message_list.py tests/test_outlined_text.py tests/test_selection.py
git commit -m "refactor(ui): address selection lines as canvas text items"
```

---

### Task 2：每個分頁一個畫布

**Files:**
- Modify: `src/ui/message_list.py`（重寫內部）、`src/ui/overlay.py`（`_bind_line`）、`tests/test_overlay.py`

**Interfaces:**
- Consumes：Task 1 的 `Line`、`draw_outlined`、`line_origin`、`line_height`、`Selection.use_rows`。
- Produces：`MessageList` 公開介面與現在相同（見 spec 第 1 節）；`bind_line(canvas)` 改為「每個分頁畫布建好時呼叫一次」。測試可用的內部名稱：`_messages`（顯示中分頁的記錄 list）、記錄欄位 `lines: tuple[Line, Line]`（原文行、譯文行）與 `tag: str`（該則訊息所有物件共用的 tag）、`_canvas`（顯示中分頁的畫布）。
- **排版不變式**（重現舊版 frame＋pack 的幾何）：第一則的列頂 y＝2；原文行原點＝(0, 列頂)；譯文行原點＝(0, 列頂＋原文行高)；下一則列頂＝本則列頂＋原文行高＋譯文行高＋4。`scrollregion`＝(0, 第一則列頂−2, 畫布寬, 最後一則列底＋2)，沒有訊息時為 (0, 0, 畫布寬, 0)。
- 刪除最舊訊息時不搬移其餘內容；`update_message` 讓行高改變時，把之後的每則訊息移動差值。
- 寬度改變：顯示中的畫布用 `itemconfigure("txt", width=wrap)` 一次改完再依不變式重排；隱藏的畫布記為過期，`show()` 時補做。
- `show(key)`：清除選取、`use_rows` 換成該分頁的登記表、pack 換畫布、捲軸改接新畫布、必要時補做換行、回到跟隨底部、更新空狀態、記 `[ui] view switched (slot=…, tab=…, messages=…)`；同一個 key 什麼都不做。
- 每個分頁畫布綁 `<Enter>`／`<Leave>` 轉發滾輪（同現行內層畫布），以及 `<Configure>`（只有顯示中的會收到）。
- `overlay._bind_line(canvas)`：在分頁畫布上綁 `<ButtonPress-1>`、`<B1-Motion>`、`<ButtonRelease-1>`、`<Button-3>`，處理函式不變。
- 移除 `refresh_scroll` 裡的 `coords(self._inner_id, 0, 0)` 補丁與 `_inner`／`_inner_id`／`_build_row`／`_drop_row` 中的 widget 邏輯、舊 `_outlined_line`、`_fit_line_height`。

- [ ] **Step 1：改寫測試（先讓它們描述新結構，會失敗）**
  - 既有測試中取 `entry.row.winfo_children()[i]` 的地方改成 `entry.lines[i]`，取顏色改 `line.canvas.itemcget(line.fg, "fill")`，取寬度改 `line.canvas.itemcget(line.fg, "width")`，取列 y 改 `ov._list._canvas.bbox(m.tag)[1]`；斷言值不變。
  - `test_expand_realigns_the_message_container` 改名並改寫為 `test_messages_received_as_a_bubble_are_visible_after_expand`：原本的情境不變，斷言最新一則的 `bbox(tag)` 完全落在 `[canvasy(0), canvasy(0)+winfo_height()]` 內。
  - 新增（Review Focus 2–5 與 spec 的測試）：
    - `test_layout_follows_the_old_frame_geometry`：兩則訊息，原文／譯文行原點符合上方不變式。
    - `test_translation_growing_a_hidden_line_pushes_later_messages_down`：分頁 1 有三則、顯示分頁 0；把第一則回填成會多換一行的長譯文；切到分頁 1 後三則的列範圍互不重疊且符合不變式。
    - `test_resize_while_hidden_rewraps_on_show`：分頁 1 有訊息、顯示分頁 0 時改變畫布寬；切到分頁 1 後所有行的 `width` 等於新的 `wrap`。
    - `test_pruning_above_a_selection_keeps_text_and_highlight`：選取第 2 則部分文字後淡出移除第 1 則；`selection.text()` 不變，反白矩形仍落在第 2 則的行範圍內。
    - `test_wheel_scrolls_a_newly_created_view`：切到新分頁、塞滿到可捲動，在其畫布上 `event_generate("<MouseWheel>", delta=120)` 後 `yview()[0]` 變小。
    - `test_switching_views_keeps_each_views_canvas`：切走再切回，顯示的是同一個畫布物件（沒有重建）。
- [ ] **Step 2：跑測試確認失敗** — `uv run pytest tests/test_overlay.py -q -n 0`，Expected：FAIL（`lines`／`tag` 屬性不存在等）
- [ ] **Step 3：重寫 `message_list.py`、修改 `overlay._bind_line`**
- [ ] **Step 4：跑測試確認通過** — `uv run pytest tests/test_overlay.py tests/test_selection.py tests/test_outlined_text.py -q -n 0`，Expected：PASS
- [ ] **Step 5：殘留檢查** — `grep -rn "_outlined_line\|_fit_line_height\|_inner_id\|winfo_children()\[" src tests`，src 不應有命中；註解與 docstring 裡描述「每行一個 canvas」的文字一併更新（`selection.py` 模組 docstring、`thin_scrollbar.py:70` 的註解）。
- [ ] **Step 6：lint、全套測試、commit**

```bash
git add src/ui/message_list.py src/ui/overlay.py src/ui/selection.py src/ui/thin_scrollbar.py tests/test_overlay.py
git commit -m "perf(ui): give each chat tab its own message canvas"
```

---

### Task 3：效能測試與門檻

**Files:**
- Create: `tests/test_overlay_perf.py`
- Modify: `pyproject.toml`（`addopts`）；必要時 `src/ui/message_list.py`（合併捲動更新）

**Interfaces:**
- Consumes：`OverlayWindow`（正式路徑）、`TabState`。

- [ ] **Step 1：在正式路徑上量一次**（不 commit 的量測腳本或直接寫測試印出數字）：兩分頁各 1000 則的切換中位數、連續新增 1000 則的平均、隱藏分頁換行過期後切過去的重排時間。把數字寫進報告。
- [ ] **Step 2：寫測試 `tests/test_overlay_perf.py`**（`pytestmark = pytest.mark.xdist_group("overlay-perf")`）
  - `test_switching_tabs_is_independent_of_message_count`：兩分頁各 1000 則，`set_tabs` 來回切 5 次（每次含 `update_idletasks()`），中位數 ≤ 0.100 秒。
  - `test_adding_a_message_stays_cheap`：顯示中分頁連續 `add_message` 1000 則，總時間（含最後一次 `update_idletasks()`）÷ 1000 ≤ 0.003 秒。
  - `test_rewrapping_a_stale_tab_is_bounded`：隱藏分頁 1000 則、改變畫布寬後切過去，該次切換 ≤ 0.500 秒。
- [ ] **Step 3：`pyproject.toml` 的 `addopts` 改為 `"-n 4 --dist loadgroup"`**（`xdist_group` 只有在 loadgroup 模式才生效；未分組的測試行為與 load 相同），並在該行上方註解補一句原因。
- [ ] **Step 4：跑測試** — `uv run pytest tests/test_overlay_perf.py -q`（走 4 worker 的預設設定）與 `uv run pytest tests/test_overlay_perf.py -q -n 0`，兩者都 PASS。
- [ ] **Step 5：若新增一則超過門檻**：把 `add_message` 中的捲動更新改成同一輪事件只做一次（例如以 `after_idle` 排一次 `refresh_scroll`，已排過就不重排），確認 `tests/test_overlay.py` 的跟隨與錨點測試全數通過後再跑 Step 4。沒超過就跳過本步。
- [ ] **Step 6：lint、全套測試、commit**

```bash
git add tests/test_overlay_perf.py pyproject.toml src/ui/message_list.py
git commit -m "test(ui): guard tab switching and message adding performance"
```

---

### 實機驗收（交給使用者）

重新打包 exe，依 spec「實機驗收」1–4 項請使用者測試：外觀並排比對、框選各情境、捲動與泡泡展開、分頁累積上百則後來回切換。
