"""收訊端：透過 wizwalker 掛入遊戲、讀聊天顯示控件 `chatLog` 的全文，差分出新增行。

聊天在記憶體/顯示層是帶標記的富文字，每則一行、以 `\n` 分隔：
    他人： <color;..><image;Art/Art_Chat_Say.dds;..> <link;GID:<id>,<名>,2>[<名>]</link> 內文 </color>
    自己： <color;..><image;Art/Art_Chat_Say.dds;..> [你] 內文 </color>          ← 無 <link;GID>
玩家發言（他人與自己）都帶 Art_Chat 頻道圖示；系統訊息用 Art_Chat_System、除錯行無圖示，
以此過濾出玩家發言，再去標記回傳乾淨的「[發送者] 內文」，
連同行首 <color;..> 的遊戲顯示色一起帶出（ChatLine，供 overlay 對齊遊戲配色）。

wizwalker 靠 root-window hook 定位 `chatLog` 控件（穩定、有序、含他人訊息），
取代舊的全記憶體掃描 + 活文件定錨。注意：wizwalker 為了 hook 會寫入遊戲程序記憶體
（注入），非純讀。
"""
import asyncio
import os
import re
import sys
from collections import deque
from typing import NamedTuple

from src.reader import hook_state

PROCESS_NAME = "WizardGraphicalClient.exe"
INPUT_CONTAINER = "chatEditContainer"  # 遊戲聊天輸入區容器：開啟輸入時 is_visible 翻 True（實測）

# 單輪 poll 的新增行數上限。實測 chatLog 會在「約 110 行的短清單」與「上千行的完整歷史」
# 之間反覆跳動；只要歷史開頭那行碰巧等於基準尾行（聊天充滿 lol/gg 等重複短行），
# align_append 就會把整段舊訊息當成新訊息回吐——而且它是唯一不印 log 的路徑，難以察覺。
# 一輪只隔 poll_interval 秒，真實聊天不可能新增這麼多，超過即判定為差分誤對齊。
# 這道防線套在所有路徑的共同出口，不只擋 align_append。
MAX_NEW_LINES_PER_POLL = 100
# 未達上限但異常大的批次：照吐，但留下診斷數據供事後判斷差分是否誤判
LARGE_BATCH_LOG_THRESHOLD = 10
# 「看過集合」容量上限（行數，FIFO 淘汰最舊）。聊天分頁共用同一個 chatLog 控件
# （實測無分頁狀態可讀），切分頁＝內容換成另一視圖，recover/reset 會把重新浮上來的
# 歷史誤判成新訊息；每輪把讀到的行記進集合，慢路徑吐出前剔除集合裡已有的行。
# 不能改存「最近幾份基準」：append 快路徑每輪都在換基準，停留同一視圖幾輪
# 就會把其他視圖的證據擠掉（實測破功），集合只受總量上限影響。
SEEN_LINES_CAP = 10000


class GameNotRunning(Exception):
    """找不到遊戲程序，或無法連上/掛入。"""


# --- 純函式：標記解析（可單元測試，不需遊戲）---
class ChatLine(NamedTuple):
    """一行乾淨的玩家聊天，帶遊戲顯示色（行內 <color;..>，overlay 用它對齊遊戲配色）。"""
    text: str
    color: str | None


_TAG = re.compile(r"<[^>]*>")
# 顏色標記的值為 6 位 RRGGBB 或 8 位 AARRGGBB（帶 alpha），顯示色一律取後 6 位
_COLOR_TAG = re.compile(r"<color;([0-9a-fA-F]{6,8})>")
_VALID = re.compile(r"^\[[^\]]{1,40}\] .+")
# 聊天頻道圖示：玩家發言（他人與自己）行都含 Art_Chat_<頻道>；系統訊息用 Art_Chat_System。
# 自己的發言是 [你] 開頭、無 <link;GID>，故不能只靠 link 過濾。
_CHAT_IMG = "<image;Art/Art_Chat"
_SYSTEM_IMG = "<image;Art/Art_Chat_System"

