# 版本更新檢查與「關於」分頁 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 啟動時在背景查 GitHub 最新 release，有新版就在疊加視窗顯示可點擊的更新橫幅；設定視窗新增「關於」分頁，提供手動檢查更新與專案／開發者／紀錄檔資訊。

**Architecture:** 新增 `src/updater.py` 單一模組回答「有沒有比目前這版更新的 release」（純函式的版本解析比較 ＋ 一支打 GitHub API 的函式），啟動路徑與設定視窗兩個入口都呼叫它。UI 端：`overlay` 新增一條與錯誤橫幅並排、獨立的更新橫幅；`settings` 新增第三個分頁。背景檢查一律走既有的「daemon 執行緒 ＋ `ui_queue`／`poll_queue`」模式，不從背景執行緒碰 tkinter。

**Tech Stack:** Python 3.11+、tkinter／ttk、httpx（既有相依）、pytest、uv。

**Spec:** `docs/superpowers/specs/2026-08-31-update-check-design.md`

## Global Constraints

- **測試**：每個 task 結束前 `uv run pytest` 全綠；新測試一律離線可跑（網路以假 client／假 checker 注入，不打真的 GitHub API）。
- **log 訊息一律英文**，前綴用模組名（本功能一律 `[update]`，UI 端既有的用 `[ui]`），內容帶足夠 context（狀態碼、版本號、例外訊息）。寫入 `sys.stderr`。
- **UI 文字一律走 `src.i18n.t()`**，禁止在 `src/ui/*.py`、`src/reader/overlay.py`、`src/main.py` 出現中日韓字串字面量（含全形標點如 `：`）——`tests/test_no_hardcoded_ui_text.py` 會掃描並擋下。
- **新增 i18n key 必須三份語言檔（`zh-TW`／`zh-CN`／`en`）同步補齊**，`tests/test_i18n.py` 會檢查 key 集合一致與變數未被翻壞。`zh-TW` 是來源語言。
- **註解與 docstring 用繁體中文全形標點**；模組與公開 function／class 要有 docstring。實作層 `#` 註解只在「WHY 不顯而易見」或「長函式的段落導引」時才寫。
- **commit message 一律英文**，conventional commits 格式（`feat(...)`／`fix(...)`／`docs(...)`／`refactor(...)`），subject 簡潔。**不要 `git push`。**
- **敏感資料絕不進 log**（本功能不碰金鑰，但既有規則仍適用）。
- **YAGNI**：不加「關閉自動檢查」的設定開關、不做定時輪詢、不做自動下載。
- 執行指令一律走 uv：`uv run pytest`、`uv run pytest tests/test_updater.py -v`、`uv run run.py`。

---

### Task 1: `src/updater.py` 的版本解析與比較（純函式）

**Files:**
- Create: `src/updater.py`
- Test: `tests/test_updater.py`（新建）

**Interfaces:**
- Consumes: `src.__version__`（版本唯一真實來源）
- Produces: `GITHUB_REPO`、`PROJECT_URL`、`RELEASES_URL`、`AUTHOR_URL`、`LATEST_API` 常數；`parse_version(text: str) -> tuple[int, int, int] | None`；`is_newer(latest: str, current: str) -> bool`

- [ ] **Step 1: 寫失敗的測試**

建立 `tests/test_updater.py`：

```python
"""更新檢查：版本解析比較（純函式）與 GitHub API 取用（假 client）。"""
from src.updater import is_newer, parse_version


def test_parse_version_accepts_plain_and_v_prefixed():
    assert parse_version("0.1.0") == (0, 1, 0)
    assert parse_version("v0.2.3") == (0, 2, 3)
    assert parse_version(" v1.20.300 ") == (1, 20, 300)


def test_parse_version_ignores_suffixes():
    # /releases/latest 已排除 pre-release，但 tag 本身仍可能帶後綴
    assert parse_version("v0.2.0-beta.1") == (0, 2, 0)
    assert parse_version("0.2.0+build.5") == (0, 2, 0)


def test_parse_version_rejects_unparsable_text():
    for text in ("", "latest", "v1.2", "1.2.x", "release-2026"):
        assert parse_version(text) is None


def test_is_newer_compares_numerically():
    assert is_newer("0.2.0", "0.1.0") is True
    assert is_newer("v0.10.0", "0.9.9") is True      # 字串比大小會判錯的例子
    assert is_newer("0.1.0", "0.1.0") is False
    assert is_newer("0.1.0", "0.2.0") is False


def test_is_newer_is_false_when_either_side_is_unparsable():
    # 寧可漏提醒也不要誤報：橫幅會把使用者導去下載頁
    assert is_newer("nightly", "0.1.0") is False
    assert is_newer("0.2.0", "unknown") is False
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `uv run pytest tests/test_updater.py -v`
Expected: FAIL — `ModuleNotFoundError: No module named 'src.updater'`

- [ ] **Step 3: 寫最小實作**

建立 `src/updater.py`：

```python
"""更新檢查：回答「GitHub 上有沒有比目前這版更新的 release」。

只負責查詢與比較，不碰 UI、不碰 config，也不下載或安裝任何東西——要不要提醒、
怎麼提醒由呼叫端決定（啟動路徑走 overlay 橫幅，設定視窗走「關於」分頁）。
"""
import re
import sys

from src import __version__

