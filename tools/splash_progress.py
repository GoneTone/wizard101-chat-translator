"""Build 期把依位元組加權的進度條織進 PyInstaller 的 splash Tcl 樣板。

背景：onefile 的 bootloader 每解壓一個檔就用 Tcl_SetVar2 設一次 status_text，
PyInstaller 內建樣板只把這個值畫成文字 —— 檔名滿天飛，使用者看不懂那是什麼。
使用者的決定是「加進度條、文字照舊顯示」，且刻意選位元組數而非檔案數：本專案
1121 個 binary 裡最大的 5 個就佔了 72% 的位元組數，用檔案數推進度會在零點幾秒內衝到
九成多，然後在剩下的大半時間裡完全不動 —— 比檔名亂跳更像當機。

`PyInstaller.building.splash.Splash.__init__` 結尾會呼叫 `__postinit__()` ->
`assemble()`，Tcl 腳本在建構當下就組好寫進資源，事後再改 `splash.script` 已經來不及。
因此 `install_progress_bar()` 必須在 `Splash(...)` 建構之前呼叫，直接改寫
`PyInstaller.building.splash_templates` 模組層級的字串。

樣板是模組層級狀態，patch 會影響整個行程；本模組用一個字串錨點偵測是否已經改過，
重複呼叫不會疊加兩份進度條。
"""
from __future__ import annotations

import os
from collections.abc import Iterable, Sequence

from PyInstaller.building import splash_templates

from src.log import log
from tools.splash_image import PROGRESS_FILL_COLOR, PROGRESS_TRACK_COLOR, SIZE

# 進度條位置：畫面最下緣、整條寬度，跟狀態文字（左下角、TEXT_ORIGIN=(24, 276)）
# 留約 16px 的間距。
_BAR_X0 = 0
_BAR_Y0 = 292
_BAR_X1 = SIZE[0]
_BAR_Y1 = SIZE[1]

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


def _size_table(binaries: Iterable[Sequence[str]]) -> dict[str, int]:
    """把 `Analysis.binaries` 轉成「basename（小寫）→ 未壓縮位元組」對照表。

    entry 形如 `(dest_name, src_path, typecode)`；用 `os.path.getsize(src_path)`
    取大小，讀不到就略過（不影響正確性，只是少算一點進度）。basename 當 key 是因為
    bootloader 回報的 status_text 只有檔名（Tcl 端用 `file tail` 取），無從得知是哪個
    子目錄底下的檔案；兩個來源不同、檔名剛好相同的極罕見情況下，後者會覆蓋前者。
    """
    table: dict[str, int] = {}
    for dest_name, src_path, _typecode in binaries:
        try:
            size = os.path.getsize(src_path)
        except OSError:
            continue
        basename = os.path.basename(dest_name).lower()
        if "{" in basename or "}" in basename:
            # 大括號在 Tcl 的 array set 清單語法裡有特殊意義，這種檔名塞不進去，
            # 略過即可 —— 二進位檔名幾乎不會出現這種字元。
            continue
        table[basename] = size
    return table


def _size_table_tcl(table: dict[str, int]) -> str:
    """把對照表格式化成 `array set` 吃的扁平清單，每個 basename 用大括號包住以防
    檔名帶空白破壞 Tcl 的清單斷詞。"""
    return " ".join(f"{{{name}}} {size}" for name, size in table.items())


def _canvas_setup_addition(total: int, table: dict[str, int]) -> str:
    return (
        "\n"
        "set _pyi_done 0\n"
        f"set _pyi_total {total}\n"
        f"array set _pyi_sizes {{{_size_table_tcl(table)}}}\n"
        f'.root.canvas create rectangle {_BAR_X0} {_BAR_Y0} {_BAR_X1} {_BAR_Y1} '
        f'-fill "{PROGRESS_TRACK_COLOR}" -outline "" -tag pyi_track\n'
        f'.root.canvas create rectangle {_BAR_X0} {_BAR_Y0} {_BAR_X0} {_BAR_Y1} '
        f'-fill "{PROGRESS_FILL_COLOR}" -outline "" -tag pyi_fill'
    )


def _text_update_addition() -> str:
    bar_width = _BAR_X1 - _BAR_X0
    return (
        "\n"
        "    global _pyi_done _pyi_total _pyi_sizes\n"
        "    set _pyi_key [string tolower [file tail $var]]\n"
        "    if {[info exists _pyi_sizes($_pyi_key)]} {\n"
        "        incr _pyi_done $_pyi_sizes($_pyi_key)\n"
        "        unset _pyi_sizes($_pyi_key)\n"
        f"        set _pyi_w [expr {{int(double($_pyi_done) / $_pyi_total * {bar_width})}}]\n"
        f"        if {{$_pyi_w > {bar_width}}} {{set _pyi_w {bar_width}}}\n"
        f"        $canvas coords pyi_fill {_BAR_X0} {_BAR_Y0} "
        f"[expr {{{_BAR_X0} + $_pyi_w}}] {_BAR_Y1}\n"
        "    }"
    )


def install_progress_bar(binaries: Iterable[Sequence[str]]) -> int:
    """把進度條織進 PyInstaller 的 Tcl 模板；回傳納入計算的總位元組數。

    必須在建構 `Splash(...)` 之前呼叫。`binaries` 是 `Analysis.binaries`
    （已經過 ffmpeg 過濾的那份）——順序錯了，過濾掉的檔案也會被算進總數，進度條
    永遠到不了滿格。
    """
    table = _size_table(binaries)
    total = sum(table.values())
    if total <= 0:
        raise ValueError(
            "splash progress bar: no binaries with a readable size; check the build "
            "(is `binaries` empty, or were all source paths unreadable?)"
        )

    if _SENTINEL in splash_templates.splash_canvas_setup:
        log("[build] splash progress bar template already patched; skip re-injecting")
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

    splash_templates.splash_canvas_setup = splash_templates.splash_canvas_setup.replace(
        _CANVAS_ANCHOR, _CANVAS_ANCHOR + _canvas_setup_addition(total, table)
    )
    splash_templates.image_script = splash_templates.image_script.replace(
        _TEXT_UPDATE_ANCHOR, _TEXT_UPDATE_ANCHOR + _text_update_addition()
    )

    log(
        f"[build] splash progress bar installed: entries={len(table)} total_bytes={total} "
        f"bar=({_BAR_X0},{_BAR_Y0})-({_BAR_X1},{_BAR_Y1})"
    )
    return total
