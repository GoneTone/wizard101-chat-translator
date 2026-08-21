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


def test_exact_unbounded_never_re_emits():
    # 記憶體讀取模式:精確比對、不設上限 —— 看過的行永不重現(即使幾百行)
    d = LineDeduper(max_seen=None, similarity=1.0)
    lines = [f"[P] m{i}" for i in range(300)]
    assert d.new_lines(lines) == lines
    assert d.new_lines(lines) == []


def test_exact_mode_is_exact_not_fuzzy():
    d = LineDeduper(max_seen=None, similarity=1.0)
    d.new_lines(["[P] hello world"])
    # 精確模式:差一字元即視為不同(不像模糊模式會吸收)
    assert d.new_lines(["[P] hello worla"]) == ["[P] hello worla"]


def test_exact_forget_allows_retry():
    d = LineDeduper(max_seen=None, similarity=1.0)
    d.new_lines(["[P] a", "[P] b"])
    d.forget(["[P] a"])
    assert d.new_lines(["[P] a", "[P] b"]) == ["[P] a"]


def test_forget_makes_line_new_again():
    d = LineDeduper()
    d.new_lines(["hello there"])
    d.forget(["hello there"])
    assert d.new_lines(["hello there"]) == ["hello there"]


def test_forget_unseen_line_is_noop():
    d = LineDeduper()
    d.new_lines(["hello there"])
    d.forget(["never seen this line"])
    # 原本已見過的行不受影響,仍視為重複
    assert d.new_lines(["hello there"]) == []


def test_forget_only_removes_listed_lines():
    d = LineDeduper()
    d.new_lines(["line a", "line b"])
    d.forget(["line a"])
    assert d.new_lines(["line a", "line b"]) == ["line a"]
