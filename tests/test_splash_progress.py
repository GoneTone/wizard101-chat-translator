"""啟動畫面進度條：位元組加權對照表的計算，以及 patch PyInstaller 樣板的行為。

不需要真的跑 PyInstaller —— 這裡只驗證 `tools/splash_progress.py` 自己的邏輯
（對照表、Tcl 片段格式、錨點檢查、重複呼叫不疊加、撞名清單彈出）。Tcl 腳本實際
長什麼樣，build 一次後開生成的 `*_script.tcl` 人工確認。
"""
import tkinter
from itertools import chain

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


def _make_entries(tmp_path, sizes: dict[str, int]) -> list[tuple[str, str, str]]:
    """建立假的 `Analysis.binaries`／`Analysis.datas` entry：
    `(dest_name, src_path, typecode)`。兩者的 TOC entry 形狀相同，測試共用一個
    builder 即可。"""
    entries = []
    for dest_name, size in sizes.items():
        src = tmp_path / dest_name.replace("/", "_")
        src.write_bytes(b"x" * size)
        entries.append((dest_name, str(src), "BINARY"))
    return entries


# ---------------------------------------------------------------------------
# _size_table
# ---------------------------------------------------------------------------


def test_size_table_maps_lowercased_basename_to_byte_size_list(tmp_path):
    entries = _make_entries(tmp_path, {"cv2/CV2.PYD": 100, "onnxruntime.dll": 50})
    table = splash_progress._size_table(entries)
    assert table == {"cv2.pyd": [100], "onnxruntime.dll": [50]}


def test_size_table_skips_entries_whose_source_is_unreadable(tmp_path):
    entries = [("missing.dll", str(tmp_path / "does_not_exist.dll"), "BINARY")]
    assert splash_progress._size_table(entries) == {}


def test_size_table_skips_basenames_with_tcl_brace_characters(tmp_path):
    """大括號在 Tcl 的 array set 清單語法裡有特殊意義，這種檔名塞不進去；略過即可，
    不該讓整個 build 因為一個異常檔名而失敗。"""
    entries = _make_entries(tmp_path, {"weird{name}.dll": 100, "normal.dll": 50})
    assert splash_progress._size_table(entries) == {"normal.dll": [50]}


def test_size_table_keeps_both_sizes_on_basename_collision(tmp_path):
    """兩個不同子目錄下同名的檔案：bootloader 回報的 status_text 只有檔名，區分
    不出是哪一份，但兩份都會各自觸發一次解壓事件，兩個大小都要留著、不能互相蓋掉
    （撞名實測涵蓋 1121 個檔案裡的 174 個，全是幾 KB 的 dist-info 中繼資料，但機制
    本身不該假設「撞名的都是小檔案」）。"""
    entries = _make_entries(tmp_path, {"a/dup.dll": 10, "b/dup.dll": 20})
    table = splash_progress._size_table(entries)
    assert sorted(table["dup.dll"]) == [10, 20]


def test_size_table_combines_binaries_and_datas(tmp_path):
    """`install_progress_bar` 的呼叫端把 binaries／datas 兩份清單串在一起餵進來，
    `_size_table` 本身不區分來源。"""
    binaries = _make_entries(tmp_path, {"cv2.pyd": 100})
    datas = _make_entries(tmp_path, {"rapidocr/models/det.onnx": 200})
    table = splash_progress._size_table(chain(binaries, datas))
    assert table == {"cv2.pyd": [100], "det.onnx": [200]}


# ---------------------------------------------------------------------------
# _size_table_tcl
# ---------------------------------------------------------------------------


def test_size_table_tcl_braces_each_basename_and_its_size_list():
    tcl = splash_progress._size_table_tcl({"a.dll": [10], "name with space.pyd": [20, 30]})
    assert tcl == "{a.dll} {10} {name with space.pyd} {20 30}"


# ---------------------------------------------------------------------------
# install_progress_bar
# ---------------------------------------------------------------------------


def test_install_progress_bar_returns_the_total_bytes_across_binaries_and_datas(tmp_path):
    binaries = _make_entries(tmp_path, {"cv2.pyd": 1000, "small.dll": 234})
    datas = _make_entries(tmp_path, {"rapidocr/models/det.onnx": 500})
    total = splash_progress.install_progress_bar(binaries, datas)
    assert total == 1734


