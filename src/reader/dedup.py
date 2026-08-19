"""聊天行去重:記住最近 N 行,模糊比對吸收 OCR 抖動。"""
from collections import deque
from difflib import SequenceMatcher


class LineDeduper:
    def __init__(self, max_seen: int = 200, similarity: float = 0.9):
        self._seen: deque[str] = deque(maxlen=max_seen)
        self._similarity = similarity

    def _is_seen(self, line: str) -> bool:
        return any(
            SequenceMatcher(None, line, old).ratio() >= self._similarity
            for old in self._seen
        )

    def new_lines(self, lines: list[str]) -> list[str]:
        fresh: list[str] = []
        for raw in lines:
            line = raw.strip()
            if not line or self._is_seen(line):
                continue
            self._seen.append(line)
            fresh.append(line)
        return fresh

    def forget(self, lines: list[str]) -> None:
        """把指定行從已見集合移除,讓它們下次可再被視為新行(例如翻譯失敗需要重試)。"""
        to_forget = set(lines)
        if not to_forget:
            return
        remaining = [line for line in self._seen if line not in to_forget]
        self._seen.clear()
        self._seen.extend(remaining)
