"""框選矩形 → 遊戲 client 座標的換算（純函式）；`crop_frame` 對合成 Frame 的裁切；
`capture_window` 從 WGC 影格裁出 client 區、單色畫面與 Win32 例外一律變成 CaptureError；
`capture_screen` 失敗時退回全黑。"""
import io
import threading

import numpy
import pytest
from PIL import Image, ImageDraw

import src.region.capture as capture_module
from src.region.capture import CaptureError, Frame, SelectionOutsideGame, crop_frame, window_region


def test_rect_inside_the_client_is_shifted_to_client_origin():
    assert window_region((150, 220, 300, 100), (100, 200), (1280, 720)) == (50, 20, 300, 100)


def test_rect_overlapping_the_edges_is_clipped():
    # 左上超出 client：裁掉超出的部分，寬高跟著縮
    assert window_region((50, 150, 300, 100), (100, 200), (1280, 720)) == (0, 0, 250, 50)
    # 右下超出 client
    assert window_region((1300, 850, 300, 100), (100, 200), (1280, 720)) == (1200, 650, 80, 70)


def test_rect_entirely_outside_the_client_is_none():
    assert window_region((0, 0, 50, 50), (100, 200), (1280, 720)) is None
    assert window_region((2000, 1000, 50, 50), (100, 200), (1280, 720)) is None


def test_rect_touching_the_edge_without_overlap_is_none():
    assert window_region((1380, 300, 50, 50), (100, 200), (1280, 720)) is None


def test_selection_outside_game_is_a_capture_error():
    assert issubclass(SelectionOutsideGame, CaptureError)


def test_non_capture_error_during_capture_becomes_a_capture_error(monkeypatch):
    # 遊戲視窗在熱鍵觸發後、擷取前消失：ClientToScreen 是第一個 Win32 呼叫，
    # 不用真的開一顆視窗就能模擬「掛掉的那一種例外」。
    def boom(hwnd, point):
        raise RuntimeError("gone")

    monkeypatch.setattr(capture_module.win32gui, "ClientToScreen", boom)
    with pytest.raises(CaptureError) as ei:
        capture_module.capture_window(1)
    assert "gone" in str(ei.value)


def _fake_window(monkeypatch, grabbed: Image.Image) -> None:
    """client 區在螢幕 (720, 181)、大小 4×3；DWM 外框左上角在 (719, 150)，WGC 拍到 grabbed。"""
    monkeypatch.setattr(capture_module.win32gui, "ClientToScreen", lambda hwnd, point: (720, 181))
    monkeypatch.setattr(capture_module.win32gui, "GetClientRect", lambda hwnd: (0, 0, 4, 3))
    monkeypatch.setattr(capture_module, "_frame_origin", lambda hwnd: (719, 150))
    monkeypatch.setattr(capture_module, "_grab_window", lambda hwnd: grabbed)


def test_capture_window_crops_the_client_area_out_of_the_dwm_frame(monkeypatch):
    grabbed = Image.new("RGB", (6, 35), "white")
    grabbed.paste(Image.new("RGB", (4, 3), "red"), (1, 31))
    grabbed.putpixel((1, 31), (0, 0, 255))
    _fake_window(monkeypatch, grabbed)
    frame = capture_module.capture_window(1)
    assert frame.client_origin == (720, 181)
    assert frame.client_size == (4, 3)
    assert frame.image.size == (4, 3)
    assert frame.image.getpixel((0, 0)) == (0, 0, 255)
    assert frame.image.getpixel((3, 2)) == (255, 0, 0)


@pytest.mark.parametrize("colour", ["white", "black"])
def test_single_colour_capture_is_a_capture_error(monkeypatch, colour):
    _fake_window(monkeypatch, Image.new("RGB", (6, 35), colour))
    with pytest.raises(CaptureError) as ei:
        capture_module.capture_window(1)
    assert "blank" in str(ei.value)


class _FakeControl:
    def __init__(self):
        self.stopped = threading.Event()

    def stop(self):
        self.stopped.set()


