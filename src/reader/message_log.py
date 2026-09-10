"""messages.log：逐輪記錄 chatLog 的原始內容與 reader 的判定結果，供事後 debug。

原文不清理不過濾（只排除遊戲自己的除錯行，見 is_debug_line），與 `mem_reader` 送去
翻譯的乾淨行（OUT）並排，一眼對照「遊戲送進來什麼 → 走哪條判定 → 實際翻了哪幾行」。
內容沒變的輪不留痕跡（掛機不長檔案）。
"""
import re
from collections import Counter

# 遊戲把自己的除錯輸出（貼圖載入失敗、音效通道回收等）也灌進 chatLog，量遠大於聊天；
# 這些行以 [WARN]／[ERRO]／[DBGM] 之類全大寫標籤開頭，聊天與系統訊息則以 <color;..> 起頭。
_DEBUG_LINE = re.compile(r"\s*\[[A-Z]{3,5}\]")


def is_debug_line(raw: str) -> bool:
    """這行是遊戲自己的除錯輸出（而非聊天）嗎？"""
    return _DEBUG_LINE.match(raw) is not None


def new_raw_lines(prev: list[str], cur: list[str]) -> list[str]:
    """本輪相對上一輪新出現的原始行（保持出現順序）。
    以出現次數差分而非集合：同一句被講第二次（lol／gg）是真的新訊息，集合會整句吃掉。"""
    remaining = Counter(prev)
    fresh = []
    for line in cur:
        if remaining.get(line):
            remaining[line] -= 1
        else:
            fresh.append(line)
    return fresh


class MessageLog:
    """把每輪的原始快照與判定結果寫進已開好的串流（時戳由 TimestampedStream 補）。"""

    def __init__(self, stream):
        self._stream = stream
        self._prev: list[str] = []
        self._poll = 0
        self._changed = False

    def snapshot(self, raw_lines: list[str], *, nodes: int, sizes_fn,
                 input_open: bool) -> None:
        """記錄本輪讀到的原始 chatLog 內容（解析之前，只濾掉遊戲自己的除錯行）。
        `sizes_fn` 回各節點的行數，只在內容有變、真的要寫一筆時才呼叫：它要整段重新
        解析 markup，掛機時每輪白算等於解析成本翻倍。"""
        self._poll += 1
        raw_lines = [line for line in raw_lines if not is_debug_line(line)]
        self._changed = raw_lines != self._prev
        if not self._changed:
            return
        fresh = new_raw_lines(self._prev, raw_lines)
        self._prev = list(raw_lines)
        # 快照變了但沒有新行（重排／重新染色／內容變少）也要留一筆：視圖切換看得見
        self._write(f"[poll={self._poll} nodes={nodes} sizes={sizes_fn()} "
                    f"lines={len(raw_lines)} input_open={input_open} new={len(fresh)}]")
        for line in fresh:
            self._write(f"  RAW {line}")

    def decision(self, path: str, appended: int | None, emitted: list[str]) -> None:
        """記錄本輪的判定路徑與最終送去翻譯的行。"""
        if not self._changed and not emitted:
            return
        counts = f"appended={appended} " if appended is not None else ""
        self._write(f"[poll={self._poll} path={path} {counts}emitted={len(emitted)}]")
        for text in emitted:
            self._write(f"  OUT {text}")

    def _write(self, line: str) -> None:
        self._stream.write(line + "\n")
