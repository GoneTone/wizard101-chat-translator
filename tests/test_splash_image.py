"""啟動畫面底圖：build 時生成，尺寸、格式與版本號都要釘住。"""
from PIL import Image

from tools.splash_image import SIZE, build_splash_image


def test_generates_a_png_of_the_expected_size(tmp_path):
    dest = build_splash_image(tmp_path / "splash.png")
    assert dest.is_file()
    with Image.open(dest) as im:
        assert im.format == "PNG"
        assert im.size == SIZE


def test_creates_missing_parent_directories(tmp_path):
    """spec 會把它寫到 build/ 底下，乾淨的 checkout 上那個目錄還不存在。"""
    dest = build_splash_image(tmp_path / "nested" / "dir" / "splash.png")
    assert dest.is_file()


def test_version_is_drawn_into_the_image(tmp_path, monkeypatch):
    """版本號必須真的畫進去 —— 否則改版後圖上會留著舊版號，沒人看得出來。"""
    first = build_splash_image(tmp_path / "a.png").read_bytes()
    monkeypatch.setattr("tools.splash_image.__version__", "9.9.9")
    second = build_splash_image(tmp_path / "b.png").read_bytes()
    assert first != second, "版本號沒有畫進圖裡"
