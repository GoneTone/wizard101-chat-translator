"""設定視窗純邏輯與 overlay set_limits／set_alpha 測試。"""
import copy
import tkinter as tk

from src.reader.overlay import OverlayWindow
from src.config import clamp_advanced
from src.ui.settings import parse_advanced_values


def test_clamp_advanced_limits_ranges():
    v = clamp_advanced({"poll_interval": 0.01, "fade_seconds": -5,
                        "max_messages": 99999, "type_delay": 9.0,
                        "overlay_alpha": 0.01, "max_parallel_translations": 99})
    assert v == {"poll_interval": 0.1, "fade_seconds": 0,
                 "max_messages": 1000, "type_delay": 0.5,
                 "overlay_alpha": 0.3, "max_parallel_translations": 8}


def test_clamp_advanced_passes_valid_values():
    v = {"poll_interval": 0.4, "fade_seconds": 0, "max_messages": 200,
         "type_delay": 0.02, "overlay_alpha": 0.84, "max_parallel_translations": 4}
    assert clamp_advanced(dict(v)) == v


def test_overlay_set_limits_trims_messages(root):
    ov = OverlayWindow(root, x=0, y=0, max_messages=5, fade_seconds=0)
    for i in range(5):
        ov.add_message(f"o{i}", f"t{i}", now=100.0)
    ov.set_limits(max_messages=3, fade_seconds=0)
    assert len(ov.visible_messages()) == 3
    assert ov.visible_messages()[0] == ("o2", "t2")  # 移除最舊


def test_overlay_set_alpha_applies_to_backdrop_and_bubble(root):
    # 雙層視窗：透明度只套在底板（backdrop）與泡泡，文字層（_win）維持不透明
    ov = OverlayWindow(root, x=0, y=0, max_messages=5, fade_seconds=0, alpha=0.84)
    ov.minimize()
    ov.set_alpha(0.5)
    assert float(ov._backdrop.attributes("-alpha")) == 0.5
    assert float(ov._win.attributes("-alpha")) == 1.0
    assert float(ov._bubble.attributes("-alpha")) == 0.5
    ov.expand()


def test_overlay_backdrop_follows_geometry(root):
    ov = OverlayWindow(root, x=30, y=40, width=460, height=300,
                       max_messages=5, fade_seconds=0)
    ov._apply_geometry(120, 90, 400, 250)
    root.update()
    assert (ov._backdrop.winfo_x(), ov._backdrop.winfo_y()) == (120, 90)
    assert (ov._backdrop.winfo_width(), ov._backdrop.winfo_height()) == (400, 250)


def _vars(root, poll=0.4, fade=0, max_msgs=200, type_delay=0.02, alpha=0.84, parallel=4):
    return (tk.DoubleVar(root, value=poll), tk.IntVar(root, value=fade),
            tk.IntVar(root, value=max_msgs), tk.DoubleVar(root, value=type_delay),
            tk.DoubleVar(root, value=alpha), tk.IntVar(root, value=parallel))


def test_parse_advanced_values_returns_error_on_non_numeric_input(root):
    poll, fade, max_msgs, type_delay, alpha, parallel = _vars(root)
    poll.set("abc")  # 模擬使用者在 Spinbox 手動鍵入非數字

    values, error = parse_advanced_values(poll, fade, max_msgs, type_delay, alpha, parallel)

    assert values is None
    assert error == "error.advanced_not_number"


def test_parse_advanced_values_clamps_valid_numeric_input(root):
    poll, fade, max_msgs, type_delay, alpha, parallel = _vars(
        root, poll=0.01, fade=-5, max_msgs=99999, type_delay=9.0, alpha=2.0)

    values, error = parse_advanced_values(poll, fade, max_msgs, type_delay, alpha, parallel)

    assert error is None
    assert values == {"poll_interval": 0.1, "fade_seconds": 0,
                      "max_messages": 1000, "type_delay": 0.5,
                      "overlay_alpha": 1.0, "max_parallel_translations": 4}


def test_parse_advanced_values_clamps_parallel(root):
    poll, fade, max_msgs, type_delay, alpha, parallel = _vars(root, parallel=99)
    values, error = parse_advanced_values(poll, fade, max_msgs, type_delay, alpha, parallel)
    assert error is None
    assert values["max_parallel_translations"] == 8


def test_parse_advanced_values_returns_error_key():
    import tkinter as tk

    from src.ui.settings import parse_advanced_values

    class BadVar:
        def get(self):
            raise ValueError("not a number")

    values, error = parse_advanced_values(BadVar(), BadVar(), BadVar(), BadVar(),
                                          BadVar(), BadVar())
    assert values is None
    assert error == "error.advanced_not_number"


