"""設定視窗純邏輯與 overlay set_limits／set_alpha 測試。"""
import copy
import tkinter as tk

import pytest

from src.config import clamp_advanced
from src.ui.bubble import BUBBLE_ALPHA_FACTOR, BUBBLE_ALPHA_FLOOR
from src.ui.overlay import OverlayWindow
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
    # 泡泡刻意比主視窗再透一些——它是收起來的浮標，該更低調，所以不是同一個值
    assert float(ov._bubble.attributes("-alpha")) == pytest.approx(0.5 * BUBBLE_ALPHA_FACTOR)
    ov.expand()


def test_bubble_alpha_stops_at_the_floor(root):
    # overlay_alpha 已經是最低時再打折會讓泡泡幾乎看不見，找不回來——所以有下限
    ov = OverlayWindow(root, x=0, y=0, max_messages=5, fade_seconds=0, alpha=0.3)
    ov.minimize()
    assert float(ov._bubble.attributes("-alpha")) == pytest.approx(BUBBLE_ALPHA_FLOOR)
    ov.expand()


@pytest.mark.real_position
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
        cfg["api"]["provider"] = "custom"
        cfg["api"]["custom"].update(base_url="http://x", model="m")
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


def test_save_stores_the_system_message_toggle(root):
    from src.config import DEFAULT_CONFIG
    from src.ui.settings import SettingsWindow

    cfg = copy.deepcopy(DEFAULT_CONFIG)
    cfg["api"]["provider"] = "custom"
    cfg["api"]["custom"].update(base_url="http://x", model="m")
    win = SettingsWindow(root, cfg, on_save=lambda: None)
    win.open()
    assert win._translate_system.get() is False   # 預設關閉
    win._translate_system.set(True)
    win._save()
    assert cfg["translate_system_messages"] is True


def test_reopening_settings_reflects_the_saved_toggle(root):
    from src.config import DEFAULT_CONFIG
    from src.ui.settings import SettingsWindow

    cfg = copy.deepcopy(DEFAULT_CONFIG)
    cfg["api"]["provider"] = "custom"
    cfg["api"]["custom"].update(base_url="http://x", model="m")
    cfg["translate_system_messages"] = True
    win = SettingsWindow(root, cfg, on_save=lambda: None)
    win.open()
    assert win._translate_system.get() is True


def _open_settings(root, on_language_preview=None):
    from src.config import DEFAULT_CONFIG
    from src.i18n import current_language
    from src.ui.settings import SettingsWindow

    cfg = copy.deepcopy(DEFAULT_CONFIG)
    cfg["api"]["provider"] = "custom"
    cfg["api"]["custom"].update(base_url="http://x", model="m")
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


def _open_settings_with_checker(root, checker):
    from src.config import DEFAULT_CONFIG
    from src.i18n import current_language
    from src.ui.settings import SettingsWindow

    cfg = copy.deepcopy(DEFAULT_CONFIG)
    cfg["api"]["provider"] = "custom"
    cfg["api"]["custom"].update(base_url="http://x", model="m")
    cfg["ui_language"] = current_language()
    win = SettingsWindow(root, cfg, on_save=lambda: None, check_update=checker)
    win.open()
    return win


def test_about_tab_shows_version_and_links(root, monkeypatch):
    from src import __version__
    from src.i18n import t
    from src.ui import settings as settings_module
    from src.updater import AUTHOR_URL, PROJECT_URL

    opened = []
    monkeypatch.setattr(settings_module.webbrowser, "open", opened.append)
    win = _open_settings_with_checker(root, lambda: None)
    tabs = [win._nb.tab(i, "text") for i in range(win._nb.index("end"))]
    assert tabs[2] == t("settings.tab.about")
    assert win._version_label.cget("text") == f"v{__version__}"
    assert win._project_link.cget("text") == PROJECT_URL
    assert win._author_link.cget("text") == "GoneTone"

    win._author_link.event_generate("<Button-1>")
    root.update()
    assert opened == [AUTHOR_URL]
    win._win.destroy()


def test_about_tab_shows_the_log_folder(root):
    from src.config import app_dir

    win = _open_settings_with_checker(root, lambda: None)
    assert win._logs_label.cget("text") == str(app_dir())
    win._win.destroy()


