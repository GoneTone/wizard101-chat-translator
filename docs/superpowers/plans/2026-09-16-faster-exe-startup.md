# exe 啟動加速與啟動畫面 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把打包版 exe 的 warm 啟動時間從約 2.8 秒降到約 2.0 秒，並讓整段等待都有畫面。

**Architecture:** 三項互相獨立的改動。A 把 `anthropic`（0.68 秒）移出啟動路徑；B 從打包內容剔除兩個確定用不到的重型檔案；C 用 PyInstaller bootloader 層的 Splash 從解壓前就顯示啟動畫面，再由 Python 端逐階段更新文字。A 與 B 不依賴 C —— 若 C 因已知 bug 被放棄，A 與 B 仍完整保留收益。

**Tech Stack:** Python 3.11+、PyInstaller 6.22.2（onefile）、Pillow、tkinter、pytest、uv。

**Spec:** `docs/superpowers/specs/2026-09-16-faster-exe-startup-design.md`

## Global Constraints

- **Python `>=3.11`**。型別註解在 3.11／3.12 會在函式定義時求值（PEP 649 的延遲求值是 3.14 才有），不可依賴 3.14 的行為。
- **維持單一 exe（onefile）**，不得改成 onedir。
- **log 訊息一律英文**，帶 `[模組]` 前綴，不直接用 `print`（`src/log.py` 的 `log()`）。敏感資料（API 金鑰）絕不可寫入 log。
- **註解與 docstring 用繁體中文**，CJK 語境一律全形標點；破折號 `——` 前後各留一個半形空格。
- **splash 上的文字一律 ASCII 英文**（含省略號用 `...` 而非 `…`）：那串字會被寫進 bootloader 的 Tcl 腳本，ASCII 沒有編碼風險。
- **ruff**：`line-length = 110`，`select = ["E", "F", "W", "I", "B", "UP", "SIM"]`。
- **commit message 英文**，conventional commits（`feat(...)`／`fix(...)`／`chore(...)`／`refactor(...)`／`docs(...)`）。
- **每次 commit 前**：`uv run ruff check src tests` 零錯誤、`uv run pytest` 全綠。Task 5 之後 lint 範圍擴成 `src tests tools`。
- **不要 `git push`**。

---

### Task 1: A —— 把 `anthropic` 移出啟動路徑

**Files:**
- Modify: `src/translation/translator.py:17`（移除頂層 import）、`:405`、`:418-423`、`:442-445`、`:455-461`
- Test: `tests/test_main.py`（新增一個 subprocess 測試）

**Interfaces:**
- Consumes: 無（第一個 task）
- Produces: 無新的公開介面。`_ClaudeClient` 新增私有屬性 `self._sdk`（即 `anthropic` 模組本身），供各方法的 `except` 子句取用。

`anthropic` 只在 `_ClaudeClient` 用到三個名稱（`Anthropic`、`APIConnectionError`、`APIStatusError`），加上 `_anthropic_detail` 的型別註解。把 import 搬進 `_ClaudeClient.__init__`，啟動路徑就完全不碰它。

- [ ] **Step 1: 寫失敗的測試**

加到 `tests/test_main.py` 的最後（檔案開頭補 `import subprocess`、`import sys`、`from pathlib import Path`）：

```python
ROOT = Path(__file__).resolve().parents[1]


def test_startup_path_does_not_import_anthropic():
    """啟動路徑不得載入 anthropic（約 0.7 秒，只有 Claude 官方 provider 用得到）。

    必須另起乾淨的直譯器問：同一個 process 內別的測試早就把 anthropic 載進來了。
    這條性質會被任何一次無心的 import 悄悄破壞，且不會有別的測試變紅。
    """
    code = "import src.main, sys; print('anthropic' in sys.modules)"
    out = subprocess.run([sys.executable, "-c", code], capture_output=True,
                         text=True, cwd=ROOT, check=True)
    assert out.stdout.strip() == "False", f"stdout={out.stdout!r} stderr={out.stderr!r}"
```

- [ ] **Step 2: 跑測試確認它失敗**

Run: `uv run pytest tests/test_main.py::test_startup_path_does_not_import_anthropic -v -n 0`
Expected: FAIL，`stdout='True\n'`

- [ ] **Step 3: 移除頂層 import**

`src/translation/translator.py` 第 17 行，刪掉這一行：

```python
import anthropic
```

（保留其下的 `import httpx`。）

- [ ] **Step 4: 改掉 `_anthropic_detail` 的型別註解**

第 405 行原本是：

```python
def _anthropic_detail(exc: anthropic.APIStatusError) -> str:
    """SDK 已把 body 解析成 dict（exc.body），取不到說明時退回 SDK 自己組的訊息。"""
```

改成（註解拿掉、改由 docstring 說明；本函式只存取 `exc.body` 與 `exc.message`，duck typing 就夠）：

