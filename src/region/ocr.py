"""本機 OCR：RapidOCR（PP-OCR ONNX 模型），PNG bytes → 文字；框選翻譯唯一的辨識路徑
（見 region.pipeline）。原本用 Windows 內建的
Windows.Media.Ocr，但它讀不動遊戲的美術字型 —— 商店標題「Items Recommended
For Your Wizard」試過 56 種前處理組合（縮放倍率、二值化、對比度全排列）全數
失敗；RapidOCR 用同一個模型就能讀出英文與簡體中文，不必挑語言包。
引擎（`rapidocr.RapidOCR`）延遲到第一次呼叫才建立：import 與載入模型約要
0.4 秒，程式啟動不該為一個可能永遠用不到的退路先付這筆成本。
"""
import io
import logging
import statistics
import threading
import time

from PIL import Image

from src.log import log

# rapidocr 套件內部固定用這個名稱建立 logger（見 rapidocr/utils/log.py），
# 且預設等級是 INFO、外加 ANSI 顏色碼 —— 原樣落進 app.log 會是一堆控制碼。
_RAPIDOCR_LOGGER_NAME = "RapidOCR"


class OcrUnavailable(Exception):
    """本機 OCR 不可用：rapidocr／onnxruntime 套件無法載入，或引擎建立失敗。"""


_engine = None
_engine_lock = threading.Lock()


def _create_engine():
    """建立 RapidOCR 引擎（預設設定即可，模型已隨套件內建）。

    `RapidOCR.__init__` 建構到一半就會用讀到的設定把 logger 等級重設成
    INFO、且初始化過程本身就會印出好幾行模型路徑 —— 建構前才 setLevel 對
    這些訊息沒有用（等級會被蓋回去），得靠 `params` 直接把設定值改成
    error，讓 `__init__` 自己設回去的等級就是我們要的；建構完再設一次
    是防禦性寫法，擋住之後可能新增、不受這個設定管的 logger 用法。
    壓到 error 而非 warning：框到沒有文字時它會以 WARNING 印「text detection result is
    empty」（帶 ANSI 顏色碼），而這個情況本模組自己已經記了一行乾淨的 log。
    """
    logging.getLogger(_RAPIDOCR_LOGGER_NAME).setLevel(logging.ERROR)
    try:
        from rapidocr import RapidOCR
    except ImportError as exc:
        raise OcrUnavailable(f"rapidocr/onnxruntime unavailable: {exc}") from exc
    try:
        engine = RapidOCR(params={"Global.log_level": "error"})
    except Exception as exc:
        raise OcrUnavailable(f"failed to create RapidOCR engine: {exc}") from exc
    logging.getLogger(_RAPIDOCR_LOGGER_NAME).setLevel(logging.ERROR)
    return engine


def _get_engine():
    global _engine
    if _engine is None:
        with _engine_lock:
            if _engine is None:
                _engine = _create_engine()
                log("[region] local OCR engine ready (rapidocr)")
    return _engine


def merge_lines(items: list[tuple[list, str]]) -> list[str]:
    """把 RapidOCR 逐框辨識的結果併回可讀的行。

    `items` 是 `(box, text)`：`box` 是框的 4 個角點 `[[x, y], ...]`。同一視覺
    行常被切成好幾個框（例如商店標題「Items」「Recommended For Your」「Wizard」
    各自成框），純靠框的順序無法還原行 —— 用框的中心 y 分組：與同一行已知
    y 差距小於「全部框高度中位數的一半」的框歸為同一行，行間再由上到下排序、
    行內由左到右排序，最後以單一空白接起來。
    """
    if not items:
        return []

    def y_center(box: list) -> float:
        ys = [point[1] for point in box]
        return sum(ys) / len(ys)

    def height(box: list) -> float:
        ys = [point[1] for point in box]
        return max(ys) - min(ys)

    def x_left(box: list) -> float:
        return min(point[0] for point in box)

    threshold = statistics.median(height(box) for box, _ in items) / 2
    ordered = sorted(items, key=lambda item: y_center(item[0]))

    lines: list[list[tuple[list, str]]] = []
    line_ys: list[float] = []
    for box, text in ordered:
        yc = y_center(box)
        if lines and abs(yc - line_ys[-1]) < threshold:
            lines[-1].append((box, text))
            line_ys[-1] = sum(y_center(b) for b, _ in lines[-1]) / len(lines[-1])
        else:
            lines.append([(box, text)])
            line_ys.append(yc)

    return [" ".join(text for _, text in sorted(line, key=lambda item: x_left(item[0])))
            for line in lines]


def recognize(png: bytes) -> str:
    """辨識 PNG 裡的文字，各行以換行合併（見 merge_lines）；沒有文字回空字串。

    丟掉 alpha 再交給引擎：`rapidocr` 的 `LoadImage` 直接接受 PIL Image，
    會依原圖是 RGB 自動轉成引擎要的 BGR，不必自己換色階順序。
    """
    started = time.monotonic()
    engine = _get_engine()
    image = Image.open(io.BytesIO(png)).convert("RGB")
    result = engine(image)
    if result.txts is None:
        log(f"[region] local OCR done (boxes=0, lines=0, "
            f"ms={(time.monotonic() - started) * 1000:.0f})")
        return ""
    lines = merge_lines(list(zip(result.boxes, result.txts, strict=True)))
    log(f"[region] local OCR done (boxes={len(result.txts)}, lines={len(lines)}, "
        f"ms={(time.monotonic() - started) * 1000:.0f})")
    return "\n".join(lines)
