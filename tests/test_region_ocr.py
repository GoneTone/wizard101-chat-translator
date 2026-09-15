"""本機 OCR：引擎語言挑選（純函式）與一次真實辨識（沒有引擎就跳過）。"""
import io

import pytest

from src.region.ocr import OcrUnavailable, pick_language, recognize


def test_pick_language_prefers_the_matching_primary_tag():
    assert pick_language(["zh-Hant-TW", "en-US", "en-GB"], "en") == "en-US"


def test_pick_language_ignores_case():
    assert pick_language(["EN-us"], "en") == "EN-us"


def test_pick_language_returns_none_when_nothing_matches():
    assert pick_language(["zh-Hant-TW", "ja-JP"], "en") is None
    assert pick_language([], "en") is None


def _rendered(text: str) -> bytes:
    from PIL import Image, ImageDraw, ImageFont
    image = Image.new("RGB", (640, 120), "white")
    ImageDraw.Draw(image).text((20, 30), text, fill="black",
                               font=ImageFont.load_default(size=40))
    buffer = io.BytesIO()
    image.save(buffer, "PNG")
    return buffer.getvalue()


def test_recognize_reads_rendered_text():
    try:
        text = recognize(_rendered("Hello Wizard"))
    except OcrUnavailable as exc:
        pytest.skip(f"no local OCR engine: {exc}")
    assert "hello" in text.lower().replace(" ", "")


def test_recognize_reads_text_on_a_transparent_background():
    # 背景用 (255, 255, 255, 0)（全透明、底色白）而非 (0, 0, 0, 0)：後者的 RGB
    # 與黑色文字完全相同，不論 alpha 怎麼處理，像素資料本身就無法分辨文字與背景，
    # 測不出 BGRA8 轉換的迴歸——這裡要驗證的是「有 alpha 通道的 PNG 不會讓引擎
    # 丟原生錯誤、且真的能讀到字」。
    from PIL import Image, ImageDraw, ImageFont
    image = Image.new("RGBA", (640, 120), (255, 255, 255, 0))
    ImageDraw.Draw(image).text((20, 30), "Hello Wizard", fill=(0, 0, 0, 255),
                               font=ImageFont.load_default(size=40))
    buffer = io.BytesIO()
    image.save(buffer, "PNG")
    try:
        text = recognize(buffer.getvalue())
    except OcrUnavailable as exc:
        pytest.skip(f"no local OCR engine: {exc}")
    assert "hello" in text.lower().replace(" ", "")


def test_recognize_returns_empty_for_a_blank_image():
    from PIL import Image
    buffer = io.BytesIO()
    Image.new("RGB", (200, 80), "white").save(buffer, "PNG")
    try:
        assert recognize(buffer.getvalue()) == ""
    except OcrUnavailable as exc:
        pytest.skip(f"no local OCR engine: {exc}")
