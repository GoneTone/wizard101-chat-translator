from src.reader.ocr import _lines_in_order


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
