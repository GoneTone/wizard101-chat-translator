"""蒐證探針:模擬 reader_loop 的完整讀取管線,但不翻譯 —— 把每輪的候選視窗、
選擇結果、將被翻譯的行全記到 JSONL,用來調查「冒舊訊息」時記憶體實際發生什麼。

用法:python scripts/probe_visible.py [秒數,預設 120]
跑的期間在遊戲裡正常聊天,重現「冒舊訊息」後停止;log 在 scripts/probe_visible.jsonl。
"""
import io
import json
import sys
import time
from pathlib import Path

sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from src.main import REANCHOR_AFTER, choose_window  # noqa: E402
from src.reader.mem_reader import GameNotRunning, VisibleReader  # noqa: E402

LOG = Path(__file__).with_suffix(".jsonl")


class ProbeReader(VisibleReader):
    """記錄每輪走了快掃還是全掃。"""

    def __init__(self):
        super().__init__()
        self.last_mode = "?"

    def _fast(self, h):
        self.last_mode = "fast"
        return super()._fast(h)

    def _full(self, h):
        self.last_mode = "full"
        return super()._full(h)


def main() -> None:
    duration = float(sys.argv[1]) if len(sys.argv) > 1 else 120.0
    reader = ProbeReader()
    prev: list[str] | None = None
    misaligned = 0
    t0 = time.time()
    n = 0
    with LOG.open("w", encoding="utf-8") as f:
        while time.time() - t0 < duration:
            rec: dict = {"t": round(time.time() - t0, 2), "poll": n}
            try:
                t_scan = time.time()
                cands = reader.read()
                rec["scan_ms"] = round((time.time() - t_scan) * 1000)
                rec["mode"] = reader.last_mode
                rec["n_addrs"] = len(reader._addrs)
                rec["candidates"] = cands  # 依份數排序
            except GameNotRunning as exc:
                rec["error"] = str(exc)
                f.write(json.dumps(rec, ensure_ascii=False) + "\n")
                time.sleep(2)
                continue

            if prev is None:
                if cands:
                    prev = cands[0]
                    rec["event"] = "anchor_init"
            else:
                best = choose_window(prev, cands)
                if best is not None:
                    chosen, lines = best
                    rec["chosen_idx"] = cands.index(chosen)
                    rec["appended"] = lines  # ← 這些就是「會被翻譯顯示」的行
                    if lines:
                        rec["event"] = "EMIT"
                    prev = chosen
                    misaligned = 0
                elif cands:
                    misaligned += 1
                    rec["event"] = f"misaligned_{misaligned}"
                    if misaligned >= REANCHOR_AFTER:
                        prev = cands[0]
                        misaligned = 0
                        rec["event"] = "REANCHOR"
            rec["prev_tail"] = (prev or [])[-3:]
            f.write(json.dumps(rec, ensure_ascii=False) + "\n")
            f.flush()
            if rec.get("event") == "EMIT":
                print(f"[{rec['t']:7.1f}s] EMIT {rec['appended']}")
            elif "event" in rec:
                print(f"[{rec['t']:7.1f}s] {rec['event']}")
            n += 1
            time.sleep(0.4)
    print(f"完成,共 {n} 輪。log:{LOG}")


if __name__ == "__main__":
    main()
