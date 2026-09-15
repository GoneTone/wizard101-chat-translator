"""測試用的共用 fixture。

開發者常常一邊玩遊戲一邊跑測試，套件不該打斷遊戲畫面或搶走前景。兩支 autouse
fixture 各擋一種打斷：
- `_park_windows` 把測試建立的視窗停到螢幕外，只有真正在斷言視窗位置的
  `@pytest.mark.real_position` 測試才豁免 —— 但這類測試預設也會被
  `pytest_collection_modifyitems` 直接跳過，本機執行不會有視窗真的畫到螢幕上；
  CI（見 `.github/workflows/ci.yml`）設 `WCT_REAL_POSITION=1` 讓這批測試照跑，
  才不會漏掉這段涵蓋範圍。
- `_keep_foreground` 把 `force_foreground`（AttachThreadInput 硬切 Windows 前景）
  換成無操作，避免 `InputBox.show()`／`_destroy()` 與
  `main.focus_running_instance` 在測試途中把遊戲踢到背景。
"""
import os
import tkinter as tk

import pytest

# 停放座標：夠遠，任何合理的桌面（含多螢幕橫向排列）都涵蓋不到。
_PARK_X, _PARK_Y = 10000, 10000


@pytest.fixture(scope="session")
def root():
    r = tk.Tk()
    r.withdraw()
    yield r
    r.destroy()


@pytest.fixture(autouse=True)
def _park_windows(request):
    """把測試建立的視窗停到螢幕外，但**保持 mapped**。

    不用 withdraw：視窗會量不到真實排版，換行寬度、下拉外部點擊那類測試會失效。
    不借用 monkeypatch fixture：與測試自己的 monkeypatch 共用還原堆疊會打亂還原順序
    （實測 test_i18n 的語言還原在 load 仍被 patch 成拋錯時執行）。
    @pytest.mark.real_position 的測試不套用 —— 它們斷言的就是視窗位置。
    """
    if request.node.get_closest_marker("real_position"):
        yield
        return
    original_wm, original_geo = tk.Wm.wm_geometry, tk.Wm.geometry

    def parked(self, newGeometry=None):
        # 只改寫位置，保留 WxH：測排版的測試仍量得到正確尺寸
        if newGeometry and "+" in newGeometry:
            newGeometry = f"{newGeometry.split('+')[0]}+{_PARK_X}+{_PARK_Y}"
        return original_wm(self, newGeometry)

    tk.Wm.wm_geometry = tk.Wm.geometry = parked
    try:
        yield
    finally:
        tk.Wm.wm_geometry, tk.Wm.geometry = original_wm, original_geo


@pytest.fixture(autouse=True)
def _keep_foreground():
    """把 `force_foreground` 換成無操作，避免測試搶走遊戲的 Windows 前景。

    不用 monkeypatch fixture，理由同 `_park_windows`：手動存還原，逐一覆寫
    在 import 當下就把名稱綁進自己命名空間的五個模組（`from ... import
    force_foreground`），而不是只改定義處的 `src.composer.paste` —— 否則已經
    綁定的名稱不會跟著變。測試自己 `monkeypatch.setattr(..., "force_foreground", ...)`
    時後蓋前，不受影響。
    """
    import src.composer.paste as paste_module
    import src.main as main_module
    import src.ui.input_box as input_box_module
    import src.ui.region_flow as region_flow_module
    import src.ui.region_select as region_select_module

    modules = (paste_module, main_module, input_box_module, region_select_module,
              region_flow_module)
    originals = [m.force_foreground for m in modules]

    def _noop(hwnd):
        return None

    for m in modules:
        m.force_foreground = _noop
    try:
        yield
    finally:
        for m, original in zip(modules, originals, strict=True):
            m.force_foreground = original


def pytest_collection_modifyitems(config, items):
    """本機預設跳過 `real_position` 測試：它們斷言視窗真實座標，會把視窗畫到
    使用者正在用的螢幕上。CI 設 `WCT_REAL_POSITION=1` 讓這批測試照跑，
    避免長期漏掉這段涵蓋範圍。"""
    if os.environ.get("WCT_REAL_POSITION") == "1":
        return
    skip_real_position = pytest.mark.skip(
        reason="draws windows on the real screen; set WCT_REAL_POSITION=1 to run"
    )
    for item in items:
        if item.get_closest_marker("real_position"):
            item.add_marker(skip_real_position)
