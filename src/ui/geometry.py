"""無邊框視窗拖曳、縮放與命中判定的純幾何函式（overlay 本體與泡泡共用，不碰 Tk）。"""

EDGE = 6        # 四邊的縮放感應寬度（px）
_CORNER = 14     # 四角的縮放感應範圍（px）：比邊寬，角落才好抓
_CLICK_THRESHOLD = 5


def moved_to(start_x: int, start_y: int, dx: int, dy: int) -> tuple[int, int]:
    """拖曳位移後的新左上角座標。"""
    return start_x + dx, start_y + dy


def point_in_rect(px: int, py: int, x: int, y: int, w: int, h: int) -> bool:
    """(px, py) 是否落在左上角 (x, y)、寬 w 高 h 的矩形內（含邊界）。"""
    return x <= px <= x + w - 1 and y <= py <= y + h - 1


def is_click(dx: int, dy: int, threshold: int = _CLICK_THRESHOLD) -> bool:
    """按下到放開的位移是否算點擊（否則視為拖曳）。"""
    return abs(dx) < threshold and abs(dy) < threshold


def edge_at(px: int, py: int, x: int, y: int, w: int, h: int,
            edge: int = EDGE, corner: int = _CORNER) -> str:
    """游標壓在視窗的哪一條邊／哪個角：`"n"`／`"se"`…，都不是則空字串。
    角落的判定帶比邊寬，且兩軸都落在角落帶內才算角——否則靠近角的邊會很難單軸縮放。"""
    if not point_in_rect(px, py, x, y, w, h):
        return ""
    left, right = px - x, x + w - 1 - px
    top, bottom = py - y, y + h - 1 - py
    vertical = "n" if top < corner else ("s" if bottom < corner else "")
    horizontal = "w" if left < corner else ("e" if right < corner else "")
    if vertical and horizontal:
        return vertical + horizontal
    if top < edge:
        return "n"
    if bottom < edge:
        return "s"
    if left < edge:
        return "w"
    if right < edge:
        return "e"
    return ""


def resized_edge(edge: str, x: int, y: int, w: int, h: int, dx: int, dy: int,
                 min_w: int, min_h: int) -> tuple[int, int, int, int]:
    """從某條邊／角拖曳 (dx, dy) 後的新幾何 (x, y, w, h)。

    拉左緣／上緣要同時改位置與尺寸，對邊才會留在原處；寬高撞到最小值後位置就凍住，
    否則游標繼續往內移會把整個視窗一起拖走。"""
    if "e" in edge:
        w = max(min_w, w + dx)
    elif "w" in edge:
        new_w = max(min_w, w - dx)
        x += w - new_w
        w = new_w
    if "s" in edge:
        h = max(min_h, h + dy)
    elif "n" in edge:
        new_h = max(min_h, h - dy)
        y += h - new_h
        h = new_h
    return x, y, w, h
