"""框選矩形 → 遊戲 client 座標的換算（純函式）；`crop_frame` 對合成 Frame 的裁切；
`capture_window` 的 Win32 例外一律變成 CaptureError。"""
import io

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
