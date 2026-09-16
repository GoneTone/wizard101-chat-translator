"""Build 期把依位元組加權的進度條織進 PyInstaller 的 splash Tcl 樣板。

用位元組數而非檔案數計分，避免少數大檔案讓進度條衝到九成後卡住不動；對照表涵蓋
`Analysis.binaries` 與 `Analysis.datas`（見 `_size_table`）。

必須在 `Splash(...)` 建構之前呼叫，樣板組裝後無法再改；`_SENTINEL` 避免重複
patch（見 `install_progress_bar`）。

解壓只推進到 80%，其餘由 `src/splash.py` 的 `PHASE_LOADING`／`PHASE_STARTING`
推到 90%／滿格；動畫細節見 `_canvas_setup_addition`。
"""
from __future__ import annotations

import os
from collections.abc import Iterable, Sequence
from itertools import chain

from PyInstaller.building import splash_templates

from src.log import log
from src.splash import PHASE_LOADING, PHASE_STARTING
from tools.splash_image import PROGRESS_FILL_COLOR, PROGRESS_TRACK_COLOR, SIZE

# 進度條位置：畫面最下緣、整條寬度，跟狀態文字（左下角、TEXT_ORIGIN=(24, 276)）
# 留約 16px 的間距。
_BAR_X0 = 0
_BAR_Y0 = 296
_BAR_X1 = SIZE[0]
_BAR_Y1 = SIZE[1]
_BAR_WIDTH = _BAR_X1 - _BAR_X0

# 解壓只推進到 80%，剩下交給兩個階段訊息（見模組說明）；`PHASE_STARTING` 推到
# 滿格，讓收尾時短暫滿格而非停在未滿的百分比，避免看起來像卡住。
_EXTRACT_CEILING = 0.80
_PHASE_LOADING_FRACTION = 0.90
_PHASE_STARTING_FRACTION = 1.0
_EXTRACT_MAX_PX = round(_BAR_WIDTH * _EXTRACT_CEILING)
_LOADING_TARGET_PX = round(_BAR_WIDTH * _PHASE_LOADING_FRACTION)
_STARTING_TARGET_PX = round(_BAR_WIDTH * _PHASE_STARTING_FRACTION)

# 用來偵測樣板是否已經被本函式改過 —— 只要這個變數名還在，代表已經 patch 過。
_SENTINEL = "_pyi_total"

# splash_templates.splash_canvas_setup 結尾的畫布建立；進度條的元素接在它後面。
_CANVAS_ANCHOR = (
    ".root.canvas create image \\\n"
    "    [expr {$image_width / 2}] \\\n"
    "    [expr {$image_height / 2}] \\\n"
    "    -image splash_image"
)
# splash_templates.image_script 裡 canvas_text_update 本體那一行；用它在更新文字
# 之外接上依位元組加權的進度推進。
_TEXT_UPDATE_ANCHOR = "    $canvas itemconfigure $tag -text $var"


def _size_table(entries: Iterable[Sequence[str]]) -> dict[str, list[int]]:
    """把 `Analysis.binaries`／`Analysis.datas` 轉成「basename（小寫）→ 位元組
    清單」對照表。

    basename 當 key 是因為 bootloader 回報的 status_text 只有檔名；撞名時累積成
    清單而非覆蓋，逐一彈出才不會漏算。讀不到大小的來源路徑直接略過。
    """
    table: dict[str, list[int]] = {}
    for dest_name, src_path, _typecode in entries:
        try:
            size = os.path.getsize(src_path)
        except OSError:
            continue
        basename = os.path.basename(dest_name).lower()
        if "{" in basename or "}" in basename:
            # 大括號在 Tcl 的 array set 清單語法裡有特殊意義，這種檔名塞不進去，
            # 略過即可 —— 二進位／資料檔名幾乎不會出現這種字元。
            continue
        table.setdefault(basename, []).append(size)
    return table


def _size_table_tcl(table: dict[str, list[int]]) -> str:
    """把對照表格式化成 `array set` 吃的扁平清單：每個 basename 用大括號包住以防
    檔名帶空白破壞 Tcl 的清單斷詞，值本身也是一份大小的清單（撞名時不只一個）。"""
    return " ".join(
        f"{{{name}}} {{{' '.join(str(size) for size in sizes)}}}" for name, sizes in table.items()
    )