GITHUB_REPO = "GoneTone/wizard101-chat-translator"
PROJECT_URL = f"https://github.com/{GITHUB_REPO}"
RELEASES_URL = f"{PROJECT_URL}/releases/latest"
AUTHOR_URL = "https://github.com/GoneTone"
LATEST_API = f"https://api.github.com/repos/{GITHUB_REPO}/releases/latest"

# 只取前三段數字，後綴（-beta.1、+build）一律忽略：/releases/latest 已排除
# pre-release，這裡容忍後綴只是為了不因為 tag 寫法而整個解析失敗。
_VERSION_RE = re.compile(r"^v?(\d+)\.(\d+)\.(\d+)")


def parse_version(text: str) -> tuple[int, int, int] | None:
    """把 `v0.2.0` 之類的版本字串解析成可比較的三段整數；解析不出來回 None。"""
    match = _VERSION_RE.match(text.strip()) if text else None
    if match is None:
        return None
    major, minor, patch = match.groups()
    return int(major), int(minor), int(patch)


def is_newer(latest: str, current: str) -> bool:
    """latest 是否嚴格新於 current。

    任一邊解析不出來就回 False——寧可漏提醒也不要誤報，畢竟提醒會把使用者
    導去下載頁。"""
    newer, mine = parse_version(latest), parse_version(current)
    if newer is None or mine is None:
        print(f"[update] version unparsable: latest={latest!r} current={current!r}",
              file=sys.stderr)
        return False
    return newer > mine
```

- [ ] **Step 4: 執行測試確認通過**

Run: `uv run pytest tests/test_updater.py -v`
Expected: PASS（5 passed）

- [ ] **Step 5: 全測試 ＋ commit**

Run: `uv run pytest`
Expected: 全綠

```bash
git add src/updater.py tests/test_updater.py
git commit -m "feat(updater): add semver parsing and comparison"
```

---

### Task 2: 取得 GitHub 最新 release

**Files:**
- Modify: `src/updater.py`（接在 Task 1 的內容之後）
- Test: `tests/test_updater.py`（接在 Task 1 的測試之後）

**Interfaces:**
- Consumes: Task 1 的 `parse_version`／`is_newer`／`LATEST_API`／`RELEASES_URL`
- Produces:
  - `class Release`（frozen dataclass）：`version: str`（已去 `v` 前綴）、`url: str`
  - `class UpdateCheckError(Exception)`
  - `fetch_latest_release(client=None) -> Release | None`（404 回 `None`，其他失敗拋 `UpdateCheckError`）
  - `check_for_update(current: str = __version__, client=None) -> Release | None`

- [ ] **Step 1: 寫失敗的測試**

在 `tests/test_updater.py` 頂端把 import 改成：

```python
"""更新檢查：版本解析比較（純函式）與 GitHub API 取用（假 client）。"""
import httpx
import pytest

from src.updater import (LATEST_API, RELEASES_URL, Release, UpdateCheckError,
                         check_for_update, fetch_latest_release, is_newer,
                         parse_version)
```

在檔案末端追加：

```python
_TAG_URL = "https://github.com/GoneTone/wizard101-chat-translator/releases/tag/v0.2.0"


class FakeResponse:
    def __init__(self, status_code=200, payload=None):
        self.status_code = status_code
        self._payload = {"tag_name": "v0.2.0", "html_url": _TAG_URL} \
            if payload is None else payload

    def json(self):
        if self._payload is _BROKEN_JSON:
            raise ValueError("not json")
        return self._payload


_BROKEN_JSON = object()


class FakeClient:
    """只認 get 的假 httpx client；記下被打的網址與標頭供斷言。"""

    def __init__(self, response=None, raises=None):
        self._response = response if response is not None else FakeResponse()
        self._raises = raises
        self.calls = []

    def get(self, url, headers=None):
        self.calls.append((url, headers))
        if self._raises is not None:
            raise self._raises
        return self._response


def test_fetch_latest_release_reads_tag_and_url():
    client = FakeClient()
    release = fetch_latest_release(client=client)
    assert release == Release(version="0.2.0", url=_TAG_URL)
    url, headers = client.calls[0]
    assert url == LATEST_API
    assert headers["Accept"] == "application/vnd.github+json"
    assert "wizard101-chat-translator" in headers["User-Agent"]


def test_fetch_latest_release_returns_none_when_no_release_exists():
    # 404＝尚未發過任何 release（也涵蓋 repo 尚未公開）：不是錯誤
    assert fetch_latest_release(client=FakeClient(FakeResponse(status_code=404))) is None


@pytest.mark.parametrize("status", [403, 429, 500])
def test_fetch_latest_release_raises_on_other_status_codes(status):
    with pytest.raises(UpdateCheckError) as exc:
        fetch_latest_release(client=FakeClient(FakeResponse(status_code=status)))
    assert str(status) in str(exc.value)


def test_fetch_latest_release_raises_when_connection_fails():
    client = FakeClient(raises=httpx.ConnectError("no route to host"))
    with pytest.raises(UpdateCheckError):
        fetch_latest_release(client=client)


def test_fetch_latest_release_raises_on_unusable_payload():
    with pytest.raises(UpdateCheckError):
        fetch_latest_release(client=FakeClient(FakeResponse(payload={})))
    with pytest.raises(UpdateCheckError):
        fetch_latest_release(client=FakeClient(FakeResponse(payload=_BROKEN_JSON)))


def test_fetch_latest_release_falls_back_to_releases_page_without_html_url():
    client = FakeClient(FakeResponse(payload={"tag_name": "v0.3.0"}))
    assert fetch_latest_release(client=client) == Release(version="0.3.0",
                                                          url=RELEASES_URL)


