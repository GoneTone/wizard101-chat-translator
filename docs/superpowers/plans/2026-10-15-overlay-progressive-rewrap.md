# 翻譯視窗漸進重新換行 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 縮放翻譯視窗時只同步畫視口所需的訊息，其餘在背景分批補畫，捲到哪先畫哪，往上捲看不到空白。

**Architecture:** `MessageList` 的每個視圖（客戶端, 分頁）記一段連續的已畫範圍 `[lo, hi)`，範圍外的記錄沒有文字物件；
換行寬度改變時刪掉全部文字物件，從錨點訊息出發同步畫滿視口，其餘交給 `after` 排程的背景補畫與每次視口移動後的
即時補畫。捲動範圍＝已畫範圍＋待補記錄的估計高度。`Selection` 的登記表改為涵蓋全部訊息（未畫的只有文字、沒有行），
caret 序號因此不受補畫順序影響。

**Tech Stack:** Python 3.14、tkinter Canvas、pytest（`-n 4 --dist loadgroup`）。

**Spec:** `docs/superpowers/specs/2026-10-15-overlay-progressive-rewrap-design.md`

## Global Constraints

- 外觀、框選、捲動跟隨與錨點行為不變；完成補畫後的排版與「以新寬度一次全部重畫」逐像素一致。
- 縮放的同步部分：1000 則中位數 ≤ 200 ms（本程序 CPU 時間），1000 則／250 則比值 ≤ 2。
- 背景補畫每批預算約 20 ms；每批實測 ≤ 60 ms。寬度停止變化約 150 ms 後才開始背景補畫，寬度再變就作廢前一輪。
- 即時補畫：視口上下緣離已畫範圍不足一個視口高度就同步補到超過，且在 Tk 重繪之前完成。
- 隱藏中的視圖不背景補畫。
- 註解預設不寫，要寫就一行講 WHY，全形標點；docstring 一行摘要。log 用英文、前綴 `[ui]`，經 `src.log.log`。
- 測試不可干擾執行中的遊戲：overlay 停在螢幕外（既有 `root` fixture 已處理），不搶焦點。
- 每個 task 提交前 `uv run ruff check src tests tools` 與 `uv run pytest` 全綠；commit 訊息英文 conventional commits；不 push。

## Review Focus

1. 只改高度不改寬度（拉高視窗）時視口變大，補畫中也不能露出空白 —— Task 2 加測試。
2. 縮放後把捲軸拖到最頂端：最終要停在第一則訊息、不能留在估計出來的空白上方 —— Task 2 加測試。
3. 縮放時分頁沒有訊息：不出錯、不記 rewrap log —— Task 2 加測試。
4. 補畫中框選拖到上緣，自動捲動進入未畫區：視口不露空白、選取的文字不錯位 —— Task 3 加測試。
5. 補畫中切到另一個客戶端再切回（雙開）：沿用或重排的規則正確，畫面沒有空白 —— Task 3 加測試。

---

### Task 1: 選取登記表涵蓋全部訊息

**Files:**
- Modify: `src/ui/selection.py`
- Modify: `src/ui/message_list.py`（登記方式；行為不變）
- Modify: `docs/superpowers/specs/2026-10-15-overlay-progressive-rewrap-design.md`（第 5 節「caret 序號跟著平移」改寫為「登記表涵蓋全部訊息，未畫的只記文字，序號不受補畫影響」）
- Test: `tests/test_selection.py`

**Interfaces:**
- Produces（`src/ui/selection.py`）：
  - `class Row(NamedTuple)`：`texts: tuple[str, ...]`、`lines: tuple[Line, ...]`（空＝尚未畫）。
  - 登記表的值一律是 `Row`；`Selection.register(row, lines)` 簽名不變，內部以 `line_text` 組出 `Row`。
  - `Selection.text()` 一律從 `Row.texts` 取字（不讀畫布）。
  - `_hit`、`_clamped`、`_draw` 略過 `lines` 為空的列；`_clamped` 的上下夾取以第一／最後一則**已畫**的列為界，
    回傳的 caret 序號仍是它在整份登記表的序號。
  - `Selection.paint(row) -> None`：只為這一列補畫反白（該列在選取範圍內且已畫時），不擦掉其他列的反白。
- Produces（`src/ui/message_list.py`）：`view.rows[m.tag] = Row((m.original, m.translated), m.lines)`；
  `update_message` 改字後同步更新該列的 `Row.texts`。

