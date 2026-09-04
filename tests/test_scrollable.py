"""捲動容器與各視窗的版面順序。

版面順序是這裡的重點：tkinter 的 pack 依宣告順序分配空間，expand=True 的內容區
若排在固定高度的邊條之前，視窗一變矮就會把邊條擠掉（按鈕列、導覽列、錯誤橫幅）。
順序是可以直接斷言的，所以把它釘起來。
"""
import tkinter as tk
from tkinter import ttk

from src.ui.scrollable import ScrollableFrame


def _pump(widget) -> None:
    """讓 tkinter 完成排版：捲軸的出現與否要等幾何算完才成立。"""
    widget.update_idletasks()
    widget.update()


def test_body_is_where_content_goes(root):
    box = ScrollableFrame(root)
    label = ttk.Label(box.body, text="hi")
    label.pack()
    assert label.winfo_parent() == str(box.body)


def test_scrollbar_packs_before_the_canvas(root):
    # 捲軸若排在 expand=True 的 canvas 之後，就會被 canvas 吃光寬度而擠不進來
    box = ScrollableFrame(root)
    names = [str(w) for w in box.pack_slaves()]
    assert names.index(str(box._scrollbar)) < names.index(str(box._canvas))
    box.destroy()


def _wheel(delta: int):
    return type("Wheel", (), {"delta": delta})()


def test_wheel_does_nothing_when_content_fits(root):
    box = ScrollableFrame(root)
    scrolled = []
    box._canvas.yview = lambda: (0.0, 1.0)          # 內容完整可見
    box._canvas.yview_scroll = lambda *args: scrolled.append(args)
    box._on_wheel(_wheel(-120))
    assert scrolled == []
    box.destroy()


def test_wheel_scrolls_when_content_overflows(root):
    box = ScrollableFrame(root)
    scrolled = []
    box._canvas.yview = lambda: (0.0, 0.4)          # 只看得到四成
    box._canvas.yview_scroll = lambda *args: scrolled.append(args)
    box._on_wheel(_wheel(-120))
    assert scrolled == [(1, "units")]
    box.destroy()


def test_inner_width_follows_canvas(root):
    # 內層寬度沒跟著 canvas 走的話，bind_wrap 算出的換行寬度會失準、說明文字水平溢出。
    # 同樣不依賴真實幾何：直接送一個 Configure 事件進去。
    class _Configure:
        width = 480

    box = ScrollableFrame(root)
    box._on_canvas_configure(_Configure())
    assert int(box._canvas.itemcget(box._body_id, "width")) == 480
    box.destroy()


def _slave_order(win) -> list[str]:
    return [str(w) for w in win.pack_slaves()]


def test_settings_button_row_is_packed_before_the_notebook(root, tmp_path):
    import copy

    from src.config import DEFAULT_CONFIG
    from src.ui.settings import SettingsWindow

    cfg = copy.deepcopy(DEFAULT_CONFIG)
    cfg["api"]["provider"] = "custom"
    cfg["api"]["custom"].update(base_url="http://x", model="m")
    win = SettingsWindow(root, cfg, on_save=lambda: None)
    win.open()
    order = _slave_order(win._win)
    notebook = [n for n in order if "notebook" in n.lower()]
    assert notebook, f"找不到 Notebook，實際版面：{order}"
    # 按鈕列（含版號 Label 與兩個 Button）必須排在 Notebook 之前
    assert order.index(notebook[0]) == len(order) - 1, (
        f"Notebook 應該最後 pack，實際版面：{order}")
    win._win.destroy()


def test_wizard_nav_is_packed_before_the_body(root):
    import copy

    from src.config import DEFAULT_CONFIG
    from src.ui.wizard import SetupWizard

    cfg = copy.deepcopy(DEFAULT_CONFIG)
    wizard = SetupWizard(root, cfg)
    order = _slave_order(wizard._win)
    assert order.index(str(wizard._body_scroll)) == len(order) - 1, (
        f"內容區應該最後 pack，實際版面：{order}")
    wizard._win.destroy()


def test_overlay_error_banner_is_packed_before_the_scroll_area(root):
    from src.ui.overlay import OverlayWindow

    ov = OverlayWindow(root, x=0, y=0, width=460, height=300)
    ov.set_error("notice.offline")
    order = _slave_order(ov._frame)
    assert order.index(str(ov._error_label)) < order.index(str(ov._scroll_area)), (
        f"錯誤橫幅應排在捲動區之前，實際版面：{order}")
    ov.clear_error()


def test_overlay_title_is_packed_after_the_bar_controls(root):
    # overlay 是無邊框視窗，✕ 是唯一的正常關閉途徑：標題（英文較長）先 pack 的話，
    # 視窗一縮窄就會把 ✕／⚙／─ 擠出畫面，程式再也關不掉。
    from src.ui.overlay import OverlayWindow

    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       on_close=lambda: None, on_settings=lambda: None)
    bar = ov._title_label.master
    order = _slave_order(bar)
    assert order.index(str(ov._title_label)) == len(order) - 1, (
        f"標題應該最後 pack，實際版面：{order}")


def test_secret_entry_reveal_button_is_packed_before_the_entry(root):
    import copy

    from src.config import DEFAULT_CONFIG
    from src.ui.fields import ApiFields

    api = copy.deepcopy(DEFAULT_CONFIG["api"])
    api["openai"].update(api_key="k", model="m")
    fields = ApiFields(root, api)
    rows = [w for w in fields._fields.pack_slaves() if w.pack_slaves()]
    reveal_rows = [r for r in rows
                   if any(isinstance(c, ttk.Button) for c in r.pack_slaves())]
    assert reveal_rows, "找不到帶「顯示」按鈕的金鑰欄位列"
    row = reveal_rows[0]
    order = _slave_order(row)
    button = [str(c) for c in row.pack_slaves() if isinstance(c, ttk.Button)][0]
    entry = [str(c) for c in row.pack_slaves() if isinstance(c, ttk.Entry)][0]
    assert order.index(button) < order.index(entry), (
        f"「顯示」按鈕應排在輸入框之前，實際版面：{order}")


def test_game_path_browse_button_is_packed_before_the_entry(root):
    import copy

    from src.config import DEFAULT_CONFIG
    from src.ui.settings import SettingsWindow

    cfg = copy.deepcopy(DEFAULT_CONFIG)
    cfg["api"]["provider"] = "custom"
    cfg["api"]["custom"].update(base_url="http://x", model="m")
    win = SettingsWindow(root, cfg, on_save=lambda: None)
    win.open()
    path_row = win._game_path_row
    order = _slave_order(path_row)
    button = [str(c) for c in path_row.pack_slaves() if isinstance(c, ttk.Button)][0]
    entry = [str(c) for c in path_row.pack_slaves() if isinstance(c, ttk.Entry)][0]
    assert order.index(button) < order.index(entry), (
        f"「瀏覽…」按鈕應排在輸入框之前，實際版面：{order}")
    win._win.destroy()
