# Wiz101 聊天翻譯助手 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 做一個 Windows 桌面工具:OCR 讀取 Wizard101 聊天框並疊加顯示繁中翻譯,熱鍵輸入繁中自動翻英貼進遊戲(不送出)。

**Architecture:** 單一 Python 應用。reader 執行緒每 1.5 秒截取聊天框 → Windows OCR → 去重 → 翻譯 → 經 UI queue 顯示於滑鼠穿透 overlay。composer 以全域熱鍵呼出 tkinter 輸入框,翻譯後還原遊戲焦點並模擬 Ctrl+V。所有 UI 操作經 `queue.Queue` 回到 tkinter 主執行緒。

**Tech Stack:** Python 3.11+、tkinter(stdlib)、mss、winocr、Pillow、httpx、keyboard、pywin32、pytest

**Spec:** `docs/superpowers/specs/2026-08-19-wiz101-chat-translator-design.md`

## Global Constraints

- 僅支援 Windows;Python 3.11+。
- 絕不讀取遊戲記憶體、不攔截封包、不注入 DLL — 只用螢幕擷取與輸入模擬。
- 發訊端貼上後**不自動送出**(不模擬 Enter)。
- `config.json` 不進 git(已在 `.gitignore`);提供 `config.example.json`。
- 套件根目錄為 `src/`(是一個 package,以 `python -m src.main` 執行;測試從專案根目錄跑 `pytest`)。
- 所有原始碼字串常數中的中文一律使用繁體中文(台灣)。
- 依賴安裝指令:`pip install mss winocr pillow httpx keyboard pywin32 pytest`

---

### Task 1: 專案骨架 + config.py

**Files:**
- Create: `pyproject.toml`
- Create: `src/__init__.py`, `src/reader/__init__.py`, `src/composer/__init__.py`, `tests/__init__.py`(全部空檔)
- Create: `src/config.py`
- Create: `config.example.json`
- Test: `tests/test_config.py`

**Interfaces:**
- Consumes: 無(第一個任務)
- Produces: `load_config(path: Path) -> dict`、`save_config(path: Path, cfg: dict) -> None`、`DEFAULT_CONFIG: dict`(結構見下方程式碼;後續所有任務讀取的 config 鍵都以此為準)

- [ ] **Step 1: 建立骨架檔案**

`pyproject.toml`:

```toml
[project]
name = "wiz101-chat-translator"
version = "0.1.0"
description = "Wizard101 聊天 AI 翻譯助手(OCR 收訊翻繁中、熱鍵輸入翻英文)"
requires-python = ">=3.11"
dependencies = [
    "mss",
    "winocr",
    "pillow",
    "httpx",
    "keyboard",
    "pywin32",
]

[tool.pytest.ini_options]
testpaths = ["tests"]
```

建立空檔:`src/__init__.py`、`src/reader/__init__.py`、`src/composer/__init__.py`、`tests/__init__.py`。

`config.example.json`:

```json
{
  "api": {
    "base_url": "http://127.0.0.1:8000",
    "model": "your-model-name",
    "api_key": ""
  },
  "chat_region": null,
  "poll_interval": 1.5,
  "fade_seconds": 180,
  "hotkey": "ctrl+space",
  "overlay_position": {"x": null, "y": null}
}
```

- [ ] **Step 2: 安裝依賴**

Run: `pip install mss winocr pillow httpx keyboard pywin32 pytest`
Expected: 安裝成功(winocr 會一併安裝 winsdk)

- [ ] **Step 3: 寫失敗測試**

`tests/test_config.py`:

```python
import json
from pathlib import Path

from src.config import DEFAULT_CONFIG, load_config, save_config


def test_load_missing_file_returns_defaults(tmp_path: Path):
    cfg = load_config(tmp_path / "nope.json")
    assert cfg == DEFAULT_CONFIG
    assert cfg is not DEFAULT_CONFIG  # 必須是副本,呼叫端改動不能污染預設值


def test_load_merges_partial_file_with_defaults(tmp_path: Path):
    p = tmp_path / "config.json"
    p.write_text(json.dumps({"poll_interval": 3.0, "api": {"model": "m1"}}), encoding="utf-8")
    cfg = load_config(p)
    assert cfg["poll_interval"] == 3.0
    assert cfg["api"]["model"] == "m1"
    assert cfg["api"]["base_url"] == DEFAULT_CONFIG["api"]["base_url"]  # 缺的欄位補預設
    assert cfg["hotkey"] == "ctrl+space"


def test_save_then_load_roundtrip(tmp_path: Path):
    p = tmp_path / "config.json"
    cfg = load_config(p)
    cfg["chat_region"] = {"left": 10, "top": 20, "width": 300, "height": 150}
    save_config(p, cfg)
    assert load_config(p)["chat_region"] == {"left": 10, "top": 20, "width": 300, "height": 150}
```

- [ ] **Step 4: 跑測試確認失敗**

Run: `pytest tests/test_config.py -v`
Expected: FAIL,`ModuleNotFoundError: No module named 'src.config'`

- [ ] **Step 5: 實作 config.py**

`src/config.py`:

```python
"""config.json 讀寫;缺漏欄位以 DEFAULT_CONFIG 補齊。"""
import copy
import json
from pathlib import Path

DEFAULT_CONFIG: dict = {
    "api": {"base_url": "http://127.0.0.1:8000", "model": "", "api_key": ""},
    "chat_region": None,
    "poll_interval": 1.5,
    "fade_seconds": 180,
    "hotkey": "ctrl+space",
    "overlay_position": {"x": None, "y": None},
}


def _merge(base: dict, override: dict) -> dict:
    out = copy.deepcopy(base)
    for key, value in override.items():
        if isinstance(value, dict) and isinstance(out.get(key), dict):
            out[key] = _merge(out[key], value)
        else:
            out[key] = value
    return out


def load_config(path: Path) -> dict:
    if not path.exists():
        return copy.deepcopy(DEFAULT_CONFIG)
    data = json.loads(path.read_text(encoding="utf-8"))
    return _merge(DEFAULT_CONFIG, data)


def save_config(path: Path, cfg: dict) -> None:
    path.write_text(json.dumps(cfg, ensure_ascii=False, indent=2), encoding="utf-8")
```