# 遊戲表情符號：訊息內文以 <image;Emoticons/名稱.dds;24;24;..> 內嵌，保留成 ：名稱： 文字
# （不轉成 emoji，只保留表情本身，避免整行只有表情時被去光而消失）。
_EMOTE_TAG = re.compile(r"<image;Emoticons/([^.;>]+)\.dds[^>]*>", re.IGNORECASE)


def _emote_to_char(m: re.Match) -> str:
    name = re.sub(r"^emoticons?_|\d+$", "", m.group(1).lower())
    if not re.fullmatch(r"[a-z0-9_]+", name):
        return ""  # 撕裂的標記（名稱夾入雜字）：丟棄該表情，保留整行其餘內容
    return f":{name}:"  # 保留成 ：名稱： 文字


def clean(text: str) -> str:
    """表情標記保留成 ：名稱：，去掉 <color;..> <image;..> <link;..> </..> 等標記，
    還原玩家實際打出的 &lt; &gt; &amp； 實體，壓縮空白。"""
    text = _EMOTE_TAG.sub(_emote_to_char, text)
    text = _TAG.sub("", text)
    text = text.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
    return " ".join(text.replace("\x00", " ").split())


def line_color(raw: str) -> str | None:
    """從一行原始 markup 取遊戲顯示色（`#rrggbb` 小寫）；沒有顏色標記回傳 None。"""
    m = _COLOR_TAG.search(raw)
    return f"#{m.group(1)[-6:].lower()}" if m else None


def lines_from_chatlog(text: str) -> list[ChatLine]:
    """把 chatLog 控件全文（以 `\n` 分行的渲染 markup）解析成乾淨玩家聊天行
    （ChatLine：文字＋遊戲顯示色），保留順序與重複。

    收玩家發言（含**自己**的 `[你]` 行與他人 `<link;GID>[名]` 行，兩者都帶 Art_Chat 頻道圖示）；
    濾掉系統訊息（Art_Chat_System：掉寶/經驗/升等）與遊戲除錯行（[STAT]/[DBGL] 無 Art_Chat 圖示）。"""
    out: list[ChatLine] = []
    for raw in text.split("\n"):
        if _CHAT_IMG not in raw or _SYSTEM_IMG in raw:
            continue
        line = clean(raw)
        if _VALID.match(line):
            out.append(ChatLine(line, line_color(raw)))
    return out


def node_sizes(texts: list[str]) -> list[int]:
    """各 chatLog 節點各自的玩家聊天行數，依串接時的排序。診斷用：看得出是哪個節點
    在灌入完整歷史，以及 sorted 的名次有沒有翻轉（翻轉時同一組節點的順序會對調）。
    只在要印診斷 log 時呼叫——每輪都算等於把解析成本翻倍。"""
    return [len(lines_from_chatlog(t)) for t in sorted(texts)]


def _find_last_run(cur_lines: list[str], seq: list[str]) -> int | None:
    """seq 以連續片段出現在 cur_lines 中的**最後**位置；找不到回傳 None。"""
    n = len(seq)
    for i in range(len(cur_lines) - n, -1, -1):
        if cur_lines[i:i + n] == seq:
            return i
    return None


def align_append(prev_lines: list[str], cur_lines: list[str]) -> list[str] | None:
    """附加/捲動對齊（快路徑）：找最短的「prev 去掉前 k 行」正好是 cur 的前綴，
    回傳 cur 尾端多出來的行（新訊息，含重複、依序）；對不齊回傳 None。
    聊天記錄平時純附加（k=0 必中）；達顯示上限修剪頭部時 k>0 吸收捲動。
    對不齊時呼叫端改走 align_recover——本函式必須嚴格要求前綴，
    否則「尾行與新行重複」（[hi]→[hi,hi]）會被誤判為無新增而漏訊。"""
    if not prev_lines:
        return None
    for k in range(len(prev_lines)):
        overlap = prev_lines[k:]
        if cur_lines[:len(overlap)] == overlap:
            return cur_lines[len(overlap):]
    return None


