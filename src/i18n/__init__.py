"""介面文案資源：依當前介面語言提供字串。

語言檔為扁平 key-value JSON（`<語言碼>.json`），繁體中文（台灣）是來源語言，
也是缺字串時的 fallback 對象——翻譯平台上譯文未完成是常態，介面不可因此炸開。
本模組只負責「給字串」，不碰 UI、不碰 config 讀寫。
"""
import json
import locale
import sys
from pathlib import Path

# 語言碼 → endonym（該語言自稱）。語言選單在任何介面語言下都顯示 endonym、不翻譯。
LANGUAGES: dict[str, str] = {
    "zh-TW": "繁體中文（台灣）",
    "zh-CN": "简体中文（中国）",
    "en": "English",
}

SOURCE_LANGUAGE = "zh-TW"   # 文案來源語言，同時是 fallback 對象
DEFAULT_LANGUAGE = "en"     # 偵測不到或語言碼不認得時的退路

# Windows locale 名稱 → 介面語言碼。未列出者一律退 DEFAULT_LANGUAGE。
_LOCALE_MAP = {
    "zh_TW": "zh-TW", "zh_HK": "zh-TW", "zh_MO": "zh-TW",
    "zh_CN": "zh-CN", "zh_SG": "zh-CN",
}

_current = SOURCE_LANGUAGE
_cache: dict[str, dict[str, str]] = {}


def _i18n_dir() -> Path:
    """語言檔所在目錄。打包（frozen）時語言檔被解壓到 _MEIPASS，與 config 所在的
    exe 旁目錄不同——語言檔是唯讀資源，不能沿用 config.app_dir()。"""
    if getattr(sys, "frozen", False):
        return Path(sys._MEIPASS) / "i18n"
    return Path(__file__).resolve().parent


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
        path = _i18n_dir() / f"{code}.json"
        with path.open(encoding="utf-8") as handle:
            _cache[code] = json.load(handle, object_pairs_hook=_no_duplicates)
    return _cache[code]


def current_language() -> str:
    """目前的介面語言碼。"""
    return _current


def set_language(code: str) -> None:
    """切換介面語言；語言碼不認得時退回 DEFAULT_LANGUAGE。"""
    global _current
    if code not in LANGUAGES:
        print(f"[i18n] unknown language: {code}, using {DEFAULT_LANGUAGE}",
              file=sys.stderr)
        code = DEFAULT_LANGUAGE
    _current = code
    _load(code)
    print(f"[i18n] language set: {code}", file=sys.stderr)


def _lookup(key: str) -> str:
    strings = _load(_current)
    if key in strings:
        return strings[key]
    print(f"[i18n] missing key: {key} lang={_current}", file=sys.stderr)
    return _load(SOURCE_LANGUAGE).get(key, key)


def t(key: str, **kwargs) -> str:
    """取當前語言的文案，並以具名變數 format。

    format 失敗（譯者把變數名打錯）時退回來源語言的字串重試——寧可顯示繁中，
    也不要讓整個視窗因為一則譯文而拋例外。"""
    template = _lookup(key)
    try:
        return template.format(**kwargs)
    except (KeyError, IndexError) as exc:
        print(f"[i18n] format failed: key={key} lang={_current} error={exc}",
              file=sys.stderr)
        fallback = _load(SOURCE_LANGUAGE).get(key, key)
        try:
            return fallback.format(**kwargs)
        except (KeyError, IndexError):
            return fallback


def map_locale_name(name: str) -> str:
    """Windows locale 名稱（如 `zh_TW`）→ 介面語言碼；未支援的語言退 DEFAULT_LANGUAGE。"""
    return _LOCALE_MAP.get(name.split(".")[0], DEFAULT_LANGUAGE)


def detect_system_language() -> str:
    """偵測 Windows 使用者介面語言。任何失敗都退 DEFAULT_LANGUAGE 並留下 log。"""
    try:
        import ctypes
        lcid = ctypes.windll.kernel32.GetUserDefaultUILanguage()
        name = locale.windows_locale.get(lcid, "")
    except Exception as exc:
        print(f"[i18n] system language detection failed: {exc}", file=sys.stderr)
        return DEFAULT_LANGUAGE
    code = map_locale_name(name)
    print(f"[i18n] system language detected: lcid={lcid} locale={name} -> {code}",
          file=sys.stderr)
    return code