def test_save_applies_ui_language(root, tmp_path):
    from src import i18n
    from src.config import DEFAULT_CONFIG
    from src.ui.settings import SettingsWindow

    before = i18n.current_language()
    try:
        i18n.set_language("zh-TW")
        cfg = copy.deepcopy(DEFAULT_CONFIG)
        cfg["api"] = {"provider": "custom", "base_url": "http://x", "model": "m",
                      "api_key": "", "thinking": False}
        saved = []
        win = SettingsWindow(root, cfg,
                             on_save=lambda: saved.append(i18n.current_language()))
        win.open()
        win._ui_language.set_value("en")
        win._save()
        assert cfg["ui_language"] == "en"
        assert i18n.current_language() == "en"
        # on_save 看到的必須已經是新語言：這行是本測試的重點，
        # 若 _save() 把 on_save 提前到套用語言之前，這裡會是 "zh-TW"。
        assert saved == ["en"]
    finally:
        i18n.set_language(before)


def _open_settings(root, on_language_preview=None):
    from src.config import DEFAULT_CONFIG
    from src.i18n import current_language
    from src.ui.settings import SettingsWindow

    cfg = copy.deepcopy(DEFAULT_CONFIG)
    cfg["api"] = {"provider": "custom", "base_url": "http://x", "model": "m",
                  "api_key": "", "thinking": False}
    cfg["ui_language"] = current_language()
    win = SettingsWindow(root, cfg, on_save=lambda: None,
                         on_language_preview=on_language_preview)
    win.open()
    return win


def test_changing_target_language_clears_the_test_result(root):
    # 測試連線顯示的譯文是用當時的目標語言翻出來的，語言一改那句就過期了——
    # 留著會讓使用者以為新語言已經驗證過。
    win = _open_settings(root)
    win._api._show_test_result(True, "connected, sample translation")
    assert win._api.test_passed
    win._language.set_value("English")
    assert not win._api.test_passed
    assert win._api._test_result.cget("text") == ""
    win._win.destroy()


def test_both_language_fields_come_before_the_api_section(root):
    # 介面語言與翻譯目標語言是最容易被搞混的一對，要相鄰且排在 API 設定之前
    win = _open_settings(root)
    basic = win._ui_language.master
    order = [str(w) for w in basic.pack_slaves()]
    ui_at = order.index(str(win._ui_language))
    target_at = order.index(str(win._language))
    api_at = order.index(str(win._api))
    assert ui_at < target_at < api_at, f"版面順序不對：{order}"
    win._win.destroy()


def test_changing_ui_language_previews_it_without_touching_config(root):
    # 切換介面語言＝預覽：設定視窗以新語言重建、常駐介面跟著換，但 cfg 尚未寫入
    from src import i18n
    from src.config import app_name
    from src.i18n import t

    before = i18n.current_language()
    try:
        i18n.set_language("zh-TW")
        relabelled = []
        win = _open_settings(root,
                             on_language_preview=lambda: relabelled.append(
                                 i18n.current_language()))
        old_win = win._win
        win._on_language_change("en")
        root.update()

        assert i18n.current_language() == "en"
        assert relabelled == ["en"]
        assert win._cfg["ui_language"] == "zh-TW"
        assert not old_win.winfo_exists()
        assert win._win.title() == t("settings.title", app=app_name())
        win._win.destroy()
    finally:
        i18n.set_language(before)


def test_language_preview_keeps_unsaved_edits(root):
    from src import i18n

    before = i18n.current_language()
    try:
        i18n.set_language("zh-TW")
        win = _open_settings(root)
        win._hotkey.set_value("ctrl+alt+k")
        win._game_path.set(r"D:\Games\Wizard101")
        win._language.set_value("日本語")
        win._max_msgs.set(321)
        win._auto_input.set(False)

        win._on_language_change("en")
        root.update()

        assert win._hotkey.value() == "ctrl+alt+k"
        assert win._game_path.get() == r"D:\Games\Wizard101"
        assert win._language.value() == "日本語"
        assert win._max_msgs.get() == 321
        assert win._auto_input.get() is False
        win._win.destroy()
    finally:
        i18n.set_language(before)


def test_language_preview_keeps_the_current_tab(root):
    # 在「進階」分頁換語言不該被彈回「基本」
    from src import i18n

    before = i18n.current_language()
    try:
        i18n.set_language("zh-TW")
        win = _open_settings(root)
        win._nb.select(1)
        win._on_language_change("en")
        root.update()

        assert win._nb.index("current") == 1
        win._win.destroy()
    finally:
        i18n.set_language(before)


def test_cancel_restores_the_previewed_language(root):
    from src import i18n

    before = i18n.current_language()
    try:
        i18n.set_language("zh-TW")
        relabelled = []
        win = _open_settings(root,
                             on_language_preview=lambda: relabelled.append(
                                 i18n.current_language()))
        win._on_language_change("en")
        root.update()
        win._cancel()

        assert i18n.current_language() == "zh-TW"
        assert relabelled == ["en", "zh-TW"]
        assert win._cfg["ui_language"] == "zh-TW"
    finally:
        i18n.set_language(before)


def test_save_keeps_the_previewed_language(root):
    from src import i18n

    before = i18n.current_language()
    try:
        i18n.set_language("zh-TW")
        win = _open_settings(root)
        win._on_language_change("en")
        root.update()
        win._save()

        assert i18n.current_language() == "en"
        assert win._cfg["ui_language"] == "en"
    finally:
        i18n.set_language(before)
