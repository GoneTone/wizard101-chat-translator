from src.reader.dedup import LineDeduper


def test_first_batch_all_new():
    d = LineDeduper()
    assert d.new_lines(["hello there", "anyone selling?"]) == ["hello there", "anyone selling?"]


def test_repeat_batch_returns_nothing():
    d = LineDeduper()
    d.new_lines(["hello there"])
    assert d.new_lines(["hello there"]) == []


def test_ocr_jitter_is_same_line():
    # OCR 抖動:l/I、少一個空格等,相似度 >= 0.9 應視為同一行
    d = LineDeduper()
    d.new_lines(["Player: want to trade my hat?"])
    assert d.new_lines(["Player: want to trade my hatl"]) == []


def test_genuinely_different_line_passes():
    d = LineDeduper()
    d.new_lines(["Player: hi"])
    assert d.new_lines(["Player: where are you from?"]) == ["Player: where are you from?"]


def test_blank_and_whitespace_lines_dropped():
    d = LineDeduper()
    assert d.new_lines(["", "   ", "real line"]) == ["real line"]


def test_duplicate_within_same_batch_kept_once():
    d = LineDeduper()
    assert d.new_lines(["same msg", "same msg"]) == ["same msg"]


def test_seen_set_is_bounded():
    d = LineDeduper(max_seen=2)
    d.new_lines(["line one hello", "line two world", "line three again"])
    # "line one hello" 已被擠出視窗,重新出現時視為新行
    assert d.new_lines(["line one hello"]) == ["line one hello"]
