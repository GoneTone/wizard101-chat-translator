"""Windows OCR(winocr)包裝:輸入 PIL Image,輸出依 y 排序的文字行。"""
from PIL import Image
import winocr

# 遊戲聊天為白字黑描邊壓在場景背景上,直接 OCR 會被背景雜訊干擾。
# 實測(2026-08-19,scripts/last_capture.png)閾值 185–200 辨識最佳。
BINARIZE_THRESHOLD = 200


def binarize(img: Image.Image, threshold: int = BINARIZE_THRESHOLD) -> Image.Image:
    """亮像素(文字)轉黑、其餘轉白,輸出黑字白底的 RGB 影像。"""
    gray = img.convert("L")
    bw = gray.point(lambda p: 0 if p >= threshold else 255)
    return bw.convert("RGB")


def _line_y(line: dict) -> int:
    words = line.get("words") or []
    return min((w["bounding_rect"]["y"] for w in words), default=0)


def _lines_in_order(result: dict) -> list[str]:
    lines = result.get("lines") or []
    return [ln["text"] for ln in sorted(lines, key=_line_y)]


def recognize_lines(img: Image.Image) -> list[str]:
    result = winocr.recognize_pil_sync(binarize(img), "en")
    return _lines_in_order(result)
