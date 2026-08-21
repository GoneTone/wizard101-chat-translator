"""緩衝生命週期探針:每輪完整全掃,記錄所有聊天群(含 >120 標記的大群)的
位址、行數、內容雜湊、尾 2 行 —— 觀察新訊息到達時:新緩衝出現在哪、舊緩衝何時消失、
是否有任何緩衝(小群或大群)就地更新。
用法:python scripts/probe_lifecycle.py [秒數,預設 90];log 在 scripts/probe_lifecycle.jsonl。
"""
import hashlib
import io
import json
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")

import src.reader.mem_reader as mr  # noqa: E402
from src.reader.mem_reader import (  # noqa: E402
    MARKER, VisibleReader, _iter_regions, _MAX_LINE_BYTES, extract_lines,
)

LOG = Path(__file__).with_suffix(".jsonl")
GAP = mr._GROUP_GAP


def groups_all(blob: bytes):
    """所有群(不設上限):回傳 (offset, 行list)。"""
    marks = []
    k = blob.find(MARKER)
    while k >= 0:
        marks.append(k)
        k = blob.find(MARKER, k + 1)
    out = []
    group: list[int] = []

    def flush():
        if len(group) >= 2:
            seg = blob[group[0]:group[-1] + _MAX_LINE_BYTES]
            out.append((group[0], extract_lines(seg, dedup=False)))

    for m in marks:
        if group and m - group[-1] > GAP:
            flush()
            group = []
        group.append(m)
    if group:
        flush()
    return out


def main() -> None:
    duration = float(sys.argv[1]) if len(sys.argv) > 1 else 90.0
    r = VisibleReader()
    h = r._open()
    t0 = time.time()
    poll = 0
    with LOG.open("w", encoding="utf-8") as f:
        while time.time() - t0 < duration:
            t_scan = time.time()
            snap = []
            for base, blob in _iter_regions(h):
                for off, lines in groups_all(blob):
                    if not lines:
                        continue
                    digest = hashlib.md5(
                        "\x00".join(lines).encode("utf-8")).hexdigest()[:10]
                    snap.append({"addr": base + off, "n": len(lines),
                                 "h": digest, "tail": lines[-2:]})
            f.write(json.dumps({"t": round(time.time() - t0, 2), "poll": poll,
                                "scan_ms": round((time.time() - t_scan) * 1000),
                                "groups": snap}, ensure_ascii=False) + "\n")
            f.flush()
            print(f"[{time.time() - t0:6.1f}s] poll {poll}: {len(snap)} groups, "
                  f"scan {round((time.time() - t_scan) * 1000)}ms")
            poll += 1
    print(f"完成。log:{LOG}")


if __name__ == "__main__":
    main()
