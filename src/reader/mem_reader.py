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
# 白名單:只允許聊天實際會用到的字元(ASCII、CJK、全形、常用標點、BMP emoji)。
# 版面/渲染緩衝的破損副本會夾入其他區塊的字元(指標位元組被當成雜字),含任一即拒絕。
_NON_CHAT = re.compile(
    "[^\x20-\x7e -⁯←-⇿☀-➿"
    "　-〿㐀-䶿一-鿿＀-￯️‍]"
)
# 版面緩衝殘留的標記碎片:角括號、HTML 實體、.dds/FFFFFF/color> 等
_MARKUP = re.compile(r"[<>]|&(?:gt|lt|amp);|\.dds|FFFFFF|color>")
# 兩則訊息被併在一起(跨越 </color> 邊界):第一個 ] 之後還出現 [(第二個發送者)
_DOUBLE_SENDER = re.compile(r"\].*\[")

_MAX_LINE_BYTES = 1400
_MAX_LINE_CHARS = 300


def clean(text: str) -> str:
    """去掉 <color;..> <image;..> </color> 等標記,壓縮空白。"""
    return " ".join(_TAG.sub("", text).replace("\x00", " ").split())


def extract_lines(blob: bytes) -> list[str]:
    """在一段記憶體 blob 中,依聊天簽章抽出乾淨聊天行(blob 內去重,保留出現順序)。"""
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
        if (_VALID.match(txt) and len(txt) <= _MAX_LINE_CHARS
                and not _NON_CHAT.search(txt) and not _MARKUP.search(txt)
                and not _DOUBLE_SENDER.search(txt) and txt not in seen):
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


def _dedup(chunks) -> list[str]:
    out: list[str] = []
    seen: set[str] = set()
    for lines in chunks:
        for line in lines:
            if line not in seen:
                seen.add(line)
                out.append(line)
    return out


class ChatReader:
    """有狀態的聊天讀取器:全掃記住「出現過聊天的區域」,之後只重掃那些熱區(快),
    每 full_scan_every 輪(或熱區為空時)做一次完整全掃自我修正。"""

    def __init__(self, process_name: str = PROCESS_NAME, full_scan_every: int = 10):
        self.process_name = process_name
        self.full_scan_every = full_scan_every
        self._hot: list[int] = []
        self._prev_full_hot: set[int] = set()  # 上一次全掃時的熱區
        self.last_absorb: list[str] = []  # 本輪「新出現區塊」的既有內容(應吸收不翻)
        self._since_full = full_scan_every  # 讓第一次讀取一定是全掃

    # --- 可在測試中覆寫的接縫 ---
    def _open(self):
        pid = _find_pid(self.process_name)
        if not pid:
            raise GameNotRunning(f"找不到 {self.process_name}")
        h = _k32.OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, False, pid)
        if not h:
            raise GameNotRunning(f"無法開啟 {self.process_name}(可能需要系統管理員權限)")
        return h

    def _close(self, handle):
        _k32.CloseHandle(handle)

    def _full_scan(self, handle):
        """回傳 (聊天行, 有命中的區域 base 清單, 應吸收的行)。
        「應吸收的行」= 只出現在『這次全掃才新變熱』區塊的行 —— 那是剛被載入的既有歷史,
        非新訊息(新訊息接在既有活躍緩衝後,屬舊熱區),應標記看過而不翻譯。"""
        out: list[str] = []
        seen: set[str] = set()
        hot: list[int] = []
        old_lines: set[str] = set()
        new_lines: set[str] = set()
        for base, blob in _iter_regions(handle):
            found = extract_lines(blob)
            if not found:
                continue
            hot.append(base)
            bucket = new_lines if base not in self._prev_full_hot else old_lines
            for line in found:
                if line not in seen:
                    seen.add(line)
                    out.append(line)
                bucket.add(line)
        absorb = [line for line in out if line in new_lines and line not in old_lines]
        self._prev_full_hot = set(hot)
        return out, hot, absorb

    def _hot_scan(self, handle):
        return _dedup(extract_lines(_read_region_at(handle, base)) for base in self._hot)

    def read(self) -> list[str]:
        """回傳當前所有聊天行(全域去重,保留出現順序)。找不到遊戲丟 GameNotRunning。
        本輪應吸收(標記看過不翻)的行另存於 self.last_absorb。"""
        handle = self._open()
        try:
            if self._since_full >= self.full_scan_every or not self._hot:
                lines, self._hot, self.last_absorb = self._full_scan(handle)
                self._since_full = 1
            else:
                lines = self._hot_scan(handle)
                self.last_absorb = []
                self._since_full += 1
            return lines
        finally:
            self._close(handle)


_default_reader: ChatReader | None = None


def read_chat_lines(process_name: str = PROCESS_NAME) -> list[str]:
    """便利函式:每輪都做完整全掃。
    最新訊息會出現在「新配置的小緩衝(新記憶體區塊)」,熱區快取的熱掃會漏讀它們,
    故一律全掃(較慢但抓得到最新)。"""
    global _default_reader
    if _default_reader is None or _default_reader.process_name != process_name:
        _default_reader = ChatReader(process_name, full_scan_every=1)
    return _default_reader.read()


def last_absorbed() -> list[str]:
    """上一次 read_chat_lines() 判定為「應吸收」(既有歷史、標記看過不翻)的行。"""
    return list(_default_reader.last_absorb) if _default_reader else []
