"""richtext：文字中的網址可點擊的標籤元件，與裸網址轉連結語法的純函式。"""
import pytest

from src.ui.richtext import linkify


@pytest.mark.parametrize("text,expected", [
    ("see https://a.example/x for help",
     "see [https://a.example/x](https://a.example/x) for help"),
    ("key at https://a.example/keys.", "key at [https://a.example/keys](https://a.example/keys)."),
    ("(https://a.example/p), then", "([https://a.example/p](https://a.example/p)), then"),
    ("no links here", "no links here"),
    ("ftp://a.example/f", "ftp://a.example/f"),
    ("[docs](https://a.example/d) stays", "[docs](https://a.example/d) stays"),
    # 中文緊接在網址後（沒有空白）：全形標點與 CJK 都不是網址的一部分
    ("至 https://a.example/keys，請檢查", "至 [https://a.example/keys](https://a.example/keys)，請檢查"),
    ("https://a.example/k。", "[https://a.example/k](https://a.example/k)。"),
])
def test_linkify_wraps_bare_urls_in_link_markup(text, expected):
    assert linkify(text) == expected


import tkinter as tk  # noqa: E402
from tkinter import font as tkfont  # noqa: E402

from src.ui import richtext  # noqa: E402
from src.ui.richtext import RichLabel  # noqa: E402


def _label(root, width=240):
    win = tk.Toplevel(root)
    win.geometry(f"{width}x120+0+0")
    label = RichLabel(win, fg="#cc3333", bg="#ffffff", font=("Segoe UI", 9))
    label.pack(fill="x")
    win.update()
    return win, label


def test_rich_label_shows_plain_text_and_reports_it_back(root):
    win, label = _label(root)
    try:
        label.set("✗ HTTP 401: bad key")
        assert label.text() == "✗ HTTP 401: bad key"
        assert label.links() == []
        label.set("")
        assert label.text() == ""
    finally:
        win.destroy()


def test_rich_label_turns_bare_urls_into_link_segments(root):
    win, label = _label(root)
    try:
        label.set("Get a key at https://a.example/keys. Then retry.")
        assert label.text() == "Get a key at https://a.example/keys. Then retry."
        assert label.links() == [("https://a.example/keys", "https://a.example/keys")]
    finally:
        win.destroy()


def test_rich_label_grows_to_fit_wrapped_lines(root):
    win, label = _label(root, width=200)
    try:
        label.set("short")
        win.update()
        one = int(label.cget("height"))
        label.set("a fairly long error message that certainly needs more than one line "
                  "at two hundred pixels wide")
        win.update()
        assert int(label.cget("height")) > one
    finally:
        win.destroy()


