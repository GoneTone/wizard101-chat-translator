"""逐位址追蹤:全掃一次找出所有聊天小群位址,之後每 0.4s 重讀各位址,
記錄每個位址的內容如何隨時間變化 —— 找出「活的視窗」(以附加/捲動方式更新)。
用法:python scripts/probe_addrs.py [秒數,預設 60];log 在 scripts/probe_addrs.jsonl。
"""
import io
import json
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")

from src.reader.mem_reader import (  # noqa: E402
    _CHUNK, _read, VisibleReader, windows_in_blob,
)

LOG = Path(__file__).with_suffix(".jsonl")


def main() -> None:
    duration = float(sys.argv[1]) if len(sys.argv) > 1 else 60.0
    r = VisibleReader()
    h = r._open()
    t_full = time.time()
    _cands, addrs = r._full(h)
    print(f"全掃 {time.time() - t_full:.1f}s,追蹤 {len(addrs)} 個位址")

    t0 = time.time()
    poll = 0
    with LOG.open("w", encoding="utf-8") as f:
        while time.time() - t0 < duration:
            for addr in addrs:
                wins = windows_in_blob(_read(h, addr, _CHUNK))
                win = list(wins[0][1]) if wins else []
                f.write(json.dumps({"t": round(time.time() - t0, 2), "poll": poll,
                                    "addr": addr, "n": len(win), "win": win},
                                   ensure_ascii=False) + "\n")
            f.flush()
            poll += 1
            time.sleep(0.4)
    print(f"完成,共 {poll} 輪。log:{LOG}")


if __name__ == "__main__":
    main()