- [ ] **Step 6: 跑測試確認通過**

Run: `pytest tests/test_config.py -v`
Expected: 3 passed

- [ ] **Step 7: Commit**

```bash
git add pyproject.toml config.example.json src tests
git commit -m "feat: project skeleton and config loading"
```

---

### Task 2: translator.py(OpenAI 相容 API client)

**Files:**
- Create: `src/translator.py`
- Test: `tests/test_translator.py`

**Interfaces:**
- Consumes: config 的 `api` 區塊(`base_url`, `model`, `api_key`)
- Produces: `Translator(base_url: str, model: str, api_key: str = "", timeout: float = 10.0, client: httpx.Client | None = None)`,方法 `to_zh(text: str) -> str`、`to_en(text: str) -> str`;失敗時丟 `httpx.HTTPError`(含子類)

- [ ] **Step 1: 寫失敗測試**

`tests/test_translator.py`:

```python
import json

import httpx
import pytest

from src.translator import EN_SYSTEM, ZH_SYSTEM, Translator


def make_translator(handler) -> Translator:
    transport = httpx.MockTransport(handler)
    client = httpx.Client(base_url="http://test", transport=transport)
    return Translator(base_url="http://test", model="test-model", client=client)


def ok_response(content: str) -> httpx.Response:
    return httpx.Response(200, json={"choices": [{"message": {"content": content}}]})


def test_to_zh_sends_model_and_system_prompt_and_returns_stripped():
    captured = {}

    def handler(request: httpx.Request) -> httpx.Response:
        captured["url"] = str(request.url)
        captured["body"] = json.loads(request.content)
        return ok_response("  你好,朋友!  ")

    t = make_translator(handler)
    assert t.to_zh("hello friend!") == "你好,朋友!"
    assert captured["url"].endswith("/v1/chat/completions")
    assert captured["body"]["model"] == "test-model"
    assert captured["body"]["messages"][0] == {"role": "system", "content": ZH_SYSTEM}
    assert captured["body"]["messages"][1] == {"role": "user", "content": "hello friend!"}


def test_to_en_uses_english_system_prompt():
    captured = {}

    def handler(request: httpx.Request) -> httpx.Response:
        captured["body"] = json.loads(request.content)
        return ok_response("wanna do a dungeon?")

    t = make_translator(handler)
    assert t.to_en("要不要打副本?") == "wanna do a dungeon?"
    assert captured["body"]["messages"][0] == {"role": "system", "content": EN_SYSTEM}


def test_server_error_raises_http_error():
    t = make_translator(lambda req: httpx.Response(500, text="boom"))
    with pytest.raises(httpx.HTTPError):
        t.to_zh("hi")


def test_api_key_sets_authorization_header():
    captured = {}

    def handler(request: httpx.Request) -> httpx.Response:
        captured["auth"] = request.headers.get("authorization")
        return ok_response("x")

    transport = httpx.MockTransport(handler)
    client = httpx.Client(base_url="http://test", transport=transport,
                          headers={"Authorization": "Bearer sk-123"})
    t = Translator(base_url="http://test", model="m", api_key="sk-123", client=client)
    t.to_zh("hi")
    assert captured["auth"] == "Bearer sk-123"
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `pytest tests/test_translator.py -v`
Expected: FAIL,`ModuleNotFoundError: No module named 'src.translator'`

- [ ] **Step 3: 實作 translator.py**

`src/translator.py`:

```python
"""共用翻譯 client:打自架的 OpenAI 相容 /v1/chat/completions。"""
import httpx

ZH_SYSTEM = (
    "你是線上遊戲 Wizard101 的聊天翻譯員。把玩家的英文聊天訊息翻成自然、口語的"
    "繁體中文(台灣用語)。遊戲術語(咒語名、地名、物品名、Boss 名)保留英文原文。"
    "只輸出譯文,不要任何解釋或標點以外的附加內容。"
)

EN_SYSTEM = (
    "You translate a player's Traditional Chinese chat messages into casual, short "
    "English suitable for in-game chat in the MMO Wizard101. Use simple, common words "
    "(the game's chat filter blocks uncommon words). Keep game terms as-is. "
    "Output only the translation, nothing else."
)


class Translator:
    def __init__(self, base_url: str, model: str, api_key: str = "",
                 timeout: float = 10.0, client: httpx.Client | None = None):
        if client is not None:
            self._client = client
        else:
            headers = {"Authorization": f"Bearer {api_key}"} if api_key else {}
            self._client = httpx.Client(base_url=base_url, headers=headers, timeout=timeout)
        self._model = model

    def _chat(self, system: str, text: str) -> str:
        resp = self._client.post("/v1/chat/completions", json={
            "model": self._model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": text},
            ],
            "temperature": 0.3,
        })
        resp.raise_for_status()
        return resp.json()["choices"][0]["message"]["content"].strip()

    def to_zh(self, text: str) -> str:
        return self._chat(ZH_SYSTEM, text)

    def to_en(self, text: str) -> str:
        return self._chat(EN_SYSTEM, text)