def _run_check(win):
    """同步跑一次檢查：worker 直接呼叫，結果自 queue 取出後交給主執行緒的處理函式
    （正式路徑是背景執行緒跑、poll_queue 主執行緒取，這裡接起來測試才不必等執行緒）。"""
    win._update_check_worker(win._update_queue)
    win._on_update_checked(win._update_queue.get_nowait())


def test_manual_check_reports_up_to_date(root):
    from src.i18n import t
    from src.ui import fields as fields_module

    win = _open_settings_with_checker(root, lambda: None)
    _run_check(win)
    assert win._update_result.cget("text") == "✓ " + t("update.latest")
    assert str(win._update_result.cget("foreground")) == fields_module.OK_COLOR
    assert win._update_btn.cget("text") == t("button.check_update")
    assert str(win._update_btn.cget("state")) == "normal"
    win._win.destroy()


def test_manual_check_reports_a_new_version(root, monkeypatch):
    from src.i18n import t
    from src.ui import fields as fields_module
    from src.ui import settings as settings_module
    from src.updater import Release

    release = Release(version="9.9.9", url="https://example.invalid/rel")
    win = _open_settings_with_checker(root, lambda: release)
    _run_check(win)
    assert win._update_result.cget("text") == t("update.available", version="9.9.9")
    assert "hand2" in str(win._update_result.cget("cursor"))
    assert str(win._update_result.cget("foreground")) == fields_module.LINK_COLOR

    opened = []
    monkeypatch.setattr(settings_module.webbrowser, "open", opened.append)
    win._update_result.event_generate("<Button-1>")
    root.update()
    assert opened == ["https://example.invalid/rel"]
    win._win.destroy()


def test_manual_check_reports_failure(root):
    from src.i18n import t
    from src.ui import fields as fields_module
    from src.updater import UpdateCheckError

    def boom():
        raise UpdateCheckError("HTTP 403")

    win = _open_settings_with_checker(root, boom)
    _run_check(win)
    assert win._update_result.cget("text") == "✗ " + t("update.failed", error="HTTP 403")
    assert str(win._update_result.cget("foreground")) == fields_module.ERROR_COLOR
    assert str(win._update_btn.cget("state")) == "normal"
    win._win.destroy()


def test_check_button_runs_the_real_thread_and_poll_path(root):
    """走完整路徑：按下按鈕 → 背景執行緒 → poll_queue 回主執行緒更新結果。

    假 checker 先卡在 Event 上才觀察得到「檢查中」；放行後 pump 事件迴圈直到結果出現
    （poll_queue 靠 after 輪詢，必須真的跑事件迴圈）。"""
    import threading
    import time

    from src.i18n import t

    release_checker = threading.Event()

    def checker():
        # 等待上限只是保險絲：平行跑測試（xdist）時一次 root.update() 就可能吃掉數秒，
        # 5 秒曾讓它提早回傳、按鈕在斷言前就恢復 normal
        release_checker.wait(30)
        return None

    win = _open_settings_with_checker(root, checker)
    win._update_btn.invoke()
    root.update()
    assert str(win._update_btn.cget("state")) == "disabled"
    assert win._update_btn.cget("text") == t("button.checking")

    release_checker.set()
    deadline = time.monotonic() + 30
    while not win._update_result.cget("text") and time.monotonic() < deadline:
        root.update()
        time.sleep(0.01)  # 讓出 CPU：純 root.update() 忙迴圈最壞情況會空轉滿 5 秒
    assert win._update_result.cget("text") == "✓ " + t("update.latest")
    assert str(win._update_btn.cget("state")) == "normal"
    assert win._update_btn.cget("text") == t("button.check_update")
    win._win.destroy()


def test_check_button_discards_a_stale_queue_result(root):
    """回歸測試：`_update_queue` 過去只在 __init__ 建一次、整個 app 生命週期共用。

    上一輪結果若因視窗提早關掉沒被 poll_queue 撈走會滯留；這裡直接塞一筆過期結果，
    驗證按下「檢查更新」看到的是這一輪的結果。"""
    import time

    from src.i18n import t

    win = _open_settings_with_checker(root, lambda: None)
    win._update_queue.put(("failed", "stale result from a previous round", None))

    win._update_btn.invoke()
    deadline = time.monotonic() + 5
    while win._update_result.cget("text") != "✓ " + t("update.latest") \
            and time.monotonic() < deadline:
        root.update()
        time.sleep(0.01)
    assert win._update_result.cget("text") == "✓ " + t("update.latest")
    win._win.destroy()


