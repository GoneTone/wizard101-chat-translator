"""螢幕查詢：某個座標所在那顆螢幕的工作區與整個螢幕矩形（多螢幕時貼齊用的邊界要跟著
遊戲所在的螢幕走，不能用主螢幕尺寸）。"""
import win32api
import win32con


def _monitor_info(x: int, y: int) -> dict:
    monitor = win32api.MonitorFromPoint((x, y), win32con.MONITOR_DEFAULTTONEAREST)
    return win32api.GetMonitorInfo(monitor)


def work_area_at(x: int, y: int) -> tuple[int, int, int, int]:
    """含 (x, y) 那顆螢幕的工作區 (x, y, w, h)（去掉工作列）。"""
    left, top, right, bottom = _monitor_info(x, y)["Work"]
    return left, top, right - left, bottom - top


def monitor_rect_at(x: int, y: int) -> tuple[int, int, int, int]:
    """含 (x, y) 那顆螢幕的整個矩形 (x, y, w, h)（含工作列；全螢幕選取層要蓋滿它）。"""
    left, top, right, bottom = _monitor_info(x, y)["Monitor"]
    return left, top, right - left, bottom - top
