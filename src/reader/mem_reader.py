"""收訊端：透過 wizwalker 掛入遊戲，以 ChatBox::AddLine hook 依序讀出新增的聊天行與分頁狀態。

本模組負責 wizwalker 連線／hook 生命週期與事件判定（主 ChatBox 過濾、廣播收斂、系統訊息開關）。
hook 與環形緩衝在 chat_hook.py、分頁狀態在 chat_tabs.py、標記解析在 markup.py、程序偵測在
process.py，都不需遊戲即可測試。

注意：root-window hook 與 AddLine hook 都會寫入遊戲程序記憶體（注入），非純讀。
"""
import asyncio
import struct
import time
from typing import NamedTuple

import win32gui

from src.log import log
from src.reader import hook_state
from src.reader.chat_hook import (
    HEADER,
    SLOT_SIZE,
    SLOTS,
    AddLineHook,
    HookRecord,
    RingRead,
    group_broadcasts,
    pending_broadcast,
    read_ring,
)
from src.reader.chat_tabs import (
    CHATBOX_WINDOW,
    CUR_TAB_OFFSET,
    LABEL_OFFSET,
    TAB_BUTTONS,
    TabState,
    read_wstring,
    visible_label,
)
from src.reader.markup import ChatLine, forget_warned_icons, parse_line
from src.reader.message_log import MessageLog
from src.reader.process import (
    install_path_of,
    pid_alive,
)
from src.reader.ui_rect import client_rect

INPUT_CONTAINER = "chatEditContainer"  # 遊戲聊天輸入區容器：開啟輸入時 is_visible 翻 True（實測）

# exe 的偏好載入位址：log 把跳躍點換算回靜態位址，才對得上反組譯
IMAGE_BASE = 0x140000000

# 等 root window 位址寫回來的上限（秒）與輪詢間隔。wizwalker 的 activate_root_window_hook
# (wait_for_ready=True) 內部是無限等：pattern 掃得到、hook 寫進去了，但那段程式碼已不在執行
# 路徑（遊戲改版的典型徵兆）時，收訊執行緒會永久卡在掛入、不拋例外，關程式時被硬砍而
# hook 殘留在遊戲記憶體裡。故一律自己等、自己逾時。
HOOK_READY_TIMEOUT = 15.0
HOOK_READY_POLL = 0.3

# 新的主 ChatBox 可能重用外來的舊位址；外來記錄限頻重找，免得每筆都掃一次 UI 樹
FOREIGN_RECHECK_SECONDS = 5.0


class ChatEvent(NamedTuple):
    """一則新聊天；`tabs` 是它所在的分頁，伺服器廣播收斂後含多個。"""
    tabs: tuple[int, ...]
    line: ChatLine


class GameNotRunning(Exception):
    """找不到遊戲程序，或無法連上/掛入。"""


class GameAccessDenied(GameNotRunning):
    """開不了遊戲程序的 handle —— 多半是遊戲以系統管理員身分執行、本程式沒有。
    只在文案上分流，上層的退避重連照舊。"""


class GameVersionMismatch(GameNotRunning):
    """掛入點與這個遊戲版本對不上，wizwalker 的 pattern 已不適用。
    只在文案上分流，上層的退避重連照舊。"""


def is_version_mismatch(exc: BaseException) -> bool:
    """掛入失敗是否為「掛入點與遊戲版本對不上」：pattern 掃不到、掃到多個（兩者都在
    寫入遊戲記憶體之前拋出）、或 hook 寫進去了卻遲遲沒被觸發（等待逾時）。
    其餘失敗（開不了程序、程序消失、讀寫錯誤）仍走「遊戲未就緒」文案。"""
    from wizwalker.errors import PatternFailed, PatternMultipleResults
    return isinstance(exc, (PatternFailed, PatternMultipleResults, TimeoutError))