def test_rich_label_click_on_link_opens_browser(root, monkeypatch):
    opened = []
    monkeypatch.setattr(richtext.webbrowser, "open", lambda url: opened.append(url))
    win, label = _label(root)
    try:
        label.set("see https://a.example/help now")
        win.update()
        start = label.link_ranges()[0][0]
        x, y, w, h = label.bbox(start)
        # Tk 的 tag 綁定看的是 current 標記，它只在指標移動時更新：先送 Motion 再按
        label.event_generate("<Motion>", x=x + w // 2, y=y + h // 2)
        label.event_generate("<Button-1>", x=x + w // 2, y=y + h // 2)
        assert opened == ["https://a.example/help"]
    finally:
        win.destroy()


def test_rich_label_fits_a_long_unbreakable_word(root):
    # Tk 對超過一行寬的「字」會逐字元折行，但 count displaylines 少算一行，
    # 用它定高度會把最後一行切掉（實測：長網址接中文）
    win, label = _label(root, width=200)
    try:
        label.set("see https://a.example/" + "verylongpath/" * 8 + "end，請開啟設定檢查")
        win.update()
        assert label.dlineinfo("end-1c") is not None   # 最後一個字真的畫在可見範圍內
    finally:
        win.destroy()


def test_rich_label_reports_height_changes(root):
    win = tk.Toplevel(root)
    win.geometry("200x120+0+0")
    changes = []
    label = RichLabel(win, fg="#cc3333", bg="#ffffff", font=("Segoe UI", 9),
                      on_height_change=lambda: changes.append(int(label.cget("height"))))
    label.pack(fill="x")
    win.update()
    try:
        label.set("a fairly long error message that certainly needs more than one line "
                  "at two hundred pixels wide")
        win.update()
        assert changes and changes[-1] > 1
    finally:
        win.destroy()


def test_rich_label_set_blocks_joins_blocks_with_a_blank_line(root):
    win, label = _label(root)
    try:
        label.set_blocks([("A", "#111111", ("Segoe UI", 9)), ("B", "#222222", ("Segoe UI", 11))])
        assert label.get("1.0", "end-1c") == "A\n\nB"
    finally:
        win.destroy()


def test_rich_label_set_blocks_colours_each_block(root):
    win, label = _label(root)
    try:
        label.set_blocks([("A", "#111111", ("Segoe UI", 9)), ("B", "#222222", ("Segoe UI", 11))])
        tags = label.tag_names("1.0")
        block_tag = next(name for name in tags if name.startswith("block"))
        assert label.tag_cget(block_tag, "foreground") == "#111111"
    finally:
        win.destroy()


def test_rich_label_set_blocks_keeps_links_clickable(root):
    win, label = _label(root)
    try:
        label.set_blocks([
            ("see https://a.example/help now", "#111111", ("Segoe UI", 9)),
            ("plain second block", "#222222", ("Segoe UI", 11)),
        ])
        assert label.links() == [("https://a.example/help", "https://a.example/help")]
    finally:
        win.destroy()


def test_rich_label_set_before_layout_never_balloons(root):
    # 先 set 再 pack（overlay 橫幅的順序）：第一次量高度時寬度還是佔位的 1px，
    # 長句被算成一字一行，橫幅先撐滿整個視窗再縮回幾行 —— 訊息列表因此閃一下（實測）
    win = tk.Toplevel(root)
    win.geometry("460x300+0+0")
    win.update()
    label = RichLabel(win, fg="#cc3333", bg="#ffffff", font=("Segoe UI", 9))
    requested = []
    label.bind("<Configure>", lambda e: requested.append(int(label.cget("height"))), add=True)
    try:
        label.set("Service temporarily unavailable, please retry later at "
                  "https://status.example.com/incidents/12345 and check the settings")
        label.pack(fill="x")
        win.update()
        final = int(label.cget("height"))
        assert 1 < final < 6
        assert max(requested) == final, requested   # 從未要求過比最終行數更高的高度
    finally:
        win.destroy()


def test_rich_label_natural_width_is_the_widest_line_unwrapped(root):
    win, label = _label(root, width=60)   # 窄到一定會換行：自然寬度不該受換行影響
    try:
        label.set("短\n這是一行比較長的文字")
        font = tkfont.Font(root=root, font=("Segoe UI", 9))
        assert label.natural_width() == font.measure("這是一行比較長的文字")
    finally:
        win.destroy()


def test_rich_label_natural_width_measures_each_block_with_its_own_font(root):
    win, label = _label(root, width=60)
    try:
        label.set_blocks([("同樣的字", "#111111", ("Segoe UI", 9)),
                          ("同樣的字", "#222222", ("Segoe UI", 14))])
        big = tkfont.Font(root=root, font=("Segoe UI", 14))
        assert label.natural_width() == big.measure("同樣的字")
    finally:
        win.destroy()


def test_rich_label_natural_width_counts_the_link_label_not_the_markup(root):
    win, label = _label(root, width=60)
    try:
        label.set("[點我](https://a.example/very/long/path)")
        font = tkfont.Font(root=root, font=("Segoe UI", 9))
        assert label.natural_width() == font.measure("點我")
    finally:
        win.destroy()
