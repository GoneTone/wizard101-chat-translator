"""框選區域翻譯的決策核心：截圖先交給翻譯後端看圖，端點不吃圖就退回本機 OCR 再翻文字。

「不吃圖」不靠猜：圖片請求回 4xx **且同一輪的文字翻譯成功**才標記 —— 同一個 client、
同一把金鑰、同一個模型，文字過、圖片被拒，問題只能是圖片；金鑰錯或模型不存在的情況
文字也會失敗，不會被誤標。不解析錯誤字串（各家後端措辭不同）。標記只存在記憶體，
設定套用時 reset()。
圖片請求回 5xx／429（TranslatorOffline）時這一輪同樣退回 OCR，但**不標記**：實測 OpenAI
對純文字模型收到圖片回的是 500 而非 400，不退回就永遠用不了；而 5xx 也可能只是暫時
故障，標了會讓支援看圖的模型莫名走 OCR。文字那一路也失敗才算端點真的掛了。
"""
import time
from dataclasses import dataclass

from src.log import log
from src.region import ocr
from src.translation.translator import TranslatorConfigError, TranslatorOffline


@dataclass(frozen=True)
class RegionResult:
    """一次框選的結果：`text` 是譯文（空字串＝畫面上沒有文字），
    `path` 是走了哪條路（`"image"`＝模型看圖、`"ocr"`＝本機 OCR＋文字翻譯），
    `source` 是原文 —— 看圖路徑是模型逐字抄寫出的畫面文字，OCR 路徑是本機辨識出的
    文字；卡片用它在譯文上方顯示原文（見 ui.region_card.show_text）。"""
    text: str
    path: str
    source: str = ""


class RegionPipeline:
    """看圖優先、退回本機 OCR 的決策；`recognize` 可注入（測試用替身）。
    `force_ocr` 是零參數 callable，每次 `run` 都重新讀一次（通常回傳
    `cfg["region_force_ocr"]`），設定套用後不必重建 pipeline 就生效。
    例外一律往上拋（TranslatorError 家族、TranslatorBadOutput、OcrUnavailable），
    由 UI 端轉成文案。"""

    def __init__(self, translator, recognize=ocr.recognize, force_ocr=lambda: False):
        self._translator = translator
        self._recognize = recognize
        self._force_ocr = force_ocr
        self._text_only = False

    @property
    def text_only(self) -> bool:
        """端點已被證實不吃圖片：之後直接走本機 OCR。"""
        return self._text_only

    def reset(self) -> None:
        """設定套用後呼叫：服務商或模型可能換了，重新給圖片路徑一次機會。"""
        if self._text_only:
            log("[region] text-only mark cleared (settings applied)")
        self._text_only = False

    def run(self, png: bytes, rect: tuple[int, int, int, int]) -> RegionResult:
        """辨識並翻譯一張截圖；`rect` 只用來記 log。
        設定強制走本機 OCR 時直接跳過圖片路徑，且不標記 `text_only`
        （那是端點被證實不吃圖片才留的記號，這裡只是使用者的選擇）。"""
        started = time.monotonic()
        image_error = None
        if self._force_ocr():
            log("[region] image path skipped (forced local OCR)")
        elif not self._text_only:
            try:
                original, text = self._translator.translate_region_image(png)
                log(f"[region] done in {time.monotonic() - started:.1f}s via image "
                    f"(rect={rect}, chars={len(text)})")
                return RegionResult(text, "image", original)
            except TranslatorConfigError as exc:
                image_error = exc
                log(f"[region] image input rejected (status={exc.status}, "
                    f"detail={exc.detail!r}); falling back to local OCR")
            except TranslatorOffline as exc:
                log(f"[region] image request failed (status={exc.status}, "
                    f"detail={exc.detail!r}); trying local OCR without marking text-only")
        source = self._recognize(png)
        if not source:
            log(f"[region] local OCR found no text (rect={rect})")
            return RegionResult("", "ocr")
        text = self._translator.translate_region_text(source)
        if image_error is not None:
            self._text_only = True
            log("[region] endpoint marked text-only: text translation succeeded after "
                "the image request was rejected")
        log(f"[region] done in {time.monotonic() - started:.1f}s via ocr "
            f"(rect={rect}, source_chars={len(source)}, chars={len(text)})")
        return RegionResult(text, "ocr", source)
