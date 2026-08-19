from src.region_picker import _to_region


def test_normal_drag_topleft_to_bottomright():
    assert _to_region(10, 20, 110, 80) == {"left": 10, "top": 20, "width": 100, "height": 60}


def test_reverse_drag_normalized():
    assert _to_region(110, 80, 10, 20) == {"left": 10, "top": 20, "width": 100, "height": 60}


def test_minimum_size_enforced():
    # 誤點一下(幾乎零面積)也至少給 1x1,避免 mss 丟例外
    r = _to_region(50, 50, 50, 50)
    assert r["width"] >= 1 and r["height"] >= 1
