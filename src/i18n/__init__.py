"""介面文案資源：依當前介面語言提供字串。

可選語言直接掃語言檔目錄得來，程式碼裡不留語言名冊：新增介面語言只要放一份扁平
key-value 的 `<locale 代碼>.json` 並在檔內宣告 `language.*` metadata，不必動程式碼。
檔名採 Crowdin 的 locale 代碼（`en-US`／`zh-TW`／`ja-JP`），與 `crowdin.yml` 的
`%locale%` 對齊，譯文下載即就位；系統語言要對到哪一份也由這個代碼經 CLDR 資料
（langcodes）判斷，不必逐語言維護對照表。
繁體中文（台灣）是來源語言（Crowdin 上傳來源、測試基準）。缺字串先退英文再退來源
語言：譯文未完成是常態，退到多數人讀得懂的語言較合理；新文案一定先進來源語言，
英文可能還沒跟上。本模組只負責給字串，不碰 UI、不碰 config 讀寫。
"""
import ctypes
import json

from langcodes import LanguageTagError, closest_match

from src.log import log
from src.resources import bundle_dir

# 語言檔自帶的 metadata（不是一般文案）：自稱（選單顯示用，也是該語言使用者預設的
# 翻譯目標）、這份譯文的譯者掛名。兩者都由該語言的譯者填，未填就當作沒有 —— 未翻譯的
# 字串不會被匯出（crowdin.yml 的 skip_untranslated_strings），不會帶著來源語言的值進來。
META_NAME = "language.name"
META_TRANSLATORS = "language.translators"

SOURCE_LANGUAGE = "zh-TW"   # 文案來源語言：Crowdin 上傳來源、測試基準、fallback 的最後一層
DEFAULT_LANGUAGE = "en-US"  # 尚未設定、偵測不到或語言碼不認得時的預設，也是缺字串時優先退的語言

_current = DEFAULT_LANGUAGE   # set_language() 被呼叫前的預設（main.py 啟動時一定會設）
_cache: dict[str, dict[str, str]] = {}
_languages: dict[str, str] | None = None   # available_languages() 的快取


def _no_duplicates(pairs: list[tuple[str, str]]) -> dict[str, str]:
    """json.load 的 object_pairs_hook：重複 key 預設會被靜默覆蓋，這裡直接擋下。"""
    seen: dict[str, str] = {}
    for key, value in pairs:
        if key in seen:
            raise ValueError(f"duplicate key: {key}")
        seen[key] = value
    return seen


def _load(code: str) -> dict[str, str]:
    if code not in _cache:
        path = bundle_dir("i18n") / f"{code}.json"
        with path.open(encoding="utf-8") as handle:
            _cache[code] = json.load(handle, object_pairs_hook=_no_duplicates)
    return _cache[code]


def _meta(code: str, key: str) -> str:
    """讀某個語言檔自己宣告的 metadata（缺就回空字串）。
    刻意不走 `t()` 的 fallback：自稱借到別的語言，漏填會偽裝成正常值
    （ja-JP.json 忘了填自稱，選單上會出現第二個「English」）。"""
    try:
        return _load(code).get(key, "")
    except (OSError, ValueError) as exc:
        log(f"[i18n] catalog unreadable: {code} error={exc}")
        return ""


def available_languages() -> dict[str, str]:
    """可選的介面語言：語言碼 → 自稱，依語言碼字母序。掃語言檔目錄得來並快取。"""
    global _languages
    if _languages is None:
        found = {}
        for path in sorted(bundle_dir("i18n").glob("*.json")):
            code = path.stem
            name = _meta(code, META_NAME)
            if not name:
                log(f"[i18n] catalog without {META_NAME}: {code}")
            found[code] = name or code
        _languages = found
        log(f"[i18n] catalogs found: {','.join(_languages) or '(none)'}")
    return _languages


def language_name(code: str) -> str:
    """語言的自稱（endonym）；語言檔沒宣告就回語言碼本身。
    語言選單在任何介面語言下都顯示自稱、不翻譯。"""
    return available_languages().get(code) or code


def translators(code: str) -> str:
    """這份譯文的譯者掛名（可帶 `[文字](網址)` 行內連結）；沒宣告或留空回空字串。
    與其他 metadata 同樣不走 `t()` 的 fallback —— 掛名借到別的語言就是把功勞掛錯人。
    來源語言由開發者自己寫，欄位空著是正常狀態，由顯示端決定不畫那一列。"""
    return _meta(code, META_TRANSLATORS)