def _canvas_setup_addition(total: int, table: dict[str, list[int]]) -> str:
    """畫布建立之後接的段落：初始化進度狀態、畫軌道／填色兩個矩形，並定義
    `pyi_progress_step`（每個 trace 事件都直接呼叫一次，見模組說明）。

    每次進入都先 `after cancel` 掉上一次排程的 id 再重排，不論由直接呼叫還是
    `after` 觸發都不會斷鏈，也不會疊出兩條並行的鏈。
    """
    return (
        "\n"
        "set _pyi_done 0\n"
        f"set _pyi_total {total}\n"
        f"array set _pyi_sizes {{{_size_table_tcl(table)}}}\n"
        "set _pyi_target 0\n"
        "set _pyi_current 0\n"
        f'.root.canvas create rectangle {_BAR_X0} {_BAR_Y0} {_BAR_X1} {_BAR_Y1} '
        f'-fill "{PROGRESS_TRACK_COLOR}" -outline "" -tag pyi_track\n'
        f'.root.canvas create rectangle {_BAR_X0} {_BAR_Y0} {_BAR_X0} {_BAR_Y1} '
        f'-fill "{PROGRESS_FILL_COLOR}" -outline "" -tag pyi_fill\n'
        "proc pyi_progress_step {} {\n"
        "    global _pyi_current _pyi_target _pyi_after_id\n"
        "    if {[info exists _pyi_after_id]} {\n"
        "        after cancel $_pyi_after_id\n"
        "        unset _pyi_after_id\n"
        "    }\n"
        "    set _pyi_current [expr {$_pyi_current + ($_pyi_target - $_pyi_current) * 0.25}]\n"
        "    if {[expr {abs($_pyi_target - $_pyi_current)}] <= 1} {\n"
        "        set _pyi_current $_pyi_target\n"
        "    }\n"
        "    catch {"
        f".root.canvas coords pyi_fill {_BAR_X0} {_BAR_Y0} "
        f"[expr {{{_BAR_X0} + int($_pyi_current)}}] {_BAR_Y1}"
        "}\n"
        "    if {$_pyi_current != $_pyi_target} {\n"
        "        set _pyi_after_id [after 16 pyi_progress_step]\n"
        "    }\n"
        "}"
    )


def _text_update_addition() -> str:
    """`canvas_text_update` 本體新增的段落：精確比對 `PHASE_LOADING`／`PHASE_STARTING`
    （從 `src.splash` import，不在此另抄字面值）推進到各自目標；否則落到 basename
    查表，彈出清單頭部的位元組數累加，上限壓在 `_EXTRACT_MAX_PX`（80%），剩下留給
    兩個階段字串推進。"""
    return (
        "\n"
        "    global _pyi_done _pyi_total _pyi_sizes _pyi_target\n"
        f"    if {{$var eq {{{PHASE_LOADING}}}}} {{\n"
        f"        set _pyi_target {_LOADING_TARGET_PX}\n"
        "        pyi_progress_step\n"
        f"    }} elseif {{$var eq {{{PHASE_STARTING}}}}} {{\n"
        f"        set _pyi_target {_STARTING_TARGET_PX}\n"
        "        pyi_progress_step\n"
        "    } else {\n"
        "        set _pyi_key [string tolower [file tail $var]]\n"
        "        if {[info exists _pyi_sizes($_pyi_key)]} {\n"
        "            set _pyi_list $_pyi_sizes($_pyi_key)\n"
        "            incr _pyi_done [lindex $_pyi_list 0]\n"
        "            set _pyi_list [lrange $_pyi_list 1 end]\n"
        "            if {[llength $_pyi_list] > 0} {\n"
        "                set _pyi_sizes($_pyi_key) $_pyi_list\n"
        "            } else {\n"
        "                unset _pyi_sizes($_pyi_key)\n"
        "            }\n"
        "            set _pyi_target "
        f"[expr {{int(double($_pyi_done) / $_pyi_total * {_EXTRACT_MAX_PX})}}]\n"
        f"            if {{$_pyi_target > {_EXTRACT_MAX_PX}}} {{set _pyi_target {_EXTRACT_MAX_PX}}}\n"
        "            pyi_progress_step\n"
        "        }\n"
        "    }"
    )


