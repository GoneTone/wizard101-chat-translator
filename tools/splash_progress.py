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

使用者實跑後回報兩個現象，這次一併修掉：(1) 進度條中段會跳一大截 —— 解壓順序裡
`cv2.pyd` 前面 1000 多個小檔案只佔 7.6% 的位元組數，`cv2.pyd` 本身佔 37.6%，單一
事件把進度條從 7.6% 推到 45.2% 是逐檔計分本身的限制，接受它、但用緩動動畫把「跳」
感磨掉；(2) 進度條解壓完就滿格，接著卻還要再等約 0.78 秒（`import src.main` 約
0.31 秒＋`[app] version=` 到 `[app] running` 實測 0.47～0.48 秒）畫面才真的可用 ——
解壓只推進到 80%，剩下的 20% 交給 `run.py`／`src/main.py` 已經會送的兩個階段訊息
（`PHASE_LOADING` 推到 90%、`PHASE_STARTING` 推到 97%），splash 在 `build_app()`
之後立刻關閉，所以刻意不推到 100%——滿格之後還晾著不動，比停在 97% 更奇怪。

`PHASE_LOADING`／`PHASE_STARTING` 定義在 `src/splash.py`（執行期真正呼叫
`splash.update()` 的地方），這裡在 build 時原封不動 import 進來烤進 Tcl 的完全比對，
不在這裡另外複製一份字面值 —— 兩邊字面值不同步只會讓進度條卡住，不會有任何測試
變紅，除非兩邊都盯著同一個常數。

進度條的推進改成緩動動畫而非瞬間跳到目標：bootloader 解壓期間 Tcl 的事件迴圈不
保證會處理 `after` —— 若動畫只靠 `after` 排程，解壓密集時迴圈沒空跑，畫面就會卡住
不動。因此每個 trace 事件（不論是解壓回報還是階段訊息）都無條件直接呼叫一次步進
函式 `pyi_progress_step`，確保「`after` 完全沒機會跑」的最壞情況下進度條依然每個
事件都會前進一點（解壓期間事件密集，逐次前進本身就有平滑效果）；`pyi_progress_step`
另外用旗標 `_pyi_animating` 避免同時有兩條 `after` 排程鏈疊加，正常情況下（例如兩個
階段訊息間隔數百毫秒）則會在約 200ms 內用 easing（每步移動剩餘距離的 25%、每 16ms
一步）把動畫補完。
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
_BAR_Y0 = 292
_BAR_X1 = SIZE[0]
_BAR_Y1 = SIZE[1]
_BAR_WIDTH = _BAR_X1 - _BAR_X0

# 解壓只推進到 80%；剩下交給兩個階段訊息（見模組說明）。splash 在 build_app() 之後
# 立刻關閉，`PHASE_STARTING` 刻意不設在 100%——滿格之後還晾著不動，比停在 97% 更奇怪。
_EXTRACT_CEILING = 0.80
_PHASE_LOADING_FRACTION = 0.90
_PHASE_STARTING_FRACTION = 0.97
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
    """畫布建立之後接的段落：初始化進度狀態、畫軌道／填色兩個矩形，並定義
    `pyi_progress_step`——每個 trace 事件都會直接呼叫它一次（見模組說明），它自己
    再視情況用 `after` 接力把動畫補到目標，兩者靠 `_pyi_animating` 旗標互相協調，
    不會疊出兩條並行的 `after` 鏈。"""
    return (
        "\n"
        "set _pyi_done 0\n"
        f"set _pyi_total {total}\n"
        f"array set _pyi_sizes {{{_size_table_tcl(table)}}}\n"
        "set _pyi_target 0\n"
        "set _pyi_current 0\n"
        "set _pyi_animating 0\n"
        f'.root.canvas create rectangle {_BAR_X0} {_BAR_Y0} {_BAR_X1} {_BAR_Y1} '
        f'-fill "{PROGRESS_TRACK_COLOR}" -outline "" -tag pyi_track\n'
        f'.root.canvas create rectangle {_BAR_X0} {_BAR_Y0} {_BAR_X0} {_BAR_Y1} '
        f'-fill "{PROGRESS_FILL_COLOR}" -outline "" -tag pyi_fill\n'
        "proc pyi_progress_step {} {\n"
        "    global _pyi_current _pyi_target _pyi_animating\n"
        "    set _pyi_current [expr {$_pyi_current + ($_pyi_target - $_pyi_current) * 0.25}]\n"
        f"    .root.canvas coords pyi_fill {_BAR_X0} {_BAR_Y0} "
        f"[expr {{{_BAR_X0} + int($_pyi_current)}}] {_BAR_Y1}\n"
        "    if {[expr {abs($_pyi_target - $_pyi_current)}] > 1} {\n"
        "        if {!$_pyi_animating} {\n"
        "            set _pyi_animating 1\n"
        "            after 16 pyi_progress_step\n"
        "        }\n"
        "    } else {\n"
        "        set _pyi_current $_pyi_target\n"
        "        set _pyi_animating 0\n"
        "    }\n"
        "}"
    )


def _text_update_addition() -> str:
    """`canvas_text_update` 本體新增的段落。先精確比對兩個階段字串（`PHASE_LOADING`／
    `PHASE_STARTING`，從 `src.splash` import 進來，不在這裡另抄一份字面值），推進
    到各自保留的目標；否則落到 basename 查表：每次報到彈出清單開頭那個位元組數並
    累加，清單空了才 `unset`（撞名的檔案都會被算到，不會被後面報到的同名項目蓋掉），
    上限壓在 `_EXTRACT_MAX_PX`（80%）而不是滿格——剩下留給兩個階段字串推進。三個
    分支只要有動到 `_pyi_target` 就呼叫一次 `pyi_progress_step`，理由見模組說明。"""
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

    必須在建構 `Splash(...)` 之前呼叫。`binaries` 是 `Analysis.binaries`
    （已經過 ffmpeg 過濾的那份），`datas` 是 `Analysis.datas` —— 兩者合起來才是
    bootloader 實際會逐一解壓、觸發 status_text 更新的完整清單（只算 binaries 會
    漏掉 rapidocr 的模型檔，這兩個檔案體積不小又排在解壓尾聲）。順序上必須排在
    ffmpeg 過濾之後 —— 過濾前算會把已排除的檔案也算進總數，進度條永遠到不了滿格。
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
