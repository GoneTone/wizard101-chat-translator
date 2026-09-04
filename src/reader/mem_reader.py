"""收訊端：透過 wizwalker 掛入遊戲、讀聊天顯示控件 `chatLog` 的全文，差分出新增行。

聊天在記憶體/顯示層是帶標記的富文字，每則一行、以 `\n` 分隔：
    他人： <color;..><image;Art/Art_Chat_Say.dds;..> <link;GID:<id>,<名>,2>[<名>]</link> 內文 </color>
    自己： <color;..><image;Art/Art_Chat_Say.dds;..> [你] 內文 </color>          ← 無 <link;GID>
玩家發言（他人與自己）都帶頻道圖示（Art_Chat_<頻道>，房間頻道為 chat_balloon_*）；
系統訊息用 Art_Chat_System、除錯行無圖示，以此過濾出玩家發言，
再去標記回傳乾淨的「[發送者] 內文」，
連同行首 <color;..> 的遊戲顯示色一起帶出（ChatLine，供 overlay 對齊遊戲配色）。

wizwalker 靠 root-window hook 定位 `chatLog` 控件（穩定、有序、含他人訊息），
取代舊的全記憶體掃描 + 活文件定錨。注意：wizwalker 為了 hook 會寫入遊戲程序記憶體
（注入），非純讀。
"""
import asyncio
import os
import re
import sys
import time
from collections import Counter, deque
from typing import NamedTuple

from src.reader import hook_state
from src.reader.message_log import MessageLog

PROCESS_NAME = "WizardGraphicalClient.exe"
INPUT_CONTAINER = "chatEditContainer"  # 遊戲聊天輸入區容器：開啟輸入時 is_visible 翻 True（實測）

# hook 掛上後，等遊戲把 root window 位址寫回來的上限（秒）與輪詢間隔。
# wizwalker 的 activate_root_window_hook(wait_for_ready=True) 內部是 asyncio.wait_for(..., None)
# ——**無限等**。pattern 掃得到、hook 也寫進去了，那段程式碼卻已不在執行路徑時
# （遊戲改版的典型徵兆），整條收訊執行緒會永久卡在掛入，不拋例外、UI 停在「連線遊戲中」，
# 關程式時 join 逾時被硬砍，hook 還留在遊戲記憶體裡。故一律自己等、自己逾時。
HOOK_READY_TIMEOUT = 15.0
HOOK_READY_POLL = 0.3

# 單輪 poll 的新增行數上限。實測 chatLog 會在「約 110 行的短清單」與「上千行的完整歷史」
# 之間反覆跳動；只要歷史開頭那行碰巧等於基準尾行（聊天充滿 lol/gg 等重複短行），
# align_append 就會把整段舊訊息當成新訊息回吐——而且它是唯一不印 log 的路徑，難以察覺。
# 一輪只隔 poll_interval 秒，真實聊天不可能新增這麼多，超過即判定為差分誤對齊。
# 這道防線套在所有路徑的共同出口，不只擋 align_append。
MAX_NEW_LINES_PER_POLL = 100
# 未達上限但異常大的批次：照吐，但留下診斷數據供事後判斷差分是否誤判
LARGE_BATCH_LOG_THRESHOLD = 10
# 系統軌的診斷 log 門檻。掉寶一輪十幾行是常態，套玩家軌的 LARGE_BATCH_LOG_THRESHOLD
# 會讓 app.log 每輪都印一行 large batch 而被洗掉。暴量防線（MAX_NEW_LINES_PER_POLL）
# 兩軌沿用同值——一輪真的超過 100 行系統訊息就是差分誤對齊，正是它該擋的。
SYSTEM_LARGE_BATCH_LOG_THRESHOLD = 40
# append 快路徑改走逐行過濾的批次大小門檻。一輪 poll 內湧出這麼多行不可能是真人發言，
# 而是轉場時 chatLog 把整份歷史重接一次（實測切伺服器：101 行→200 行→吐出 100 行舊訊息）。
# 下方「全部看過」那道判定是全有全無，批次裡混進一行沒讀過的就整批放行，擋不住這種
# 混合批次；達此門檻的批次改逐行剔除看過的行，沒見過的行照吐、不會吞掉真正的新訊息。
BULK_APPEND_FILTER_MIN = 10
# 暴增批次判定為「內容重浮」的門檻：剔除看過的行之後，沒讀過的行不到全批的 1/N。
# 切到沒讀過的視圖時整批都是生面孔（實測前綴巧合：1 行基準冒出 4 行全新），
# chatLog 膨脹成重複版本時則幾乎全是看過的行、只夾著剛抵達的一兩句新訊息。
DUPLICATE_BURST_SEEN_RATIO = 4
# 基準建立後的暖機輪數：期間 reset（零重疊讀取）一律靜默吸收。啟動時基準與看過集合
# 只蓋到當前分頁，其他分頁的歷史在頭幾輪浮上來會被誤當新訊息（實測都發生在前 1-3 輪，
# 視圖每輪輪播、看過集合幾輪內學完，5 輪已保守）；暖機拉太長會放大代價——期間切到
# 別的頻道說的第一句（走 reset）會被吸收。暖機期過後 reset 恢復以看過集合過濾。
RESET_WARMUP_POLLS = 5
# 「看過集合」容量上限（行數，FIFO 淘汰最舊）。聊天分頁共用同一個 chatLog 控件
# （實測無分頁狀態可讀），切分頁＝內容換成另一視圖，recover/reset 會把重新浮上來的
# 歷史誤判成新訊息；每輪把讀到的行記進集合，慢路徑吐出前剔除集合裡已有的行。
# 不能改存「最近幾份基準」：append 快路徑每輪都在換基準，停留同一視圖幾輪
# 就會把其他視圖的證據擠掉（實測破功），集合只受總量上限影響。
SEEN_LINES_CAP = 10000
# 輸入框關聯放行的有效輪數：使用者剛送出訊息時，遊戲輸入框必在前 1-2 輪內開啟過。
# 登出再登入不會重啟程序、看過集合殘留舊 session 字樣，重打同一句（Test/lol 等）
# 走 reset 會被誤判重浮吞掉；輸入框剛關閉＋視圖尾行同字＝剛送出的訊息，放行。
INPUT_RELEASE_POLLS = 2
# 連續空讀達此輪數即視舊基準過期：恢復內容後不得再拿它做 append/recover 對齊——
# 置換式視圖（朋友/私訊視窗只顯示最近一則）重打同一句、或登出前尾行與登入後第一句
# 同字時，append 會誤判「無變化」把新句靜默吞掉（實測且無 log 可循）。
# 過期後強制走 reset 語意，由看過集合（歷史灌回不重翻）與輸入框關聯放行
# （剛送出的同字句照翻）接手。門檻取小值涵蓋「兩則訊息之間的短暫清空」：
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
    """開不了遊戲程序的 handle——多半是遊戲以系統管理員身分執行、本程式沒有。
    繼承 GameNotRunning，上層的退避重連原封不動生效，只在文案上分流。"""


