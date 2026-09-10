"""chatLog 富文字標記的純解析：去標記、判斷發言者／系統訊息、取遊戲顯示色、
把多個 chatLog 節點串成單一行序列。全部是純函式，不需遊戲即可測試。

聊天在顯示層是帶標記的富文字，每則一行、以 `\n` 分隔：
    他人： <color;..><image;Art/Art_Chat_Say.dds;..> <link;GID:<id>,<名>,2>[<名>]</link> 內文 </color>
    自己： <color;..><image;Art/Art_Chat_Say.dds;..> [你] 內文 </color>          ← 無 <link;GID>
玩家發言帶頻道圖示（Art_Chat_<頻道>，房間頻道為 chat_balloon_*）、系統訊息用
Art_Chat_System、除錯行無圖示，以此分流；去標記後回傳「[發送者] 內文」，連同行首
<color;..> 的遊戲顯示色一起帶出（ChatLine，供 overlay 對齊遊戲配色）。
"""
import re
from collections import Counter
from typing import NamedTuple

from src.log import log


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
# 玩家發言行都帶頻道圖示：多數頻道是 Art_Chat_<頻道>，房間頻道實測是
# chat_balloon_<Owner/Guest>，快捷訊息（禁言帳號只能用選單發話）是 Art_Word_Balloon。
# 自己的發言是 [你] 開頭、無 <link;GID>，故不能只靠 link 過濾。
_PLAYER_IMG_PREFIXES = ("<image;Art/Art_Chat", "<image;Art/chat_balloon",
                        "<image;Art/Art_Word_Balloon")
_SYSTEM_IMG = "<image;Art/Art_Chat_System"
# 伺服器公告（維修預告等）實測連圖示都沒有，只有行首 <color;..>（見 _is_server_broadcast）
_ART_IMG = "<image;Art/"
_LEADING_COLOR = re.compile(r"^\s*<color;")
# 他人發言的名字是可點擊的玩家連結，自己的只有純文字 [你]；[你] 各語系用語不同，
# 故以連結有無判斷是否自己講的，不比對名稱字串
_OTHER_PLAYER_LINK = "<link;GID"
# 任意 Art/ 圖示（診斷用）：長得像聊天行但圖示不在白名單 → 可能是漏接的頻道
_ANY_ART_IMG = re.compile(r"<image;(Art/[^.;>]+)\.dds", re.IGNORECASE)
_warned_icons: set[str] = set()  # 每種未知圖示每次執行只警告一次，避免洗版

# 遊戲表情以 <image;Emoticons/名稱.dds;24;24;..> 內嵌，保留成 :名稱: 文字（不轉 emoji），
# 避免整行只有表情時被去光而消失
_EMOTE_TAG = re.compile(r"<image;Emoticons/([^.;>]+)\.dds[^>]*>", re.IGNORECASE)


def _emote_to_char(m: re.Match) -> str:
    name = re.sub(r"^emoticons?_|\d+$", "", m.group(1).lower())
    if not re.fullmatch(r"[a-z0-9_]+", name):
        return ""  # 撕裂的標記（名稱夾入雜字）：丟棄該表情，保留整行其餘內容
    return f":{name}:"


def clean(text: str) -> str:
    """表情標記保留成 `:名稱:`，去掉 <color;..> <image;..> <link;..> </..> 等標記，
    還原玩家實際打出的 `&lt;` `&gt;` `&amp;` 實體，壓縮空白。"""
    text = _EMOTE_TAG.sub(_emote_to_char, text)
    text = _TAG.sub("", text)
    text = text.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
    return " ".join(text.replace("\x00", " ").split())


def line_color(raw: str) -> str | None:
    """從一行原始 markup 取遊戲顯示色（`#rrggbb` 小寫）；沒有顏色標記回傳 None。"""
    m = _COLOR_TAG.search(raw)
    return f"#{m.group(1)[-6:].lower()}" if m else None


def lines_from_chatlog(text: str) -> list[ChatLine]:
    """把 chatLog 全文（以 `\n` 分行的渲染 markup）解析成乾淨聊天行，保留順序與重複。

    收玩家發言（含自己的 `[你]` 行與他人 `<link;GID>[名]` 行）、系統訊息
    （Art_Chat_System）與伺服器公告（_is_server_broadcast）；濾掉遊戲除錯行
    （[STAT]/[DBGL]/[DBGM]，既無頻道圖示也無行首顏色）。玩家行必須是「[發送者] 內容」
    （_VALID）；系統訊息與公告沒有固定前綴，clean() 後非空即收。"""
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
    """有行首顏色、卻沒有任何 Art/ 圖示 → 伺服器公告。

    實機樣本只有 `<color;D9ABF8>[Server Message] 內文</color>`，前綴不保證存在故不比對
    字串；遊戲除錯輸出（`RECEIVED STATUS UPDATE for [id]`）同樣無圖示但不以 <color;..>
    起頭。先排除帶 Art/ 圖示的行，未收錄的頻道才會照舊落到 _warn_unknown_icon，
    不會被悄悄吞成系統訊息。"""
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
    log(f"[reader] unrecognized chat icon {m.group(1)!r}, line dropped "
        f"(add prefix to _PLAYER_IMG_PREFIXES if this is a player channel); "
        f"raw={raw[:160]!r}")


def _mirrors(part: list[ChatLine], main: list[ChatLine]) -> bool:
    """part 的每一行（含重複次數）都能在 main 裡找到 → part 只是 main 的鏡射。"""
    remaining = Counter(line.text for line in main)
    for line in part:
        if not remaining[line.text]:
            return False
        remaining[line.text] -= 1
    return True


def lines_from_nodes(texts: list[str]) -> tuple[list[ChatLine], int]:
    """把各 chatLog 節點的全文依傳入順序串接成單一聊天行序列，
    回傳（行序列，被剔除的鏡射節點數）。

    組隊等浮動聊天視窗各自是一個 chatLog 節點，且會把同一則訊息再渲染一份（實測開組隊
    視窗後發話，同一句出現在兩個節點，sizes=[1, 1, 0]）；盲目串接會翻兩次，之後主視圖在
    完整歷史與精簡視圖間跳動時，多出的那份還會被 align_append 當成尾端新增而每次重翻。
    以玩家行數最多的節點為主視圖，玩家行被它完全涵蓋（含重複次數）的節點判定為鏡射；
    代價是兩個視窗恰各出現一句一字不差的訊息時只翻一次。空節點不算鏡射，否則診斷 log
    每輪都在響。鏡射判定只看玩家行 —— 系統訊息在不同節點的出現方式未經實測，
    判定完成後才從保留的節點取出系統行。"""
    parts = [lines_from_chatlog(t) for t in texts]
    if len(parts) < 2:
        return [line for p in parts for line in p], 0
    players = [[line for line in p if not line.system] for p in parts]
    main = max(range(len(parts)), key=lambda i: len(players[i]))
    kept = [p for i, p in enumerate(parts)
            if i == main or not (players[i] and _mirrors(players[i], players[main]))]
    return [line for p in kept for line in p], len(parts) - len(kept)


def node_sizes(texts: list[str]) -> list[int]:
    """各 chatLog 節點的聊天行數（含系統訊息），依串接時的排序；診斷用，看得出哪個節點在
    灌入完整歷史、sorted 名次有無翻轉。只在要印診斷 log 時呼叫 —— 每輪都算等於解析成本翻倍。"""
    return [len(lines_from_chatlog(t)) for t in sorted(texts)]
