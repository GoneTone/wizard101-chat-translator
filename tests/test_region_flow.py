"""框選流程：熱鍵切換、凍結畫面擷取／裁切失敗、背景結果回填、過期 session 丟棄；
選取層與卡片用替身。"""
import queue

from src.i18n import t
from src.region.capture import CaptureError, SelectionOutsideGame
from src.region.ocr import OcrUnavailable
from src.region.pipeline import RegionResult
from src.translation.translator import TranslatorBadOutput, TranslatorConfigError, TranslatorOffline
from src.ui.region_flow import RegionFlow, describe_error

_RECT = (100, 100, 300, 120)
_MONITOR = (0, 0, 1920, 1080)
_FRAME = object()   # 凍結畫面本身的內容跟流程無關，只要能原封不動傳到 crop 即可


class FakeSelector:
    def __init__(self):
        self.is_open = False
        self.frame = None
        self.backdrop = None
        self.on_select = None
        self.on_cancel = None
        self.cancelled = 0

    def show(self, monitor, frame, on_select, on_cancel=None, backdrop=None):
        self.is_open, self.frame, self.backdrop = True, frame, backdrop
        self.on_select, self.on_cancel = on_select, on_cancel

    def cancel(self):
        """比照真的 RegionSelector.cancel(notify=True)：關層後呼叫 on_cancel。"""
        self.is_open = False
        self.cancelled += 1
        if self.on_cancel is not None:
            self.on_cancel()

    def pick(self, rect):
        self.is_open = False
        self.on_select(rect)


class FakeCard:
    def __init__(self):
        self.events = []
        self.is_open = False
        self.on_close = None

    def show_pending(self, rect):
        self.is_open = True
        self.events.append(("pending", rect))

    def show_text(self, text, source=""):
        self.events.append(("text", text, source))

    def show_error(self, message):
        self.events.append(("error", message))

    def hide(self):
        if self.is_open:
            self.events.append(("hide",))
        self.is_open = False

    def set_alpha(self, alpha):
        self.events.append(("alpha", alpha))


class FakePipeline:
    def __init__(self, result=None, raises=None):
        self._result, self._raises = result, raises

    def run(self, png, rect):
        if self._raises:
            raise self._raises
        return self._result


def _flow(root, pipeline, capture_window=lambda hwnd: _FRAME,
         crop=lambda frame, rect: b"png", capture_screen=lambda monitor: None,
         foreground=lambda hwnd: None, find_game=lambda: 0x1234):
    ui_queue = queue.Queue()
    selector, card = FakeSelector(), FakeCard()
    flow = RegionFlow(root, pipeline, ui_queue, alpha=0.8, selector=selector, card=card,
                      capture_window=capture_window, crop=crop, capture_screen=capture_screen,
                      monitor_at=lambda x, y: _MONITOR, foreground=foreground,
                      find_game=find_game)
    return flow, selector, card, ui_queue


def _drain(ui_queue, flow):
    flow._thread.join(timeout=5)
    while not ui_queue.empty():
        ui_queue.get_nowait()()


def test_toggle_opens_the_selector_and_a_second_toggle_cancels_it(root):
    flow, selector, card, _ = _flow(root, FakePipeline())
    flow.toggle(0x1234)
    assert selector.is_open and flow.is_selecting
    flow.toggle(0x1234)
    assert selector.cancelled == 1 and not flow.is_selecting


def test_toggle_passes_the_captured_frame_to_the_selector(root):
    flow, selector, card, _ = _flow(root, FakePipeline())
    flow.toggle(0x1234)
    assert selector.frame is _FRAME


def test_toggle_passes_the_captured_backdrop_to_the_selector(root):
    backdrop = object()
    flow, selector, card, _ = _flow(root, FakePipeline(), capture_screen=lambda monitor: backdrop)
    flow.toggle(0x1234)
    assert selector.backdrop is backdrop


