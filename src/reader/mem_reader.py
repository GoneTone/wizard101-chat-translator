"""收訊端：透過 wizwalker 掛入遊戲、讀聊天顯示控件 `chatLog` 的全文，差分出新增行。

本模組負責 wizwalker 連線／hook 生命週期，以及玩家軌與系統軌的差分狀態機（各門檻的
取捨都寫在常數旁）。標記解析在 markup.py、對齊演算法在 diff.py、程序偵測在 process.py，
三者都是純函式，不需遊戲即可測試。

注意：wizwalker 靠 root-window hook 定位控件，為此會寫入遊戲程序記憶體（注入），非純讀。
"""
import asyncio
import time
from typing import NamedTuple

import win32gui

from src.log import log
from src.reader import hook_state
from src.reader.diff import (
    Track,
    align,
    filter_resurfaced,
    player_out_with_idx,
)
from src.reader.markup import ChatLine, forget_warned_icons, lines_from_nodes, node_sizes
from src.reader.message_log import MessageLog
from src.reader.process import (
    detect_install_path,
    pid_alive,
)
from src.reader.ui_rect import client_rect

INPUT_CONTAINER = "chatEditContainer"  # 遊戲聊天輸入區容器：開啟輸入時 is_visible 翻 True（實測）

# 等 root window 位址寫回來的上限（秒）與輪詢間隔。wizwalker 的 activate_root_window_hook
# (wait_for_ready=True) 內部是無限等：pattern 掃得到、hook 寫進去了，但那段程式碼已不在執行
# 路徑（遊戲改版的典型徵兆）時，收訊執行緒會永久卡在掛入、不拋例外，關程式時被硬砍而
# hook 殘留在遊戲記憶體裡。故一律自己等、自己逾時。
HOOK_READY_TIMEOUT = 15.0
HOOK_READY_POLL = 0.3

# 單輪 poll 的新增行數上限，套在所有路徑的共同出口。實測 chatLog 會在約 110 行的短清單
# 與上千行的完整歷史之間跳動，歷史開頭恰等於基準尾行（lol/gg 等重複短行）時 align_append
# 會把整段舊訊息當新訊息回吐且不印 log；一輪 poll_interval 內真實聊天不可能新增這麼多。
MAX_NEW_LINES_PER_POLL = 100
# 未達上限但異常大的批次：照吐，只留診斷數據
LARGE_BATCH_LOG_THRESHOLD = 10
# 系統軌的診斷門檻：掉寶一輪十幾行是常態，套玩家軌的門檻會讓 app.log 每輪都印 large batch。
# 暴量防線 MAX_NEW_LINES_PER_POLL 兩軌沿用同值 —— 一輪超過 100 行系統訊息就是差分誤對齊。
SYSTEM_LARGE_BATCH_LOG_THRESHOLD = 40
# append 快路徑改走逐行過濾的批次大小。一輪湧出這麼多行不會是真人發言，而是轉場時 chatLog
# 把整份歷史重接一次（實測切伺服器：101→200 行、吐出 100 行舊訊息）；「全部看過」那道判定
# 是全有全無，混進一行沒讀過的就整批放行，故達此門檻改逐行剔除看過的行、沒見過的照吐。
BULK_APPEND_FILTER_MIN = 10
# 暴增批次判定為「內容重浮」的門檻：剔除看過的行後，沒讀過的行不到全批的 1/N。
# 切到沒讀過的視圖時整批都是生面孔（實測前綴巧合：1 行基準冒出 4 行全新）；
# chatLog 膨脹成重複版本時幾乎全是看過的行、只夾著剛抵達的一兩句新訊息。
DUPLICATE_BURST_SEEN_RATIO = 4
# 輸入框關聯放行的有效輪數：剛送出訊息時，遊戲輸入框必在前 1-2 輪內開啟過。登出再登入
# 不重啟程序、看過集合殘留舊 session 字樣，重打同一句（Test/lol 等）走 reset 會被誤判
# 重浮吞掉；輸入框剛關閉＋視圖尾行同字＝剛送出的訊息，放行。
INPUT_RELEASE_POLLS = 2
# 連續空讀達此輪數即視舊基準過期，恢復內容後強制走 reset 語意：置換式視圖（朋友/私訊
# 視窗只顯示最近一則）重打同一句、或登出前尾行與登入後第一句同字時，append 會誤判
# 「無變化」靜默吞掉新句（實測且無 log 可循）。門檻取小值以涵蓋兩則訊息之間的短暫清空；
# 轉場的暫態清空同樣走 reset，但填回的舊內容全在看過集合裡，不會重譯。
STALE_BASELINE_EMPTY_POLLS = 2


