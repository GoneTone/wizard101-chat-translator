"""介面字型：依當前介面語言選字族。

微軟正黑體雖然顯示得出簡體字，但字形是台灣標準；英文介面用它也不如 Segoe UI。
所有 tkinter 字型都經過這裡，語言一換字型就跟著換。"""
from src.i18n import current_language

_FAMILIES = {
    "zh-TW": "Microsoft JhengHei",
    "zh-CN": "Microsoft YaHei",
    "en": "Segoe UI",
}
_FALLBACK_FAMILY = "Segoe UI"


def ui_font(size: int, weight: str | None = None) -> tuple:
    """回傳 tkinter 字型 tuple（字族取自當前介面語言）。"""
    family = _FAMILIES.get(current_language(), _FALLBACK_FAMILY)
    return (family, size) if weight is None else (family, size, weight)
