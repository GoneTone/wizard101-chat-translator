"""覆疊視窗 `OverlayWindow` 的測試：標題列、橫幅、泡泡縮小與展開、分頁列、位置與縮放、更新提示與選取操作。"""
import pytest

from src.i18n import t
from src.reader.chat_tabs import TabState
from src.ui.geometry import edge_at, is_click, moved_to, point_in_rect, resized_edge
from src.ui.overlay.bubble import should_auto_expand
from src.ui.overlay.message_list import _row_bottom
from src.ui.overlay.outlined_text import line_origin
from src.ui.overlay.selection import TEXT_ORIGIN, Caret
from src.ui.overlay.window import (
    _GRIP_SIZE,
    BG,
    BG_UPDATE,
    MIN_HEIGHT,
    MIN_WIDTH,
    STATUS_COLORS,
    OverlayWindow,
    autoscroll_pixels,
)
from src.ui.thin_scrollbar import scroll_fraction, thumb_span
from tests.ui.overlay.overlay_helpers import (
    _at_bottom,
    _bar_state,
    _configure_width,
    _covered,
    _filled_overlay,
    _line_box,
    _long_overlay,
    _Press,
    _select_whole_message,
    _view,
)


def test_is_click_within_threshold():
    assert is_click(0, 0) is True
    assert is_click(4, -3) is True   # 位移小於門檻＝點擊


def test_is_click_beyond_threshold_is_drag():
    assert is_click(6, 0) is False
    assert is_click(0, -8) is False


def test_minimize_counts_unread_and_expand_resets(root):
    ov = OverlayWindow(root, x=0, y=0, max_messages=10, fade_seconds=0)
    ov.add_message("m0", "t0")
    ov.minimize()
    assert ov.minimized
    ov.add_message("m1", "t1")
    ov.add_message("m2", "t2")
    assert ov.unread_count() == 2      # 縮小期間累積未讀
    ov.expand()
    assert not ov.minimized
    assert ov.unread_count() == 0      # 展開歸零
    assert len(ov.visible_messages()) == 3  # 縮小期間的訊息沒有遺失


def test_moved_to_adds_delta():
    assert moved_to(100, 200, 15, -30) == (115, 170)


def test_resized_edge_south_east_grows_without_moving():
    assert resized_edge("se", 100, 200, 400, 300, 50, 20,
                        MIN_WIDTH, MIN_HEIGHT) == (100, 200, 450, 320)


def test_resized_edge_south_east_clamps_to_minimum():
    assert resized_edge("se", 100, 200, 400, 300, -1000, -1000,
                        MIN_WIDTH, MIN_HEIGHT) == (100, 200, MIN_WIDTH, MIN_HEIGHT)


def test_resized_edge_west_moves_origin_and_keeps_right_edge():
    # 往左拖 40：左緣左移 40、寬度加 40，右緣（x+w）不動
    assert resized_edge("w", 100, 200, 400, 300, -40, 0,
                        MIN_WIDTH, MIN_HEIGHT) == (60, 200, 440, 300)


def test_resized_edge_north_moves_origin_and_keeps_bottom_edge():
    assert resized_edge("n", 100, 200, 400, 300, 0, -40,
                        MIN_WIDTH, MIN_HEIGHT) == (100, 160, 400, 340)


def test_resized_edge_west_freezes_origin_at_minimum_width():
    # 撞到最小寬度後左緣要停住；否則游標繼續右移會把整個視窗一起往右拉走
    x, y, w, h = resized_edge("w", 100, 200, 400, 300, 1000, 0,
                              MIN_WIDTH, MIN_HEIGHT)
    assert (w, h) == (MIN_WIDTH, 300)
    assert x == 100 + 400 - MIN_WIDTH   # 右緣仍固定在 500
    assert y == 200


def test_resized_edge_north_west_resizes_both_axes():
    assert resized_edge("nw", 100, 200, 400, 300, -30, -20,
                        MIN_WIDTH, MIN_HEIGHT) == (70, 180, 430, 320)


def test_edge_at_outside_window_is_no_edge():
    assert edge_at(99, 300, 100, 200, 400, 300) == ""


def test_edge_at_interior_is_no_edge():
    assert edge_at(300, 350, 100, 200, 400, 300) == ""


def test_edge_at_each_side():
    assert edge_at(102, 350, 100, 200, 400, 300) == "w"
    assert edge_at(497, 350, 100, 200, 400, 300) == "e"
    assert edge_at(300, 202, 100, 200, 400, 300) == "n"
    assert edge_at(300, 497, 100, 200, 400, 300) == "s"


