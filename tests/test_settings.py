"""設定視窗純邏輯與 overlay set_limits／set_alpha 測試。"""
import tkinter as tk

import pytest

from src.config import clamp_advanced
from src.ui import form as form_module
from src.ui.bubble import BUBBLE_ALPHA_FACTOR, BUBBLE_ALPHA_FLOOR
from src.ui.overlay import OverlayWindow
from src.ui.richtext import LINK_COLOR
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
    # 泡泡刻意比主視窗再透一些 —— 它是收起來的浮標，該更低調，所以不是同一個值
    assert float(ov._bubble.attributes("-alpha")) == pytest.approx(0.5 * BUBBLE_ALPHA_FACTOR)
    ov.expand()


def test_bubble_alpha_stops_at_the_floor(root):
    # overlay_alpha 已經是最低時再打折會讓泡泡幾乎看不見，找不回來 —— 所以有下限
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
    from src.ui.settings import SettingsWindow
    from tests.config_helpers import configured_cfg

    before = i18n.current_language()
    try:
        i18n.set_language("zh-TW")
        cfg = configured_cfg()
        saved = []
        win = SettingsWindow(root, cfg,
                             on_save=lambda: saved.append(i18n.current_language()))
        win.open()
        win._ui_language.set_value("en-US")
        win._save()
        assert cfg["ui_language"] == "en-US"
        assert i18n.current_language() == "en-US"
        # on_save 看到的必須已經是新語言：這行是本測試的重點，
        # 若 _save() 把 on_save 提前到套用語言之前，這裡會是 "zh-TW"。
        assert saved == ["en-US"]
    finally:
        i18n.set_language(before)


def test_save_stores_the_system_message_toggle(root):
    from src.ui.settings import SettingsWindow
    from tests.config_helpers import configured_cfg

    cfg = configured_cfg()
    win = SettingsWindow(root, cfg, on_save=lambda: None)
    win.open()
    assert win._translate_system.get() is False   # 預設關閉
    win._translate_system.set(True)
    win._save()
    assert cfg["translate_system_messages"] is True


def test_reopening_settings_reflects_the_saved_toggle(root):
    from src.ui.settings import SettingsWindow
    from tests.config_helpers import configured_cfg

    cfg = configured_cfg()
    cfg["translate_system_messages"] = True
    win = SettingsWindow(root, cfg, on_save=lambda: None)
    win.open()
    assert win._translate_system.get() is True


def _open_settings(root, on_language_preview=None):
    from src.i18n import current_language
    from src.ui.settings import SettingsWindow
    from tests.config_helpers import configured_cfg

    cfg = configured_cfg()
    cfg["ui_language"] = current_language()
    win = SettingsWindow(root, cfg, on_save=lambda: None,
                         on_language_preview=on_language_preview)
    win.open()
    return win


def test_changing_target_language_clears_the_test_result(root):
    # 測試連線顯示的譯文是用當時的目標語言翻出來的，語言一改那句就過期了 ——
    # 留著會讓使用者以為新語言已經驗證過。
    win = _open_settings(root)
    win._api._show_test_result(True, "connected, sample translation")
    assert win._api.test_passed
    win._language.set_value("English")
    assert not win._api.test_passed
    assert win._api._test_result.text() == ""
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
        win._on_language_change("en-US")
        root.update()

        assert i18n.current_language() == "en-US"
        assert relabelled == ["en-US"]
        assert win._cfg["ui_language"] == "zh-TW"
        assert not old_win.winfo_exists()
        assert win._win.title() == t("settings.title", app=app_name())
        win._win.destroy()
    finally:
        i18n.set_language(before)


def test_language_preview_flushes_the_overlay_repaint_before_rebuilding(root):
    """回歸測試：overlay relabel 之後必須先跑一輪完整事件迴圈（update），再重建設定視窗。
    新視窗映射時湧出的繪圖事件會把 overlay 標籤縮短後騰出區域的重繪往後推，舊語言多出的
    那截字殘留約 0.3 秒（實機截圖：「对话翻译助手or」、「(● 监听中ɡ」）；update_idletasks
    不夠，逐幀擷取實測仍殘留。"""
    from src import i18n

    before = i18n.current_language()
    events = []
    real_update = root.update
    try:
        i18n.set_language("zh-TW")
        win = _open_settings(root)
        old_win = win._win
        win._on_language_preview = lambda: events.append("relabel")
        root.update = lambda: events.append("update")
        win._on_language_change("en-US")
        real_update()
        assert events == ["relabel", "update"]
        assert not old_win.winfo_exists() and win._win.winfo_exists()
        win._win.destroy()
    finally:
        root.update = real_update
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

        win._on_language_change("en-US")
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
        win._on_language_change("en-US")
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
        win._on_language_change("en-US")
        root.update()
        win._cancel()

        assert i18n.current_language() == "zh-TW"
        assert relabelled == ["en-US", "zh-TW"]
        assert win._cfg["ui_language"] == "zh-TW"
    finally:
        i18n.set_language(before)


