"""overlay 標題列下方的分頁列：照抄遊戲聊天分頁按鈕的文字，高亮目前分頁，不可點擊。"""
import tkinter as tk

from src.ui.fonts import ui_font
from src.ui.palette import TAB_ACCENT, TAB_BG, TAB_FG, TAB_FG_ACTIVE

_ACCENT_HEIGHT = 2


class TabBar:
    """分頁列元件；沒有分頁狀態時整列隱藏。文字是遊戲內容，不經 i18n。"""

    def __init__(self, parent: tk.Misc):
        # re-pack 會排到最後（訊息區之下）：記下建構當下最後一個 pack 的兄弟，顯示時接在它後面
        slaves = parent.pack_slaves()
        self._after = slaves[-1] if slaves else None
        self._frame = tk.Frame(parent, bg=TAB_BG)
        self._labels: list[tuple[str, bool]] = []

    def set(self, tabs: tuple[tuple[int, str], ...], current: int | None) -> None:
        """換成新的分頁清單；`tabs` 為空時隱藏整列。"""
        labels = [(text, index == current) for index, text in tabs]
        if labels == self._labels:
            return
        self._labels = labels
        for child in self._frame.winfo_children():
            child.destroy()
        if not labels:
            self._frame.pack_forget()
            return
        for text, active in labels:
            cell = tk.Frame(self._frame, bg=TAB_BG)
            cell.pack(side="left", padx=(6, 0))
            tk.Label(cell, text=text, bg=TAB_BG, fg=TAB_FG_ACTIVE if active else TAB_FG,
                     font=ui_font(8)).pack(side="top")
            tk.Frame(cell, height=_ACCENT_HEIGHT,
                     bg=TAB_ACCENT if active else TAB_BG).pack(side="top", fill="x")
        if self._after is not None:
            self._frame.pack(side="top", fill="x", after=self._after)
        else:
            self._frame.pack(side="top", fill="x")

    def labels(self) -> list[tuple[str, bool]]:
        """目前顯示的（文字, 是否目前分頁）；測試用。"""
        return list(self._labels)