class _Outcome(NamedTuple):
    """一輪差分的結果：判定路徑、要吐出去的行，以及對齊路徑取得的新增行數
    （早退路徑沒有對齊，appended 為 None）。路徑名只供 messages.log 診斷。"""
    path: str
    emitted: list["ChatLine"]
    appended: int | None = None


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
    """透過 wizwalker 讀 `chatLog` 全文，回傳每輪新增的聊天行。

    首次連上只記錄現況、不回吐既有歷史；之後每輪讀完整聊天記錄，與上輪基準差分取新增行
    （重複訊息逐行保留不會漏）。空讀（傳送/轉場暫態清空）保留基準、忽略；與基準完全
    對不齊（切到沒讀過的分頁視圖／relog）視情況吸收或過濾（見 _diff_new_lines 的 reset
    分支）。所有路徑共用一道出口防線：單輪超過 MAX_NEW_LINES_PER_POLL 行視為差分誤對齊。"""

    def __init__(self, hwnd: int, game_path: str | None = None,
                 message_log: MessageLog | None = None, slot: int = 0):
        self._hwnd = hwnd
        self._slot = slot          # 只用在 log：雙開時分辨這條是哪個客戶端
        self._game_path = game_path
        # 玩家軌與系統軌各一份差分狀態、完全分離：共用同一個看過集合會讓掉寶刷屏把
        # 玩家說過的話擠出容量上限，視圖一切換那些玩家訊息就被當成沒見過而重吐重翻。
        self._player = Track()
        self._system = Track()
        self.emit_system = False   # 是否輸出系統訊息（對應 config 的 translate_system_messages）
        self._input_recent = 0     # 輸入框開啟後的剩餘關聯輪數（見 INPUT_RELEASE_POLLS）
        self._input_was_open = False
        self._released_for_input = False  # 本次輸入框開啟是否已用掉關聯放行額度
        self._empty_streak = 0     # 連續空讀輪數（見 STALE_BASELINE_EMPTY_POLLS）
        self._node_count: int | None = None  # 上輪讀到的 chatLog 節點數（變動＝串接結構改變）
        self._mirrored_nodes = 0   # 上輪剔除的鏡射節點數（見 lines_from_nodes）
        self._synced = False          # 是否已建立初始基準（建立後才開始回報新增）
        self._connected = False
        self._loop: asyncio.AbstractEventLoop | None = None
        self._client = None
        self._pid = 0
        self._edit_node = None  # chatEditContainer 節點快取（input_open 用）
        self._msg_log = message_log  # messages.log（None＝不記錄，測試預設不落檔）

    @property
    def anchored(self) -> bool:
        """是否已連上並掛入遊戲（未連上時上層顯示『定位中』）。"""
        return self._connected

    def read_new(self) -> list[ChatLine]:
        """回傳自上次呼叫後新增的聊天行（依序、含重複）；無新訊息回傳 []。
        玩家發言恆回傳；系統訊息由 emit_system 決定（見 _diff_system_lines），
        兩者依遊戲內的原順序交錯。找不到遊戲或連線中斷丟 GameNotRunning。"""
        outcome = self._diff_new_lines()
        if self._msg_log is not None:
            self._msg_log.decision(outcome.path, outcome.appended,
                                   [line.text for line in outcome.emitted])
        return outcome.emitted

    def _diff_new_lines(self) -> _Outcome:
        """差分出本輪新增的聊天行，連同判定路徑（read_new 記進 messages.log）。

        玩家行與系統行走各自獨立的差分軌（後者見 _diff_system_lines），最後依原索引
        合併還原遊戲內順序。回傳的 path 是**玩家軌**的判定路徑。"""
        texts, cur_all = self._read_snapshot()
        # 兩軌分離但索引同源：最後依原索引合併，遊戲內的交錯順序即完整還原
        player_idx = [i for i, line in enumerate(cur_all) if not line.system]
        system_idx = [i for i, line in enumerate(cur_all) if line.system]
        cur_system_texts = [cur_all[i].text for i in system_idx]
        cur = [cur_all[i] for i in player_idx]
        # 差分只看文字：切頻道時 chatLog 會把同樣的訊息以該頻道顏色重新染色，
        # 顏色參與相等比較會被誤判成「無重疊 → reset」而重吐整份舊訊息（重複翻譯）
        cur_texts = [line.text for line in cur]
        if not self._synced:
            # 首次連上：記錄現況（含既有歷史），不回吐
            self._rebaseline_all(cur_texts, cur_system_texts)
            self._node_count = len(texts)
            self._synced = True
            log(f"[reader] baseline established (lines={len(cur)}, "
                f"nodes={len(texts)})")
            return _Outcome("baseline", [])
        # 暖機以輪數計且含空讀：重開遊戲後聊天常長時間空白，只數非空讀會讓暖機永不過期，
        # 各頻道從空白冒出的第一句（走 reset）被無限吸收
        if self._player.warmup_left > 0:
            self._player.warmup_left -= 1
        if not cur:
            # 空讀（傳送/轉場暫態清空）：玩家軌保留基準、忽略。系統軌不能跟著吸收 ——
            # 「這一輪沒有玩家行、只有掉寶」是日常狀態，在此吞掉等於系統訊息永遠吐不出來。
            self._empty_streak += 1
            # 節點數變化要等有玩家行的那一輪才比對 _node_count，這裡 node_added 只能是 False
            system_out = self._diff_system_lines(cur_system_texts, system_idx,
                                                 cur_all, texts, node_added=False)
            return _Outcome("empty", [cur_all[i] for i in system_out])
        baseline_stale = self._empty_streak >= STALE_BASELINE_EMPTY_POLLS
        self._empty_streak = 0
        if baseline_stale:
            log(f"[reader] baseline stale after a long empty stretch, "
                f"handling as reset (lines={len(cur)})")
        node_change = self._track_node_count(texts)
        if node_change == "decrease":
            self._rebaseline_all(cur_texts, cur_system_texts)
            return _Outcome("node-decrease", [])
        node_added = node_change == "increase"
        prev_texts = self._player.prev
        prev_len = len(prev_texts)
        path, appended = self._align_player(prev_texts, cur_texts,
                                            node_added or baseline_stale, texts)
        if appended is None:
            self._rebaseline_all(cur_texts, cur_system_texts)
            return _Outcome("warmup", [])
        # 基準先換、看過集合最後才記：本輪剛出現的新行還不在集合裡，過濾才吐得出來
        self._player.prev = cur_texts
        emitted = self._filter_player(path, appended, cur, prev_len, baseline_stale,
                                      content_changed=cur_texts != prev_texts)
        self._player.seen.remember(cur_texts)
        player_out = self._guard_burst(emitted, path, prev_len, len(cur), texts)
        system_out = self._diff_system_lines(cur_system_texts, system_idx, cur_all,
                                             texts, node_added)
        # 依原索引合併：兩軌各自走了哪條路徑都不影響相對順序（索引同源）
        merged = sorted(player_out_with_idx(player_out, cur, player_idx) + system_out)
        return _Outcome(path, [cur_all[i] for i in merged], len(appended))

    def _read_snapshot(self) -> tuple[list[str], list[ChatLine]]:
        """讀一輪 chatLog：取各節點全文、取樣輸入框、落 messages.log、解析成行序列。
        回傳（各節點原文，解析後的完整行序列，含系統行）。"""
        texts = self._read_chatlog_texts()
        input_open_now = self._track_input_box()
        # 控件列舉順序不保證穩定：排序讓多節點的串接結果確定，差分才有意義
        ordered = sorted(texts)
        raw = "\n".join(ordered)
        if self._msg_log is not None:
            # 解析與過濾之前先落檔：messages.log 要的是未經加工的原文
            self._msg_log.snapshot(raw.split("\n") if raw else [],
                                   nodes=len(texts), sizes_fn=lambda: node_sizes(texts),
                                   input_open=input_open_now)
        cur_all, mirrored = lines_from_nodes(ordered)
        if mirrored != self._mirrored_nodes:
            # 只在鏡射節點數變動時印：組隊視窗開著時每輪都成立，逐輪印會洗版
            log(f"[reader] mirrored chatLog nodes {self._mirrored_nodes}->{mirrored} "
                f"(nodes={len(texts)}, sizes={node_sizes(texts)}, "
                f"merged_lines={len(cur_all)}); mirrored copies are not retranslated")
            self._mirrored_nodes = mirrored
        return texts, cur_all

    def _rebaseline_all(self, player_texts: list[str], system_texts: list[str]) -> None:
        """兩軌一起把本輪內容立為新基準（靜默吸收、不吐任何行的路徑用）。"""
        self._player.rebaseline(player_texts)
        self._system.rebaseline(system_texts)

    def _track_node_count(self, texts: list[str]) -> str | None:
        """比對 chatLog 節點數與上輪並記下新值：回傳 "decrease"／"increase"／None（無變化）。
        減少（關閉私訊視窗等）：內容只會消失不會新增，呼叫端靜默重建基準。
        增加（開私訊視窗／聊天 UI 生成）：新節點可能正載著使用者的第一句，不可盲目吸收
        （實測私訊第一句被吞）；串接結構已變、對齊無意義，呼叫端直接走 reset 語意。"""
        if len(texts) == self._node_count:
            return None
        if len(texts) < self._node_count:
            change, action = "decrease", "re-baselining without emitting"
        else:
            change, action = "increase", "handling as reset"
        log(f"[reader] chatLog node count {change}d "
            f"({self._node_count}->{len(texts)}, sizes={node_sizes(texts)}), {action}")
        self._node_count = len(texts)
        return change

    def _align_player(self, prev_texts: list[str], cur_texts: list[str],
                      force_reset: bool, texts: list[str]) -> tuple[str, list[str] | None]:
        """玩家軌的對齊階梯（見 diff.align）。回傳（路徑，新增行）；新增行為 None＝暖機期內
        的零重疊讀取，呼叫端應靜默吸收（重建基準、不吐行）。"""
        path, appended = align(prev_texts, cur_texts, force_reset)
        if path == "recover":
            log(f"[reader] baseline misaligned, recovered via tail anchor "
                f"(prev={len(prev_texts)}, cur={len(cur_texts)}, emitted={len(appended)}, "
                f"nodes={len(texts)}, sizes={node_sizes(texts)})")
        if appended is not None:
            return path, appended
        # 與基準完全無重疊：首次切到沒讀過的分頁視圖、relog 成全新內容，或單行置換式
        # 視圖（朋友視窗每句新話取代整個內容）的新訊息。暖機期內一律靜默吸收（堵啟動
        # 盲區），之後交由看過集合過濾；基準為空（連上時聊天是空的）不受暖機限制。
        if prev_texts and self._player.warmup_left > 0:
            log(f"[reader] no overlap with baseline during warmup, absorbed "
                f"(lines={len(cur_texts)}, cur_head={cur_texts[0][:40]!r}, "
                f"prev_tail={prev_texts[-1][:40]!r})")
            return path, None
        log(f"[reader] chat log has no overlap with baseline, treating as reset "
            f"(lines={len(cur_texts)}, cur_head={cur_texts[0][:40]!r}, "
            f"prev_tail={prev_texts[-1][:40] if prev_texts else ''!r})")
        return path, cur_texts

    def _filter_player(self, path: str, appended: list[str], cur: list[ChatLine],
                       prev_len: int, baseline_stale: bool,
                       content_changed: bool) -> list[ChatLine]:
        """把對齊結果切回 ChatLine（帶出當前顏色）並過濾重浮歷史：append 快路徑走巧合對齊
        防線（_filter_append），慢路徑（視圖切換/異常讀取）剔除看過集合已有的行。
        例外：空讀轉場後只冒出一行且內容與清空前不同＝剛到的新訊息，不過濾 —— 照過濾會
        吞掉與舊訊息同字的新訊息（實機回報：轉場後第一句私訊 Test 因基準裡有人講過同一句
        而被吞）。內容一字不差填回則不適用：視圖只有一行玩家訊息時，轉場清空再還原長得
        就像單行新訊息，例外會讓它每次轉場都重譯（實機回報）；使用者自己重打的同字句仍由
        輸入框關聯放行。必須在把本輪內容記進看過集合之前呼叫（見 _drop_resurfaced）。"""
        # 對齊各路徑回傳的都是 cur 的尾段：以長度切回 ChatLine
        emitted = cur[len(cur) - len(appended):]
        if path == "append":
            return self._filter_append(appended, emitted, prev_len)
        if not emitted:
            return emitted
        if baseline_stale and len(emitted) == 1 and content_changed:
            log(f"[reader] single line after an empty stretch kept as new "
                f"(text={emitted[0].text[:40]!r}, path={path})")
            return emitted
        return self._drop_resurfaced(emitted, path)

    def _track_input_box(self) -> bool:
        """每輪取樣遊戲輸入框，推進關聯放行的輪數計數（見 INPUT_RELEASE_POLLS）；
        回傳輸入框目前是否開著。"""
        input_open_now = self.input_open()
        if input_open_now:
            # 輸入框開著＝還在打字，不可能有「剛送出」的訊息；此時放行會把切頁籤浮出的
            # 舊訊息當成新訊息（實機：開輸入框後 60ms 就放行了一次）。額度也還原，關掉後再算。
            self._input_recent = 0
            self._released_for_input = False
        elif self._input_was_open:
            # 輸入框剛關閉才是「剛送出」的訊號：實機的送出輪 input_open=False，
            # 訊息在輸入框關掉之後才出現在 chatLog
            self._input_recent = INPUT_RELEASE_POLLS
        elif self._input_recent > 0:
            self._input_recent -= 1
        self._input_was_open = input_open_now
        return input_open_now

    def _filter_append(self, appended: list[str], emitted: list[ChatLine],
                       prev_len: int) -> list[ChatLine]:
        """append 快路徑的巧合對齊防線：視圖 A 的內容恰為視圖 B 的前綴時，A→B 的切換會被
        append 誤判成「新增了 B 的其餘舊行」且不經任何過濾（實機每次切分頁重翻的主因）。
        單行 append（正常訊息與重複的 lol/gg）永不過濾。"""
        if len(appended) >= 2 and all(t in self._player.seen for t in appended):
            log(f"[reader] append of {len(appended)} all-seen lines absorbed "
                f"as view resurface (prev={prev_len})")
            return []
        if len(appended) >= 3 and len(appended) > 2 * prev_len:
            # 一輪暴增超過基準兩倍＝不可能的人為速度，判定為切到內容較多的視圖（代價：
            # 基準 1-2 行時 1 秒內連發 3 句會被吸收，下句恢復）。整批丟棄前先看有多少是
            # 看過的：chatLog 會短暫膨脹成重複版本（實機 102 行 ×8 ≈ 822 行），
            # 此時抵達的新訊息夾在裡面會一起被丟掉。
            kept = self._drop_resurfaced(emitted, "append")
            if len(kept) > len(appended) // DUPLICATE_BURST_SEEN_RATIO:
                log(f"[reader] implausible append burst absorbed as view switch "
                    f"(appended={len(appended)}, unseen={len(kept)}, "
                    f"prev={prev_len})")
                return []
            return kept
        if len(appended) >= BULK_APPEND_FILTER_MIN:
            return self._drop_resurfaced(emitted, "append")
        return emitted

    def _drop_resurfaced(self, emitted: list[ChatLine], path: str) -> list[ChatLine]:
        """剔除看過集合裡已有的行（重浮歷史），沒見過的行保留。
        必須在把本輪內容記進看過集合之前呼叫 —— 本輪剛出現的新行還不在集合裡，才吐得出來。"""
        kept = filter_resurfaced(emitted, self._player.seen)
        if (len(kept) != len(emitted) and self._input_recent > 0
                and not self._released_for_input and emitted[-1].own
                and (not kept or kept[-1] is not emitted[-1])):
            # 輸入框剛關閉＝使用者剛送出：尾行與舊訊息同字（重打同一句）會被誤判重浮，關聯
            # 放行尾行。限自己講的那行 —— 轉場期間輸入框狀態會亂跳，只看輸入框活動會把別人的
            # 舊訊息當「剛送出」放行而重翻（實機回報）。每次開啟只放行一次：此機制分不出
            # 「重打同一句」與「切頁籤讓同一句重浮」，不設限後者每切回來就重複顯示。
            log(f"[reader] released tail line suppressed as resurfaced: input "
                f"closed recently, treating as a just-sent message via {path}")
            self._released_for_input = True
            kept = kept + [emitted[-1]]
        if len(kept) != len(emitted):
            log(f"[reader] suppressed {len(emitted) - len(kept)} resurfaced "
                f"lines via {path} (kept={len(kept)}, "
                f"seen={len(self._player.seen)})")
        return kept

    def _diff_system_lines(self, cur_texts: list[str], system_idx: list[int],
                           cur_all: list[ChatLine], texts: list[str],
                           node_added: bool = False) -> list[int]:
        """系統訊息的差分軌，回傳新增行在完整序列中的索引。

        路徑決策不與玩家軌共用：「這一輪沒有系統訊息」是日常狀態，照搬
        STALE_BASELINE_EMPTY_POLLS 會讓基準不斷過期、把系統軌長期推去走 reset 而拉高重吐。
        `node_added` 由 _diff_new_lines() 統一判定：節點增加代表串接結構已變，對齊會生出假的
        append，而 append 路徑不過看過集合、剛掉過的寶會再吐一次，故比照玩家軌直接走 reset。
        `emit_system` 為 False 時仍照常推進基準與看過集合，否則中途打開開關的瞬間，
        整份歷史系統訊息會被當成新訊息一次吐出、翻上百則。"""
        if not cur_texts:
            return []            # 沒有系統訊息：保留基準，不累計 stale
        track = self._system
        if track.warmup_left > 0:
            track.warmup_left -= 1
        prev_len = len(track.prev)
        path, appended = align(track.prev, cur_texts, node_added)
        if appended is None:
            if track.prev and track.warmup_left > 0:
                log(f"[reader] system track absorbed during warmup "
                    f"(lines={len(cur_texts)})")
                track.rebaseline(cur_texts)
                return []
            reason = ("chatLog node count increased" if node_added
                      else "no overlap with baseline")
            log(f"[reader] system track handled as reset ({reason}): "
                f"prev={prev_len}, cur={len(cur_texts)}")
            appended = cur_texts
        track.prev = cur_texts
        emitted_texts = appended
        if path != "append" or len(appended) >= BULK_APPEND_FILTER_MIN:
            # 與玩家軌同策略：正常新增一律放行，只在慢路徑與大批次過濾重浮歷史
            kept = filter_resurfaced(emitted_texts, track.seen, text=lambda line: line)
            if len(kept) != len(emitted_texts):
                log(f"[reader] system track dropped {len(emitted_texts) - len(kept)} "
                    f"resurfaced lines via {path}")
            emitted_texts = kept
        track.seen.remember(cur_texts)
        if len(emitted_texts) > MAX_NEW_LINES_PER_POLL:
            log(f"[reader] implausible system burst suppressed via {path}: "
                f"{len(emitted_texts)} new lines in one poll (prev={prev_len}, "
                f"cur={len(cur_texts)}, nodes={len(texts)}); re-baselined without "
                f"emitting")
            return []
        if len(emitted_texts) > SYSTEM_LARGE_BATCH_LOG_THRESHOLD:
            log(f"[reader] large system batch via {path}: {len(emitted_texts)} lines "
                f"(prev={prev_len}, cur={len(cur_texts)}, nodes={len(texts)})")
        if not self.emit_system:
            return []            # 關閉中：基準已推進，只是不輸出
        # 換算回完整序列的索引：各路徑回傳的都是 cur_texts 的尾段
        tail = system_idx[len(cur_texts) - len(appended):]
        pending = list(emitted_texts)
        out = []
        for idx in tail:
            text = cur_all[idx].text
            if text in pending:
                pending.remove(text)       # 逐一消耗，重複行只對應一個索引
                out.append(idx)
        return out

    def _guard_burst(self, appended: list[ChatLine], path: str, prev_len: int,
                     cur_len: int, texts: list[str]) -> list[ChatLine]:
        """所有差分路徑的共同出口：擋下不可能為真的暴量新增（見 MAX_NEW_LINES_PER_POLL），
        並為接近上限的批次留下診斷數據。基準已在呼叫端更新，擋下即等同靜默重建基準。"""
        if len(appended) > MAX_NEW_LINES_PER_POLL:
            log(f"[reader] implausible burst suppressed via {path}: {len(appended)} new "
                f"lines in one poll (prev={prev_len}, cur={cur_len}, "
                f"nodes={len(texts)}, sizes={node_sizes(texts)}); "
                f"re-baselined without emitting")
            return []
        if len(appended) > LARGE_BATCH_LOG_THRESHOLD:
            log(f"[reader] large batch via {path}: {len(appended)} lines "
                f"(prev={prev_len}, cur={cur_len}, nodes={len(texts)}, "
                f"sizes={node_sizes(texts)})")
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
            log(f"[reader] game chat input rect unavailable: {type(exc).__name__}: {exc}")
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

    def _read_chatlog_texts(self) -> list[str]:
        """讀所有 `chatLog` 控件的全文（每節點一個字串）；連線中斷則丟 GameNotRunning。"""
        if not self._connected:
            self._connect()
        try:
            return self._grab_texts()
        except Exception as exc:  # 遊戲關閉/文件釋放/記憶體讀取失敗 → 視為斷線，由上層重連
            self._teardown()
            raise GameNotRunning(f"chatlog read failed (likely left the game): {exc}") from exc

    # --- 與 wizwalker 的 I/O 接縫（測試中覆寫 _grab_texts）---
    def _grab_texts(self) -> list[str]:
        async def _grab() -> list[str]:
            nodes = await self._client.root_window.get_windows_with_name("chatLog")
            return [await n.maybe_text() for n in nodes]

        return self._run(_grab())

    def _connect(self) -> None:
        import wizwalker
        import wizwalker.utils
        from pymem.exception import CouldNotOpenProcess

        path = self._game_path or detect_install_path()
        log(f"[reader] game path: {path!r} "
            f"(source={'config' if self._game_path else 'detected'}, slot={self._slot})")
        if path:
            wizwalker.utils._OVERRIDE_PATH = path  # Steam 版無登錄檔安裝路徑，需覆寫

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
        hook_state.sweep(pid_alive)          # 清掉已不在執行的程序的殘留狀態檔
        self._repair_leaked_hooks(self._pid)  # 修復上次髒退出遺留的 hook（免重開遊戲）
        try:
            # 只啟讀聊天所需的 root_window hook（不啟 player/duel/quest 等）：注入最小化、
            # 不受是否在世界內影響。掛入與等待就緒拆成兩步：wizwalker 內建的等待無限期
            # （見 HOOK_READY_TIMEOUT），且 hook 此刻已寫進遊戲記憶體，要先把還原狀態存起來，
            # 等待途中被硬砍才修得回來。
            self._run(self._client.hook_handler.activate_root_window_hook(
                wait_for_ready=False))
            self._save_hook_state(self._pid)
            waited = self._wait_root_window_ready()
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
                        f"(len={len(original)}): {type(exc).__name__}: {exc}")
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
            log(f"[reader] unhook failed, leaving repair state for next launch: {exc}")
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
        # 差分狀態屬於單一遊戲 session，斷線後全部歸零：沿用舊 session 的基準/看過集合，
        # 會把新 session 與舊訊息同字的第一句（Test/lol 等）誤判為重浮歷史而吞掉
        self._player.reset()
        self._system.reset()
        self._node_count = None
        self._mirrored_nodes = 0
        self._synced = False
        forget_warned_icons()

    def close(self) -> None:
        """停止時呼叫：解除 hook、關閉連線。"""
        self._teardown()