- [ ] **Step 1: 寫失敗測試**（`tests/test_selection.py`，沿用 `three_messages` fixture 的建法）

```python
def test_text_of_a_span_over_an_undrawn_row_comes_from_its_texts(three_messages):
    sel, rows = three_messages
    sel._rows[rows[1][0]] = Row(("中間原文", "中間譯文"), ())   # 中間那則尚未畫
    sel.begin(*_at(rows[0][1]))
    sel.extend(*_at(rows[2][2], dx=_width(rows[2][2]) - 1))
    assert "中間原文\n中間譯文" in sel.text()


def test_hit_testing_skips_undrawn_rows(three_messages):
    sel, rows = three_messages
    sel._rows[rows[0][0]] = Row(("a", "b"), ())
    assert sel.begin(*_at(rows[1][1]))
    assert sel.span() is None and sel._anchor.message == 1


def test_paint_adds_the_highlight_of_one_row_only(three_messages):
    sel, rows = three_messages
    sel.begin(*_at(rows[0][1]))
    sel.extend(*_at(rows[2][2], dx=_width(rows[2][2]) - 1))
    canvas = rows[1][1].canvas
    canvas.delete("sel")
    sel.paint(rows[1][0])
    assert canvas.find_withtag("sel")
```

- [ ] **Step 2: 跑測試確認失敗** — `uv run pytest tests/test_selection.py -n0 -q`，預期 `Row` 未定義／`paint` 不存在。
- [ ] **Step 3: 實作** `Row`、`register`、`text`、`paint`、`_hit`／`_clamped`／`_draw` 略過空列；`message_list.py` 改登記 `Row`。
  既有呼叫 `self._rows.values()` 取行的地方改取 `.lines`。
- [ ] **Step 4: 跑全套** — `uv run ruff check src tests tools && uv run pytest`，全綠。
- [ ] **Step 5: 同步 spec 第 5 節後提交** — `refactor(ui): register every message with the selection, drawn or not`

---

### Task 2: 已畫範圍、只畫視口的重排與即時補畫

**Files:**
- Modify: `src/ui/outlined_text.py`、`src/ui/message_list.py`
- Test: `tests/test_outlined_text.py`、`tests/test_overlay.py`、`tests/test_overlay_perf.py`

**Interfaces:**
- Consumes: Task 1 的 `Row`、`Selection.paint(row)`。
- Produces（`src/ui/outlined_text.py`）：`measure_height(canvas, text, *, font, wrap) -> int`：不留下任何物件，
  回傳值等於以同參數 `draw_outlined` 畫出後的 `line_height`。
- Produces（`src/ui/message_list.py`）：
  - `_View.lo: int`、`_View.hi: int`：已畫範圍（`records[lo:hi]` 有文字物件，其餘 `lines == ()`）。
  - `_Message.tag` 在 `add_message` 時就指派（不再等到畫的時候）；新增欄位 `height: float`（上次畫出時的整則高度，
    含上下 `_ROW_PAD`）與 `drawn_wrap: int`（量到該高度時的換行寬度），供估計用。
  - `MessageList._estimate(m: _Message) -> float`：不排版的估計高度（依 `drawn_wrap`／目前 `wrap` 比例放大多行訊息；
    從沒畫過的訊息用兩種字型各一行的高度）。
  - `MessageList._relayout(key) -> None`：取代 `_rewrap`。刪掉該視圖全部文字物件、全部記錄 `lines=()`，
    依 spec 第 1 節的錨點同步畫滿視口；**不排背景補畫**（Task 3 接上）。
  - `MessageList._fill_viewport() -> None`：顯示中視圖的視口上下緣離已畫範圍不足一個視口高度就同步補畫；畫到最頂端
    （`lo == 0`）時，若視口停在內容上方就把捲動範圍收回並貼齊第一則。在 `on_wheel`、`_user_scroll`、`scroll_by`、
    `refresh_scroll`（含錨點復位之後）與高度改變的 `<Configure>` 結尾呼叫。
  - `MessageList.complete_layout() -> None`：把顯示中視圖剩下的全部同步畫完（測試與極端跳轉用）。
  - `_region()`：已畫範圍上下緣各加上待補記錄的估計高度總和。
  - `_view_anchor()`：改取視口頂端那則**已畫**訊息與其列頂的視口偏移；`_restore_anchor` 找不到該 tag 時退回目前
    第一則已畫訊息、偏移不變。
  - 新訊息：`hi == len(records)` 時照舊畫在最下方（含隱藏視圖）；否則只記錄。`update_message`、`_trim`、`prune`
    對未畫記錄只改記錄／`rows`，對已畫的照舊；兩者都要維護 `lo`／`hi`。
  - 同步階段耗時 ≥ 50 ms 且有訊息時記 `[ui] view relaid out (slot=…, tab=…, drawn=…, messages=…, wrap=…, ms=…)`。