def align_recover(prev_lines: list[str], cur_lines: list[str]) -> list[str] | None:
    """恢復對齊（align_append 對不齊時的退路）：找最長的 prev 尾段 prev[k:]，
    其以連續片段出現在 cur 的**最後**位置，回傳該片段之後的行；
    完全無重疊回傳 None（呼叫端視為聊天重置）。

    走到這裡代表讀取異常——撕裂讀取（遊戲寫入中讀到中段缺行/壞行的全文）
    或多 chatLog 串接結構變化。取「最後」位置是關鍵：聊天充滿重複行
    （lol/gg 等），錨到較早的重複行會把其後整段舊訊息當新行重吐
    （翻譯洪水＋timeout 螺旋）；錨到最後頂多漏掉少數重複的新行，代價小得多。"""
    if not prev_lines:
        return None
    for k in range(len(prev_lines)):
        overlap = prev_lines[k:]
        idx = _find_last_run(cur_lines, overlap)
        if idx is not None:
            return cur_lines[idx + len(overlap):]
    return None


def filter_resurfaced(emitted: list[ChatLine], seen: set[str]) -> list[ChatLine]:
    """剔除看過集合已有的行（＝視圖切換時重新浮上來的歷史），保留真正的新行。
    只用在 recover/reset 慢路徑：代價是恰在視圖切換那一輪出現的「與近期舊訊息
    一字不差的重複句」會被略過，與 align_recover 既有的取捨一致。"""
    return [line for line in emitted if line.text not in seen]


# --- 遊戲安裝路徑偵測（wizwalker 需要它讀 Data/GameData 的 WAD） ---
def detect_install_path() -> str | None:
    """從執行中的 WizardGraphicalClient.exe 推導遊戲根目錄（...\\Bin\\ 的上一層）。
    找不到回傳 None。用 pywin32 列舉程序，不掃描記憶體。"""
    try:
        import win32api
        import win32process
    except ImportError:
        return None
    for pid in win32process.EnumProcesses():
        try:
            h = win32api.OpenProcess(0x0410, False, pid)  # QUERY_INFORMATION | VM_READ
            try:
                path = win32process.GetModuleFileNameEx(h, 0)
            finally:
                win32api.CloseHandle(h)
        except Exception:
            continue
        if path and path.lower().endswith(PROCESS_NAME.lower()):
            return os.path.dirname(os.path.dirname(path))  # ...\Bin\exe → 根目錄
    return None


def _pid_alive(pid: int) -> bool:
    """PID 是否仍在執行（供清掉殘留狀態檔）；判斷不了就當活著，不誤刪。"""
    try:
        import win32process
        return pid in win32process.EnumProcesses()
    except Exception:
        return True


