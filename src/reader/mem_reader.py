"""記憶體收訊端:掃描 Wizard101 程序記憶體,抽出當前聊天行(純讀,不寫不 hook)。

聊天行在記憶體裡是帶標記的 UTF-16 富文字:
    <color;FFFFFF><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> [發送者] 內文 </color>
以這段簽章為錨點掃描,去標記後回傳乾淨的「[發送者] 內文」。

只開 PROCESS_VM_READ,不做任何寫入或程式碼注入。
"""
import ctypes
import ctypes.wintypes as wt
import re

PROCESS_NAME = "WizardGraphicalClient.exe"

# --- 純函式:標記解析(可單元測試,不需遊戲)---
MARKER = "<color;FFFFFF><image;Art/Art_Chat".encode("utf-16-le")
CLOSE = "</color>".encode("utf-16-le")
_TAG = re.compile(r"<[^>]*>")
_VALID = re.compile(r"^\[[^\]]{1,40}\] .+")
_MAX_LINE_BYTES = 1400


# 遊戲表情符號:訊息內文以 <image;Emoticons/名稱.dds;24;24;..> 內嵌,轉成對應 emoji 顯示。
_EMOTE_TAG = re.compile(r"<image;Emoticons/([^.;>]+)\.dds[^>]*>", re.IGNORECASE)
_EMOJI_BY_NAME = {
    "smile": "🙂", "big_smile": "😃", "laugh": "😆", "laughter": "😂",
    "wink": "😉", "frown": "🙁", "sad": "😢", "cry": "😢", "streamcrying": "😭",
    "shocked": "😲", "confused": "😕", "angry": "😠", "mad": "😠", "cool": "😎",
    "tongue_stick_out": "😛", "grin": "😁", "sleep": "😴", "kiss": "😘",
    "heart": "❤️", "brokenheart": "💔", "broken_heart": "💔",
    "eyes": "👀", "battle_eye": "👁️", "battle_sun": "☀️", "battle_moon": "🌙",
    "battle_star": "⭐", "school_death": "💀", "school_fire": "🔥",
    "school_ice": "❄️", "school_storm": "⚡", "school_life": "🌿",
    "school_myth": "🐍", "school_balance": "⚖️", "teacup": "🍵",
    "transgender": "⚧️", "thumbsup": "👍", "thumbs_up": "👍", "star": "⭐",
    "moon": "🌙", "sun": "☀️", "music": "🎵", "ghost": "👻", "crown": "👑",
    "skull": "💀", "flower": "🌸", "rainbow": "🌈", "pizza": "🍕",
    "cake": "🎂", "clap": "👏", "wave": "👋", "fire": "🔥",
}


def _emote_to_char(m: re.Match) -> str:
    name = re.sub(r"^emoticons?_|\d+$", "", m.group(1).lower())
    if not re.fullmatch(r"[a-z0-9_]+", name):
        return ""  # 撕裂的標記(名稱夾入雜字):丟棄該表情,保留整行其餘內容
    return _EMOJI_BY_NAME.get(name, f":{name}:")  # 沒對應的以 :名稱: 顯示


def clean(text: str) -> str:
    """表情標記轉 emoji,去掉 <color;..> <image;..> </color> 等標記,
    還原玩家實際打出的 &lt; &gt; &amp; 實體,壓縮空白。"""
    text = _EMOTE_TAG.sub(_emote_to_char, text)
    text = _TAG.sub("", text)
    text = text.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
    return " ".join(text.replace("\x00", " ").split())


def extract_lines(blob: bytes, dedup: bool = True) -> list[str]:
    """在一段記憶體 blob 中,依聊天簽章抽出乾淨聊天行,保留出現順序。
    dedup=True:blob 內去重(掃全記憶體時用,避免大量重複副本)。
    dedup=False:保留重複(讀可視視窗時用,同一句連續出現要照實保留)。"""
    out: list[str] = []
    seen: set[str] = set()
    s = 0
    while True:
        j = blob.find(MARKER, s)
        if j < 0:
            break
        s = j + 2
        end = blob.find(CLOSE, j, j + _MAX_LINE_BYTES)
        if end < 0:
            continue
        txt = clean(blob[j:end].decode("utf-16-le", "replace"))
        if _VALID.match(txt) and not (dedup and txt in seen):
            seen.add(txt)
            out.append(txt)
    return out