def test_check_for_update_returns_release_only_when_newer():
    client = FakeClient()
    assert check_for_update(current="0.1.0", client=client).version == "0.2.0"
    assert check_for_update(current="0.2.0", client=client) is None
    assert check_for_update(current="9.9.9", client=client) is None


def test_check_for_update_returns_none_when_no_release_exists():
    client = FakeClient(FakeResponse(status_code=404))
    assert check_for_update(current="0.1.0", client=client) is None
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `uv run pytest tests/test_updater.py -v`
Expected: FAIL — `ImportError: cannot import name 'Release' from 'src.updater'`

- [ ] **Step 3: 寫最小實作**

`src/updater.py` 的 import 區補上：

```python
import re
import sys
from dataclasses import dataclass

import httpx

from src import __version__
```

在常數區補上（接在 `LATEST_API` 之後）：

```python
_TIMEOUT = 10.0   # 啟動路徑不等它，但也不能讓手動檢查的按鈕卡住不放
# GitHub API 要求帶 User-Agent，缺了會被拒（403）
_HEADERS = {"Accept": "application/vnd.github+json",
            "User-Agent": f"wizard101-chat-translator/{__version__}"}
```

在檔案末端追加：

```python
@dataclass(frozen=True)
class Release:
    """GitHub 上的一個 release：版本號（已去 `v` 前綴）與該 release 的網頁網址。"""
    version: str
    url: str


class UpdateCheckError(Exception):
    """檢查更新失敗（連線不通、配額用盡、回應無法解析）。

    「目前沒有新版」與「還沒發過任何 release」都不走這個例外——那是正常結果。"""


def _fetch(http) -> Release | None:
    try:
        resp = http.get(LATEST_API, headers=_HEADERS)
    except httpx.HTTPError as exc:
        raise UpdateCheckError(str(exc)) from exc
    if resp.status_code == 404:
        print("[update] no release published yet (404)", file=sys.stderr)
        return None
    if resp.status_code != 200:
        raise UpdateCheckError(f"HTTP {resp.status_code}")
    try:
        data = resp.json()
        tag = str(data["tag_name"])
        url = str(data.get("html_url") or RELEASES_URL)
    except (ValueError, TypeError, KeyError, AttributeError) as exc:
        raise UpdateCheckError(f"unexpected response: {exc}") from exc
    return Release(version=tag.lstrip("vV"), url=url)


def fetch_latest_release(client=None) -> Release | None:
    """取 GitHub 上最新的正式 release；尚未發過任何 release（404）時回 None。

    `/releases/latest` 已自動排除 draft 與 pre-release。`client` 供測試注入假
    client（沿用 translator 的同名慣例）；沒給就自己開一個用完即關的 httpx client。"""
    if client is not None:
        return _fetch(client)
    with httpx.Client(timeout=_TIMEOUT) as http:
        return _fetch(http)


def check_for_update(current: str = __version__, client=None) -> Release | None:
    """回傳「比 current 新的 release」，沒有新版（含尚未發版）時回 None。
    查詢失敗拋 UpdateCheckError，由呼叫端決定要顯示錯誤還是靜默。"""
    print("[update] checking latest release", file=sys.stderr)
    release = fetch_latest_release(client)
    if release is None:
        return None
    newer = is_newer(release.version, current)
    print(f"[update] latest={release.version} current={current} newer={newer}",
          file=sys.stderr)
    return release if newer else None
```

- [ ] **Step 4: 執行測試確認通過**

Run: `uv run pytest tests/test_updater.py -v`
Expected: PASS（含 parametrize 共 15 passed）

- [ ] **Step 5: 全測試 ＋ commit**

Run: `uv run pytest`

```bash
git add src/updater.py tests/test_updater.py
git commit -m "feat(updater): fetch the latest GitHub release"
```

---

### Task 3: 疊加視窗的更新橫幅

**Files:**
- Modify: `src/reader/overlay.py`（`import` 區、`__init__` 的狀態欄位、`_on_canvas_configure`、`set_error` 之後新增方法、`refresh_labels`、測試輔助區）
- Modify: `src/i18n/zh-TW.json`、`src/i18n/zh-CN.json`、`src/i18n/en.json`
- Test: `tests/test_overlay.py`

**Interfaces:**
- Consumes: Task 2 的 `Release`
- Produces: `OverlayWindow.set_update(release)`、`OverlayWindow.clear_update()`、`OverlayWindow.update_text() -> str | None`、常數 `FG_UPDATE`

- [ ] **Step 1: 補三份語言檔的 `update.available`**

三份檔案都把新 key 插在 `notice.config_error` 之後（`status.locating` 之前），保持三份順序一致：

`src/i18n/zh-TW.json`

```json
  "update.available": "⬆  有新版本 v{version}，點此下載",
```

`src/i18n/zh-CN.json`

```json
  "update.available": "⬆  有新版本 v{version}，点此下载",
```

`src/i18n/en.json`

```json
  "update.available": "⬆  Version {version} is available — click to download",
```

- [ ] **Step 2: 寫失敗的測試**

在 `tests/test_overlay.py` 末端追加：