class WizChatReader:
    """透過 wizwalker 讀 `chatLog` 全文，回傳每輪新增的玩家聊天行。

    首次連上只記錄現況、不回吐既有歷史（只翻之後的新訊息）。之後每輪讀完整聊天記錄，
    與上輪做尾端差分（align_append）取新增行；重複訊息因逐行保留不會漏。
    空讀（傳送/轉場時聊天暫態清空）保留基準、忽略，避免填回同樣歷史時重譯；
    與基準對不齊（relog/清空成全新內容）則視為新訊息輸出。
    所有路徑共用一道出口防線：單輪吐出超過 MAX_NEW_LINES_PER_POLL 行視為差分誤對齊，
    不吐並重建基準（見該常數的說明）。"""

    def __init__(self, game_path: str | None = None, process_name: str = PROCESS_NAME):
        self.process_name = process_name
        self._game_path = game_path
        self._prev: list[str] = []  # 基準只存文字：顏色不參與差分（見 read_new）
        self._seen: set[str] = set()          # 近期讀過的行文字（各視圖聯集）
        self._seen_order: deque[str] = deque()  # 進入順序，供容量上限 FIFO 淘汰
        self._node_count: int | None = None  # 上輪讀到的 chatLog 節點數（變動＝串接結構改變）
        self._synced = False          # 是否已建立初始基準（建立後才開始回報新增）
        self._connected = False
        self._loop: asyncio.AbstractEventLoop | None = None
        self._handler = None
        self._client = None
        self._pid = 0
        self._edit_node = None  # chatEditContainer 節點快取（input_open 用）

    @property
    def anchored(self) -> bool:
        """是否已連上並掛入遊戲（未連上時上層顯示『定位中』）。"""
        return self._connected

    def read_new(self) -> list[ChatLine]:
        """回傳自上次呼叫後新增的玩家聊天行（依序、含重複）；無新訊息回傳 []。
        找不到遊戲或連線中斷丟 GameNotRunning。"""
        texts = self._read_chatlog_texts()
        # 控件列舉順序不保證穩定：排序讓多節點的串接結果確定，差分才有意義
        cur = lines_from_chatlog("\n".join(sorted(texts)))
        # 差分只看文字：切頻道時 chatLog 會把同樣的訊息以該頻道顏色重新染色，
        # 顏色參與相等比較會被誤判成「無重疊 → reset」而重吐整份舊訊息（重複翻譯）
        cur_texts = [l.text for l in cur]
        if not self._synced:
            self._prev = cur_texts  # 首次連上：記錄現況（含既有歷史），不回吐
            self._remember(cur_texts)
            self._node_count = len(texts)
            self._synced = True
            print(f"[reader] baseline established (lines={len(cur)}, "
                  f"nodes={len(texts)})", file=sys.stderr)
            return []
        if not cur:
            return []               # 空讀（傳送/轉場暫態清空）→ 保留基準、忽略，不重譯
        if len(texts) != self._node_count:
            # 節點數量變動（UI 事件生出/收掉 chatLog）→ 串接結構改變，無法歸因新舊：
            # 靜默重建基準、不回吐，避免把其他節點的舊內容當成新訊息（洪水）
            print(f"[reader] chatLog node count changed "
                  f"({self._node_count}->{len(texts)}, sizes={node_sizes(texts)}), "
                  f"re-baselining without emitting", file=sys.stderr)
            self._node_count = len(texts)
            self._prev = cur_texts
            self._remember(cur_texts)
            return []
        prev_len = len(self._prev)
        path = "append"
        appended = align_append(self._prev, cur_texts)
        if appended is None:
            # 前綴對不齊（撕裂讀取等）→ 以尾段在 cur 的最後出現位置恢復
            path = "recover"
            appended = align_recover(self._prev, cur_texts)
            if appended is not None:
                print(f"[reader] baseline misaligned, recovered via tail anchor "
                      f"(prev={prev_len}, cur={len(cur)}, emitted={len(appended)}, "
                      f"nodes={len(texts)}, sizes={node_sizes(texts)})", file=sys.stderr)
        if appended is None:
            # 與基準完全無重疊 → 聊天已重置（relog/清空成全新內容），cur 全部視為新訊息。
            # 印記錄供事後查證：若此路徑在非 relog 情境被觸發，代表差分邏輯仍有漏洞。
            path = "reset"
            appended = cur_texts
            # 帶頭尾樣本：事後才分得出是內容真的全新（relog）還是差分誤判（如另一視圖）
            print(f"[reader] chat log has no overlap with baseline, treating as reset "
                  f"(lines={len(cur)}, cur_head={cur_texts[0][:40]!r}, "
                  f"prev_tail={self._prev[-1][:40] if self._prev else ''!r})",
                  file=sys.stderr)
        self._prev = cur_texts
        # 對齊各路徑回傳的都是 cur 的尾段：以長度切回 ChatLine，帶出當前顏色
        emitted = cur[len(cur) - len(appended):]
        # 慢路徑（視圖切換/異常讀取）才過濾：append 快路徑的正常重複發言不受影響。
        # 過濾要在 _remember 之前——本輪剛出現的新行還不在集合裡，才吐得出來。
        if path != "append" and emitted:
            kept = filter_resurfaced(emitted, self._seen)
            if len(kept) != len(emitted):
                print(f"[reader] suppressed {len(emitted) - len(kept)} resurfaced "
                      f"lines via {path} (kept={len(kept)}, "
                      f"seen={len(self._seen)})", file=sys.stderr)
            emitted = kept
        self._remember(cur_texts)
        return self._guard_burst(emitted, path, prev_len, len(cur), texts)

    def _remember(self, texts: list[str]) -> None:
        """把行文字記進看過集合；超過 SEEN_LINES_CAP 從最舊的開始淘汰。"""
        for t in texts:
            if t not in self._seen:
                self._seen.add(t)
                self._seen_order.append(t)
        while len(self._seen_order) > SEEN_LINES_CAP:
            self._seen.discard(self._seen_order.popleft())

    def _guard_burst(self, appended: list[ChatLine], path: str, prev_len: int,
                     cur_len: int, texts: list[str]) -> list[ChatLine]:
        """所有差分路徑的共同出口：擋下不可能為真的暴量新增（見 MAX_NEW_LINES_PER_POLL），
        並為接近上限的批次留下診斷數據。基準已在呼叫端更新，擋下即等同靜默重建基準。"""
        if len(appended) > MAX_NEW_LINES_PER_POLL:
            print(f"[reader] implausible burst suppressed via {path}: {len(appended)} new "
                  f"lines in one poll (prev={prev_len}, cur={cur_len}, "
                  f"nodes={len(texts)}, sizes={node_sizes(texts)}); "
                  f"re-baselined without emitting", file=sys.stderr)
            return []
        if len(appended) > LARGE_BATCH_LOG_THRESHOLD:
            print(f"[reader] large batch via {path}: {len(appended)} lines "
                  f"(prev={prev_len}, cur={cur_len}, nodes={len(texts)}, "
                  f"sizes={node_sizes(texts)})", file=sys.stderr)
        return appended

    def input_open(self) -> bool:
        """遊戲聊天輸入框目前是否開啟；未連上或讀取失敗一律視為關閉。
        節點快取：首次全樹搜尋一次，之後每輪只讀一個可見性旗標；節點失效自動重找。"""
        if not self._connected:
            return False
        try:
            return self._run(self._input_open_async())
        except Exception:
            self._edit_node = None
            return False

    async def _input_open_async(self) -> bool:
        if self._edit_node is None:
            nodes = await self._client.root_window.get_windows_with_name(INPUT_CONTAINER)
            if not nodes:
                return False
            self._edit_node = nodes[0]
        try:
            return await self._edit_node.is_visible()
        except Exception:
            self._edit_node = None  # 控件被遊戲重建：下一輪重找
            return False

    def _read_chatlog_texts(self) -> list[str]:
        """讀所有 `chatLog` 控件的全文（每節點一個字串）；連線中斷則丟 GameNotRunning。"""
        if not self._connected:
            self._connect()
        try:
            return self._grab_texts()
        except Exception as exc:  # 遊戲關閉/文件釋放/記憶體讀取失敗 → 視為斷線，由上層重連
            self._teardown()
            raise GameNotRunning(f"讀取聊天失敗（可能已離開遊戲）：{exc}") from exc

    # --- 與 wizwalker 的 I/O 接縫（測試中覆寫 _grab_texts）---
    def _grab_texts(self) -> list[str]:
        async def _grab() -> list[str]:
            nodes = await self._client.root_window.get_windows_with_name("chatLog")
            return [await n.maybe_text() for n in nodes]

        return self._run(_grab())

    def _connect(self) -> None:
        import wizwalker.utils
        from wizwalker import ClientHandler

        path = self._game_path or detect_install_path()
        if path:
            wizwalker.utils._OVERRIDE_PATH = path  # Steam 版無登錄檔安裝路徑，需覆寫

        self._loop = asyncio.new_event_loop()
        self._handler = ClientHandler()
        clients = self._handler.get_new_clients()
        if not clients:
            self._teardown()
            raise GameNotRunning(f"找不到 {self.process_name}")
        self._client = clients[0]
        self._pid = self._client.process_id
        hook_state.sweep(_pid_alive)          # 清掉已不在執行的程序的殘留狀態檔
        self._repair_leaked_hooks(self._pid)  # 修復上次髒退出遺留的 hook（免重開遊戲）
        try:
            # 只啟讀聊天所需的 root_window hook（不啟 player/duel/quest 等），
            # 注入最小化、且不受是否在世界內等遊戲狀態影響。
            self._run(self._client.hook_handler.activate_root_window_hook())
        except Exception as exc:
            self._teardown()
            raise GameNotRunning(f"無法掛入遊戲（{exc}）") from exc
        self._connected = True
        print(f"[reader] attached to game (pid={self._pid})", file=sys.stderr)
        self._save_hook_state(self._pid)      # 掛入成功 → 存還原狀態，供下次髒退出修復

    def _run(self, coro):
        return self._loop.run_until_complete(coro)

    def _module_base(self) -> int:
        try:
            return self._client._pymem.base_address
        except Exception:
            return self._client.hook_handler.process.base_address

    def _repair_leaked_hooks(self, pid: int) -> None:
        """若偵測到上次對同一 process 髒退出遺留的 hook，把原始 bytes 寫回（等同 unhook）。
        module base 不符（PID 被重用給別的程序）則視為過期、不套用，只刪檔。"""
        saved_base, ops = hook_state.load_state(pid)
        if not ops:
            return
        try:
            match = saved_base == self._module_base()
        except Exception:
            match = False
        if match:
            for addr, original in ops:
                try:
                    self._run(self._client.hook_handler.write_bytes(addr, original))
                except Exception:
                    pass
            print(f"[reader] repaired hooks leaked by previous dirty exit "
                  f"(writes={len(ops)}, pid={pid}), no game restart needed", file=sys.stderr)
        hook_state.clear_state(pid)  # 套用或過期，一律刪除

    def _save_hook_state(self, pid: int) -> None:
        """把 unhook 所需狀態（autobot 原始 prologue + 每個 hook 的 jump 原碼）存檔。"""
        h = self._client.hook_handler
        ops: list[tuple[int, bytes]] = []
        addr = getattr(h, "_autobot_address", None)
        obytes = getattr(h, "_original_autobot_bytes", None)
        if addr and obytes:
            ops.append((addr, bytes(obytes)))
        for hook in getattr(h, "_active_hooks", {}).values():
            ja = getattr(hook, "jump_address", None)
            jb = getattr(hook, "jump_original_bytecode", None)
            if ja and jb:
                ops.append((ja, bytes(jb)))
        try:
            hook_state.save_state(pid, self._module_base(), ops)
        except Exception:
            pass

    def _teardown(self) -> None:
        """關閉 wizwalker 連線與事件迴圈，回到未連線狀態（下次 read_new 會重連）。"""
        unhooked = False
        try:
            if self._handler is not None and self._loop is not None:
                self._loop.run_until_complete(self._handler.close())
                unhooked = True
        except Exception as exc:
            # 解 hook 失敗：狀態檔會留著，下次啟動由 _repair_leaked_hooks 寫回原始 bytes。
            # 過去這裡靜默吞掉，導致上層照樣印 shutdown complete，下次啟動才冒出
            # 「repaired hooks leaked by previous dirty exit」而查不出原因。
            print(f"[reader] unhook failed, leaving repair state for next launch: {exc}",
                  file=sys.stderr)
        if unhooked and self._pid:
            hook_state.clear_state(self._pid)  # 已乾淨 unhook → 無遺留，清除還原狀態
        try:
            if self._loop is not None:
                self._loop.close()
        except Exception:
            pass
        self._loop = None
        self._handler = None
        self._client = None
        self._edit_node = None
        self._connected = False

    def close(self) -> None:
        """停止時呼叫：解除 hook、關閉連線。"""
        self._teardown()
