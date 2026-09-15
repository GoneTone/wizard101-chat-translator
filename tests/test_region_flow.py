"""框選流程：熱鍵切換、擷取失敗、背景結果回填、過期 session 丟棄；選取層與卡片用替身。"""
import queue

from src.i18n import t
from src.region.capture import CaptureError, SelectionOutsideGame
from src.region.ocr import OcrUnavailable
from src.region.pipeline import RegionResult
from src.translation.translator import TranslatorBadOutput, TranslatorConfigError, TranslatorOffline
from src.ui.region_flow import RegionFlow, describe_error

_RECT = (100, 100, 300, 120)
_MONITOR = (0, 0, 1920, 1080)


class FakeSelector:
    def __init__(self):
        self.is_open = False
        self.on_select = None
        self.cancelled = 0

    def show(self, monitor, on_select, on_cancel=None):
        self.is_open, self.on_select = True, on_select

    def cancel(self):
        self.is_open = False
        self.cancelled += 1

    def pick(self, rect):
        self.is_open = False
        self.on_select(rect)


class FakeCard:
    def __init__(self):
        self.events = []
        self.is_open = False

    def show_pending(self, rect):
        self.is_open = True
        self.events.append(("pending", rect))

    def show_text(self, text):
        self.events.append(("text", text))

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


def _flow(root, pipeline, capture=lambda hwnd, rect: b"png"):
    ui_queue = queue.Queue()
    selector, card = FakeSelector(), FakeCard()
    flow = RegionFlow(root, pipeline, ui_queue, alpha=0.8, selector=selector, card=card,
                      capture=capture, monitor_at=lambda x, y: _MONITOR,
                      foreground=lambda hwnd: None)
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


def test_selection_captures_and_fills_the_card_with_the_translation(root):
    flow, selector, card, ui_queue = _flow(root, FakePipeline(RegionResult("譯文", "image")))
    flow.toggle(0x1234)
    selector.pick(_RECT)
    _drain(ui_queue, flow)
    assert card.events == [("pending", _RECT), ("text", "譯文")]


def test_capture_failure_is_shown_on_the_card_without_a_worker(root):
    def failing(hwnd, rect):
        raise SelectionOutsideGame("selection outside game window")

    flow, selector, card, _ = _flow(root, FakePipeline(), capture=failing)
    flow.toggle(0x1234)
    selector.pick(_RECT)
    assert card.events == [("pending", _RECT), ("error", t("region.outside_game"))]
    assert flow._thread is None


def test_plain_capture_error_is_shown_as_capture_failed(root):
    boom = CaptureError("PrintWindow failed")

    def failing(hwnd, rect):
        raise boom

    flow, selector, card, _ = _flow(root, FakePipeline(), capture=failing)
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
        return RegionResult(f"譯文{rect[2]}", "image")


def test_stale_result_is_dropped_after_a_new_selection(root):
    flow, selector, card, ui_queue = _flow(root, RectPipeline())
    flow.toggle(0x1234)
    selector.pick(_RECT)
    flow._thread.join(timeout=5)     # 第一輪結果已排進 ui_queue，尚未回填
    flow.toggle(0x1234)              # 新一輪：舊結果回填時 session 已對不上
    selector.pick((0, 0, 50, 50))
    _drain(ui_queue, flow)
    assert ("text", "譯文300") not in card.events
    assert ("text", "譯文50") in card.events


def test_toggle_hides_a_previous_card(root):
    flow, selector, card, ui_queue = _flow(root, FakePipeline(RegionResult("譯文", "image")))
    flow.toggle(0x1234)
    selector.pick(_RECT)
    _drain(ui_queue, flow)
    flow.toggle(0x1234)
    assert card.events[-1] == ("hide",)


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