```python
def test_set_update_shows_a_clickable_banner(root):
    from src.updater import Release

    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    assert ov.update_text() is None
    ov.set_update(Release(version="0.2.0", url="https://example.invalid/rel"))
    assert ov.update_text() == t("update.available", version="0.2.0")
    assert ov._update_label.cget("cursor") == "hand2"
    ov.clear_update()
    assert ov.update_text() is None


def test_update_banner_and_error_banner_coexist(root):
    # 兩者生命週期完全不同（錯誤隨狀態來去、更新是一次性），不該互相覆蓋
    from src.updater import Release

    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.set_error("notice.offline")
    ov.set_update(Release(version="0.2.0", url="https://example.invalid/rel"))
    assert ov.error_text() == t("notice.offline")
    assert ov.update_text() == t("update.available", version="0.2.0")
    ov.clear_update()
    assert ov.error_text() == t("notice.offline")   # 關掉更新橫幅不影響錯誤橫幅


def test_update_banner_follows_language_and_width(root):
    from src import i18n
    from src.updater import Release

    before = i18n.current_language()
    try:
        i18n.set_language("zh-TW")
        ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
        ov.set_update(Release(version="0.2.0", url="https://example.invalid/rel"))
        i18n.set_language("en")
        ov.refresh_labels()
        assert ov.update_text() == "⬆  Version 0.2.0 is available — click to download"

        class FakeEvent:
            width = 240

        ov._on_canvas_configure(FakeEvent())
        assert ov._update_label.cget("wraplength") == max(80, 240 - 12)
    finally:
        i18n.set_language(before)
```

- [ ] **Step 3: 執行測試確認失敗**

Run: `uv run pytest tests/test_overlay.py -k update -v`
Expected: FAIL — `AttributeError: 'OverlayWindow' object has no attribute 'update_text'`

- [ ] **Step 4: 寫實作**

`src/reader/overlay.py` 的 import 區補一行（依字母序放在既有 import 之間）：

```python
import webbrowser
```

顏色常數區（`FG_ERROR = "#ff5f5f"` 之後）補：

```python
# 更新橫幅用連結藍。fields.py 的 #4a7ddc 是給淺色設定視窗用的，放在 overlay 的
# 深色底（#101018）上會暗到看不出是可點的連結，故另取一個亮一階的藍。
FG_UPDATE = "#6fa8ff"
```

`__init__` 中緊接 `self._error_key` 那幾行補上狀態欄位：

```python
        self._update_row: tk.Frame | None = None
        self._update_label: tk.Label | None = None
        self._update_release = None   # 目前橫幅對應的 Release，語言切換後重繪用
```

`_on_canvas_configure` 內既有的錯誤橫幅換行寬度更新之後，補上同樣的一段（第 744 行附近）：

```python
        if self._update_label is not None:
            self._update_label.configure(wraplength=self._wrap)
```

`set_error`／`clear_error` 之後新增：

```python
    def set_update(self, release) -> None:
        """顯示更新橫幅：整列可點（開瀏覽器到下載頁），右側 ✕ 只關掉這一次。

        與錯誤橫幅各佔一列、互不覆蓋：錯誤橫幅隨遊戲與翻譯狀態自動來去，這條則是
        一次性的告知，兩者可能同時該被看到。release 存起來，換語言時才重繪得出來。"""
        self.clear_update()
        self._update_release = release
        row = tk.Frame(self._frame, bg=BG)
        close = tk.Label(row, text="✕", bg=BG, fg=FG_UPDATE, font=ui_font(9),
                         cursor="hand2")
        # ✕ 先 pack：expand=True 的文字若先宣告會吃光整列寬度，把它擠出畫面
        close.pack(side="right", padx=(4, 6))
        close.bind("<Button-1>", lambda e: self.clear_update())
        label = tk.Label(row, text=t("update.available", version=release.version),
                         bg=BG, fg=FG_UPDATE, font=ui_font(10, "bold"), anchor="w",
                         cursor="hand2", wraplength=self._wrap)
        label.pack(side="left", fill="x", expand=True)
        label.bind("<Button-1>", lambda e: webbrowser.open(release.url))
        # before＝捲動區：與錯誤橫幅同理，排在 expand=True 的捲動區之後會在視窗
        # 被縮小時被擠掉。
        row.pack(side="bottom", fill="x", pady=2, before=self._scroll_area)
        self._update_row = row
        self._update_label = label
        print(f"[update] banner shown for {release.version}", file=sys.stderr)

    def clear_update(self) -> None:
        self._update_release = None
        if self._update_row is not None:
            self._update_row.destroy()
            self._update_row = None
            self._update_label = None
```

`refresh_labels` 末端（`if self._error_key is not None:` 那段之後）補：

```python
        if self._update_release is not None:
            self.set_update(self._update_release)
```

測試輔助區（`error_text` 之後）補：

```python
    def update_text(self) -> str | None:
        return self._update_label.cget("text") if self._update_label else None
```

確認 `overlay.py` 檔案頂端已有 `import sys`（`set_update` 的 log 需要）；若沒有就補上。

- [ ] **Step 5: 執行測試確認通過**

Run: `uv run pytest tests/test_overlay.py tests/test_i18n.py tests/test_no_hardcoded_ui_text.py -v`
Expected: PASS

- [ ] **Step 6: 全測試 ＋ commit**

Run: `uv run pytest`

```bash
git add src/reader/overlay.py src/i18n tests/test_overlay.py
git commit -m "feat(overlay): add a clickable update banner"
```

---

### Task 4: 啟動時在背景檢查更新

**Files:**
- Modify: `src/main.py`（import 區、新增 `announce_update`、`main()` 內啟動執行緒）
- Test: `tests/test_updater.py`

