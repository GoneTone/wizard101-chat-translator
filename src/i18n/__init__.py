"""介面文案資源：依當前介面語言提供字串。

可選語言直接掃語言檔目錄得來，程式碼裡不留語言名冊：新增介面語言只要放一份扁平
key-value 的 `<語言碼>.json` 並在檔內宣告 `language.*` metadata，不必動程式碼。
繁體中文（台灣）是來源語言（Crowdin 上傳來源、測試基準）。缺字串先退英文再退來源
語言：譯文未完成是常態，退到多數人讀得懂的語言較合理；新文案一定先進來源語言，
英文可能還沒跟上。本模組只負責給字串，不碰 UI、不碰 config 讀寫。
"""
import json
import locale

from src.log import log
from src.resources import bundle_dir

# 語言檔自帶的 metadata（不是給譯者翻的文案）：自稱（選單顯示用，也是該語言使用者預設
# 的翻譯目標）、介面字族、要吃下的 Windows locale（同語言不同字集才需指名，見 map_locale_name）。
META_NAME = "language.name"
META_FONT = "language.font"
META_LOCALES = "language.locales"

SOURCE_LANGUAGE = "zh-TW"   # 文案來源語言：Crowdin 上傳來源、測試基準、fallback 的最後一層
DEFAULT_LANGUAGE = "en"     # 尚未設定、偵測不到或語言碼不認得時的預設，也是缺字串時優先退的語言

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
    刻意不走 `t()` 的 fallback：自稱與字型借到別的語言，漏填會偽裝成正常值
    （ja.json 忘了填自稱，選單上會出現第二個「English」）。"""
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


def font_family(code: str) -> str | None:
    """語言檔宣告的介面字族；沒宣告回 None，由呼叫端決定保底字型。"""
    return _meta(code, META_FONT) or None


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
    全部不行才回傳 key 本身——寧可顯示 key，也不讓整個視窗因一則譯文拋例外。"""
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


def map_locale_name(name: str) -> str:
    """Windows locale 名稱（如 `zh_TW`）→ 介面語言碼；對不上退 DEFAULT_LANGUAGE。
    先看各語言檔宣告的 language.locales（同語言不同字集如 zh_TW／zh_CN 必須指名），
    沒人認領才比對語言前綴（`ja_JP` → `ja`），所以多數語言不必宣告 locales。"""
    name = name.split(".")[0]
    for code in available_languages():
        if name in _meta(code, META_LOCALES).split():
            return code
    prefix = name.split("_")[0].lower()
    for code in available_languages():
        if code.lower() == prefix:
            return code
    return DEFAULT_LANGUAGE


def detect_system_language() -> str:
    """偵測 Windows 使用者介面語言。任何失敗都退 DEFAULT_LANGUAGE 並留下 log。"""
    try:
        import ctypes
        lcid = ctypes.windll.kernel32.GetUserDefaultUILanguage()
        name = locale.windows_locale.get(lcid, "")
    except Exception as exc:
        log(f"[i18n] system language detection failed: {exc}")
        return DEFAULT_LANGUAGE
    code = map_locale_name(name)
    log(f"[i18n] system language detected: lcid={lcid} locale={name} -> {code}")
    return code
