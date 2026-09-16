"""Build 期把依位元組加權的進度條織進 PyInstaller 的 splash Tcl 樣板。

背景：onefile 的 bootloader 每解壓一個檔就用 Tcl_SetVar2 設一次 status_text，
PyInstaller 內建樣板只把這個值畫成文字 —— 檔名滿天飛，使用者看不懂那是什麼。
使用者的決定是「加進度條、文字照舊顯示」，且刻意選位元組數而非檔案數：本專案封存
內容最大的 5 個檔就佔了 72% 的位元組數，用檔案數推進度會在零點幾秒內衝到九成多，
然後在剩下的大半時間裡完全不動 —— 比檔名亂跳更像當機。

對照表涵蓋 `Analysis.binaries` 與 `Analysis.datas` 兩者：只算 binaries 實測只覆蓋
封存內容的 83%（190.3 / 229.3 MB），漏掉的大宗是 `collect_data_files("rapidocr")`
帶進來的模型檔（`PP-OCRv6_rec_small.onnx` 21.2 MB、`PP-OCRv6_det_small.onnx` 9.9
MB）—— 這兩個檔案體積不小又排在解壓尾聲，正好是使用者盯著畫面等最久的那段，不能漏。

basename 當 key 會撞名（實測 1121 個檔案裡 67 個 key 撞到 174 個檔案），但撞名的
清一色是 dist-info 中繼資料（`license.md`／`INSTALLER`／`METADATA`／`RECORD`／
`REQUESTED`，各僅幾 KB），沒有大檔案撞名，所以撞名不會讓進度條的視覺誤差有感。即
使如此，仍選擇**清單彈出**而非「後者覆蓋前者」：同一個 basename 的每個位元組數各自
進一個 Tcl 清單，bootloader 每回報一次就彈出清單開頭那個並累加，清單空了才 `unset`
——撞名的檔案也都會被算到，不會漏算，成本只是清單長度撐大了 array set 一點點。

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
from itertools import chain

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


def _size_table(entries: Iterable[Sequence[str]]) -> dict[str, list[int]]:
    """把 `Analysis.binaries`／`Analysis.datas` 轉成「basename（小寫）→ 未壓縮位元組
    清單」對照表。

    entry 形如 `(dest_name, src_path, typecode)`；用 `os.path.getsize(src_path)`
    取大小，讀不到就略過（不影響正確性，只是少算一點進度 —— `datas` 裡偶爾會有
    build 當下已經不存在的來源路徑）。basename 當 key 是因為 bootloader 回報的
    status_text 只有檔名（Tcl 端用 `file tail` 取），無從得知是哪個子目錄底下的
    檔案；撞名時兩個都留著（同一個 key 底下累積成清單），而不是後者覆蓋前者 ——
    撞名的檔案在解壓時一樣會各自觸發一次 status_text 更新，漏算會讓進度條卡在
    不到 100% 的地方。
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
    """`canvas_text_update` 本體新增的段落：依 basename 查表，每次報到彈出清單開頭
    那個位元組數並累加，清單空了才 `unset` —— 撞名的檔案（同一 basename 對應多個
    來源檔）靠這個機制都會被算到，不會被後面報到的同名項目蓋掉。"""
    bar_width = _BAR_X1 - _BAR_X0
    return (
        "\n"
        "    global _pyi_done _pyi_total _pyi_sizes\n"
        "    set _pyi_key [string tolower [file tail $var]]\n"
        "    if {[info exists _pyi_sizes($_pyi_key)]} {\n"
        "        set _pyi_list $_pyi_sizes($_pyi_key)\n"
        "        incr _pyi_done [lindex $_pyi_list 0]\n"
        "        set _pyi_list [lrange $_pyi_list 1 end]\n"
        "        if {[llength $_pyi_list] > 0} {\n"
        "            set _pyi_sizes($_pyi_key) $_pyi_list\n"
        "        } else {\n"
        "            unset _pyi_sizes($_pyi_key)\n"
        "        }\n"
        f"        set _pyi_w [expr {{int(double($_pyi_done) / $_pyi_total * {bar_width})}}]\n"
        f"        if {{$_pyi_w > {bar_width}}} {{set _pyi_w {bar_width}}}\n"
        f"        $canvas coords pyi_fill {_BAR_X0} {_BAR_Y0} "
        f"[expr {{{_BAR_X0} + $_pyi_w}}] {_BAR_Y1}\n"
        "    }"
    )


def install_progress_bar(binaries: Iterable[Sequence[str]], datas: Iterable[Sequence[str]]) -> int:
    """把進度條織進 PyInstaller 的 Tcl 模板；回傳納入計算的總位元組數。

    必須在建構 `Splash(...)` 之前呼叫。`binaries` 是 `Analysis.binaries`
    （已經過 ffmpeg 過濾的那份），`datas` 是 `Analysis.datas` —— 兩者合起來才是
    bootloader 實際會逐一解壓、觸發 status_text 更新的完整清單（只算 binaries 會
    漏掉 rapidocr 的模型檔，這兩個檔案體積不小又排在解壓尾聲）。順序上必須排在
    ffmpeg 過濾之後 —— 過濾前算會把已排除的檔案也算進總數，進度條永遠到不了滿格。
    """
    table = _size_table(chain(binaries, datas))
    total = sum(chain.from_iterable(table.values()))
    if total <= 0:
        raise ValueError(
            "splash progress bar: no binaries/datas with a readable size; check the build "
            "(are `binaries`/`datas` empty, or were all source paths unreadable?)"
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

    entry_count = sum(len(sizes) for sizes in table.values())
    log(
        f"[build] splash progress bar installed: keys={len(table)} entries={entry_count} "
        f"total_bytes={total} bar=({_BAR_X0},{_BAR_Y0})-({_BAR_X1},{_BAR_Y1})"
    )
    return total
