"""介面字型：依當前介面語言的文字系統選字族。

字族由語言碼的文字系統（script）推導：CLDR 知道 `ja-JP` 是 Jpan、`zh-TW` 是 Hant，
語言檔不必自己宣告 —— 譯者本來也不會知道該填哪個 Windows 字族，而 Crowdin 匯出的
語言檔更不該帶著來源語言的字族。所有 tkinter 字型都經過這裡，語言一換字型就跟著換。
"""
from langcodes import Language, LanguageTagError

from src.i18n import current_language
from src.log import log

# CJK 三家各自要專用字族：微軟正黑體雖然顯示得出簡體字，但字形是台灣標準，日韓漢字
# 借用中文字族同樣會出現當地不用的字形。其餘文字系統（拉丁、斯拉夫、希臘、希伯來、
# 阿拉伯……）Segoe UI 都涵蓋，不必逐一列舉。
_FAMILIES = {
    "Hant": "Microsoft JhengHei",
    "Hans": "Microsoft YaHei",
    "Jpan": "Yu Gothic UI",
    "Kore": "Malgun Gothic",
}
_FALLBACK_FAMILY = "Segoe UI"


def font_family(code: str) -> str:
    """語言碼 → 介面字族；文字系統認不得或沒有專用字族時回保底字族。"""
    try:
        script = Language.get(code).maximize().script
    except LanguageTagError as exc:
        log(f"[ui] unparsable language tag for font: {code!r} error={exc}")
        return _FALLBACK_FAMILY
    return _FAMILIES.get(script, _FALLBACK_FAMILY)


def ui_font(size: int, weight: str | None = None) -> tuple:
    """回傳 tkinter 字型 tuple（字族取自當前介面語言的文字系統）。"""
    family = font_family(current_language())
    return (family, size) if weight is None else (family, size, weight)
