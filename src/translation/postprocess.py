"""譯文後處理與品質判定：去掉 <think> 區塊、清除模型自行補上的括號英文、
判斷譯文有沒有落回拉丁文字（改用官方英文名或整句沒翻）。純字串函式，不碰網路。
"""
import re

from src.reader.markup import SENDER_PREFIX
from src.translation.prompts import REGION_SEPARATOR

_THINK_BLOCK = re.compile(r"<think>.*?</think>\s*", re.DOTALL | re.IGNORECASE)


def strip_think(text: str) -> str:
    """移除回應中的 <think>…</think> 推理區塊（reasoning 模型會把思考夾在 content 裡）；
    無論是否啟用思考都套用。"""
    return _THINK_BLOCK.sub("", text)


_SENDER_PREFIX = re.compile(rf"^{SENDER_PREFIX}\s*")
# 半形或全形括號包住、以英文字母開頭的內容（模型補上的英文名長這樣）
_PAREN_ENGLISH = re.compile(r"\s*[（(][A-Za-z][A-Za-z0-9 .'\-]*[)）]")
_LATIN = re.compile(r"[A-Za-z]")
# 一段連續的拉丁文字（字母起頭，可含數字、詞內標點與空白）：
# 「Received a friend request from Amy」算一段，而非被空白切成六段。
_LATIN_RUN = re.compile(r"[A-Za-z][A-Za-z0-9 .,'’\-]*")
# 目標語言的「字」：漢字、假名、諺文、西里爾字母都是 \w，空白、數字與標點不是。
_NON_LATIN_WORD = re.compile(r"[^\W\d_]")


def _content_has_latin(text: str) -> bool:
    """訊息內容裡有沒有拉丁字母；`[發送者]` 前綴不算（發送者名是英文，與內容無關）。"""
    return _LATIN.search(_SENDER_PREFIX.sub("", text)) is not None


def strip_invented_english(source: str, translated: str) -> str:
    """原文不含英文時，移除譯文裡以括號補上的英文名。

    提示詞已要求括號英文只能照抄原文（見 _game_noun_rule），但小模型遵從度不穩：實測
    同一則簡體中文材料名兩次分別補上 (Psychedelic Wood) 與 (Mystic Wood)，遊戲裡並沒有
    這個英文名。原文一個英文字母都沒有時，括號英文必然是憑空生成。
    括號裡不是英文（中文註解等）一律不動。"""
    if _content_has_latin(source):
        return translated   # 原文有英文，括號裡可能是照抄的
    return _PAREN_ENGLISH.sub("", translated)


def uses_latin_script(language: str) -> bool:
    """這個語言是否以拉丁字母書寫。看語言名稱本身的文字：目標語言是人讀名稱，預設清單
    一律是自稱（見 ui.fields.COMMON_LANGUAGES），`English`、`Español` 對上
    `繁體中文（台灣）`、`日本語`，不必在程式碼裡列語言名冊。"""
    return _LATIN.search(language) is not None


def _squash(text: str) -> str:
    """比對譯文片段與原文用的正規化：空白壓成一格、忽略大小寫。"""
    return re.sub(r"\s+", " ", text).strip().casefold()


def has_stray_latin(source: str, translated: str, target_language: str) -> bool:
    """譯文是否出現了不該有的拉丁文字 —— 模型改用英文名，或整句沒翻。

    實機症狀：中文伺服器的裸名詞被翻成官方英文名（`雪刺帽` → `Snowspike Hat`），
    音譯的玩家名被還原成英文（`卡拉米蒂` → `Calamity`）；strip_invented_english
    只清括號裡的英文，抓不到這種整段或半段英譯。

    兩條規則缺一不可：
    1. 譯文整段都是拉丁、沒有一個目標語言的字 —— 改用了英文名，或原文原樣吐回
       （拉丁文字伺服器上的主要失敗樣態）。
    2. 譯文裡某個拉丁片段不是原文既有的字串。只看「原文有沒有英文」不夠：系統訊息
       常夾英文玩家名（`收到 [Amy] 的伙伴邀请`），整條放行後整句英譯也抓不到；
       逐片段比對才擋得住，又不誤傷照抄的 `Amy`。

    已知盲點（字元層面無解）：目標語言本身用拉丁字母（`Español`）時一律回 False，
    `Snowspike Hat` 與 `Sombrero de Nieve` 分不出來；同文字系統之間也看不出
    （目標繁中卻回吐簡中）。"""
    if uses_latin_script(target_language):
        return False
    body = _SENDER_PREFIX.sub("", translated)
    if not _LATIN.search(body):
        return False
    runs = [run.group() for run in _LATIN_RUN.finditer(body)]
    if not _NON_LATIN_WORD.search(_LATIN_RUN.sub("", body)):
        return True     # 規則 1：整段沒有一個目標語言的字
    haystack = _squash(_SENDER_PREFIX.sub("", source))
    return any(_squash(run) not in haystack for run in runs)   # 規則 2


def split_region_output(text: str) -> tuple[str, str]:
    """把區域翻譯看圖路徑的原始輸出切成（逐字抄寫, 譯文）。

    分隔線就是那行 strip() 後只剩連字號的一行（見 prompts.REGION_SEPARATOR）；
    容忍前後多餘空白，也容忍模型多打幾個連字號的變體，只要求至少 3 個、不再要求
    剛好等於 REGION_SEPARATOR。找不到分隔線代表模型沒照格式輸出（多半是舊版提示詞
    快取或本機小模型不遵從格式），整段回退當成純譯文，原文留空 —— 卡片就不顯示
    原文列，行為等同關閉這個功能前。"""
    if not text:
        return "", ""
    lines = text.split("\n")
    for i, line in enumerate(lines):
        stripped = line.strip()
        if stripped == REGION_SEPARATOR or (stripped.strip("-") == "" and len(stripped) >= 3):
            original = "\n".join(lines[:i]).strip()
            translated = "\n".join(lines[i + 1:]).strip()
            return original, translated
    return "", text.strip()