def test_save_keeps_the_previewed_language(root):
    from src import i18n

    before = i18n.current_language()
    try:
        i18n.set_language("zh-TW")
        win = _open_settings(root)
        win._on_language_change("en-US")
        root.update()
        win._save()

        assert i18n.current_language() == "en-US"
        assert win._cfg["ui_language"] == "en-US"
    finally:
        i18n.set_language(before)


def _open_settings_with_checker(root, checker):
    from src.i18n import current_language
    from src.ui.settings import SettingsWindow
    from tests.config_helpers import configured_cfg

    cfg = configured_cfg()
    cfg["ui_language"] = current_language()
    win = SettingsWindow(root, cfg, on_save=lambda: None, check_update=checker)
    win.open()
    return win


def test_about_tab_shows_version_and_links(root, monkeypatch):
    from src import __version__
    from src.i18n import t
    from src.ui import settings as settings_module
    from src.updater import AUTHOR_URL, ISSUES_URL, PROJECT_URL

    opened = []
    monkeypatch.setattr(settings_module.webbrowser, "open", opened.append)
    win = _open_settings_with_checker(root, lambda: None)
    tabs = [win._nb.tab(i, "text") for i in range(win._nb.index("end"))]
    assert tabs[2] == t("settings.tab.about")
    assert win._version_label.cget("text") == f"v{__version__}"
    assert win._project_link.cget("text") == PROJECT_URL
    assert win._author_link.cget("text") == "GoneTone"
    assert win._issues_link.cget("text") == ISSUES_URL

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
    """同步跑一次檢查：直接呼叫 checker，結果（或例外）交給主執行緒的處理函式
    （正式路徑是 BackgroundButton 在背景執行緒跑、poll_queue 主執行緒取，
    這裡接起來測試才不必等執行緒）。"""
    try:
        result = win._check_update()
    except Exception as exc:
        result = exc
    win._on_update_checked(result)


def test_manual_check_reports_up_to_date(root):
    from src.i18n import t

    win = _open_settings_with_checker(root, lambda: None)
    _run_check(win)
    assert win._update_result.cget("text") == "✓ " + t("update.latest")
    assert str(win._update_result.cget("foreground")) == form_module.OK_COLOR
    assert win._update_btn.cget("text") == t("button.check_update")
    assert str(win._update_btn.cget("state")) == "normal"
    win._win.destroy()


def test_manual_check_hands_a_new_version_to_on_update_found(root):
    """手動檢查查到新版時，除了關於分頁的連結，也要讓 overlay 顯示橫幅。"""
    from src.i18n import current_language
    from src.ui.settings import SettingsWindow
    from src.updater import Release
    from tests.config_helpers import configured_cfg

    release = Release(version="9.9.9", url="https://example.invalid/rel")
    found = []
    cfg = configured_cfg()
    cfg["ui_language"] = current_language()
    win = SettingsWindow(root, cfg, on_save=lambda: None, check_update=lambda: release,
                         on_update_found=found.append)
    win.open()
    _run_check(win)
    assert found == [release]
    win._win.destroy()


def test_manual_check_keeps_on_update_found_quiet_when_up_to_date(root):
    from src.i18n import current_language
    from src.ui.settings import SettingsWindow
    from tests.config_helpers import configured_cfg

    found = []
    cfg = configured_cfg()
    cfg["ui_language"] = current_language()
    win = SettingsWindow(root, cfg, on_save=lambda: None, check_update=lambda: None,
                         on_update_found=found.append)
    win.open()
    _run_check(win)
    assert found == []
    win._win.destroy()


def test_manual_check_reports_a_new_version(root, monkeypatch):
    from src.i18n import t
    from src.ui import settings as settings_module
    from src.updater import Release

    release = Release(version="9.9.9", url="https://example.invalid/rel")
    win = _open_settings_with_checker(root, lambda: release)
    _run_check(win)
    assert win._update_result.cget("text") == t("update.available", version="9.9.9")
    assert "hand2" in str(win._update_result.cget("cursor"))
    assert str(win._update_result.cget("foreground")) == LINK_COLOR

    opened = []
    monkeypatch.setattr(settings_module.webbrowser, "open", opened.append)
    win._update_result.event_generate("<Button-1>")
    root.update()
    assert opened == ["https://example.invalid/rel"]
    win._win.destroy()


