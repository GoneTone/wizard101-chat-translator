"""介面文案資源：依當前介面語言提供字串。

語言檔為扁平 key-value JSON（`<語言碼>.json`），繁體中文（台灣）是來源語言
（Crowdin 的上傳來源、測試基準）。缺字串時**先退英文**——翻譯平台上譯文未完成
是常態，退到多數人讀得懂的語言比退到繁中合理；繁中排在英文之後當保底，因為新文案
一定先進來源語言，英文有可能還沒跟上。本模組只負責「給字串」，不碰 UI、不碰 config 讀寫。
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

SOURCE_LANGUAGE = "zh-TW"   # 文案來源語言：Crowdin 上傳來源、測試基準、fallback 的最後一層
DEFAULT_LANGUAGE = "en"     # 尚未設定、偵測不到或語言碼不認得時的預設，也是缺字串時優先退的語言

# Windows locale 名稱 → 介面語言碼。未列出者一律退 DEFAULT_LANGUAGE。
_LOCALE_MAP = {
    "zh_TW": "zh-TW", "zh_HK": "zh-TW", "zh_MO": "zh-TW",
    "zh_CN": "zh-CN", "zh_SG": "zh-CN",
}

_current = DEFAULT_LANGUAGE   # set_language() 被呼叫前的預設（main.py 啟動時一定會設）
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
    """切換介面語言；語言碼不認得時退回 DEFAULT_LANGUAGE。

    先載入再切換 `_current`：載入失敗（缺檔、內容損毀）就拋出例外，
    此時 `_current` 仍停在原本能正常運作的語言，不會讓後續 `t()` 全數炸開。"""
    global _current
    if code not in LANGUAGES:
        print(f"[i18n] unknown language: {code}, using {DEFAULT_LANGUAGE}",
              file=sys.stderr)
        code = DEFAULT_LANGUAGE
    _load(code)
    _current = code
    print(f"[i18n] language set: {code}", file=sys.stderr)


def fallback_order() -> list[str]:
    """字串查找順序：當前語言 → 英文 → 來源語言（繁中），重複的語言碼只留第一個。"""
    order = []
    for code in (_current, DEFAULT_LANGUAGE, SOURCE_LANGUAGE):
        if code not in order:
            order.append(code)
    return order


def t(key: str, **kwargs) -> str:
    """取當前語言的文案，並以具名變數 format。

    缺字串、或 format 失敗（譯者把變數名打壞）都往 fallback_order() 的下一個語言退；
    每一種語言都不行才回傳 key 本身——寧可顯示英文、繁中，甚至 key，
    也不要讓整個視窗因為一則譯文而拋例外。"""
    for code in fallback_order():
        template = _load(code).get(key)
        if template is None:
            print(f"[i18n] missing key: {key} lang={code}", file=sys.stderr)
            continue
        try:
            return template.format(**kwargs)
        except (KeyError, IndexError) as exc:
            print(f"[i18n] format failed: key={key} lang={code} error={exc}",
                  file=sys.stderr)
    return key


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