**Interfaces:**
- Consumes: Task 2 的 `check_for_update`／`UpdateCheckError`、Task 3 的 `overlay.set_update`
- Produces: `src.main.announce_update(ui_queue, overlay, checker=check_for_update) -> None`

- [ ] **Step 1: 寫失敗的測試**

在 `tests/test_updater.py` 末端追加：

```python
def test_announce_update_queues_the_banner_when_newer_exists():
    import queue as queue_module

    from src.main import announce_update

    class FakeOverlay:
        def __init__(self):
            self.shown = []

        def set_update(self, release):
            self.shown.append(release)

    release = Release(version="0.2.0", url=_TAG_URL)
    ui_queue = queue_module.Queue()
    overlay = FakeOverlay()

    announce_update(ui_queue, overlay, checker=lambda: release)

    # 背景執行緒只把回呼排進 ui_queue，由主執行緒取出後才碰 tkinter
    assert overlay.shown == []
    ui_queue.get_nowait()()
    assert overlay.shown == [release]


def test_announce_update_stays_quiet_without_a_newer_release():
    import queue as queue_module

    from src.main import announce_update

    ui_queue = queue_module.Queue()
    announce_update(ui_queue, object(), checker=lambda: None)
    assert ui_queue.empty()


def test_announce_update_swallows_check_failures():
    # 檢查更新失敗絕不能影響啟動與收訊
    import queue as queue_module

    from src.main import announce_update

    def boom():
        raise UpdateCheckError("HTTP 403")

    ui_queue = queue_module.Queue()
    announce_update(ui_queue, object(), checker=boom)
    assert ui_queue.empty()
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `uv run pytest tests/test_updater.py -k announce -v`
Expected: FAIL — `ImportError: cannot import name 'announce_update' from 'src.main'`

- [ ] **Step 3: 寫實作**

`src/main.py` 的 import 區補（放在 `from src.translator import Translator` 一帶，維持字母序）：

```python
from src.updater import check_for_update
```

在 `drain_ui_queue` 之後新增：

```python
def announce_update(ui_queue: queue.Queue, overlay, checker=check_for_update) -> None:
    """檢查更新，有新版就把橫幅回呼排進 ui_queue（供背景執行緒呼叫）。

    任何失敗都只留 log：更新檢查是附加功能，不能影響啟動與收訊。`checker` 可注入
    是為了測試，正式路徑用預設的 check_for_update。"""
    try:
        release = checker()
    except Exception as exc:
        print(f"[update] check failed: {exc}", file=sys.stderr)
        return
    if release is None:
        return
    ui_queue.put(lambda: overlay.set_update(release))
```

`main()` 內，`reader_thread.start()` 之後補：

```python
    # 更新檢查另開一條 daemon 執行緒：網路慢或不通都不該拖住啟動，關閉程式時也
    # 不等它（結果只是一條橫幅，丟掉無妨）。
    threading.Thread(target=announce_update, args=(ui_queue, overlay),
                     daemon=True).start()
```

- [ ] **Step 4: 執行測試確認通過**

Run: `uv run pytest tests/test_updater.py -v`
Expected: PASS

- [ ] **Step 5: 全測試 ＋ commit**

Run: `uv run pytest`

```bash
git add src/main.py tests/test_updater.py
git commit -m "feat(app): check for a newer release on startup"
```

---

### Task 5: 連結標籤共用元件

**Files:**
- Modify: `src/ui/fields.py`（新增 `LINK_COLOR` 與 `link_label`，並讓既有的「取得金鑰」連結改用它；原始碼約在第 439-442 行）
- Test: `tests/test_fields.py`

**Interfaces:**
- Produces: `src.ui.fields.LINK_COLOR`、`src.ui.fields.link_label(parent, text: str, url: str) -> ttk.Label`

- [ ] **Step 1: 寫失敗的測試**

在 `tests/test_fields.py` 末端追加：

```python
def test_link_label_opens_the_url_on_click(root, monkeypatch):
    import tkinter as tk

    from src.ui import fields
    from src.ui.fields import LINK_COLOR, link_label

    opened = []
    monkeypatch.setattr(fields.webbrowser, "open", opened.append)
    holder = tk.Frame(root)
    label = link_label(holder, "GoneTone", "https://example.invalid/author")

    assert label.cget("text") == "GoneTone"
    assert str(label.cget("foreground")) == LINK_COLOR
    assert label.cget("cursor") == "hand2"
    label.event_generate("<Button-1>")
    root.update()
    assert opened == ["https://example.invalid/author"]
    holder.destroy()
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `uv run pytest tests/test_fields.py -k link_label -v`
Expected: FAIL — `ImportError: cannot import name 'LINK_COLOR' from 'src.ui.fields'`

- [ ] **Step 3: 寫實作**

`src/ui/fields.py`，在 `LABEL_WIDTH` 常數附近新增：

```python
LINK_COLOR = "#4a7ddc"   # 可點連結的字色（設定視窗與精靈共用）
```

在 `filter_models` 之後（純函式區）新增：

```python
def link_label(parent, text: str, url: str) -> ttk.Label:
    """藍字可點的連結標籤：點擊以系統瀏覽器開啟 url。"""
    label = ttk.Label(parent, text=text, foreground=LINK_COLOR, cursor="hand2")
    label.bind("<Button-1>", lambda e: webbrowser.open(url))
    return label
```

把既有「取得金鑰」連結那三行（`link = ttk.Label(self._fields, ...)` 起）換成：