class _FakeSession:
    """代替 WindowsCapture：start_free_threaded 後在另一條執行緒依序送出 events。"""

    instances: list["_FakeSession"] = []

    def __init__(self, events, **kwargs):
        self.events, self.kwargs, self.handlers = events, kwargs, {}
        self.control = _FakeControl()
        _FakeSession.instances.append(self)

    def event(self, handler):
        self.handlers[handler.__name__] = handler
        return handler

    def start_free_threaded(self):
        def run():
            for name, payload in self.events:
                if name == "frame":
                    self.handlers["on_frame_arrived"](payload, self.control)
                else:
                    self.handlers["on_closed"]()
        threading.Thread(target=run, daemon=True).start()
        return self.control


class _FakeFrame:
    def __init__(self, bgra: numpy.ndarray):
        self.frame_buffer = bgra
        self.height, self.width = bgra.shape[:2]


def _use_sessions(monkeypatch, events):
    _FakeSession.instances = []
    monkeypatch.setattr(capture_module, "WindowsCapture",
                        lambda **kwargs: _FakeSession(events, **kwargs))


def test_grab_window_converts_the_first_frame_from_bgra(monkeypatch):
    bgra = numpy.zeros((2, 3, 4), dtype=numpy.uint8)
    bgra[0, 0] = (255, 0, 0, 255)       # BGRA 的藍
    _use_sessions(monkeypatch, [("frame", _FakeFrame(bgra))])
    image = capture_module._grab_window(0x9b0fc8)
    assert image.size == (3, 2)
    assert image.getpixel((0, 0)) == (0, 0, 255)
    session = _FakeSession.instances[0]
    assert session.kwargs == {"cursor_capture": False, "draw_border": False,
                              "window_hwnd": 0x9b0fc8}
    assert session.control.stopped.is_set()


def test_grab_window_times_out_when_no_frame_arrives(monkeypatch):
    monkeypatch.setattr(capture_module, "_FRAME_TIMEOUT_S", 0.05)
    _use_sessions(monkeypatch, [])
    with pytest.raises(CaptureError) as ei:
        capture_module._grab_window(1)
    assert "no frame" in str(ei.value)
    assert _FakeSession.instances[0].control.stopped.is_set()


def test_grab_window_session_closed_before_a_frame_is_a_capture_error(monkeypatch):
    _use_sessions(monkeypatch, [("closed", None)])
    with pytest.raises(CaptureError) as ei:
        capture_module._grab_window(1)
    assert "closed" in str(ei.value)


def _client_frame() -> Frame:
    """200×100 白底、(50, 30) 起 20×10 的黑色矩形，模擬凍結下來的整個 client 畫面。"""
    image = Image.new("RGB", (200, 100), "white")
    ImageDraw.Draw(image).rectangle((50, 30, 69, 39), fill="black")
    return Frame(image, client_origin=(1000, 500), client_size=(200, 100))


def test_crop_frame_returns_a_png_matching_the_selected_rect():
    frame = _client_frame()
    png = crop_frame(frame, (1050, 530, 20, 10))
    decoded = Image.open(io.BytesIO(png))
    assert decoded.size == (20, 10)
    assert decoded.getpixel((0, 0)) == (0, 0, 0)


def test_crop_frame_clips_a_rect_partially_outside_the_client():
    frame = _client_frame()
    screen_rect = (1180, 530, 40, 10)
    expected = window_region(screen_rect, frame.client_origin, frame.client_size)
    png = crop_frame(frame, screen_rect)
    decoded = Image.open(io.BytesIO(png))
    assert decoded.size == (expected[2], expected[3])


def test_crop_frame_rect_entirely_outside_the_client_is_selection_outside_game():
    frame = _client_frame()
    with pytest.raises(SelectionOutsideGame):
        crop_frame(frame, (0, 0, 10, 10))


def test_capture_screen_falls_back_to_black_when_grab_fails(monkeypatch):
    def boom(**kwargs):
        raise RuntimeError("no display")

    monkeypatch.setattr(capture_module.ImageGrab, "grab", boom)
    image = capture_module.capture_screen((100, 200, 300, 150))
    assert image.size == (300, 150)
    assert image.getpixel((0, 0)) == (0, 0, 0)


def test_capture_screen_returns_the_grabbed_image_on_success(monkeypatch):
    stub = Image.new("RGB", (10, 10), "red")
    monkeypatch.setattr(capture_module.ImageGrab, "grab", lambda **kwargs: stub)
    assert capture_module.capture_screen((0, 0, 10, 10)) is stub
