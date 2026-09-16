"""啟動畫面進度條：位元組加權對照表的計算，以及 patch PyInstaller 樣板的行為。

不需要真的跑 PyInstaller —— 這裡只驗證 `tools/splash_progress.py` 自己的邏輯
（對照表、Tcl 片段格式、錨點檢查、重複呼叫不疊加）。Tcl 腳本實際長什麼樣，
build 一次後開 `Wizard101ChatTranslator_script.tcl` 人工確認。
"""
import pytest
from PyInstaller.building import splash_templates

from tools import splash_progress

# 測試開始前的原始樣板內容，每個測試前都重置回這份，避免測試之間互相污染
# （splash_templates 是模組層級的可變狀態，install_progress_bar 會直接改寫它）。
_ORIGINAL_CANVAS_SETUP = splash_templates.splash_canvas_setup
_ORIGINAL_IMAGE_SCRIPT = splash_templates.image_script


@pytest.fixture(autouse=True)
def _reset_templates(monkeypatch):
    monkeypatch.setattr(splash_templates, "splash_canvas_setup", _ORIGINAL_CANVAS_SETUP)
    monkeypatch.setattr(splash_templates, "image_script", _ORIGINAL_IMAGE_SCRIPT)


def _make_binaries(tmp_path, sizes: dict[str, int]) -> list[tuple[str, str, str]]:
    """建立假的 `Analysis.binaries` entry：`(dest_name, src_path, typecode)`。"""
    entries = []
    for dest_name, size in sizes.items():
        src = tmp_path / dest_name.replace("/", "_")
        src.write_bytes(b"x" * size)
        entries.append((dest_name, str(src), "BINARY"))
    return entries


# ---------------------------------------------------------------------------
# _size_table
# ---------------------------------------------------------------------------


def test_size_table_maps_lowercased_basename_to_byte_size(tmp_path):
    binaries = _make_binaries(tmp_path, {"cv2/CV2.PYD": 100, "onnxruntime.dll": 50})
    table = splash_progress._size_table(binaries)
    assert table == {"cv2.pyd": 100, "onnxruntime.dll": 50}


def test_size_table_skips_entries_whose_source_is_unreadable(tmp_path):
    binaries = [("missing.dll", str(tmp_path / "does_not_exist.dll"), "BINARY")]
    assert splash_progress._size_table(binaries) == {}


def test_size_table_last_entry_wins_on_basename_collision(tmp_path):
    """兩個不同子目錄下同名的檔案：bootloader 回報的 status_text 只有檔名，
    區分不出是哪一份，後面的覆蓋前面的是刻意行為，不是 bug。"""
    binaries = _make_binaries(tmp_path, {"a/dup.dll": 10, "b/dup.dll": 20})
    assert splash_progress._size_table(binaries) == {"dup.dll": 20}


# ---------------------------------------------------------------------------
# _size_table_tcl
# ---------------------------------------------------------------------------


def test_size_table_tcl_braces_each_basename():
    tcl = splash_progress._size_table_tcl({"a.dll": 10, "name with space.pyd": 20})
    assert tcl == "{a.dll} 10 {name with space.pyd} 20"


# ---------------------------------------------------------------------------
# install_progress_bar
# ---------------------------------------------------------------------------


def test_install_progress_bar_returns_the_total_bytes(tmp_path):
    binaries = _make_binaries(tmp_path, {"cv2.pyd": 1000, "small.dll": 234})
    total = splash_progress.install_progress_bar(binaries)
    assert total == 1234


def test_install_progress_bar_injects_the_size_table_and_bar_elements(tmp_path):
    binaries = _make_binaries(tmp_path, {"cv2.pyd": 1000})
    splash_progress.install_progress_bar(binaries)

    canvas_setup = splash_templates.splash_canvas_setup
    assert "set _pyi_total 1000" in canvas_setup
    assert "array set _pyi_sizes {{cv2.pyd} 1000}" in canvas_setup
    assert "-tag pyi_track" in canvas_setup
    assert "-tag pyi_fill" in canvas_setup

    text_update = splash_templates.image_script
    # 文字更新那行原樣保留在最前面 —— 使用者要求檔名照舊顯示，不能被拿掉
    assert "$canvas itemconfigure $tag -text $var\n    global _pyi_done" in text_update
    assert "info exists _pyi_sizes($_pyi_key)" in text_update
    assert "$canvas coords pyi_fill" in text_update


def test_install_progress_bar_raises_if_no_binaries_have_a_readable_size():
    with pytest.raises(ValueError, match="no binaries with a readable size"):
        splash_progress.install_progress_bar([])


def test_install_progress_bar_raises_if_canvas_anchor_is_missing(tmp_path, monkeypatch):
    monkeypatch.setattr(splash_templates, "splash_canvas_setup", "no such anchor here")
    binaries = _make_binaries(tmp_path, {"cv2.pyd": 1000})
    with pytest.raises(RuntimeError, match="splash_canvas_setup anchor not found"):
        splash_progress.install_progress_bar(binaries)


def test_install_progress_bar_raises_if_text_update_anchor_is_missing(tmp_path, monkeypatch):
    monkeypatch.setattr(splash_templates, "image_script", "no such anchor here")
    binaries = _make_binaries(tmp_path, {"cv2.pyd": 1000})
    with pytest.raises(RuntimeError, match="canvas_text_update anchor not found"):
        splash_progress.install_progress_bar(binaries)


def test_install_progress_bar_does_not_double_inject_on_repeated_calls(tmp_path):
    binaries = _make_binaries(tmp_path, {"cv2.pyd": 1000})

    first = splash_progress.install_progress_bar(binaries)
    second = splash_progress.install_progress_bar(binaries)

    assert first == second == 1000
    assert splash_templates.splash_canvas_setup.count("array set _pyi_sizes") == 1
    assert splash_templates.splash_canvas_setup.count("-tag pyi_fill") == 1
    assert splash_templates.image_script.count("global _pyi_done") == 1
