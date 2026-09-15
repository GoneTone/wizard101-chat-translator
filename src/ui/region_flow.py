"""框選翻譯的主流程：熱鍵 → 選取層 → 擷取 → 背景辨識翻譯 → 結果卡片。

所有公開方法都在 Tk 主執行緒呼叫（熱鍵回呼經 ui_queue 排進來）；辨識翻譯跑背景執行緒，
結果經 ui_queue 回主執行緒。每次框選 session +1，過期結果丟掉（與 InputBox 同一招）。
選取層、卡片、擷取、螢幕查詢都可注入替身供測試。
"""
import queue
import threading
import tkinter as tk
import traceback

from src.composer.paste import force_foreground
from src.i18n import t
from src.log import log
from src.region.capture import CaptureError, SelectionOutsideGame, capture_region
from src.region.ocr import OcrUnavailable
from src.translation.translator import TranslatorBadOutput, TranslatorError
from src.ui.form import friendly_error
from src.ui.monitors import monitor_rect_at
from src.ui.region_card import RegionCard
from src.ui.region_select import RegionSelector


def describe_error(exc: Exception) -> str:
    """辨識翻譯的例外 → 卡片上的一句話（已經 t() 過）。
    翻譯端的錯誤沿用設定視窗測試連線那套文案（狀態碼與 API 說明照實顯示）。"""
    if isinstance(exc, OcrUnavailable):
        return t("region.ocr_unavailable")
    if isinstance(exc, TranslatorBadOutput):
        return t("region.failed", error=exc)
    if isinstance(exc, TranslatorError):
        key, kwargs = friendly_error(exc)
        return t(key, **kwargs)
    return t("error.unexpected", error=exc)


class RegionFlow:
    """一次一個框選；`toggle(game_hwnd)` 是熱鍵的入口（開層／取消層）。"""

    def __init__(self, root: tk.Tk, pipeline, ui_queue: queue.Queue, alpha: float,
                 selector=None, card=None, capture=capture_region,
                 monitor_at=monitor_rect_at, foreground=force_foreground):
        self._pipeline = pipeline
        self._queue = ui_queue
        self._selector = selector if selector is not None else RegionSelector(root)
        self._card = card if card is not None else RegionCard(root, alpha)
        self._capture = capture
        self._monitor_at = monitor_at
        self._foreground = foreground
        self._session = 0
        self._thread: threading.Thread | None = None

    @property
    def is_selecting(self) -> bool:
        """選取層是否開著。熱鍵執行緒也會讀（此時前景是選取層而非遊戲）。"""
        return self._selector.is_open

    def set_alpha(self, alpha: float) -> None:
        self._card.set_alpha(alpha)

    def toggle(self, game_hwnd: int) -> None:
        """熱鍵：選取層開著就取消；否則收掉舊卡片、在遊戲所在的螢幕開選取層。"""
        if self._selector.is_open:
            log("[region] selection cancelled by hotkey")
            self._selector.cancel()
            return
        self._card.hide()
        x, y = _window_center(game_hwnd)
        log(f"[region] selection started (game_hwnd={game_hwnd:#x})")
        self._selector.show(self._monitor_at(x, y),
                            on_select=lambda rect: self._selected(rect, game_hwnd),
                            on_cancel=lambda: log("[region] selection cancelled"))

    def _selected(self, rect: tuple[int, int, int, int], game_hwnd: int) -> None:
        self._foreground(game_hwnd)
        self._card.show_pending(rect)
        try:
            png = self._capture(game_hwnd, rect)
        except SelectionOutsideGame as exc:
            log(f"[region] capture failed (hwnd={game_hwnd:#x}, rect={rect}): {exc}")
            self._card.show_error(t("region.outside_game"))
            return
        except CaptureError as exc:
            log(f"[region] capture failed (hwnd={game_hwnd:#x}, rect={rect}): {exc}")
            self._card.show_error(t("region.capture_failed", error=exc))
            return
        self._session += 1
        session = self._session
        self._thread = threading.Thread(target=self._worker, args=(png, rect, session),
                                        daemon=True)
        self._thread.start()

    def _worker(self, png: bytes, rect: tuple[int, int, int, int], session: int) -> None:
        try:
            result = self._pipeline.run(png, rect)
        except Exception as exc:
            if not isinstance(exc, (TranslatorError, TranslatorBadOutput, OcrUnavailable)):
                log(f"[region] unexpected failure (rect={rect}): {type(exc).__name__}: {exc}\n"
                    f"{traceback.format_exc()}")
            else:
                log(f"[region] translate failed (rect={rect}): {type(exc).__name__}: {exc}")
            message = describe_error(exc)
            self._queue.put(lambda: self._show_error(message, session))
            return
        self._queue.put(lambda: self._show_result(result.text, session))

    def _show_result(self, text: str, session: int) -> None:
        if session != self._session:
            log(f"[region] stale result dropped (session={session}, current={self._session})")
            return
        self._card.show_text(text)

    def _show_error(self, message: str, session: int) -> None:
        if session != self._session:
            return
        self._card.show_error(message)


def _window_center(hwnd: int) -> tuple[int, int]:
    """遊戲視窗的中心點（決定選取層要蓋哪顆螢幕）；查不到就用 (0, 0)＝主螢幕。"""
    try:
        import win32gui
        left, top, right, bottom = win32gui.GetWindowRect(hwnd)
        return (left + right) // 2, (top + bottom) // 2
    except Exception as exc:
        log(f"[region] GetWindowRect failed (hwnd={hwnd:#x}): {exc}")
        return 0, 0