def test_manual_check_reports_failure(root):
    from src.i18n import t
    from src.updater import UpdateCheckError

    def boom():
        raise UpdateCheckError("HTTP 403")

    win = _open_settings_with_checker(root, boom)
    _run_check(win)
    assert win._update_result.cget("text") == "✗ " + t("update.failed", error="HTTP 403")
    assert str(win._update_result.cget("foreground")) == form_module.ERROR_COLOR
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
        # 不設等待上限：這個假 checker 的語意就是「測試放行前絕不回應」。平行跑測試
        # （xdist）時一次 root.update() 就可能吃掉數秒，任何逾時都可能先到期讓它提早
        # 回傳、按鈕在斷言前就恢復 normal。執行緒是 daemon，測試提早失敗也不會擋住行程。
        release_checker.wait()
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


def test_update_result_is_only_clickable_over_its_text(root):
    """回歸測試：結果標籤不可撐滿整列 —— 「有新版」時整個標籤是可點的連結，撐滿會讓
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


def test_about_tab_hides_the_cache_row_without_a_cache(root):
    from src.ui.settings import SettingsWindow
    from tests.config_helpers import configured_cfg
    win = SettingsWindow(root, configured_cfg(), on_save=lambda: None)
    win.open()
    assert not hasattr(win, "_cache_result"), "沒有快取就不該畫出那一列"


def test_clear_cache_button_clears_and_reports_the_count(root):
    from src.i18n import t
    from src.ui.settings import SettingsWindow
    from tests.config_helpers import configured_cfg
    cache = FakeCache(count=42)
    win = SettingsWindow(root, configured_cfg(), on_save=lambda: None, cache=cache)
    win.open()
    assert win._cache_result.cget("text") == ""      # 還沒按之前不顯示任何結果
    win._clear_cache()
    assert cache.cleared == 1
    assert win._cache_result.cget("text") == t("about.cache_cleared", count=42)


def test_clear_cache_reports_zero_when_the_cache_was_already_empty(root):
    from src.i18n import t
    from src.ui.settings import SettingsWindow
    from tests.config_helpers import configured_cfg
    cache = FakeCache(count=0)
    win = SettingsWindow(root, configured_cfg(), on_save=lambda: None, cache=cache)
    win.open()
    win._clear_cache()
    assert win._cache_result.cget("text") == t("about.cache_cleared", count=0)


def _texts(frame):
    """一列 widget 的文字，依 pack 順序。"""
    return [w.cget("text") for w in frame.pack_slaves()]


def _fake_translators(monkeypatch, credit):
    """假造各語言的譯者掛名。兩個模組各自取值：下拉底下那列走 form 的共用元件，
    關於分頁自己取（那一列是 grid 的兩欄，版面與共用元件不同）。"""
    from src.ui import settings as settings_module
    for module in (form_module, settings_module):
        monkeypatch.setattr(module, "translators", credit)


def test_translators_row_follows_the_ui_language_field(root, monkeypatch):
    from src.i18n import t

    _fake_translators(monkeypatch, lambda code: "[A](https://a.example)")
    win = _open_settings(root)
    basic = win._ui_language.master
    order = [str(w) for w in basic.pack_slaves()]
    # 譯者掛的是「選到的這個介面語言」，所以緊跟在下拉之下，不與翻譯目標語言隔開
    assert order.index(str(win._translators_row)) \
        == order.index(str(win._ui_language)) + 1
    label, names = win._translators_row.pack_slaves()
    assert label.cget("text") == t("credit.translators")
    assert _texts(names) == ["A"]
    # 「協助我們翻譯」接在譯者之下：兩者都屬於選到的這個介面語言
    assert (order.index(str(win._help_translate_link))
            == order.index(str(win._translators_row)) + 1)
    win._win.destroy()


def test_help_translate_link_opens_crowdin(root, monkeypatch):
    from src.i18n import t
    from src.updater import CROWDIN_URL

    _fake_translators(monkeypatch, lambda code: "")
    opened = []
    monkeypatch.setattr(form_module.webbrowser, "open", opened.append)
    win = _open_settings(root)
    link = win._help_translate_link
    assert link.cget("text") == t("credit.help_translate")
    link.event_generate("<Button-1>")
    assert opened == [CROWDIN_URL]
    win._win.destroy()


def test_about_tab_shows_the_translators(root, monkeypatch):
    from src.i18n import t

    _fake_translators(monkeypatch, lambda code: "[A](https://a.example)、B")
    win = _open_settings_with_checker(root, lambda: None)
    assert _texts(win._about_translators) == ["A", "、B"]
    grid = win._about_translators.grid_info()
    # 譯者接在開發者之下：兩者都是掛名，中間不該插進「回報問題」那類操作列
    assert (int(grid["row"]), int(grid["column"])) \
        == (int(win._author_link.grid_info()["row"]) + 1, 1)
    assert win._issues_link.grid_info()["row"] > grid["row"]
    label = win._about_translators.master.grid_slaves(row=int(grid["row"]), column=0)[0]
    assert label.cget("text") == t("credit.translators")
    # 「協助翻譯」緊接在譯者之下，同樣排在「回報問題」之前
    contribute = win._contribute_link.grid_info()
    assert int(contribute["row"]) == int(grid["row"]) + 1
    assert win._issues_link.grid_info()["row"] > contribute["row"]
    win._win.destroy()


def test_about_tab_links_to_crowdin(root, monkeypatch):
    from src.i18n import t
    from src.updater import CROWDIN_URL

    _fake_translators(monkeypatch, lambda code: "")
    win = _open_settings_with_checker(root, lambda: None)
    link = win._contribute_link
    assert link.cget("text") == CROWDIN_URL
    grid = link.grid_info()
    label = link.master.grid_slaves(row=int(grid["row"]), column=0)[0]
    assert label.cget("text") == t("about.help_translate")
    # 沒有譯者列時直接接在開發者之下
    assert int(grid["row"]) == int(win._author_link.grid_info()["row"]) + 1
    win._win.destroy()


def test_translators_rows_are_hidden_when_the_language_credits_nobody(root, monkeypatch):
    _fake_translators(monkeypatch, lambda code: "")
    win = _open_settings_with_checker(root, lambda: None)
    assert win._translators_row is None, "沒有譯者就不該畫出那一列"
    assert win._about_translators is None, "沒有譯者就不該畫出那一列"
    # 邀請協助翻譯的連結不看有沒有譯者：沒人翻的語言更需要
    basic = win._ui_language.master
    order = [str(w) for w in basic.pack_slaves()]
    assert (order.index(str(win._help_translate_link))
            == order.index(str(win._ui_language)) + 1)
    assert win._contribute_link.winfo_manager() == "grid"
    win._win.destroy()


def test_translators_follow_the_previewed_language(root, monkeypatch):
    # 換介面語言會整個重建視窗，譯者列必須跟著換成新語言的掛名
    from src import i18n

    _fake_translators(monkeypatch,
                      lambda code: "" if code == "zh-TW" else code.upper())
    before = i18n.current_language()
    try:
        i18n.set_language("zh-TW")
        win = _open_settings(root)
        assert win._translators_row is None
        win._on_language_change("en-US")
        root.update()
        assert _texts(win._translators_row.pack_slaves()[1]) == ["EN-US"]
        win._win.destroy()
    finally:
        i18n.set_language(before)


def test_settings_window_does_not_stay_on_top(root):
    # 開窗時抬一次是為了不被 topmost 的 overlay 蓋住，但抬完就該放掉 ——
    # 設定視窗不該一直壓在遊戲以外的其他程式之上
    win = _open_settings(root)
    assert bool(win._win.attributes("-topmost")) is True   # 抬起中
    root.update_idletasks()
    assert bool(win._win.attributes("-topmost")) is False
    win._win.destroy()


def test_releasing_topmost_survives_a_window_closed_in_the_meantime(root):
    # idle 佇列跑到之前視窗可能已被關掉（或換語言重建），不該炸 TclError
    win = _open_settings(root)
    closed = win._win
    closed.destroy()
    win._release_topmost(closed)


def test_paste_hotkey_defaults_on_and_saves_from_the_settings_toggle(root):
    from src.config import DEFAULT_CONFIG
    from src.ui.settings import SettingsWindow
    from tests.config_helpers import configured_cfg

    assert DEFAULT_CONFIG["paste_hotkey"] is True
    cfg = configured_cfg()
    win = SettingsWindow(root, cfg, on_save=lambda: None)
    win.open()
    assert win._paste_hotkey.get() is True
    win._paste_hotkey.set(False)
    win._save()
    assert cfg["paste_hotkey"] is False


def test_save_stores_the_region_hotkey(root):
    from src.ui.settings import SettingsWindow
    from tests.config_helpers import configured_cfg

    cfg = configured_cfg()
    win = SettingsWindow(root, cfg, on_save=lambda: None)
    win.open()
    assert win._region_hotkey.value() == "ctrl+shift+space"
    win._region_hotkey.set_value("ctrl+alt+r")
    win._save()
    assert cfg["region_hotkey"] == "ctrl+alt+r"


def test_save_rejects_identical_hotkeys(root, monkeypatch):
    from src.i18n import t
    from src.ui import settings as settings_module
    from src.ui.settings import SettingsWindow
    from tests.config_helpers import configured_cfg

    warnings = []
    monkeypatch.setattr(settings_module.messagebox, "showwarning",
                        lambda title, message, parent=None: warnings.append(message))
    cfg = configured_cfg()
    saved = []
    win = SettingsWindow(root, cfg, on_save=lambda: saved.append(1))
    win.open()
    win._region_hotkey.set_value(cfg["hotkey"])
    win._save()
    assert saved == []
    assert t("error.hotkeys_same") in warnings[0]
    assert cfg["region_hotkey"] == "ctrl+shift+space"
    win._win.destroy()
