"""記憶體收訊端:掃描 Wizard101 程序記憶體,抽出當前聊天行(純讀,不寫不 hook)。

聊天行在記憶體裡是帶標記的 UTF-16 富文字:
    <color;FFFFFF><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> [發送者] 內文 </color>
以這段簽章為錨點掃描,去標記後回傳乾淨的「[發送者] 內文」。

只開 PROCESS_VM_READ,不做任何寫入或程式碼注入。
"""
import ctypes
import ctypes.wintypes as wt
import re
from collections import Counter

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
_MAX_INSERT_CATCHUP = 8      # 插入式定錨/驗證單次補翻上限:超過視為快取跳躍(舊訊息),只重新同步
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


def inserted_lines(prev_lines: list[str], cur_lines: list[str]) -> list[str] | None:
    """插入式文件差分(多重集合):回傳 cur 比 prev『多出現』的行,照 cur 位置順序、
    每種行最多一次。對應依頻道分節的聊天總文件 —— 新訊息插中段、尾端不動。
    用出現次數而非逐位置比對:分節文件重繪常把整段搬動/重排,位置式差分會把
    搬家的舊行誤判成新增(冒舊訊息);行數量不變的移動在這裡天然不算新訊息。
    批內去重:同一則訊息可能同時插入多個節,同批相同行只取一次;真實重複訊息
    分屬不同輪差分,仍會分次輸出。相似度不足回傳 None;無新增回傳 []。"""
    if not prev_lines or not cur_lines:
        return None
    prev_counts = Counter(prev_lines)
    shared = sum((prev_counts & Counter(cur_lines)).values())
    if shared < 0.6 * max(len(prev_lines), len(cur_lines)):
        return None
    out: list[str] = []
    emitted: set[str] = set()
    seen: Counter = Counter()
    for ln in cur_lines:
        seen[ln] += 1
        if seen[ln] > prev_counts[ln] and ln not in emitted:
            emitted.add(ln)
            out.append(ln)
    return out


def doc_diff(prev_lines: list[str], cur_lines: list[str]) -> list[str] | None:
    """活文件差分:先試尾端附加對齊(附加式文件,精確且保留重複);
    失敗再試整份插入比對(插入式文件)。都不像同一份文件回傳 None。"""
    appended = align_append(prev_lines, cur_lines)
    if appended is not None:
        return appended
    return inserted_lines(prev_lines, cur_lines)


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


_MIN_DOC_LINES = 2      # 候選聊天文件至少幾行(排除單行碎片)
_TAIL_FP = 12           # 尾指紋行數:用文件最後 N 行在下一份文件裡定位「已處理到哪」
_MAX_CATCHUP = 40       # 單次最多輸出幾行:超過視為對到殭屍舊內容,只重新同步不輸出
_FULL_EVERY_DOC = 25    # 快取位址最多連用幾輪就強制全掃校正


class LiveChatReader:
    """讀遊戲聊天並回傳每輪新增的聊天行。

    逆向結論(2026-08-21 指標/雙緩衝探針):遊戲把聊天渲染成多份文件(完整歷史 +
    可視視窗 + 一次性快照),每次更新整份複製到新位址(雙緩衝乒乓),舊副本凍結成殭屍;
    沒有可從固定位址到達的穩定指標,且同時存在的多份文件尾端各有對方沒有的行 ——
    追單一文件會在多份之間乒乓、反覆冒出彼此差異。

    因此改用『全域多重集合』:把當前記憶體所有聊天文件的行取聯集(每行取最大出現次數)
    當狀態。新訊息 = 這次聯集比上次『多出來』的行(依最長文件的順序展開,含重複)。
    這對乒乓/搬家/殭屍天生免疫 —— 那些行早在聯集裡,最大次數不會憑空增加;
    只有真的新訊息會讓某行的全域最大次數 +1。基準只增不減,舊訊息永不重播。"""

    def __init__(self, process_name: str = PROCESS_NAME):
        self.process_name = process_name
        self._counts: Counter | None = None  # 全域每行最大出現次數(基準);None = 未初始化
        self._addrs: list[int] = []    # 快取的聊天文件位址(乒乓副本)
        self._bytes = 0                # 文件位元組長度估計(決定快取讀取量)
        self._since_full = _FULL_EVERY_DOC

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

    def _full_docs(self, h):
        """全掃(慢):回傳所有候選聊天文件 [(位址, 位元組長, 行list), ...]。"""
        out = []
        for base, blob in _iter_regions(h):
            for start, end, lines in groups_in_blob(blob):
                if len(lines) >= _MIN_DOC_LINES:
                    out.append((base + start, end - start, lines))
        return out

    def _cached_docs(self, h):
        """只讀快取位址(快):回傳那些位址附近的候選聊天文件。"""
        out = []
        need = min(self._bytes + _DOC_MARGIN, _MAX_DOC_READ)
        for addr in self._addrs:
            blob = _read_clamped(h, addr, need)
            for start, end, lines in groups_in_blob(blob):
                if len(lines) >= _MIN_DOC_LINES:
                    out.append((addr + start, end - start, lines))
        return out

    # --- 主流程 ---
    @property
    def anchored(self) -> bool:
        """是否已建立基準(第一次全掃後即為 True)。"""
        return self._counts is not None

    def read_new(self) -> list[str]:
        """回傳自上次呼叫後新增的聊天行(依序、含重複);無新訊息回傳 []。
        找不到遊戲丟 GameNotRunning。"""
        h = self._open()
        try:
            if self._counts is not None and self._addrs and self._since_full < _FULL_EVERY_DOC:
                self._since_full += 1
                return self._emit(self._cached_docs(h), refresh_addrs=False)
            self._since_full = 0
            return self._emit(self._full_docs(h), refresh_addrs=True)  # 首次/校正:全掃
        finally:
            self._close(h)

    @staticmethod
    def _union(docs) -> Counter:
        """所有文件的行聯集:每行取跨文件的最大出現次數。"""
        u: Counter = Counter()
        for _addr, _nbytes, lines in docs:
            c = Counter(lines)
            for ln, n in c.items():
                if n > u[ln]:
                    u[ln] = n
        return u

    def _emit(self, docs, refresh_addrs: bool) -> list[str]:
        if refresh_addrs and docs:
            self._addrs = self._twin_addrs(docs)
            self._bytes = max(nb for _a, nb, _l in docs)

        u = self._union(docs)
        if self._counts is None:      # 首次:既有內容當基準,不輸出
            self._counts = u
            return []
        delta = u - self._counts      # 多重集合正差 = 這次多出來的行
        self._counts |= u             # 基準取聯集(只增不減,舊訊息永不重播)
        if not delta:
            return []

        # 依最長文件的行順序展開 delta(≈時間序;含重複)
        longest = max(docs, key=lambda d: len(d[2]))[2] if docs else []
        need = dict(delta)
        out: list[str] = []
        for ln in longest:
            if need.get(ln, 0) > 0:
                out.append(ln)
                need[ln] -= 1
        for ln, n in need.items():    # 不在最長文件裡的(罕見)補在後面
            out.extend([ln] * n)
        if len(out) > _MAX_CATCHUP:
            return []  # 一次冒出太多 = 啟動/大跳,基準已更新,不倒一堆舊訊息
        return out

    def _twin_addrs(self, docs) -> list[int]:
        """最長的幾份文件位址(乒乓活副本 + 殭屍),供下輪快取讀取(聯集自然涵蓋)。"""
        return [a for a, _, _ in sorted(docs, key=lambda d: -len(d[2]))[:_MAX_ANCHORS]]