class WizChatReader:
    """透過 wizwalker 掛 AddLine hook，回傳每輪新增的聊天事件與遊戲聊天框的分頁狀態。

    hook 只看得到掛上之後新增的行，不回補連上之前的歷史；序號屬於單一連線，重連從 0 開始。"""

    def __init__(self, hwnd: int, message_log: MessageLog | None = None, slot: int = 0):
        self._hwnd = hwnd
        self._slot = slot          # 只用在 log：雙開時分辨這條是哪個客戶端
        self.emit_system = False   # 是否輸出系統訊息（對應 config 的 translate_system_messages）
        self._connected = False
        self._loop: asyncio.AbstractEventLoop | None = None
        self._client = None
        self._pid = 0
        self._edit_node = None  # chatEditContainer 節點快取（input_open 用）
        self._msg_log = message_log  # messages.log（None＝不記錄，測試預設不落檔）
        self._ring = 0
        self._last_seq = 0
        self._chatbox: int | None = None  # 主 ChatBox 位址快取
        self._buttons: list[tuple[int, int]] | None = None  # 分頁按鈕（編號, 位址）快取
        self._foreign: set[int] = set()   # 確認不是主 ChatBox 的 `this`
        self._tabs_logged: object = None  # 上次記進 app.log 的分頁狀態（只在變化時記）
        self._foreign_checked = 0.0       # 上次為外來記錄重找 ChatBox 的時間（monotonic）
        self._held_from = 0               # 上一輪保留的廣播開頭序號（同一段只保留一輪）
        self._refound = False             # 本輪 read_new 是否已重找過 ChatBox
        self._no_chatbox: list[int] = []  # 本輪因找不到 ChatBox 而丟棄的序號

    @property
    def anchored(self) -> bool:
        """是否已連上並掛入遊戲（未連上時上層顯示『定位中』）。"""
        return self._connected

    def read_new(self) -> list[ChatEvent]:
        """回傳自上次呼叫後新增的聊天事件（依序、含重複）；找不到遊戲或連線中斷丟 GameNotRunning。

        `emit_system` 為 False 時不回傳系統行，但序號照推，打開開關不會倒出舊訊息。"""
        if not self._connected:
            self._connect()
        try:
            got = self._read_records()
        except Exception as exc:  # 遊戲關閉/記憶體讀取失敗 → 視為斷線，由上層重連
            self._teardown()
            raise GameNotRunning(f"chat ring read failed (likely left the game): {exc}") from exc
        first = self._last_seq + 1
        self._last_seq = got.last_seq
        if got.lost:
            self._note(f"lost {got.lost} chat lines (ring overwritten)",
                       f"window={first}-{got.last_seq}")
        records = got.records
        held = pending_broadcast(records)
        if held and records[-1].seq == got.last_seq and records[-held].seq != self._held_from:
            # 廣播可能被這一輪切開：游標停在開頭之前，下一輪連同後段一起收斂
            self._held_from = records[-held].seq
            self._last_seq = self._held_from - 1
            records = records[:-held]
        self._refound = False
        self._no_chatbox = []
        events = []
        for tabs, rec in group_broadcasts(records):
            line = self._judge(tabs, rec)
            if line is not None:
                events.append(ChatEvent(tabs, line))
        if self._no_chatbox:
            log(f"[reader] AddLine dropped, main chat box not found (slot={self._slot}, "
                f"pid={self._pid}, dropped={len(self._no_chatbox)}, "
                f"seq={self._no_chatbox[0]}-{self._no_chatbox[-1]})")
        return events

    def _judge(self, tabs: tuple[int, ...], rec: HookRecord) -> ChatLine | None:
        """判定一組記錄（廣播收斂後的代表記錄）並寫進 messages.log；要回傳的回 ChatLine。"""
        if rec.copied < rec.length:
            self._note("chat line truncated",
                       f"seq={rec.seq}, tab={rec.tab}, length={rec.length}, kept={rec.copied}")
        line = None
        if (rejected := self._chatbox_verdict(rec)) is not None:
            verdict = rejected
        elif (line := parse_line(rec.text)) is None:
            verdict = "dropped"
        elif line.system and not self.emit_system:
            verdict, line = "system-off", None
        elif len(tabs) > 1:
            verdict = "broadcast"
        else:
            verdict = "system" if line.system else "player"
        self._record(rec.seq, rec.tab, rec.text, verdict)
        if len(tabs) > 1:
            log(f"[reader] server broadcast collapsed across {len(tabs)} tabs "
                f"(slot={self._slot}, pid={self._pid}, seq={rec.seq}, tabs={tabs})")
            # 廣播是連號寫入，其餘各筆的序號與分頁可由代表記錄還原
            for i, tab in enumerate(tabs[1:], 1):
                self._record(rec.seq + i, tab, rec.text, "broadcast-dup")
        return line

    def _chatbox_verdict(self, rec: HookRecord) -> str | None:
        """`this` 是主 ChatBox 回 None（照收）；否則回 `foreign` 或 `no-chatbox`。

        不認得的 `this` 先重找 ChatBox（登出入後 ChatBox 可能是新物件），每輪 read_new 至多一次。"""
        if rec.this == self._chatbox:
            return None
        if (rec.this in self._foreign
                and time.monotonic() - self._foreign_checked < FOREIGN_RECHECK_SECONDS):
            return "foreign"
        if not self._refound:
            self._refound = True
            self._foreign_checked = time.monotonic()
            try:
                self._refind_chatbox()
            except Exception as exc:
                self._chatbox = None
                log(f"[reader] chat box lookup failed (slot={self._slot}, pid={self._pid}): "
                    f"{type(exc).__name__}: {exc}")
            if rec.this == self._chatbox:
                return None
        # 找不到 ChatBox（轉場中）時不記為外來，否則主 ChatBox 會被永久擋掉
        if self._chatbox is None:
            self._no_chatbox.append(rec.seq)
            return "no-chatbox"
        if rec.this not in self._foreign:
            self._foreign.add(rec.this)
            log(f"[reader] ignoring AddLine from another chat box (slot={self._slot}, "
                f"pid={self._pid}, this={rec.this:#x}, tab={rec.tab}, main={self._chatbox:#x})")
        return "foreign"

    def _refind_chatbox(self) -> None:
        """重找主 ChatBox；位址變了就清掉按鈕快取與外來集合（新物件可能重用外來的舊位址）。"""
        found = self._find_chatbox()
        if found is not None and found != self._chatbox:
            self._buttons = None
            self._foreign.clear()
            log(f"[reader] main chat box at {found:#x} (slot={self._slot}, pid={self._pid})")
        self._chatbox = found

    def _record(self, seq: int, tab: int, raw: str, verdict: str) -> None:
        if self._msg_log is not None:
            self._msg_log.record(seq, tab, raw, verdict)

    def _note(self, what: str, detail: str) -> None:
        """同一則事件寫進 app.log 與 messages.log。"""
        log(f"[reader] {what} (slot={self._slot}, pid={self._pid}, {detail})")
        if self._msg_log is not None:
            self._msg_log.note(f"{what} ({detail})")

    def tab_state(self) -> TabState | None:
        """遊戲聊天框的目前分頁與可見分頁；未連上、找不到 ChatBox 或讀取失敗回 None。"""
        if not self._connected:
            return None
        state = outcome = None
        try:
            if self._chatbox is None or self._buttons is None:
                self._refind_chatbox()
            if self._chatbox is not None:
                state = outcome = self._run(self._read_tab_state(self._chatbox))
        except Exception as exc:
            self._buttons = None  # 控件可能被遊戲重建：下一輪重找 ChatBox 與按鈕
            outcome = f"read failed: {type(exc).__name__}: {exc}"
        if outcome != self._tabs_logged:
            self._tabs_logged = outcome
            shown = (f"current={state.current}, visible={list(state.tabs)}" if state
                     else outcome or "unavailable (chat box not found)")
            log(f"[reader] chat tabs {shown} (slot={self._slot}, pid={self._pid})")
        return state

    async def _read_tab_state(self, chatbox: int) -> TabState:
        read = self._client.hook_handler.read_bytes
        if self._buttons is None:
            self._buttons = await self._find_tab_buttons()
        current = struct.unpack("<i", await read(chatbox + CUR_TAB_OFFSET, 4))[0]
        if current not in {tab for _, tab in TAB_BUTTONS}:
            raise ValueError(f"current tab {current} out of range (stale chat box {chatbox:#x}?)")
        tabs = []
        for tab, base in self._buttons:
            label = visible_label(await read_wstring(read, base + LABEL_OFFSET))
            if label is not None:
                tabs.append((tab, label))
        return TabState(current, tuple(tabs))

    async def _find_tab_buttons(self) -> list[tuple[int, int]]:
        found = []
        for name, tab in TAB_BUTTONS:
            nodes = await self._client.root_window.get_windows_with_name(name)
            if nodes:
                found.append((tab, await nodes[0].read_base_address()))
        return found

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
        node = await self._edit_container()
        if node is None:
            return False
        try:
            return await node.is_visible()
        except Exception:
            self._edit_node = None  # 控件被遊戲重建：下一輪重找
            return False

    async def _edit_container(self):
        """快取的 chatEditContainer 節點（找不到回 None）；input_open 與定位共用同一份快取。"""
        if self._edit_node is None:
            nodes = await self._client.root_window.get_windows_with_name(INPUT_CONTAINER)
            if not nodes:
                return None
            self._edit_node = nodes[0]
        return self._edit_node

    def input_box_screen_rect(self) -> tuple[int, int, int, int] | None:
        """遊戲聊天輸入框在螢幕上的矩形 (x, y, w, h)，供翻譯輸入框貼齊；
        未連上、節點失效或視窗量不到一律回 None（呼叫端退回預設位置）。"""
        if not self._connected:
            return None
        try:
            return self._run(self._input_rect_async())
        except Exception as exc:
            self._edit_node = None
            log(f"[reader] game chat input rect unavailable (slot={self._slot}): "
                f"{type(exc).__name__}: {exc}")
            return None

    async def _input_rect_async(self) -> tuple[int, int, int, int] | None:
        node = await self._edit_container()
        if node is None:
            return None
        rect = await node.window_rectangle()
        # x2／y2 是含端點的邊：遊戲畫出的邊框比 x2−x1 多 1 邏輯 px（實機截圖逐像素比對）
        size = (rect.x2 - rect.x1 + 1, rect.y2 - rect.y1 + 1)
        offsets = [(rect.x1, rect.y1)]
        for parent in await node.get_parents():
            parent_rect = await parent.window_rectangle()
            offsets.append((parent_rect.x1, parent_rect.y1))
        root = await self._client.root_window.window_rectangle()
        hwnd = self._client.window_handle
        _, _, client_w, client_h = win32gui.GetClientRect(hwnd)
        x, y, w, h = client_rect(offsets, size, (root.x2 - root.x1, root.y2 - root.y1),
                                 (client_w, client_h))
        origin_x, origin_y = win32gui.ClientToScreen(hwnd, (0, 0))
        return (origin_x + x, origin_y + y, w, h)

    # --- 與 wizwalker 的 I/O 接縫（測試中覆寫）---
    def _read_records(self) -> RingRead:
        return self._run(read_ring(self._client.hook_handler.read_bytes, self._ring,
                                   self._last_seq))

    def _find_chatbox(self) -> int | None:
        """root window 下 `WizardChatBox` 的位址；找不到回 None，讀取失敗照拋。"""
        async def _find() -> int | None:
            nodes = await self._client.root_window.get_windows_with_name(CHATBOX_WINDOW)
            return await nodes[0].read_base_address() if nodes else None

        return self._run(_find())

    def _install_chat_hook(self) -> int:
        """掛上 AddLine hook 並登記給 wizwalker（client.close() 時一併 unhook），回傳 ring 位址。"""
        handler = self._client.hook_handler
        hook = AddLineHook(handler)
        try:
            self._run(hook.hook())
        except Exception as exc:
            log(f"[reader] AddLine hook install failed (slot={self._slot}, pid={self._pid}): "
                f"{type(exc).__name__}: {exc}; a pattern failure here needs chat_hook.PATTERN "
                f"updated, not the wizwalker fork")
            raise
        handler._active_hooks[AddLineHook] = hook
        log(f"[reader] AddLine hook installed (slot={self._slot}, pid={self._pid}, "
            f"jump={hook.jump_address - self._module_base() + IMAGE_BASE:#x}, "
            f"ring={hook.ring:#x}, ring_size={HEADER + SLOTS * SLOT_SIZE:#x})")
        return hook.ring

    def _connect(self) -> None:
        import wizwalker
        from pymem.exception import CouldNotOpenProcess

        self._loop = asyncio.new_event_loop()
        try:
            # 綁定指定視窗：雙開時每個 reader 各掛自己的客戶端，不能拿列舉結果的第一個
            self._client = wizwalker.Client(self._hwnd)
        except CouldNotOpenProcess as exc:
            # 程序在、handle 開不了＝完整性等級對不上（medium 開不了 high）。
            # 這裡不 teardown 會每輪重試漏掉一個 event loop。
            self._teardown()
            raise GameAccessDenied(
                f"cannot open game process handle ({exc}); the game is likely "
                f"running elevated while this program is not") from exc
        except Exception as exc:
            self._teardown()
            raise GameNotRunning(f"failed to open game process: {exc}") from exc
        self._pid = self._client.process_id
        path = install_path_of(self._pid)
        log(f"[reader] game path: {path!r} (slot={self._slot}, pid={self._pid})")
        hook_state.sweep(pid_alive)          # 清掉已不在執行的程序的殘留狀態檔
        self._repair_leaked_hooks(self._pid)  # 修復上次髒退出遺留的 hook（免重開遊戲）
        try:
            # 只掛 root_window（輸入框、ChatBox 定位）與 AddLine（收訊）兩個 hook，不啟
            # player/duel/quest 等。每個 hook 寫入後立刻存還原狀態，等待途中被硬砍才修得回來；
            # AddLine 等 root 就緒才裝，逾時重試不會每次漏一塊 ring。
            self._run(self._client.hook_handler.activate_root_window_hook(
                wait_for_ready=False))
            self._save_hook_state(self._pid)
            waited = self._wait_root_window_ready()
            self._ring = self._install_chat_hook()
            self._save_hook_state(self._pid)
        except Exception as exc:
            self._teardown()
            if is_version_mismatch(exc):
                raise GameVersionMismatch(
                    f"hook does not match this game build: {exc}") from exc
            raise GameNotRunning(f"failed to attach to game: {exc}") from exc
        self._connected = True
        log(f"[reader] attached to game (slot={self._slot}, pid={self._pid}, "
            f"hwnd={self._hwnd:#x}, hook_ready_in={waited:.1f}s)")

    def _run(self, coro):
        return self._loop.run_until_complete(coro)

    def _wait_root_window_ready(self) -> float:
        """等 hook 把 current root window 位址寫回來（非 0 ＝ hook 真的被執行過），
        回傳等待秒數。逾時丟 TimeoutError，由 is_version_mismatch 歸類為版本不相容。

        hook 剛寫入時位址尚未有效，讀取失敗是常態，一律當成「還沒好」繼續等；
        真正的失敗只有逾時一種。"""
        async def _wait() -> bool:
            deadline = time.monotonic() + HOOK_READY_TIMEOUT
            while True:
                try:
                    if await self._client.hook_handler.read_current_root_window_base():
                        return True
                except Exception:
                    pass
                if time.monotonic() >= deadline:
                    return False
                await asyncio.sleep(HOOK_READY_POLL)

        started = time.monotonic()
        ready = self._run(_wait())
        elapsed = time.monotonic() - started
        if not ready:
            log(f"[reader] root window hook never fired within "
                f"{HOOK_READY_TIMEOUT:.0f}s (slot={self._slot}, pid={self._pid}); "
                f"the hook pattern likely does not match this game build")
            raise TimeoutError(
                f"root window hook did not fire within {HOOK_READY_TIMEOUT:.0f}s")
        return elapsed

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
            current_base = self._module_base()
        except Exception as exc:
            log(f"[reader] cannot read module base, ignoring hook state "
                f"(slot={self._slot}, pid={pid}): {type(exc).__name__}: {exc}")
            current_base = None
        if current_base == saved_base:
            failed = 0
            for addr, original in ops:
                try:
                    self._run(self._client.hook_handler.write_bytes(addr, original))
                except Exception as exc:
                    failed += 1
                    log(f"[reader] hook repair write failed at {addr:#x} "
                        f"(slot={self._slot}, len={len(original)}): "
                        f"{type(exc).__name__}: {exc}")
            outcome = ("no game restart needed" if not failed
                       else "restart the game if chat stays silent")
            log(f"[reader] repaired hooks leaked by previous dirty exit "
                f"(writes={len(ops) - failed}, failed={failed}, "
                f"slot={self._slot}, pid={pid}), {outcome}")
        elif current_base is not None:
            log(f"[reader] stale hook state ignored (slot={self._slot}, pid={pid} "
                f"was reused: saved_base={saved_base:#x}, current_base={current_base:#x})")
        hook_state.clear_state(pid)  # 套用或過期，一律刪除

    def _save_hook_state(self, pid: int) -> None:
        """把 unhook 所需狀態存檔：各 hook 的 jump 原碼在前、autobot 原始 prologue 在後。

        修復依序寫回：先還原 autobot 的話，仍指向它的跳躍點會跑進被改回的原始程式碼。"""
        h = self._client.hook_handler
        ops: list[tuple[int, bytes]] = []
        for hook in getattr(h, "_active_hooks", {}).values():
            ja = getattr(hook, "jump_address", None)
            jb = getattr(hook, "jump_original_bytecode", None)
            if ja and jb:
                ops.append((ja, bytes(jb)))
        addr = getattr(h, "_autobot_address", None)
        obytes = getattr(h, "_original_autobot_bytes", None)
        if addr and obytes:
            ops.append((addr, bytes(obytes)))
        try:
            hook_state.save_state(pid, self._module_base(), ops)
        except Exception as exc:
            # 存不了就修不回：下次髒退出後只能重開遊戲，得讓 app.log 看得出原因
            log(f"[reader] could not save hook state "
                f"(slot={self._slot}, pid={pid}, ops={len(ops)}): "
                f"{type(exc).__name__}: {exc}")

    def _teardown(self) -> None:
        """關閉 wizwalker 連線與事件迴圈，回到未連線狀態（下次 read_new 會重連）。"""
        unhooked = False
        try:
            if self._client is not None and self._loop is not None:
                self._loop.run_until_complete(self._client.close())
                unhooked = True
        except Exception as exc:
            # 狀態檔留著，下次啟動由 _repair_leaked_hooks 寫回原始 bytes。不能靜默吞：否則上層
            # 照印 shutdown complete，下次啟動才冒出「repaired hooks leaked」而查不出原因。
            log(f"[reader] unhook failed, leaving repair state for next launch "
                f"(slot={self._slot}): {exc}")
        if unhooked and self._pid:
            hook_state.clear_state(self._pid)  # 已乾淨 unhook → 無遺留，清除還原狀態
        try:
            if self._loop is not None:
                self._loop.close()
        except Exception:
            pass
        self._loop = None
        self._client = None
        self._edit_node = None
        self._connected = False
        # 序號與 ChatBox 位址都屬於單一連線：重連後是新的 hook 與緩衝
        self._ring = 0
        self._last_seq = 0
        self._chatbox = None
        self._buttons = None
        self._foreign.clear()
        self._tabs_logged = None
        self._held_from = 0
        forget_warned_icons()

    def close(self) -> None:
        """停止時呼叫：解除 hook、關閉連線。"""
        self._teardown()