```python
def _anthropic_detail(exc) -> str:
    """`anthropic.APIStatusError` → 一行錯誤說明。

    型別不寫進簽名：`anthropic` 已改成延遲 import（見 `_ClaudeClient.__init__`），
    而 3.11／3.12 會在函式定義時求值註解，寫了會 NameError。
    SDK 已把 body 解析成 dict（exc.body），取不到說明時退回 SDK 自己組的訊息。
    """
```

- [ ] **Step 5: 在 `_ClaudeClient.__init__` 裡延遲 import**

第 418-423 行原本是：

```python
    def __init__(self, model: str, api_key: str, effort: str = EFFORT_AUTO,
                 timeout: float = _TIMEOUT, client=None):
        self._client = client if client is not None else anthropic.Anthropic(
            api_key=api_key, timeout=timeout)
        self._model = model
        self._effort = effort
```

改成：

```python
    def __init__(self, model: str, api_key: str, effort: str = EFFORT_AUTO,
                 timeout: float = _TIMEOUT, client=None):
        # anthropic 的 import 要約 0.7 秒（大量 pydantic model 定義），而只有選 Claude
        # 官方 API 的使用者需要它 —— 放在這裡，啟動路徑就完全不碰。
        import anthropic
        self._sdk = anthropic
        self._client = client if client is not None else anthropic.Anthropic(
            api_key=api_key, timeout=timeout)
        self._model = model
        self._effort = effort
```

- [ ] **Step 6: 改掉 `chat()` 的兩個 `except` 子句**

第 442-445 行，把 `anthropic.` 換成 `self._sdk.`：

```python
        except (self._sdk.APIConnectionError, httpx.HTTPError, httpx.StreamError) as exc:
            raise TranslatorOffline(_one_line(str(exc))) from exc
        except self._sdk.APIStatusError as exc:
            error = _status_error(exc.status_code, _anthropic_detail(exc))
```

- [ ] **Step 7: 改掉 `list_models()` 的兩個 `except` 子句**

第 458-461 行同理：

```python
        except self._sdk.APIConnectionError as exc:
            raise TranslatorOffline(_one_line(str(exc))) from exc
        except self._sdk.APIStatusError as exc:
            error = _model_list_error(exc.status_code, _anthropic_detail(exc))
```

- [ ] **Step 8: 跑測試確認通過**

Run: `uv run pytest tests/test_main.py::test_startup_path_does_not_import_anthropic tests/test_translator.py -v -n 0`
Expected: 全部 PASS。`tests/test_translator.py` 自己 `import anthropic` 並注入 `FakeAnthropicClient`，不受影響 —— 它測的是行為，不是 import 時機。

- [ ] **Step 9: 量一下實際省了多少**

Run:
```bash
uv run python -c "import time; t=time.perf_counter(); import src.main; print('%.3f s' % (time.perf_counter()-t))"
```
Expected: 約 0.31 秒（改動前是 0.99 秒）。第一次跑若是冷的會偏高，再跑一次取 warm 值。

- [ ] **Step 10: Lint、全套測試、commit**

```bash
uv run ruff check src tests
uv run pytest
git add src/translation/translator.py tests/test_main.py
git commit -m "perf(translator): import anthropic lazily to cut ~0.7s off startup"
```

---

### Task 2: B —— 從打包內容剔除兩個死重

**Files:**
- Modify: `build.spec`（`Analysis` 的 `excludes`，以及 `Analysis` 之後過濾 `a.binaries`）

**Interfaces:**
- Consumes: 無
- Produces: 無程式介面。產出是更小的打包內容（少約 39 MB 未壓縮）。

| 項目 | 未壓縮 | 為什麼確定用不到 |
|---|---|---|
| `cv2\opencv_videoio_ffmpeg500_64.dll` | 30.88 MB | rapidocr 全套只用影像處理 API（`resize`／`cvtColor`／`findContours` 等），無一處碰 `VideoCapture`／`imshow` |
| `PIL\_avif.pyd` | 7.89 MB | 專案沒有任何 AVIF 用途 |

**注意：`tests/test_resources.py:51-53` 會斷言 `build.spec` 的內容**（`icon="src/assets/icon.ico"` 與 `("src/assets/*", "assets")` 兩行字串必須在）。本 task 不動這兩行，但改 spec 時別誤刪。

- [ ] **Step 1: 加 `excludes`**

`build.spec` 的 `Analysis(...)` 裡，把 `excludes=[]` 改成：

```python
    excludes=[
        # AVIF 解碼器（PIL/_avif.pyd，7.9 MB 未壓縮）：本專案只處理 PNG 與螢幕擷取。
        # 切斷 import 鏈通常 .pyd 就不會被收 —— 打包後要用 Step 4 確認它真的不在。
        "PIL.AvifImagePlugin",
    ],
```

- [ ] **Step 2: 在 `Analysis` 之後過濾 `a.binaries`**

緊接在 `a = Analysis(...)` 的結尾括號之後、`pyz = PYZ(a.pure)` 之前插入：