def test_install_progress_bar_injects_the_size_table_and_bar_elements(tmp_path):
    binaries = _make_entries(tmp_path, {"cv2.pyd": 1000})
    splash_progress.install_progress_bar(binaries, [])

    canvas_setup = splash_templates.splash_canvas_setup
    assert "set _pyi_total 1000" in canvas_setup
    assert "array set _pyi_sizes {{cv2.pyd} {1000}}" in canvas_setup
    assert "-tag pyi_track" in canvas_setup
    assert "-tag pyi_fill" in canvas_setup

    text_update = splash_templates.image_script
    # 文字更新那行原樣保留在最前面 —— 使用者要求檔名照舊顯示，不能被拿掉
    assert "$canvas itemconfigure $tag -text $var\n    global _pyi_done" in text_update
    assert "info exists _pyi_sizes($_pyi_key)" in text_update
    assert "lindex $_pyi_list 0" in text_update
    assert "$canvas coords pyi_fill" in text_update


def test_install_progress_bar_raises_if_nothing_has_a_readable_size():
    with pytest.raises(ValueError, match="no binaries/datas with a readable size"):
        splash_progress.install_progress_bar([], [])


def test_install_progress_bar_raises_if_canvas_anchor_is_missing(tmp_path, monkeypatch):
    monkeypatch.setattr(splash_templates, "splash_canvas_setup", "no such anchor here")
    binaries = _make_entries(tmp_path, {"cv2.pyd": 1000})
    with pytest.raises(RuntimeError, match="splash_canvas_setup anchor not found"):
        splash_progress.install_progress_bar(binaries, [])


def test_install_progress_bar_raises_if_text_update_anchor_is_missing(tmp_path, monkeypatch):
    monkeypatch.setattr(splash_templates, "image_script", "no such anchor here")
    binaries = _make_entries(tmp_path, {"cv2.pyd": 1000})
    with pytest.raises(RuntimeError, match="canvas_text_update anchor not found"):
        splash_progress.install_progress_bar(binaries, [])


def test_install_progress_bar_does_not_double_inject_on_repeated_calls(tmp_path):
    binaries = _make_entries(tmp_path, {"cv2.pyd": 1000})

    first = splash_progress.install_progress_bar(binaries, [])
    second = splash_progress.install_progress_bar(binaries, [])

    assert first == second == 1000
    assert splash_templates.splash_canvas_setup.count("array set _pyi_sizes") == 1
    assert splash_templates.splash_canvas_setup.count("-tag pyi_fill") == 1
    assert splash_templates.image_script.count("global _pyi_done") == 1


# ---------------------------------------------------------------------------
# 用真正的 Tcl 直譯器驗證清單彈出邏輯（純字串比對測不出 Tcl 語法本身寫錯的地方）
# ---------------------------------------------------------------------------


def test_progress_tracking_pops_one_size_per_report_in_a_real_tcl_interpreter():
    """`tkinter.Tcl()` 給一個沒有視窗、沒有畫布的純 Tcl 直譯器 —— 用假的
    `.root.canvas` proc 吞掉 `itemconfigure`／`coords` 呼叫，藉此在不建立任何 Tk
    視窗的情況下，實際執行 `_text_update_addition()` 產生的 Tcl，驗證：
    (1) 撞名的兩個大小都會被算到、彼此獨立彈出；
    (2) basename 比對真的有轉小寫；
    (3) 清單彈完才 `unset` 該 key。
    """
    table = {"dup.dll": [10, 20], "solo.pyd": [5]}
    total = sum(size for sizes in table.values() for size in sizes)

    interp = tkinter.Tcl()
    interp.eval("proc .root.canvas {args} {}")
    interp.eval(f"set _pyi_total {total}")
    interp.eval(f"array set _pyi_sizes {{{splash_progress._size_table_tcl(table)}}}")
    interp.eval("set _pyi_done 0")
    proc_body = (
        "upvar $_var var\n"
        "$canvas itemconfigure $tag -text $var\n" + splash_progress._text_update_addition()
    )
    interp.eval(f"proc canvas_text_update {{canvas tag _var}} {{{proc_body}}}")

    interp.eval("set status_text dup.dll")
    interp.eval("canvas_text_update .root.canvas vartext status_text")
    interp.eval("set status_text DUP.DLL")   # 同一個 key 第二次報到，basename 要轉小寫比對
    interp.eval("canvas_text_update .root.canvas vartext status_text")
    interp.eval("set status_text solo.pyd")
    interp.eval("canvas_text_update .root.canvas vartext status_text")

    assert int(interp.eval("set _pyi_done")) == total
    assert interp.eval("array names _pyi_sizes") == ""
