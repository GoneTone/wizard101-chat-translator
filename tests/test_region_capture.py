"""框選矩形 → 遊戲 client 座標的換算（純函式，不碰 Win32）。"""
import pytest

import src.region.capture as capture_module
from src.region.capture import CaptureError, SelectionOutsideGame, window_region


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
    # 遊戲視窗在滑鼠放開後、擷取前消失：ClientToScreen 是第一個 Win32 呼叫，
    # 不用真的開一顆視窗就能模擬「掛掉的那一種例外」。
    def boom(hwnd, point):
        raise RuntimeError("gone")

    monkeypatch.setattr(capture_module.win32gui, "ClientToScreen", boom)
    with pytest.raises(CaptureError) as ei:
        capture_module.capture_region(1, (0, 0, 10, 10))
    assert "gone" in str(ei.value)