def test_selection_captures_and_fills_the_card_with_the_translation(root):
    flow, selector, card, ui_queue = _flow(root, FakePipeline(RegionResult("譯文")))
    flow.toggle(0x1234)
    selector.pick(_RECT)
    _drain(ui_queue, flow)
    assert card.events == [("pending", _RECT), ("text", "譯文", "")]


def test_selection_passes_the_source_text_to_the_card(root):
    flow, selector, card, ui_queue = _flow(
        root, FakePipeline(RegionResult("譯文", "Talk to Merle")))
    flow.toggle(0x1234)
    selector.pick(_RECT)
    _drain(ui_queue, flow)
    assert ("text", "譯文", "Talk to Merle") in card.events


def test_selection_crops_the_same_frame_that_was_captured(root):
    captured = []

    def recording_crop(frame, rect):
        captured.append(frame)
        return b"png"

    flow, selector, card, ui_queue = _flow(
        root, FakePipeline(RegionResult("譯文")), crop=recording_crop)
    flow.toggle(0x1234)
    selector.pick(_RECT)
    _drain(ui_queue, flow)
    assert captured == [_FRAME]


def test_frame_capture_failure_is_shown_without_opening_the_selector(root):
    boom = CaptureError("PrintWindow failed")

    def failing(hwnd):
        raise boom

    flow, selector, card, _ = _flow(root, FakePipeline(), capture_window=failing)
    flow.toggle(0x1234)
    assert card.events == [("pending", (0, 0, 0, 0)), ("error", t("region.capture_failed", error=boom))]
    assert not selector.is_open
    assert flow._thread is None


def test_capture_failure_is_shown_on_the_card_without_a_worker(root):
    def failing(frame, rect):
        raise SelectionOutsideGame("selection outside game window")

    flow, selector, card, _ = _flow(root, FakePipeline(), crop=failing)
    flow.toggle(0x1234)
    selector.pick(_RECT)
    assert card.events == [("pending", _RECT), ("error", t("region.outside_game"))]
    assert flow._thread is None


def test_plain_capture_error_is_shown_as_capture_failed(root):
    boom = CaptureError("PrintWindow failed")

    def failing(frame, rect):
        raise boom

    flow, selector, card, _ = _flow(root, FakePipeline(), crop=failing)
    flow.toggle(0x1234)
    selector.pick(_RECT)
    assert card.events == [("pending", _RECT), ("error", t("region.capture_failed", error=boom))]
    assert flow._thread is None


def test_pipeline_errors_are_described_on_the_card(root):
    flow, selector, card, ui_queue = _flow(
        root, FakePipeline(raises=OcrUnavailable("no pack")))
    flow.toggle(0x1234)
    selector.pick(_RECT)
    _drain(ui_queue, flow)
    assert card.events[-1] == ("error", t("region.ocr_unavailable"))


class RectPipeline:
    """譯文帶矩形寬度，兩輪框選的結果才分得出新舊。"""

    def run(self, png, rect):
        return RegionResult(f"譯文{rect[2]}")


def test_stale_result_is_dropped_after_a_new_selection(root):
    flow, selector, card, ui_queue = _flow(root, RectPipeline())
    flow.toggle(0x1234)
    selector.pick(_RECT)
    flow._thread.join(timeout=5)     # 第一輪結果已排進 ui_queue，尚未回填
    flow.toggle(0x1234)              # 新一輪：舊結果回填時 session 已對不上
    selector.pick((0, 0, 50, 50))
    _drain(ui_queue, flow)
    assert ("text", "譯文300", "") not in card.events
    assert ("text", "譯文50", "") in card.events


def test_stale_error_is_dropped_after_a_new_selection(root):
    flow, selector, card, ui_queue = _flow(
        root, FakePipeline(raises=OcrUnavailable("no pack")))
    flow.toggle(0x1234)
    selector.pick(_RECT)
    flow._thread.join(timeout=5)     # 第一輪錯誤已排進 ui_queue，尚未回填
    flow.toggle(0x1234)              # 新一輪：舊錯誤回填時 session 已對不上
    selector.pick((0, 0, 50, 50))
    _drain(ui_queue, flow)
    # 兩輪的錯誤文案相同（OcrUnavailable 一律映射同一句），靠次數而非內容分辨
    # 舊結果確實被 _show_error 的 session 檢查擋下、沒有重複回填卡片
    assert card.events.count(("error", t("region.ocr_unavailable"))) == 1