def test_edge_at_corners_take_priority_over_sides():
    assert edge_at(108, 208, 100, 200, 400, 300) == "nw"
    assert edge_at(492, 208, 100, 200, 400, 300) == "ne"
    assert edge_at(108, 492, 100, 200, 400, 300) == "sw"
    assert edge_at(492, 492, 100, 200, 400, 300) == "se"


def test_edge_at_ignores_corner_band_away_from_the_other_axis():
    # 距離左緣 10px（大於邊界寬、小於角落寬）但在視窗中段：不是角落也不算邊
    assert edge_at(110, 350, 100, 200, 400, 300) == ""


def test_error_banner_toggle(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300)
    assert ov.error_text() is None
    ov.set_error("notice.offline")
    assert ov.error_text() == t("notice.offline")
    ov.clear_error()
    assert ov.error_text() is None


@pytest.mark.real_position
def test_explicit_position_and_size_applied(root):
    ov = OverlayWindow(root, x=0, y=0, width=400, height=250)
    ov._win.update_idletasks()
    geo = ov._win.geometry()  # 形如 "400x250+0+0"
    assert geo.startswith("400x250+0+0")


def test_geometry_change_callback_fires_on_manual_apply(root):
    saved = []
    ov = OverlayWindow(root, x=10, y=20, width=400, height=250,
                       on_geometry_change=lambda x, y, w, h: saved.append((x, y, w, h)))
    ov._apply_geometry(50, 60, 300, 200)
    ov._win.update_idletasks()
    ov._emit_geometry()
    assert saved and saved[-1][2:] == (300, 200)


def test_set_status_updates_bar_label(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300)
    assert ov.status_text() == ""
    ov.set_status("listening")
    assert ov.status_text() == t("status.listening")
    ov.set_status("translating")
    assert ov.status_text() == t("status.translating")


def test_thumb_hidden_when_content_fits():
    assert thumb_span(0.0, 1.0, 200) is None       # 全部可見＝不需捲軸
    assert thumb_span(0.0, 0.5, 0) is None         # 軌道還沒有高度


def test_thumb_span_follows_view_fraction():
    assert thumb_span(0.0, 0.5, 200) == (0, 100)
    assert thumb_span(0.5, 1.0, 200) == (100, 200)


def test_thumb_span_keeps_minimum_length_inside_track():
    assert thumb_span(0.0, 0.02, 200, min_thumb=20) == (0, 20)
    # 捲到最底且比例極小：撐到最短長度，但不可超出軌道下緣
    assert thumb_span(0.98, 1.0, 200, min_thumb=20) == (180, 200)


def test_scrollbar_stops_above_resize_grip(root):
    # 把手 place 在視窗右下角：捲軸鋪到底會被壓住，滑塊捲到底時尤其明顯
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300)
    ov._win.update_idletasks()
    sb = ov._list._scrollbar
    bottom = sb.winfo_rooty() - ov._win.winfo_rooty() + sb.winfo_height()
    assert bottom <= ov._win.winfo_height() - _GRIP_SIZE


def test_scroll_fraction_subtracts_grab_offset_and_clamps():
    assert scroll_fraction(100, 0.0, 200) == 0.5
    assert scroll_fraction(100, 0.25, 200) == 0.25  # 抓在滑塊中段：扣掉偏移
    assert scroll_fraction(-50, 0.0, 200) == 0.0    # 拖出軌道上緣
    assert scroll_fraction(400, 0.0, 200) == 1.0    # 拖出軌道下緣


def test_point_in_rect_edges():
    assert point_in_rect(10, 10, 10, 10, 64, 64) is True
    assert point_in_rect(73, 73, 10, 10, 64, 64) is True
    assert point_in_rect(74, 40, 10, 10, 64, 64) is False   # 右緣外
    assert point_in_rect(9, 40, 10, 10, 64, 64) is False    # 左緣外


def test_should_auto_expand_when_bubble_becomes_foreground():
    # 點工作列按鈕／Alt+Tab 切回本工具：泡泡這一輪才變前景，游標不在泡泡上
    assert should_auto_expand(foreground=101, previous=202, bubble_hwnd=101,
                              cursor_on_bubble=False) is True


def test_no_auto_expand_while_cursor_on_bubble():
    # 直接按泡泡也會讓它變前景；展開與否交給既有的點擊／拖曳邏輯，否則拖不動
    assert should_auto_expand(foreground=101, previous=202, bubble_hwnd=101,
                              cursor_on_bubble=True) is False


def test_no_auto_expand_when_bubble_already_foreground():
    # 已經是前景（例如拖曳結束游標剛離開泡泡）不算切換，不重複觸發
    assert should_auto_expand(foreground=101, previous=101, bubble_hwnd=101,
                              cursor_on_bubble=False) is False


