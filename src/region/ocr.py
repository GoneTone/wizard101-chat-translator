"""本機 OCR 退路：Windows 內建的 Windows.Media.Ocr，PNG bytes → 文字。

只在翻譯後端不吃圖片時使用（見 region.pipeline）。Windows OCR 是逐語言的引擎，
辨識語言優先挑遊戲語言（prompts.OUTGOING_LANGUAGE_TAG）：實測用使用者介面語言的引擎
辨識遊戲文字會把空格全部吃掉（「HelloWizard」）。winrt 模組延遲到第一次呼叫才載入：
套件缺了只讓這條退路不可用，不影響程式啟動。
"""
import asyncio
import io

from PIL import Image

from src.log import log
from src.translation.prompts import OUTGOING_LANGUAGE_TAG

# 自適應縮放（與 PowerToys 文字擷取同一套做法）：Windows OCR 對字高約 40 px 的文字最準，
# 太小會掉字、放太大會糊成別的字（實測：一律放大 2 倍救回了商店標題，卻把另一行
# 較大的字讀壞）。先原尺寸辨識一次量出平均字高，再縮放到理想字高重跑一次。
IDEAL_WORD_HEIGHT = 40.0
MIN_SCALE, MAX_SCALE = 0.5, 4.0
RESCALE_THRESHOLD = 0.1   # 倍率與 1 差不到這麼多就不重跑


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


def ideal_scale(word_heights: list[float], ideal: float = IDEAL_WORD_HEIGHT) -> float:
    """依第一次辨識量到的字高算出第二次辨識的縮放倍率（夾在 MIN_SCALE～MAX_SCALE）；
    沒有量到任何字回 1.0（沒東西可據以縮放）。"""
    if not word_heights:
        return 1.0
    average = sum(word_heights) / len(word_heights)
    if average <= 0:
        return 1.0
    return max(MIN_SCALE, min(MAX_SCALE, ideal / average))


def scaled_png(png: bytes, factor: float) -> bytes:
    """把 PNG 依 factor 縮放（LANCZOS）後重新編碼。"""
    # 先丟掉 alpha：Pillow 縮放 RGBA 會用預乘 alpha，透明區的 RGB 被壓成全黑，
    # 黑字就跟背景混在一起；辨識時本來就忽略 alpha（見 recognize），轉 RGB 語意一致
    image = Image.open(io.BytesIO(png)).convert("RGB")
    size = (max(1, round(image.width * factor)), max(1, round(image.height * factor)))
    buffer = io.BytesIO()
    image.resize(size, Image.LANCZOS).save(buffer, "PNG")
    return buffer.getvalue()


def recognize(png: bytes) -> str:
    """辨識 PNG 裡的文字，各行以換行合併；沒有文字回空字串。
    先原尺寸辨識量字高，倍率離 1 夠遠就縮放後再辨識一次、以第二次為準（見 ideal_scale）。
    引擎每次重建（很便宜），不跨執行緒共用 WinRT 物件。"""
    (OcrEngine, BitmapDecoder, BitmapPixelFormat, BitmapAlphaMode,
     DataWriter, InMemoryRandomAccessStream) = _winrt()
    engine = _create_engine(OcrEngine)

    async def run(data: bytes):
        stream = InMemoryRandomAccessStream()
        writer = DataWriter(stream)
        writer.write_bytes(data)
        await writer.store_async()
        await writer.flush_async()
        stream.seek(0)
        decoder = await BitmapDecoder.create_async(stream)
        # 引擎只接受 BGRA8、alpha 模式為 premultiplied 或 ignore 兩種（微軟官方文件
        # 列的支援範圍）；PNG 解出來的原生格式可能是索引色／灰階，直接丟給
        # recognize_async 在某些解碼路徑會拋 WinRT 原生錯誤，所以一律轉換。alpha
        # 模式選 ignore、不選 premultiplied：後者對 alpha=0 的像素一律把 RGB 乘成
        # 全黑，若透明區域底色恰好較深、文字又是深色，轉換後兩者顏色會疊在一起讓
        # 引擎讀不到字（實測驗證過）；ignore 保留原始 RGB 不做任何相乘，色彩不失真，
        # 而且擷取流程（region.capture）產出的 PNG 本來就是不透明 RGB，語意上也沒有
        # 需要合成的 alpha 通道。
        bitmap = await decoder.get_software_bitmap_converted_async(
            BitmapPixelFormat.BGRA8, BitmapAlphaMode.IGNORE)
        return await engine.recognize_async(bitmap)

    async def both() -> str:
        result = await run(png)
        heights = [word.bounding_rect.height for line in result.lines for word in line.words]
        factor = ideal_scale(heights)
        if abs(factor - 1.0) >= RESCALE_THRESHOLD:
            log(f"[region] local OCR rescale (words={len(heights)}, factor={factor:.2f})")
            result = await run(scaled_png(png, factor))
        return "\n".join(line.text for line in result.lines).strip()

    return asyncio.run(both())
