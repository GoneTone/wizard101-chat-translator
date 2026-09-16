from pathlib import Path

from src.main import apply_window_icon
from src.resources import bundle_dir, icon_path, png_icon_path

SRC = Path(__file__).resolve().parents[1] / "src"
BUILD_SPEC = Path(__file__).resolve().parents[1] / "build.spec"
# Windows 各處要用的尺寸都得有原生 frame：16 標題列／20-24 工作列／32 Alt+Tab／
# 40-48 檔案總管／64+ 大圖示。少了 24 的話工作列只能從 32 硬縮（1.33:1 非整數），
# 整張圖連螺旋一起軟掉 —— 這正是「工作列圖示很模糊」的成因。
EXPECTED_SIZES = [16, 20, 24, 32, 40, 48, 64, 128]
# overlay 用得到的 PNG：標題列（_BAR_ICON）與縮小後的泡泡（_BUBBLE_SIZE）
PNG_SIZES = [16, 48]


def test_bundle_dir_points_at_src_subdirectory():
    assert bundle_dir("i18n") == SRC / "i18n"
    assert bundle_dir("i18n").is_dir()


def test_icon_file_exists():
    assert icon_path() == SRC / "assets" / "icon.ico"
    assert icon_path().is_file()


def test_icon_is_valid_ico_with_expected_sizes():
    data = icon_path().read_bytes()
    assert data[:4] == b"\x00\x00\x01\x00", "not an ICO header"
    count = int.from_bytes(data[4:6], "little")
    # ICO 目錄項的寬高各佔 1 byte，0 代表 256
    sizes = [data[6 + i * 16] or 256 for i in range(count)]
    assert sizes == EXPECTED_SIZES


def test_png_icons_exist_at_the_sizes_tkinter_needs():
    # PhotoImage 只能整數倍縮放（畫質很差），所以每個用到的尺寸都得有原生檔案
    for size in PNG_SIZES:
        path = png_icon_path(size)
        assert path == SRC / "assets" / f"icon_{size}.png"
        assert path.is_file()
        header = path.read_bytes()[:24]
        assert header[:8] == b"\x89PNG\r\n\x1a\n", f"{path.name} is not a PNG"
        width = int.from_bytes(header[16:20], "big")
        height = int.from_bytes(header[20:24], "big")
        assert (width, height) == (size, size)


def test_build_spec_bundles_and_applies_the_icon():
    # 兩處都要提到 icon：exe 圖示，以及讓 tkinter 視窗拿得到的 datas 收錄，只改一邊就會
    # 「exe 有圖示、視窗沒有」；datas 收整個 assets 目錄，新增圖檔不必再改 spec
    spec = BUILD_SPEC.read_text(encoding="utf-8")
    assert 'icon="src/assets/icon.ico"' in spec
    assert '("src/assets/*", "assets")' in spec


def test_build_spec_wires_up_the_splash_screen():
    """splash／splash.binaries 要排在 a.binaries、a.datas 之前，onefile 才能顯示；
    鎖連續多行的順序，字串「有出現」抓不到重排。"""
    spec = BUILD_SPEC.read_text(encoding="utf-8")
    assert "Splash(" in spec
    # 狀態文字必須是 ASCII：它會被寫進 bootloader 的 Tcl 腳本
    assert 'text_default="Initializing..."' in spec
    assert (
        "    splash,\n"
        "    splash.binaries,\n"
        "    a.binaries,\n"
        "    a.datas,\n"
    ) in spec


def test_build_spec_installs_the_progress_bar_before_constructing_splash():
    """`install_progress_bar` 要在 `Splash(...)` 建構之前呼叫，否則樣板已經組好、
    改不了；只檢查字串「有出現」抓不到順序錯誤。"""
    spec = BUILD_SPEC.read_text(encoding="utf-8")
    assert "install_progress_bar(a.binaries, a.datas)" in spec
    assert spec.index("install_progress_bar(a.binaries, a.datas)") < spec.index("splash = Splash(")


def test_apply_window_icon_survives_missing_file(root, monkeypatch, tmp_path):
    monkeypatch.setattr("src.main.icon_path", lambda: tmp_path / "nope.ico")
    apply_window_icon(root)   # 只該留 log，不該把啟動流程帶掉


def test_apply_window_icon_installs_it_on_the_window_class(root):
    # tkinter 的 iconbitmap 挑不對 ICO 的 frame（實測掛出來是放大裁切過的糊圖），
    # 所以改成自己載入正確尺寸再寫進視窗類別 —— 之後建立的每個 Toplevel 都該沿用它。
    import tkinter as tk

    import win32con
    import win32gui

    hicon = apply_window_icon(root)
    assert hicon, "沒有載入到 icon"

    probe = tk.Toplevel(root)
    probe.withdraw()
    probe.update_idletasks()
    hwnd = win32gui.GetAncestor(probe.winfo_id(), 2)   # GA_ROOT
    try:
        assert win32gui.GetClassLong(hwnd, win32con.GCL_HICON) == hicon
    finally:
        probe.destroy()