class GameVersionMismatch(GameNotRunning):
    """掛入點與這個遊戲版本對不上，wizwalker 的 pattern 已不適用。
    繼承 GameNotRunning，上層的退避重連原封不動生效，只在文案上分流。"""


def is_version_mismatch(exc: BaseException) -> bool:
    """判斷掛入失敗是否為「掛入點與遊戲版本對不上」。

    三種徵兆：pattern 掃不到、pattern 掃到多個（兩者都在 wizwalker 寫入任何遊戲
    記憶體之前拋出，遊戲是乾淨的），以及 hook 寫進去了卻遲遲沒被觸發（等待逾時）。
    其餘失敗（開不了程序、程序消失、讀寫錯誤）不屬此類，仍走「遊戲未就緒」文案。"""
    from wizwalker.errors import PatternFailed, PatternMultipleResults
    return isinstance(exc, (PatternFailed, PatternMultipleResults, TimeoutError))


# --- 純函式：標記解析（可單元測試，不需遊戲）---
class ChatLine(NamedTuple):
    """一行乾淨的聊天，帶遊戲顯示色（行內 <color;..>，overlay 用它對齊遊戲配色）。
    own＝這句是自己講的（見 _OTHER_PLAYER_LINK）；system＝遊戲系統訊息
    （掉寶／經驗／升等廣播等，見 _SYSTEM_IMG；伺服器公告見 _is_server_broadcast），
    走與玩家對話分離的差分軌與翻譯路徑。"""
    text: str
    color: str | None
    own: bool = False
    system: bool = False


_TAG = re.compile(r"<[^>]*>")
# 顏色標記的值為 6 位 RRGGBB 或 8 位 AARRGGBB（帶 alpha），顯示色一律取後 6 位
_COLOR_TAG = re.compile(r"<color;([0-9a-fA-F]{6,8})>")
_VALID = re.compile(r"^\[[^\]]{1,40}\] .+")
# 聊天頻道圖示：玩家發言（他人與自己）行都帶頻道圖示——多數頻道是 Art_Chat_<頻道>，
# 房間頻道實測是 chat_balloon_<Owner/Guest>，快捷訊息（禁言帳號只能用選單發話）是
# Art_Word_Balloon；系統訊息用 Art_Chat_System。
# 自己的發言是 [你] 開頭、無 <link;GID>，故不能只靠 link 過濾。
_PLAYER_IMG_PREFIXES = ("<image;Art/Art_Chat", "<image;Art/chat_balloon",
                        "<image;Art/Art_Word_Balloon")
