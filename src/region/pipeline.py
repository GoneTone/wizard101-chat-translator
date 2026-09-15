"""框選區域翻譯的決策核心：本機 OCR 辨識畫面文字，再交給翻譯後端翻成目標語言。

一律走本機 OCR、不把截圖交給模型看圖：兩條路徑接上 RapidOCR 後翻譯品質相當，
單一路徑對使用者更一致 —— 截圖不再離開本機，也不必偵測某個模型支不支援看圖片
（各家端點的行為不一，偵測邏輯本身就是一種維護負擔）。
"""
import time
from dataclasses import dataclass

from src.log import log
from src.region import ocr


@dataclass(frozen=True)
class RegionResult:
    """一次框選的結果：`text` 是譯文（空字串＝畫面上沒有文字），
    `source` 是本機 OCR 辨識出的原文；卡片用它在譯文上方顯示原文
    （見 ui.region_card.show_text）。"""
    text: str
    source: str = ""


class RegionPipeline:
    """本機 OCR → 文字翻譯的決策；`recognize` 可注入（測試用替身）。
    例外一律往上拋（TranslatorError 家族、TranslatorBadOutput、OcrUnavailable），
    由 UI 端轉成文案。"""

    def __init__(self, translator, recognize=ocr.recognize):
        self._translator = translator
        self._recognize = recognize

    def run(self, png: bytes, rect: tuple[int, int, int, int]) -> RegionResult:
        """辨識並翻譯一張截圖；`rect` 只用來記 log。"""
        started = time.monotonic()
        source = self._recognize(png)
        if not source:
            log(f"[region] local OCR found no text (rect={rect})")
            return RegionResult("", "")
        text = self._translator.translate_region_text(source)
        log(f"[region] done in {time.monotonic() - started:.1f}s "
            f"(rect={rect}, source_chars={len(source)}, chars={len(text)})")
        return RegionResult(text, source)