# --- Win32 純讀掃描 ---
class GameNotRunning(Exception):
    """找不到遊戲程序,或無法開啟供讀取。"""


PROCESS_QUERY_INFORMATION = 0x0400
PROCESS_VM_READ = 0x0010
MEM_COMMIT = 0x1000
PAGE_GUARD = 0x100
PAGE_NOACCESS = 0x01

_k32 = ctypes.WinDLL("kernel32", use_last_error=True)
_psapi = ctypes.WinDLL("psapi", use_last_error=True)
_c_void_p, _c_size_t, _PTR = ctypes.c_void_p, ctypes.c_size_t, ctypes.POINTER
_k32.OpenProcess.argtypes = [wt.DWORD, wt.BOOL, wt.DWORD]
_k32.OpenProcess.restype = _c_void_p
_k32.CloseHandle.argtypes = [_c_void_p]
_k32.ReadProcessMemory.argtypes = [_c_void_p, _c_void_p, _c_void_p, _c_size_t, _PTR(_c_size_t)]
_k32.ReadProcessMemory.restype = wt.BOOL
_k32.VirtualQueryEx.argtypes = [_c_void_p, _c_void_p, _c_void_p, _c_size_t]
_k32.VirtualQueryEx.restype = _c_size_t
_psapi.EnumProcesses.argtypes = [_PTR(wt.DWORD), wt.DWORD, _PTR(wt.DWORD)]
_psapi.GetModuleBaseNameW.argtypes = [_c_void_p, _c_void_p, ctypes.c_wchar_p, wt.DWORD]
_psapi.GetModuleBaseNameW.restype = wt.DWORD

_MAX_ADDR = 0x7FFFFFFFFFFF
_MAX_REGION = 512 * 1024 * 1024


class _MBI(ctypes.Structure):
    _fields_ = [
        ("BaseAddress", ctypes.c_ulonglong), ("AllocationBase", ctypes.c_ulonglong),
        ("AllocationProtect", wt.DWORD), ("__a1", wt.DWORD),
        ("RegionSize", ctypes.c_ulonglong),
        ("State", wt.DWORD), ("Protect", wt.DWORD), ("Type", wt.DWORD), ("__a2", wt.DWORD),
    ]


def _find_pid(name: str) -> int:
    arr = (wt.DWORD * 8192)()
    got = wt.DWORD()
    _psapi.EnumProcesses(arr, ctypes.sizeof(arr), ctypes.byref(got))
    for i in range(got.value // ctypes.sizeof(wt.DWORD)):
        pid = arr[i]
        if not pid:
            continue
        h = _k32.OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, False, pid)
        if not h:
            continue
        buf = ctypes.create_unicode_buffer(260)
        _psapi.GetModuleBaseNameW(h, None, buf, 260)
        _k32.CloseHandle(h)
        if buf.value.lower() == name.lower():
            return pid
    return 0


def _iter_regions(h):
    """逐一 yield (base, bytes) —— 每個可讀已提交記憶體區域。"""
    addr = 0
    mbi = _MBI()
    while addr < _MAX_ADDR:
        if not _k32.VirtualQueryEx(h, _c_void_p(addr), _c_void_p(ctypes.addressof(mbi)),
                                   ctypes.sizeof(mbi)):
            break
        base, size = mbi.BaseAddress, mbi.RegionSize
        nxt = base + size
        readable = (mbi.State == MEM_COMMIT and not (mbi.Protect & PAGE_GUARD)
                    and mbi.Protect not in (0, PAGE_NOACCESS))
        if readable and 0 < size < _MAX_REGION:
            buf = (ctypes.c_char * size)()
            got = _c_size_t(0)
            if _k32.ReadProcessMemory(h, _c_void_p(base), ctypes.cast(buf, _c_void_p),
                                      size, ctypes.byref(got)) and got.value:
                yield base, bytes(buf[:got.value])
        if nxt <= addr:
            break
        addr = nxt


