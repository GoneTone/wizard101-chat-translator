"""Windows OCR(winocr)包裝:輸入 PIL Image,輸出依 y 排序的文字行。"""
from PIL import Image
import winocr


def _line_y(line: dict) -> int:
    words = line.get("words") or []
    return min((w["bounding_rect"]["y"] for w in words), default=0)


def _lines_in_order(result: dict) -> list[str]:
    lines = result.get("lines") or []
    return [ln["text"] for ln in sorted(lines, key=_line_y)]


def recognize_lines(img: Image.Image) -> list[str]:
    result = winocr.recognize_pil_sync(img, "en")
    return _lines_in_order(result)
