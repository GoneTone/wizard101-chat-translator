"""聊天行去重。
- 模糊模式(similarity < 1.0):記住最近 N 行,SequenceMatcher 吸收 OCR 抖動。
- 精確模式(similarity >= 1.0):精確比對成員;max_seen=None 時不設上限,
  適合記憶體讀取(每次掃描回傳大量行,看過的行永不重現)。"""
from collections import deque
from difflib import SequenceMatcher


class LineDeduper:
    def __init__(self, max_seen: int | None = 200, similarity: float = 0.9):
        self._max = max_seen
        self._similarity = similarity
        self._exact = similarity >= 1.0
        self._order: deque[str] = deque()  # 記錄插入順序供淘汰
        self._set: set[str] = set()        # 精確成員查詢

    def _is_seen(self, line: str) -> bool:
        if self._exact:
            return line in self._set
        return any(
            SequenceMatcher(None, line, old).ratio() >= self._similarity
            for old in self._order
        )

    def _remember(self, line: str) -> None:
        self._order.append(line)
        self._set.add(line)
        if self._max is not None:
            while len(self._order) > self._max:
                self._set.discard(self._order.popleft())

    def new_lines(self, lines: list[str]) -> list[str]:
        fresh: list[str] = []
        for raw in lines:
            line = raw.strip()
            if not line or self._is_seen(line):
                continue
            self._remember(line)
            fresh.append(line)
        return fresh

    def forget(self, lines: list[str]) -> None:
        """把指定行從已見集合移除,讓它們下次可再被視為新行(例如翻譯失敗需要重試)。"""
        to_forget = set(lines)
        if not to_forget:
            return
        self._set -= to_forget
        self._order = deque(line for line in self._order if line not in to_forget)