_SYSTEM_IMG = "<image;Art/Art_Chat_System"
# 伺服器公告（維修預告等）實測連圖示都沒有，只有行首 <color;..>：
# 以「有沒有 Art/ 圖示」與「有沒有行首顏色」兩者一起認（見 _is_server_broadcast）。
_ART_IMG = "<image;Art/"
_LEADING_COLOR = re.compile(r"^\s*<color;")
# 他人發言的名字是可點擊的玩家連結；自己的發言只有純文字 [你]（各語系用語不同，
# 故以「有沒有這個連結」判斷是不是自己講的，不比對名稱字串）
_OTHER_PLAYER_LINK = "<link;GID"
# 任意 Art/ 圖示（診斷用）：長得像聊天行但圖示不在白名單 → 可能是漏接的頻道
_ANY_ART_IMG = re.compile(r"<image;(Art/[^.;>]+)\.dds", re.IGNORECASE)
_warned_icons: set[str] = set()  # 每種未知圖示每次執行只警告一次，避免洗版

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
    """把 chatLog 控件全文（以 `\n` 分行的渲染 markup）解析成乾淨聊天行
    （ChatLine：文字＋遊戲顯示色＋system 旗標），保留順序與重複。

    收玩家發言（含**自己**的 `[你]` 行與他人 `<link;GID>[名]` 行，圖示前綴見
    _PLAYER_IMG_PREFIXES）、系統訊息（Art_Chat_System）與伺服器公告
    （無圖示，見 _is_server_broadcast）；濾掉遊戲除錯行
    （[STAT]/[DBGL]/[DBGM]，既無頻道圖示也無行首顏色）。

    放行判準不同：玩家行必須是「[發送者] 內容」（_VALID），系統訊息與伺服器公告沒有
    固定的發送者前綴，只要 clean() 後非空即收。"""
    out: list[ChatLine] = []
    for raw in text.split("\n"):
        if _SYSTEM_IMG in raw or _is_server_broadcast(raw):
            line = clean(raw)
            if line:
                out.append(ChatLine(line, line_color(raw), False, True))
            continue
        if not any(p in raw for p in _PLAYER_IMG_PREFIXES):
            _warn_unknown_icon(raw)
            continue
        line = clean(raw)
        if _VALID.match(line):
            out.append(ChatLine(line, line_color(raw),
                                _OTHER_PLAYER_LINK not in raw))
    return out


def _is_server_broadcast(raw: str) -> bool:
    """這行是伺服器公告嗎——有行首顏色、卻沒有任何 Art/ 頻道圖示。

    實機樣本只有 `<color;D9ABF8>[Server Message] 內文</color>`，發送者前綴不保證存在，
    故不比對字串。遊戲自己的除錯輸出（`RECEIVED STATUS UPDATE for [id]` 之類）同樣
    沒有圖示，但不以 `<color;..>` 起頭，靠這點分辨。
    先排除帶 Art/ 圖示的行，未收錄的頻道才會照舊落到 _warn_unknown_icon，
    不會被這條規則悄悄吞成系統訊息。"""
    return _ART_IMG not in raw and _LEADING_COLOR.match(raw) is not None


def _warn_unknown_icon(raw: str) -> None:
    """帶 Art/ 圖示、格式像聊天行、但圖示不在白名單：每種圖示警告一次。
    漏接頻道（如尚未取樣的組隊頻道）能直接從 app.log 讀到圖示名稱，免再探測。"""
    m = _ANY_ART_IMG.search(raw)
    if not m or m.group(1) in _warned_icons:
        return
    if not _VALID.match(clean(raw)):
        return  # 格式不像聊天行（系統/除錯雜訊）：不值得警告
    _warned_icons.add(m.group(1))
    print(f"[reader] unrecognized chat icon {m.group(1)!r}, line dropped "
          f"(add prefix to _PLAYER_IMG_PREFIXES if this is a player channel); "
          f"raw={raw[:160]!r}", file=sys.stderr)


def _mirrors(part: list[ChatLine], main: list[ChatLine]) -> bool:
    """part 的每一行（含重複次數）都能在 main 裡找到 → part 只是 main 的鏡射。"""
    remaining = Counter(l.text for l in main)
    for line in part:
        if not remaining[line.text]:
            return False
        remaining[line.text] -= 1
    return True


def lines_from_nodes(texts: list[str]) -> tuple[list[ChatLine], int]:
    """把各 chatLog 節點的全文合併成單一聊天行序列（依傳入順序串接），
    回傳（行序列, 被剔除的鏡射節點數）。

    組隊等浮動聊天視窗各自是一個 chatLog 節點，且會把同一則訊息再渲染一份
    （實測：開組隊視窗後發話，同一句同時出現在兩個節點，sizes=[1, 1, 0]）。
    盲目串接會讓一則訊息變成兩行——當下就翻兩次，之後主視圖在完整歷史與精簡視圖
    之間來回跳時，多出來的那份還會被 align_append 當成尾端新增而每次重翻。
    以玩家行數最多的節點為主視圖，玩家行被它完全涵蓋（含重複次數）的節點即判定為鏡射。
    代價：兩個視窗剛好各自出現一句一字不差的訊息時只翻一次，與差分既有的取捨一致。
    空節點不算鏡射——它本來就不貢獻任何行，計進去會讓診斷 log 每輪都在響。

    **鏡射判定只看玩家行**：系統訊息在不同節點的出現方式未經實測，讓它參與判定會改變
    這條調校過的規則。判定完成後才從保留的節點取出系統行。
    """
    parts = [lines_from_chatlog(t) for t in texts]
    if len(parts) < 2:
        return [l for p in parts for l in p], 0
    players = [[l for l in p if not l.system] for p in parts]
    main = max(range(len(parts)), key=lambda i: len(players[i]))
    kept = [p for i, p in enumerate(parts)
            if i == main or not (players[i] and _mirrors(players[i], players[main]))]
    return [l for p in kept for l in p], len(parts) - len(kept)