def forget_catalogs() -> None:
    """清掉語言檔與語言清單的快取。執行期語言檔是唯讀資源不會變，這是給測試用的。"""
    global _languages
    _cache.clear()
    _languages = None


def current_language() -> str:
    """目前的介面語言碼。"""
    return _current


def set_language(code: str) -> None:
    """切換介面語言；語言碼不認得時退回 DEFAULT_LANGUAGE。
    先載入再切換 `_current`：載入失敗就拋例外，`_current` 仍停在原本能用的語言，
    後續 `t()` 不會全數炸開。"""
    global _current
    if code not in available_languages():
        log(f"[i18n] unknown language: {code}, using {DEFAULT_LANGUAGE}")
        code = DEFAULT_LANGUAGE
    _load(code)
    _current = code
    log(f"[i18n] language set: {code}")


def fallback_order() -> list[str]:
    """字串查找順序：當前語言 → 英文 → 來源語言（繁中），重複的語言碼只留第一個。"""
    order = []
    for code in (_current, DEFAULT_LANGUAGE, SOURCE_LANGUAGE):
        if code not in order:
            order.append(code)
    return order


def t(key: str, **kwargs) -> str:
    """取當前語言的文案，並以具名變數 format。
    缺字串或 format 失敗（譯者把變數名打壞）都往 fallback_order() 的下一個語言退，
    全部不行才回傳 key 本身 —— 寧可顯示 key，也不讓整個視窗因一則譯文拋例外。"""
    for code in fallback_order():
        template = _load(code).get(key)
        if template is None:
            log(f"[i18n] missing key: {key} lang={code}")
            continue
        try:
            return template.format(**kwargs)
        except (KeyError, IndexError) as exc:
            log(f"[i18n] format failed: key={key} lang={code} error={exc}")
    return key


def best_match(tags: list[str]) -> str:
    """使用者偏好的語言標籤（BCP-47，依偏好由高到低）→ 最接近的介面語言碼。

    比對交給 CLDR 的語言距離資料（`langcodes`）：`zh-HK` 找得到 `zh-TW`、`en-GB`
    找得到 `en-US`、`pt-MZ` 找得到 `pt-PT`，語言檔不必自己宣告要認領哪些 locale。
    偏好清單裡第一個找得到夠近語言檔的標籤勝出；都不夠近（回 `und`）就退
    DEFAULT_LANGUAGE。"""
    codes = list(available_languages())
    for tag in tags:
        try:
            code, distance = closest_match(tag, codes)
        except LanguageTagError as exc:
            log(f"[i18n] unparsable language tag: {tag!r} error={exc}")
            continue
        if code != "und":
            log(f"[i18n] language tag matched: {tag} -> {code} distance={distance}")
            return code
        log(f"[i18n] no catalog close enough: {tag}")
    return DEFAULT_LANGUAGE


def preferred_ui_languages() -> list[str]:
    """Windows 使用者的介面語言偏好清單（BCP-47 標籤，依偏好排序）。

    刻意不走 GetUserDefaultUILanguage 的 LCID：`locale.windows_locale` 會把中性的
    舊 LCID 譯成 `zh_CHT` 這種非 BCP-47 的名稱，語言配對認不得（`CHT` 不是合法地區
    碼，會被當成單純的 `zh` 而對到簡中）。這支 API 直接給合法標籤，還多給了偏好順序。"""
    MUI_LANGUAGE_NAME = 0x8
    count = ctypes.c_ulong()
    size = ctypes.c_ulong()
    get = ctypes.windll.kernel32.GetUserPreferredUILanguages
    if not get(MUI_LANGUAGE_NAME, ctypes.byref(count), None, ctypes.byref(size)):
        raise OSError(ctypes.get_last_error())
    buffer = ctypes.create_unicode_buffer(size.value)
    if not get(MUI_LANGUAGE_NAME, ctypes.byref(count), buffer, ctypes.byref(size)):
        raise OSError(ctypes.get_last_error())
    return [tag for tag in buffer[:size.value].split("\0") if tag]


def detect_system_language() -> str:
    """偵測 Windows 使用者介面語言。任何失敗都退 DEFAULT_LANGUAGE 並留下 log。"""
    try:
        tags = preferred_ui_languages()
    except Exception as exc:
        log(f"[i18n] system language detection failed: {exc}")
        return DEFAULT_LANGUAGE
    code = best_match(tags)
    log(f"[i18n] system language detected: preferred={','.join(tags) or '(none)'} -> {code}")
    return code