```python
# cv2 的 ffmpeg 解碼器（30.9 MB 未壓縮）：rapidocr 只用影像處理 API，不碰 VideoCapture。
# 它不是 Python 模組，`excludes` 管不到，而 `exclude_system_libraries()` 只對 POSIX
# 有效（只掃 /lib*、/usr/lib*），所以只能在 Analysis 之後從 binaries 濾掉。
a.binaries = [b for b in a.binaries if "opencv_videoio_ffmpeg" not in b[0]]
```

- [ ] **Step 3: 打包**

Run: `uv run pyinstaller build.spec --noconfirm`
Expected: 成功，`dist/Wizard101ChatTranslator.exe` 產生。這一步要數分鐘。

- [ ] **Step 4: 檢查兩個檔案真的不在打包內容裡**

Run:
```bash
uv run python -c "
from PyInstaller.archive.readers import CArchiveReader
r = CArchiveReader('dist/Wizard101ChatTranslator.exe')
names = list(r.toc)
bad = [n for n in names if 'ffmpeg' in n or '_avif' in n]
total = sum(e[2] for e in r.toc.values())
print('殘留:', bad or '無')
print('未壓縮總計: %.1f MB' % (total/1e6))
"
```
Expected: `殘留: 無`，未壓縮總計約 **245 MB**（改動前是 284.2 MB）。

若 `_avif` 仍在，把 Step 2 的過濾改成同時涵蓋它：

```python
_DEAD_WEIGHT = ("opencv_videoio_ffmpeg", "_avif")
a.binaries = [b for b in a.binaries if not any(k in b[0] for k in _DEAD_WEIGHT)]
```
改完重跑 Step 3、Step 4。

- [ ] **Step 5: 實機確認框選翻譯仍可用**

開著遊戲（登入進世界內），跑 `dist/Wizard101ChatTranslator.exe`，按框選熱鍵框一段有文字的畫面，確認辨識與翻譯正常。**這一步是本 task 的關鍵驗收** —— 排除 ffmpeg DLL 若害 `import cv2` 失敗，只有實跑才看得出來。

- [ ] **Step 6: Lint、測試、commit**

```bash
uv run ruff check src tests
uv run pytest
git add build.spec
git commit -m "chore(build): drop unused opencv ffmpeg and PIL avif binaries"
```

---

### Task 3: `src/splash.py` —— 啟動畫面的薄包裝

**Files:**
- Create: `src/splash.py`
- Test: `tests/test_splash.py`

**Interfaces:**
- Consumes: `src.log.log`
- Produces: `is_available() -> bool`、`update(text: str) -> None`、`close() -> None`。三者在任何環境都不拋例外。

`pyi_splash` 只存在於打包版、且只在 bootloader 帶了 splash 時可用。本模組的職責就是吞掉所有環境差異，讓呼叫端可以無條件呼叫。

- [ ] **Step 1: 寫失敗的測試**

Create `tests/test_splash.py`：

```python
"""啟動畫面包裝：沒有 pyi_splash 時要安靜不動，有的時候要確實轉呼叫過去。"""
from src import splash


class _FakeSplash:
    """假的 pyi_splash：記下收到的呼叫，或依 `fail` 拋出真實會遇到的例外。"""

    def __init__(self, fail=False):
        self.texts = []
        self.closed = False
        self._fail = fail

    def update_text(self, msg):
        if self._fail:
            raise ConnectionError("socket gone")
        self.texts.append(msg)

    def close(self):
        if self._fail:
            raise RuntimeError("this module is not initialized")
        self.closed = True


def test_no_ops_without_pyi_splash(monkeypatch):
    monkeypatch.setattr(splash, "_splash", None)
    monkeypatch.setattr(splash, "_closed", False)
    splash.update("anything")   # 開發模式的常態，不該拋例外
    splash.close()
    assert splash.is_available() is False


def test_forwards_to_pyi_splash(monkeypatch):
    fake = _FakeSplash()
    monkeypatch.setattr(splash, "_splash", fake)
    monkeypatch.setattr(splash, "_closed", False)
    splash.update("Loading components...")
    splash.close()
    assert fake.texts == ["Loading components..."]
    assert fake.closed is True
    assert splash.is_available() is True


def test_update_after_close_is_ignored(monkeypatch):
    """關掉之後再更新不該再碰 pyi_splash —— 首次執行精靈那條路徑就會這樣走，
    真的送出去只會換來一行誤導人的失敗 log。"""
    fake = _FakeSplash()
    monkeypatch.setattr(splash, "_splash", fake)
    monkeypatch.setattr(splash, "_closed", False)
    splash.close()
    splash.update("Starting...")
    assert fake.texts == []


def test_close_is_idempotent(monkeypatch):
    fake = _FakeSplash()
    monkeypatch.setattr(splash, "_splash", fake)
    monkeypatch.setattr(splash, "_closed", False)
    splash.close()
    fake.closed = False        # 第二次呼叫若真的轉過去，這裡會被改回 True
    splash.close()
    assert fake.closed is False


def test_failures_are_swallowed(monkeypatch):
    monkeypatch.setattr(splash, "_splash", _FakeSplash(fail=True))
    monkeypatch.setattr(splash, "_closed", False)
    splash.update("x")   # ConnectionError 不該逃出去
    splash.close()       # RuntimeError 同理
```