def _read_region_at(h, addr: int) -> bytes:
    """讀取「包含 addr 的那個已提交區域」的內容;讀不到回傳空。"""
    mbi = _MBI()
    if not _k32.VirtualQueryEx(h, _c_void_p(addr), _c_void_p(ctypes.addressof(mbi)),
                               ctypes.sizeof(mbi)):
        return b""
    base, size = mbi.BaseAddress, mbi.RegionSize
    readable = (mbi.State == MEM_COMMIT and not (mbi.Protect & PAGE_GUARD)
                and mbi.Protect not in (0, PAGE_NOACCESS))
    if not (readable and 0 < size < _MAX_REGION):
        return b""
    buf = (ctypes.c_char * size)()
    got = _c_size_t(0)
    if _k32.ReadProcessMemory(h, _c_void_p(base), ctypes.cast(buf, _c_void_p),
                              size, ctypes.byref(got)) and got.value:
        return bytes(buf[:got.value])
    return b""


def _read(h, addr: int, n: int) -> bytes:
    buf = (ctypes.c_char * n)()
    got = _c_size_t(0)
    if _k32.ReadProcessMemory(h, _c_void_p(addr), ctypes.cast(buf, _c_void_p),
                              n, ctypes.byref(got)) and got.value:
        return bytes(buf[:got.value])
    return b""


def _read_clamped(h, addr: int, need: int) -> bytes:
    """從 addr 讀最多 need bytes,但不跨出所在區域(跨區域讀取會整批失敗)。"""
    mbi = _MBI()
    if not _k32.VirtualQueryEx(h, _c_void_p(addr), _c_void_p(ctypes.addressof(mbi)),
                               ctypes.sizeof(mbi)):
        return b""
    readable = (mbi.State == MEM_COMMIT and not (mbi.Protect & PAGE_GUARD)
                and mbi.Protect not in (0, PAGE_NOACCESS))
    if not readable:
        return b""
    end = mbi.BaseAddress + mbi.RegionSize
    return _read(h, addr, min(need, end - addr))


# --- 活聊天文件(遊戲聊天視窗的資料來源) ---
# 逆向結論(2026-08-21 生命週期探針):遊戲把聊天視窗內容維護成「一份就地附加的
# 富文字文件」(位址穩定、每有新訊息尾端就地成長);此外每次重繪還會產生大量
# 一次性渲染快照(新位址誕生、舊位址殘留成垃圾)—— 快照不可信,只有活文件可信。
_GROUP_GAP = 4000            # 同一份文件內相鄰標記的最大位址間隔(bytes)
_MIN_GROUP_MARKERS = 1       # 單行群也納入 —— 空聊天室只有一句時文件才看得到
_NEW_DOC_BASELINE = 2        # 定錨時基準 ≤ 這個行數視為全新文件,連基準行一起補翻
_DOC_MARGIN = 256 * 1024     # 定錨輪詢時,文件長度之外多讀的餘量(容納成長與頭部修剪)
_MAX_DOC_READ = 8 * 1024 * 1024
_UNANCHOR_FAILS = 2          # 副本連續對不齊幾輪視為已搬移/釋放 → 淘汰該副本
_MAX_ANCHORS = 8             # 同時追蹤的文件副本位址上限(每輪各輪詢一次,~10ms/個)
_IDLE_RECHECK_POLLS = 75     # 錨點太久無變化(可能全是殘骸)→ 驗證掃描(0.4s 輪 ≈ 30s)
_EMITTED_KEEP = 50           # 記住最近已輸出的行數(換錨時做重疊裁剪防重播)


