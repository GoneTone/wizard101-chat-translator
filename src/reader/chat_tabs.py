"""收訊端：讀遊戲聊天框的分頁狀態（目前分頁編號、各分頁按鈕上的文字）。

記憶體讀取以 `async (addr, size) -> bytes` 注入，不需遊戲即可測試。
"""
import struct
from typing import NamedTuple

CHATBOX_WINDOW = "WizardChatBox"
CUR_TAB_OFFSET = 0x7A0
LABEL_OFFSET = 0x2E0

# 依遊戲由左到右；編號 3、4、5–8 是靜態推論，待實機確認
TAB_BUTTONS: tuple[tuple[str, int], ...] = (
    ("MainFilterButton", 0),
    ("GroupFilterButton", 1),
    ("HouseFilterButton", 2),
    ("FriendFilterButton", 3),
    ("Channel1FilterButton", 5),
    ("Channel2FilterButton", 6),
    ("Channel3FilterButton", 7),
    ("Channel4FilterButton", 8),
    ("TeamUpFilterButton", 4),
)

MAX_LABEL_CHARS = 4096
SSO_CAPACITY = 8  # wchar_t 內嵌最多 7 字；capacity 達 8 時 +0 是堆積指標


class TabState(NamedTuple):
    """`tabs` 是目前可見分頁的（編號, 按鈕文字），依 `TAB_BUTTONS` 順序。"""
    current: int
    tabs: tuple[tuple[int, str], ...]


def visible_label(raw: str) -> str | None:
    """隱藏分頁的按鈕文字是未解析的 `<string;...>` token，回 None；其餘原樣回傳。"""
    return None if not raw or raw.startswith("<string;") else raw


async def read_wstring(read_bytes, addr: int) -> str:
    """讀 MSVC `std::wstring`；size 為 0 或超過上限回空字串。"""
    head = await read_bytes(addr, 0x20)
    size, capacity = struct.unpack_from("<QQ", head, 0x10)
    if not 0 < size <= MAX_LABEL_CHARS:
        return ""
    if capacity >= SSO_CAPACITY:
        raw = await read_bytes(struct.unpack_from("<Q", head)[0], size * 2)
    else:
        raw = head[:size * 2]
    return raw.decode("utf-16-le", "replace")