def node_sizes(texts: list[str]) -> list[int]:
    """各 chatLog 節點各自的聊天行數（含系統訊息），依串接時的排序。診斷用：看得出是哪個節點
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


def player_out_with_idx(emitted: list[ChatLine], cur_player: list[ChatLine],
                        player_idx: list[int]) -> list[int]:
    """把玩家軌吐出的行換算回它們在完整序列（含系統行）中的索引。

    emitted 是 cur_player 的**子序列**：對齊路徑先切出尾段，慢路徑再逐行剔除重浮歷史
    （見 _drop_resurfaced），中間可能被挖空，故不能只取同長度的尾段索引。
    以物件識別（`is`）順向比對——聊天充滿一字不差的重複行，比對文字會對到錯的索引。"""
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
        # 走到這裡代表 emitted 已不是 cur_player 的子序列（例如某條路徑改用 _replace
        # 重建了 ChatLine，物件識別就斷了）。沒對上的行會被靜默丟掉——正是本模組
        # 註解裡一再出現的「玩家訊息被吞」那一類，留 log 才查得出來。
        print(f"[reader] player index mapping incomplete: {len(emitted)} emitted, "
              f"{len(out)} mapped (subsequence invariant broken)", file=sys.stderr)
    return out


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
    與基準完全對不齊（首次切到沒讀過的分頁視圖／relog）則靜默吸收為新基準、
    不輸出（見 read_new 的 reset 分支說明）。
    所有路徑共用一道出口防線：單輪吐出超過 MAX_NEW_LINES_PER_POLL 行視為差分誤對齊，
    不吐並重建基準（見該常數的說明）。"""

    def __init__(self, game_path: str | None = None, process_name: str = PROCESS_NAME,
                 message_log: MessageLog | None = None):
        self.process_name = process_name
        self._game_path = game_path
        self._prev: list[str] = []  # 基準只存文字：顏色不參與差分（見 read_new）
        self._warmup_left = RESET_WARMUP_POLLS  # 剩餘暖機輪數（reset 吸收期）
        self._seen: set[str] = set()          # 近期讀過的行文字（各視圖聯集）
        self._seen_order: deque[str] = deque()  # 進入順序，供容量上限 FIFO 淘汰
        self.emit_system = False   # 是否輸出系統訊息（對應 config 的 translate_system_messages）
        # 系統軌狀態：與玩家軌完全分離。共用同一個 _seen 會讓掉寶刷屏把玩家說過的話
        # 擠出容量上限，視圖一切換那些玩家訊息就被當成沒見過而重吐重翻。
        self._prev_system: list[str] = []
        self._seen_system: set[str] = set()
        self._seen_system_order: deque[str] = deque()
        self._warmup_system_left = RESET_WARMUP_POLLS
        self._input_recent = 0     # 輸入框開啟後的剩餘關聯輪數（見 INPUT_RELEASE_POLLS）
        self._input_was_open = False
        self._released_for_input = False  # 本次輸入框開啟是否已用掉關聯放行額度
        self._empty_streak = 0     # 連續空讀輪數（見 STALE_BASELINE_EMPTY_POLLS）
        self._node_count: int | None = None  # 上輪讀到的 chatLog 節點數（變動＝串接結構改變）
        self._mirrored_nodes = 0   # 上輪剔除的鏡射節點數（見 lines_from_nodes）
        self._synced = False          # 是否已建立初始基準（建立後才開始回報新增）
        self._connected = False
        self._loop: asyncio.AbstractEventLoop | None = None
        self._handler = None
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
                                   [l.text for l in outcome.emitted])
        return outcome.emitted

    def _diff_new_lines(self) -> _Outcome:
        """差分出本輪新增的聊天行，連同判定路徑（read_new 記進 messages.log）。

        玩家行與系統行走各自獨立的差分軌（後者見 _diff_system_lines），最後依原索引
        合併還原遊戲內順序。回傳的 path 是**玩家軌**的判定路徑。"""
        texts = self._read_chatlog_texts()
        # 每輪記錄輸入框狀態，供 filter 關聯放行「剛送出、與舊訊息同字」的訊息
        input_open_now = self.input_open()
        if input_open_now:
            # 輸入框開著＝還在打字，這一刻不可能有「剛送出」的訊息。此時放行等於把
            # 切頁籤浮出來的舊訊息當成新訊息（實機：開輸入框後 60ms 就放行了一次）。
            # 順手把額度還原，使用者關掉輸入框後才重新計算。
            self._input_recent = 0
            self._released_for_input = False
        elif self._input_was_open:
            # 輸入框剛關閉才是「剛送出」的訊號：實機的送出輪 input_open=False，
            # 訊息是在輸入框關掉之後才出現在 chatLog 裡。
            self._input_recent = INPUT_RELEASE_POLLS
        elif self._input_recent > 0:
            self._input_recent -= 1
        self._input_was_open = input_open_now
        # 控件列舉順序不保證穩定：排序讓多節點的串接結果確定，差分才有意義
        ordered = sorted(texts)
        raw = "\n".join(ordered)
        if self._msg_log is not None:
            # 解析與過濾之前先落檔：messages.log 要的是未經加工的原文
            self._msg_log.snapshot(raw.split("\n") if raw else [],
                                   nodes=len(texts), sizes=node_sizes(texts),
                                   input_open=input_open_now)
        cur, mirrored = lines_from_nodes(ordered)
        if mirrored != self._mirrored_nodes:
            # 只在鏡射節點數變動時印：組隊視窗開著時每輪都成立，逐輪印會洗版
            print(f"[reader] mirrored chatLog nodes {self._mirrored_nodes}->{mirrored} "
                  f"(nodes={len(texts)}, sizes={node_sizes(texts)}, "
                  f"merged_lines={len(cur)}); mirrored copies are not retranslated",
                  file=sys.stderr)
            self._mirrored_nodes = mirrored
        # 兩軌分離但索引同源：最後依原索引合併，遊戲內的交錯順序即完整還原
        cur_all = cur                                  # 完整序列（含系統行），索引的基準
        player_idx = [i for i, l in enumerate(cur_all) if not l.system]
        system_idx = [i for i, l in enumerate(cur_all) if l.system]
        cur_system_texts = [cur_all[i].text for i in system_idx]
        cur = [cur_all[i] for i in player_idx]
        # 差分只看文字：切頻道時 chatLog 會把同樣的訊息以該頻道顏色重新染色，
        # 顏色參與相等比較會被誤判成「無重疊 → reset」而重吐整份舊訊息（重複翻譯）
        cur_texts = [l.text for l in cur]
        if not self._synced:
            self._prev = cur_texts  # 首次連上：記錄現況（含既有歷史），不回吐
            self._remember(cur_texts)
            self._prev_system = cur_system_texts
            self._remember_system(cur_system_texts)
            self._node_count = len(texts)
            self._synced = True
            print(f"[reader] baseline established (lines={len(cur)}, "
                  f"nodes={len(texts)})", file=sys.stderr)
            return _Outcome("baseline", [])
        # 暖機以「輪數」計且含空讀：重開遊戲後聊天常長時間空白，若只數非空讀，
        # 暖機永不過期，各頻道從空白冒出的第一句（走 reset）會被無限吸收
        if self._warmup_left > 0:
            self._warmup_left -= 1
        if not cur:
            # 空讀（傳送/轉場暫態清空）→ 玩家軌保留基準、忽略，不重譯。
            # 系統軌不能跟著吸收：「這一輪沒有玩家行、只有掉寶」是日常狀態，
            # 在這裡靜默吞掉等於系統訊息永遠吐不出來。
            self._empty_streak += 1
            # 節點數變化在這條早退路徑上還沒判（玩家軌一律等到有玩家行的那一輪才比對
            # _node_count），因此 node_added 只能是 False；下一輪非空讀才會補判。
            system_out = self._diff_system_lines(cur_system_texts, system_idx,
                                                 cur_all, texts, node_added=False)
            return _Outcome("empty", [cur_all[i] for i in system_out])
        baseline_stale = self._empty_streak >= STALE_BASELINE_EMPTY_POLLS
        self._empty_streak = 0
        if baseline_stale:
            print(f"[reader] baseline stale after a long empty stretch, "
                  f"handling as reset (lines={len(cur)})", file=sys.stderr)
        node_added = False
        if len(texts) != self._node_count:
            if len(texts) < self._node_count:
                # 節點減少（關閉私訊視窗等）：內容只會消失不會新增，靜默重建基準
                print(f"[reader] chatLog node count decreased "
                      f"({self._node_count}->{len(texts)}, sizes={node_sizes(texts)}), "
                      f"re-baselining without emitting", file=sys.stderr)
                self._node_count = len(texts)
                self._prev = cur_texts
                self._remember(cur_texts)
                self._prev_system = cur_system_texts
                self._remember_system(cur_system_texts)
                return _Outcome("node-decrease", [])
            # 節點增加（開私訊視窗／聊天 UI 生成）：新節點可能正載著使用者的第一句，
            # 不可盲目吸收（實測私訊第一句被吞）。串接結構已變、對齊無意義，
            # 跳過對齊直接走 reset 語意：空基準照吐、暖機期吸收、其後看過集合過濾
            print(f"[reader] chatLog node count increased "
                  f"({self._node_count}->{len(texts)}, sizes={node_sizes(texts)}), "
                  f"handling as reset", file=sys.stderr)
            self._node_count = len(texts)
            node_added = True
        prev_texts = self._prev
        prev_len = len(prev_texts)
        force_reset = node_added or baseline_stale
        path = "append"
        appended = None if force_reset else align_append(self._prev, cur_texts)
        if appended is None and not force_reset:
            # 前綴對不齊（撕裂讀取等）→ 以尾段在 cur 的最後出現位置恢復
            path = "recover"
            appended = align_recover(self._prev, cur_texts)
            if appended is not None:
                print(f"[reader] baseline misaligned, recovered via tail anchor "
                      f"(prev={prev_len}, cur={len(cur)}, emitted={len(appended)}, "
                      f"nodes={len(texts)}, sizes={node_sizes(texts)})", file=sys.stderr)
        if appended is None:
            # 與基準完全無重疊 → 首次切到沒讀過的分頁視圖、relog 成全新內容，
            # 或單行置換式視圖（朋友視窗：每句新話取代整個視圖內容）的新訊息。
            # 暖機期內（見 RESET_WARMUP_POLLS）一律靜默吸收——堵啟動盲區；
            # 暖機期後照常輸出、交由下方看過集合過濾：沒見過的行（真新訊息）
            # 照吐，重浮歷史剔除。基準為空（連上時聊天是空的）不受暖機限制。
            path = "reset"
            if self._prev and self._warmup_left > 0:
                print(f"[reader] no overlap with baseline during warmup, absorbed "
                      f"(lines={len(cur)}, cur_head={cur_texts[0][:40]!r}, "
                      f"prev_tail={self._prev[-1][:40]!r})", file=sys.stderr)
                self._prev = cur_texts
                self._remember(cur_texts)
                self._prev_system = cur_system_texts
                self._remember_system(cur_system_texts)
                return _Outcome("warmup", [])
            print(f"[reader] chat log has no overlap with baseline, treating as reset "
                  f"(lines={len(cur)}, cur_head={cur_texts[0][:40]!r}, "
                  f"prev_tail={self._prev[-1][:40] if self._prev else ''!r})",
                  file=sys.stderr)
            appended = cur_texts
        self._prev = cur_texts
        # 對齊各路徑回傳的都是 cur 的尾段：以長度切回 ChatLine，帶出當前顏色
        emitted = cur[len(cur) - len(appended):]
        if path == "append":
            # 巧合對齊防線：視圖 A 的內容恰為視圖 B 的前綴時，A→B 的切換會被 append
            # 誤判成「新增了 B 的其餘舊行」且不經過任何過濾（實機每次切分頁重翻的主因）。
            # 單行 append（正常訊息與重複的 lol/gg）永不過濾。
            if len(appended) >= 2 and all(t in self._seen for t in appended):
                # 吐出的行全部見過 → 視圖重浮
                print(f"[reader] append of {len(appended)} all-seen lines absorbed "
                      f"as view resurface (prev={prev_len})", file=sys.stderr)
                emitted = []
            elif len(appended) >= 3 and len(appended) > 2 * prev_len:
                # 一輪暴增超過基準兩倍 → 不可能的人為速度，判定為切到內容較多的視圖。
                # 代價：聊天剛起步（基準 1-2 行）時 1 秒內連發 3 句會被吸收，下句恢復。
                # 但整批丟棄前要先看有多少是看過的：chatLog 會短暫膨脹成重複版本
                # （實機 102 行 ×8 ≈ 822 行），此時抵達的新訊息夾在裡面會一起被丟掉。
                kept = self._drop_resurfaced(emitted, path)
                if len(kept) > len(appended) // DUPLICATE_BURST_SEEN_RATIO:
                    print(f"[reader] implausible append burst absorbed as view switch "
                          f"(appended={len(appended)}, unseen={len(kept)}, "
                          f"prev={prev_len})", file=sys.stderr)
                    emitted = []
                else:
                    # 絕大多數都看過＝內容重浮，剩下的寥寥幾行是夾在裡面的新訊息 → 照吐
                    emitted = kept
            elif len(appended) >= BULK_APPEND_FILTER_MIN:
                # 混合批次（大量重浮歷史夾帶少數沒讀過的行）：見 BULK_APPEND_FILTER_MIN
                emitted = self._drop_resurfaced(emitted, path)
        # 慢路徑（視圖切換/異常讀取）過濾重浮歷史。
        # 過濾要在 _remember 之前——本輪剛出現的新行還不在集合裡，才吐得出來。
        # 例外：空讀轉場後只冒出一行**且內容與清空前不同**＝剛到的新訊息，不過濾；
        # 照過濾會讓與舊訊息同字的新訊息整句消失（實機回報：轉場後的第一句私訊
        # Test 因啟動基準裡有人講過同一句而被吞，對方講第二次才翻得出來）。
        # 內容一字不差地填回則不適用——視圖裡只有一行玩家訊息時，轉場清空再還原
        # 長得就像「單行新訊息」，例外會讓它每次轉場都重譯（實機回報）。這種讀取
        # 交回看過集合過濾；使用者自己重打的同字句仍由輸入框關聯放行。
        elif emitted:
            if baseline_stale and len(emitted) == 1 and cur_texts != prev_texts:
                print(f"[reader] single line after an empty stretch kept as new "
                      f"(text={emitted[0].text[:40]!r}, path={path})", file=sys.stderr)
            else:
                emitted = self._drop_resurfaced(emitted, path)
        self._remember(cur_texts)
        player_out = self._guard_burst(emitted, path, prev_len, len(cur), texts)
        system_out = self._diff_system_lines(cur_system_texts, system_idx, cur_all,
                                             texts, node_added)
        # 依原索引合併：兩軌各自走了哪條路徑都不影響相對順序（索引同源）
        merged = sorted(player_out_with_idx(player_out, cur, player_idx) + system_out)
        return _Outcome(path, [cur_all[i] for i in merged], len(appended))

    def _drop_resurfaced(self, emitted: list[ChatLine], path: str) -> list[ChatLine]:
        """剔除看過集合裡已有的行（重浮歷史），沒見過的行保留。
        呼叫端必須在 _remember 之前呼叫——本輪剛出現的新行還不在集合裡，才吐得出來。"""
        kept = filter_resurfaced(emitted, self._seen)
        if (len(kept) != len(emitted) and self._input_recent > 0
                and not self._released_for_input and emitted[-1].own
                and (not kept or kept[-1] is not emitted[-1])):
            # 輸入框剛關閉＝使用者剛送出訊息：視圖尾行與舊訊息同字（重打同一句）
            # 會被誤判重浮，關聯放行尾行；其餘被攔的行維持剔除。
            # 限自己講的那行：轉場期間輸入框狀態會亂跳，只看輸入框活動會把別人的
            # 舊訊息當成「剛送出」放行而重翻（實機回報）
            # 每次開啟輸入框只放行一次（見 _released_for_input 的重置點）：這個機制
            # 分不出「重打同一句」與「切頁籤讓同一句重新浮現」，不設限的話後者會
            # 每切回來就重複顯示一次。
            print(f"[reader] released tail line suppressed as resurfaced: input "
                  f"closed recently, treating as a just-sent message via {path}",
                  file=sys.stderr)
            self._released_for_input = True
            kept = kept + [emitted[-1]]
        if len(kept) != len(emitted):
            print(f"[reader] suppressed {len(emitted) - len(kept)} resurfaced "
                  f"lines via {path} (kept={len(kept)}, "
                  f"seen={len(self._seen)})", file=sys.stderr)
        return kept

    def _diff_system_lines(self, cur_texts: list[str], system_idx: list[int],
                           cur_all: list[ChatLine], texts: list[str],
                           node_added: bool = False) -> list[int]:
        """系統訊息的差分軌，回傳新增行在完整序列中的索引。

        路徑決策刻意不與玩家軌共用：玩家軌的「空讀＝轉場暫態清空」在這裡不成立——
        「這一輪沒有系統訊息」是日常狀態，照搬 STALE_BASELINE_EMPTY_POLLS 會讓基準
        不斷過期、把系統軌長期推去走 reset，大幅拉高重吐機率。

        `node_added` 是**兩軌共同的事實**（由 _diff_new_lines() 統一判定）：節點增加
        代表串接結構已變，對齊此時毫無意義，會生出假的 append——而 append 路徑不過
        看過集合，剛掉過的寶就會再吐一次、再打一次 API。因此比照玩家軌跳過對齊直接
        走 reset 語意，讓 _seen_system 把重浮的行擋下來。

        `emit_system` 為 False 時仍照常推進基準與看過集合，只是不回傳——否則使用者
        中途打開開關的瞬間，整份歷史系統訊息會被當成新訊息一次吐出、翻上百則。
        """
        if not cur_texts:
            return []            # 沒有系統訊息：保留基準，不累計 stale
        if self._warmup_system_left > 0:
            self._warmup_system_left -= 1
        prev = self._prev_system
        prev_len = len(prev)
        path = "append"
        appended = None if node_added else align_append(prev, cur_texts)
        if appended is None and not node_added:
            path = "recover"
            appended = align_recover(prev, cur_texts)
        if appended is None:
            path = "reset"
            if prev and self._warmup_system_left > 0:
                print(f"[reader] system track absorbed during warmup "
                      f"(lines={len(cur_texts)})", file=sys.stderr)
                self._prev_system = cur_texts
                self._remember_system(cur_texts)
                return []
            reason = ("chatLog node count increased" if node_added
                      else "no overlap with baseline")
            print(f"[reader] system track handled as reset ({reason}): "
                  f"prev={prev_len}, cur={len(cur_texts)}", file=sys.stderr)
            appended = cur_texts
        self._prev_system = cur_texts
        emitted_texts = appended
        if path != "append" or len(appended) >= BULK_APPEND_FILTER_MIN:
            # 與玩家軌同策略：正常新增一律放行，只在慢路徑與大批次過濾重浮歷史
            kept = [t for t in emitted_texts if t not in self._seen_system]
            if len(kept) != len(emitted_texts):
                print(f"[reader] system track dropped {len(emitted_texts) - len(kept)} "
                      f"resurfaced lines via {path}", file=sys.stderr)
            emitted_texts = kept
        self._remember_system(cur_texts)
        if len(emitted_texts) > MAX_NEW_LINES_PER_POLL:
            print(f"[reader] implausible system burst suppressed via {path}: "
                  f"{len(emitted_texts)} new lines in one poll (prev={prev_len}, "
                  f"cur={len(cur_texts)}, nodes={len(texts)}); re-baselined without "
                  f"emitting", file=sys.stderr)
            return []
        if len(emitted_texts) > SYSTEM_LARGE_BATCH_LOG_THRESHOLD:
            print(f"[reader] large system batch via {path}: {len(emitted_texts)} lines "
                  f"(prev={prev_len}, cur={len(cur_texts)}, nodes={len(texts)})",
                  file=sys.stderr)
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

    def _remember_system(self, texts: list[str]) -> None:
        """系統軌的看過集合，容量上限與玩家軌相同但完全獨立。"""
        for t in texts:
            if t not in self._seen_system:
                self._seen_system.add(t)
                self._seen_system_order.append(t)
        while len(self._seen_system_order) > SEEN_LINES_CAP:
            self._seen_system.discard(self._seen_system_order.popleft())

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
            raise GameNotRunning(f"chatlog read failed (likely left the game): {exc}") from exc

    # --- 與 wizwalker 的 I/O 接縫（測試中覆寫 _grab_texts）---
    def _grab_texts(self) -> list[str]:
        async def _grab() -> list[str]:
            nodes = await self._client.root_window.get_windows_with_name("chatLog")
            return [await n.maybe_text() for n in nodes]

        return self._run(_grab())

    def _connect(self) -> None:
        import wizwalker.utils
        from pymem.exception import CouldNotOpenProcess
        from wizwalker import ClientHandler

        path = self._game_path or detect_install_path()
        if path:
            wizwalker.utils._OVERRIDE_PATH = path  # Steam 版無登錄檔安裝路徑，需覆寫

        self._loop = asyncio.new_event_loop()
        self._handler = ClientHandler()
        try:
            clients = self._handler.get_new_clients()
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
        if not clients:
            self._teardown()
            raise GameNotRunning(f"game process not found: {self.process_name}")
        self._client = clients[0]
        self._pid = self._client.process_id
        hook_state.sweep(_pid_alive)          # 清掉已不在執行的程序的殘留狀態檔
        self._repair_leaked_hooks(self._pid)  # 修復上次髒退出遺留的 hook（免重開遊戲）
        try:
            # 只啟讀聊天所需的 root_window hook（不啟 player/duel/quest 等），
            # 注入最小化、且不受是否在世界內等遊戲狀態影響。
            # 掛入與等待就緒刻意拆成兩步（wait_for_ready=False）：wizwalker 內建的等待
            # 無限期（見 HOOK_READY_TIMEOUT），且中間這一步要先把還原狀態存起來——
            # hook 此刻已寫進遊戲記憶體，等待途中被硬砍才修得回來。
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
        print(f"[reader] attached to game (pid={self._pid}, hook_ready_in={waited:.1f}s)",
              file=sys.stderr)

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
            print(f"[reader] root window hook never fired within "
                  f"{HOOK_READY_TIMEOUT:.0f}s (pid={self._pid}); the hook pattern "
                  f"likely does not match this game build", file=sys.stderr)
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
        # 差分狀態屬於單一遊戲 session：斷線（多半是遊戲重開）後全部歸零。
        # 沿用舊 session 的基準/看過集合，會把新 session 與舊訊息同字的第一句
        # （Test/lol 等常用字）誤判為重浮歷史而吞掉。
        self._prev = []
        self._node_count = None
        self._mirrored_nodes = 0
        self._synced = False
        self._seen.clear()
        self._seen_order.clear()
        self._warmup_left = RESET_WARMUP_POLLS
        self._prev_system = []
        self._seen_system.clear()
        self._seen_system_order.clear()
        self._warmup_system_left = RESET_WARMUP_POLLS

    def close(self) -> None:
        """停止時呼叫：解除 hook、關閉連線。"""
        self._teardown()
