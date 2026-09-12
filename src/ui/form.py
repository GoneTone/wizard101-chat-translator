"""精靈與設定視窗共用的小元件：顏色常數、說明文字、連結標籤、結果標籤、
背景執行緒結果的輪詢，以及把翻譯例外轉成文案 key。不含任何完整欄位群。
"""
import queue
import tkinter as tk
import webbrowser
from tkinter import ttk

from src.i18n import current_language, t, translators
from src.translation.translator import TranslatorConfigError, TranslatorOffline
from src.ui.responsive import bind_wrap
from src.ui.richtext import LINK_COLOR, RichLabel, parse_link_markup
from src.updater import CROWDIN_URL

# 欄位標籤欄的字元寬：標籤、模型欄與欄位說明共用同一個值才對得齊
LABEL_WIDTH = 14

# 精靈、設定視窗與輸入框共用的字色：欄位說明的灰、成功綠、失敗紅（連結藍見 richtext）
HINT_COLOR = "#888888"
OK_COLOR = "#2e8b57"
ERROR_COLOR = "#cc3333"


def show_outcome(label, ok: bool, message: str) -> None:
    """把一次操作的結果寫進標籤：成功「✓ 」綠字、失敗「✗ 」紅字
    （測試連線與檢查更新共用同一種呈現；前者是 RichLabel、後者是 ttk.Label）。"""
    text, color = ("✓ " if ok else "✗ ") + message, OK_COLOR if ok else ERROR_COLOR
    if isinstance(label, RichLabel):
        label.set(text, color)
    else:
        label.configure(text=text, foreground=color)


def link_label(parent, text: str, url: str) -> ttk.Label:
    """藍字可點的連結標籤：點擊以系統瀏覽器開啟 url。"""
    label = ttk.Label(parent, text=text, foreground=LINK_COLOR, cursor="hand2")
    label.bind("<Button-1>", lambda e: webbrowser.open(url))
    return label


def linked_text(parent, text: str) -> ttk.Frame:
    """把一行帶行內連結的文字排成一列標籤：文字段是一般標籤，連結段是 link_label。
    刻意不自動換行 —— 用它的是譯者掛名這類短句，折行的複雜度換不到什麼。"""
    row = ttk.Frame(parent)
    for segment, url in parse_link_markup(text):
        if url is None:
            ttk.Label(row, text=segment).pack(side="left")
        else:
            link_label(row, segment, url).pack(side="left")
    return row


def translators_row(parent) -> ttk.Frame | None:
    """目前介面語言的譯者掛名列（灰標籤 ＋ 可能帶連結的名單）；沒有掛名回 None。
    給介面語言下拉的正下方用 —— 設定視窗與精靈各一處，掛名屬於選到的那個語言。"""
    credit = translators(current_language())
    if not credit:
        return None
    row = ttk.Frame(parent)
    ttk.Label(row, text=t("credit.translators"),
              foreground=HINT_COLOR).pack(side="left")
    linked_text(row, credit).pack(side="left", padx=(6, 0))
    return row


def help_translate_link(parent) -> ttk.Label:
    """邀請協助翻譯的連結，點開 Crowdin 專案頁。接在譯者掛名列之下，
    但不看有沒有掛名 —— 沒人翻的語言更需要這個入口。"""
    return link_label(parent, t("credit.help_translate"), CROWDIN_URL)


def hint_label(parent, text: str, *, trailing: int = 8) -> ttk.Label:
    """灰色說明文字，換行寬度跟著父容器走；呼叫端自己 pack／grid。
    設定視窗與精靈裡每一句說明都長這樣，集中在這裡免得八處各抄一份。"""
    label = ttk.Label(parent, text=text, foreground=HINT_COLOR, justify="left")
    bind_wrap(label, trailing=trailing)
    return label


def poll_queue(widget, result_queue: queue.Queue, on_result, interval_ms: int = 100):
    """輪詢背景執行緒放進 queue 的結果，取到就在主執行緒交給 on_result。
    tkinter 的 after 不保證跨執行緒安全：worker 只放 queue，由主執行緒輪詢取用。
    等待期間視窗被關閉即停止輪詢、結果丟棄。"""
    try:
        if not widget.winfo_exists():
            return
    except tk.TclError:
        return
    try:
        result = result_queue.get_nowait()
    except queue.Empty:
        widget.after(interval_ms,
                     lambda: poll_queue(widget, result_queue, on_result, interval_ms))
        return
    on_result(result)


def friendly_error(exc: Exception) -> tuple[str, dict]:
    """把翻譯例外轉成（文案 key，format 變數）；顯示端一律 `t(key, **kwargs)`。
    有 API 說明（detail）就照實顯示 —— 狀態碼猜的提示會誤導（自架端點 404 多半是
    網址路徑錯而非模型錯），只在 API 什麼都沒說時才退回用狀態碼猜。"""
    if isinstance(exc, (TranslatorConfigError, TranslatorOffline)) and exc.detail:
        if exc.status is not None:
            return "error.api_response", {"status": exc.status, "message": exc.detail}
        return "error.offline_detail", {"message": exc.detail}
    if isinstance(exc, TranslatorConfigError):
        if exc.status in (401, 403):
            return "error.bad_key", {}
        if exc.status == 404:
            return "error.model_not_found", {}
        return "error.api_http", {"status": exc.status}
    if isinstance(exc, TranslatorOffline):
        return "error.offline", {}
    return "error.unexpected", {"error": exc}
