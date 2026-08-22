"""收訊端:透過 wizwalker 掛入遊戲、讀聊天顯示控件 `chatLog` 的全文,差分出新增行。

聊天在記憶體/顯示層是帶標記的富文字,每則一行、以 `\n` 分隔:
    他人: <color;..><image;Art/Art_Chat_Say.dds;..> <link;GID:<id>,<名>,2>[<名>]</link> 內文 </color>
    自己: <color;..><image;Art/Art_Chat_Say.dds;..> [你] 內文 </color>          ← 無 <link;GID>
玩家發言(他人與自己)都帶 Art_Chat 頻道圖示;系統訊息用 Art_Chat_System、除錯行無圖示,
以此過濾出玩家發言,再去標記回傳乾淨的「[發送者] 內文」。

wizwalker 靠 root-window hook 定位 `chatLog` 控件(穩定、有序、含他人訊息),
取代舊的全記憶體掃描 + 活文件定錨。注意:wizwalker 為了 hook 會寫入遊戲程序記憶體
(注入),非純讀。
"""
import asyncio
import os
import re
import sys

from src.reader import hook_state

PROCESS_NAME = "WizardGraphicalClient.exe"


class GameNotRunning(Exception):
    """找不到遊戲程序,或無法連上/掛入。"""


# --- 純函式:標記解析(可單元測試,不需遊戲)---
_TAG = re.compile(r"<[^>]*>")
_VALID = re.compile(r"^\[[^\]]{1,40}\] .+")
# 聊天頻道圖示:玩家發言(他人與自己)行都含 Art_Chat_<頻道>;系統訊息用 Art_Chat_System。
# 自己的發言是 [你] 開頭、無 <link;GID>,故不能只靠 link 過濾。
_CHAT_IMG = "<image;Art/Art_Chat"
_SYSTEM_IMG = "<image;Art/Art_Chat_System"

# 遊戲表情符號:訊息內文以 <image;Emoticons/名稱.dds;24;24;..> 內嵌,保留成 :名稱: 文字
# (不轉成 emoji,只保留表情本身,避免整行只有表情時被去光而消失)。
_EMOTE_TAG = re.compile(r"<image;Emoticons/([^.;>]+)\.dds[^>]*>", re.IGNORECASE)


def _emote_to_char(m: re.Match) -> str:
    name = re.sub(r"^emoticons?_|\d+$", "", m.group(1).lower())
    if not re.fullmatch(r"[a-z0-9_]+", name):
        return ""  # 撕裂的標記(名稱夾入雜字):丟棄該表情,保留整行其餘內容
    return f":{name}:"  # 保留成 :名稱: 文字


def clean(text: str) -> str:
    """表情標記保留成 :名稱:,去掉 <color;..> <image;..> <link;..> </..> 等標記,
    還原玩家實際打出的 &lt; &gt; &amp; 實體,壓縮空白。"""
    text = _EMOTE_TAG.sub(_emote_to_char, text)
    text = _TAG.sub("", text)
    text = text.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
    return " ".join(text.replace("\x00", " ").split())


def lines_from_chatlog(text: str) -> list[str]:
    """把 chatLog 控件全文(以 `\n` 分行的渲染 markup)解析成乾淨玩家聊天行,保留順序與重複。

    收玩家發言(含**自己**的 `[你]` 行與他人 `<link;GID>[名]` 行,兩者都帶 Art_Chat 頻道圖示);
    濾掉系統訊息(Art_Chat_System:掉寶/經驗/升等)與遊戲除錯行([STAT]/[DBGL] 無 Art_Chat 圖示)。"""
    out: list[str] = []
    for raw in text.split("\n"):
        if _CHAT_IMG not in raw or _SYSTEM_IMG in raw:
            continue
        line = clean(raw)
        if _VALID.match(line):
            out.append(line)
    return out


def align_append(prev_lines: list[str], cur_lines: list[str]) -> list[str] | None:
    """附加/捲動對齊:找最短的『prev 去掉前 k 行』正好是 cur 的前綴,
    回傳 cur 尾端多出來的行(新訊息,含重複、依序);對不齊回傳 None。
    聊天記錄平時純附加(k=0 必中);達顯示上限修剪頭部時 k>0 吸收捲動。"""
    if not prev_lines:
        return None
    for k in range(len(prev_lines)):
        overlap = prev_lines[k:]
        if cur_lines[:len(overlap)] == overlap:
            return cur_lines[len(overlap):]
    return None


# --- 遊戲安裝路徑偵測(wizwalker 需要它讀 Data/GameData 的 WAD) ---
def detect_install_path() -> str | None:
    """從執行中的 WizardGraphicalClient.exe 推導遊戲根目錄(...\\Bin\\ 的上一層)。
    找不到回傳 None。用 pywin32 列舉程序,不掃描記憶體。"""
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
    """PID 是否仍在執行(供清掉殘留狀態檔);判斷不了就當活著,不誤刪。"""
    try:
        import win32process
        return pid in win32process.EnumProcesses()
    except Exception:
        return True


