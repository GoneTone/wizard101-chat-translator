"""測試用的共用 fixture。

跑測試時視窗不該彈到使用者畫面上搶焦點；`_park_windows` 把視窗停到螢幕外，
只有真正在斷言視窗位置的測試才豁免。
"""
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
    @pytest.mark.real_position 的測試不套用——它們斷言的就是視窗位置。
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
