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


def test_recognize_returns_empty_for_a_blank_image():
    from PIL import Image
    buffer = io.BytesIO()
    Image.new("RGB", (200, 80), "white").save(buffer, "PNG")
    try:
        assert recognize(buffer.getvalue()) == ""
    except OcrUnavailable as exc:
        pytest.skip(f"no local OCR engine: {exc}")
