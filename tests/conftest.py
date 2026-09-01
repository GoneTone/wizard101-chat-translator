"""測試用的共用 fixture。

跑測試時不該有視窗彈到使用者的畫面上搶焦點——開發者多半正在同一台機器上做別的事。
`_park_windows` 把每個視窗停到螢幕外，只有真正在斷言視窗位置的測試才豁免。
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

    不用 withdraw：那樣視窗量不到真實排版，換行寬度、下拉選單的外部點擊那類測試
    會整批失效。改寫位置則兩者兼得——畫得出來，但不在使用者看得到的地方。

    不借用 monkeypatch fixture：它與測試自己的 monkeypatch 共用同一個還原堆疊，
    會打亂還原順序（實測讓 test_i18n 的語言還原在 load 仍被 patch 成拋錯時執行）。

    以 @pytest.mark.real_position 標註的測試不套用——它們斷言的就是視窗位置，
    改掉位置等於改壞被測目標。
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
