"""本機 OCR 退路：Windows 內建的 Windows.Media.Ocr，PNG bytes → 文字。

只在翻譯後端不吃圖片時使用（見 region.pipeline）。Windows OCR 是逐語言的引擎，
辨識語言優先挑遊戲語言（prompts.OUTGOING_LANGUAGE_TAG）：實測用使用者介面語言的引擎
辨識遊戲文字會把空格全部吃掉（「HelloWizard」）。winrt 模組延遲到第一次呼叫才載入：
套件缺了只讓這條退路不可用，不影響程式啟動。
"""
import asyncio

from src.log import log
from src.translation.prompts import OUTGOING_LANGUAGE_TAG


class OcrUnavailable(Exception):
    """本機 OCR 不可用：winrt 套件載入失敗，或 Windows 沒有可用的 OCR 語言包。"""


_announced = False   # 引擎語言只在第一次成功時記一行，之後每次框選不重複


def pick_language(tags: list[str], preferred: str) -> str | None:
    """從可用的辨識語言標籤挑一個：主標籤與 preferred 相符（`en` 對 `en-US`）的第一個，
    沒有回 None（呼叫端改用使用者設定檔語言）。"""
    wanted = preferred.casefold()
    for tag in tags:
        if tag.casefold().split("-")[0] == wanted:
            return tag
    return None


def _winrt():
    try:
        from winrt.windows.graphics.imaging import BitmapAlphaMode, BitmapDecoder, BitmapPixelFormat
        from winrt.windows.media.ocr import OcrEngine
        from winrt.windows.storage.streams import DataWriter, InMemoryRandomAccessStream
    except ImportError as exc:
        raise OcrUnavailable(f"winrt OCR modules unavailable: {exc}") from exc
    return (OcrEngine, BitmapDecoder, BitmapPixelFormat, BitmapAlphaMode,
            DataWriter, InMemoryRandomAccessStream)


def _create_engine(OcrEngine):
    global _announced
    languages = list(OcrEngine.available_recognizer_languages)
    tags = [language.language_tag for language in languages]
    chosen = pick_language(tags, OUTGOING_LANGUAGE_TAG)
    if chosen is not None:
        engine = OcrEngine.try_create_from_language(languages[tags.index(chosen)])
    else:
        engine = OcrEngine.try_create_from_user_profile_languages()
    if engine is None:
        raise OcrUnavailable(f"no OCR language pack available (installed={tags})")
    if not _announced:
        _announced = True
        log(f"[region] local OCR engine ready "
            f"(language={engine.recognizer_language.language_tag}, installed={tags})")
    return engine


def recognize(png: bytes) -> str:
    """辨識 PNG 裡的文字，各行以換行合併；沒有文字回空字串。
    引擎每次重建（很便宜），不跨執行緒共用 WinRT 物件。"""
    (OcrEngine, BitmapDecoder, BitmapPixelFormat, BitmapAlphaMode,
     DataWriter, InMemoryRandomAccessStream) = _winrt()
    engine = _create_engine(OcrEngine)

    async def run() -> str:
        stream = InMemoryRandomAccessStream()
        writer = DataWriter(stream)
        writer.write_bytes(png)
        await writer.store_async()
        await writer.flush_async()
        stream.seek(0)
        decoder = await BitmapDecoder.create_async(stream)
        # 引擎只吃 BGRA8；PNG 解出來的原生格式可能是索引色／灰階，直接丟給
        # recognize_async 在某些解碼路徑會拋 WinRT 原生錯誤，所以一律轉換。alpha
        # 模式選 STRAIGHT、不選 PREMULTIPLIED：後者對 alpha=0 的像素一律把 RGB
        # 乘成全黑，若透明區域底色恰好較深、文字又是深色，轉換後兩者顏色會疊在一起
        # 讓引擎讀不到字（實測驗證過）；STRAIGHT 保留原始 RGB，不會有這個問題。
        bitmap = await decoder.get_software_bitmap_converted_async(
            BitmapPixelFormat.BGRA8, BitmapAlphaMode.STRAIGHT)
        result = await engine.recognize_async(bitmap)
        return "\n".join(line.text for line in result.lines).strip()

    return asyncio.run(run())