def install_progress_bar(binaries: Iterable[Sequence[str]], datas: Iterable[Sequence[str]]) -> int:
    """把進度條織進 PyInstaller 的 Tcl 模板；回傳納入計算的總位元組數。

    必須在建構 `Splash(...)` 之前呼叫，且要排在 ffmpeg 過濾之後 —— 過濾前算會把
    已排除的檔案也算進總數，進度條永遠到不了滿格。`binaries`／`datas` 合計才是
    bootloader 實際會逐一解壓的完整清單，只算 binaries 會漏掉 rapidocr 的模型檔。
    """
    for phase in (PHASE_LOADING, PHASE_STARTING):
        if "{" in phase or "}" in phase:
            # 跟 basename 一樣的理由：大括號會破壞 `$var eq {...}` 的 Tcl 分組語法。
            # 這兩個字串是本專案自己定義的常數，理論上不會發生，但生成出一份語法
            # 錯誤的 Tcl 比 build 直接紅掉更難查。
            raise ValueError(
                f"splash progress bar: phase string {phase!r} contains Tcl brace characters "
                "and can't be safely embedded in the generated script"
            )

    table = _size_table(chain(binaries, datas))
    total = sum(chain.from_iterable(table.values()))
    if total <= 0:
        raise ValueError(
            "splash progress bar: no binaries/datas with a readable size; check the build "
            "(are `binaries`/`datas` empty, or were all source paths unreadable?)"
        )

    if _SENTINEL in splash_templates.splash_canvas_setup:
        log("[build] splash progress bar template already patched; skip re-injecting")
        # 函式每次 build 只被呼叫一次，所以回傳的 total 跟模板裡寫死的 _pyi_total 相符；若被呼叫多次
        # 且每次輸入不同，就會默默出現不符的情況。
        return total

    if _CANVAS_ANCHOR not in splash_templates.splash_canvas_setup:
        raise RuntimeError(
            "splash_progress: splash_canvas_setup anchor not found. PyInstaller's splash "
            "template may have changed — update tools/splash_progress.py to match the new "
            "template before shipping a build with a silently broken splash screen."
        )
    if _TEXT_UPDATE_ANCHOR not in splash_templates.image_script:
        raise RuntimeError(
            "splash_progress: canvas_text_update anchor not found. PyInstaller's splash "
            "template may have changed — update tools/splash_progress.py to match the new "
            "template before shipping a build with a silently broken splash screen."
        )

    if splash_templates.splash_canvas_setup.count(_CANVAS_ANCHOR) != 1:
        raise RuntimeError(
            f"splash_progress: splash_canvas_setup anchor found "
            f"{splash_templates.splash_canvas_setup.count(_CANVAS_ANCHOR)} times (expected 1). "
            f"PyInstaller's splash template may have changed — update tools/splash_progress.py "
            f"to match the new template before shipping a build."
        )
    splash_templates.splash_canvas_setup = splash_templates.splash_canvas_setup.replace(
        _CANVAS_ANCHOR, _CANVAS_ANCHOR + _canvas_setup_addition(total, table), count=1
    )

    if splash_templates.image_script.count(_TEXT_UPDATE_ANCHOR) != 1:
        raise RuntimeError(
            f"splash_progress: canvas_text_update anchor found "
            f"{splash_templates.image_script.count(_TEXT_UPDATE_ANCHOR)} times (expected 1). "
            f"PyInstaller's splash template may have changed — update tools/splash_progress.py "
            f"to match the new template before shipping a build."
        )
    splash_templates.image_script = splash_templates.image_script.replace(
        _TEXT_UPDATE_ANCHOR, _TEXT_UPDATE_ANCHOR + _text_update_addition(), count=1
    )

    entry_count = sum(len(sizes) for sizes in table.values())
    log(
        f"[build] splash progress bar installed: keys={len(table)} entries={entry_count} "
        f"total_bytes={total} bar=({_BAR_X0},{_BAR_Y0})-({_BAR_X1},{_BAR_Y1}) "
        f"extract_max_px={_EXTRACT_MAX_PX} loading_px={_LOADING_TARGET_PX} "
        f"starting_px={_STARTING_TARGET_PX}"
    )
    return total
