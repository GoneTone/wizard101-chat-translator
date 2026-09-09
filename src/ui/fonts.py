"""介面字型：依當前介面語言選字族。

字族由各語言檔自己宣告（`language.font`） —— 微軟正黑體雖然顯示得出簡體字，
但字形是台灣標準；英文介面用它也不如 Segoe UI，這種取捨只有該語言自己知道。
所有 tkinter 字型都經過這裡，語言一換字型就跟著換。"""
from src.i18n import current_language, font_family

_FALLBACK_FAMILY = "Segoe UI"   # 語言檔沒宣告 language.font 時的保底


def ui_font(size: int, weight: str | None = None) -> tuple:
    """回傳 tkinter 字型 tuple（字族取自當前介面語言的語言檔）。"""
    family = font_family(current_language()) or _FALLBACK_FAMILY
    return (family, size) if weight is None else (family, size, weight)