**往上畫的位置**（演算法，`_draw` 只會往下接）：

```
top_of_drawn = 已畫第一則的列頂 - _ROW_PAD
h0 = measure_height(原文), h1 = measure_height(譯文)
新一則的原文行原點 y = top_of_drawn - _ROW_PAD - (h0 + h1)   # 與往下接時的列距 2*_ROW_PAD 相同
```

- [ ] **Step 1: 寫失敗測試**

`tests/test_outlined_text.py`：

```python
@pytest.mark.parametrize("text,wrap", [("短", 300), ("很長的訊息" * 30, 200), ("word " * 60, 120)])
def test_measured_height_matches_the_drawn_line(root, text, wrap):
    c = tk.Canvas(root)
    expected = line_height(draw_outlined(c, 0, 10, text, fill="#fff", font=ui_font(11), wrap=wrap, tag="x"))
    before = len(c.find_all())
    assert measure_height(c, text, font=ui_font(11), wrap=wrap) == expected
    assert len(c.find_all()) == before
```

`tests/test_overlay.py`（新增 helper 與測試；`_long_overlay(root, n)` 建 `OverlayWindow(width=460, height=300,
max_messages=n)`，加入 n 則長度不一、會換行的中英文訊息並 `update()`）：

```python
def _covered(ov) -> bool:
    """視口範圍內沒有未畫的區域。"""
    ml, view = ov._list, ov._list._views[ov._list.shown_key]
    top, bottom = ml._canvas.canvasy(0), ml._canvas.canvasy(ml._canvas.winfo_height())
    first, last = view.records[view.lo], view.records[view.hi - 1]
    return ((view.lo == 0 or line_origin(first.lines[0])[1] <= top)
            and (view.hi == len(view.records) or _row_bottom(last) >= bottom))

def test_resize_draws_only_the_viewport_then_completes_identically(root):
    ov = _long_overlay(root, 300)
    _configure_width(ov, 300)                      # 送 FakeEvent 給 _on_canvas_configure
    view = ov._list._views[ov._list.shown_key]
    assert view.hi == 300 and view.hi - view.lo < 60
    assert _covered(ov)
    ov._list.complete_layout()
    assert _layout(ov) == _layout(_fresh_overlay_with_same_messages(root, ov, wrap=ov._list.wrap))
    # _layout：每則兩行相對第一則的 y 偏移、行高與 itemcget width 的串列

def test_wheeling_up_after_a_resize_never_shows_a_gap(root):
    ov = _long_overlay(root, 300)
    _configure_width(ov, 300)
    for _ in range(400):
        ov._list.on_wheel(_Wheel(delta=120))
        assert _covered(ov)

def test_dragging_the_scrollbar_to_the_top_lands_on_the_first_message(root):
    ov = _long_overlay(root, 300)
    _configure_width(ov, 300)
    ov._list._user_scroll("moveto", 0.0)
    ov._win.update_idletasks()
    first = ov._list._messages[0]
    assert first.lines and round(line_origin(first.lines[0])[1]) - ov._list._canvas.canvasy(0) == _ROW_PAD

def test_taller_window_during_partial_layout_shows_no_gap(root):
    ov = _long_overlay(root, 300)
    _configure_width(ov, 300)
    ov._apply_geometry(10000, 10000, ov._list._canvas.winfo_width() + 12, 700)
    ov._win.update_idletasks()
    assert _covered(ov)

def test_resizing_an_empty_view_logs_nothing(root, monkeypatch):
    logged = []
    monkeypatch.setattr(message_list_module, "log", logged.append)
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    _configure_width(ov, 240)
    assert not [line for line in logged if "relaid out" in line or "rewrapped" in line]

def test_follow_mode_keeps_the_newest_message_at_the_bottom_after_a_resize(root):
    ov = _long_overlay(root, 300)
    _configure_width(ov, 300)
    ov._win.update_idletasks()
    assert ov._list._canvas.yview()[1] == 1.0

def test_reading_history_keeps_the_viewport_top_message_in_place_on_resize(root):
    ov = _long_overlay(root, 300)
    _scroll_up(ov, 0.5)
    ov._win.update()
    tag, offset = ov._list._view_anchor()
    _configure_width(ov, 300)
    ov._win.update_idletasks()
    assert abs(_offset_of(ov, tag) - offset) <= 1

def test_updates_trims_and_adds_during_partial_layout_match_a_full_redraw(root):
    ov = _long_overlay(root, 300)   # max_messages=300
    _configure_width(ov, 300)
    ov.update_message(ov._list._messages[5].msg_id, "補上的長譯文" * 20)      # 未畫
    ov.update_message(ov._list._messages[-1].msg_id, "補上的長譯文" * 20)     # 已畫
    ov.add_message("新來的", "擠掉最舊一則")
    ov._list.complete_layout()
    assert _layout(ov) == _layout(_fresh_overlay_with_same_messages(root, ov, wrap=ov._list.wrap))
```