def test_selection_moves_foreground_before_showing_the_pending_card(root):
    calls = []

    def foreground(hwnd):
        calls.append(("foreground", hwnd))

    flow, selector, card, _ = _flow(
        root, FakePipeline(RegionResult("譯文")), foreground=foreground)
    original_show_pending = card.show_pending

    def recording_show_pending(rect):
        calls.append(("pending", rect))
        original_show_pending(rect)

    card.show_pending = recording_show_pending
    flow.toggle(0x1234)
    selector.pick(_RECT)
    assert calls == [("foreground", 0x1234), ("pending", _RECT)]
    flow._thread.join(timeout=5)


def test_toggle_hides_a_previous_card(root):
    flow, selector, card, ui_queue = _flow(root, FakePipeline(RegionResult("譯文")))
    flow.toggle(0x1234)
    selector.pick(_RECT)
    _drain(ui_queue, flow)
    flow.toggle(0x1234)
    assert card.events[-1] == ("hide",)


def test_esc_style_cancel_restores_the_games_foreground(root):
    calls = []
    flow, selector, card, _ = _flow(root, FakePipeline(), foreground=calls.append)
    flow.toggle(0x1234)
    selector.cancel()   # 模擬選取層自己觸發 on_cancel（Esc、右鍵、點一下沒拖動）
    assert calls == [0x1234]


def test_hotkey_cancel_restores_the_games_foreground(root):
    calls = []
    flow, selector, card, _ = _flow(root, FakePipeline(), foreground=calls.append)
    flow.toggle(0x1234)
    flow.toggle(0x1234)   # 選取層開著時再按一次熱鍵＝取消
    assert calls == [0x1234]


def test_toggle_with_hwnd_zero_is_ignored_once_the_selector_is_already_closed(root):
    flow, selector, card, _ = _flow(root, FakePipeline())
    flow.toggle(0)
    assert not selector.is_open and not flow.is_selecting


def test_set_alpha_forwards_to_the_card(root):
    flow, selector, card, _ = _flow(root, FakePipeline())
    flow.set_alpha(0.5)
    assert ("alpha", 0.5) in card.events


# --- start_from_button：標題列按鈕入口，遊戲 hwnd 靠列舉找而非讀前景 ---
def test_start_from_button_opens_the_selector_with_the_found_window(root):
    flow, selector, card, _ = _flow(root, FakePipeline(), find_game=lambda: 0x9999)
    flow.start_from_button()
    assert selector.is_open and flow.is_selecting
    assert flow._game_hwnd == 0x9999


def test_start_from_button_shows_an_error_card_when_no_game_window_is_found(monkeypatch, root):
    from src.ui import region_flow as region_flow_module

    monkeypatch.setattr(region_flow_module, "cursor_position", lambda: (40, 50))
    flow, selector, card, _ = _flow(root, FakePipeline(), find_game=lambda: None)

    flow.start_from_button()

    assert not selector.is_open and not flow.is_selecting
    assert card.events == [("pending", (40, 50, 0, 0)), ("error", t("region.no_game"))]


def test_start_from_button_cancels_an_already_open_selector(root):
    flow, selector, card, _ = _flow(root, FakePipeline(), find_game=lambda: 0x1234)
    flow.start_from_button()
    assert selector.is_open

    flow.start_from_button()

    assert selector.cancelled == 1 and not flow.is_selecting


def test_describe_error_maps_each_failure_kind():
    assert describe_error(TranslatorConfigError("bad key", status=401)) == t(
        "error.api_response", status=401, message="bad key")
    assert describe_error(TranslatorOffline("refused")) == t(
        "error.offline_detail", message="refused")
    assert describe_error(OcrUnavailable("x")) == t("region.ocr_unavailable")
    bad = TranslatorBadOutput("truncated")
    assert describe_error(bad) == t("region.failed", error=bad)
    boom = RuntimeError("boom")
    assert describe_error(boom) == t("error.unexpected", error=boom)


