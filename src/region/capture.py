"""遊戲視窗畫面擷取：分兩步 —— `capture_window` 對遊戲 HWND 用 PrintWindow 拍一次完整的
client 區畫面，凍結成 `Frame`；`crop_frame` 再從這份凍結畫面依框選矩形裁成 PNG。

分兩步是為了讓選取層顯示的畫面跟最終送去辨識的畫面是同一幀 —— 使用者拖曳框選矩形時看到
的背景，必須跟放開滑鼠後裁下來的內容完全一致；等放開滑鼠才重新拍一次的話，遊戲畫面這段
時間可能已經變了，跟使用者選取當下看到的不一樣（也可能剛好拍到疊加層自己的東西）。
只拍遊戲視窗自己畫的東西：疊加視窗、翻譯輸入框、結果卡片與其他程式的視窗都不會入鏡，也
不必「先藏視窗再拍」。座標換算（`window_region`）與裁切（`crop_frame`）都是純函式（可測），
Win32 那層集中在 `capture_window`。
"""
import ctypes
import io
from dataclasses import dataclass

import win32gui
import win32ui
from PIL import Image

from src.log import log

# Win 8.1 起的旗標：連 DirectX 畫的內容也交給 DWM 渲染進 DC，沒有它 DX 視窗會拍到全黑
_PW_RENDERFULLCONTENT = 0x2


class CaptureError(Exception):
    """擷取失敗：矩形落在遊戲視窗外、PrintWindow 回失敗、或拍到整張空白。"""


class SelectionOutsideGame(CaptureError):
    """框選矩形完全落在遊戲 client 區之外。"""


@dataclass(frozen=True)
class Frame:
    """一次 `capture_window` 拍到的完整遊戲 client 區畫面 —— 選取層顯示它、`crop_frame`
    從它裁切，兩邊用的是同一幀，不會受擷取之後遊戲又畫了新內容影響。"""

    image: Image.Image
    client_origin: tuple[int, int]
    client_size: tuple[int, int]


def window_region(screen_rect: tuple[int, int, int, int], client_origin: tuple[int, int],
                  client_size: tuple[int, int]) -> tuple[int, int, int, int] | None:
    """螢幕矩形 (x, y, w, h) → 遊戲 client 內的矩形；超出 client 的部分裁掉，
    完全落在外面回 None。"""
    x, y, w, h = screen_rect
    ox, oy = client_origin
    cw, ch = client_size
    left, top = max(x - ox, 0), max(y - oy, 0)
    right, bottom = min(x - ox + w, cw), min(y - oy + h, ch)
    if right <= left or bottom <= top:
        return None
    return left, top, right - left, bottom - top


def capture_window(hwnd: int) -> Frame:
    """把遊戲視窗 hwnd 的整個 client 區拍成一張 `Frame`，做為這次框選要用的凍結畫面。

    整段 Win32／PIL 呼叫集中在這個 try：熱鍵觸發到真正擷取之間遊戲視窗可能已經消失或
    改變，`ClientToScreen`／`GetWindowRect`／`PrintWindow` 任一個都可能拋 pywintypes 或
    win32ui 的例外、裁切也可能因為尺寸不合拋 ValueError —— 這些都不是呼叫端該處理的型別，
    一律包成 CaptureError 讓 region_flow 能用單一 except 接住。
    """
    try:
        client_origin = win32gui.ClientToScreen(hwnd, (0, 0))
        _, _, client_w, client_h = win32gui.GetClientRect(hwnd)
        win_left, win_top, win_right, win_bottom = win32gui.GetWindowRect(hwnd)
        image = _print_window(hwnd, win_right - win_left, win_bottom - win_top)
        # PrintWindow 的原點是視窗外框左上角，client 區再往內偏一段邊框
        dx, dy = client_origin[0] - win_left, client_origin[1] - win_top
        client_image = image.crop((dx, dy, dx + client_w, dy + client_h))
        if client_image.getextrema() == ((0, 0), (0, 0), (0, 0)):
            raise CaptureError(f"blank capture (hwnd={hwnd:#x})")
    except CaptureError:
        raise
    except Exception as exc:
        raise CaptureError(f"capture failed (hwnd={hwnd:#x}): "
                           f"{type(exc).__name__}: {exc}") from exc
    log(f"[region] frame captured (hwnd={hwnd:#x}, client={client_origin}, "
        f"size={client_w}x{client_h})")
    return Frame(client_image, client_origin, (client_w, client_h))


def crop_frame(frame: Frame, screen_rect: tuple[int, int, int, int]) -> bytes:
    """從已經凍結的 `Frame` 裁出框選矩形，編碼成 PNG bytes；不碰 Win32，畫面在
    `capture_window` 當下就已經定格，這裡只需要座標換算與裁切。"""
    region = window_region(screen_rect, frame.client_origin, frame.client_size)
    if region is None:
        raise SelectionOutsideGame(
            f"selection outside game window (rect={screen_rect}, "
            f"client={frame.client_origin + frame.client_size})")
    rx, ry, rw, rh = region
    cropped = frame.image.crop((rx, ry, rx + rw, ry + rh))
    buffer = io.BytesIO()
    cropped.save(buffer, "PNG")
    log(f"[region] region cropped (rect={screen_rect}, region={region}, "
        f"png_bytes={buffer.tell()})")
    return buffer.getvalue()


def _print_window(hwnd: int, width: int, height: int) -> Image.Image:
    """整個視窗（含外框）的 PrintWindow 結果；GDI 物件在 finally 一律釋放。"""
    hwnd_dc = win32gui.GetWindowDC(hwnd)
    src_dc = win32ui.CreateDCFromHandle(hwnd_dc)
    mem_dc = src_dc.CreateCompatibleDC()
    bitmap = win32ui.CreateBitmap()
    try:
        bitmap.CreateCompatibleBitmap(src_dc, width, height)
        mem_dc.SelectObject(bitmap)
        ok = ctypes.windll.user32.PrintWindow(hwnd, mem_dc.GetSafeHdc(), _PW_RENDERFULLCONTENT)
        if not ok:
            raise CaptureError(f"PrintWindow failed (hwnd={hwnd:#x}, size={width}x{height})")
        info = bitmap.GetInfo()
        if info["bmWidth"] != width or info["bmHeight"] != height:
            log(f"[region] bitmap size {info['bmWidth']}x{info['bmHeight']} differs from "
                f"window size {width}x{height}; crop may be off (DPI scaling?)")
        data = bitmap.GetBitmapBits(True)
        return Image.frombuffer("RGB", (info["bmWidth"], info["bmHeight"]), data,
                                "raw", "BGRX", 0, 1).copy()
    finally:
        win32gui.DeleteObject(bitmap.GetHandle())
        mem_dc.DeleteDC()
        src_dc.DeleteDC()
        win32gui.ReleaseDC(hwnd, hwnd_dc)