def groups_in_blob(blob: bytes) -> list[tuple[int, int, list[str]]]:
    """把 blob 內的聊天標記依間隔分群,回傳每群的 (起始 offset, 結尾 offset, 有序聊天行)。
    保留重複,不設群大小上限(活文件可達數百行)。"""
    marks = []
    k = blob.find(MARKER)
    while k >= 0:
        marks.append(k)
        k = blob.find(MARKER, k + 1)
    out: list[tuple[int, int, list[str]]] = []
    group: list[int] = []

    def flush():
        if len(group) >= _MIN_GROUP_MARKERS:
            end = group[-1] + _MAX_LINE_BYTES
            out.append((group[0], end, extract_lines(blob[group[0]:end], dedup=False)))

    for m in marks:
        if group and m - group[-1] > _GROUP_GAP:
            flush()
            group = []
        group.append(m)
    if group:
        flush()
    return out


def align_append(prev_lines: list[str], cur_lines: list[str]) -> list[str] | None:
    """附加/捲動對齊:找最短的『prev 去掉前 k 行(非空)』正好是 cur 的前綴,
    回傳 cur 尾端多出來的行(新訊息,含重複、依序);對不齊回傳 None。
    活文件平時純附加(k=0 必中,無歧義);達容量上限修剪頭部時 k>0 吸收捲動。"""
    if not prev_lines:
        return None
    for k in range(len(prev_lines)):
        overlap = prev_lines[k:]
        if cur_lines[:len(overlap)] == overlap:
            return cur_lines[len(overlap):]
    return None


def after_last_tail(doc: list[str], tail: list[str]) -> list[str] | None:
    """在 doc 中找 tail(或其較短字尾)最後一次出現的位置,回傳其後的行;找不到回傳 None。
    重定錨時用:已知尾行之後的內容 = 錨定空窗期漏掉的訊息,補翻不漏不重。"""
    for probe in (tail, tail[-3:], tail[-1:]):
        if not probe:
            continue
        n = len(probe)
        for i in range(len(doc) - n, -1, -1):
            if doc[i:i + n] == probe:
                return doc[i + n:]
    return None


