"""偵察:可視窗滾動對齊。追蹤『小的、乾淨滾動的可視窗文件』,
每輪找 align_append 延續上次視窗的候選,顯示新增行 —— 驗證重複也逐則偵測、
舊副本/乒乓不誤報。輸出 scripts/probe_window.log。跑時在遊戲打字(含重複 Test)。
"""
import io
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")

from src.reader.mem_reader import (  # noqa: E402
    LiveChatReader, align_append, groups_in_blob, _iter_regions,
)

_MIN, _MAX = 3, 120  # 可視窗行數範圍(排除 1000+ 歷史大文件與碎片)


def windows(h):
    out = []
    for base, blob in _iter_regions(h):
        for start, end, lines in groups_in_blob(blob):
            if _MIN <= len(lines) <= _MAX:
                out.append((base + start, lines))
    return out


def main():
    r = LiveChatReader()
    h = r._open()
    win = None
    t0 = time.time()
    n = 0
    while time.time() - t0 < 150:
        cands = windows(h)
        if win is None:
            # 起始:取「份數最多的內容」當可視窗(遊戲當前顯示會被渲染成多份)
            from collections import Counter
            c = Counter(tuple(l) for _, l in cands)
            win = list(c.most_common(1)[0][0]) if c else []
            print(f"起始視窗 n={len(win)} 尾={win[-2:] if win else []}", flush=True)
        else:
            # 找 align 延續當前視窗的候選,取新增>0 的;優先最短新增(純滾動一步)
            best = None
            for addr, lines in cands:
                ap = align_append(win, list(lines))
                if ap:
                    if best is None or len(ap) < len(best[1]):
                        best = (addr, ap, list(lines))
            if best:
                print(f"[{time.time()-t0:6.1f}s] +{best[1]}  (視窗 n={len(best[2])})", flush=True)
                win = best[2]
        n += 1
        time.sleep(0.4)
    print(f"done {n} polls", flush=True)
    r._close(h)


if __name__ == "__main__":
    main()
