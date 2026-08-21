"""逆向臨門一腳:驗證「指標槽」是否穩定追蹤搬家的視窗緩衝。
1) 跨位址鎖定當前緩衝 A。2) 找指向 A 的指標槽集合。
3) 每當緩衝搬到 A',檢查哪些槽的『位址不變、值更新為 A'』——那就是穩定槽,
   讀它即可永遠拿到最新緩衝,不必全掃。輸出 scripts/probe_slot.log。
跑的時候在遊戲反覆打字(含重複),讓緩衝多搬幾次家。
"""
import io
import struct
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")

from src.reader.mem_reader import (  # noqa: E402
    LiveChatReader, align_append, groups_in_blob, _iter_regions, _read,
)

_GROUP_MIN = 2


def snapshot(h):
    out = {}
    for base, blob in _iter_regions(h):
        for start, end, lines in groups_in_blob(blob):
            if len(lines) >= _GROUP_MIN:
                out[base + start] = tuple(lines)
    return out


def ptrs_to(h, target: int) -> set[int]:
    needle = struct.pack("<Q", target)
    hits = set()
    for base, blob in _iter_regions(h):
        k = blob.find(needle)
        while k >= 0:
            if (base + k) % 8 == 0:
                hits.add(base + k)
            k = blob.find(needle, k + 1)
    return hits


def read_ptr(h, slot: int) -> int:
    b = _read(h, slot, 8)
    return struct.unpack("<Q", b)[0] if len(b) == 8 else 0


def find_current(h, known):
    """回傳延續 known 的最長群 (addr, lines);找不到回傳 (None, known)。"""
    best = None
    for addr, lines in snapshot(h).items():
        if align_append(list(known), list(lines)):
            if best is None or len(lines) > len(best[1]):
                best = (addr, lines)
    return best if best else (None, known)


def main():
    r = LiveChatReader()
    h = r._open()
    cur = snapshot(h)
    addr, known = max(cur.items(), key=lambda kv: len(kv[1]))
    print(f"起始緩衝 @{addr:x} n={len(known)}", flush=True)

    slots = None
    stable = None
    t0 = time.time()
    moves = 0
    while time.time() - t0 < 200 and moves < 6:
        time.sleep(1.0)
        a2, lines2 = find_current(h, known)
        if a2 is None or lines2 == known:
            if slots:  # 緩衝沒前進:檢查已知槽現在指向哪
                vals = {s: read_ptr(h, s) for s in slots}
                pointing = [s for s, v in vals.items() if v == addr]
                print(f"  [{time.time()-t0:.0f}s] 無前進;{len(pointing)}/{len(slots)} "
                      f"槽仍指向當前緩衝", flush=True)
            continue
        moves += 1
        ap = align_append(list(known), list(lines2))
        print(f"\n[{time.time()-t0:.0f}s] 緩衝前進 @{addr:x} -> @{a2:x} "
              f"{'同位址' if a2 == addr else '搬家'} 新增={ap}", flush=True)
        new_slots = ptrs_to(h, a2)
        if slots is not None:
            survived = slots & new_slots  # 位址不變、且現在指向新緩衝 = 穩定槽
            print(f"    上輪槽 {len(slots)} 個;本輪指向新緩衝 {len(new_slots)} 個;"
                  f"位址不變且已更新的穩定槽 = {len(survived)} 個", flush=True)
            for s in list(survived)[:8]:
                print(f"        穩定槽 @{s:x}", flush=True)
            stable = survived if stable is None else (stable & new_slots)
        slots, addr, known = new_slots, a2, lines2

    print(f"\n=== 結論:{moves} 次搬家後,持續穩定的槽 = "
          f"{len(stable) if stable else 0} 個 ===", flush=True)
    if stable:
        for s in list(stable)[:8]:
            print(f"    @{s:x} -> 讀值 {read_ptr(h, s):x}", flush=True)
    print("done", flush=True)
    r._close(h)


if __name__ == "__main__":
    main()