```python
            link_label(self._fields, t("link.get_key"), prov.key_url).pack(
                anchor="w", pady=(2, 0))
```

- [ ] **Step 4: 執行測試確認通過**

Run: `uv run pytest tests/test_fields.py -v`
Expected: PASS

- [ ] **Step 5: 全測試 ＋ commit**

Run: `uv run pytest`

```bash
git add src/ui/fields.py tests/test_fields.py
git commit -m "refactor(ui): extract a shared link label widget"
```

---

### Task 6: 設定視窗「關於」分頁

**Files:**
- Modify: `src/i18n/zh-TW.json`、`src/i18n/zh-CN.json`、`src/i18n/en.json`
- Modify: `src/ui/settings.py`（import 區、`__init__` 簽名與欄位、`open()` 內新增分頁、新增 `_build_about`／`_start_update_check`／`_update_check_worker`／`_on_update_checked`／`_open_log_folder`）
- Test: `tests/test_settings.py`

**Interfaces:**
- Consumes: Task 2 的 `check_for_update`／`UpdateCheckError`／`PROJECT_URL`／`AUTHOR_URL`、Task 5 的 `link_label`、既有的 `fields.poll_queue`、`config.app_dir`
- Produces: `SettingsWindow(..., check_update=check_for_update)` 建構參數（測試注入用）；分頁索引 2 為「關於」

- [ ] **Step 1: 補三份語言檔**

三份都補以下 key，位置與既有同類 key 相鄰（`settings.tab.about` 緊接 `settings.tab.advanced`；`about.*` 接在 `settings.game_path_hint` 之後；`button.*` 與其他 button key 同一區；`update.latest`／`update.failed` 緊接 Task 3 加的 `update.available`），三份順序一致。

`src/i18n/zh-TW.json`

```json
  "update.latest": "已是最新版本",
  "update.failed": "檢查更新失敗：{error}",
  "settings.tab.about": "關於",
  "about.version": "版本",
  "about.project": "專案",
  "about.author": "開發者",
  "about.logs": "紀錄檔",
  "about.logs_hint": "回報問題時，請附上這個資料夾裡的 app.log 與 messages.log",
  "button.check_update": "檢查更新",
  "button.checking": "檢查中…",
  "button.open_folder": "開啟資料夾",
```

`src/i18n/zh-CN.json`

```json
  "update.latest": "已是最新版本",
  "update.failed": "检查更新失败：{error}",
  "settings.tab.about": "关于",
  "about.version": "版本",
  "about.project": "项目",
  "about.author": "开发者",
  "about.logs": "日志文件",
  "about.logs_hint": "反馈问题时，请附上这个文件夹里的 app.log 与 messages.log",
  "button.check_update": "检查更新",
  "button.checking": "检查中…",
  "button.open_folder": "打开文件夹",
```

`src/i18n/en.json`

```json
  "update.latest": "You're on the latest version",
  "update.failed": "Update check failed: {error}",
  "settings.tab.about": "About",
  "about.version": "Version",
  "about.project": "Project",
  "about.author": "Developer",
  "about.logs": "Log files",
  "about.logs_hint": "When reporting an issue, attach app.log and messages.log from this folder",
  "button.check_update": "Check for updates",
  "button.checking": "Checking…",
  "button.open_folder": "Open folder",
```

- [ ] **Step 2: 寫失敗的測試**

在 `tests/test_settings.py` 末端追加（沿用檔案裡既有的 `_open_settings` helper 的作法，但要能注入假的 checker，故另寫一個 helper）：

```python
def _open_settings_with_checker(root, checker):
    from src.config import DEFAULT_CONFIG
    from src.i18n import current_language
    from src.ui.settings import SettingsWindow

    cfg = copy.deepcopy(DEFAULT_CONFIG)
    cfg["api"]["provider"] = "custom"
    cfg["api"]["custom"].update(base_url="http://x", model="m")
    cfg["ui_language"] = current_language()
    win = SettingsWindow(root, cfg, on_save=lambda: None, check_update=checker)
    win.open()
    return win


def test_about_tab_shows_version_and_links(root):
    from src import __version__
    from src.i18n import t
    from src.updater import AUTHOR_URL, PROJECT_URL

    win = _open_settings_with_checker(root, lambda: None)
    tabs = [win._nb.tab(i, "text") for i in range(win._nb.index("end"))]
    assert tabs[2] == t("settings.tab.about")
    assert win._version_label.cget("text") == f"v{__version__}"
    assert win._project_link.cget("text") == PROJECT_URL
    assert win._author_link.cget("text") == "GoneTone"
    assert win._author_link_url == AUTHOR_URL
    win._win.destroy()


def test_about_tab_shows_the_log_folder(root):
    from src.config import app_dir

    win = _open_settings_with_checker(root, lambda: None)
    assert win._logs_label.cget("text") == str(app_dir())
    win._win.destroy()


def _run_check(win):
    """同步跑一次檢查：worker 直接呼叫，結果自 queue 取出後交給主執行緒的處理函式。
    正式路徑是 worker 在背景執行緒跑、poll_queue 在主執行緒取，這裡把兩段接起來，
    測試才不必等執行緒。"""
    win._update_check_worker()
    win._on_update_checked(win._update_queue.get_nowait())


def test_manual_check_reports_up_to_date(root):
    from src.i18n import t

    win = _open_settings_with_checker(root, lambda: None)
    _run_check(win)
    assert win._update_result.cget("text") == "✓ " + t("update.latest")
    assert win._update_btn.cget("text") == t("button.check_update")
    assert str(win._update_btn.cget("state")) == "normal"
    win._win.destroy()


def test_manual_check_reports_a_new_version(root, monkeypatch):
    from src.i18n import t
    from src.ui import settings as settings_module
    from src.updater import Release

    release = Release(version="9.9.9", url="https://example.invalid/rel")
    win = _open_settings_with_checker(root, lambda: release)
    _run_check(win)
    assert win._update_result.cget("text") == t("update.available", version="9.9.9")
    assert win._update_result.cget("cursor") == "hand2"

    opened = []
    monkeypatch.setattr(settings_module.webbrowser, "open", opened.append)
    win._update_result.event_generate("<Button-1>")
    root.update()
    assert opened == ["https://example.invalid/rel"]
    win._win.destroy()


def test_manual_check_reports_failure(root):
    from src.i18n import t
    from src.updater import UpdateCheckError

    def boom():
        raise UpdateCheckError("HTTP 403")

    win = _open_settings_with_checker(root, boom)
    _run_check(win)
    assert win._update_result.cget("text") == "✗ " + t("update.failed", error="HTTP 403")
    assert str(win._update_btn.cget("state")) == "normal"
    win._win.destroy()
```