既有 `test_rewrap_between_taking_and_restoring_the_anchor_keeps_the_view` 改為：取錨後縮放、`refresh_scroll(anchor)`，
斷言錨點那則（視口頂端那則）的視口偏移不變。其餘需要「全部已畫」的既有斷言，在斷言前呼叫 `ov._list.complete_layout()`；
不得放寬斷言。

`tests/test_overlay_perf.py`：

```python
def test_the_synchronous_part_of_a_resize_is_independent_of_message_count(root):
    """縮放的同步部分：1000 則中位數 ≤ 0.2 秒（CPU），1000／250 ≤ 2。"""
    small, large = _stale_overlay(root, 250), _stale_overlay(root, 1000)
    _switch(small, 1), _switch(large, 1)
    def cpu(ov, width):
        start = time.process_time(); _resize(ov, width); return time.process_time() - start
    pairs = [(cpu(small, w), cpu(large, w)) for w in (240, 460, 240, 460, 240)]
    t1000 = statistics.median(p[1] for p in pairs)
    assert t1000 <= 0.200, pairs
    assert t1000 / max(statistics.median(p[0] for p in pairs), 1e-3) <= 2, pairs
```

並刪除 `test_rewrapping_a_stale_tab_scales_linearly` 與只供它用的 `_stale_rewrap_seconds`。

- [ ] **Step 2: 跑測試確認失敗** — `uv run pytest tests/test_outlined_text.py tests/test_overlay.py tests/test_overlay_perf.py -n0 -q`
- [ ] **Step 3: 實作** 上述 Interfaces。`_on_canvas_configure`：寬度變了 → `_relayout(shown_key)`；不論寬度有沒有變，
  結尾都排 `refresh_scroll`（它會呼叫 `_fill_viewport`）。`show()`：換行寬度過期 → `_relayout`；否則照舊。
  縮放時已存在的選取依 spec 第 5 節：caret 照留，`redraw()` 只畫已畫的行；每畫出一則就 `selection.paint(tag)`。
- [ ] **Step 4: 跑全套** — `uv run ruff check src tests tools && uv run pytest`，全綠。
- [ ] **Step 5: 提交** — `perf(ui): lay out only the viewport when the wrap width changes`

---

### Task 3: 背景補畫與視圖切換

**Files:**
- Modify: `src/ui/message_list.py`
- Modify: `docs/superpowers/specs/2026-10-15-overlay-progressive-rewrap-design.md`（實測後若調整 20 ms／150 ms 要同步）
- Test: `tests/test_overlay.py`、`tests/test_overlay_perf.py`

**Interfaces:**
- Consumes: Task 2 的 `_View.lo/hi`、`_relayout`、`_fill_viewport`、`complete_layout`。
- Produces（`src/ui/message_list.py`）：
  - 常數 `_BACKFILL_BUDGET = 0.020`（秒）、`_BACKFILL_DELAY_MS = 150`。
  - `MessageList._backfill_once() -> bool`：為顯示中視圖補畫一批（以 `time.perf_counter` 控制在預算內，至少畫一則），
    離視口近的一側優先、兩側輪流；回傳是否還有待補。
  - 排程：`_relayout` 後以 `after(_BACKFILL_DELAY_MS)` 啟動；之後每批以 `after(1)` 接續；換行寬度再變、切走視圖時
    取消排程（`after_cancel`）並以世代計數作廢已排入的批次。
  - `show(key)`：寬度相同且已畫範圍含最新一則 → 沿用、`refresh_scroll`（貼底後 `_fill_viewport`）並恢復背景補畫；
    寬度相同但最新一則未畫 → 視同過期，`_relayout`。
  - 補完時記 `[ui] view rewrapped (slot=…, tab=…, messages=…, wrap=…, ms=…)`，ms＝從 `_relayout` 開始到補完的牆鐘時間；
    沒有訊息不記。

