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