```

- [ ] **Step 4: 跑測試確認通過**

Run: `pytest tests/test_translator.py -v`
Expected: 4 passed

- [ ] **Step 5: Commit**

```bash
git add src/translator.py tests/test_translator.py
git commit -m "feat: OpenAI-compatible translator client"
```

---

### Task 3: dedup.py(行級去重)

**Files:**
- Create: `src/reader/dedup.py`
- Test: `tests/test_dedup.py`

**Interfaces:**
- Consumes: 無
- Produces: `LineDeduper(max_seen: int = 200, similarity: float = 0.9)`,方法 `new_lines(lines: list[str]) -> list[str]`(回傳這批之中「從未見過」的行,順序保留;空白行剔除;與已見行相似度 ≥ similarity 視為同一行)

- [ ] **Step 1: 寫失敗測試**

`tests/test_dedup.py`:

```python
from src.reader.dedup import LineDeduper


def test_first_batch_all_new():
    d = LineDeduper()
    assert d.new_lines(["hello there", "anyone selling?"]) == ["hello there", "anyone selling?"]


def test_repeat_batch_returns_nothing():
    d = LineDeduper()
    d.new_lines(["hello there"])
    assert d.new_lines(["hello there"]) == []


def test_ocr_jitter_is_same_line():
    # OCR 抖動:l/I、少一個空格等,相似度 >= 0.9 應視為同一行
    d = LineDeduper()
    d.new_lines(["Player: want to trade my hat?"])
    assert d.new_lines(["Player: want to trade my hatl"]) == []


def test_genuinely_different_line_passes():
    d = LineDeduper()
    d.new_lines(["Player: hi"])
    assert d.new_lines(["Player: where are you from?"]) == ["Player: where are you from?"]


def test_blank_and_whitespace_lines_dropped():
    d = LineDeduper()
    assert d.new_lines(["", "   ", "real line"]) == ["real line"]


def test_duplicate_within_same_batch_kept_once():
    d = LineDeduper()
    assert d.new_lines(["same msg", "same msg"]) == ["same msg"]


def test_seen_set_is_bounded():
    d = LineDeduper(max_seen=2)
    d.new_lines(["line one hello", "line two world", "line three again"])
    # "line one hello" 已被擠出視窗,重新出現時視為新行
    assert d.new_lines(["line one hello"]) == ["line one hello"]
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `pytest tests/test_dedup.py -v`
Expected: FAIL,`ModuleNotFoundError: No module named 'src.reader.dedup'`

- [ ] **Step 3: 實作 dedup.py**

`src/reader/dedup.py`:

```python
"""聊天行去重:記住最近 N 行,模糊比對吸收 OCR 抖動。"""
from collections import deque
from difflib import SequenceMatcher


class LineDeduper:
    def __init__(self, max_seen: int = 200, similarity: float = 0.9):
        self._seen: deque[str] = deque(maxlen=max_seen)
        self._similarity = similarity

    def _is_seen(self, line: str) -> bool:
        return any(
            SequenceMatcher(None, line, old).ratio() >= self._similarity
            for old in self._seen
        )

    def new_lines(self, lines: list[str]) -> list[str]:
        fresh: list[str] = []
        for raw in lines:
            line = raw.strip()
            if not line or self._is_seen(line):
                continue
            self._seen.append(line)
            fresh.append(line)
        return fresh
```

- [ ] **Step 4: 跑測試確認通過**

Run: `pytest tests/test_dedup.py -v`
Expected: 7 passed

- [ ] **Step 5: Commit**

```bash
git add src/reader/dedup.py tests/test_dedup.py
git commit -m "feat: fuzzy line deduplication for OCR output"
```

---

### Task 4: capture.py + ocr.py(截圖與辨識)

**Files:**
- Create: `src/reader/capture.py`
- Create: `src/reader/ocr.py`
- Create: `scripts/check_ocr.py`(手動驗證用)
- Test: `tests/test_ocr.py`

**Interfaces:**
- Consumes: config 的 `chat_region`(`{"left","top","width","height"}`)
- Produces:
  - `grab_region(region: dict, scale: int = 2) -> PIL.Image.Image`(BGRA→RGB,放大 scale 倍改善小字辨識)
  - `recognize_lines(img: PIL.Image.Image) -> list[str]`(呼叫 winocr,英文,依 y 座標排序)
  - `_lines_in_order(result: dict) -> list[str]`(純函式,可測;解析 winocr 回傳 dict)

- [ ] **Step 1: 寫失敗測試(針對純解析函式)**

`tests/test_ocr.py`:

```python
from src.reader.ocr import _lines_in_order


def fake_line(text: str, y: int) -> dict:
    return {"text": text, "words": [{"text": text, "bounding_rect": {"x": 0, "y": y, "width": 50, "height": 12}}]}


def test_lines_sorted_by_y():
    result = {"lines": [fake_line("second", 40), fake_line("first", 10), fake_line("third", 90)]}
    assert _lines_in_order(result) == ["first", "second", "third"]


def test_empty_result_gives_empty_list():
    assert _lines_in_order({}) == []
    assert _lines_in_order({"lines": []}) == []


def test_line_without_words_defaults_to_top():
    result = {"lines": [fake_line("late", 50), {"text": "no-words-line", "words": []}]}
    assert _lines_in_order(result) == ["no-words-line", "late"]
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `pytest tests/test_ocr.py -v`
Expected: FAIL,`ModuleNotFoundError: No module named 'src.reader.ocr'`

- [ ] **Step 3: 實作 capture.py 與 ocr.py**

`src/reader/capture.py`:

```python
"""以 mss 截取聊天框區域,轉為 PIL Image 並放大以利 OCR。"""
import mss
from PIL import Image