class WizChatReader:
    """透過 wizwalker 讀 `chatLog` 全文,回傳每輪新增的玩家聊天行。

    首次連上只記錄現況、不回吐既有歷史(只翻之後的新訊息)。之後每輪讀完整聊天記錄,
    與上輪做尾端差分(align_append)取新增行;重複訊息因逐行保留不會漏。
    空讀(傳送/轉場時聊天暫態清空)保留基準、忽略,避免填回同樣歷史時重譯;
    與基準對不齊(relog/清空成全新內容)則視為新訊息輸出。"""

    def __init__(self, game_path: str | None = None, process_name: str = PROCESS_NAME):
        self.process_name = process_name
        self._game_path = game_path
        self._prev: list[str] = []
        self._synced = False          # 是否已建立初始基準(建立後才開始回報新增)
        self._connected = False
        self._loop: asyncio.AbstractEventLoop | None = None
        self._handler = None
        self._client = None
        self._pid = 0

    @property
    def anchored(self) -> bool:
        """是否已連上並掛入遊戲(未連上時上層顯示『定位中』)。"""
        return self._connected

    def read_new(self) -> list[str]:
        """回傳自上次呼叫後新增的玩家聊天行(依序、含重複);無新訊息回傳 []。
        找不到遊戲或連線中斷丟 GameNotRunning。"""
        text = self._read_chatlog_text()
        cur = lines_from_chatlog(text)
        if not self._synced:
            self._prev = cur        # 首次連上:記錄現況(含既有歷史),不回吐
            self._synced = True
            return []
        if not cur:
            return []               # 空讀(傳送/轉場暫態清空)→ 保留基準、忽略,不重譯
        appended = align_append(self._prev, cur)
        if appended is None:
            # 與基準對不齊 → 聊天已重置(relog/清空成全新內容),cur 全部視為新訊息
            self._prev = cur
            return cur
        self._prev = cur
        return appended             # 正常延續(無新增時為 [])

    def _read_chatlog_text(self) -> str:
        """讀所有 `chatLog` 控件的全文並串接;連線中斷則丟 GameNotRunning。"""
        if not self._connected:
            self._connect()
        try:
            return self._grab_text()
        except Exception as exc:  # 遊戲關閉/文件釋放/記憶體讀取失敗 → 視為斷線,由上層重連
            self._teardown()
            raise GameNotRunning(f"讀取聊天失敗（可能已離開遊戲）：{exc}") from exc

    # --- 與 wizwalker 的 I/O 接縫(測試中覆寫 _grab_text)---
    def _grab_text(self) -> str:
        async def _grab() -> str:
            nodes = await self._client.root_window.get_windows_with_name("chatLog")
            texts = [await n.maybe_text() for n in nodes]
            return "\n".join(texts)

        return self._run(_grab())

    def _connect(self) -> None:
        import wizwalker.utils
        from wizwalker import ClientHandler

        path = self._game_path or detect_install_path()
        if path:
            wizwalker.utils._OVERRIDE_PATH = path  # Steam 版無登錄檔安裝路徑,需覆寫

        self._loop = asyncio.new_event_loop()
        self._handler = ClientHandler()
        clients = self._handler.get_new_clients()
        if not clients:
            self._teardown()
            raise GameNotRunning(f"找不到 {self.process_name}")
        self._client = clients[0]
        self._pid = self._client.process_id
        hook_state.sweep(_pid_alive)          # 清掉已不在執行的程序的殘留狀態檔
        self._repair_leaked_hooks(self._pid)  # 修復上次髒退出遺留的 hook(免重開遊戲)
        try:
            # 只啟讀聊天所需的 root_window hook(不啟 player/duel/quest 等),
            # 注入最小化、且不受是否在世界內等遊戲狀態影響。
            self._run(self._client.hook_handler.activate_root_window_hook())
        except Exception as exc:
            self._teardown()
            raise GameNotRunning(f"無法掛入遊戲（{exc}）") from exc
        self._connected = True
        self._save_hook_state(self._pid)      # 掛入成功 → 存還原狀態,供下次髒退出修復

    def _run(self, coro):
        return self._loop.run_until_complete(coro)

    def _module_base(self) -> int:
        try:
            return self._client._pymem.base_address
        except Exception:
            return self._client.hook_handler.process.base_address

    def _repair_leaked_hooks(self, pid: int) -> None:
        """若偵測到上次對同一 process 髒退出遺留的 hook,把原始 bytes 寫回(等同 unhook)。
        module base 不符(PID 被重用給別的程序)則視為過期、不套用,只刪檔。"""
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
            print(f"[reader] 已修復上次遺留的 hook（{len(ops)} 處），免重開遊戲", file=sys.stderr)
        hook_state.clear_state(pid)  # 套用或過期,一律刪除

    def _save_hook_state(self, pid: int) -> None:
        """把 unhook 所需狀態(autobot 原始 prologue + 每個 hook 的 jump 原碼)存檔。"""
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
        """關閉 wizwalker 連線與事件迴圈,回到未連線狀態(下次 read_new 會重連)。"""
        unhooked = False
        try:
            if self._handler is not None and self._loop is not None:
                self._loop.run_until_complete(self._handler.close())
                unhooked = True
        except Exception:
            pass
        if unhooked and self._pid:
            hook_state.clear_state(self._pid)  # 已乾淨 unhook → 無遺留,清除還原狀態
        try:
            if self._loop is not None:
                self._loop.close()
        except Exception:
            pass
        self._loop = None
        self._handler = None
        self._client = None
        self._connected = False

    def close(self) -> None:
        """停止時呼叫:解除 hook、關閉連線。"""
        self._teardown()
