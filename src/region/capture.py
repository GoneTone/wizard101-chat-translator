"""遊戲視窗畫面擷取：分兩步 —— `capture_window` 對遊戲 HWND 用 Windows Graphics Capture
拍一次完整的 client 區畫面，凍結成 `Frame`；`crop_frame` 再從這份凍結畫面依框選矩形裁成 PNG。
另外 `capture_screen` 拍一次整顆螢幕，純粹給選取層當暗化背景用，跟前兩者的用途分開。

分兩步是為了讓選取層顯示的畫面跟最終送去辨識的畫面是同一幀 —— 使用者拖曳框選矩形時看到
的背景，必須跟放開滑鼠後裁下來的內容完全一致；等放開滑鼠才重新拍一次的話，遊戲畫面這段
時間可能已經變了，跟使用者選取當下看到的不一樣（也可能剛好拍到疊加層自己的東西）。
只拍遊戲視窗自己畫的東西：疊加視窗、翻譯輸入框、結果卡片與其他程式的視窗都不會入鏡，也
不必「先藏視窗再拍」。座標換算（`window_region`）與裁切（`crop_frame`）都是純函式（可測），
Win32 那層集中在 `capture_window`／`capture_screen`。
"""
import ctypes
import io
import threading
import time
from ctypes import wintypes
from dataclasses import dataclass

import win32gui
from PIL import Image, ImageGrab
from windows_capture import WindowsCapture

from src.log import log

_DWMWA_EXTENDED_FRAME_BOUNDS = 9
_FRAME_TIMEOUT_S = 2.0


class CaptureError(Exception):
    """擷取失敗：矩形落在遊戲視窗外、等不到影格、或拍到整張單色。"""


class SelectionOutsideGame(CaptureError):
    """框選矩形完全落在遊戲 client 區之外。"""


@dataclass(frozen=True)
class Frame:
    """一次 `capture_window` 拍到的完整遊戲 client 區畫面，連同 client 區的螢幕位置與大小。"""

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
    """把遊戲視窗 hwnd 的整個 client 區拍成一張 `Frame`。
    Win32／WGC／PIL 任一層的例外（視窗消失、等不到影格、尺寸不合）一律包成 CaptureError。"""
    try:
        client_origin = win32gui.ClientToScreen(hwnd, (0, 0))
        _, _, client_w, client_h = win32gui.GetClientRect(hwnd)
        frame_left, frame_top = _frame_origin(hwnd)
        started = time.perf_counter()
        image = _grab_window(hwnd)
        elapsed_ms = round((time.perf_counter() - started) * 1000)
        # WGC 影格的原點是 DWM 看得見的外框左上角，client 區再往內偏一段標題列與邊框
        dx, dy = client_origin[0] - frame_left, client_origin[1] - frame_top
        if dx < 0 or dy < 0 or dx + client_w > image.width or dy + client_h > image.height:
            log(f"[region] frame {image.width}x{image.height} does not contain client "
                f"{client_w}x{client_h} at offset ({dx}, {dy}); crop may be off (DPI scaling?)")
        client_image = image.crop((dx, dy, dx + client_w, dy + client_h))
        extrema = client_image.getextrema()
        # 單色畫面（全白、全黑）代表擷取沒拿到遊戲內容，送去 OCR 只會辨識不到字
        if all(low == high for low, high in extrema):
            raise CaptureError(f"blank capture (hwnd={hwnd:#x}, "
                               f"colour={tuple(low for low, _ in extrema)})")
    except CaptureError:
        raise
    except Exception as exc:
        raise CaptureError(f"capture failed (hwnd={hwnd:#x}): "
                           f"{type(exc).__name__}: {exc}") from exc
    log(f"[region] frame captured (hwnd={hwnd:#x}, client={client_origin}, "
        f"size={client_w}x{client_h}, frame={image.width}x{image.height}, ms={elapsed_ms})")
    return Frame(client_image, client_origin, (client_w, client_h))


def capture_screen(monitor: tuple[int, int, int, int]) -> Image.Image:
    """把 monitor（螢幕矩形 x, y, w, h）整顆拍成圖片，只給選取層當暗化背景；
    失敗退回全黑，框選本身不受影響。"""
    x, y, w, h = monitor
    try:
        return ImageGrab.grab(bbox=(x, y, x + w, y + h), all_screens=True)
    except Exception as exc:
        log(f"[region] screen grab failed (monitor={monitor}): {type(exc).__name__}: {exc}")
        return Image.new("RGB", (w, h), "black")


def crop_frame(frame: Frame, screen_rect: tuple[int, int, int, int]) -> bytes:
    """從凍結的 `Frame` 裁出框選矩形（螢幕座標），編碼成 PNG bytes；純座標換算與裁切。"""
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


def _frame_origin(hwnd: int) -> tuple[int, int]:
    """DWM 看得見的視窗外框左上角（螢幕座標）；不含 GetWindowRect 算進去的隱形拖曳邊框。"""
    rect = wintypes.RECT()
    result = ctypes.windll.dwmapi.DwmGetWindowAttribute(
        wintypes.HWND(hwnd), _DWMWA_EXTENDED_FRAME_BOUNDS, ctypes.byref(rect), ctypes.sizeof(rect))
    if result != 0:
        raise CaptureError(f"DwmGetWindowAttribute failed (hwnd={hwnd:#x}, "
                           f"hresult={result & 0xFFFFFFFF:#010x})")
    return rect.left, rect.top


def _grab_window(hwnd: int) -> Image.Image:
    """用 Windows Graphics Capture 拍 hwnd 的一張影格（範圍是 DWM 看得見的外框）。

    擷取在套件自己的執行緒跑：程式已透過 pywin32 在主執行緒初始化 COM，直接在這裡
    啟動會報 Failed to initialize WinRT。"""
    arrived = threading.Event()
    grabbed: dict[str, object] = {}
    session = WindowsCapture(cursor_capture=False, draw_border=False, window_hwnd=hwnd)

    @session.event
    def on_frame_arrived(frame, control):
        if not arrived.is_set():
            try:
                # 影格緩衝只在回呼期間有效，先複製成圖片
                grabbed["image"] = Image.frombytes(
                    "RGB", (frame.width, frame.height), frame.frame_buffer.tobytes(),
                    "raw", "BGRX")
            except Exception as exc:
                grabbed["error"] = exc
            arrived.set()
        control.stop()

    @session.event
    def on_closed():
        arrived.set()

    control = session.start_free_threaded()
    try:
        if not arrived.wait(_FRAME_TIMEOUT_S):
            raise CaptureError(f"no frame within {_FRAME_TIMEOUT_S}s (hwnd={hwnd:#x})")
    finally:
        control.stop()
    if "error" in grabbed:
        raise CaptureError(f"frame conversion failed (hwnd={hwnd:#x}): "
                           f"{type(grabbed['error']).__name__}: {grabbed['error']}")
    if "image" not in grabbed:
        raise CaptureError(f"capture session closed before a frame arrived (hwnd={hwnd:#x})")
    return grabbed["image"]