class FakeBox:
    def __init__(self):
        self.is_open = False
        self.rects = []
        self.on_change = None

    def show(self, rect):
        self.is_open = True
        self.rects.append(rect)

    def hide(self):
        self.is_open = False

    def adjust(self, rect):
        """比照真的 RegionBox：使用者調整完放開，回報新矩形。"""
        self.on_change(rect)


def _flow_with_box(root, pipeline, **kwargs):
    ui_queue = queue.Queue()
    selector, card, box = FakeSelector(), FakeCard(), FakeBox()
    flow = RegionFlow(root, pipeline, ui_queue, alpha=0.8, selector=selector, card=card,
                      box=box, capture_screen=lambda monitor: None,
                      monitor_at=lambda x, y: _MONITOR, foreground=lambda hwnd: None,
                      **kwargs)
    return flow, selector, card, box, ui_queue


def test_selection_leaves_the_box_on_the_selected_rect(root):
    flow, selector, card, box, ui_queue = _flow_with_box(
        root, FakePipeline(RegionResult("譯文")), capture_window=lambda hwnd: _FRAME,
        crop=lambda frame, rect: b"png")
    flow.toggle(0x1234)
    selector.pick(_RECT)
    _drain(ui_queue, flow)
    assert box.is_open and box.rects == [_RECT]


def test_adjusting_the_box_recaptures_the_live_game_and_retranslates(root):
    frames = iter([_FRAME, "live frame"])
    cropped = []

    def recording_crop(frame, rect):
        cropped.append((frame, rect))
        return b"png"

    flow, selector, card, box, ui_queue = _flow_with_box(
        root, RectPipeline(), capture_window=lambda hwnd: next(frames), crop=recording_crop)
    flow.toggle(0x1234)
    selector.pick(_RECT)
    _drain(ui_queue, flow)
    new_rect = (120, 110, 340, 130)
    box.adjust(new_rect)
    _drain(ui_queue, flow)
    assert cropped == [(_FRAME, _RECT), ("live frame", new_rect)]
    assert card.events[-2:] == [("pending", new_rect), ("text", "譯文340", "")]
    assert box.is_open


def test_capture_failure_while_adjusting_is_shown_and_keeps_the_box(root):
    boom = CaptureError("PrintWindow failed")
    frames = iter([_FRAME])

    def capture(hwnd):
        try:
            return next(frames)
        except StopIteration:
            raise boom from None

    flow, selector, card, box, ui_queue = _flow_with_box(
        root, FakePipeline(RegionResult("譯文")), capture_window=capture,
        crop=lambda frame, rect: b"png")
    flow.toggle(0x1234)
    selector.pick(_RECT)
    _drain(ui_queue, flow)
    box.adjust((0, 0, 50, 50))
    assert card.events[-2:] == [("pending", (0, 0, 50, 50)),
                                ("error", t("region.capture_failed", error=boom))]
    assert box.is_open


def test_closing_the_card_hides_the_box(root):
    flow, selector, card, box, ui_queue = _flow_with_box(
        root, FakePipeline(RegionResult("譯文")), capture_window=lambda hwnd: _FRAME,
        crop=lambda frame, rect: b"png")
    flow.toggle(0x1234)
    selector.pick(_RECT)
    _drain(ui_queue, flow)
    card.on_close()
    assert not box.is_open


def test_a_new_selection_hides_the_previous_box(root):
    flow, selector, card, box, ui_queue = _flow_with_box(
        root, FakePipeline(RegionResult("譯文")), capture_window=lambda hwnd: _FRAME,
        crop=lambda frame, rect: b"png")
    flow.toggle(0x1234)
    selector.pick(_RECT)
    _drain(ui_queue, flow)
    flow.toggle(0x1234)
    assert not box.is_open and selector.is_open
