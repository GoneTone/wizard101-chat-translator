"""逆向探針:找出遊戲聊天視窗的資料結構。
1) 兩次全掃找「純附加成長」的群 = 遊戲聊天視窗的行緩衝(權威來源)。
2) 全記憶體掃指向該緩衝起始位址(及附近物件頭)的 8-byte 指標。
3) 對找到的指標,再往上找指向它的指標(指標鏈),看是否落在穩定區域。
輸出寫 scripts/probe_pointers.log。跑的時候在遊戲打幾則字(含重複的)讓緩衝成長。
"""
import io
import struct
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")

from src.reader.mem_reader import (  # noqa: E402
    LiveChatReader, align_append, groups_in_blob, _iter_regions, _read_clamped,
)


def find_pointers_to(h, target: int) -> list[int]:
    """全記憶體掃 8-byte little-endian == target 的位址(8 對齊)。"""
    needle = struct.pack("<Q", target)
    hits = []
    for base, blob in _iter_regions(h):
        k = blob.find(needle)
        while k >= 0:
            if (base + k) % 8 == 0:
                hits.append(base + k)
            k = blob.find(needle, k + 1)
    return hits


def region_of(h, addr: int) -> str:
    for base, blob in _iter_regions(h):
        if base <= addr < base + len(blob):
            return f"base={base:x} size={len(blob)}"
    return "?"


def main() -> None:
    r = LiveChatReader()
    h = r._open()
    print("等待聊天視窗緩衝成長(請在遊戲打字)…", flush=True)

    # 1) 找純附加成長的群
    def snapshot():
        out = {}
        for base, blob in _iter_regions(h):
            for start, end, lines in groups_in_blob(blob):
                if len(lines) >= 2:
                    out[base + start] = tuple(lines)
        return out

    # 跨位址追蹤:維護「已知視窗行序列」,每輪找延續它的最長群(不管換不換位址)。
    def longest_group(cur):
        return max(cur.items(), key=lambda kv: len(kv[1]), default=(None, ()))

    cur = snapshot()
    known_addr, known = longest_group(cur)
    print(f"起始視窗候選 @{known_addr:x} n={len(known)} 尾={known[-1][:40]!r}"
          if known else "無群", flush=True)
    target = None
    t0 = time.time()
    n = 0
    while time.time() - t0 < 180 and target is None:
        time.sleep(1.0)
        cur = snapshot()
        n += 1
        # 找「延續已知視窗(跨位址附加)」的群
        grown = None
        for addr, lines in cur.items():
            ap = align_append(list(known), list(lines))
            if ap and (grown is None or len(lines) > len(cur[grown])):
                grown = addr
        if grown is not None:
            ap = align_append(list(known), list(cur[grown]))
            print(f"  掃描 {n} ({time.time()-t0:.0f}s): 視窗前進 @{grown:x} "
                  f"{'(同位址)' if grown == known_addr else '(搬家!)'} 新增={ap}", flush=True)
            known_addr, known = grown, cur[grown]
            target = grown  # 鎖定「當前視窗緩衝」= 剛前進到的位址
        else:
            print(f"  掃描 {n} ({time.time()-t0:.0f}s) groups={len(cur)} 無前進", flush=True)
            ka, kl = longest_group(cur)  # 已知視窗可能已消失,改追當前最長群
            if kl and align_append(list(known), list(kl)) is None:
                known_addr, known = ka, kl
    if target is None:
        print("超時:沒觀察到附加式成長。", flush=True)
        return

    # 2) 掃指向緩衝(及物件頭 target-8..target-256)的指標
    print("\n掃描指向緩衝的指標…", flush=True)
    for off in (0, -8, -16, -24, -32, -48, -64, -128):
        cand = target + off
        hits = find_pointers_to(h, cand)
        if hits:
            print(f"  指向 target{off:+d} (={cand:x}) 的指標: {len(hits)} 個", flush=True)
            for p in hits[:8]:
                print(f"      @{p:x}  ({region_of(h, p)})", flush=True)
            # 3) 指標鏈:找指向這些指標的指標(往上一層)
            for p in hits[:3]:
                up = find_pointers_to(h, p)
                if up:
                    print(f"      ↑ 指向 @{p:x} 的上層指標: "
                          f"{[hex(u) for u in up[:5]]}", flush=True)
    print("\ndone", flush=True)
    r._close(h)


if __name__ == "__main__":
    main()