class LiveChatReader:
    """定錨『活聊天文件』並回傳每輪新增的聊天行。

    未定錨:每次呼叫做一次全掃並與上一次全掃比對 —— 同位址、內容以附加方式成長的
    群 = 活文件;取最長者定錨,以已知尾行對齊補翻空窗期訊息。
    已定錨:同時追蹤文件的「所有同步副本」位址,每輪逐一輕量輪詢(各 ~10ms)取最新
    對齊結果 —— 單一副本被搬移/釋放成殭屍時,其他活副本無縫接手,不會卡在殘骸上。
    全部副本都對不齊 → 解除定錨重新探索;錨點長期無變化 → 驗證掃描:找到延伸我們
    內容的文件就當場重定錨並補翻(單次掃描完成,不必等下一次成長)。"""

    def __init__(self, process_name: str = PROCESS_NAME):
        self.process_name = process_name
        self._addrs: dict[int, int] = {}  # 錨點副本位址 → 連續對不齊次數;空 = 未定錨
        self._bytes = 0               # 文件位元組長度估計
        self._lines: list[str] = []   # 文件目前內容;未定錨時為最後已知尾行
        self._snapshot: dict[int, tuple[int, tuple[str, ...]]] | None = None
        self._seen_texts: set[str] | None = None  # 探索期已見過的行文字(新穎性比對)
        self._emitted: list[str] = []  # 最近已輸出的行(換錨時做重疊裁剪防重播)
        self._idle = 0

    # --- 可在測試中覆寫的接縫 ---
    def _open(self):
        pid = _find_pid(self.process_name)
        if not pid:
            raise GameNotRunning(f"找不到 {self.process_name}")
        h = _k32.OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, False, pid)
        if not h:
            raise GameNotRunning(f"無法開啟 {self.process_name}（可能需要系統管理員權限）")
        return h

    def _close(self, handle):
        _k32.CloseHandle(handle)

    def _scan_groups(self, h):
        """全掃:yield (絕對位址, 位元組長, 行tuple)。"""
        for base, blob in _iter_regions(h):
            for start, end, lines in groups_in_blob(blob):
                yield base + start, end - start, tuple(lines)

    def _poll_groups(self, h, addr: int):
        """讀某個錨點副本附近:回傳 [(絕對位址, 位元組長, 行list), ...]。"""
        need = min(self._bytes + _DOC_MARGIN, _MAX_DOC_READ)
        blob = _read_clamped(h, addr, need)
        return [(addr + start, end - start, lines)
                for start, end, lines in groups_in_blob(blob)]

    # --- 主流程 ---
    @property
    def anchored(self) -> bool:
        """是否已定錨活聊天文件(未定錨時每輪為全掃探索,較慢)。"""
        return bool(self._addrs)

    def read_new(self) -> list[str]:
        """回傳自上次呼叫後新增的聊天行(依序、含重複);無新訊息回傳 []。
        找不到遊戲丟 GameNotRunning。"""
        h = self._open()
        try:
            new_lines = self._poll_doc(h) if self._addrs else self._discover(h)
        finally:
            self._close(h)
        if new_lines:
            self._emitted = (self._emitted + new_lines)[-_EMITTED_KEEP:]
        return new_lines

    def _trim_emitted_overlap(self, lines: list[str]) -> list[str]:
        """換錨(探索/重定錨)時的保險:輸出開頭若與『最近已輸出的行』尾端重疊,裁掉重疊段。
        活文件與同樣就地更新的渲染快取行集合略有差異,換錨對不上尾行時 fallback 可能
        重播剛輸出過的行 —— 用已輸出串流裁剪掉。已定錨的正常輪詢是精確自我差分,
        不套用(以免吃掉真實的連續重複訊息)。"""
        for k in range(min(len(lines), len(self._emitted)), 0, -1):
            if self._emitted[len(self._emitted) - k:] == lines[:k]:
                return lines[k:]
        return lines

    def _poll_doc(self, h) -> list[str]:
        best = None
        for addr in list(self._addrs):
            aligned = None
            for a2, nbytes, lines in self._poll_groups(h, addr):
                appended = align_append(self._lines, lines)
                if appended is not None and (aligned is None or len(lines) > len(aligned[2])):
                    aligned = (a2, nbytes, lines, appended)
            if aligned is None:
                # 此副本已被搬移/覆寫成殭屍:計次,連續兩輪就淘汰(其他副本照常供訊息)
                self._addrs[addr] = self._addrs[addr] + 1
                if self._addrs[addr] >= _UNANCHOR_FAILS:
                    del self._addrs[addr]
                continue
            if aligned[0] != addr:  # 群起點因頭部修剪前移:更新位址鍵
                del self._addrs[addr]
            self._addrs[aligned[0]] = 0
            if best is None or len(aligned[2]) > len(best[2]):
                best = aligned
        if best is None:
            if not self._addrs:
                self._unanchor()
            return []
        _, self._bytes, self._lines, appended = best
        if appended:
            self._idle = 0
        else:
            self._idle += 1
            if self._idle >= _IDLE_RECHECK_POLLS:
                self._idle = 0
                return self._verify_anchor(h)  # 錨點太久沒動靜:驗證是否已成殘骸
        return appended

    def _verify_anchor(self, h) -> list[str]:
        """驗證掃描(單次全掃):錨點長期無變化時,確認世界上是否有「延伸我們內容」的
        文件 —— 有就當場重定錨並補翻(錨點是殘骸、真文件已搬走的情況);內容仍是最新
        就重新蒐集同步副本位址;完全對不上才解錨重探索。"""
        groups = [(addr, nbytes, list(lines))
                  for addr, nbytes, lines in self._scan_groups(h) if lines]
        best = None
        current: list[tuple[int, int]] = []
        for addr, nbytes, lines in groups:
            appended = align_append(self._lines, lines)
            if appended is None:
                continue
            if appended:
                if best is None or len(lines) > len(best[2]):
                    best = (addr, nbytes, lines, appended)
            else:
                current.append((addr, nbytes))
        if best is not None:  # 有文件比我們新 → 錨點是殘骸,搬過去並補翻
            self._anchor_to(best[0], best[1], best[2],
                            [a for a, _, ls in groups if ls == best[2]])
            return self._trim_emitted_overlap(best[3])
        if current:  # 內容仍是最新:刷新副本位址(重新抓齊雙胞胎)
            self._addrs = {a: 0 for a, _ in current[:_MAX_ANCHORS]}
            self._bytes = max(n for _, n in current)
            return []
        self._unanchor()
        return []

    def _anchor_to(self, addr: int, nbytes: int, lines: list[str],
                   twin_addrs: list[int]) -> None:
        self._addrs = {addr: 0}
        for a in twin_addrs[:_MAX_ANCHORS]:
            self._addrs[a] = 0
        self._bytes = nbytes
        self._lines = lines
        self._snapshot = None
        self._seen_texts = None
        self._idle = 0

    def _unanchor(self) -> None:
        self._lines = self._lines[-8:]  # 保留尾行,重定錨時對齊用
        self._addrs = {}
        self._bytes = 0
        self._idle = 0
        self._snapshot = None
        self._seen_texts = None

    def _discover(self, h) -> list[str]:
        snap: dict[int, tuple[int, tuple[str, ...]]] = {}
        for addr, nbytes, lines in self._scan_groups(h):
            if lines:
                snap[addr] = (nbytes, lines)
        prev, self._snapshot = self._snapshot, snap
        if prev is None:
            # 第一掃:所有既有內容視為歷史(不翻),當作新穎性比對基準
            self._seen_texts = {ln for _, lines in snap.values() for ln in lines}
            self._seen_texts.update(self._emitted)
            return []
        best = None
        for addr, (nbytes, lines) in snap.items():
            old = prev.get(addr)
            if old is None or old[1] == lines:
                continue
            appended = align_append(list(old[1]), list(lines))
            if appended and (best is None or len(lines) > len(best[2])):
                best = (addr, nbytes, list(lines), appended, list(old[1]))
        if best is not None:
            known_tail = self._lines
            addr, nbytes, lines, appended, baseline = best
            self._anchor_to(addr, nbytes, lines,
                            [a for a, (_, ls) in snap.items() if list(ls) == lines])
            if known_tail:
                after = after_last_tail(self._lines, known_tail)
                if after is not None:
                    return self._trim_emitted_overlap(after)
            if len(baseline) <= _NEW_DOC_BASELINE:
                # 全新文件(空聊天室的頭幾句):基準行也是剛出現的訊息,一起補翻。
                # 搬移的舊文件基準必然很長,不會走到這裡。
                return self._trim_emitted_overlap(baseline + appended)
            return self._trim_emitted_overlap(appended)
        return self._novel_lines(snap)

    def _novel_lines(self, snap) -> list[str]:
        """探索期的新穎性路徑:還沒有可定錨的成長,但這次全掃出現了上次沒有的新文字
        = 剛到達的訊息(例:空聊天室的第一句),立刻翻、不必等文件成長。
        防垃圾:新文字必須出現在至少 2 個緩衝(真訊息立刻被渲染成多份副本;
        撕裂的破損副本內容各不相同,永遠只有 1 份)。"""
        counts: dict[str, int] = {}
        for _, lines in snap.values():
            for ln in set(lines):
                counts[ln] = counts.get(ln, 0) + 1
        novel = {ln for ln, c in counts.items()
                 if c >= 2 and ln not in self._seen_texts}
        self._seen_texts.update(ln for ln, c in counts.items() if c >= 2)
        if not novel:
            return []
        # 從「含最多新行的最長群」依序取出,保持時間順序
        best_lines = max((list(lines) for _, lines in snap.values()),
                         key=lambda ls: (sum(1 for ln in set(ls) if ln in novel), len(ls)))
        return self._trim_emitted_overlap([ln for ln in best_lines if ln in novel])
