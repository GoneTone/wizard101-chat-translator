"""譯文後處理與品質判定：去掉 <think> 區塊、清除模型自行補上的括號原文、還原發送者、
判斷譯文有沒有落回拉丁文字（改用官方英文名或整句沒翻）。純字串函式，不碰網路。
"""
import re

from src.reader.markup import SENDER_PREFIX

_THINK_BLOCK = re.compile(r"<think>.*?</think>\s*", re.DOTALL | re.IGNORECASE)


def strip_think(text: str) -> str:
    """移除回應中的 <think>…</think> 推理區塊（reasoning 模型會把思考夾在 content 裡）；
    無論是否啟用思考都套用。"""
    return _THINK_BLOCK.sub("", text)


_SENDER_PREFIX = re.compile(rf"^{SENDER_PREFIX}\s*")
_PARENTHESISED = re.compile(r"\s*[（(]([^（）()\n]+)[)）]")
_LATIN = re.compile(r"[A-Za-z]")
# 非拉丁字母的字（漢字、假名、諺文、西里爾等）；帶重音的拉丁字母（é、ñ）不算
_NON_LATIN_LETTER = re.compile(r"[^\W\d_\u0000-ɏḀ-ỿ]")
# 一段連續的拉丁文字（字母起頭，可含數字、詞內標點與空白）：
# 「Received a friend request from Amy」算一段，而非被空白切成六段。
_LATIN_RUN = re.compile(r"[A-Za-z][A-Za-z0-9 .,'’\-]*")
# 任何文字的字母（拉丁、漢字、假名、諺文、西里爾…）；空白、數字與標點不算。
_LETTER = re.compile(r"[^\W\d_]")


_NON_WORD_ONLY = re.compile(r"[\W\d_]*")


def _alnum(text: str) -> str:
    """只留字母與數字並忽略大小寫：模型照抄原文時常換掉撇號、連字號或空白。"""
    return re.sub(r"[\W_]+", "", text).casefold()


def tidy_parentheses(source: str, translated: str) -> str:
    """移除譯文裡不該有的括號原文：原文沒出現過的外文，以及只是重複前面譯名的。

    提示詞要求括號只能照抄原文（見 _game_noun_rule），但模型遵從度不穩：實測替簡中材料名
    自編 (Mystic Wood)、替英文原文自編 (塞壬)，或名詞沒翻就重複一次（Malistaire（Malistaire））。
    「外文」以括號外的譯文用什麼文字判斷，不看目標語言名稱：名稱可自由輸入（Japanese 也行）。
    括號裡是譯文本身那種文字的說明一律不動；`[發送者]` 不算原文。"""
    body = _PARENTHESISED.sub("", _SENDER_PREFIX.sub("", translated))
    latin_translation = _NON_LATIN_LETTER.search(body) is None
    haystack = _alnum(_SENDER_PREFIX.sub("", source))

    def foreign(content: str) -> bool:
        if _NON_WORD_ONLY.fullmatch(body):
            return False    # 括號外沒有字（整行都是括號）就無從判斷，一律保留
        if latin_translation:
            return _NON_LATIN_LETTER.search(content) is not None
        return _LATIN.search(content) is not None and _NON_LATIN_LETTER.search(content) is None

    def keep(match: re.Match) -> str:
        content = _alnum(match.group(1))
        if foreign(match.group(1)) and content not in haystack:
            return ""
        # 只認有字母的重複：玩家自己打的「lvl 50 (50)」不是模型的回聲
        if (len(content) > 1 and _LETTER.search(content)
                and _alnum(translated[:match.start()]).endswith(content)):
            return ""
        return match.group()

    return _PARENTHESISED.sub(keep, translated)


_LEADING_BRACKETS = re.compile(rf"^(?:{SENDER_PREFIX}\s*)+")


def _leading_bracket_count(text: str) -> int:
    match = _LEADING_BRACKETS.match(text)
    return len(re.findall(SENDER_PREFIX, match.group())) if match else 0


def sender_of(text: str) -> str | None:
    """行首的 `[發送者]`（不含其後空白）；沒有回 None。"""
    match = _SENDER_PREFIX.match(text)
    return match.group().rstrip() if match else None


def restore_sender(source: str, translated: str) -> str:
    """譯文開頭的 `[發送者]` 一律換回原文那一個；模型丟掉時補回去。

    發送者名不交給模型：實測漢化包的簡中玩家名會被改字（贾斯廷 渡鸦 → 賈斯汀 渡鴉）。
    以開頭連續的中括號組數判斷模型是改了還是丟了發送者，內容本身以 `[WTS]` 開頭時才不會誤換。"""
    prefix = sender_of(source)
    if prefix is None:
        return translated
    if _leading_bracket_count(translated) < _leading_bracket_count(source):
        return f"{prefix} {translated.lstrip()}"
    return f"{prefix} {_SENDER_PREFIX.sub('', translated, count=1)}"


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
    音譯的玩家名被還原成英文（`卡拉米蒂` → `Calamity`）；tidy_parentheses
    只清括號裡的外文，抓不到這種整段或半段英譯。

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
    if not _LETTER.search(_LATIN_RUN.sub("", body)):
        return True     # 規則 1：整段沒有一個目標語言的字
    haystack = _squash(_SENDER_PREFIX.sub("", source))
    return any(_squash(run) not in haystack for run in runs)   # 規則 2


_NUMBERED_LINE = re.compile(r"^\s*(\d+)\s*[.．、)]\s*(.*)$")


def nonblank_lines(text: str) -> list[str]:
    """拆成去掉前後空白的非空白行。"""
    return [line.strip() for line in text.splitlines() if line.strip()]


def number_lines(text: str) -> tuple[list[str], str]:
    """把多行文字拆成非空白行並加上 `1. `、`2. ` 編號，回傳（原始行, 編號後的文字）。
    給範例生成用：每行是一條獨立的示範，編號讓逐行驗證可由程式核對。"""
    lines = nonblank_lines(text)
    return lines, "\n".join(f"{i}. {line}" for i, line in enumerate(lines, 1))


def numbered_entries(output: str) -> dict[int, str]:
    """模型回傳的編號行 → {編號: 內容}；沒帶編號的行不算。"""
    entries: dict[int, str] = {}
    for line in output.splitlines():
        match = _NUMBERED_LINE.match(line)
        if match:
            entries[int(match.group(1))] = match.group(2).strip()
    return entries


def unnumber_lines(output: str, originals: list[str]) -> str:
    """把模型回傳的編號譯文依編號對回原始行；缺的編號以原文補上（寧可留原文也不漏行）。
    模型完全沒帶編號但行數剛好相同時視為逐行對應；其餘情況整段原樣回傳。"""
    numbered = numbered_entries(output)
    if numbered:
        return "\n".join(numbered.get(i) or original
                         for i, original in enumerate(originals, 1))
    plain = nonblank_lines(output)
    if len(plain) == len(originals):
        return "\n".join(plain)
    return output.strip()
