"""收訊與發話共用的近期聊天原文行緩衝。

上下文由 reader 的「讀取順序」推進，不是由「翻譯成功」推進 —— 平行翻譯時
同一批訊息才看得到彼此（提示詞用的是原文，不需要等前面幾則翻完）。
reader 執行緒、翻譯 worker、發話執行緒共用同一份，故所有存取持鎖。
"""
import threading
from collections import deque

# 帶進提示詞的近期對話行數：短窗涵蓋眼前的對話線，避免遠處舊話題污染判斷。
CONTEXT_LINES = 8


class ChatContext:
    """近期聊天原文行的環狀緩衝，執行緒安全。"""

    def __init__(self, max_lines: int = CONTEXT_LINES):
        self._lines: deque[str] = deque(maxlen=max_lines)
        self._lock = threading.Lock()

    def push(self, line: str) -> None:
        """把一行原文加入上下文（超出容量時擠掉最舊的）。"""
        with self._lock:
            self._lines.append(line)

    def snapshot(self) -> list[str]:
        """取得目前上下文的複本。呼叫端可長期持有：後續 push 不會影響已取出的快照。"""
        with self._lock:
            return list(self._lines)
