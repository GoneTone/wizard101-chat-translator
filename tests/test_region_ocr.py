"""本機 OCR：`merge_lines` 純函式與一次真實辨識（沒有引擎就跳過）。"""
import io

import pytest

from src.region.ocr import OcrUnavailable, merge_lines, recognize


def _box(x0, y0, x1, y1):
    return [[x0, y0], [x1, y0], [x1, y1], [x0, y1]]


def test_merge_lines_joins_boxes_on_one_visual_line():
    items = [
        (_box(18, 33, 97, 66), "Items"),
        (_box(480, 32, 620, 68), "Wizard"),
        (_box(146, 32, 400, 68), "Recommended For Your"),
    ]
    assert merge_lines(items) == ["Items Recommended For Your Wizard"]


def test_merge_lines_orders_lines_top_to_bottom():
    items = [
        (_box(10, 200, 100, 240), "second"),
        (_box(10, 20, 100, 60), "first"),
    ]
    assert merge_lines(items) == ["first", "second"]


def test_merge_lines_returns_empty_list_for_no_boxes():
    assert merge_lines([]) == []


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
    # 測不出「丟掉 alpha」這一步的迴歸 —— 這裡要驗證的是有 alpha 通道的 PNG
    # 一樣能正確轉成 RGB 再辨識。
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