- [ ] **Step 2: 跑測試確認它失敗**

Run: `uv run pytest tests/test_splash.py -v -n 0`
Expected: FAIL，`ModuleNotFoundError: No module named 'src.splash'`

- [ ] **Step 3: 寫實作**

Create `src/splash.py`：

```python
"""啟動畫面：把 PyInstaller bootloader 的 splash 包成隨時可呼叫的三個函式。

`pyi_splash` 只存在於打包版、且只在 bootloader 帶了 splash 時可用 —— 開發模式
（`uv run run.py`）會 ImportError，沒帶 splash 會 RuntimeError，IPC socket 斷了會
ConnectionError。本模組把這些全部吞掉，呼叫端不必判斷環境、也不必包 try。

文字一律英文：解壓期間顯示的那句是打包時寫死的（那時 Python 還沒啟動，讀不到使用者
的介面語言），字族同樣是打包時定死、執行期換不了，中途換語言只會有跳躍感。

本模組刻意不在 import 時記 log：`run.py` 會在 `redirect_output()` 之前就呼叫 `update()`，
那時 windowed exe 的 stderr 還是 None，寫出去的行會直接消失。狀態改由 `main()` 在輸出
導向之後記一次（見 `is_available`）。
"""
from src.log import log

try:
    import pyi_splash as _splash
except ImportError:     # 開發模式，或打包時沒帶 splash
    _splash = None

_closed = False


def is_available() -> bool:
    """bootloader 是否帶了啟動畫面（打包版且 spec 有加 Splash 才為真）。"""
    return _splash is not None


def update(text: str) -> None:
    """更新啟動畫面的狀態文字。沒有啟動畫面、或已經關掉時什麼都不做。"""
    if _splash is None or _closed:
        return
    try:
        _splash.update_text(text)
    except Exception as exc:
        log(f"[splash] update failed: {type(exc).__name__}: {exc}")


def close() -> None:
    """關閉啟動畫面。沒有啟動畫面時什麼都不做；重複呼叫安全。"""
    global _closed
    if _splash is None or _closed:
        return
    _closed = True
    try:
        _splash.close()
    except Exception as exc:
        log(f"[splash] close failed: {type(exc).__name__}: {exc}")
```

- [ ] **Step 4: 跑測試確認通過**

Run: `uv run pytest tests/test_splash.py -v -n 0`
Expected: 5 passed

- [ ] **Step 5: Lint、全套測試、commit**

```bash
uv run ruff check src tests
uv run pytest
git add src/splash.py tests/test_splash.py
git commit -m "feat(splash): add a safe wrapper around the PyInstaller splash screen"
```

---

### Task 4: 接上呼叫點（`run.py` 與 `src/main.py`）

**Files:**
- Modify: `run.py`（整個檔案）
- Modify: `src/main.py`（imports、`main()` 的 523-546 行區段）
- Test: `tests/test_main.py`（新增一個測試）

**Interfaces:**
- Consumes: `src.splash.is_available()`、`src.splash.update(text)`、`src.splash.close()`（Task 3）
- Produces: 無新的公開介面。

`main()` 有三條路徑會離開啟動階段，每條都要關閉啟動畫面，否則它會一直浮在最上層。

- [ ] **Step 1: 寫失敗的測試**

加到 `tests/test_main.py`（檔案開頭補 `from src import main` 與 `from src.config import DEFAULT_CONFIG`，`import copy` 已經有了）：

```python
def test_splash_closes_before_focusing_an_existing_instance(monkeypatch):
    """第二份實例：啟動畫面要在既有視窗被喚起「之前」關掉，否則會蓋在它上面。

    這條路徑在建立任何視窗之前就 return，所以測得到；另外兩個出口（首次執行精靈、
    正常啟動）會進 Tk 與 mainloop，改由打包後的實機驗證涵蓋。
    """
    events = []
    monkeypatch.setattr(main, "redirect_output", lambda: None)
    monkeypatch.setattr(main, "load_config", lambda path: copy.deepcopy(DEFAULT_CONFIG))
    monkeypatch.setattr(main, "bootstrap_language", lambda cfg, existed: "en-US")
    monkeypatch.setattr(main, "set_language", lambda code: None)
    monkeypatch.setattr(main, "app_name", lambda: "Wizard101 Chat Translator")
    monkeypatch.setattr(main, "acquire_single_instance", lambda: None)
    monkeypatch.setattr(main.splash, "close", lambda: events.append("close"))
    monkeypatch.setattr(main, "focus_running_instance",
                        lambda title: events.append("focus") or False)

    main.main()

    assert events == ["close", "focus"]
```

