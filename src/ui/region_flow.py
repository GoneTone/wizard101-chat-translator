"""框選翻譯的主流程：熱鍵 → 擷取一次凍結畫面 → 選取層 → 從凍結畫面裁切 → 背景辨識翻譯
→ 結果卡片。

所有公開方法都在 Tk 主執行緒呼叫（熱鍵回呼經 ui_queue 排進來）；辨識翻譯跑背景執行緒，
結果經 ui_queue 回主執行緒。每次框選 session +1，過期結果丟掉（與 InputBox 同一招）。
熱鍵一觸發就先拍好整個 client 區（`capture_window`），選取層顯示這張凍結畫面、放開滑鼠
後也是從同一張畫面裁切（`crop_frame`）—— 使用者選取當下看到的內容就是最後送去辨識的
內容，不會因為放開滑鼠才重新拍一次而跟遊戲當下的畫面產生落差。
選取層、卡片、擷取、裁切、螢幕查詢都可注入替身供測試。
"""
import queue
import threading
import tkinter as tk
import traceback

import win32gui

from src.composer.paste import force_foreground
from src.i18n import t
from src.log import log
from src.reader.process import find_game_window
from src.region.capture import (
    CaptureError,
    Frame,
    SelectionOutsideGame,
    capture_screen,
    capture_window,
    crop_frame,
)
from src.region.ocr import OcrUnavailable
from src.translation.translator import TranslatorBadOutput, TranslatorError
from src.ui.form import friendly_error
from src.ui.input_box import cursor_position
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
                 selector=None, card=None, capture_window=capture_window, crop=crop_frame,
                 capture_screen=capture_screen, monitor_at=monitor_rect_at,
                 foreground=force_foreground, find_game=find_game_window):
        self._pipeline = pipeline
        self._queue = ui_queue
        self._selector = selector if selector is not None else RegionSelector(root)
        self._card = card if card is not None else RegionCard(root, alpha)
        self._capture_window = capture_window
        self._crop = crop
        self._capture_screen = capture_screen
        self._monitor_at = monitor_at
        self._foreground = foreground
        self._find_game = find_game
        self._session = 0
        self._thread: threading.Thread | None = None
        self._game_hwnd = 0

    @property
    def is_selecting(self) -> bool:
        """選取層是否開著。熱鍵執行緒也會讀（此時前景是選取層而非遊戲）。"""
        return self._selector.is_open

    def set_alpha(self, alpha: float) -> None:
        self._card.set_alpha(alpha)

    def toggle(self, game_hwnd: int) -> None:
        """熱鍵：選取層開著就取消（觸發 `_cancelled` 還前景，見下）；否則收掉舊卡片、
        拍一張凍結畫面，在遊戲所在的螢幕開選取層顯示它。`game_hwnd == 0` 代表選取層已經
        關了（熱鍵取消跟滑鼠放開兩條路徑競速時可能發生），不開一顆綁著假 hwnd 的選取層。"""
        if self._selector.is_open:
            log("[region] selection cancelled by hotkey")
            self._selector.cancel()
            return
        if game_hwnd == 0:
            log("[region] toggle ignored: no game window (selector already closed)")
            return
        self._card.hide()
        self._game_hwnd = game_hwnd
        x, y = _window_center(game_hwnd)
        try:
            frame = self._capture_window(game_hwnd)
        except CaptureError as exc:
            # 選取層還沒開，遊戲仍是前景，不必像 _selected 那樣還前景
            log(f"[region] frame capture failed (hwnd={game_hwnd:#x}): {exc}")
            self._card.show_pending((x, y, 0, 0))
            self._card.show_error(t("region.capture_failed", error=exc))
            return
        monitor = self._monitor_at(x, y)
        backdrop = self._capture_screen(monitor)   # 失敗一律退回全黑，不會擋住框選
        log(f"[region] selection started (game_hwnd={game_hwnd:#x})")
        self._selector.show(monitor, frame,
                            on_select=lambda rect: self._selected(rect, game_hwnd, frame),
                            on_cancel=self._cancelled, backdrop=backdrop)

    def start_from_button(self) -> None:
        """標題列按鈕的入口：與熱鍵不同，按下當下遊戲多半不是前景視窗，
        `game_hwnd` 要靠列舉視窗找（見 `find_game_window`），找不到就在游標處顯示錯誤卡片。"""
        if self._selector.is_open:
            self.toggle(0)
            return
        hwnd = self._find_game()
        if hwnd is None:
            log("[region] no game window found for the overlay button")
            x, y = cursor_position()
            self._card.show_pending((x, y, 0, 0))
            self._card.show_error(t("region.no_game"))
            return
        self.toggle(hwnd)

    def _cancelled(self) -> None:
        """選取層被取消（Esc、右鍵、點一下沒拖動、或熱鍵取消都會觸發）：把前景還給遊戲
        —— 選取層開層時用 AttachThreadInput 搶走了前景，取消時換它換回來，否則使用者要先點
        一下遊戲視窗，下一次熱鍵才不會被 `on_region_hotkey` 當成「前景不是遊戲」擋掉。"""
        log("[region] selection cancelled")
        self._foreground(self._game_hwnd)

    def _selected(self, rect: tuple[int, int, int, int], game_hwnd: int, frame: Frame) -> None:
        self._foreground(game_hwnd)
        self._card.show_pending(rect)
        try:
            png = self._crop(frame, rect)
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
            result = self._pipeline.run(
                png, rect,
                progress=lambda stage: self._queue.put(lambda: self._show_stage(stage, session)))
        except Exception as exc:
            if not isinstance(exc, (TranslatorError, TranslatorBadOutput, OcrUnavailable)):
                log(f"[region] unexpected failure (rect={rect}): {type(exc).__name__}: {exc}\n"
                    f"{traceback.format_exc()}")
            else:
                log(f"[region] translate failed (rect={rect}): {type(exc).__name__}: {exc}")
            message = describe_error(exc)
            self._queue.put(lambda: self._show_error(message, session))
            return
        self._queue.put(lambda: self._show_result(result.text, result.source, session))

    def _show_result(self, text: str, source: str, session: int) -> None:
        if session != self._session:
            log(f"[region] stale result dropped (session={session}, current={self._session})")
            return
        self._card.show_text(text, source)

    def _show_stage(self, stage: str, session: int) -> None:
        """退回本機 OCR 時的階段更新（見 `RegionPipeline.run` 的 `progress`）：
        辨識中先顯示 `region.recognizing`，翻譯中換回既有的 `notice.pending`。"""
        if session != self._session:
            return
        text = t("region.recognizing") if stage == "recognizing" else t("notice.pending")
        self._card.show_stage(text)

    def _show_error(self, message: str, session: int) -> None:
        if session != self._session:
            return
        self._card.show_error(message)


def _window_center(hwnd: int) -> tuple[int, int]:
    """遊戲視窗的中心點（決定選取層要蓋哪顆螢幕）；查不到就用 (0, 0)＝主螢幕。"""
    try:
        left, top, right, bottom = win32gui.GetWindowRect(hwnd)
        return (left + right) // 2, (top + bottom) // 2
    except Exception as exc:
        log(f"[region] GetWindowRect failed (hwnd={hwnd:#x}): {exc}")
        return 0, 0
