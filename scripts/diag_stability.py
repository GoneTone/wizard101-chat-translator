import time, sys, io
sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
sys.path.insert(0, ".")
from src.reader import mem_reader as mr
from src.reader.mem_reader import LiveChatReader


class Diag(LiveChatReader):
    def _relocate(self, h):
        t = time.time(); r = super()._relocate(h)
        state = ('換錨補' + str(len(r)) + '則' if r else
                 ('保持殭屍' if self._addr else '解錨回探索'))
        print(f"    ★ 重定位({round((time.time()-t)*1000)}ms) → {state}", flush=True)
        return r

    def _discover(self, h):
        r = super()._discover(h); self._m = "全掃探索"; return r

    def _poll_doc(self, h):
        r = super()._poll_doc(h); self._m = "錨點輪詢"; return r


r = Diag(); t0 = time.time(); prev = None; last_new = None; maxgap = 0; anchor_t = None
for i in range(180):
    t = time.time()
    try:
        new = r.read_new()
    except Exception as e:
        print("err", e, flush=True); time.sleep(0.4); continue
    ms = round((time.time() - t) * 1000)
    if r.anchored != prev:
        if r.anchored and anchor_t is None:
            anchor_t = round(time.time() - t0, 1)
        print(f"[{time.time()-t0:5.1f}s] 定錨:{prev}→{r.anchored}", flush=True); prev = r.anchored
    if new:
        if last_new:
            maxgap = max(maxgap, time.time() - last_new)
        last_new = time.time()
        print(f"[{time.time()-t0:5.1f}s]({ms:4}ms) {r._m} NEW×{len(new)}: {new[-1][:26]!r}", flush=True)
    time.sleep(0.4)
print(f"done. 首次定錨≈{anchor_t}s, 最長訊息間隔≈{maxgap:.1f}s", flush=True)
