"""chatLog 兩輪讀取之間的差分：對齊演算法（append／recover）、看過集合、每條差分軌的
狀態，以及慢路徑用的重浮歷史過濾。全部是純函式與純資料結構，不需遊戲即可測試；
判定階梯怎麼串、各門檻怎麼用見 mem_reader.WizChatReader。
"""
from collections import deque
from collections.abc import Container

from src.log import log
from src.reader.markup import ChatLine

# 基準建立後的暖機輪數，期間 reset（零重疊讀取）一律靜默吸收：啟動時基準只蓋到當前分頁，
# 其他分頁的歷史在頭幾輪浮上來會被誤當新訊息（實測都在前 1-3 輪，5 輪已保守）。
# 拉太長會放大代價 —— 期間切到別的頻道說的第一句（走 reset）會被吸收。
RESET_WARMUP_POLLS = 5
# 「看過集合」容量上限（行數，FIFO）。聊天分頁共用同一個 chatLog 控件且無分頁狀態可讀，
# 切分頁＝內容換成另一視圖，recover/reset 會把重浮的歷史誤判成新訊息，故靠它過濾。
# 不能改存「最近幾份基準」：append 每輪換基準，停留同一視圖幾輪就把其他視圖的證據
# 擠掉（實測破功）。
SEEN_LINES_CAP = 10000


def _find_last_run(cur_lines: list[str], seq: list[str]) -> int | None:
    """seq 以連續片段出現在 cur_lines 中的**最後**位置；找不到回傳 None。"""
    n = len(seq)
    for i in range(len(cur_lines) - n, -1, -1):
        if cur_lines[i:i + n] == seq:
            return i
    return None


def align_append(prev_lines: list[str], cur_lines: list[str]) -> list[str] | None:
    """附加/捲動對齊（快路徑）：找最短的「prev 去掉前 k 行」正好是 cur 的前綴，回傳 cur
    尾端多出來的行（含重複、依序）；對不齊回傳 None，呼叫端改走 align_recover。
    平時純附加 k=0 必中；達顯示上限修剪頭部時 k>0 吸收捲動。必須嚴格要求前綴，
    否則「尾行與新行重複」（[hi]→[hi,hi]）會被誤判為無新增而漏訊。"""
    if not prev_lines:
        return None
    for k in range(len(prev_lines)):
        overlap = prev_lines[k:]
        if cur_lines[:len(overlap)] == overlap:
            return cur_lines[len(overlap):]
    return None


def align_recover(prev_lines: list[str], cur_lines: list[str]) -> list[str] | None:
    """恢復對齊（align_append 的退路）：找最長的 prev 尾段 prev[k:]，其以連續片段出現在
    cur 的**最後**位置，回傳其後的行；完全無重疊回傳 None（呼叫端視為聊天重置）。
    走到這裡代表撕裂讀取（遊戲寫入中讀到缺行/壞行）或多 chatLog 串接結構變化。
    取「最後」位置是關鍵：聊天充滿重複行（lol/gg），錨到較早的重複行會把其後整段舊訊息
    重吐（翻譯洪水＋timeout 螺旋）；錨到最後頂多漏掉少數重複的新行。"""
    if not prev_lines:
        return None
    for k in range(len(prev_lines)):
        overlap = prev_lines[k:]
        idx = _find_last_run(cur_lines, overlap)
        if idx is not None:
            return cur_lines[idx + len(overlap):]
    return None


class SeenLines:
    """近期讀過的行文字集合（各視圖聯集），超過容量從最舊的開始淘汰（FIFO）。
    容量上限的取捨見 SEEN_LINES_CAP。"""

    def __init__(self, cap: int = SEEN_LINES_CAP):
        self._cap = cap
        self._set: set[str] = set()
        self._order: deque[str] = deque()

    def remember(self, texts: list[str]) -> None:
        for text in texts:
            if text not in self._set:
                self._set.add(text)
                self._order.append(text)
        while len(self._order) > self._cap:
            self._set.discard(self._order.popleft())

    def clear(self) -> None:
        self._set.clear()
        self._order.clear()

    def __contains__(self, text: object) -> bool:
        return text in self._set

    def __len__(self) -> int:
        return len(self._set)


class Track:
    """一條差分軌的狀態：上輪基準（只存文字，顏色不參與差分）、看過集合、剩餘暖機輪數。
    玩家軌與系統軌各持一份、完全獨立（見 WizChatReader.__init__）。"""

    def __init__(self):
        self.prev: list[str] = []
        self.seen = SeenLines()
        self.warmup_left = RESET_WARMUP_POLLS

    def rebaseline(self, texts: list[str]) -> None:
        """把本輪內容立為新基準並記進看過集合（靜默吸收、不吐任何行的路徑用）。"""
        self.prev = texts
        self.seen.remember(texts)

    def reset(self) -> None:
        """回到剛連上時的狀態（斷線後呼叫）。"""
        self.prev = []
        self.seen.clear()
        self.warmup_left = RESET_WARMUP_POLLS


def align(prev: list[str], cur: list[str],
           force_reset: bool) -> tuple[str, list[str] | None]:
    """差分的判定階梯：append（快路徑）→ recover（退路）→ reset（完全無重疊）。
    回傳（路徑名，新增行）；reset 時新增行為 None，由呼叫端決定吸收或整批放行。
    force_reset＝串接結構已變或基準已過期，對齊沒有意義，直接跳到 reset。"""
    if not force_reset:
        appended = align_append(prev, cur)
        if appended is not None:
            return "append", appended
        appended = align_recover(prev, cur)
        if appended is not None:
            return "recover", appended
    return "reset", None


def filter_resurfaced(emitted: list, seen: Container[str],
                      text=lambda line: line.text) -> list:
    """剔除看過集合已有的行（＝視圖切換時重新浮上來的歷史），保留真正的新行。
    只用在 recover/reset 慢路徑：代價是恰在視圖切換那一輪出現的「與近期舊訊息
    一字不差的重複句」會被略過，與 align_recover 既有的取捨一致。
    `text` 取出每個元素用來比對的字串：玩家軌傳 ChatLine（預設），系統軌傳純文字。"""
    return [item for item in emitted if text(item) not in seen]


def player_out_with_idx(emitted: list[ChatLine], cur_player: list[ChatLine],
                        player_idx: list[int]) -> list[int]:
    """把玩家軌吐出的行換算回它們在完整序列（含系統行）中的索引。

    emitted 是 cur_player 的**子序列**：對齊路徑先切出尾段，慢路徑再逐行剔除重浮歷史
    （見 _drop_resurfaced），中間可能被挖空，故不能只取同長度的尾段索引。
    以物件識別（`is`）順向比對 —— 聊天充滿一字不差的重複行，比對文字會對到錯的索引。"""
    if not emitted:
        return []
    out: list[int] = []
    remaining = iter(emitted)
    want = next(remaining)
    for pos, line in enumerate(cur_player):
        if line is want:
            out.append(player_idx[pos])
            want = next(remaining, None)
            if want is None:
                break
    if want is not None:
        # emitted 已不是 cur_player 的子序列（例如某條路徑用 _replace 重建了 ChatLine，
        # 物件識別斷了）：沒對上的行會被靜默丟掉，正是「玩家訊息被吞」那一類，留 log 才查得出來
        log(f"[reader] player index mapping incomplete: {len(emitted)} emitted, "
            f"{len(out)} mapped (subsequence invariant broken)")
    return out
