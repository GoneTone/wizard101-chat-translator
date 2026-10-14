"""messages.log：逐筆記錄 AddLine hook 讀到的聊天行與 reader 的判定，供事後 debug。

原文不清理不過濾（只排除遊戲自己的除錯行，見 is_debug_line）；沒有事件的輪不留痕跡。
雙開時每個客戶端一個實例共寫同一串流，以 `[slot=N]` 前綴區分。
"""
import re

# 遊戲把自己的除錯輸出（貼圖載入失敗、音效通道回收等）也寫進聊天框，量遠大於聊天；
# 這些行以 [WARN]／[ERRO]／[DBGM] 之類全大寫標籤開頭，聊天與系統訊息則以 <color;..> 起頭。
_DEBUG_LINE = re.compile(r"\s*\[[A-Z]{3,5}\]")


def is_debug_line(raw: str) -> bool:
    """這行是遊戲自己的除錯輸出（而非聊天）嗎？"""
    return _DEBUG_LINE.match(raw) is not None


class MessageLog:
    """把每筆 hook 記錄與判定寫進已開好的串流（時戳由 TimestampedStream 補）。"""

    def __init__(self, stream, slot: int | None = None):
        self._stream = stream
        self._prefix = f"[slot={slot}] " if slot is not None else ""

    def record(self, seq: int, tab: int, raw: str, verdict: str) -> None:
        """記一筆 hook 記錄：序號、分頁、判定與原始 markup；除錯行不記。"""
        if not is_debug_line(raw):
            self._write(f"[seq={seq} tab={tab} {verdict}] {raw}")

    def note(self, text: str) -> None:
        """記一則不屬於單筆記錄的事件（遺失、截斷等）。"""
        self._write(text)

    def _write(self, line: str) -> None:
        self._stream.write(self._prefix + line + "\n")