- [ ] **Step 3: 執行測試確認失敗**

Run: `uv run pytest tests/test_settings.py -k about -v`
Expected: FAIL — `TypeError: SettingsWindow.__init__() got an unexpected keyword argument 'check_update'`

- [ ] **Step 4: 寫實作**

`src/ui/settings.py` 的 import 區補：

```python
import os
import queue
import threading
import webbrowser

from src.config import (ADVANCED_LIMITS, DEFAULT_CONFIG, app_dir, app_name,
                        clamp_advanced)
from src.ui.fields import (LINK_COLOR, ApiFields, HotkeyField, LanguageField,
                           UiLanguageField, link_label, poll_queue,
                           validate_api_form)
from src.updater import AUTHOR_URL, PROJECT_URL, check_for_update
```

（`app_dir` 併入既有的 `src.config` import；`LINK_COLOR`／`link_label`／`poll_queue` 併入既有的 `src.ui.fields` import。）

模組常數區補（`_HINT_TRAILING` 之後）：

```python
# 檢查更新結果的字色：沿用測試連線那組（成功綠、失敗紅），有新版用連結藍。
_UPDATE_COLORS = {"latest": "#2e8b57", "available": LINK_COLOR, "failed": "#cc3333"}
```

`__init__` 簽名末端加參數與欄位：

```python
    def __init__(self, root: tk.Tk, cfg: dict, on_save, on_alpha_preview=None,
                 on_language_preview=None, check_update=check_for_update):
        ...
        self._check_update = check_update   # 可注入是為了測試，正式路徑用預設
        self._update_queue: queue.Queue = queue.Queue()
```

`open()` 內，在「進階」分頁那段（`bind_wrap(hint, trailing=_HINT_TRAILING)`）之後、`if self._restore_tab is not None:` 之前插入：

```python
        self._build_about(nb)
```

在 `_alpha_slider` 之前（或 `_spin` 之後，位置擇一即可，維持相關方法相鄰）新增：

