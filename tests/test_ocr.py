from PIL import Image

from src.reader.ocr import _lines_in_order, binarize


def fake_line(text: str, y: int) -> dict:
    return {"text": text, "words": [{"text": text, "bounding_rect": {"x": 0, "y": y, "width": 50, "height": 12}}]}


def test_lines_sorted_by_y():
    result = {"lines": [fake_line("second", 40), fake_line("first", 10), fake_line("third", 90)]}
    assert _lines_in_order(result) == ["first", "second", "third"]


def test_empty_result_gives_empty_list():
    assert _lines_in_order({}) == []
    assert _lines_in_order({"lines": []}) == []


def test_line_without_words_defaults_to_top():
    result = {"lines": [fake_line("late", 50), {"text": "no-words-line", "words": []}]}
    assert _lines_in_order(result) == ["no-words-line", "late"]


def test_binarize_bright_text_becomes_black_on_white():
    # 遊戲聊天是白字壓在雜訊背景上:亮像素(文字)→ 黑,其餘(背景)→ 白
    img = Image.new("RGB", (2, 1))
    img.putpixel((0, 0), (255, 255, 255))  # 白色文字像素
    img.putpixel((1, 0), (120, 100, 80))   # 石板路背景
    bw = binarize(img)
    assert bw.mode == "RGB"
    assert bw.getpixel((0, 0)) == (0, 0, 0)
    assert bw.getpixel((1, 0)) == (255, 255, 255)


def test_binarize_threshold_boundary():
    img = Image.new("L", (2, 1))
    img.putpixel((0, 0), 200)  # 恰在閾值上 → 視為文字
    img.putpixel((1, 0), 199)  # 低於閾值 → 背景
    bw = binarize(img, threshold=200)
    assert bw.getpixel((0, 0)) == (0, 0, 0)
    assert bw.getpixel((1, 0)) == (255, 255, 255)
