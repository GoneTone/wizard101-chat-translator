"""啟動畫面進度條：位元組加權對照表的計算，以及 patch PyInstaller 樣板的行為。

不需要真的跑 PyInstaller —— 這裡只驗證 `tools/splash_progress.py` 自己的邏輯
（對照表、Tcl 片段格式、錨點檢查、重複呼叫不疊加、撞名清單彈出、解壓上限、階段
目標、緩動動畫）。Tcl 腳本實際長什麼樣，build 一次後開生成的 `*_script.tcl` 人工
確認。
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


def _tcl_interpreter_with_progress_state(total: int, table: dict[str, list[int]]) -> tkinter.Tcl:
    """準備一個沒有視窗的純 Tcl 直譯器：假的 `.root.canvas` proc 吞掉畫布呼叫，讓
    `_canvas_setup_addition()`／`_text_update_addition()` 產生的 Tcl 真的被執行
    （含 `pyi_progress_step`），而不只是字串比對。"""
    interp = tkinter.Tcl()
    interp.eval("proc .root.canvas {args} {}")
    interp.eval(splash_progress._canvas_setup_addition(total, table))
    proc_body = (
        "upvar $_var var\n"
        "$canvas itemconfigure $tag -text $var\n" + splash_progress._text_update_addition()
    )
    interp.eval(f"proc canvas_text_update {{canvas tag _var}} {{{proc_body}}}")
    return interp


def _report(interp: tkinter.Tcl, text: str) -> None:
    """模擬 bootloader 對 `status_text` 的一次寫入，觸發 `canvas_text_update`。"""
    interp.eval(f"set status_text {{{text}}}")
    interp.eval("canvas_text_update .root.canvas vartext status_text")


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
    """兩個子目錄下同名的檔案都會各自觸發一次解壓事件，兩個大小都要留著、不能互相
    蓋掉（不能假設撞名的都是小檔案）。"""
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
    assert "proc pyi_progress_step {}" in canvas_setup
    assert ".root.canvas coords pyi_fill" in canvas_setup

    text_update = splash_templates.image_script
    # 文字更新那行原樣保留在最前面 —— 使用者要求檔名照舊顯示，不能被拿掉
    assert "$canvas itemconfigure $tag -text $var\n    global _pyi_done" in text_update
    assert "info exists _pyi_sizes($_pyi_key)" in text_update
    assert "lindex $_pyi_list 0" in text_update
    assert "pyi_progress_step" in text_update
    # 兩個階段字串要精確比對，不能被 basename 那條路徑吃掉
    assert "$var eq {Loading components...}" in text_update
    assert "$var eq {Starting...}" in text_update


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


def test_install_progress_bar_raises_if_canvas_anchor_appears_twice(tmp_path, monkeypatch):
    """Unbounded replace would silently inject the addition twice; exact-once assertion
    catches template drift before it ships a broken progress bar."""
    canvas_setup_with_duplicate = _ORIGINAL_CANVAS_SETUP + "\n" + splash_progress._CANVAS_ANCHOR
    monkeypatch.setattr(splash_templates, "splash_canvas_setup", canvas_setup_with_duplicate)
    binaries = _make_entries(tmp_path, {"cv2.pyd": 1000})
    with pytest.raises(RuntimeError, match="splash_canvas_setup anchor found 2 times"):
        splash_progress.install_progress_bar(binaries, [])


def test_install_progress_bar_raises_if_text_update_anchor_appears_twice(tmp_path, monkeypatch):
    """Unbounded replace would silently inject the addition twice; exact-once assertion
    catches template drift before it ships a broken progress bar."""
    image_script_with_duplicate = _ORIGINAL_IMAGE_SCRIPT + "\n" + splash_progress._TEXT_UPDATE_ANCHOR
    monkeypatch.setattr(splash_templates, "image_script", image_script_with_duplicate)
    binaries = _make_entries(tmp_path, {"cv2.pyd": 1000})
    with pytest.raises(RuntimeError, match="canvas_text_update anchor found 2 times"):
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
# 用真正的 Tcl 直譯器驗證清單彈出邏輯、解壓上限、階段目標、緩動動畫
# （純字串比對測不出 Tcl 語法本身寫錯的地方）
# ---------------------------------------------------------------------------


def test_progress_tracking_pops_one_size_per_report_in_a_real_tcl_interpreter():
    """驗證：(1) 撞名的兩個大小都會被算到、彼此獨立彈出；(2) basename 比對真的有
    轉小寫；(3) 清單彈完才 `unset` 該 key。"""
    table = {"dup.dll": [10, 20], "solo.pyd": [5]}
    total = sum(size for sizes in table.values() for size in sizes)
    interp = _tcl_interpreter_with_progress_state(total, table)

    _report(interp, "dup.dll")
    _report(interp, "DUP.DLL")   # 同一個 key 第二次報到，basename 要轉小寫比對
    _report(interp, "solo.pyd")

    assert int(interp.eval("set _pyi_done")) == total
    assert interp.eval("array names _pyi_sizes") == ""


def test_extraction_target_is_capped_at_the_reserved_ceiling_not_full_width():
    """解壓只能推進到 80%（384/480px），不是滿格；剩下 20% 留給兩個階段字串。用
    字面數字釘住，不能只對照產生它的常數本身。"""
    assert splash_progress._BAR_WIDTH == 480
    assert splash_progress._EXTRACT_MAX_PX == 384   # 480px 的 80%

    table = {"cv2.pyd": [1000]}
    interp = _tcl_interpreter_with_progress_state(1000, table)

    _report(interp, "cv2.pyd")   # 唯一一個檔案，消耗掉全部位元組數

    assert int(interp.eval("set _pyi_target")) == 384


def test_phase_strings_move_the_target_to_their_reserved_percentage():
    """`PHASE_LOADING`／`PHASE_STARTING` 要精確比對成功，推到各自保留的百分比而非
    被誤判成查不到的檔名；用字面數字釘住，且排序要對得上使用者體感順序。
    """
    assert splash_progress._LOADING_TARGET_PX == 432    # 480px 的 90%
    assert splash_progress._STARTING_TARGET_PX == 480   # 滿格

    table = {"cv2.pyd": [1000]}
    interp = _tcl_interpreter_with_progress_state(1000, table)

    _report(interp, splash_progress.PHASE_LOADING)
    assert int(interp.eval("set _pyi_target")) == 432

    _report(interp, splash_progress.PHASE_STARTING)
    assert int(interp.eval("set _pyi_target")) == 480

    # 排序要對得上使用者體感的順序：解壓上限 < Loading < Starting（= 滿格）
    assert 384 < 432 < 480 <= splash_progress._BAR_WIDTH


def test_progress_step_advances_on_a_single_direct_call_with_no_after_ever_firing():
    """模擬 `after` 完全沒機會觸發的最壞情況（trace 事件密集時常見）：確認單次直接
    呼叫 `pyi_progress_step` 仍會前進，但不會瞬間跳到目標。"""
    interp = _tcl_interpreter_with_progress_state(1000, {"a.dll": [1000]})
    interp.eval("set _pyi_target 300")

    interp.eval("pyi_progress_step")   # 唯一一次直接呼叫，模擬 after 從未觸發

    first_current = float(interp.eval("set _pyi_current"))
    assert 0 < first_current < 300, "直接呼叫應該讓畫面前進，但還沒瞬間到達目標"


def test_progress_step_after_chain_converges_when_the_event_loop_actually_runs():
    """用 `vwait` 真的跑 Tcl 事件迴圈，讓 `pyi_progress_step` 排的 `after` 鏈自己
    接力收斂，而非只靠測試手動呼叫；輪詢＋逾時取代固定 sleep，避免 flake。"""
    interp = _tcl_interpreter_with_progress_state(1000, {"a.dll": [1000]})
    interp.eval("set _pyi_target 300")
    interp.eval("pyi_progress_step")   # 只手動踢一次，後續全靠 after 鏈自己接力

    interp.eval("set _pyi_test_result {}")
    interp.eval(
        "proc _pyi_test_poll {} {\n"
        "    global _pyi_current _pyi_target _pyi_test_result\n"
        "    if {$_pyi_current == $_pyi_target} {\n"
        "        set _pyi_test_result converged\n"
        "    } else {\n"
        "        after 16 _pyi_test_poll\n"
        "    }\n"
        "}"
    )
    interp.eval("after 16 _pyi_test_poll")
    interp.eval("after 3000 {set _pyi_test_result timeout}")
    interp.eval("vwait _pyi_test_result")   # 讓事件迴圈真的跑，給 after 鏈機會接力

    assert interp.eval("set _pyi_test_result") == "converged", (
        "after 鏈在 3 秒內沒有接力到收斂 —— 這正是 round 3 要修的 bug（鏈只接力一次就斷掉）"
    )


def test_progress_step_cancels_the_previous_after_before_scheduling_a_new_one():
    """快速連續兩次直接呼叫時，每次進入都要先取消上一次排的 `after` 再重排，最後
    只會剩一條排程在等，不會疊出兩條。"""
    interp = _tcl_interpreter_with_progress_state(1000, {"a.dll": [1000]})

    interp.eval("set _pyi_target 100")
    interp.eval("pyi_progress_step")
    interp.eval("set _pyi_target 200")
    interp.eval("pyi_progress_step")   # 第二次直接呼叫：必須先取消第一次排的那個

    pending = interp.eval("after info")
    assert len(pending.split()) == 1, f"應該只剩一條 after 排程在等，實際：{pending!r}"