def grab_region(region: dict, scale: int = 2) -> Image.Image:
    with mss.mss() as sct:
        shot = sct.grab({
            "left": region["left"], "top": region["top"],
            "width": region["width"], "height": region["height"],
        })
    img = Image.frombytes("RGB", shot.size, shot.bgra, "raw", "BGRX")
    if scale != 1:
        img = img.resize((img.width * scale, img.height * scale), Image.LANCZOS)
    return img
```

`src/reader/ocr.py`:

```python
"""Windows OCR(winocr)包裝:輸入 PIL Image,輸出依 y 排序的文字行。"""
from PIL import Image
import winocr


def _line_y(line: dict) -> int:
    words = line.get("words") or []
    return min((w["bounding_rect"]["y"] for w in words), default=0)


def _lines_in_order(result: dict) -> list[str]:
    lines = result.get("lines") or []
    return [ln["text"] for ln in sorted(lines, key=_line_y)]


def recognize_lines(img: Image.Image) -> list[str]:
    result = winocr.recognize_pil_sync(img, "en")
    return _lines_in_order(result)
```

`scripts/check_ocr.py`(手動驗證;不進測試套件):

```python
"""手動驗證 OCR:讀 config 的 chat_region,截圖並印出辨識行。
用法:遊戲開著、config.json 已有 chat_region,執行 python scripts/check_ocr.py
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from src.config import load_config
from src.reader.capture import grab_region
from src.reader.ocr import recognize_lines

cfg = load_config(Path("config.json"))
if not cfg["chat_region"]:
    sys.exit("config.json 還沒有 chat_region,先跑 python -m src.main --pick-region")
img = grab_region(cfg["chat_region"])
img.save("scripts/last_capture.png")
print("辨識結果:")
for line in recognize_lines(img):
    print(f"  | {line}")
print("(截圖已存 scripts/last_capture.png,可對照)")
```

- [ ] **Step 4: 跑測試確認通過**

Run: `pytest tests/test_ocr.py -v`
Expected: 3 passed

- [ ] **Step 5: 確認 winocr 可載入(冒煙測試,不需遊戲)**

Run: `python -c "from PIL import Image; from src.reader.ocr import recognize_lines; print(recognize_lines(Image.new('RGB', (200, 50), 'white')))"`
Expected: 印出 `[]`(空白圖無文字),無例外。若丟 winsdk/語言包錯誤,記下錯誤訊息並回報(Windows 需已安裝英文語言包)。

- [ ] **Step 6: Commit**

```bash
git add src/reader/capture.py src/reader/ocr.py scripts/check_ocr.py tests/test_ocr.py
git commit -m "feat: screen capture and Windows OCR wrapper"
```

---

### Task 5: overlay.py(疊加顯示視窗)

**Files:**
- Create: `src/reader/overlay.py`
- Create: `scripts/demo_overlay.py`(手動驗證用)
- Test: `tests/test_overlay.py`

**Interfaces:**
- Consumes: tkinter root(由呼叫端建立)、config 的 `overlay_position`、`fade_seconds`
- Produces: `OverlayWindow(root, x: int | None, y: int | None, max_messages: int = 8, fade_seconds: int = 180)`,方法:
  - `add_message(original: str, translated: str) -> None`
  - `set_error(text: str) -> None` / `clear_error() -> None`
  - `prune(now: float | None = None) -> None`(移除逾時訊息;由主迴圈定期呼叫)
  - 內部以 pywin32 設 `WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE` 滑鼠穿透

- [ ] **Step 1: 寫失敗測試(邏輯部分;本機有桌面環境,tkinter 可建立)**

`tests/test_overlay.py`:

```python
import tkinter as tk

import pytest

from src.reader.overlay import OverlayWindow


@pytest.fixture
def root():
    r = tk.Tk()
    r.withdraw()
    yield r
    r.destroy()


def test_add_message_appends_and_caps_at_max(root):
    ov = OverlayWindow(root, x=0, y=0, max_messages=3, fade_seconds=180)
    for i in range(5):
        ov.add_message(f"msg {i}", f"訊息 {i}")
    texts = ov.visible_messages()
    assert len(texts) == 3
    assert texts[-1] == ("msg 4", "訊息 4")
    assert texts[0] == ("msg 2", "訊息 2")


def test_prune_removes_expired(root):
    ov = OverlayWindow(root, x=0, y=0, fade_seconds=10)
    ov.add_message("old", "舊", now=100.0)
    ov.add_message("new", "新", now=105.0)
    ov.prune(now=111.0)  # 100+10 < 111 過期;105+10 >= 111 保留
    assert ov.visible_messages() == [("new", "新")]


def test_error_banner_toggle(root):
    ov = OverlayWindow(root, x=0, y=0)
    assert ov.error_text() is None
    ov.set_error("⚠ 翻譯伺服器離線")
    assert ov.error_text() == "⚠ 翻譯伺服器離線"
    ov.clear_error()
    assert ov.error_text() is None
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `pytest tests/test_overlay.py -v`
Expected: FAIL,`ModuleNotFoundError: No module named 'src.reader.overlay'`

- [ ] **Step 3: 實作 overlay.py**

`src/reader/overlay.py`:

```python
"""疊加視窗:無邊框、置頂、半透明、滑鼠穿透;顯示原文+繁中譯文。"""
import time
import tkinter as tk

import win32con
import win32gui

BG = "#101018"
FG_ORIGINAL = "#9a9aa8"
FG_TRANSLATED = "#f2f2f7"
FG_ERROR = "#ff5f5f"


class OverlayWindow:
    def __init__(self, root: tk.Tk, x: int | None, y: int | None,
                 max_messages: int = 8, fade_seconds: int = 180):
        self._max = max_messages
        self._fade = fade_seconds
        self._messages: list[tuple[float, str, str, tk.Frame]] = []
        self._error_label: tk.Label | None = None

        self._win = tk.Toplevel(root)
        self._win.overrideredirect(True)
        self._win.attributes("-topmost", True)
        self._win.attributes("-alpha", 0.85)
        self._win.configure(bg=BG)
        self._win.geometry(f"+{x or 40}+{y or 40}")
        self._frame = tk.Frame(self._win, bg=BG)
        self._frame.pack(fill="both", expand=True, padx=6, pady=4)
        self._win.update_idletasks()
        self._make_click_through()

    def _make_click_through(self) -> None:
        hwnd = win32gui.GetParent(self._win.winfo_id()) or self._win.winfo_id()
        styles = win32gui.GetWindowLong(hwnd, win32con.GWL_EXSTYLE)
        win32gui.SetWindowLong(
            hwnd, win32con.GWL_EXSTYLE,
            styles | win32con.WS_EX_LAYERED | win32con.WS_EX_TRANSPARENT | win32con.WS_EX_NOACTIVATE,
        )

    def add_message(self, original: str, translated: str, now: float | None = None) -> None:
        row = tk.Frame(self._frame, bg=BG)
        tk.Label(row, text=original, bg=BG, fg=FG_ORIGINAL,
                 font=("Microsoft JhengHei", 9), anchor="w", justify="left",
                 wraplength=420).pack(fill="x")
        tk.Label(row, text=translated, bg=BG, fg=FG_TRANSLATED,
                 font=("Microsoft JhengHei", 11), anchor="w", justify="left",
                 wraplength=420).pack(fill="x")
        row.pack(fill="x", pady=2)
        self._messages.append((now if now is not None else time.time(), original, translated, row))
        while len(self._messages) > self._max:
            _, _, _, old_row = self._messages.pop(0)
            old_row.destroy()

    def prune(self, now: float | None = None) -> None:
        cutoff = (now if now is not None else time.time()) - self._fade
        keep = []
        for entry in self._messages:
            if entry[0] <= cutoff:
                entry[3].destroy()
            else:
                keep.append(entry)
        self._messages = keep

    def set_error(self, text: str) -> None:
        self.clear_error()
        self._error_label = tk.Label(self._frame, text=text, bg=BG, fg=FG_ERROR,
                                     font=("Microsoft JhengHei", 10, "bold"), anchor="w")
        self._error_label.pack(fill="x", pady=2)

    def clear_error(self) -> None:
        if self._error_label is not None:
            self._error_label.destroy()
            self._error_label = None

    # --- 測試/除錯輔助 ---
    def visible_messages(self) -> list[tuple[str, str]]:
        return [(orig, trans) for _, orig, trans, _ in self._messages]

    def error_text(self) -> str | None:
        return self._error_label.cget("text") if self._error_label else None
```

`scripts/demo_overlay.py`:

```python
"""手動驗證 overlay:應看到置頂半透明視窗,訊息逐則出現,且滑鼠點擊會穿透到底下視窗。"""
import sys
import tkinter as tk
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from src.reader.overlay import OverlayWindow

root = tk.Tk()
root.withdraw()
ov = OverlayWindow(root, x=100, y=100, fade_seconds=15)

samples = [
    ("Wolf: anyone need help with Rattlebones?", "Wolf:有人需要幫忙打 Rattlebones 嗎?"),
    ("Amber: selling seeds at bazaar", "Amber:我在 bazaar 賣種子"),
    ("Duncan: brb 5 min", "Duncan:離開一下,5 分鐘回來"),
]

def feed(i=0):
    if i < len(samples):
        ov.add_message(*samples[i])
        root.after(2000, feed, i + 1)

def tick():
    ov.prune()
    root.after(1000, tick)

feed()
tick()
root.after(30000, root.destroy)  # 30 秒後自動關閉
root.mainloop()
```

- [ ] **Step 4: 跑測試確認通過**

Run: `pytest tests/test_overlay.py -v`
Expected: 3 passed

- [ ] **Step 5: 跑 demo 冒煙測試(可自動,不需遊戲)**

Run: `python scripts/demo_overlay.py`
Expected: 執行 30 秒自動結束,無例外。畫面上應出現半透明訊息視窗(執行者若無法目視,確認無例外即可,目視留到 Task 8 手動驗證)。

- [ ] **Step 6: Commit**

```bash
git add src/reader/overlay.py scripts/demo_overlay.py tests/test_overlay.py
git commit -m "feat: click-through translation overlay window"
```

---

### Task 6: region_picker.py(框選聊天區域)

**Files:**
- Create: `src/region_picker.py`
- Test: `tests/test_region_picker.py`

**Interfaces:**
- Consumes: 無
- Produces:
  - `pick_region() -> dict | None`(全螢幕遮罩拖曳框選;Esc 取消回傳 None)
  - `_to_region(x0: int, y0: int, x1: int, y1: int) -> dict`(純函式:任意兩角→`{"left","top","width","height"}`)

- [ ] **Step 1: 寫失敗測試(純函式部分)**

`tests/test_region_picker.py`:

```python
from src.region_picker import _to_region


def test_normal_drag_topleft_to_bottomright():
    assert _to_region(10, 20, 110, 80) == {"left": 10, "top": 20, "width": 100, "height": 60}


def test_reverse_drag_normalized():
    assert _to_region(110, 80, 10, 20) == {"left": 10, "top": 20, "width": 100, "height": 60}


def test_minimum_size_enforced():
    # 誤點一下(幾乎零面積)也至少給 1x1,避免 mss 丟例外
    r = _to_region(50, 50, 50, 50)
    assert r["width"] >= 1 and r["height"] >= 1
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `pytest tests/test_region_picker.py -v`
Expected: FAIL,`ModuleNotFoundError: No module named 'src.region_picker'`

- [ ] **Step 3: 實作 region_picker.py**

`src/region_picker.py`:

```python
"""全螢幕半透明遮罩,拖曳框選聊天框區域。Esc 取消。"""
import tkinter as tk


def _to_region(x0: int, y0: int, x1: int, y1: int) -> dict:
    return {
        "left": min(x0, x1),
        "top": min(y0, y1),
        "width": max(abs(x1 - x0), 1),
        "height": max(abs(y1 - y0), 1),
    }


def pick_region() -> dict | None:
    root = tk.Tk()
    root.attributes("-fullscreen", True)
    root.attributes("-alpha", 0.35)
    root.attributes("-topmost", True)
    root.configure(bg="black", cursor="crosshair")

    canvas = tk.Canvas(root, bg="black", highlightthickness=0)
    canvas.pack(fill="both", expand=True)
    canvas.create_text(
        root.winfo_screenwidth() // 2, 60,
        text="拖曳框選遊戲聊天框範圍(Esc 取消)",
        fill="white", font=("Microsoft JhengHei", 16),
    )

    state: dict = {"start": None, "rect": None, "result": None}

    def on_press(e):
        state["start"] = (e.x_root, e.y_root)
        state["rect"] = canvas.create_rectangle(e.x, e.y, e.x, e.y, outline="#00d0ff", width=2)

    def on_drag(e):
        if state["rect"] is not None:
            x0, y0 = state["start"]
            canvas.coords(state["rect"], x0, y0, e.x_root, e.y_root)

    def on_release(e):
        if state["start"] is not None:
            x0, y0 = state["start"]
            state["result"] = _to_region(x0, y0, e.x_root, e.y_root)
        root.destroy()

    def on_escape(_e):
        state["result"] = None
        root.destroy()

    canvas.bind("<ButtonPress-1>", on_press)
    canvas.bind("<B1-Motion>", on_drag)
    canvas.bind("<ButtonRelease-1>", on_release)
    root.bind("<Escape>", on_escape)
    root.mainloop()
    return state["result"]
```

- [ ] **Step 4: 跑測試確認通過**

Run: `pytest tests/test_region_picker.py -v`
Expected: 3 passed

- [ ] **Step 5: Commit**

```bash
git add src/region_picker.py tests/test_region_picker.py
git commit -m "feat: fullscreen drag region picker"
```

---

### Task 7: composer(輸入框 + 貼上)

**Files:**
- Create: `src/composer/paste.py`
- Create: `src/composer/input_box.py`
- Test: `tests/test_paste.py`

**Interfaces:**
- Consumes: `Translator.to_en(text) -> str`(Task 2)
- Produces:
  - `set_clipboard(text: str) -> None` / `get_clipboard() -> str`
  - `paste_into_window(hwnd: int | None) -> None`(還原焦點→送 Ctrl+V;不送 Enter)
  - `InputBox(root, translate_fn, ui_queue, on_translated)`:方法 `show() -> None`(記住目前前景視窗、彈出輸入框)。Enter 觸發:於背景執行緒呼叫 `translate_fn(text)`,完成後把 `lambda: on_translated(english, hwnd)` 放進 `ui_queue`;失敗則在框內顯示錯誤、保留原文。Esc 關閉。

- [ ] **Step 1: 寫失敗測試(剪貼簿 roundtrip;本機 Windows 可直接測)**

`tests/test_paste.py`:

```python
from src.composer.paste import get_clipboard, set_clipboard


def test_clipboard_roundtrip_unicode():
    set_clipboard("hey wanna team up? 一起打副本")
    assert get_clipboard() == "hey wanna team up? 一起打副本"


def test_clipboard_overwrite():
    set_clipboard("first")
    set_clipboard("second")
    assert get_clipboard() == "second"
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `pytest tests/test_paste.py -v`
Expected: FAIL,`ModuleNotFoundError: No module named 'src.composer.paste'`

- [ ] **Step 3: 實作 paste.py 與 input_box.py**

`src/composer/paste.py`:

```python
"""剪貼簿寫入與貼上模擬。只貼上,絕不模擬 Enter 送出。"""
import time

import keyboard
import win32clipboard
import win32con
import win32gui


def set_clipboard(text: str) -> None:
    win32clipboard.OpenClipboard()
    try:
        win32clipboard.EmptyClipboard()
        win32clipboard.SetClipboardData(win32con.CF_UNICODETEXT, text)
    finally:
        win32clipboard.CloseClipboard()


def get_clipboard() -> str:
    win32clipboard.OpenClipboard()
    try:
        return win32clipboard.GetClipboardData(win32con.CF_UNICODETEXT)
    finally:
        win32clipboard.CloseClipboard()


def paste_into_window(hwnd: int | None) -> None:
    """還原前景視窗並送 Ctrl+V。剪貼簿須已由呼叫端填好。"""
    if hwnd and win32gui.IsWindow(hwnd):
        try:
            win32gui.SetForegroundWindow(hwnd)
        except Exception:
            pass  # Windows 前景鎖:失敗就讓使用者自己點回遊戲手動 Ctrl+V
        time.sleep(0.15)
    keyboard.send("ctrl+v")
```

`src/composer/input_box.py`:

```python
"""熱鍵呼出的繁中輸入框:Enter 翻譯、Esc 關閉。翻譯跑背景執行緒,結果經 ui_queue 回主執行緒。"""
import queue
import threading
import tkinter as tk

import win32gui

BG = "#1a1a24"
FG = "#f2f2f7"


class InputBox:
    def __init__(self, root: tk.Tk, translate_fn, ui_queue: queue.Queue, on_translated):
        self._root = root
        self._translate = translate_fn
        self._queue = ui_queue
        self._on_translated = on_translated
        self._win: tk.Toplevel | None = None
        self._entry: tk.Entry | None = None
        self._status: tk.Label | None = None
        self._target_hwnd: int | None = None

    def show(self) -> None:
        if self._win is not None:  # 已開著就聚焦
            self._win.lift()
            self._entry.focus_force()
            return
        self._target_hwnd = win32gui.GetForegroundWindow()
        self._win = tk.Toplevel(self._root)
        self._win.title("翻譯輸入")
        self._win.attributes("-topmost", True)
        self._win.configure(bg=BG)
        self._win.geometry("460x84+200+200")
        self._entry = tk.Entry(self._win, bg="#262636", fg=FG, insertbackground=FG,
                               font=("Microsoft JhengHei", 12))
        self._entry.pack(fill="x", padx=8, pady=(10, 4))
        self._status = tk.Label(self._win, text="打繁中,Enter 翻譯並貼進遊戲(不會自動送出),Esc 關閉",
                                bg=BG, fg="#9a9aa8", font=("Microsoft JhengHei", 9), anchor="w")
        self._status.pack(fill="x", padx=8)
        self._entry.bind("<Return>", self._on_enter)
        self._win.bind("<Escape>", lambda e: self.close())
        self._win.protocol("WM_DELETE_WINDOW", self.close)
        self._entry.focus_force()

    def close(self) -> None:
        if self._win is not None:
            self._win.destroy()
            self._win = None
            self._entry = None
            self._status = None

    def _on_enter(self, _event) -> None:
        text = self._entry.get().strip()
        if not text:
            return
        self._entry.configure(state="disabled")
        self._status.configure(text="翻譯中…", fg="#9a9aa8")
        hwnd = self._target_hwnd
        threading.Thread(target=self._worker, args=(text, hwnd), daemon=True).start()

    def _worker(self, text: str, hwnd: int | None) -> None:
        try:
            english = self._translate(text)
        except Exception as exc:
            self._queue.put(lambda: self._show_error(f"翻譯失敗:{exc}"))
            return
        self._queue.put(lambda: self._finish(english, hwnd))

    def _show_error(self, message: str) -> None:
        if self._entry is None:
            return
        self._entry.configure(state="normal")
        self._status.configure(text=message, fg="#ff5f5f")

    def _finish(self, english: str, hwnd: int | None) -> None:
        self.close()
        self._on_translated(english, hwnd)
```

- [ ] **Step 4: 跑測試確認通過**

Run: `pytest tests/test_paste.py -v`
Expected: 2 passed

- [ ] **Step 5: Commit**

```bash
git add src/composer/paste.py src/composer/input_box.py tests/test_paste.py
git commit -m "feat: hotkey input box and clipboard paste (no auto-send)"
```

---

### Task 8: main.py 整合 + README

**Files:**
- Create: `src/main.py`
- Create: `README.md`
- Test: 全套 `pytest` + 手動驗證清單(交給使用者)

**Interfaces:**
- Consumes: 前七個任務的全部公開介面(`load_config`/`save_config`、`Translator`、`LineDeduper`、`grab_region`、`recognize_lines`、`OverlayWindow`、`pick_region`、`InputBox`、`set_clipboard`、`paste_into_window`)
- Produces: `python -m src.main` 可執行;`--pick-region` 重新框選

- [ ] **Step 1: 實作 main.py**

`src/main.py`:

```python
"""進入點:reader 執行緒 + 全域熱鍵 + tkinter 主迴圈(UI 事件經 ui_queue 序列化)。"""
import argparse
import queue
import sys
import threading
import time
import tkinter as tk
from pathlib import Path

import httpx
import keyboard
import win32gui

from src.composer.input_box import InputBox
from src.composer.paste import paste_into_window, set_clipboard
from src.config import load_config, save_config
from src.reader.capture import grab_region
from src.reader.dedup import LineDeduper
from src.reader.ocr import recognize_lines
from src.reader.overlay import OverlayWindow
from src.region_picker import pick_region
from src.translator import Translator

CONFIG_PATH = Path("config.json")
BACKOFF_STEPS = [5, 15, 30]  # 翻譯伺服器離線時的重試間隔(秒)
GAME_WINDOW_TITLE = "Wizard101"


def game_window_hidden() -> bool:
    """遊戲視窗最小化時回傳 True(暫停截圖)。找不到視窗時不暫停,照常截圖。"""
    hwnd = win32gui.FindWindow(None, GAME_WINDOW_TITLE)
    return bool(hwnd) and bool(win32gui.IsIconic(hwnd))


def reader_loop(cfg: dict, translator: Translator, overlay: OverlayWindow,
                ui_queue: queue.Queue, stop: threading.Event) -> None:
    deduper = LineDeduper()
    backoff_index = 0
    while not stop.is_set():
        interval = cfg["poll_interval"]
        try:
            if not game_window_hidden():
                img = grab_region(cfg["chat_region"])
                for line in deduper.new_lines(recognize_lines(img)):
                    translated = translator.to_zh(line)
                    ui_queue.put(lambda o=line, t=translated: overlay.add_message(o, t))
                if backoff_index:
                    backoff_index = 0
                    ui_queue.put(overlay.clear_error)
        except httpx.HTTPError:
            interval = BACKOFF_STEPS[min(backoff_index, len(BACKOFF_STEPS) - 1)]
            backoff_index += 1
            ui_queue.put(lambda: overlay.set_error("⚠ 翻譯伺服器離線,重試中…"))
        except Exception as exc:  # OCR/截圖偶發錯誤:略過該輪,不讓執行緒死掉
            print(f"[reader] 略過此輪:{exc}", file=sys.stderr)
        stop.wait(interval)


def main() -> None:
    parser = argparse.ArgumentParser(description="Wiz101 聊天翻譯助手")
    parser.add_argument("--pick-region", action="store_true", help="重新框選聊天框區域")
    args = parser.parse_args()

    cfg = load_config(CONFIG_PATH)
    if args.pick_region or not cfg["chat_region"]:
        region = pick_region()
        if region is None:
            sys.exit("已取消框選,離開。")
        cfg["chat_region"] = region
        save_config(CONFIG_PATH, cfg)
        print(f"聊天框區域已存檔:{region}")

    if not cfg["api"]["model"]:
        sys.exit("請先把 config.example.json 複製為 config.json,填入 api.base_url 與 api.model。")

    translator = Translator(**cfg["api"])
    ui_queue: queue.Queue = queue.Queue()

    root = tk.Tk()
    root.withdraw()
    overlay = OverlayWindow(
        root,
        x=cfg["overlay_position"]["x"], y=cfg["overlay_position"]["y"],
        fade_seconds=cfg["fade_seconds"],
    )

    def on_translated(english: str, hwnd: int | None) -> None:
        set_clipboard(english)
        paste_into_window(hwnd)

    input_box = InputBox(root, translator.to_en, ui_queue, on_translated)
    keyboard.add_hotkey(cfg["hotkey"], lambda: ui_queue.put(input_box.show))

    stop = threading.Event()
    threading.Thread(target=reader_loop, args=(cfg, translator, overlay, ui_queue, stop),
                     daemon=True).start()

    def pump() -> None:
        while True:
            try:
                ui_queue.get_nowait()()
            except queue.Empty:
                break
        overlay.prune()
        root.after(50, pump)

    print(f"執行中:熱鍵 {cfg['hotkey']} 呼出輸入框;Ctrl+C 結束。")
    pump()
    try:
        root.mainloop()
    finally:
        stop.set()
        keyboard.unhook_all()


if __name__ == "__main__":
    main()
```

- [ ] **Step 2: 寫 README.md**

```markdown
# Wiz101 聊天翻譯助手

Wizard101 聊天 AI 翻譯:聊天框英文訊息即時翻成繁體中文疊加顯示(保留原文);
熱鍵輸入繁中自動翻成英文貼進遊戲聊天欄(**不會自動送出**,自己確認後按 Enter)。

純螢幕 OCR + 輸入模擬 — 不讀記憶體、不攔封包、不注入,無封號疑慮。

## 安裝

1. Python 3.11+(Windows)
2. `pip install mss winocr pillow httpx keyboard pywin32`
3. 複製 `config.example.json` 為 `config.json`,填入自架 AI 伺服器的
   `api.base_url` 與 `api.model`(OpenAI 相容 `/v1/chat/completions`)

## 使用

1. 以「視窗化 / 無邊框」模式開啟 Wizard101
2. `python -m src.main` — 第一次會要你拖曳框選聊天框範圍(之後可用
   `python -m src.main --pick-region` 重選)
3. 聊天框出現英文訊息 → 畫面上疊加「原文 + 繁中」;訊息 3 分鐘後淡出
4. 想發言:先在遊戲裡點開聊天輸入欄 → 按 `Ctrl+Space` → 打繁中 → Enter →
   英文自動貼進聊天欄 → 自己檢查後按 Enter 送出
   (若貼上時機不對,英文已在剪貼簿,手動 Ctrl+V 即可)

## 已知限制

- 遊戲聊天白名單:非白名單英文詞可能被遊戲過濾,任何翻譯工具都繞不過
- 訊息顯示延遲約 1.5–3 秒(輪詢 + 翻譯)
- 只翻聊天框內容;頭頂泡泡不追蹤
- 全域熱鍵(keyboard 套件)在部分環境需以系統管理員身分執行
```

- [ ] **Step 3: 跑全套測試**

Run: `pytest -v`
Expected: 全部通過(config 3 + translator 4 + dedup 7 + ocr 3 + overlay 3 + region_picker 3 + paste 2 = 25 passed)

- [ ] **Step 4: 冒煙測試(不開遊戲)**

Run: `python -c "import src.main"`(確認 import 無誤)
Run: `python -m src.main`(在沒有 config.json 的狀態下會先跳出框選遮罩 — 執行者按 Esc 取消,Expected: 印出「已取消框選,離開。」)
Expected: 兩者皆無 traceback。

- [ ] **Step 5: Commit**

```bash
git add src/main.py README.md
git commit -m "feat: main entry point wiring reader, composer and overlay"
```

- [ ] **Step 6: 使用者手動驗證清單(交付前必跑,需要遊戲與自架伺服器)**

以下由使用者(或在使用者陪同下)執行,逐項確認:

1. `config.json` 填入真實 `base_url`/`model` → `python -m src.main` → 框選遊戲聊天框
2. 請朋友(或第二隻帳號)在遊戲裡打英文 → 2~3 秒內 overlay 出現原文+繁中,無重複翻譯
3. `python scripts/check_ocr.py` 對照 `scripts/last_capture.png`,確認 OCR 辨識率可接受
4. overlay 蓋在遊戲上時,滑鼠點 overlay 區域能穿透操作遊戲
5. 遊戲裡開聊天欄 → `Ctrl+Space` → 打「我們去打副本好嗎」→ Enter → 英文出現在遊戲聊天欄且**沒有**自動送出
6. 關掉自架伺服器 → overlay 出現「⚠ 翻譯伺服器離線,重試中…」;重開伺服器 → 提示消失、翻譯恢復
7. 最小化遊戲 → 確認不再截圖翻譯;還原後恢復

發現問題記下現象(截圖佳),回到對應任務修正。
