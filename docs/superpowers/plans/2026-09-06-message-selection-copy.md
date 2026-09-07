# 疊加視窗訊息框選與複製 實作計畫

> **狀態：已執行完畢（2026-09-06），本文是當時的執行記錄，不是現行設計。**
> 六個 task 都已完成並合併回 `master`。實機驗證後有數項決策改變——跨訊息選取、
> 拖曳自動捲動、右鍵選單改為自繪視窗等——本文**未**回頭改寫，因此下文描述的
> 行為有一部分已經不是程式現況。現行設計請看
> [spec 的「事後修訂」節](../specs/2026-09-06-message-selection-copy-design.md#事後修訂2026-09-07)。

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓使用者能在疊加視窗的訊息上用滑鼠框選文字（可跨同一則的原文行與譯文行、不跨訊息），並以 `Ctrl+C` 或右鍵選單複製到剪貼簿。

**Architecture:** 新增 `src/ui/selection.py`，內含「向 Tk 問排版」的純函式與持有選取狀態的 `Selection` 類別；反白以 `create_rectangle` 自繪並壓在描邊文字之下。`src/ui/overlay.py` 只負責事件路由（文字 canvas 與 backdrop 雙路由，統一螢幕座標）、剪貼簿與右鍵選單。

**Tech Stack:** Python 3.14、tkinter（Canvas text item 的 `index("@x,y")` 與 `tkinter.font.Font.measure`）、pywin32、pytest（含既有的 `root` fixture）、uv。

**Spec:** `docs/superpowers/specs/2026-09-06-message-selection-copy-design.md`

## Global Constraints

- **回答與文件用繁體中文（台灣）**，全形標點；**commit message 用英文**、conventional commits 格式。
- **log 訊息內容一律英文**，前綴用模組名 `[ui]`；寫進 log 的只有長度與索引，**不得寫入聊天內容**。用 `src/log.py` 的 `log()`，不要 `print`。
- **註解節制**：預設不寫實作層 `#` 註解；只在 WHY 不顯而易見或需要段落導引時寫。模組與公開函式寫 docstring。
- **不重複造輪子**：命中判定用既有的 `src/ui/geometry.py:point_in_rect`，字型用 `src/ui/fonts.py:ui_font`，不另寫同類函式。
- **UI 文字一律走 i18n**（`src/i18n/__init__.py` 的 `t()`），三份語言檔 `zh-TW`（來源）／`en`／`zh-CN` 都要補齊。
- **提交前品質檢查**：`uv run ruff check src tests` 零錯誤、`uv run pytest` 全過。不得用 `--no-verify`。**不要 git push。**
- 反白色 `SELECT_BG = "#2d4a7a"`，**絕不可用 `BG`**（`#101018` 是本體的 `-transparentcolor`，該色像素連 hit-test 都被 Windows 跳過）。
- 描邊文字 item 的繪製原點是 `(2, 2)`、`anchor="nw"`（見 `overlay._outlined_line`）；本色 item 掛 `"fg"` tag，九個 item 都掛 `"txt"` tag。

---

### Task 1: 選取幾何純函式（`src/ui/selection.py`）

**Files:**
- Create: `src/ui/selection.py`
- Modify: `src/ui/palette.py`（新增 `SELECT_BG`）
- Test: `tests/test_selection.py`（新檔）

**Interfaces:**
- Consumes: `overlay._outlined_line`（僅測試用，用來建出與正式路徑相同的文字行）、`src/ui/fonts.py:ui_font`
- Produces:
  - `Caret(NamedTuple)`：`line: int`、`index: int`
  - `TEXT_ORIGIN: int = 2`
  - `line_text(canvas) -> str`
  - `line_font(canvas) -> tkinter.font.Font`
  - `caret_at(canvas, x: int, y: int) -> int`
  - `visual_lines(canvas, font) -> list[tuple[int, int, int]]`（`(起始索引, 結束索引, y_top)`）
  - `highlight_rects(canvas, font, start: int, end: int) -> list[tuple[int, int, int, int]]`
  - `selected_text(texts: list[str], start: Caret, end: Caret) -> str`
  - `src/ui/palette.py:SELECT_BG`

- [ ] **Step 1: 先加反白色到 palette**

在 `src/ui/palette.py` 的 `OUTLINE` 之後加上：

```python
# 框選反白底色。不可用 BG——那是本體的 -transparentcolor，畫上去等於沒畫（見檔頭）。
# 用不透明色另有好處：反白過的區域從此接得到滑鼠，拖曳回頭經過時事件不會掉到 backdrop。
SELECT_BG = "#2d4a7a"
```

- [ ] **Step 2: 寫失敗的測試**

建立 `tests/test_selection.py`：

```python
import tkinter as tk

import pytest

from src.ui.fonts import ui_font
from src.ui.overlay import _outlined_line
from src.ui.selection import (
    TEXT_ORIGIN,
    Caret,
    caret_at,
    highlight_rects,
    line_font,
    line_text,
    selected_text,
    visual_lines,
)

WRAPPED = "Hello wizard friends 這是一段中英混排的訊息內容用來測試換行"


@pytest.fixture
def line(root):
    """一行描邊文字，寬度窄到一定會換行（建法與 overlay 正式路徑相同）。"""
    frame = tk.Frame(root)
    frame.pack()
    canvas = _outlined_line(frame, WRAPPED, "#f2f2f7", ui_font(11), 120)
    canvas.pack()
    root.update_idletasks()
    yield canvas
    frame.destroy()


def test_line_text_reads_the_foreground_item(line):
    assert line_text(line) == WRAPPED


def test_visual_lines_cover_the_whole_text(line):
    lines = visual_lines(line, line_font(line))
    assert len(lines) > 1, "測試字串應該要換行，否則這個測試沒有守到東西"
    assert lines[0][0] == 0
    assert lines[-1][1] == len(WRAPPED)
    for (_, end, _), (start, _, _) in zip(lines, lines[1:]):
        assert end == start


def test_visual_line_starts_agree_with_tk(line):
    # 行首索引是向 Tk 問出來的，這裡再問一次確認我們挑的取樣點落在正確的視覺行上
    font = line_font(line)
    spacing = font.metrics("linespace")
    for start, _, top in visual_lines(line, font):
        assert caret_at(line, TEXT_ORIGIN, top + spacing // 2) == start


def test_highlight_x_agrees_with_tk_hit_test(line):
    # 哨兵測試：反白的 x 是 font.measure 算的，Tk 的換行排版必須認同同一個位置。
    # 這條轉紅就代表 font.measure 與 Tk 的排版脫鉤了，反白會整片偏掉。
    font = line_font(line)
    spacing = font.metrics("linespace")
    first_start, first_end, top = visual_lines(line, font)[0]
    middle = (first_start + first_end) // 2
    (x0, y0, _, _) = highlight_rects(line, font, middle, first_end)[0]
    assert y0 == top
    assert caret_at(line, x0, top + spacing // 2) == middle


def test_highlight_spans_every_visual_line(line):
    font = line_font(line)
    rects = highlight_rects(line, font, 0, len(WRAPPED))
    assert len(rects) == len(visual_lines(line, font))


def test_empty_selection_has_no_rects(line):
    assert highlight_rects(line, line_font(line), 5, 5) == []


def test_selected_text_within_one_line():
    assert selected_text(["abcdef", "xyz"], Caret(0, 1), Caret(0, 4)) == "bcd"


def test_selected_text_across_both_lines():
    assert selected_text(["abcdef", "xyz"], Caret(0, 4), Caret(1, 2)) == "ef\nxy"


def test_selected_text_of_an_empty_span():
    assert selected_text(["abcdef", "xyz"], Caret(0, 2), Caret(0, 2)) == ""
```

- [ ] **Step 3: 跑測試確認失敗**

Run: `uv run pytest tests/test_selection.py -v`
Expected: FAIL — `ModuleNotFoundError: No module named 'src.ui.selection'`

- [ ] **Step 4: 寫出模組**

建立 `src/ui/selection.py`：

```python
"""疊加視窗訊息的框選：把 Tk 的排版問出來，換算成反白矩形與選取字串。

一則訊息由兩個描邊文字 canvas 組成（原文行、譯文行，見 `overlay._outlined_line`）。
Tk 的 canvas 原生選取全 app 只有一份、跨不了這兩個 canvas，所以反白自繪；但換行
規則不自行實作——逐行問 Tk「這個座標是第幾個字元」即可還原排版。
"""
import tkinter.font as tkfont
from typing import NamedTuple

TEXT_ORIGIN = 2   # 描邊文字 item 的繪製原點（見 overlay._outlined_line 的 create_text(2, 2, ...)）


class Caret(NamedTuple):
    """選取端點。line＝該則訊息內第幾個 canvas（0＝原文行，1＝譯文行）；
    index＝該行文字內的字元索引。NamedTuple 的比較順序恰好就是閱讀順序。"""
    line: int
    index: int


def line_text(canvas) -> str:
    """該行目前顯示的文字。本色 item（"fg" tag）是唯一真實來源，不在別處另記一份。"""
    return canvas.itemcget("fg", "text")


def line_font(canvas) -> tkfont.Font:
    """該行的字型，包成可量測的 Font 物件。"""
    return tkfont.Font(canvas, font=canvas.itemcget("fg", "font"))


def caret_at(canvas, x: int, y: int) -> int:
    """canvas 本地座標對應的字元索引。座標超出文字範圍時 Tk 自己會夾到頭／尾。"""
    return canvas.index("fg", f"@{x},{y}")


def visual_lines(canvas, font) -> list[tuple[int, int, int]]:
    """換行後每個視覺行的 `(起始索引, 結束索引, y_top)`。

    bbox 只取本色 item：描邊副本往外多 1px，用 `bbox("all")` 會把視覺行數多算出來。"""
    box = canvas.bbox("fg")
    text = line_text(canvas)
    if not box or not text:
        return []
    spacing = font.metrics("linespace")
    count = max(1, round((box[3] - box[1]) / spacing))
    starts = [caret_at(canvas, TEXT_ORIGIN, TEXT_ORIGIN + n * spacing + spacing // 2)
              for n in range(count)]
    return [(start, starts[n + 1] if n + 1 < count else len(text),
             TEXT_ORIGIN + n * spacing)
            for n, start in enumerate(starts)]


def highlight_rects(canvas, font, start: int, end: int) -> list[tuple[int, int, int, int]]:
    """`[start, end)` 在該行要反白的矩形（canvas 本地座標，每個視覺行一個）。"""
    if start >= end:
        return []
    text = line_text(canvas)
    spacing = font.metrics("linespace")
    rects = []
    for first, last, top in visual_lines(canvas, font):
        lo, hi = max(start, first), min(end, last)
        if lo >= hi:
            continue
        rects.append((TEXT_ORIGIN + font.measure(text[first:lo]), top,
                      TEXT_ORIGIN + font.measure(text[first:hi]), top + spacing))
    return rects


def selected_text(texts: list[str], start: Caret, end: Caret) -> str:
    """選取範圍內的文字；跨行時以換行相接。純字串運算，不碰 Tk。"""
    if start >= end:
        return ""
    if start.line == end.line:
        return texts[start.line][start.index:end.index]
    parts = [texts[start.line][start.index:]]
    parts.extend(texts[line] for line in range(start.line + 1, end.line))
    parts.append(texts[end.line][:end.index])
    return "\n".join(parts)
```

- [ ] **Step 5: 跑測試確認通過**

Run: `uv run pytest tests/test_selection.py -v`
Expected: PASS（10 個測試）

若 `test_visual_lines_cover_the_whole_text` 抱怨「測試字串應該要換行」，代表 `WRAPPED` 在 120px 寬下沒換到行——把 `_outlined_line` 的 wrap 參數再調小（例如 90），不要改斷言。

- [ ] **Step 6: Lint 與全測試**

Run: `uv run ruff check src tests && uv run pytest -q`
Expected: `All checks passed!` 且全部通過

- [ ] **Step 7: Commit**

```bash
git add src/ui/selection.py src/ui/palette.py tests/test_selection.py
git commit -m "feat(ui): add selection geometry recovered from Tk's own layout"
```

---

### Task 2: `Selection` 類別（狀態、夾取、反白繪製）

**Files:**
- Modify: `src/ui/selection.py`
- Test: `tests/test_selection.py`

**Interfaces:**
- Consumes: Task 1 的 `Caret`／`caret_at`／`visual_lines`／`highlight_rects`／`selected_text`／`line_text`／`line_font`／`SELECT_BG`、既有的 `src/ui/geometry.py:point_in_rect`、`src/log.py:log`
- Produces: `Selection` 類別，方法與屬性如下（Task 3-5 依賴這些名稱）：
  - `register(row, canvases) -> None`
  - `forget(row) -> None`
  - `holds(row) -> bool`
  - `begin(x_root: int, y_root: int) -> bool`
  - `extend(x_root: int, y_root: int) -> None`
  - `finish() -> None`
  - `clear(reason: str) -> None`
  - `redraw() -> None`
  - `span() -> tuple[Caret, Caret] | None`
  - `text() -> str`
  - `active: bool`（property）
  - `dragging: bool`（property）

- [ ] **Step 1: 寫失敗的測試**

在 `tests/test_selection.py` 末端追加（並把 `Selection` 加進檔頭的 import）：

```python
@pytest.fixture
def message(root):
    """兩行一組的訊息列（原文行 ＋ 譯文行），已註冊進一個 Selection。"""
    frame = tk.Frame(root)
    frame.pack()
    first = _outlined_line(frame, "original text", "#c0c0cd", ui_font(9), 400)
    first.pack(fill="x")
    second = _outlined_line(frame, "translated text", "#f2f2f7", ui_font(11), 400)
    second.pack(fill="x")
    root.update_idletasks()
    sel = Selection()
    sel.register(frame, (first, second))
    yield sel, frame, first, second
    frame.destroy()


def _at(canvas, dx=TEXT_ORIGIN, dy=None):
    """canvas 內某點的螢幕座標（dy 預設取該 canvas 的垂直中線）。"""
    if dy is None:
        dy = canvas.winfo_height() // 2
    return canvas.winfo_rootx() + dx, canvas.winfo_rooty() + dy


def test_begin_on_a_line_reports_a_hit(message):
    sel, _, first, _ = message
    assert sel.begin(*_at(first)) is True
    assert sel.dragging is True


def test_begin_outside_every_message_reports_no_hit(message):
    sel, frame, first, _ = message
    x, y = _at(first)
    assert sel.begin(x, y - frame.winfo_height() - 200) is False


def test_a_bare_press_is_not_a_selection(message):
    sel, _, first, _ = message
    sel.begin(*_at(first))
    sel.finish()
    assert sel.active is False
    assert sel.text() == ""


def test_dragging_across_both_lines_selects_both(message):
    sel, _, first, second = message
    sel.begin(*_at(first))
    sel.extend(*_at(second, dx=1000))
    assert sel.text() == "original text\ntranslated text"


def test_reverse_drag_selects_the_same_text(message):
    sel, _, first, second = message
    sel.begin(*_at(second, dx=1000))
    sel.extend(*_at(first))
    assert sel.text() == "original text\ntranslated text"


def test_dragging_below_the_message_clamps_to_its_end(message):
    sel, _, first, second = message
    sel.begin(*_at(first))
    x, y = _at(second)
    sel.extend(x, y + 500)
    assert sel.text() == "original text\ntranslated text"


def test_dragging_above_the_message_clamps_to_its_start(message):
    sel, _, first, second = message
    sel.begin(*_at(second, dx=1000))
    x, y = _at(first)
    sel.extend(x, y - 500)
    assert sel.text() == "original text\ntranslated text"


def test_selection_never_crosses_into_another_message(root):
    # 「不跨訊息」是夾出來的：拖進另一則的範圍，focus 仍夾在起手那一則的結尾
    frames = []
    sel = Selection()
    for original, translated in (("first one", "first two"), ("second one", "second two")):
        frame = tk.Frame(root)
        frame.pack()
        top = _outlined_line(frame, original, "#c0c0cd", ui_font(9), 400)
        top.pack(fill="x")
        bottom = _outlined_line(frame, translated, "#f2f2f7", ui_font(11), 400)
        bottom.pack(fill="x")
        sel.register(frame, (top, bottom))
        frames.append((frame, top, bottom))
    root.update_idletasks()

    sel.begin(*_at(frames[0][1]))
    sel.extend(*_at(frames[1][2], dx=1000))
    assert sel.text() == "first one\nfirst two"
    for frame, _, _ in frames:
        frame.destroy()


def test_highlight_is_drawn_below_the_text(message):
    sel, _, first, second = message
    sel.begin(*_at(first))
    sel.extend(*_at(second, dx=1000))
    assert first.find_withtag("sel"), "原文行應該畫出反白矩形"
    assert first.find_withtag("sel")[-1] < first.find_withtag("txt")[0], \
        "反白必須壓在文字之下，否則會蓋掉字"


def test_clear_removes_the_highlight_and_the_selection(message):
    sel, _, first, second = message
    sel.begin(*_at(first))
    sel.extend(*_at(second, dx=1000))
    sel.clear("test")
    assert sel.active is False
    assert sel.text() == ""
    assert first.find_withtag("sel") == ()


def test_forget_clears_a_selection_in_that_row(message):
    sel, frame, first, second = message
    sel.begin(*_at(first))
    sel.extend(*_at(second, dx=1000))
    sel.forget(frame)
    assert sel.active is False
    assert sel.holds(frame) is False


def test_redraw_keeps_the_selected_text(message):
    sel, _, first, second = message
    sel.begin(*_at(first))
    sel.extend(*_at(second, dx=1000))
    sel.redraw()
    assert sel.text() == "original text\ntranslated text"
    assert first.find_withtag("sel")
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `uv run pytest tests/test_selection.py -v`
Expected: FAIL — `ImportError: cannot import name 'Selection'`

- [ ] **Step 3: 實作 `Selection`**

在 `src/ui/selection.py` 檔頭補上 import：

```python
from src.log import log
from src.ui.geometry import point_in_rect
from src.ui.palette import SELECT_BG
```

在檔末加入類別：

```python
class Selection:
    """疊加視窗訊息的選取狀態：命中、夾取、反白繪製與取字。

    只認螢幕座標——事件從文字 canvas（墨跡像素）或 backdrop（其餘區域因透明色鍵
    而穿透過去）進來都一樣，呼叫端負責換算。選取不跨訊息：起手決定作用中的那一則，
    拖出範圍就夾到該則的頭或尾。"""

    def __init__(self):
        self._rows: dict = {}          # 訊息列 → 該列的行 canvas（插入順序＝顯示順序）
        self._fonts: dict = {}         # 行 canvas → Font；拖曳時每個事件都要量測，不能重建
        self._active = None            # 作用中的那一則（訊息列）
        self._anchor: Caret | None = None
        self._focus: Caret | None = None
        self._dragging = False

    @property
    def active(self) -> bool:
        """目前是否有非空的選取。"""
        return self.span() is not None

    @property
    def dragging(self) -> bool:
        """目前是否正在拖曳選取（拖曳期間畫面不該自動貼底）。"""
        return self._dragging

    def register(self, row, canvases) -> None:
        """登記一則訊息的行 canvas（由上而下）。"""
        self._rows[row] = list(canvases)

    def forget(self, row) -> None:
        """訊息列即將被銷毀：解除登記，選取落在該列時一併清掉——否則選取會指向
        已銷毀的 widget，之後任何一次重畫都會炸。"""
        canvases = self._rows.pop(row, None)
        if canvases is None:
            return
        for canvas in canvases:
            self._fonts.pop(canvas, None)
        if self._active is row:
            self._active = self._anchor = self._focus = None
            self._dragging = False
            log("[ui] selection cleared (row destroyed)")

    def holds(self, row) -> bool:
        """目前的選取是否落在這一列。"""
        return self._active is row

    def span(self) -> tuple[Caret, Caret] | None:
        """正規化後的 `(start, end)`；沒有選取或選取為空則 None。"""
        if self._anchor is None or self._focus is None or self._anchor == self._focus:
            return None
        return min(self._anchor, self._focus), max(self._anchor, self._focus)

    def text(self) -> str:
        """目前選取到的文字（跨行以換行相接）。"""
        span = self.span()
        if span is None:
            return ""
        return selected_text([line_text(c) for c in self._rows[self._active]], *span)

    def begin(self, x_root: int, y_root: int) -> bool:
        """框選起手；回傳是否命中某一則訊息。"""
        hit = self._hit(x_root, y_root)
        self.clear("new press")
        if hit is None:
            return False
        self._active, caret = hit
        self._anchor = self._focus = caret
        self._dragging = True
        log(f"[ui] selection begin line={caret.line} index={caret.index}")
        return True

    def extend(self, x_root: int, y_root: int) -> None:
        """拖曳中：更新 focus（夾在起手那一則內）並重畫反白。"""
        if not self._dragging:
            return
        self._focus = self._clamped(x_root, y_root)
        self._draw()

    def finish(self) -> None:
        """放開滑鼠。"""
        if not self._dragging:
            return
        self._dragging = False
        span = self.span()
        if span is None:
            log("[ui] selection end empty")
            return
        start, end = span
        log(f"[ui] selection end start={start.line}:{start.index} "
            f"end={end.line}:{end.index} chars={len(self.text())}")

    def clear(self, reason: str) -> None:
        """清除選取與反白。`reason` 只進 log，方便事後定位「選取莫名消失」。"""
        if self._active is None:
            return
        self._erase()
        self._active = self._anchor = self._focus = None
        self._dragging = False
        log(f"[ui] selection cleared ({reason})")

    def redraw(self) -> None:
        """換行寬度改變後依現有的 anchor／focus 重畫（字元索引不受換行影響）。"""
        self._draw()

    # --- 內部 ---
    def _font(self, canvas):
        if canvas not in self._fonts:
            self._fonts[canvas] = line_font(canvas)
        return self._fonts[canvas]

    def _caret_in(self, canvas, line: int, x_root: int, y_root: int) -> Caret:
        return Caret(line, caret_at(canvas, x_root - canvas.winfo_rootx(),
                                    y_root - canvas.winfo_rooty()))

    def _hit(self, x_root: int, y_root: int):
        """螢幕座標落在哪一則的哪個 caret；都沒命中回 None。"""
        for row, canvases in self._rows.items():
            for line, canvas in enumerate(canvases):
                if point_in_rect(x_root, y_root,
                                 canvas.winfo_rootx(), canvas.winfo_rooty(),
                                 canvas.winfo_width(), canvas.winfo_height()):
                    return row, self._caret_in(canvas, line, x_root, y_root)
        return None

    def _clamped(self, x_root: int, y_root: int) -> Caret:
        """拖曳點換算成作用中那一則的 caret。垂直方向決定落在哪一行、超出則夾到
        該則的頭或尾；水平方向交給 Tk 自己夾（見 `caret_at`）。"""
        canvases = self._rows[self._active]
        if y_root < canvases[0].winfo_rooty():
            return Caret(0, 0)
        for line, canvas in enumerate(canvases):
            if y_root <= canvas.winfo_rooty() + canvas.winfo_height() - 1:
                return self._caret_in(canvas, line, x_root, y_root)
        return Caret(len(canvases) - 1, len(line_text(canvases[-1])))

    def _erase(self) -> None:
        for canvas in self._rows.get(self._active, ()):
            canvas.delete("sel")

    def _draw(self) -> None:
        self._erase()
        span = self.span()
        if span is None:
            return
        start, end = span
        for line, canvas in enumerate(self._rows[self._active]):
            if line < start.line or line > end.line:
                continue
            lo = start.index if line == start.line else 0
            hi = end.index if line == end.line else len(line_text(canvas))
            rects = highlight_rects(canvas, self._font(canvas), lo, hi)
            for x0, y0, x1, y1 in rects:
                canvas.create_rectangle(x0, y0, x1, y1, fill=SELECT_BG,
                                        outline="", tags="sel")
            if rects:
                canvas.tag_lower("sel", "txt")
```

- [ ] **Step 4: 跑測試確認通過**

Run: `uv run pytest tests/test_selection.py -v`
Expected: PASS（全部，含 Task 1 的 10 個）

- [ ] **Step 5: Lint 與全測試**

Run: `uv run ruff check src tests && uv run pytest -q`
Expected: `All checks passed!` 且全部通過

- [ ] **Step 6: Commit**

```bash
git add src/ui/selection.py tests/test_selection.py
git commit -m "feat(ui): track and draw a selection clamped to one message"
```

---

### Task 3: 把 `Selection` 掛進 overlay（登記、失效邊界、重畫）

本 task **不碰滑鼠事件綁定**（那是 Task 4）；先讓選取的生命週期正確，測試直接呼叫 `ov._selection.begin/extend`。

**Files:**
- Modify: `src/ui/overlay.py`
- Test: `tests/test_overlay.py`

**Interfaces:**
- Consumes: Task 2 的 `Selection`
- Produces: `OverlayWindow._selection: Selection`、`OverlayWindow._drop_row(entry)`；`_fit_line_height` 改用 `bbox("txt")`

- [ ] **Step 1: 寫失敗的測試**

在 `tests/test_overlay.py` 末端追加（檔頭 import 補 `from src.ui.selection import TEXT_ORIGIN`）：

```python
def _select_whole_message(ov, index=0):
    """把第 index 則訊息整則選起來，回傳它的兩個行 canvas。"""
    ov._win.update_idletasks()
    first, second = ov._messages[index].row.winfo_children()
    ov._selection.begin(first.winfo_rootx() + TEXT_ORIGIN,
                        first.winfo_rooty() + first.winfo_height() // 2)
    ov._selection.extend(second.winfo_rootx() + 1000,
                         second.winfo_rooty() + second.winfo_height() // 2)
    return first, second


def test_prune_clears_a_selection_in_the_removed_row(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=60)
    ov.add_message("原文一", "譯文一", now=0.0)
    _select_whole_message(ov)
    assert ov._selection.active is True

    ov.prune(now=1000.0)

    assert ov._selection.active is False
    assert ov.visible_messages() == []


def test_max_messages_overflow_clears_a_selection_in_the_dropped_row(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=1, fade_seconds=0)
    ov.add_message("原文一", "譯文一")
    _select_whole_message(ov)

    ov.add_message("原文二", "譯文二")

    assert ov._selection.active is False


def test_update_message_clears_a_selection_in_that_row(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.add_message("原文一", "翻譯中…", msg_id=7, pending=True)
    _select_whole_message(ov)

    ov.update_message(7, "譯文一")

    assert ov._selection.active is False


def test_minimize_clears_the_selection(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.add_message("原文一", "譯文一")
    _select_whole_message(ov)

    ov.minimize()

    assert ov._selection.active is False
    ov.expand()


def test_resize_redraws_the_highlight_and_keeps_the_selection(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.add_message("原文一原文一原文一", "譯文一譯文一譯文一")
    first, _ = _select_whole_message(ov)
    selected = ov._selection.text()

    class FakeEvent:
        width = 240

    ov._on_canvas_configure(FakeEvent())
    ov._win.update_idletasks()

    assert ov._selection.text() == selected
    assert first.find_withtag("sel")


def test_line_height_ignores_the_highlight(root):
    # 反白矩形的底緣以 linespace 為準，可能比文字墨跡低一兩個像素；_fit_line_height
    # 若把它算進去，視窗每縮放一次列高就長高一點
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.add_message("原文一", "譯文一")
    first, _ = _select_whole_message(ov)
    ov._win.update_idletasks()
    before = first.winfo_reqheight()

    for _ in range(3):
        _fit_line_height(first)
    ov._win.update_idletasks()

    assert first.winfo_reqheight() == before
```

檔頭的 `from src.ui.overlay import (...)` 要補上 `_fit_line_height`。

- [ ] **Step 2: 跑測試確認失敗**

Run: `uv run pytest tests/test_overlay.py -k "selection or highlight or line_height or minimize_clears" -v`
Expected: FAIL — `AttributeError: 'OverlayWindow' object has no attribute '_selection'`

- [ ] **Step 3: 建立 `_selection` 並登記每一列**

`src/ui/overlay.py` 檔頭 import 補：

```python
from src.ui.selection import Selection
```

在 `_build_message_area` 的最後（`self._placeholder.place(...)` 之後）加上：

```python
        self._selection = Selection()
```

把 `add_message` 建列的三行改成保留參考並登記：

```python
        row = tk.Frame(self._inner, bg=BG)
        original_line = _outlined_line(row, original,
                                       dimmed(color) if color else FG_ORIGINAL,
                                       ui_font(9), self._wrap)
        original_line.pack(fill="x")
        translated_line = _outlined_line(row, translated,
                                         FG_PENDING if pending else (color or FG_TRANSLATED),
                                         ui_font(11), self._wrap)
        translated_line.pack(fill="x")
        row.pack(side="top", fill="x", pady=2)  # 最新在最下
        self._selection.register(row, (original_line, translated_line))
```

- [ ] **Step 4: 抽出 `_drop_row` 並換掉三處 destroy**

在 `add_message` 之前加入：

```python
    def _drop_row(self, entry: _Message) -> None:
        """移除一則訊息的畫面元件。銷毀列與解除選取登記必須成對——漏掉任一處，
        選取就會指向已銷毀的 widget。"""
        self._selection.forget(entry.row)
        entry.row.destroy()
```

把以下三處 `self._messages.pop(0).row.destroy()` ／ `entry.row.destroy()` 改掉：

- `add_message` 的 `while len(self._messages) > self._max:` → `self._drop_row(self._messages.pop(0))`
- `set_limits` 的同一個迴圈 → `self._drop_row(self._messages.pop(0))`
- `prune` 的 `entry.row.destroy()` → `self._drop_row(entry)`

- [ ] **Step 5: 補上其餘失效邊界與重畫**

`update_message` 中，在 `line.itemconfigure("txt", text=translated)` 之前插入：

```python
            if self._selection.holds(m.row):
                self._selection.clear("translated line replaced")
```

`minimize` 中，在 `self._win.update_idletasks()` 之前插入：

```python
        self._selection.clear("minimized to bubble")
```

`_on_canvas_configure` 中，在更新完各列 wrap 的迴圈之後、`if self._error_label is not None:` 之前插入：

```python
        self._selection.redraw()
```

- [ ] **Step 6: `_fit_line_height` 只認文字**

把 `_fit_line_height` 的 `bbox = c.bbox("all")` 改成：

```python
    bbox = c.bbox("txt")   # 只量文字：反白矩形的底緣比墨跡低，算進去會讓列高每次縮放都長一點
```

- [ ] **Step 7: 跑測試確認通過**

Run: `uv run pytest tests/test_overlay.py -v`
Expected: PASS（含既有測試；`test_resize_updates_existing_message_wraplength` 仍須綠燈——它遍歷 `row.winfo_children()` 並斷言 `itemcget("txt", "width")`，Step 3 沒有改變子元件的數量與順序）

- [ ] **Step 8: Lint 與全測試**

Run: `uv run ruff check src tests && uv run pytest -q`
Expected: `All checks passed!` 且全部通過

- [ ] **Step 9: Commit**

```bash
git add src/ui/overlay.py tests/test_overlay.py
git commit -m "feat(ui): give the overlay a selection and keep it in sync with its rows"
```

---

### Task 4: 滑鼠事件雙路由與焦點

**Files:**
- Modify: `src/ui/overlay.py`
- Test: `tests/test_overlay.py`

**Interfaces:**
- Consumes: Task 3 的 `self._selection`、既有的 `_edge_under`／`_resize_start`／`_refresh_scroll`
- Produces: `OverlayWindow._selection_press(e)`、`_selection_drag(e)`、`_selection_release(e)`、`_in_message_area(x_root, y_root)`、`_focus_for_copy()`；`_edge_press`／`_edge_drag`／`_edge_release` 在非縮放時改走選取

- [ ] **Step 1: 寫失敗的測試**

在 `tests/test_overlay.py` 追加：

```python
class _Press:
    """假的滑鼠事件（只用到螢幕座標）。"""

    def __init__(self, x_root, y_root):
        self.x_root = x_root
        self.y_root = y_root
        self.widget = None


def test_press_on_a_message_starts_a_selection(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.add_message("原文一", "譯文一")
    ov._win.update_idletasks()
    first, second = ov._messages[0].row.winfo_children()

    ov._selection_press(_Press(first.winfo_rootx() + TEXT_ORIGIN,
                               first.winfo_rooty() + first.winfo_height() // 2))
    ov._selection_drag(_Press(second.winfo_rootx() + 1000,
                              second.winfo_rooty() + second.winfo_height() // 2))
    ov._selection_release(_Press(0, 0))

    assert ov._selection.text() == "原文一\n譯文一"


def test_press_outside_the_message_area_clears_the_selection(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.add_message("原文一", "譯文一")
    _select_whole_message(ov)
    canvas = ov._canvas

    ov._selection_press(_Press(canvas.winfo_rootx() - 400, canvas.winfo_rooty() - 400))

    assert ov._selection.active is False


def test_backdrop_press_away_from_any_edge_starts_a_selection(root):
    # 訊息列的底色是透明色鍵，字間空隙的點擊會落到 backdrop——那條路徑也要能起手
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.add_message("原文一", "譯文一")
    ov._win.update_idletasks()
    first, second = ov._messages[0].row.winfo_children()

    ov._edge_press(_Press(first.winfo_rootx() + TEXT_ORIGIN,
                          first.winfo_rooty() + first.winfo_height() // 2))
    ov._edge_drag(_Press(second.winfo_rootx() + 1000,
                         second.winfo_rooty() + second.winfo_height() // 2))
    ov._edge_release(_Press(0, 0))

    assert ov._selection.text() == "原文一\n譯文一"
    assert ov._resize is None


@pytest.mark.real_position
def test_backdrop_press_on_an_edge_still_resizes(root):
    ov = OverlayWindow(root, x=200, y=200, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov._win.update_idletasks()

    ov._edge_press(_Press(200, 200 + 150))   # 左緣

    assert ov._resize is not None
    assert ov._selection.active is False
    ov._edge_release(_Press(200, 350))


def test_view_does_not_jump_to_the_bottom_while_selecting(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=50, fade_seconds=0)
    for i in range(20):
        ov.add_message(f"原文{i}", f"譯文{i}")
    ov._win.update_idletasks()
    first, second = ov._messages[0].row.winfo_children()
    ov._selection_press(_Press(first.winfo_rootx() + TEXT_ORIGIN,
                               first.winfo_rooty() + first.winfo_height() // 2))
    ov._canvas.yview_moveto(0.0)
    ov._win.update_idletasks()
    before = ov._canvas.yview()[0]

    ov._refresh_scroll()

    assert ov._canvas.yview()[0] == before
    ov._selection_release(_Press(0, 0))
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `uv run pytest tests/test_overlay.py -k "press or selecting" -v`
Expected: FAIL — `AttributeError: 'OverlayWindow' object has no attribute '_selection_press'`

- [ ] **Step 3: 加入選取事件處理**

`src/ui/overlay.py` 檔頭把 `point_in_rect` 加進 geometry 的 import：

```python
from src.ui.geometry import EDGE, edge_at, moved_to, point_in_rect, resized_edge
```

在 `_edge_release` 之後、`# --- 訊息 ---` 之前加入：

```python
    # --- 框選 ---
    def _in_message_area(self, x_root: int, y_root: int) -> bool:
        """螢幕座標是否落在可捲動的訊息視口內。捲出視野的列仍有幾何位置，不先擋一道
        的話，點在把手或標題列附近會選到看不見的訊息。"""
        c = self._canvas
        return point_in_rect(x_root, y_root, c.winfo_rootx(), c.winfo_rooty(),
                             c.winfo_width(), c.winfo_height())

    def _selection_press(self, e) -> None:
        """框選起手。訊息列的底色是本體的透明色鍵，只有文字墨跡接得到滑鼠、其餘落到
        backdrop，兩條路徑都導進這裡，一律用螢幕座標。"""
        if not self._in_message_area(e.x_root, e.y_root):
            self._selection.clear("press outside the message area")
            return
        if self._selection.begin(e.x_root, e.y_root):
            self._focus_for_copy()

    def _selection_drag(self, e) -> None:
        self._selection.extend(e.x_root, e.y_root)

    def _selection_release(self, e) -> None:
        self._selection.finish()

    def _focus_for_copy(self) -> None:
        """把鍵盤焦點交給本體，Ctrl+C 才收得到——backdrop 帶 WS_EX_NOACTIVATE，
        從它起手的選取不會給焦點。"""
        try:
            self._win.focus_force()
            log("[ui] selection took keyboard focus")
        except tk.TclError as exc:
            log(f"[ui] selection focus failed: {exc}")
```

- [ ] **Step 4: 綁到文字 canvas**

在 `add_message` 的 `self._selection.register(...)` 之後加入：

```python
        for line in (original_line, translated_line):
            line.bind("<ButtonPress-1>", self._selection_press)
            line.bind("<B1-Motion>", self._selection_drag)
            line.bind("<ButtonRelease-1>", self._selection_release)
```

- [ ] **Step 5: 讓 backdrop 的非縮放路徑走選取**

把 `_edge_press`／`_edge_drag`／`_edge_release` 改成：

```python
    def _edge_press(self, e) -> None:
        edge = self._edge_under(e)
        if edge:
            self._resize_start(e, edge)
        else:
            self._selection_press(e)   # 透明背景區的點擊落到這裡，交給框選

    def _edge_drag(self, e) -> None:
        if self._resize is None:
            self._selection_drag(e)   # 這次按下不在邊上：可能正在框選
            return
        sx, sy, ox, oy, ow, oh, edge = self._resize
        self._apply_geometry(*resized_edge(edge, ox, oy, ow, oh,
                                           e.x_root - sx, e.y_root - sy,
                                           MIN_WIDTH, MIN_HEIGHT))

    def _edge_release(self, e) -> None:
        if self._resize is None:
            self._selection_release(e)
            return
        edge = self._resize[6]
        self._resize = None
        log(f"[ui] overlay resize end edge={edge} geometry="
            f"{self._w}x{self._h}+{self._win.winfo_x()}+{self._win.winfo_y()}")
        self._emit_geometry()
```

- [ ] **Step 6: 拖曳期間不貼底**

`_refresh_scroll` 的貼底條件改成：

```python
        if self._follow and not self._selection.dragging:
            self._canvas.yview_moveto(1.0)
```

並把該方法 docstring 末尾補一句：

```
        框選拖曳期間不貼底：畫面被新訊息拉走的話，游標下的字會整個換掉。
```

- [ ] **Step 7: 跑測試確認通過**

Run: `uv run pytest tests/test_overlay.py -v`
Expected: PASS（含既有的縮放測試——`_bar_drag`／`_bar_release` 只在 `_resize is not None` 時才轉呼叫 `_edge_drag`／`_edge_release`，行為不變）

- [ ] **Step 8: Lint 與全測試**

Run: `uv run ruff check src tests && uv run pytest -q`
Expected: `All checks passed!` 且全部通過

- [ ] **Step 9: Commit**

```bash
git add src/ui/overlay.py tests/test_overlay.py
git commit -m "feat(ui): route selection drags from both the text lines and the backdrop"
```

---

### Task 5: 複製（`Ctrl+C`、右鍵選單、i18n、README）

**Files:**
- Modify: `src/ui/overlay.py`、`src/i18n/zh-TW.json`、`src/i18n/en.json`、`src/i18n/zh-CN.json`、`README.md`
- Test: `tests/test_overlay.py`

**Interfaces:**
- Consumes: Task 2 的 `Selection.text()`／`Selection.active`、既有的 `src/i18n:t`、`src/ui/fonts:ui_font`
- Produces: `OverlayWindow.copy_selection(_event=None) -> None`、`OverlayWindow._selection_menu(e)`、i18n key `menu.copy`

- [ ] **Step 1: 補三份語言檔**

三個檔案都在 `"app.name"` 那一行之後插入一行：

`src/i18n/zh-TW.json`：
```json
  "menu.copy": "複製",
```

`src/i18n/en.json`：
```json
  "menu.copy": "Copy",
```

`src/i18n/zh-CN.json`：
```json
  "menu.copy": "复制",
```

- [ ] **Step 2: 跑 i18n 測試確認三份一致**

Run: `uv run pytest tests/test_i18n.py -v`
Expected: PASS（`test_every_language_has_the_same_keys` 是這一步的守門員；漏了任何一份會轉紅）

- [ ] **Step 3: 寫失敗的測試**

在 `tests/test_overlay.py` 追加：

```python
def test_copy_writes_only_when_something_is_selected(root):
    # 兩個斷言刻意合成一個測試：剪貼簿是全機器共用的資源，拆成兩個測試在
    # pytest-xdist 的 4 個 worker 下會互相覆蓋（addopts 的 -n 4）
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.add_message("原文一", "譯文一")
    _select_whole_message(ov)

    ov.copy_selection()
    assert root.clipboard_get() == "原文一\n譯文一"

    ov._selection.clear("test")
    ov.copy_selection()
    assert root.clipboard_get() == "原文一\n譯文一", "沒有選取時不該動剪貼簿"


def test_right_click_without_a_selection_pops_no_menu(root, monkeypatch):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.add_message("原文一", "譯文一")
    monkeypatch.setattr(ov, "_build_selection_menu",
                        lambda: pytest.fail("沒有選取時不該建立選單"))

    ov._selection_menu(_Press(0, 0))


def test_right_click_menu_labels_follow_the_ui_language(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.add_message("原文一", "譯文一")
    _select_whole_message(ov)

    menu = ov._build_selection_menu()

    assert menu.entrycget(0, "label") == t("menu.copy")
    menu.destroy()
```

- [ ] **Step 4: 跑測試確認失敗**

Run: `uv run pytest tests/test_overlay.py -k "copy or right_click" -v`
Expected: FAIL — `AttributeError: 'OverlayWindow' object has no attribute 'copy_selection'`

（`test_right_click_without_a_selection_pops_no_menu` 會先因 `monkeypatch.setattr` 找不到 `_build_selection_menu` 而失敗，同樣是預期的紅燈。）

- [ ] **Step 5: 實作複製與右鍵選單**

在 Task 4 加入的 `_focus_for_copy` 之後加入：

```python
    def copy_selection(self, _event=None) -> None:
        """把目前選取的文字寫進系統剪貼簿。沒有選取就什麼都不做——寫入空字串會把
        使用者原本的剪貼簿內容清掉。"""
        text = self._selection.text()
        if not text:
            return
        self._win.clipboard_clear()
        self._win.clipboard_append(text)
        self._win.update()   # Windows 下要 flush 過，內容才真的落進系統剪貼簿
        log(f"[ui] copied selection chars={len(text)}")

    def _build_selection_menu(self) -> "tk.Menu":
        """右鍵選單。每次現建：語言一換文字就跟著換，不必另存狀態重繪。"""
        menu = tk.Menu(self._win, tearoff=0, font=ui_font(9))
        menu.add_command(label=t("menu.copy"), command=self.copy_selection)
        return menu

    def _selection_menu(self, e) -> None:
        """有選取時才彈出右鍵選單；沒選取就不彈，不做灰掉的空選單。"""
        if not self._selection.active:
            return
        menu = self._build_selection_menu()
        try:
            menu.tk_popup(e.x_root, e.y_root)
        finally:
            menu.grab_release()
```

- [ ] **Step 6: 綁定 `Ctrl+C` 與右鍵**

在 `_attach_to_shell` 的最後（`self._backdrop.lower(self._win)` 之後）加入：

```python
        # Caps Lock 開著時 Tk 送的是 <Control-C>，兩個都要接。不用 bind_all——
        # 那會連設定視窗的輸入框一起攔截。
        self._win.bind("<Control-c>", self.copy_selection)
        self._win.bind("<Control-C>", self.copy_selection)
        # 右鍵與左鍵一樣要雙路由：選取區以外的空白處按右鍵，事件會穿透到 backdrop
        self._backdrop.bind("<Button-3>", self._selection_menu)
```

在 `add_message` 的事件綁定迴圈裡補上右鍵：

```python
        for line in (original_line, translated_line):
            line.bind("<ButtonPress-1>", self._selection_press)
            line.bind("<B1-Motion>", self._selection_drag)
            line.bind("<ButtonRelease-1>", self._selection_release)
            line.bind("<Button-3>", self._selection_menu)
```

`tests/test_overlay.py` 檔頭若尚未 import `t`，補上 `from src.i18n import t`（既有檔案已有）。

- [ ] **Step 7: 跑測試確認通過**

Run: `uv run pytest tests/test_overlay.py -v`
Expected: PASS

- [ ] **Step 8: 更新 README**

`README.md` 第 153-155 行那組「可互動」說明，在「注意：視窗不滑鼠穿透」那一行之前插入一個項目：

```markdown
   - 可複製：**在訊息上按住左鍵拖曳選字**（可跨同一則的原文行與譯文行，不跨訊息），
     再按 `Ctrl+C` 或在選取處按右鍵選「複製」
```

- [ ] **Step 9: Lint 與全測試**

Run: `uv run ruff check src tests && uv run pytest -q`
Expected: `All checks passed!` 且全部通過

- [ ] **Step 10: Commit**

```bash
git add src/ui/overlay.py src/i18n/zh-TW.json src/i18n/en.json src/i18n/zh-CN.json README.md tests/test_overlay.py
git commit -m "feat(ui): copy the selected message text with Ctrl+C or the context menu"
```

---

### Task 6: 實機驗證

**Files:** 無（只驗證，發現問題才回頭修）

- [ ] **Step 1: 啟動**

開啟 Wizard101 並**登入進遊戲世界內**，然後 `uv run run.py`。

- [ ] **Step 2: 逐項確認**

1. 在文字墨跡上按下拖曳可選字，反白即時跟隨。
2. **從字間空隙／行尾起手也能選**（backdrop 路由）。
3. **從空隙起手、拖過墨跡、再回到空隙，選取不中斷**（跨 Toplevel 的 implicit grab）。
4. 由原文行拖到譯文行可連續選取。
5. 拖出該則的上下界會夾住，不會選到別則。
6. `Ctrl+C` 有效（遊戲全螢幕時也要試一次）。
7. 右鍵選單「複製」有效，且在空白處按右鍵也彈得出來。
8. 選取期間新訊息進來不打斷選取、畫面不被拉走。
9. 縮放視窗後反白仍貼齊文字。
10. 四邊縮放、右下把手、標題列拖曳、滾輪捲動全部維持原狀。

- [ ] **Step 3: 檢查 log**

匯出 exe 旁（或專案根目錄）的 `app.log`，確認 `[ui] selection begin`／`selection end`／`copied selection chars=`／`selection cleared (...)` 都有出現，且**沒有任何一行帶著聊天內容**。

- [ ] **Step 4: 若第 6 項失敗**

`Ctrl+C` 收不到代表 `focus_force()` 在遊戲全螢幕下被擋。**先看 `app.log` 有沒有 `selection focus failed`**，再回報；不要直接改用全域鍵盤 hook 攔 `Ctrl+C`（那會攔到整個系統的複製）。右鍵選單是這個情況下的保底路徑，功能不算失敗。

---

## 自我檢查結果

**Spec 覆蓋**：spec 的五節皆有對應 task——選取模型（Task 1、2）、事件路由與生命週期（Task 3、4）、複製（Task 5）、介面文字與 log（Task 5 與各 task 的 log 步驟）、測試策略（各 task 的測試步驟 ＋ Task 6 實機）。spec 列出的三個「連帶要改」（`_fit_line_height` 改 `bbox("txt")`、`_drop_row` 抽出、`_refresh_scroll` 拖曳暫停貼底）分別落在 Task 3 Step 6、Task 3 Step 4、Task 4 Step 6。

**與 spec 的一處簡化**：spec 寫「右鍵選單⋯⋯語言切換時由既有的 `refresh_labels()` 重建」。計畫改為 `_build_selection_menu()` 每次現建，語言一換文字自然跟著換，`refresh_labels` 不必動、也不必多存一個 menu 狀態。達成的效果與 spec 相同而狀態更少。

**Self-review 抓到並已修正的三件事**：

1. **`_hit` 會命中捲出視野的列**。捲動視口之外的訊息列仍有幾何位置，點在右下把手或標題列附近可能選到看不見的訊息。已加入 `_in_message_area()` 先擋一道（Task 4 Step 3），並在 `_selection_press` 最前面呼叫。
2. **`_clamped` 原本只判斷「在不在任一行內」**，游標水平方向滑出訊息列（但垂直仍在行內）時會被誤判成「拖出該則」而直接跳到結尾。已改成**先用垂直位置決定落在哪一行、水平方向交給 Tk 自己夾**（Task 2 Step 3 的 `_clamped`）。
3. **`begin` 的清除順序**。原本先設 `_active` 再清除，會把新列的反白一起擦掉。已改成先 `clear("new press")` 再設定新的 `_active`／`_anchor`（Task 2 Step 3）。
4. **剪貼簿測試在 `-n 4` 下會互相覆蓋**。剪貼簿是全機器共用資源，原本拆成「有選取」與「無選取」兩個測試，落在不同 worker 時會彼此清掉。已合併成單一測試，兩段斷言在同一個 worker 內依序執行（Task 5 Step 3）。
5. **右鍵選單的測試原本 monkeypatch 掉整個 `tk.Menu`**（`src.ui.overlay.tk` 就是 tkinter 本體，等於全域替換）。已改為 patch `OverlayWindow._build_selection_menu` 這個實例方法，範圍剛好、意圖也直接。

**型別一致性**：`Caret(line, index)`、`Selection.register/forget/holds/begin/extend/finish/clear/redraw/span/text/active/dragging`、`caret_at/visual_lines/highlight_rects/selected_text/line_text/line_font`、`TEXT_ORIGIN`、`SELECT_BG`、`OverlayWindow._drop_row/_selection_press/_selection_drag/_selection_release/_in_message_area/_focus_for_copy/copy_selection/_build_selection_menu/_selection_menu` 在各 task 之間的名稱與簽名已逐一核對一致。