def test_a_stale_worker_cannot_land_in_a_later_rounds_queue(root):
    """回歸測試：worker 必須寫回啟動它的那一輪 queue，不能回頭讀 `_update_queue`。

    兩輪重疊：按下檢查 → 結果回來前關窗 → 重開 → 再按一次。舊 worker 若在 put 當下才查
    `self._update_queue`，過期結果會被新一輪的 poll_queue 撈走顯示。這裡先放行舊的，
    確認它落在自己的 queue、畫面不受影響。"""
    import threading
    import time

    from src.i18n import t
    from src.updater import UpdateCheckError

    stale_started, release_stale = threading.Event(), threading.Event()
    release_current = threading.Event()

    def stale_checker():
        stale_started.set()
        release_stale.wait(5)
        raise UpdateCheckError("stale round")

    win = _open_settings_with_checker(root, stale_checker)
    win._update_btn.invoke()
    assert stale_started.wait(5)
    stale_queue = win._update_queue

    # 結果回來前關窗再重開：按鈕與結果標籤都是新的，可以再按一次
    win._win.destroy()
    win.open()

    def current_checker():
        release_current.wait(5)
        return None

    win._check_update = current_checker
    win._update_btn.invoke()
    assert win._update_queue is not stale_queue

    # 先放行舊 worker，並給 poll_queue（100ms 一輪）足夠機會誤撈
    release_stale.set()
    deadline = time.monotonic() + 1
    while time.monotonic() < deadline:
        root.update()
        time.sleep(0.01)
    assert stale_queue.qsize() == 1               # 舊結果留在自己那一輪
    assert win._update_result.cget("text") == ""  # 畫面仍停在「檢查中」

    release_current.set()
    deadline = time.monotonic() + 5
    while not win._update_result.cget("text") and time.monotonic() < deadline:
        root.update()
        time.sleep(0.01)
    assert win._update_result.cget("text") == "✓ " + t("update.latest")
    win._win.destroy()


def test_update_result_is_only_clickable_over_its_text(root):
    """回歸測試：結果標籤不可撐滿整列——「有新版」時整個標籤是可點的連結，撐滿會讓
    文字後的空白也可點、游標也變手指。"""
    win = _open_settings_with_checker(root, lambda: None)
    info = win._update_result.pack_info()

    assert not int(info["expand"]), f"結果標籤不該 expand：{info}"
    assert str(info["fill"]) == "none", f"結果標籤不該 fill：{info}"
    win._win.destroy()


class FakeCache:
    """記錄 clear 呼叫次數與回傳筆數。"""

    def __init__(self, count=42):
        self.count = count
        self.cleared = 0

    def clear(self):
        self.cleared += 1
        return self.count


def _configured_cfg():
    from src.config import DEFAULT_CONFIG
    cfg = copy.deepcopy(DEFAULT_CONFIG)
    cfg["api"]["provider"] = "custom"
    cfg["api"]["custom"].update(base_url="http://x", model="m")
    return cfg


def test_about_tab_hides_the_cache_row_without_a_cache(root):
    from src.ui.settings import SettingsWindow
    win = SettingsWindow(root, _configured_cfg(), on_save=lambda: None)
    win.open()
    assert not hasattr(win, "_cache_result"), "沒有快取就不該畫出那一列"


def test_clear_cache_button_clears_and_reports_the_count(root):
    from src.i18n import t
    from src.ui.settings import SettingsWindow
    cache = FakeCache(count=42)
    win = SettingsWindow(root, _configured_cfg(), on_save=lambda: None, cache=cache)
    win.open()
    assert win._cache_result.cget("text") == ""      # 還沒按之前不顯示任何結果
    win._clear_cache()
    assert cache.cleared == 1
    assert win._cache_result.cget("text") == t("about.cache_cleared", count=42)


def test_clear_cache_reports_zero_when_the_cache_was_already_empty(root):
    from src.i18n import t
    from src.ui.settings import SettingsWindow
    cache = FakeCache(count=0)
    win = SettingsWindow(root, _configured_cfg(), on_save=lambda: None, cache=cache)
    win.open()
    win._clear_cache()
    assert win._cache_result.cget("text") == t("about.cache_cleared", count=0)
