"""遊戲聊天輸入框的螢幕座標：邏輯座標換算（純函式）與 WizChatReader 以假節點讀取（不需遊戲）。"""
import asyncio

import pytest

from src.reader import mem_reader
from src.reader.mem_reader import WizChatReader
from src.reader.ui_rect import client_rect

# 實機探測值：根視窗 1370×770 邏輯座標、client 1920×1080，容器鏈
# chatEditContainer(0,190) → chatContainer(0,0) → WizardChatBox(23,300) → WorldView → Root
_CHAIN = [(0, 190), (0, 0), (23, 300), (0, 0), (0, 0)]


def test_client_rect_scales_summed_offsets_by_client_over_root():
    # 比例 1.4015：x 32.2 → 32、y 686.7 → 687、w 734.4 → 734、h 46.2 → 46（四捨五入）
    assert client_rect(_CHAIN, (524, 33), (1370, 770), (1920, 1080)) == (32, 687, 734, 46)


def test_client_rect_rounds_instead_of_truncating():
    # 實機截圖：ui_scale 1.276 下遊戲金框 669 px 寬＝524 × 1.276 四捨五入；截尾會短 1 px
    assert client_rect([(0, 0)], (524, 33), (1370, 770), (1748, 983))[2] == 669


def test_client_rect_is_identity_when_root_matches_client():
    assert client_rect([(10, 20), (5, 5)], (100, 40), (800, 600), (800, 600)) == (15, 25, 100, 40)


def test_client_rect_rejects_degenerate_root():
    with pytest.raises(ValueError):
        client_rect(_CHAIN, (523, 32), (0, 770), (1920, 1080))


class _Rect:
    def __init__(self, x1, y1, x2, y2):
        self.x1, self.y1, self.x2, self.y2 = x1, y1, x2, y2


class _Node:
    def __init__(self, rect, parents=()):
        self._rect = rect
        self._parents = list(parents)
        self.fail = False

    async def window_rectangle(self):
        if self.fail:
            raise RuntimeError("stale node")
        return _Rect(*self._rect)

    async def get_parents(self):
        return self._parents

    async def is_visible(self):
        return True


class _Root(_Node):
    def __init__(self, rect, node):
        super().__init__(rect)
        self.node = node
        self.searches = 0

    async def get_windows_with_name(self, name):
        self.searches += 1
        return [self.node]


class _Client:
    window_handle = 0x1234

    def __init__(self, root):
        self.root_window = root


def _reader(monkeypatch, node, root_rect=(0, 0, 1370, 770)):
    root = _Root(root_rect, node)
    r = WizChatReader()
    r._connected = True
    r._loop = asyncio.new_event_loop()
    r._client = _Client(root)
    monkeypatch.setattr(mem_reader.win32gui, "GetClientRect",
                        lambda hwnd: (0, 0, 1920, 1080))
    monkeypatch.setattr(mem_reader.win32gui, "ClientToScreen",
                        lambda hwnd, pt: (717 + pt[0], 207 + pt[1]))
    return r


def test_input_box_screen_rect_walks_parents_and_offsets_by_window_origin(monkeypatch):
    parents = [_Node((0, 0, 523, 222)), _Node((23, 300, 562, 522)),
               _Node((0, 0, 1370, 770)), _Node((0, 0, 1370, 770))]
    node = _Node((0, 190, 523, 222), parents)
    r = _reader(monkeypatch, node)
    try:
        # 遊戲把矩形的 x2／y2 當含端點畫（實機截圖量出金框比 x2−x1 多 1 邏輯 px）
        assert r.input_box_screen_rect() == (749, 894, 734, 46)
        assert r._client.root_window.searches == 1
        assert r.input_box_screen_rect() == (749, 894, 734, 46)
        assert r._client.root_window.searches == 1  # 沿用 input_open 的節點快取
    finally:
        r._loop.close()


def test_input_box_screen_rect_none_when_node_read_fails(monkeypatch):
    node = _Node((0, 190, 523, 222))
    r = _reader(monkeypatch, node)
    try:
        node.fail = True
        assert r.input_box_screen_rect() is None
        assert r._edit_node is None  # 失效節點丟掉，下一輪重找
    finally:
        r._loop.close()


def test_input_box_screen_rect_none_when_not_connected():
    assert WizChatReader().input_box_screen_rect() is None