- [ ] **Step 2: 跑測試確認它失敗**

Run: `uv run pytest tests/test_main.py::test_splash_closes_before_focusing_an_existing_instance -v -n 0`
Expected: FAIL，`AttributeError: module 'src.main' has no attribute 'splash'`

- [ ] **Step 3: 在 `src/main.py` 補 import**

在 `from src import __version__` 那行之後加一行（ruff 的 isort 會要求 `from src import ...` 合併，照它的建議走）：

```python
from src import __version__, splash
```

- [ ] **Step 4: 記一行啟動畫面狀態**

`main()` 開頭，`log(f"[app] version={__version__}")` 之後加：

```python
    log(f"[splash] startup screen {'active' if splash.is_available() else 'not present'} "
        f"(frozen={getattr(sys, 'frozen', False)})")
```

帶上 `frozen` 才分得出兩種「not present」：開發模式（預期如此）與打包版漏掉 splash（真的有問題）。

- [ ] **Step 5: 出口一 —— 第二份實例**

第 523-527 行，在 `focus_running_instance` 之前插入 `splash.close()`：

```python
    instance_lock = acquire_single_instance()
    if instance_lock is None:
        splash.close()   # 先關掉，否則會蓋在被喚起的既有視窗上
        focused = focus_running_instance(app_name())
        log(f"[app] another instance is already running (focused={focused}), exiting")
        return
```

- [ ] **Step 6: 出口二 —— 首次執行精靈**

第 536-544 行，在 `run_wizard` 之前插入 `splash.close()`：

```python
    if not is_configured(cfg):
        from src.ui.wizard import run_wizard
        splash.close()   # 精靈要跟使用者互動，啟動畫面不能擋在前面
        log("[app] config incomplete, launching first-run wizard")
```

- [ ] **Step 7: 出口三 —— 正常路徑**

第 546 行前後改成：

```python
    splash.update("Starting...")
    app = build_app(cfg, root, message_log)
    splash.close()
```

- [ ] **Step 8: 改寫 `run.py`**

整個檔案換成：

```python
"""啟動器：`uv run run.py`（等同 `python -m src.main`）。"""


def _run() -> None:
    """先把啟動畫面的文字換掉再 import —— `src.main` 的 import 要約 0.3 秒，
    這段時間畫面上還停在 bootloader 寫死的 `Initializing...`。

    兩個 import 都放在函式內：模組頂層的 import 會被 ruff 的 E402 擋下，而順序
    （先 update 再 import src.main）正是這裡的重點。
    """
    from src import splash
    splash.update("Loading components...")
    from src.main import main
    main()


if __name__ == "__main__":
    _run()
```

- [ ] **Step 9: 跑測試確認通過**

Run: `uv run pytest tests/test_main.py -v -n 0`
Expected: 全部 PASS

- [ ] **Step 10: 確認開發模式沒被弄壞**

Run: `uv run python -c "import run; print('run.py import ok')"`
Expected: `run.py import ok`（只驗 import，不啟動 GUI）

- [ ] **Step 11: Lint、全套測試、commit**

```bash
uv run ruff check src tests
uv run pytest
git add run.py src/main.py tests/test_main.py
git commit -m "feat(splash): drive the splash screen from the startup path"
```

---

### Task 5: `tools/splash_image.py` —— build 時生成底圖

**Files:**
- Create: `tools/splash_image.py`
- Test: `tests/test_splash_image.py`

**Interfaces:**
- Consumes: `src.resources.icon_path()`、`src.__version__`、`src.log.log`
- Produces: `SIZE: tuple[int, int]`（`(480, 300)`）、`build_splash_image(dest: Path) -> Path`（回傳實際寫出的路徑）

PyInstaller 的 Splash 只提供一行動態文字，所以名稱與版本號必須先畫進底圖。版本號從 `__version__` 帶入，維持版本號的單一真實來源。

**排版重點：狀態文字是左下錨點，不會置中。** PyInstaller 的 Tcl 模板用 `-anchor sw` 建立文字項目（`-justify center` 只管多行對齊），而狀態文字長度會變，沒有辦法置中。所以底圖把 icon、名稱、版本號置中，狀態文字則刻意安排成左下角的一行小字（像安裝程式那樣），視覺上是有意為之而非沒對齊。

- [ ] **Step 1: 寫失敗的測試**

Create `tests/test_splash_image.py`：

