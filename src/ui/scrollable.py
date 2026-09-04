"""可捲動的內容容器：內容超出可視高度時出現捲軸，而不是把同一個視窗裡的其他區塊擠掉。

tkinter 沒有現成的捲動容器，標準組合是 Canvas ＋ Scrollbar ＋ 一個裝內容的 Frame。
呼叫端把內容放進 `.body`，其餘交給本元件。
"""
import tkinter as tk
from tkinter import ttk


class ScrollableFrame(ttk.Frame):
    """垂直捲動容器：內容放進 `.body`。

    捲軸常駐，不隨內容多寡顯隱：`pack`／`pack_forget` 會改變 canvas 寬度，寬度一變
    又回呼到 `yscrollcommand`，在「加上捲軸就剛好放得下」的臨界高度會無限來回切換、
    把整個 UI 卡死（實測踩過）。常留一條捲軸換來可預測的版面，划算。
    """

    def __init__(self, parent, padding=0, **kwargs):
        super().__init__(parent, **kwargs)
        # tk.Canvas 不吃 ttk 主題，背景要自己對齊，否則捲動區會是一塊突兀的白底
        background = ttk.Style().lookup("TFrame", "background")
        self._canvas = tk.Canvas(self, highlightthickness=0, bd=0,
                                 background=background)
        self._scrollbar = ttk.Scrollbar(self, orient="vertical",
                                        command=self._canvas.yview)
        self._canvas.configure(yscrollcommand=self._scrollbar.set)
        # 捲軸先 pack：後宣告會被 expand=True 的 canvas 擠掉
        self._scrollbar.pack(side="right", fill="y")
        self._canvas.pack(side="left", fill="both", expand=True)

        self.body = ttk.Frame(self._canvas, padding=padding)
        self._body_id = self._canvas.create_window((0, 0), window=self.body,
                                                   anchor="nw")

        self.body.bind("<Configure>", self._on_body_configure)
        self._canvas.bind("<Configure>", self._on_canvas_configure)
        # 滑鼠移入才接管滾輪：同一個視窗裡有多個捲動區時，只有游標所在的那個該捲動
        self._canvas.bind("<Enter>", self._bind_wheel)
        self._canvas.bind("<Leave>", self._unbind_wheel)

    def content_overflows(self) -> bool:
        """內容是否超出可視高度（決定滾輪要不要作用）。"""
        first, last = self._canvas.yview()
        return not (first <= 0.0 and last >= 1.0)

    # --- 幾何同步 ---
    def _on_body_configure(self, _event=None) -> None:
        self._canvas.configure(scrollregion=self._canvas.bbox("all"))

    def _on_canvas_configure(self, event) -> None:
        """內層寬度跟著 canvas 走。不同步的話 body 會用自然寬度，
        bind_wrap 依容器寬度算出的換行寬度就會失準、說明文字水平溢出。"""
        self._canvas.itemconfigure(self._body_id, width=event.width)

    # --- 滾輪 ---
    def _bind_wheel(self, _event=None) -> None:
        self._canvas.bind_all("<MouseWheel>", self._on_wheel)

    def _unbind_wheel(self, _event=None) -> None:
        self._canvas.unbind_all("<MouseWheel>")

    def _on_wheel(self, event) -> None:
        if self.content_overflows():  # 內容放得下就別捲，免得畫面莫名抖動
            self._canvas.yview_scroll(-event.delta // 120, "units")