- [ ] **Step 1: 寫失敗測試**

`tests/test_overlay.py`：

```python
def test_background_backfill_completes_without_further_input(root):
    ov = _long_overlay(root, 300)
    _configure_width(ov, 300)
    _pump_until(root, lambda: _view(ov).lo == 0 and _view(ov).hi == 300, timeout=15)
    assert _layout(ov) == _layout(_fresh_overlay_with_same_messages(root, ov, wrap=ov._list.wrap))

def test_only_the_last_width_of_a_drag_is_backfilled(root):
    ov = _long_overlay(root, 300)
    for width in (420, 380, 340, 300):
        _configure_width(ov, width)
    _pump_until(root, lambda: _view(ov).lo == 0, timeout=15)
    for m in ov._list._messages:
        for line in m.lines:
            assert int(line.canvas.itemcget(line.fg, "width")) == ov._list.wrap

def test_a_hidden_view_is_not_backfilled_and_resumes_when_shown(root):
    ov = _long_overlay_with_two_clients(root, 300)   # slot 1 與 slot 2 各 300 則
    _configure_width(ov, 300)
    lo = _view(ov).lo
    ov.set_foreground_slot(2)
    _pump(root, seconds=0.5)
    ov.set_foreground_slot(1)
    assert _view(ov).lo == lo and _covered(ov)         # 隱藏期間沒補；切回沿用已畫範圍
    _pump_until(root, lambda: _view(ov).lo == 0, timeout=15)

def test_switching_back_to_a_view_whose_newest_message_is_undrawn_relays_it_out(root):
    ov = _long_overlay_with_two_clients(root, 300)
    _scroll_up(ov, 0.2)
    _configure_width(ov, 300)                            # 錨點在上方，最新一則還沒畫
    assert _view(ov).hi < 300
    ov.set_foreground_slot(2)
    ov.set_foreground_slot(1)
    ov._win.update_idletasks()
    assert _view(ov).hi == 300 and _covered(ov) and ov._list._canvas.yview()[1] == 1.0

def test_autoscrolling_a_selection_into_the_undrawn_area_keeps_it_on_the_same_text(root):
    ov = _long_overlay(root, 300)
    _configure_width(ov, 300)
    _select_whole_message(ov, index=_view(ov).lo)
    before = ov._selection.text()
    for _ in range(30):
        ov._list.scroll_by(-40)                          # 框選拖到上緣時的自動捲動路徑
        assert _covered(ov)
    assert _view(ov).lo < 300 - 60 and ov._selection.text() == before
```

`tests/test_overlay_perf.py`：

```python
def test_each_backfill_batch_stays_short(root):
    """背景補畫每批 ≤ 60 ms（CPU；預算 20 ms 的三倍餘裕）。"""
    ov = _stale_overlay(root, COUNT)
    _switch(ov, 1)
    _resize(ov, 240)
    batches = []
    while True:
        start = time.process_time()
        more = ov._list._backfill_once()
        batches.append(time.process_time() - start)
        if not more:
            break
    assert max(batches) <= 0.060, max(batches)
```

（`_view(ov)` 回傳顯示中的 `_View`；`_pump_until`／`_pump` 以 `root.update()` 加 `time.sleep(0.01)` 迴圈實作，逾時就失敗；
`_long_overlay_with_two_clients` 以 `set_tabs` 建 slot 1、2 各一個分頁並各加 300 則，前景停在 slot 1。）

- [ ] **Step 2: 跑測試確認失敗** — `uv run pytest tests/test_overlay.py tests/test_overlay_perf.py -n0 -q -k "backfill or hidden or undrawn or drag"`
- [ ] **Step 3: 實作** 上述 Interfaces。
- [ ] **Step 4: 跑全套** — `uv run ruff check src tests tools && uv run pytest`，全綠。
- [ ] **Step 5: 若門檻依實測調整，同步 spec；提交** — `perf(ui): backfill rewrapped messages in the background`