```python
"""啟動畫面底圖：build 時生成，尺寸、格式與版本號都要釘住。"""
from PIL import Image

from tools.splash_image import SIZE, build_splash_image


def test_generates_a_png_of_the_expected_size(tmp_path):
    dest = build_splash_image(tmp_path / "splash.png")
    assert dest.is_file()
    with Image.open(dest) as im:
        assert im.format == "PNG"
        assert im.size == SIZE


def test_creates_missing_parent_directories(tmp_path):
    """spec 會把它寫到 build/ 底下，乾淨的 checkout 上那個目錄還不存在。"""
    dest = build_splash_image(tmp_path / "nested" / "dir" / "splash.png")
    assert dest.is_file()


def test_version_is_drawn_into_the_image(tmp_path, monkeypatch):
    """版本號必須真的畫進去 —— 否則改版後圖上會留著舊版號，沒人看得出來。"""
    first = build_splash_image(tmp_path / "a.png").read_bytes()
    monkeypatch.setattr("tools.splash_image.__version__", "9.9.9")
    second = build_splash_image(tmp_path / "b.png").read_bytes()
    assert first != second, "版本號沒有畫進圖裡"
```

- [ ] **Step 2: 跑測試確認它失敗**

Run: `uv run pytest tests/test_splash_image.py -v -n 0`
Expected: FAIL，`ModuleNotFoundError: No module named 'tools'`

- [ ] **Step 3: 寫實作**

Create `tools/splash_image.py`：

```python
"""啟動畫面底圖：build 時生成，不進版控。

PyInstaller 的 Splash 只提供一行動態文字（單一 `text_pos`），所以名稱與版本號必須先
畫進底圖。版本號從 `src.__version__` 帶入，維持版本號的單一真實來源 —— 每次發布都會
變，不該有第二份要手改的圖。

狀態文字由 Tcl 以 `-anchor sw`（左下）畫上去、且長度會變，沒有辦法置中，所以這裡把它
的位置讓給左下角，icon 與標題則置中 —— 排版是配合那個限制設計的，不是沒對齊。
"""
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

from src import __version__
from src.log import log
from src.resources import icon_path

SIZE = (480, 300)
TEXT_ORIGIN = (24, 276)      # 狀態文字的左下錨點，build.spec 的 text_pos 要用同一組值

_TITLE = "Wizard101 Chat Translator"
# 底色避開 magenta（#ff00ff）—— Windows 版的 splash 拿它當透明色（見 PyInstaller 的
# transparent_setup 模板），用到的話整塊會被挖成透明。
_BACKGROUND = "#1b1b2f"
_TITLE_COLOR = "#f0f0f5"
_VERSION_COLOR = "#9a9ab0"
_ICON_SIZE = 128             # icon.ico 的原生 frame，不放大
_ICON_TOP = 40
_TITLE_TOP = 186
_VERSION_TOP = 220
# Windows 的 Segoe UI Semibold 檔名是 seguisb.ttf（不是 segoeuisb.ttf）。找不到就退
# 一般體、再退 Pillow 內建 —— 字型只影響美觀，不值得讓 release 掛掉。
_TITLE_FONTS = ("seguisb.ttf", "segoeui.ttf")
_VERSION_FONTS = ("segoeui.ttf",)


def _font(candidates: tuple[str, ...], size: int):
    for name in candidates:
        try:
            return ImageFont.truetype(name, size)
        except OSError:
            continue
    log(f"[build] none of {candidates} is installed; splash falls back to the default font")
    return ImageFont.load_default(size)


def _centered(draw: ImageDraw.ImageDraw, top: int, text: str, font, color: str) -> None:
    width = draw.textbbox((0, 0), text, font=font)[2]
    draw.text(((SIZE[0] - width) // 2, top), text, font=font, fill=color)


def build_splash_image(dest: Path) -> Path:
    """畫出底圖並存成 PNG，回傳實際寫出的路徑。

    失敗一律往上拋，讓 build 紅掉 —— 默默發出一個沒有啟動畫面的版本，比 build 失敗糟。
    """
    canvas = Image.new("RGB", SIZE, _BACKGROUND)
    icon = Image.open(icon_path())
    icon.size = (_ICON_SIZE, _ICON_SIZE)   # ICO 有多個 frame，指定要取的原生尺寸
    icon = icon.convert("RGBA")
    canvas.paste(icon, ((SIZE[0] - _ICON_SIZE) // 2, _ICON_TOP), icon)

    draw = ImageDraw.Draw(canvas)
    _centered(draw, _TITLE_TOP, _TITLE, _font(_TITLE_FONTS, 22), _TITLE_COLOR)
    _centered(draw, _VERSION_TOP, f"v{__version__}", _font(_VERSION_FONTS, 14),
              _VERSION_COLOR)

    dest.parent.mkdir(parents=True, exist_ok=True)
    canvas.save(dest, "PNG")
    log(f"[build] splash image written: path={dest} size={SIZE} version={__version__}")
    return dest
```

- [ ] **Step 4: 跑測試確認通過**

Run: `uv run pytest tests/test_splash_image.py -v -n 0`
Expected: 3 passed

- [ ] **Step 5: 肉眼看一下生成的圖**

Run:
```bash
uv run python -c "
from pathlib import Path
from tools.splash_image import build_splash_image
print(build_splash_image(Path('build/splash-preview.png')))
"
```
然後開 `build/splash-preview.png` 確認排版沒有重疊、字沒有被切掉。看完刪掉這張預覽圖。

