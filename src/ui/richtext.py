"""文字中帶可點連結的顯示元件與其純邏輯。

`[文字](網址)` 是語言檔用的行內連結語法（parse_link_markup）；API 回傳的錯誤訊息
則常夾著裸網址（「You can find your API key at https://…」），linkify 先把它們
轉成同一種語法，顯示端只需認一種格式。
"""
import re
import tkinter as tk
import webbrowser
from tkinter import font as tkfont
from tkinter import ttk

from src.log import log

# 淺色設定視窗上的連結藍（overlay 的深色底另有 palette.FG_UPDATE）
LINK_COLOR = "#4a7ddc"

_LINK_MARKUP = re.compile(r"\[([^\]\n]+)\]\(([^)\s]+)\)")
_SAFE_SCHEMES = ("http://", "https://")
# 網址只收 RFC 3986 的 ASCII 字元：訊息裡的網址常緊接中文或全形標點（「…keys，請檢查」），
# 用「非空白」當邊界會把後面整句吃進連結。括號與方括號也排除，免得與連結語法打架。
_BARE_URL = re.compile(r"https?://[A-Za-z0-9\-._~:/?#@!$&'*+,;=%]+", re.IGNORECASE)
_URL_TRAILING_PUNCT = ".,;:!?'"


def parse_link_markup(text: str) -> list[tuple[str, str | None]]:
    """把 `[文字](網址)` 的行內連結語法切成（顯示文字，網址或 None）的段落。
    只認連結一種語法，其餘字元原樣留在文字段裡。

    網址只放行 http／https：語言檔可以由外部貢獻者提供，其他 scheme（`file:`、
    `javascript:`）降級成不可點的純文字 —— 寧可少一條連結，也不要讓一份譯文開得了
    任意 URI。"""
    segments: list[tuple[str, str | None]] = []
    cursor = 0
    for match in _LINK_MARKUP.finditer(text):
        if match.start() > cursor:
            segments.append((text[cursor:match.start()], None))
        label, url = match.group(1), match.group(2)
        if url.lower().startswith(_SAFE_SCHEMES):
            segments.append((label, url))
        else:
            log(f"[ui] link markup rejected: scheme={url.split(':', 1)[0][:16]}")
            segments.append((label, None))
        cursor = match.end()
    if cursor < len(text):
        segments.append((text[cursor:], None))
    return segments


def linkify(text: str) -> str:
    """把文字中的裸 http／https 網址包成 `[網址](網址)`；已經是連結語法的不重複包。
    句尾標點不算網址的一部分（「…at https://a.example/keys.」的句點）。"""
    def wrap(match: re.Match) -> str:
        whole = match.group(0)
        before = text[max(0, match.start() - 2):match.start()]
        if before.endswith(("](", "[")):
            return whole
        url = whole.rstrip(_URL_TRAILING_PUNCT)
        return f"[{url}]({url}){whole[len(url):]}"
    return _BARE_URL.sub(wrap, text)


def ttk_background(widget: tk.Misc) -> str:
    """ttk 主題的底色：RichLabel 是 tk 元件，放進 ttk 版面得自己配成同色才看不出邊界。"""
    return ttk.Style(widget).lookup("TFrame", "background") or "SystemButtonFace"


class RichLabel(tk.Text):
    """看起來像 Label、但文字中的網址可以點的唯讀文字框。

    tk.Label 的文字是一整塊，沒辦法讓其中一段可點；改用 Text 排版：依寬度自動換行、
    行數決定高度（跟 Label 一樣只佔內容需要的空間），網址段掛連結色與手指游標，
    點擊以系統瀏覽器開啟。文字先經 linkify 再依連結語法切段，所以固定文案也能用
    `[文字](網址)` 放連結。"""

    def __init__(self, parent, *, fg: str, bg: str, font, link_fg: str = LINK_COLOR,
                 on_height_change=None):
        super().__init__(parent, wrap="word", width=1, height=1, bd=0, relief="flat",
                         highlightthickness=0, padx=0, pady=0, bg=bg, fg=fg, font=font,
                         cursor="", takefocus=0, state="disabled")
        self._links: list[tuple[str, str]] = []
        self._ranges: list[tuple[str, str]] = []
        self._fit_pending = False
        self._on_height_change = on_height_change  # 行數變了才叫：外層據此重算視窗高度
        self._line_height = max(1, tkfont.Font(root=self, font=font).metrics("linespace"))
        self.tag_configure("link", foreground=link_fg, underline=True)
        self.tag_bind("link", "<Enter>", lambda e: self.configure(cursor="hand2"))
        self.tag_bind("link", "<Leave>", lambda e: self.configure(cursor=""))
        self.bind("<Configure>", lambda e: self._schedule_fit())

    def set(self, text: str, fg: str | None = None) -> None:
        """換掉整段文字（可一併換字色）；空字串＝清空、只留一行高。"""
        self.configure(state="normal")
        self.delete("1.0", "end")
        self._links, self._ranges = [], []
        for segment, url in parse_link_markup(linkify(text)):
            if url is None:
                self.insert("end", segment)
                continue
            tag = f"link{len(self._links)}"
            start = self.index("end-1c")
            self.insert("end", segment, ("link", tag))
            self.tag_bind(tag, "<Button-1>", lambda e, u=url: self._open(u))
            self._links.append((segment, url))
            self._ranges.append((start, self.index("end-1c")))
        if fg is not None:
            self.configure(fg=fg)
        self.configure(state="disabled")
        self._schedule_fit()

    def text(self) -> str:
        return self.get("1.0", "end-1c")

    def links(self) -> list[tuple[str, str]]:
        """（顯示文字，網址）列表，測試與除錯用。"""
        return list(self._links)

    def link_ranges(self) -> list[tuple[str, str]]:
        """各連結段的（起，迄）文字索引，測試用。"""
        return list(self._ranges)

    def _open(self, url: str) -> None:
        log(f"[ui] opening link from message: {url}")
        webbrowser.open(url)

    def _schedule_fit(self) -> None:
        # 排到 idle 才量行數：Configure 當下寬度還是過渡值；set 之後也要等排版完成。
        if self._fit_pending:
            return
        self._fit_pending = True
        self.after_idle(self._fit)

    def _fit(self) -> None:
        """把高度設成內容的行數：內容總像素高（count 帶 update，否則 Tk 背景慢慢算的
        行距還沒好）除以字型行高。不用 count displaylines —— 一個「字」（長網址）
        超過一行寬時 Tk 逐字元折行，displaylines 少算那一行，最後一行就被切掉
        （Tk 8.6.15 實測，帶 update 也一樣）。"""
        self._fit_pending = False
        if not self.winfo_exists():
            return
        pixels = _scalar(self.count("1.0", "end", "update", "ypixels"))
        lines = max(1, -(-pixels // self._line_height))
        if int(self.cget("height")) != lines:
            self.configure(height=lines)
            if self._on_height_change is not None:
                self._on_height_change()


def _scalar(value) -> int:
    """tkinter 的 Text.count 依版本回 int 或單元素 tuple。"""
    return int(value[0] if isinstance(value, tuple) else value or 0)