def test_no_auto_expand_for_other_windows():
    assert should_auto_expand(foreground=303, previous=202, bubble_hwnd=101,
                              cursor_on_bubble=False) is False


def test_minimize_starts_foreground_watch_and_expand_stops_it(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    assert ov._watch_job is None
    ov.minimize()
    assert ov._watch_job is not None
    ov.expand()
    assert ov._watch_job is None


def test_bubble_release_without_press_is_ignored(root):
    # 點 ─ 縮小時 minimize() withdraw 掉正被按住的視窗、隱式 grab 斷掉，放開滑鼠的事件落到
    # 剛出現的泡泡上 —— 沒有對應的 press，不得丟例外（實機 AttributeError），也不得展開
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.minimize()

    class FakeEvent:
        x_root = 10
        y_root = 10

    ov._bubble._release(FakeEvent())
    assert ov.minimized is True


def test_expand_reanchors_view_to_bottom(root):
    # 泡泡期間的訊息是在 unmap 狀態下排版的，展開後尺寸才確定。expand() 必須自己重算並貼底 ——
    # deiconify 不保證帶來 <Configure>，捲動範圍停在舊值的話最新訊息怎麼捲都捲不到
    ov = _filled_overlay(root)
    ov.minimize()
    root.update()
    for i in range(100, 106):
        ov.add_message(f"bubbled line {i} arriving while the window is a bubble",
                       f"第 {i} 則泡泡期間的譯文，長度足以換行", msg_id=i)
    ov._list._canvas.yview_moveto(0.0)   # 模擬 unmap 期間的排版落差把視圖推離底部
    root.update()
    assert ov._list._follow, "非使用者操作，跟隨狀態不該改變"
    ov.expand()
    root.update()
    assert _at_bottom(ov)


def test_messages_received_as_a_bubble_are_visible_after_expand(root):
    # 泡泡期間的訊息是在隱藏狀態下排版的：展開後最新一則要完整落在視口內，不能少了最後幾則
    ov = _filled_overlay(root)
    ov.minimize()
    root.update()
    for i in range(200, 204):
        ov.add_message(f"bubbled line {i} arriving while the window is a bubble",
                       f"第 {i} 則泡泡期間的譯文，長度足以換行", msg_id=i)
        root.update()
    ov.expand()
    root.update()
    canvas = ov._list._canvas
    _, top, _, bottom = canvas.bbox(*(ln.tag for ln in ov._list._messages[-1].lines))
    view_top = canvas.canvasy(0)
    assert view_top <= top and bottom <= view_top + canvas.winfo_height()


def test_refresh_labels_retranslates_status_and_banner(root):
    from src import i18n

    before = i18n.current_language()
    try:
        i18n.set_language("zh-TW")
        ov = OverlayWindow(root, x=0, y=0, width=460, height=300)
        ov.set_status("listening")
        ov.set_error("notice.offline")
        i18n.set_language("en-US")
        ov.refresh_labels()
        assert ov.status_text() == "●  Listening"
        assert ov.error_text() == "⚠  Translation server is offline — retrying…"
    finally:
        i18n.set_language(before)


def test_message_font_follows_language(root):
    # 新訊息的原文／譯文行要用建立當下的介面語言取字型，不能沿用啟動時鎖住的常數
    from src import i18n

    before = i18n.current_language()
    try:
        i18n.set_language("zh-CN")
        ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
        ov.add_message("[A] hi", "嗨", msg_id=1)
        lines = ov._list._messages[0].lines
        fonts = {str(line.canvas.itemcget(item, "font"))
                 for line in lines for item in line.canvas.find_withtag(line.tag)}
        assert all("YaHei" in f for f in fonts)
    finally:
        i18n.set_language(before)


def test_window_close_button_runs_the_clean_shutdown(root):
    # Alt+F4 與工作列「關閉視窗」都送 WM_DELETE_WINDOW；沒有 handler 時 Tk 只 destroy 這一個
    # Toplevel，留下 backdrop 孤兒、主迴圈照跑。兩個有工作列按鈕的視窗都必須接回乾淨關閉
    closed = []
    ov = OverlayWindow(root, x=0, y=0, max_messages=10, fade_seconds=0,
                       on_close=lambda: closed.append("win"))

    # tkinter 預設把這個 protocol 設成「destroy 這個 Toplevel」，所以不能只斷言它非空
    handler = ov._win.protocol("WM_DELETE_WINDOW")
    assert not handler.endswith("destroy"), "文字層仍停在 Tk 預設的 destroy 行為"
    root.call(handler)
    assert closed == ["win"]
    assert ov._win.winfo_exists(), "關閉要交給乾淨關閉流程，不是就地拆掉文字層"


def test_bubble_close_button_runs_the_clean_shutdown(root):
    # 泡泡同樣有工作列按鈕：被 Alt+F4 關掉而只 destroy 泡泡的話，主視窗仍是隱藏狀態，
    # 使用者會完全找不到這支程式。
    closed = []
    ov = OverlayWindow(root, x=0, y=0, max_messages=10, fade_seconds=0,
                       on_close=lambda: closed.append("bubble"))
    ov.minimize()

    handler = ov._bubble.protocol("WM_DELETE_WINDOW")
    assert not handler.endswith("destroy"), "泡泡仍停在 Tk 預設的 destroy 行為"
    root.call(handler)
    assert closed == ["bubble"]


def test_set_update_shows_a_clickable_banner(root):
    from src.runtime.updater import Release

    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    assert ov.update_text() is None
    ov.set_update(Release(version="0.2.0", url="https://example.invalid/rel"))
    assert ov.update_text() == t("update.available", version="0.2.0")
    assert ov._update_label.cget("cursor") == "hand2"
    ov.clear_update()
    assert ov.update_text() is None


def test_update_banner_uses_an_opaque_background(root):
    """回歸測試：橫幅底色不可等於 BG —— 本體視窗把 BG 設成 `-transparentcolor`，
    符合該色的像素在 Windows 下連 hit-test 都跳過，整列與右側 ✕ 就都點不到。"""
    from src.runtime.updater import Release

    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.set_update(Release(version="0.2.0", url="https://example.invalid/rel"))
    row = ov._update_row
    close, label = row.winfo_children()

    assert str(row.cget("bg")) == BG_UPDATE
    assert str(label.cget("bg")) == BG_UPDATE
    assert str(close.cget("bg")) == BG_UPDATE
    assert BG_UPDATE != BG


def test_update_banner_leaves_room_for_the_resize_grip(root):
    """回歸測試：橫幅右側要讓出縮放把手的寬度。

    把手 place 在右下角、底色是透明色鍵，疊在不透明橫幅上會挖出缺口並蓋掉半個 ✕。"""
    from src.runtime.updater import Release

    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.set_update(Release(version="0.2.0", url="https://example.invalid/rel"))

    # pack_info 的 padx 在不同 Tk 版本可能是 (0, 16) 序對或 "0 16" 字串
    padx = ov._update_row.pack_info()["padx"]
    right = padx[-1] if isinstance(padx, (tuple, list)) else str(padx).split()[-1]
    assert int(str(right)) == _GRIP_SIZE, f"右側未讓出把手寬度：padx={padx}"


def test_error_banner_formats_the_api_message_and_survives_language_switch(root):
    from src import i18n

    before = i18n.current_language()
    try:
        i18n.set_language("zh-TW")
        ov = OverlayWindow(root, x=0, y=0, width=460, height=300)
        ov.set_error("notice.config_error_detail", status=401, message="Incorrect API key")
        assert ov.error_text() == t("notice.config_error_detail", status=401,
                                    message="Incorrect API key")
        assert "Incorrect API key" in ov.error_text()
        i18n.set_language("en-US")
        ov.refresh_labels()   # 換語言重繪時 format 變數不可丟失
        assert ov.error_text() == t("notice.config_error_detail", status=401,
                                    message="Incorrect API key")
        assert "Incorrect API key" in ov.error_text()
    finally:
        i18n.set_language(before)


def test_error_banner_makes_urls_in_the_api_message_clickable(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300)
    ov.set_error("notice.config_error_detail", status=401,
                 message="see https://a.example/keys for a key")
    assert "https://a.example/keys" in ov.error_text()
    assert ov._error_label.links() == [("https://a.example/keys", "https://a.example/keys")]
    ov.clear_error()
    assert ov.error_text() is None


def test_update_banner_and_error_banner_coexist(root):
    # 兩者生命週期完全不同（錯誤隨狀態來去、更新是一次性），不該互相覆蓋
    from src.runtime.updater import Release

    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.set_error("notice.offline")
    ov.set_update(Release(version="0.2.0", url="https://example.invalid/rel"))
    assert ov.error_text() == t("notice.offline")
    assert ov.update_text() == t("update.available", version="0.2.0")
    ov.clear_update()
    assert ov.error_text() == t("notice.offline")   # 關掉更新橫幅不影響錯誤橫幅


def test_update_banner_follows_language_and_width(root):
    from src import i18n
    from src.runtime.updater import Release

    before = i18n.current_language()
    try:
        i18n.set_language("zh-TW")
        ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
        ov.set_update(Release(version="0.2.0", url="https://example.invalid/rel"))
        i18n.set_language("en-US")
        ov.refresh_labels()
        assert ov.update_text() == "⬆  Version 0.2.0 is available — click to download"

        class FakeEvent:
            width = 240

        ov._list._on_canvas_configure(FakeEvent())
        assert ov._update_label.cget("wraplength") == max(80, 240 - 12)
    finally:
        i18n.set_language(before)


def test_every_status_has_both_a_label_and_a_colour():
    """set_status 直接查 STATUS_COLORS[state]，漏一個顏色＝執行期 KeyError，
    而狀態列出現的時機（掛不進遊戲）正是最不該再炸一次的時候。"""
    import json
    from pathlib import Path
    catalog = json.loads(
        (Path(__file__).resolve().parents[3] / "src" / "i18n" / "zh-TW.json")
        .read_text(encoding="utf-8"))
    labelled = {k.removeprefix("status.") for k in catalog if k.startswith("status.")}
    assert labelled == set(STATUS_COLORS)


def test_wrapped_banners_stay_left_aligned(root):
    """tk.Label 多行預設置中：版本不相容這類長橫幅換行後會歪成階梯狀。"""
    from types import SimpleNamespace
    ov = OverlayWindow(root, x=0, y=0, width=320, height=200,
                       max_messages=10, fade_seconds=0)
    ov.set_update(SimpleNamespace(version="1.2.3", url="https://example.invalid"))
    assert str(ov._update_label.cget("justify")) == "left"


def test_refresh_labels_keeps_the_title_free_of_the_old_menu_glyph(root):
    from src import i18n
    from src.config import app_name

    before = i18n.current_language()
    try:
        i18n.set_language("zh-TW")
        ov = OverlayWindow(root, x=0, y=0, width=460, height=300)
        initial = ov._title_label.cget("text")
        i18n.set_language("en-US")
        ov.refresh_labels()
        # 標題列早已用 icon 取代 ≡，換語言重繪後也不該把它加回來
        assert ov._title_label.cget("text") == app_name()
        assert "≡" not in initial and "≡" not in ov._title_label.cget("text")
    finally:
        i18n.set_language(before)


def test_minimize_clears_the_selection(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.add_message("原文一", "譯文一")
    _select_whole_message(ov)

    ov.minimize()

    assert ov._selection.active is False
    ov.expand()


def test_press_on_a_message_starts_a_selection(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.add_message("原文一", "譯文一")
    # update()（非 update_idletasks）：_in_message_area 量的是 ov._list._canvas 的實際尺寸，
    # 這個 expand=True 的捲動畫布在全新 overrideredirect 視窗裡，只有真的跑過一輪
    # 事件迴圈（Windows 送 WM_SIZE）才會拿到非 1x1 的量測值，idle 佇列處理不到這段
    ov._win.update()
    first, second = ov._list._messages[0].lines

    x, top, height = _line_box(first)
    ov._selection_press(_Press(x + TEXT_ORIGIN, top + height // 2))
    x, top, height = _line_box(second)
    ov._selection_drag(_Press(x + 1000, top + height // 2))
    ov._selection_release(_Press(0, 0))

    assert ov._selection.text() == "原文一\n譯文一"


def test_press_on_a_row_scrolled_out_of_view_is_ignored(root):
    # 捲出視口的訊息列仍留著幾何位置：少了 _in_message_area 這道關卡，點在標題列
    # 附近就會選到看不見的訊息
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=50, fade_seconds=0)
    for i in range(20):
        ov.add_message(f"原文{i}", f"譯文{i}")
    ov._win.update()
    ov._list.complete_layout()   # 首次 <Configure> 改了換行寬度、只重排視口附近，第一則可能還沒畫
    hidden_top, hidden_bottom = ov._list._messages[0].lines
    x, top, height = _line_box(hidden_top)
    assert top < ov._list._canvas.winfo_rooty(), \
        "第一則應該已經捲出視口上方，否則這個測試沒有守到東西"

    ov._selection_press(_Press(x + 40, top + height // 2))
    x, top, height = _line_box(hidden_bottom)
    ov._selection_drag(_Press(x + 1000, top + height // 2))

    assert ov._selection.dragging is False
    assert ov._selection.text() == ""


def test_backdrop_press_away_from_any_edge_starts_a_selection(root):
    # 訊息列的底色是透明色鍵，字間空隙的點擊會落到 backdrop —— 那條路徑也要能起手
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.add_message("原文一", "譯文一")
    ov._win.update()   # 同上：_in_message_area 要量到真實的 canvas 尺寸
    first, second = ov._list._messages[0].lines

    # +40（非 +TEXT_ORIGIN）：捲動區沒有留邊，畫布左緣與視窗左緣重合，
    # +TEXT_ORIGIN 會落在 EDGE 縮放感應帶內、被 _edge_press 誤判成縮放
    x, top, height = _line_box(first)
    ov._edge_press(_Press(x + 40, top + height // 2))
    x, top, height = _line_box(second)
    ov._edge_drag(_Press(x + 1000, top + height // 2))
    ov._edge_release(_Press(0, 0))

    assert ov._selection.text().endswith("\n譯文一")
    assert ov._selection.active is True
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
    ov._win.update()   # 同上：_in_message_area 要量到真實的 canvas 尺寸
    # 用最後一則（非第一則）：視圖貼底時第一則已捲出視口，_in_message_area
    # 會擋下這次按下、根本起不了選取
    first, _ = ov._list._messages[-1].lines
    x, top, height = _line_box(first)
    ov._selection_press(_Press(x + TEXT_ORIGIN, top + height // 2))
    ov._list._canvas.yview_moveto(0.0)
    ov._win.update_idletasks()
    before = ov._list._canvas.yview()[0]

    ov._list.refresh_scroll()

    assert ov._list._canvas.yview()[0] == before
    ov._selection_release(_Press(0, 0))


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


def test_right_click_without_a_selection_pops_no_menu(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.add_message("原文一", "譯文一")

    ov._selection_menu(_Press(300, 300))
    ov._win.update()

    assert ov._popup.visible is False


def test_right_click_with_a_selection_pops_the_themed_menu(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.add_message("原文一", "譯文一")
    _select_whole_message(ov)

    ov._selection_menu(_Press(300, 300))
    ov._win.update()

    assert ov._popup.visible is True
    assert ov._popup.label_text() == t("menu.copy")


def test_choosing_copy_from_the_menu_copies_and_closes(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.add_message("原文一", "譯文一")
    _select_whole_message(ov)
    ov._selection_menu(_Press(300, 300))
    ov._win.update()

    ov._popup._clicked(None)
    ov._win.update()

    assert root.clipboard_get() == "原文一\n譯文一"
    assert ov._popup.visible is False


def test_clicking_a_message_closes_the_menu(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.add_message("原文一", "譯文一")
    first, _ = _select_whole_message(ov)
    ov._selection_menu(_Press(300, 300))
    ov._win.update()

    first.canvas.event_generate("<ButtonPress-1>", x=2, y=2)
    ov._win.update()

    assert ov._popup.visible is False


def test_clicking_the_title_bar_closes_the_menu(root):
    # 「點別的地方就關掉」不能只涵蓋訊息區：標題列、捲軸、右下把手都是使用者會直覺
    # 點的地方，事件沿 bindtags 傳到 toplevel，綁在那裡才全部涵蓋得到
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.add_message("原文一", "譯文一")
    _select_whole_message(ov)
    ov._selection_menu(_Press(300, 300))
    ov._win.update()

    ov._title_label.event_generate("<ButtonPress-1>", x=2, y=2)
    ov._win.update()

    assert ov._popup.visible is False


def test_autoscroll_is_still_inside_the_viewport():
    assert autoscroll_pixels(500, 400, 600) == 0
    assert autoscroll_pixels(400, 400, 600) == 0
    assert autoscroll_pixels(600, 400, 600) == 0


def test_autoscroll_goes_up_above_the_viewport_and_down_below_it():
    assert autoscroll_pixels(392, 400, 600) < 0
    assert autoscroll_pixels(608, 400, 600) > 0


def test_autoscroll_speeds_up_with_distance_but_is_capped():
    near = autoscroll_pixels(610, 400, 600, lo=2, hi=24)
    far = autoscroll_pixels(650, 400, 600, lo=2, hi=24)
    assert 0 < near < far < 24
    assert autoscroll_pixels(9000, 400, 600, lo=2, hi=24) == 24


def _tick_autoscroll(ov, times=20):
    """手動跑幾輪自動捲動。

    每輪都要讓 Tk 重新排版：`winfo_rooty()` 回報的是上次排版的位置，連續同步呼叫
    而不 update 的話，caret 會一直算在捲動前的那一列上。正式路徑每輪是獨立的
    `after` 回呼，中間本來就有事件迴圈。每輪自己排的下一輪也要取消，免得留
    after 排程給別的測試。"""
    for _ in range(times):
        ov._autoscroll()
        ov._stop_autoscroll()
        ov._win.update()


def test_dragging_above_the_viewport_scrolls_and_grows_the_selection(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=50, fade_seconds=0)
    for i in range(20):
        ov.add_message(f"原文{i}", f"譯文{i}")
    ov._win.update()
    first, _ = ov._list._messages[-1].lines
    x, top, height = _line_box(first)
    ov._selection_press(_Press(x + TEXT_ORIGIN, top + height // 2))
    ov._selection_drag(_Press(x + 10, ov._list._canvas.winfo_rooty() - 60))
    ov._stop_autoscroll()
    ov._win.update()
    view_before, text_before = ov._list._canvas.yview()[0], ov._selection.text()

    _tick_autoscroll(ov)
    ov._win.update()

    assert ov._list._canvas.yview()[0] < view_before, "拖到視口上方應該要往上捲"
    assert len(ov._selection.text()) > len(text_before), "捲動後選取要跟著長出來"


def test_autoscroll_stops_once_the_drag_ends(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=50, fade_seconds=0)
    for i in range(20):
        ov.add_message(f"原文{i}", f"譯文{i}")
    ov._win.update()
    first, _ = ov._list._messages[-1].lines
    x, top, height = _line_box(first)
    ov._selection_press(_Press(x + TEXT_ORIGIN, top + height // 2))
    ov._selection_drag(_Press(x + 10, ov._list._canvas.winfo_rooty() - 60))
    ov._selection_release(_Press(0, 0))
    ov._win.update()
    view_after_release = ov._list._canvas.yview()[0]

    _tick_autoscroll(ov)
    ov._win.update()

    assert ov._list._canvas.yview()[0] == view_after_release
    assert ov._autoscroll_job is None


def test_minimize_closes_the_menu(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.add_message("原文一", "譯文一")
    _select_whole_message(ov)
    ov._selection_menu(_Press(300, 300))
    ov._win.update()

    ov.minimize()
    ov._win.update()

    assert ov._popup.visible is False
    ov.expand()


def test_selection_entry_points_are_bound(root):
    # 所有既有測試都直接呼叫 handler，綁定整組刪掉也不會轉紅 —— 這條守住實際入口
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       max_messages=10, fade_seconds=0)
    ov.add_message("原文一", "譯文一")

    assert ov._win.bind("<Control-c>")
    assert ov._win.bind("<Control-C>")
    assert ov._backdrop.bind("<Button-3>")
    for line in ov._list._messages[0].lines:
        for sequence in ("<ButtonPress-1>", "<B1-Motion>",
                         "<ButtonRelease-1>", "<Button-3>"):
            assert line.canvas.bind(sequence), f"{sequence} 未綁定"


def test_offer_update_shows_the_banner_when_nothing_was_dismissed(root):
    from src.runtime.updater import Release

    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.offer_update(Release(version="0.2.0", url="https://example.invalid/rel"))
    assert ov.update_text() == t("update.available", version="0.2.0")
    ov.clear_update()


def test_offer_update_skips_a_version_the_user_dismissed(root):
    """每小時自動檢查會反覆查到同一版：使用者按 ✕ 關掉後，同版（或更舊）不再跳出，
    只有更新的版本才再顯示。"""
    from src.runtime.updater import Release

    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.set_update(Release(version="0.2.0", url="https://example.invalid/rel"))
    ov._dismiss_update()
    assert ov.update_text() is None

    ov.offer_update(Release(version="0.2.0", url="https://example.invalid/rel"))
    assert ov.update_text() is None
    ov.offer_update(Release(version="0.3.0", url="https://example.invalid/rel"))
    assert ov.update_text() == t("update.available", version="0.3.0")
    ov.clear_update()


def test_set_update_ignores_the_dismissed_version(root):
    """手動檢查走 set_update：使用者自己按了「檢查更新」，關掉過的版本也要再顯示。"""
    from src.runtime.updater import Release

    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.set_update(Release(version="0.2.0", url="https://example.invalid/rel"))
    ov._dismiss_update()
    ov.set_update(Release(version="0.2.0", url="https://example.invalid/rel"))
    assert ov.update_text() == t("update.available", version="0.2.0")
    ov.clear_update()


def test_region_button_invokes_the_callback_when_provided(root):
    calls = []
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       on_region=lambda: calls.append("region"))
    ov._win.update()
    ov._region_btn.event_generate("<Button-1>")
    ov._win.update()
    assert calls == ["region"]


def test_region_button_absent_without_a_callback(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300)
    assert ov._region_btn is None


def _tooltip_for(ov, widget):
    return next(tip for tip in ov._tooltips if tip._widget is widget)


def test_region_button_tooltip_shows_the_configured_hotkey(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       on_region=lambda: None, region_hotkey="ctrl+shift+space")
    tip = _tooltip_for(ov, ov._region_btn)
    tip._show()  # 直接觸發顯示：text_fn 現取當下的熱鍵，不必等真的懸停
    assert tip.text() == t("tooltip.region", hotkey="ctrl+shift+space")


def test_set_region_hotkey_updates_the_tooltip_on_the_next_show(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300,
                       on_region=lambda: None, region_hotkey="ctrl+shift+space")
    tip = _tooltip_for(ov, ov._region_btn)

    ov.set_region_hotkey("ctrl+alt+r")
    tip._show()

    assert tip.text() == t("tooltip.region", hotkey="ctrl+alt+r")


def test_clear_update_does_not_count_as_dismissing(root):
    """程式自己收橫幅（clear_update）不是使用者的決定，之後同版仍該再提醒。"""
    from src.runtime.updater import Release

    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.set_update(Release(version="0.2.0", url="https://example.invalid/rel"))
    ov.clear_update()
    ov.offer_update(Release(version="0.2.0", url="https://example.invalid/rel"))
    assert ov.update_text() == t("update.available", version="0.2.0")
    ov.clear_update()


def test_tab_bar_shows_game_labels_and_highlights_current(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.set_tabs(1, _bar_state(1))
    assert ov.tab_labels() == [("主要的", False), ("队伍", True), ("房间(1)", False)]
    # 排在標題列之下、訊息區之上
    assert ov._win.pack_slaves().index(ov._tab_bar._frame) == 1


def test_tab_bar_hidden_until_a_state_arrives(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    assert ov.tab_labels() == []


def test_messages_are_kept_per_tab_and_follow_the_game_tab(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.set_tabs(1, _bar_state(0))
    ov.add_message("main", "主", slot=1, tabs=(0,))
    ov.add_message("group", "隊", slot=1, tabs=(1,))
    assert ov.visible_messages() == [("main", "主")]
    ov.set_tabs(1, _bar_state(1))
    assert ov.visible_messages() == [("group", "隊")]


def test_messages_for_a_tab_not_in_the_bar_are_kept(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.set_tabs(1, TabState(0, ((0, "主要的"), (1, "队伍"))))
    ov.add_message("friend", "友", slot=1, tabs=(3,))
    assert ov.visible_messages() == []
    ov.set_tabs(1, TabState(3, ((0, "主要的"), (1, "队伍"), (3, "好友"))))
    assert ov.visible_messages() == [("friend", "友")]


def test_broadcast_lands_in_every_listed_tab(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.add_message("news", "公告", slot=1, tabs=(0, 1, 2, 3))
    for tab in (0, 1, 2, 3):
        ov.set_tabs(1, TabState(tab, ((tab, "x"),)))
        assert ov.visible_messages() == [("news", "公告")]


def test_foreground_slot_switches_the_whole_view(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.set_tabs(1, _bar_state(0))
    ov.set_tabs(2, TabState(1, ((0, "Main"), (1, "Group"))))
    ov.add_message("one", "一", slot=1, tabs=(0,))
    ov.add_message("two", "二", slot=2, tabs=(1,))
    assert ov.visible_messages() == [("one", "一")]
    ov.set_foreground_slot(2)
    assert ov.tab_labels() == [("Main", False), ("Group", True)]
    assert ov.visible_messages() == [("two", "二")]


def test_repeating_the_foreground_slot_keeps_the_view(root):
    # 收訊執行緒每輪都會送前景客戶端：同一個客戶端不可重建清單（會打斷捲動與選取）
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.set_tabs(1, _bar_state(0))
    ov.add_message("one", "一", slot=1, tabs=(0,))
    rows = [m.lines for m in ov._list._messages]
    ov.set_foreground_slot(1)
    assert [m.lines for m in ov._list._messages] == rows


def test_unread_counts_only_the_foreground_client(root):
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.set_tabs(1, _bar_state(0))
    ov.minimize()
    ov.add_message("x", "x", slot=2)
    ov.add_message("y", "y", slot=2)
    ov.add_message("news", "公告", slot=1, tabs=(0, 1, 2, 3))
    assert ov.unread_count() == 1
    ov.expand()


def test_autoscrolling_a_selection_into_the_undrawn_area_keeps_it_on_the_same_text(root):
    ov = _long_overlay(root, 120)
    _configure_width(ov, 300)
    sel = ov._selection
    sel._anchor, sel._focus = Caret(80, 0, 0), Caret(119, 1, 3)   # 選取大半在未畫區
    sel.redraw()
    before = sel.text()
    canvas = ov._list._canvas
    for _ in range(300):
        ov._list.scroll_by(-40)                          # 框選拖到上緣時的自動捲動路徑
        assert _covered(ov) and sel.text() == before
        view = _view(ov)
        for m in view.records[max(80, view.lo):view.hi]:
            top, bottom = line_origin(m.lines[0])[1], _row_bottom(m)
            assert [i for i in canvas.find_withtag("sel")
                    if top <= canvas.coords(i)[1] < bottom], m.original
        if view.lo < 75:
            break
    assert _view(ov).lo < 75
