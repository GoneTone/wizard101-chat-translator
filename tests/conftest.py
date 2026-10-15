"""測試用的共用 fixture。

開發者常常一邊玩遊戲一邊跑測試，套件不該打斷遊戲畫面或搶走前景，但也不能因此
跳過測試。兩支 autouse fixture 各擋一種打斷，兩者都不跳過任何測試：
- `_park_windows` 把測試建立的視窗停到螢幕外；但 `@pytest.mark.real_position`
  測試斷言的就是視窗的真實座標，停到螢幕外會直接改壞被測目標，所以這類測試
  改成讓新開的視窗維持真實座標、但強制全透明（`-alpha` 釘在 0.0）—— 座標、
  尺寸、DWM frame bounds、`event_generate` 都維持真實可測，畫面上就是看不到。
- `_keep_foreground` 把 `force_foreground`（AttachThreadInput 硬切 Windows 前景）
  換成無操作，避免 `InputBox.show()`／`_destroy()` 與
  `single_instance.focus_running_instance` 在測試途中把遊戲踢到背景。
"""
import ast
import gc
import importlib
import tkinter as tk
from pathlib import Path

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


def _modules_binding_force_foreground() -> list[str]:
    """src 底下把 `force_foreground` 綁進自己命名空間的模組（含定義它的 composer.paste）。

    掃原始碼而不是手動列清單：新模組 import 它卻忘了登記，測試就會真的去搶遊戲前景，
    而且不會有任何錯誤提醒。"""
    root = Path(__file__).resolve().parents[1]
    found = {"src.composer.paste"}
    for path in (root / "src").rglob("*.py"):
        tree = ast.parse(path.read_text(encoding="utf-8"))
        if any(isinstance(node, ast.ImportFrom)
               and any(alias.name == "force_foreground" for alias in node.names)
               for node in ast.walk(tree)):
            parts = path.relative_to(root).with_suffix("").parts
            found.add(".".join(parts[:-1] if parts[-1] == "__init__" else parts))
    return sorted(found)


FOREGROUND_MODULES = _modules_binding_force_foreground()


@pytest.fixture(autouse=True)
def _keep_foreground():
    """把 `force_foreground` 換成無操作，避免測試搶走遊戲的 Windows 前景。

    不用 monkeypatch fixture，理由同 `_park_windows`：手動存還原，逐一覆寫
    在 import 當下就把名稱綁進自己命名空間的模組（`FOREGROUND_MODULES`），而不是只改
    定義處的 `src.composer.paste` —— 否則已經綁定的名稱不會跟著變。測試自己
    `monkeypatch.setattr(..., "force_foreground", ...)` 時後蓋前，不受影響。
    """
    modules = [importlib.import_module(name) for name in FOREGROUND_MODULES]
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


@pytest.fixture(autouse=True)
def _destroy_test_windows(request):
    """測試結束後銷毀它在共用 root 底下開的視窗，排在 after 的背景補畫才不會漏到之後的測試裡跑。"""
    root = request.getfixturevalue("root") if "root" in request.fixturenames else None
    yield
    children = root.winfo_children() if root is not None else []
    for widget in children:
        widget.destroy()
    if children:
        # 在主執行緒回收：Tk 物件的 __del__ 若被別的測試的執行緒觸發，每個都會卡約 1 秒等 mainloop
        gc.collect()