- [ ] **Step 6: 把 `tools/` 納入 lint 範圍（9 處要一起改）**

`tools/` 是新的原始碼目錄，而既有的 lint 指令只掃 `src tests` —— 不改的話這個檔案永遠
不會被 ruff 檢查。`uv run ruff check src tests` 這條指令在版控中重複出現 9 次，全部要改
成 `uv run ruff check src tests tools`：

| 檔案 | 位置 |
|---|---|
| `.github/workflows/ci.yml` | 第 32 行 |
| `.github/workflows/release-windows.yml` | 第 77 行 |
| `CLAUDE.md` | 第 15 行（「優先用 `uv`」）、第 21 行（「提交前品質檢查」第 1 點） |
| `AGENTS.md` | 第 15 行、第 21 行（與 CLAUDE.md 逐字相同） |
| `README.md` | 第 149 行 |
| `README_ZH-TW.md` | 第 149 行 |
| `README_ZH-CN.md` | 第 149 行 |

兩點注意：

- **`CLAUDE.md` 與 `AGENTS.md` 必須逐字相同**，改完用 `diff CLAUDE.md AGENTS.md` 確認
  無輸出。
- 三份 README 的那一行**各自帶該語言的註解文字**（英／繁中／簡中），只改指令本身，
  不要把某一版的註解複製到另外兩份。
- `.superpowers/sdd/` 底下的 followup 報告也有同一條指令，但那些是**歷史執行紀錄，
  不要動**。

Run:
```bash
uv run ruff check src tests tools
diff CLAUDE.md AGENTS.md && echo "CLAUDE.md 與 AGENTS.md 一致"
```
Expected: `All checks passed!`，以及 `CLAUDE.md 與 AGENTS.md 一致`

- [ ] **Step 7: Lint、全套測試、commit**

```bash
uv run ruff check src tests tools
uv run pytest
git add tools/splash_image.py tests/test_splash_image.py \
        CLAUDE.md AGENTS.md README.md README_ZH-TW.md README_ZH-CN.md \
        .github/workflows/ci.yml .github/workflows/release-windows.yml
git commit -m "feat(build): generate the splash background image at build time"
```

---

### Task 6: `build.spec` 接上 Splash

**Files:**
- Modify: `build.spec`（imports、生成底圖、`Splash(...)`、`EXE(...)`）
- Test: `tests/test_resources.py`（既有的 spec 內容斷言要擴充）

**Interfaces:**
- Consumes: `tools.splash_image.build_splash_image`、`tools.splash_image.TEXT_ORIGIN`（Task 5）
- Produces: 帶啟動畫面的 exe。

- [ ] **Step 1: 寫失敗的測試**

加到 `tests/test_resources.py`（`BUILD_SPEC` 已經定義好了）：

```python
def test_build_spec_wires_up_the_splash_screen():
    """啟動畫面要同時進 EXE 的 splash 與 splash.binaries —— 少一個就不會顯示。"""
    spec = BUILD_SPEC.read_text(encoding="utf-8")
    assert "Splash(" in spec
    assert "splash," in spec
    assert "splash.binaries," in spec
    # 狀態文字必須是 ASCII：它會被寫進 bootloader 的 Tcl 腳本
    assert 'text_default="Initializing..."' in spec
```

- [ ] **Step 2: 跑測試確認它失敗**

Run: `uv run pytest tests/test_resources.py::test_build_spec_wires_up_the_splash_screen -v -n 0`
Expected: FAIL，`assert 'Splash(' in spec`

- [ ] **Step 3: 在 `build.spec` 補 import 與底圖生成**

`build.spec` 開頭的 `import sys` 改成：

```python
import sys
from pathlib import Path
```

然後在既有的 `from src import __version__` 之後加一行（兩者都必須在 `sys.path.insert(0, SPECPATH)` 之後）：

```python
from tools.splash_image import TEXT_ORIGIN, build_splash_image
```

在 `VERSION_INFO = VSVersionInfo(...)` 那一整段之後、`a = Analysis(...)` 之前插入：

```python
# 啟動畫面的底圖：每次 build 重新生成，版本號才不會是舊的（見 tools/splash_image.py）
SPLASH_IMAGE = build_splash_image(Path(SPECPATH) / "build" / "splash.png")
```

- [ ] **Step 4: 建立 `Splash` 目標**

在 `pyz = PYZ(a.pure)` 之前插入：

```python
splash = Splash(
    str(SPLASH_IMAGE),
    binaries=a.binaries,   # 讓它偵測到 tkinter 已經打包了 tcl/tk，沿用而不再塞一份
    datas=a.datas,
    text_pos=TEXT_ORIGIN,
    text_size=10,
    text_color="#f0f0f5",
    # 解壓期間顯示的字：打包時就寫死，那時 Python 還沒啟動、讀不到介面語言設定。
    # 一律 ASCII —— 這串字會被寫進 bootloader 的 Tcl 腳本。
    text_default="Initializing...",
    always_on_top=True,
)
```