```python
    def _build_about(self, nb) -> None:
        """關於分頁：版本與手動檢查更新、專案與開發者連結、紀錄檔位置。"""
        about_scroll = ScrollableFrame(nb, padding=12)
        about = about_scroll.body
        nb.add(about_scroll, text=t("settings.tab.about"))
        about.columnconfigure(1, weight=1)

        ttk.Label(about, text=t("about.version")).grid(row=0, column=0, sticky="w",
                                                       pady=2)
        version_row = ttk.Frame(about)
        version_row.grid(row=0, column=1, sticky="ew", padx=(8, 0), pady=2)
        self._version_label = ttk.Label(version_row, text=f"v{__version__}")
        self._version_label.pack(side="left")
        self._update_btn = ttk.Button(version_row, text=t("button.check_update"),
                                      command=self._start_update_check)
        self._update_btn.pack(side="left", padx=(8, 0))
        self._update_result = ttk.Label(version_row, text="")
        self._update_result.pack(side="left", fill="x", expand=True, padx=8)
        bind_wrap(self._update_result)

        ttk.Label(about, text=t("about.project")).grid(row=1, column=0, sticky="w",
                                                       pady=2)
        self._project_link = link_label(about, PROJECT_URL, PROJECT_URL)
        self._project_link.grid(row=1, column=1, sticky="w", padx=(8, 0), pady=2)

        ttk.Label(about, text=t("about.author")).grid(row=2, column=0, sticky="w",
                                                      pady=2)
        # 開發者名稱是識別碼不是文案，不進語言檔（與服務商品牌名同理）
        self._author_link_url = AUTHOR_URL
        self._author_link = link_label(about, "GoneTone", AUTHOR_URL)
        self._author_link.grid(row=2, column=1, sticky="w", padx=(8, 0), pady=2)

        ttk.Label(about, text=t("about.logs")).grid(row=3, column=0, sticky="w",
                                                    pady=(10, 2))
        logs_row = ttk.Frame(about)
        logs_row.grid(row=3, column=1, sticky="ew", padx=(8, 0), pady=(10, 2))
        # 按鈕先 pack：expand=True 的路徑標籤若先宣告會吃光整列，把按鈕擠掉
        ttk.Button(logs_row, text=t("button.open_folder"),
                   command=self._open_log_folder).pack(side="right", padx=(4, 0))
        self._logs_label = ttk.Label(logs_row, text=str(app_dir()))
        self._logs_label.pack(side="left", fill="x", expand=True)
        logs_hint = ttk.Label(about, text=t("about.logs_hint"), foreground="#888888",
                              justify="left")
        logs_hint.grid(row=4, column=0, columnspan=2, sticky="ew")
        bind_wrap(logs_hint, trailing=_HINT_TRAILING)

    def _start_update_check(self) -> None:
        """手動檢查更新：背景查詢，結果經 queue 交回主執行緒顯示（見 poll_queue）。"""
        self._update_btn.configure(state="disabled", text=t("button.checking"))
        self._update_result.configure(text="")
        threading.Thread(target=self._update_check_worker, daemon=True).start()
        poll_queue(self._win, self._update_queue, self._on_update_checked)

    def _update_check_worker(self) -> None:
        try:
            release = self._check_update()
        except Exception as exc:
            print(f"[update] manual check failed: {exc}", file=sys.stderr)
            self._update_queue.put(("failed", t("update.failed", error=exc), None))
            return
        if release is None:
            print("[update] manual check: already up to date", file=sys.stderr)
            self._update_queue.put(("latest", t("update.latest"), None))
            return
        print(f"[update] manual check: {release.version} available", file=sys.stderr)
        self._update_queue.put(("available",
                                t("update.available", version=release.version),
                                release.url))

    def _on_update_checked(self, result) -> None:
        state, message, url = result
        self._update_btn.configure(state="normal", text=t("button.check_update"))
        prefix = {"latest": "✓ ", "failed": "✗ ", "available": ""}[state]
        self._update_result.configure(text=prefix + message,
                                      foreground=_UPDATE_COLORS[state],
                                      cursor="hand2" if url else "")
        self._update_result.unbind("<Button-1>")
        if url:
            self._update_result.bind("<Button-1>", lambda e: webbrowser.open(url))

    def _open_log_folder(self) -> None:
        """開啟 app.log／messages.log 所在的資料夾（Windows 檔案總管）。"""
        path = app_dir()
        try:
            os.startfile(path)
        except OSError as exc:
            print(f"[ui] open log folder failed: path={path} error={exc}",
                  file=sys.stderr)
```

- [ ] **Step 5: 執行測試確認通過**

Run: `uv run pytest tests/test_settings.py tests/test_i18n.py tests/test_no_hardcoded_ui_text.py -v`
Expected: PASS

- [ ] **Step 6: 全測試 ＋ commit**

Run: `uv run pytest`

```bash
git add src/ui/settings.py src/i18n tests/test_settings.py
git commit -m "feat(settings): add an About tab with a manual update check"
```

---

### Task 7: README 與實機驗收

**Files:**
- Modify: `README.md`（「一般使用者」段、「放版」段）

**Interfaces:**
- Consumes: 前六個 task 的成果
- Produces: 無程式介面

- [ ] **Step 1: 更新 README「一般使用者」段**

在該段第 5 點（結束程式）之後補一點：

```markdown
6. 每次啟動會自動檢查 GitHub 上有沒有新版本，有的話疊加視窗會多出一條藍色橫幅
   （點橫幅開下載頁，點右側 ✕ 關掉這次提醒）；也可以在設定視窗的「關於」分頁
   手動檢查更新、查看專案連結與紀錄檔所在資料夾
```

- [ ] **Step 2: 更新 README「放版」段**

在第 5 點（建立 Release）之後補一句提醒：

```markdown
> 更新檢查看的是 GitHub 的 **Release**（`/releases/latest`），只推 tag 不建 Release
> 的話使用者端不會收到更新提醒。
```

- [ ] **Step 3: 全測試**

Run: `uv run pytest`
Expected: 全綠

- [ ] **Step 4: 實機驗收（開著遊戲、登入進世界內）**

Run: `uv run run.py`

依序確認：

1. **目前狀態（GitHub 上還沒有任何 release）**：不出現更新橫幅；主控台／`app.log` 有 `[update] checking latest release` 與 `[update] no release published yet (404)`。
2. **關於分頁**：齒輪 ⚙ → 「關於」分頁，按「檢查更新」→ 顯示綠色「✓ 已是最新版本」；點專案連結與開發者名稱會開瀏覽器；「開啟資料夾」會開出 `app.log` 所在資料夾。
3. **有新版的樣子**：暫時把 `src/updater.py` 的 `GITHUB_REPO` 改成任何一個有 release 的公開 repo（例如 `astral-sh/uv`）重跑，確認橫幅出現、可點開瀏覽器、✕ 可關掉，且與「遊戲未就緒」橫幅可同時顯示（把遊戲關掉即可觸發）。**驗完務必把 `GITHUB_REPO` 改回來**（`git diff` 確認乾淨）。
4. **斷網**：關掉網路連線後啟動，確認程式照常運作、收訊不受影響，`app.log` 留下 `[update] check failed: ...`。

- [ ] **Step 5: Commit**

```bash
git add README.md
git commit -m "docs: document the update check and About tab"
```

---

## 完成後檢查

- `uv run pytest` 全綠。
- `git status` 乾淨（`GITHUB_REPO` 沒有被驗收步驟改壞）。
- spec 的五條驗收條件全數符合（見 `docs/superpowers/specs/2026-08-31-update-check-design.md`）。
- 未執行 `git push`（放版與推送由使用者決定）。
