"""測試用的共用 fixture。

開發者常常一邊玩遊戲一邊跑測試，套件不該打斷遊戲畫面或搶走前景，但也不能因此
跳過測試。兩支 autouse fixture 各擋一種打斷，兩者都不跳過任何測試：
- `_park_windows` 把測試建立的視窗停到螢幕外；但 `@pytest.mark.real_position`
  測試斷言的就是視窗的真實座標，停到螢幕外會直接改壞被測目標，所以這類測試
  改成讓新開的視窗維持真實座標、但強制全透明（`-alpha` 釘在 0.0）—— 座標、
  尺寸、DWM frame bounds、`event_generate` 都維持真實可測，畫面上就是看不到。
- `_keep_foreground` 把 `force_foreground`（AttachThreadInput 硬切 Windows 前景）
  換成無操作，避免 `InputBox.show()`／`_destroy()` 與
  `main.focus_running_instance` 在測試途中把遊戲踢到背景。
"""
import tkinter as tk

import pytest

# 停放座標：夠遠，任何合理的桌面（含多螢幕橫向排列）都涵蓋不到。
_PARK_X, _PARK_Y = 10000, 10000


def _is_alpha_write(args):
    """`wm_attributes` 的位置參數是不是在寫 alpha（選項名前面帶不帶 `-` 都算）。"""
    if len(args) < 2:
        return False
    option = args[0]
    return isinstance(option, str) and option.lstrip("-") == "alpha"


@pytest.fixture(scope="session")
def root():
    r = tk.Tk()
    r.withdraw()
    yield r
    r.destroy()


@pytest.fixture(autouse=True)
def _park_windows(request):
    """把測試建立的視窗停到螢幕外，但**保持 mapped**；real_position 測試改為全透明。

    不用 withdraw：視窗會量不到真實排版，換行寬度、下拉外部點擊那類測試會失效。
    不借用 monkeypatch fixture：與測試自己的 monkeypatch 共用還原堆疊會打亂還原順序
    （實測 test_i18n 的語言還原在 load 仍被 patch 成拋錯時執行）。

    @pytest.mark.real_position 的測試不套用停放，改套用透明化：包一層
    `tk.Toplevel.__init__`，原生建構完成後立刻把該視窗的 `-alpha` 設成 0.0；
    再包一層 `wm_attributes`（含 `attributes` 別名，stdlib 裡兩者是同一個函式
    物件），把任何寫入 `-alpha` 的呼叫（位置參數或關鍵字參數皆算）強制改寫成
    0.0，讀取與其他屬性原樣放行 —— 這樣被測程式自己呼叫 `attributes("-alpha",
    x)` 調不透明度也現不了形。
    """
    if request.node.get_closest_marker("real_position"):
        original_init = tk.Toplevel.__init__
        original_wm_attributes = tk.Wm.wm_attributes

        def invisible_init(self, *args, **kwargs):
            original_init(self, *args, **kwargs)
            self.attributes("-alpha", 0.0)

        def forced_transparent(self, *args, **kwargs):
            if "alpha" in kwargs:
                kwargs = dict(kwargs, alpha=0.0)
            elif _is_alpha_write(args):
                args = (args[0], 0.0) + args[2:]
            return original_wm_attributes(self, *args, **kwargs)

        tk.Toplevel.__init__ = invisible_init
        tk.Wm.wm_attributes = tk.Wm.attributes = forced_transparent
        try:
            yield
        finally:
            tk.Toplevel.__init__ = original_init
            tk.Wm.wm_attributes = tk.Wm.attributes = original_wm_attributes
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