- [ ] **Step 5: 把 splash 接進 `EXE`**

`EXE(...)` 的前幾個位置參數改成（onefile 模式要同時帶 `splash` 與 `splash.binaries`）：

```python
exe = EXE(
    pyz,
    a.scripts,
    splash,
    splash.binaries,
    a.binaries,
    a.datas,
    name="Wizard101ChatTranslator",
```

其餘參數（`icon`、`version`、`console`、`upx`）不動。

- [ ] **Step 6: 跑測試確認通過**

Run: `uv run pytest tests/test_resources.py -v -n 0`
Expected: 全部 PASS（既有的 icon 相關斷言也要還在）

- [ ] **Step 7: 打包**

Run: `uv run pyinstaller build.spec --noconfirm`
Expected: 成功。build log 裡應該看得到 `[build] splash image written: ...`。

- [ ] **Step 8: Lint、全套測試、commit**

```bash
uv run ruff check src tests tools
uv run pytest
git add build.spec tests/test_resources.py
git commit -m "feat(build): show a splash screen while the onefile exe unpacks"
```

---

### Task 7: 打包後的實機驗證

**Files:** 無（驗證 task，不改程式碼）

**Interfaces:**
- Consumes: Task 1-6 的全部成果
- Produces: 通過或不通過的判定。不通過就回頭改對應的 task。

**前兩項是 PyInstaller 官方記載的已知問題，先驗它們** —— 若焦點問題無解，應放棄 Task 3、4、6（啟動畫面），保留 Task 1、2（A 與 B 的收益不受影響）。

- [ ] **Step 1: 確認手上是最新的 exe**

Run: `uv run pyinstaller build.spec --noconfirm`

- [ ] **Step 2: 驗焦點（已知問題一）**

開著遊戲（登入進世界內），跑 `dist/Wizard101ChatTranslator.exe`。
確認：啟動畫面關閉之後，**疊加視窗仍在遊戲畫面之上**，沒有被丟到遊戲背後。
不通過 → 記錄現象，回報後再決定是否放棄啟動畫面。

- [ ] **Step 3: 驗圖示（已知問題二）**

確認工作列按鈕與視窗標題列的圖示正常（不是預設的空白圖示、也不是模糊的縮圖）。
對照 `app.log` 裡的 `[ui] window icon applied: ...` 那一行有沒有出現、有沒有改成 failed。

- [ ] **Step 4: 驗首次執行精靈的出口**

把 exe 旁的 `config.json` 改名成 `config.json.bak`，再跑一次 exe。
確認：啟動畫面在精靈視窗出現**之前**就關掉了，沒有蓋在精靈上面。
驗完把 `config.json.bak` 改回 `config.json`。

- [ ] **Step 5: 驗第二實例的出口**

exe 還開著的時候，再雙擊一次 exe。
確認：第二份的啟動畫面關掉、既有視窗被帶到前景，畫面上沒有殘留一個關不掉的啟動畫面。

- [ ] **Step 6: 驗框選翻譯（Task 2 的排除無害）**

按框選熱鍵框一段有文字的畫面，確認辨識與翻譯都正常。

- [ ] **Step 7: 量前後的啟動時間**

從 `app.log` 取最後一段 session，比對 `[app] version=` 與 `[app] running` 兩行的時戳；再記一下自己從雙擊到看見疊加視窗的體感秒數。
Expected: 整體 warm 約 2.0 秒（改動前約 2.8 秒），且**全程有畫面**。

- [ ] **Step 8: 把驗證結果記進 spec**

在 `docs/superpowers/specs/2026-09-16-faster-exe-startup-design.md` 末尾補一節「事後修訂」（沿用 `2026-09-15-region-translation-design.md` 的作法），寫下實機驗證的結果，特別是兩個已知問題到底有沒有發生。

```bash
git add docs/superpowers/specs/2026-09-16-faster-exe-startup-design.md
git commit -m "docs(spec): record real-run verification results for exe startup work"
```

---

## 附錄：本次改動的預期數字

| 項目 | 改動前 | 改動後 |
|---|---|---|
| exe 未壓縮內容 | 284.2 MB | 約 245 MB |
| bootloader 解壓 | 1.35 s | 約 1.17 s |
| `import src.main` | 0.99 s | 約 0.31 s |
| `[app] version=` → `[app] running` | 0.47 s | 0.47 s（不變） |
| **合計（warm）** | **約 2.8 s** | **約 2.0 s** |
| 使用者看到畫面的時間點 | 約 2.8 s（疊加視窗） | **約 0 s（啟動畫面）** |

選 Claude 官方 provider 的使用者省不到 `anthropic` 那 0.68 秒 —— 成本只是從 import 階段移到 `build_app()`，那時啟動畫面已顯示 `Starting...`。
