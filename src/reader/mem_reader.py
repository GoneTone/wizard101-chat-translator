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


_MIN_WIN = 5            # 可視窗最少幾行(排除碎片)
_MAX_WIN = 200          # 可視窗最多幾行(排除 1000+ 的完整歷史大文件)
_MAX_CATCHUP = 40       # 單次最多輸出幾行:超過視為對到過期快照,只重新同步不輸出
_FULL_EVERY_DOC = 25    # 快取位址最多連用幾輪就強制全掃校正


def _senders(lines) -> int:
    """一份文件裡的不同發送者數(用來認出『所有人聊天可視窗』,排除只含自己的小 buffer)。"""
    return len({l.split("]", 1)[0] for l in lines if "]" in l})


class LiveChatReader:
    """讀遊戲『聊天可視窗』並回傳每輪新增的聊天行。

    逆向結論(2026-08-21 探針):遊戲把聊天渲染成多份文件(完整歷史 + 可視視窗 + 快照),
    每次更新整份複製到新位址(雙緩衝乒乓),舊副本凍結。聊天可視窗是遊戲聊天框實際顯示的
    最近數十行(含所有發送者),是『滾動有序』的:新訊息從底部進、舊的從頂部出。

    追這個可視窗:每輪找 align_append 能延續上次視窗的候選(滾動對齊),取其尾端新增行。
    可視窗有序 → 過期快照/舊副本相對當前視窗是『倒退』,對齊自然失敗被排除,不會像
    無序的歷史文件那樣在多份間乒乓;重複訊息會讓視窗尾端真的多一行,逐則偵測得到。
    首次以『發送者最多樣的最長候選』認出可視窗(排除只含自己的小回顯 buffer)。"""

    def __init__(self, process_name: str = PROCESS_NAME):
        self.process_name = process_name
        self._win: list[str] | None = None  # 目前追蹤的可視窗行序列;None = 未初始化
        self._addrs: list[int] = []    # 快取的可視窗文件位址(乒乓副本)
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
        """全掃(慢):回傳所有可視窗大小的候選 [(位址, 位元組長, 行list), ...]。"""
        out = []
        for base, blob in _iter_regions(h):
            for start, end, lines in groups_in_blob(blob):
                if _MIN_WIN <= len(lines) <= _MAX_WIN:
                    out.append((base + start, end - start, lines))
        return out

    def _cached_docs(self, h):
        """只讀快取位址(快):回傳那些位址附近的可視窗候選。"""
        out = []
        need = min(self._bytes + _DOC_MARGIN, _MAX_DOC_READ)
        for addr in self._addrs:
            blob = _read_clamped(h, addr, need)
            for start, end, lines in groups_in_blob(blob):
                if _MIN_WIN <= len(lines) <= _MAX_WIN:
                    out.append((addr + start, end - start, lines))
        return out

    # --- 主流程 ---
    @property
    def anchored(self) -> bool:
        """是否已認出可視窗。"""
        return self._win is not None

    def read_new(self) -> list[str]:
        """回傳自上次呼叫後新增的聊天行(依序、含重複);無新訊息回傳 []。
        找不到遊戲丟 GameNotRunning。"""
        h = self._open()
        try:
            if self._win is not None and self._addrs and self._since_full < _FULL_EVERY_DOC:
                docs = self._cached_docs(h)
                if any(align_append(self._win, l) is not None for _, _, l in docs):
                    self._since_full += 1
                    return self._emit(docs, refresh_addrs=False)
            self._since_full = 0
            return self._emit(self._full_docs(h), refresh_addrs=True)  # 首次/搬家/校正:全掃
        finally:
            self._close(h)

    @staticmethod
    def _pick_window(docs):
        """認出聊天可視窗:發送者最多樣、其次最長的候選(排除只含自己的小 buffer)。"""
        best = None
        for addr, nbytes, lines in docs:
            key = (_senders(lines), len(lines))
            if best is None or key > best[0]:
                best = (key, addr, nbytes, lines)
        return best  # (key, addr, nbytes, lines) 或 None

    def _emit(self, docs, refresh_addrs: bool) -> list[str]:
        if self._win is None:
            # 首次:認出可視窗當基準,不輸出既有內容
            picked = self._pick_window(docs)
            if picked:
                self._win = list(picked[3])
                if refresh_addrs:
                    self._addrs = self._twin_addrs(docs)
                    self._bytes = picked[2]
            return []

        # 找滾動延續當前視窗的候選:取最完整(最新)的那份。
        best = None  # (lines, new, nbytes)
        for addr, nbytes, lines in docs:
            new = align_append(self._win, lines)
            if new is None:
                continue  # 過期快照/舊副本(倒退)→ 對齊失敗,排除
            if best is None or len(lines) > len(best[0]):
                best = (lines, new, nbytes)

        if best is None:
            # 沒有延續當前視窗的(視窗被整批換掉/搬到不同大小)→ 重新認視窗,不輸出
            picked = self._pick_window(docs)
            if picked:
                self._win = list(picked[3])
                if refresh_addrs:
                    self._addrs = self._twin_addrs(docs)
                    self._bytes = picked[2]
            return []

        lines, new, nbytes = best
        if refresh_addrs:
            self._addrs = self._twin_addrs(docs, self._win)
            self._bytes = nbytes
        self._win = list(lines)
        if len(new) > _MAX_CATCHUP:
            return []  # 一次冒出太多 = 對到大跳,只重新同步不倒舊訊息
        return new

    def _twin_addrs(self, docs, win=None) -> list[int]:
        """快取的可視窗副本位址:延續 win 的(乒乓副本);win 未給時取發送者多樣的幾份。"""
        if win is not None:
            addrs = [a for a, _, l in docs if align_append(win, l) is not None]
            if addrs:
                return addrs[:_MAX_ANCHORS]
        return [a for a, _, _ in sorted(
            docs, key=lambda d: (_senders(d[2]), len(d[2])), reverse=True)[:_MAX_ANCHORS]]
