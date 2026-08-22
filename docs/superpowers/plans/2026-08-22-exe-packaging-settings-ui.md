# exe 打包與軟體內設定 UI 實作計畫

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 將本工具打包成單一 exe，並以「首次設定精靈＋一般設定視窗」取代手動編輯 config.json。

**Architecture:** tkinter 原生 UI（新增 `src/ui/`：wizard／settings／fields 共用欄位元件）；translator 拆成 OpenAI 相容與 Claude 兩種 provider client、錯誤統一映射為可重試／設定錯誤兩類例外；PyInstaller onefile 打包，config 與 log 存 exe 旁。

**Tech Stack:** Python 3.11＋、tkinter／ttk、httpx、anthropic SDK、keyboard、wizwalker（git fork）、PyInstaller、uv、pytest。

**Spec:** `docs/superpowers/specs/2026-08-22-exe-packaging-settings-ui-design.md`

## Global Constraints

- 回答、註解、docstring、UI 文字一律繁體中文（台灣），CJK 語境用全形標點。
- 產品命名一律全名 **Wizard101**：exe 為 `Wizard101ChatTranslator.exe`、`pyproject.toml` 專案名 `wizard101-chat-translator`。
- OpenAI 相容路徑 `temperature=0`；兩種 client 請求 timeout 均為 **60 秒**。
- 翻譯語言不可寫死：目標語言來自 config、發話固定 `translator.OUTGOING_LANGUAGE`；命名用方向（incoming／outgoing）。
- 套件管理用 `uv`（`uv add`／`uv sync`／`uv run pytest`／`uv run run.py`）。
- 每個 task 結束前 `uv run pytest` 必須全綠才 commit；不要 `git push`。
- YAGNI：不做 CI、安裝程式、自動更新、系統匣。
- 模組與公開 function／class 寫 docstring；實作層 `#` 註解僅在 WHY 不顯而易見時寫。

---

### Task 1: 打包冒煙測試（spike——先驗證 frozen 可行）

此 task 是 spike：驗證 wizwalker（git fork）、tkinter、keyboard、pymem 在 PyInstaller frozen 環境可用。**驗證不過就停下回報，不進後續 task。**

**Files:**
- Modify: `pyproject.toml`（dev 依賴加 pyinstaller）
- Create: `build.spec`

**Interfaces:**
- Produces: `build.spec`（後續 Task 7 改 `console=False` 定稿）；建置指令 `uv run pyinstaller build.spec`。

- [ ] **Step 1: 加 dev 依賴**

```bash
uv add --dev pyinstaller
```

- [ ] **Step 2: 寫 build.spec（spike 階段 console=True，方便看錯誤輸出）**

```python
# build.spec —— PyInstaller 打包設定:單一 exe。
# spike 階段 console=True 便於除錯;定稿(Task 7)改 console=False。
a = Analysis(
    ["run.py"],
    pathex=[],
    binaries=[],
    datas=[],
    hiddenimports=[],  # spike 發現缺模組時補在這裡,並註明原因
    excludedimports=[],
)
pyz = PYZ(a.pure)
exe = EXE(
    pyz,
    a.scripts,
    a.binaries,
    a.datas,
    name="Wizard101ChatTranslator",
    console=True,
    upx=False,
)
```

- [ ] **Step 3: 建置**

```bash
uv run pyinstaller build.spec --noconfirm
```

預期：`dist/Wizard101ChatTranslator.exe` 產生。建置失敗（缺 hook、蒐集不到模組）→ 依錯誤訊息補 `hiddenimports`／`datas` 後重建。

- [ ] **Step 4: 實機驗證（需遊戲登入進世界內）**

把現有可用的 `config.json` 複製到 `dist/`，在終端機執行 `dist/Wizard101ChatTranslator.exe`（注意：目前 config 路徑是相對 cwd，先 `cd dist` 再執行）。驗收：
1. overlay 出現、狀態走到「監聽中」（wizwalker 掛入成功）
2. 遊戲聊天有新訊息時 overlay 顯示譯文
3. 熱鍵呼出輸入框、Enter 能翻譯鍵入
4. 關閉程式後重開遊戲內再跑一次不觸發 `PatternFailed`（hook 有正常解除）

常見問題排查：`ModuleNotFoundError` → 把該模組加進 `hiddenimports`；wizwalker 若有讀套件內資料檔 → 用 `datas=[(來源路徑, "wizwalker/…")]` 補。把所有補丁連同原因註解記錄在 build.spec。

- [ ] **Step 5: 跑測試並 commit**

```bash
uv run pytest
git add pyproject.toml uv.lock build.spec
git commit -m "build: PyInstaller 打包設定與冒煙驗證(spike)"
```

---

### Task 2: config——應用程式目錄、provider 欄位與舊設定遷移

**Files:**
- Modify: `src/config.py`
- Modify: `src/main.py:19`（`CONFIG_PATH` 改自 config 模組取得）
- Test: `tests/test_config.py`

**Interfaces:**
- Produces:
  - `config.app_dir() -> Path`：frozen 時為 `sys.executable` 所在目錄，否則專案根目錄。
  - `config.CONFIG_PATH: Path`＝`app_dir() / "config.json"`。
  - `DEFAULT_CONFIG["api"]` 新增 `"provider": "openai"`。
  - `config.is_configured(cfg: dict) -> bool`：API 設定是否完整（精靈觸發條件）。
  - `load_config` 遷移規則：檔案存在、有 `api` 區塊但無 `provider` → 視為 `"custom"`。

- [ ] **Step 1: 寫失敗測試（加進 tests/test_config.py）**

```python
import sys

from src.config import app_dir, is_configured


def test_app_dir_dev_mode_is_project_root():
    # 開發模式(非 frozen):專案根目錄(pyproject.toml 所在)
    assert (app_dir() / "pyproject.toml").exists()


def test_app_dir_frozen_uses_executable_dir(monkeypatch, tmp_path):
    monkeypatch.setattr(sys, "frozen", True, raising=False)
    monkeypatch.setattr(sys, "executable", str(tmp_path / "Wizard101ChatTranslator.exe"))
    assert app_dir() == tmp_path


def test_load_old_config_without_provider_migrates_to_custom(tmp_path):
    p = tmp_path / "config.json"
    p.write_text(json.dumps({"api": {"base_url": "http://127.0.0.1:8000", "model": "m1"}}),
                 encoding="utf-8")
    cfg = load_config(p)
    assert cfg["api"]["provider"] == "custom"  # 舊使用者的自架端點設定原封不動繼續用


def test_load_config_without_api_block_keeps_default_provider(tmp_path):
    p = tmp_path / "config.json"
    p.write_text(json.dumps({"hotkey": "f8"}), encoding="utf-8")
    cfg = load_config(p)
    assert cfg["api"]["provider"] == "openai"


def test_is_configured():
    cfg = load_config(Path("nope.json"))
    assert not is_configured(cfg)  # model 空
    cfg["api"].update(provider="openai", model="gpt-x", api_key="sk-1")
    assert is_configured(cfg)
    cfg["api"]["api_key"] = ""
    assert not is_configured(cfg)  # openai/claude 需要金鑰
    cfg["api"].update(provider="custom", model="m", base_url="http://127.0.0.1:8000")
    assert is_configured(cfg)  # custom 不需金鑰,需 base_url
    cfg["api"]["base_url"] = ""
    assert not is_configured(cfg)
```

- [ ] **Step 2: 跑測試確認失敗**

```bash
uv run pytest tests/test_config.py -v
```

預期：新測試 FAIL（`ImportError: cannot import name 'app_dir'`）。

- [ ] **Step 3: 實作 src/config.py**

在檔案頂部加 `import sys`，並加入：

```python
def app_dir() -> Path:
    """應用程式目錄:config.json 與 log 的存放處。
    打包執行(frozen)時為 exe 所在目錄;開發時為專案根目錄。"""
    if getattr(sys, "frozen", False):
        return Path(sys.executable).parent
    return Path(__file__).resolve().parent.parent


CONFIG_PATH = app_dir() / "config.json"
```

`DEFAULT_CONFIG["api"]` 改為：

```python
    "api": {"provider": "openai", "base_url": "http://127.0.0.1:8000",
            "model": "", "api_key": "", "thinking": False},
```

`load_config` 在 merge 前加遷移：

```python
def load_config(path: Path) -> dict:
    if not path.exists():
        return copy.deepcopy(DEFAULT_CONFIG)
    data = json.loads(path.read_text(encoding="utf-8"))
    # 舊版 config 沒有 provider 欄位:一律視為自訂端點,原設定不動
    if isinstance(data.get("api"), dict) and "provider" not in data["api"]:
        data["api"]["provider"] = "custom"
    return _merge(DEFAULT_CONFIG, data)
```

新增：

```python
def is_configured(cfg: dict) -> bool:
    """API 設定是否完整(不完整 → 啟動時進首次設定精靈)。"""
    api = cfg["api"]
    if not api["model"]:
        return False
    if api["provider"] in ("openai", "claude"):
        return bool(api["api_key"])
    return bool(api["base_url"])
```

- [ ] **Step 4: main.py 改用共用路徑**

`src/main.py:19` 的 `CONFIG_PATH = Path("config.json")` 刪除，import 改為：

```python
from src.config import CONFIG_PATH, load_config, save_config
```

（`from pathlib import Path` 若因此不再使用則一併移除。）

- [ ] **Step 5: 跑全部測試確認通過後 commit**

```bash
uv run pytest
git add src/config.py src/main.py tests/test_config.py
git commit -m "feat: config 支援應用程式目錄、provider 欄位與舊設定遷移"
```

---

### Task 3: translator——provider client 拆分與錯誤分類

**Files:**
- Modify: `pyproject.toml`（依賴加 anthropic）
- Modify: `src/translator.py`
- Modify: `src/main.py:45-121`（reader_loop 錯誤處理）
- Test: `tests/test_translator.py`（新檔）

**Interfaces:**
- Consumes: `cfg["api"]` 含 `provider/base_url/model/api_key/thinking`（Task 2）。
- Produces:
  - `class TranslatorOffline(Exception)`：可重試（連線失敗、逾時、429、5xx）。
  - `class TranslatorConfigError(Exception)`：不可重試（401／403 金鑰無效、404 模型不存在），帶 `status: int | None` 屬性。
  - `Translator(*, provider="custom", base_url="", model="", api_key="", thinking=False, target_language, timeout=60.0, client=None)`——`translate_incoming`／`translate_outgoing` 介面不變。
  - `Translator.reconfigure(*, provider, base_url, model, api_key, thinking, target_language)`：設定儲存後就地重建 client（Task 6 用）。
  - `translator.test_translate(api: dict, target_language: str) -> str`：測試連線用（Task 4 用）。
  - `translator.OPENAI_BASE_URL = "https://api.openai.com"`。

- [ ] **Step 1: 加依賴**

```bash
uv add anthropic
```

- [ ] **Step 2: 寫失敗測試 tests/test_translator.py**

```python
"""translator 的 provider 選擇與錯誤映射測試(mock client,不打真 API)。"""
import anthropic
import httpx
import httpx2
import pytest

from src.translator import (
    OPENAI_BASE_URL, Translator, TranslatorConfigError, TranslatorOffline,
)


class FakeResponse:
    def __init__(self, status_code=200, content="譯文"):
        self.status_code = status_code
        self._content = content

    def json(self):
        return {"choices": [{"message": {"content": self._content}}]}

    def raise_for_status(self):
        if self.status_code >= 400:
            raise httpx.HTTPStatusError("err", request=None, response=None)


class FakeHttpxClient:
    """替身 httpx.Client:post 回傳預設回應或拋出預設例外。"""
    def __init__(self, response=None, raises=None):
        self._response = response or FakeResponse()
        self._raises = raises
        self.last_body = None

    def post(self, url, json):
        self.last_body = json
        if self._raises:
            raise self._raises
        return self._response


def _make(client, provider="custom"):
    return Translator(provider=provider, base_url="http://x", model="m",
                      target_language="繁體中文（台灣）", client=client)


def test_openai_compat_sends_temperature_zero():
    fake = FakeHttpxClient()
    assert _make(fake).translate_outgoing("哈囉") == "譯文"
    assert fake.last_body["temperature"] == 0


def test_openai_compat_connection_error_maps_to_offline():
    fake = FakeHttpxClient(raises=httpx.ConnectError("refused"))
    with pytest.raises(TranslatorOffline):
        _make(fake).translate_incoming("[A] hi")


@pytest.mark.parametrize("status", [401, 403, 404])
def test_openai_compat_auth_or_model_error_maps_to_config_error(status):
    fake = FakeHttpxClient(response=FakeResponse(status_code=status))
    with pytest.raises(TranslatorConfigError) as ei:
        _make(fake).translate_incoming("[A] hi")
    assert ei.value.status == status


@pytest.mark.parametrize("status", [429, 500, 503])
def test_openai_compat_retryable_status_maps_to_offline(status):
    fake = FakeHttpxClient(response=FakeResponse(status_code=status))
    with pytest.raises(TranslatorOffline):
        _make(fake).translate_incoming("[A] hi")


class FakeAnthropicMessages:
    def __init__(self, raises=None):
        self._raises = raises

    def create(self, **kwargs):
        if self._raises:
            raise self._raises

        class Block:
            type = "text"
            text = "克勞德譯文"

        class Resp:
            content = [Block()]

        return Resp()


class FakeAnthropicClient:
    def __init__(self, raises=None):
        self.messages = FakeAnthropicMessages(raises)


def _anthropic_status_error(status):
    resp = httpx2.Response(status, request=httpx2.Request("POST", "http://x"))
    return anthropic.APIStatusError("err", response=resp, body=None)


def test_claude_provider_returns_text():
    t = Translator(provider="claude", model="claude-opus-5", api_key="k",
                   target_language="繁體中文（台灣）", client=FakeAnthropicClient())
    assert t.translate_incoming("[A] hi") == "克勞德譯文"


def test_claude_connection_error_maps_to_offline():
    err = anthropic.APIConnectionError(request=httpx2.Request("POST", "http://x"))
    t = Translator(provider="claude", model="m", api_key="k",
                   target_language="繁體中文（台灣）", client=FakeAnthropicClient(raises=err))
    with pytest.raises(TranslatorOffline):
        t.translate_incoming("[A] hi")


@pytest.mark.parametrize("status,exc", [(401, TranslatorConfigError),
                                        (404, TranslatorConfigError),
                                        (429, TranslatorOffline),
                                        (500, TranslatorOffline)])
def test_claude_status_error_mapping(status, exc):
    t = Translator(provider="claude", model="m", api_key="k",
                   target_language="繁體中文（台灣）",
                   client=FakeAnthropicClient(raises=_anthropic_status_error(status)))
    with pytest.raises(exc):
        t.translate_incoming("[A] hi")


def test_reconfigure_switches_provider():
    t = _make(FakeHttpxClient())
    t.reconfigure(provider="claude", base_url="", model="claude-opus-5", api_key="k",
                  thinking=False, target_language="日本語")
    # reconfigure 後為 Claude client(真物件);此處只驗證型別切換,不打 API
    from src.translator import _ClaudeClient
    assert isinstance(t._impl, _ClaudeClient)
```

- [ ] **Step 3: 跑測試確認失敗**

```bash
uv run pytest tests/test_translator.py -v
```

預期：FAIL（`ImportError: cannot import name 'TranslatorOffline'`）。

- [ ] **Step 4: 改寫 src/translator.py**

保留：模組 docstring 主旨、`OUTGOING_LANGUAGE`、`build_incoming_system`、`build_outgoing_system`、`_DISABLE_THINKING`、`strip_think`。`Translator` 類與其後改為：

```python
import anthropic
import httpx

OPENAI_BASE_URL = "https://api.openai.com"  # ChatGPT preset 固定官方端點
_TIMEOUT = 60.0
_CLAUDE_MAX_TOKENS = 1024
TEST_SAMPLE = "[Tester] Hello! How are you?"  # 測試連線用固定原文


class TranslatorOffline(Exception):
    """可重試的翻譯失敗:連線失敗、逾時、429、5xx。"""


class TranslatorConfigError(Exception):
    """不可重試的設定錯誤:金鑰無效(401/403)、模型不存在(404)。"""

    def __init__(self, message: str, status: int | None = None):
        super().__init__(message)
        self.status = status


class _OpenAICompatClient:
    """OpenAI 相容端點(ChatGPT 官方與自訂伺服器共用):打 /v1/chat/completions。"""

    def __init__(self, base_url: str, model: str, api_key: str = "",
                 thinking: bool = True, timeout: float = _TIMEOUT, client=None):
        if client is not None:
            self._client = client
        else:
            headers = {"Authorization": f"Bearer {api_key}"} if api_key else {}
            self._client = httpx.Client(base_url=base_url, headers=headers, timeout=timeout)
        self._model = model
        self._thinking = thinking

    def chat(self, system: str, text: str) -> str:
        body = {
            "model": self._model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": text},
            ],
            "temperature": 0,
        }
        if not self._thinking:
            body.update(_DISABLE_THINKING)
        try:
            resp = self._client.post("/v1/chat/completions", json=body)
        except httpx.HTTPError as exc:
            raise TranslatorOffline(str(exc)) from exc
        if resp.status_code in (401, 403, 404):
            raise TranslatorConfigError(f"HTTP {resp.status_code}", status=resp.status_code)
        if resp.status_code == 429 or resp.status_code >= 500:
            raise TranslatorOffline(f"HTTP {resp.status_code}")
        resp.raise_for_status()
        content = resp.json()["choices"][0]["message"]["content"]
        return strip_think(content).strip()


class _ClaudeClient:
    """Claude 官方 API(anthropic SDK):打 /v1/messages。
    Claude 5 系不接受 temperature(會 400),thinking 用預設(adaptive),皆不帶。"""

    def __init__(self, model: str, api_key: str, timeout: float = _TIMEOUT, client=None):
        self._client = client if client is not None else anthropic.Anthropic(
            api_key=api_key, timeout=timeout)
        self._model = model

    def chat(self, system: str, text: str) -> str:
        try:
            resp = self._client.messages.create(
                model=self._model, max_tokens=_CLAUDE_MAX_TOKENS,
                system=system, messages=[{"role": "user", "content": text}])
        except anthropic.APIConnectionError as exc:
            raise TranslatorOffline(str(exc)) from exc
        except anthropic.APIStatusError as exc:
            code = exc.status_code
            if code in (401, 403, 404):
                raise TranslatorConfigError(f"HTTP {code}", status=code) from exc
            if code == 429 or code >= 500:
                raise TranslatorOffline(f"HTTP {code}") from exc
            raise
        content = "".join(b.text for b in resp.content if b.type == "text")
        return strip_think(content).strip()


def _build_client(provider: str, base_url: str, model: str, api_key: str,
                  thinking: bool, timeout: float, client):
    if provider == "claude":
        return _ClaudeClient(model=model, api_key=api_key, timeout=timeout, client=client)
    if provider == "openai":
        base_url = OPENAI_BASE_URL
    return _OpenAICompatClient(base_url=base_url, model=model, api_key=api_key,
                               thinking=thinking, timeout=timeout, client=client)


class Translator:
    """共用翻譯 client:依 provider 選擇後端,收訊/發話介面不變。"""

    def __init__(self, *, provider: str = "custom", base_url: str = "", model: str = "",
                 api_key: str = "", thinking: bool = True, target_language: str,
                 timeout: float = _TIMEOUT, client=None):
        self._impl = _build_client(provider, base_url, model, api_key, thinking,
                                   timeout, client)
        self._target_language = target_language

    def reconfigure(self, *, provider: str, base_url: str, model: str, api_key: str,
                    thinking: bool, target_language: str) -> None:
        """設定變更後就地重建後端 client(呼叫端不需換 Translator 實例)。"""
        self._impl = _build_client(provider, base_url, model, api_key, thinking,
                                   _TIMEOUT, None)
        self._target_language = target_language

    def translate_incoming(self, text: str) -> str:
        """收訊:把遊戲聊天(任何語言)翻成使用者設定的目標語言。"""
        return self._impl.chat(build_incoming_system(self._target_language), text)

    def translate_outgoing(self, text: str) -> str:
        """發話:把玩家輸入(任何語言)翻成遊戲聊天語言(固定)。"""
        return self._impl.chat(build_outgoing_system(OUTGOING_LANGUAGE), text)


def test_translate(api: dict, target_language: str) -> str:
    """測試連線:用表單當下的 api 設定實際翻一句固定文字,與正式翻譯同一條路。"""
    return Translator(**api, target_language=target_language).translate_incoming(TEST_SAMPLE)
```

- [ ] **Step 5: main.py reader_loop 改抓新例外**

`src/main.py`：import 區把 `from src.translator import Translator` 改為

```python
from src.translator import Translator, TranslatorConfigError, TranslatorOffline
```

`import httpx` 若因此不再使用則移除。常數區加：

```python
CONFIG_ERROR_INTERVAL = 15.0  # API 設定錯誤時的重試間隔(秒);使用者修正後自動恢復
```

`reader_loop` 內翻譯迴圈與其後改為（`went_offline` 上方加 `config_error = False`、迴圈外層 `backoff_index` 旁加 `error_banner = False`）：

```python
        while pending:
            line = pending[0]
            try:
                translated = translator.translate_incoming(line)
            except TranslatorOffline:
                went_offline = True  # line 留在 pending,下輪重試
                break
            except TranslatorConfigError:
                config_error = True  # 設定錯誤:行留在 pending,等使用者修正後自動恢復
                break
            except Exception as exc:
                print(f"[translate] 略過此行（{exc}）：{line}", file=sys.stderr)
                pending.popleft()
                continue
            translated_ok = True
            pending.popleft()
            ui_queue.put(lambda o=line, t=translated: overlay.add_message(o, t))

        if config_error:
            interval = CONFIG_ERROR_INTERVAL
            error_banner = True
            ui_queue.put(lambda: overlay.set_error("⚠  API 設定有誤，請開啟設定（⚙）檢查"))
        elif went_offline:
            interval = BACKOFF_STEPS[min(backoff_index, len(BACKOFF_STEPS) - 1)]
            backoff_index += 1
            error_banner = True
            ui_queue.put(lambda: overlay.set_error("⚠  翻譯伺服器離線，重試中…"))
        else:
            set_status("listening" if reader.anchored else "locating")
            if translated_ok and error_banner:
                # 真的翻譯成功 → 伺服器/設定已恢復,清橫幅並重置退避
                error_banner = False
                backoff_index = 0
                ui_queue.put(overlay.clear_error)
```

（原本 `if translated_ok and backoff_index:` 的清除邏輯由 `error_banner` 取代，涵蓋設定錯誤恢復的情境。）

- [ ] **Step 6: 跑全部測試確認通過後 commit**

```bash
uv run pytest
git add pyproject.toml uv.lock src/translator.py src/main.py tests/test_translator.py
git commit -m "feat: translator 拆 provider client(OpenAI 相容/Claude)與錯誤分類"
```

---

### Task 4: fields.py——共用欄位元件與純邏輯

**Files:**
- Create: `src/ui/__init__.py`（空檔案＋一行 docstring）
- Create: `src/ui/fields.py`
- Test: `tests/test_fields.py`（新檔）

**Interfaces:**
- Consumes: `translator.test_translate`、`TranslatorOffline`、`TranslatorConfigError`（Task 3）。
- Produces（Task 5／6 用）:
  - `PROVIDERS: dict[str, Provider]`（key：`"openai"`／`"claude"`／`"custom"`；`Provider` 有 `label`、`needs_base_url: bool`、`models: list[str]`、`key_url: str | None`）。
  - `validate_api_form(api: dict) -> list[str]`：錯誤訊息列表，空＝通過。
  - `friendly_error(exc: Exception) -> str`：例外 → 人話錯誤。
  - `hotkey_from_event(keysym: str, state: int) -> str | None`：tkinter 按鍵事件 → keyboard 格式熱鍵字串。
  - `COMMON_LANGUAGES: list[str]`。
  - `class ApiFields(ttk.Frame)`：`get_values() -> dict`（api 區塊）、`set_values(api)`、`run_test(target_language_fn)`、`test_passed: bool`。
  - `class HotkeyField(ttk.Frame)`：`value() -> str`、`set_value(s)`。
  - `class LanguageField(ttk.Frame)`：`value() -> str`、`set_value(s)`。

- [ ] **Step 1: 查證 OpenAI 現行模型 ID**

WebFetch `https://platform.openai.com/docs/models`（或搜尋「OpenAI models list <當前年份>」）確認現行主流 chat 模型 ID，取 2–3 個（第一個當預設）填入 Step 3 的 `PROVIDERS["openai"].models`。查不到時用 fallback `["gpt-5.1", "gpt-5.1-mini", "gpt-5"]` 並在 commit message 註明未查證。Claude 固定 `["claude-opus-5", "claude-sonnet-5", "claude-haiku-4-5"]`（已依 claude-api 參考查證）。

- [ ] **Step 2: 寫失敗測試 tests/test_fields.py（純邏輯部分）**

```python
"""fields 純邏輯測試:表單驗證、錯誤文案、熱鍵字串。"""
from src.translator import TranslatorConfigError, TranslatorOffline
from src.ui.fields import (
    PROVIDERS, friendly_error, hotkey_from_event, validate_api_form,
)


def test_providers_metadata():
    assert set(PROVIDERS) == {"openai", "claude", "custom"}
    assert PROVIDERS["custom"].needs_base_url
    assert not PROVIDERS["openai"].needs_base_url
    assert PROVIDERS["openai"].models  # 有預設模型清單
    assert PROVIDERS["claude"].models[0] == "claude-opus-5"


def test_validate_requires_model():
    errs = validate_api_form({"provider": "openai", "model": "", "api_key": "k",
                              "base_url": "", "thinking": False})
    assert any("模型" in e for e in errs)


def test_validate_requires_key_for_official_providers():
    errs = validate_api_form({"provider": "claude", "model": "claude-opus-5",
                              "api_key": "", "base_url": "", "thinking": False})
    assert any("金鑰" in e for e in errs)


def test_validate_requires_base_url_for_custom_only():
    api = {"provider": "custom", "model": "m", "api_key": "", "base_url": "",
           "thinking": False}
    assert any("網址" in e for e in validate_api_form(api))
    api["base_url"] = "http://127.0.0.1:8000"
    assert validate_api_form(api) == []  # custom 不需金鑰


def test_friendly_error_messages():
    assert "金鑰" in friendly_error(TranslatorConfigError("HTTP 401", status=401))
    assert "模型" in friendly_error(TranslatorConfigError("HTTP 404", status=404))
    assert "連線" in friendly_error(TranslatorOffline("refused"))


def test_hotkey_from_event():
    assert hotkey_from_event("space", 0x4) == "ctrl+space"
    assert hotkey_from_event("F8", 0) == "f8"
    assert hotkey_from_event("x", 0x4 | 0x20000) == "ctrl+alt+x"
    assert hotkey_from_event("Control_L", 0x4) is None  # 純修飾鍵不成立
```

- [ ] **Step 3: 跑測試確認失敗，然後實作 src/ui/fields.py**

`src/ui/__init__.py`：

```python
"""設定相關 UI:首次設定精靈、一般設定視窗與共用欄位元件。"""
```

`src/ui/fields.py`：

```python
"""精靈與設定視窗共用的欄位元件與純邏輯:
服務商選擇、API 欄位、測試連線、熱鍵捕捉、語言選擇。"""
import queue
import threading
import tkinter as tk
import webbrowser
from dataclasses import dataclass, field
from tkinter import ttk

from src.translator import TranslatorConfigError, TranslatorOffline, test_translate


@dataclass(frozen=True)
class Provider:
    label: str
    needs_base_url: bool
    models: list[str] = field(default_factory=list)
    key_url: str | None = None


PROVIDERS: dict[str, Provider] = {
    "openai": Provider(label="ChatGPT（OpenAI）", needs_base_url=False,
                       models=["<Step 1 查證的模型 ID>", "<次選>", "<三選>"],
                       key_url="https://platform.openai.com/api-keys"),
    "claude": Provider(label="Claude（Anthropic）", needs_base_url=False,
                       models=["claude-opus-5", "claude-sonnet-5", "claude-haiku-4-5"],
                       key_url="https://console.anthropic.com/settings/keys"),
    "custom": Provider(label="自訂端點（進階）", needs_base_url=True),
}

COMMON_LANGUAGES = ["繁體中文（台灣）", "简体中文", "日本語", "한국어",
                    "Español", "Português", "Deutsch", "Français"]

CLAUDE_MODEL_HINT = "較快較省：claude-haiku-4-5"


def validate_api_form(api: dict) -> list[str]:
    """檢查 API 表單必填欄位,回傳錯誤訊息列表(空=通過)。"""
    errors = []
    provider = PROVIDERS[api["provider"]]
    if not api["model"].strip():
        errors.append("請選擇或輸入模型")
    if not provider.needs_base_url and not api["api_key"].strip():
        errors.append("請輸入 API 金鑰")
    if provider.needs_base_url and not api["base_url"].strip():
        errors.append("請輸入伺服器網址")
    return errors


def friendly_error(exc: Exception) -> str:
    """把翻譯例外轉成一般使用者看得懂的錯誤訊息。"""
    if isinstance(exc, TranslatorConfigError):
        if exc.status in (401, 403):
            return "金鑰無效或過期，請確認 API 金鑰"
        return "找不到模型，請確認模型名稱"
    if isinstance(exc, TranslatorOffline):
        return "無法連線到伺服器，請檢查網址與網路"
    return f"發生錯誤：{exc}"


_MOD_KEYSYMS = {"Control_L", "Control_R", "Shift_L", "Shift_R",
                "Alt_L", "Alt_R", "Win_L", "Win_R"}


def hotkey_from_event(keysym: str, state: int) -> str | None:
    """tkinter 按鍵事件 → keyboard 套件格式的熱鍵字串(如 "ctrl+alt+x")。
    只按到修飾鍵本身時回 None(組合尚未完成)。"""
    if keysym in _MOD_KEYSYMS:
        return None
    parts = []
    if state & 0x4:
        parts.append("ctrl")
    if state & 0x20000:  # Windows 的 Alt 位元
        parts.append("alt")
    if state & 0x1:
        parts.append("shift")
    parts.append(keysym.lower())
    return "+".join(parts)


class ApiFields(ttk.Frame):
    """API 設定欄位群:服務商 radio + 動態欄位 + 測試連線。"""

    def __init__(self, parent, initial: dict, on_change=None):
        super().__init__(parent)
        self._on_change = on_change
        self.test_passed = False
        self._queue: queue.Queue = queue.Queue()  # 測試結果由背景執行緒送回主執行緒
        self._provider = tk.StringVar(value=initial["provider"])
        self._api_key = tk.StringVar(value=initial["api_key"])
        self._model = tk.StringVar(value=initial["model"])
        self._base_url = tk.StringVar(value=initial["base_url"])
        self._thinking = tk.BooleanVar(value=initial["thinking"])
        for var in (self._api_key, self._model, self._base_url):
            var.trace_add("write", lambda *_: self._invalidate_test())

        radio_row = ttk.Frame(self)
        radio_row.pack(fill="x", pady=(0, 6))
        for key, prov in PROVIDERS.items():
            ttk.Radiobutton(radio_row, text=prov.label, value=key,
                            variable=self._provider,
                            command=self._rebuild_fields).pack(anchor="w")

        self._fields = ttk.Frame(self)
        self._fields.pack(fill="x")

        test_row = ttk.Frame(self)
        test_row.pack(fill="x", pady=(8, 0))
        self._test_btn = ttk.Button(test_row, text="測試連線", command=self._start_test)
        self._test_btn.pack(side="left")
        self._test_result = ttk.Label(test_row, text="", wraplength=360)
        self._test_result.pack(side="left", padx=8)

        self._target_language_fn = lambda: "繁體中文（台灣）"
        self._rebuild_fields()

    # --- 值存取 ---
    def get_values(self) -> dict:
        return {"provider": self._provider.get(), "api_key": self._api_key.get().strip(),
                "model": self._model.get().strip(),
                "base_url": self._base_url.get().strip(),
                "thinking": self._thinking.get()}

    def set_values(self, api: dict) -> None:
        self._provider.set(api["provider"])
        self._api_key.set(api["api_key"])
        self._model.set(api["model"])
        self._base_url.set(api["base_url"])
        self._thinking.set(api["thinking"])
        self._rebuild_fields()

    def set_target_language_fn(self, fn) -> None:
        """測試連線時取得目標語言的 callback(精靈階段語言還沒選,用預設)。"""
        self._target_language_fn = fn

    # --- 動態欄位 ---
    def _rebuild_fields(self) -> None:
        for w in self._fields.winfo_children():
            w.destroy()
        prov = PROVIDERS[self._provider.get()]
        if prov.needs_base_url:
            self._labeled_entry("伺服器網址", self._base_url)
            self._labeled_entry("模型名稱", self._model)
            self._labeled_entry("API 金鑰（選填）", self._api_key, secret=True)
            ttk.Checkbutton(self._fields, text="啟用模型思考（thinking）",
                            variable=self._thinking).pack(anchor="w", pady=2)
        else:
            self._labeled_entry("API 金鑰", self._api_key, secret=True)
            row = ttk.Frame(self._fields)
            row.pack(fill="x", pady=2)
            ttk.Label(row, text="模型").pack(side="left")
            combo = ttk.Combobox(row, textvariable=self._model, values=prov.models)
            combo.pack(side="left", fill="x", expand=True, padx=(8, 0))
            if not self._model.get():
                self._model.set(prov.models[0])
            if self._provider.get() == "claude":
                ttk.Label(self._fields, text=CLAUDE_MODEL_HINT,
                          foreground="#888888").pack(anchor="w")
            link = ttk.Label(self._fields, text="取得金鑰 ↗", foreground="#4a7ddc",
                             cursor="hand2")
            link.pack(anchor="w", pady=(2, 0))
            link.bind("<Button-1>", lambda e: webbrowser.open(prov.key_url))
        self._invalidate_test()
        if self._on_change:
            self._on_change()

    def _labeled_entry(self, label: str, var: tk.StringVar, secret: bool = False):
        row = ttk.Frame(self._fields)
        row.pack(fill="x", pady=2)
        ttk.Label(row, text=label, width=14).pack(side="left")
        entry = ttk.Entry(row, textvariable=var, show="●" if secret else "")
        entry.pack(side="left", fill="x", expand=True)
        if secret:
            btn = ttk.Button(row, text="顯示", width=5,
                             command=lambda: entry.configure(
                                 show="" if entry.cget("show") else "●"))
            btn.pack(side="left", padx=(4, 0))

    # --- 測試連線 ---
    def _invalidate_test(self) -> None:
        self.test_passed = False

    def _start_test(self) -> None:
        api = self.get_values()
        errors = validate_api_form(api)
        if errors:
            self._show_test_result(False, "；".join(errors))
            return
        self._test_btn.configure(state="disabled", text="測試中…")
        self._test_result.configure(text="")
        target = self._target_language_fn()
        threading.Thread(target=self._test_worker, args=(api, target),
                         daemon=True).start()
        self._poll_result()

    def _test_worker(self, api: dict, target_language: str) -> None:
        try:
            sample = test_translate(api, target_language)
        except Exception as exc:
            self._queue.put((False, friendly_error(exc)))
            return
        self._queue.put((True, f"連線成功　範例：{sample}"))

    def _poll_result(self) -> None:
        # tkinter 的 after 不保證跨執行緒安全:worker 只放 queue,主執行緒輪詢取用
        try:
            ok, message = self._queue.get_nowait()
        except queue.Empty:
            self.after(100, self._poll_result)
            return
        self._test_btn.configure(state="normal", text="測試連線")
        self._show_test_result(ok, message)

    def _show_test_result(self, ok: bool, message: str) -> None:
        self.test_passed = ok
        prefix = "✓ " if ok else "✗ "
        color = "#2e8b57" if ok else "#cc3333"
        self._test_result.configure(text=prefix + message, foreground=color)
        if self._on_change:
            self._on_change()


class HotkeyField(ttk.Frame):
    """熱鍵欄位:顯示目前值,點「更改」進入捕捉模式,按下組合鍵即設定。"""

    def __init__(self, parent, initial: str):
        super().__init__(parent)
        self._value = initial
        self._label = ttk.Label(self, text=initial)
        self._label.pack(side="left")
        self._btn = ttk.Button(self, text="更改", width=6, command=self._capture)
        self._btn.pack(side="left", padx=8)

    def value(self) -> str:
        return self._value

    def set_value(self, s: str) -> None:
        self._value = s
        self._label.configure(text=s)

    def _capture(self) -> None:
        self._btn.configure(text="請按鍵…", state="disabled")
        top = self.winfo_toplevel()
        top.grab_set()
        binding = top.bind("<Key>", lambda e: self._on_key(e, top), add="+")
        self._binding = binding

    def _on_key(self, event, top) -> None:
        combo = hotkey_from_event(event.keysym, event.state)
        if combo is None:
            return  # 只按到修飾鍵,等組合完成
        self.set_value(combo)
        top.unbind("<Key>", self._binding)
        top.grab_release()
        self._btn.configure(text="更改", state="normal")


class LanguageField(ttk.Frame):
    """翻譯目標語言:常用語言下拉+可自行輸入。"""

    def __init__(self, parent, initial: str):
        super().__init__(parent)
        self._var = tk.StringVar(value=initial)
        combo = ttk.Combobox(self, textvariable=self._var, values=COMMON_LANGUAGES)
        combo.pack(fill="x")

    def value(self) -> str:
        return self._var.get().strip()

    def set_value(self, s: str) -> None:
        self._var.set(s)
```

注意：`PROVIDERS["openai"].models` 裡的 `<Step 1 查證的模型 ID>` 佔位字必須替換成 Step 1 查證的實際 ID，不可原樣提交。

- [ ] **Step 4: 跑測試確認通過後 commit**

```bash
uv run pytest
git add src/ui/ tests/test_fields.py
git commit -m "feat: 設定 UI 共用欄位元件(服務商/金鑰/測試連線/熱鍵/語言)"
```

---

### Task 5: 首次設定精靈與啟動整合

**Files:**
- Create: `src/ui/wizard.py`
- Modify: `src/main.py:124-127`（main() 開頭）
- Test: `tests/test_wizard.py`（新檔）

**Interfaces:**
- Consumes: `ApiFields`／`HotkeyField`／`LanguageField`／`validate_api_form`（Task 4）、`is_configured`（Task 2）。
- Produces:
  - `wizard.run_wizard(root: tk.Tk, cfg: dict) -> bool`：跑完精靈回 `True` 並把結果寫進 `cfg`（記憶體）；使用者中途關閉回 `False`。呼叫端負責 `save_config`。
  - `wizard.can_advance(step: int, api_test_passed: bool, api_errors: list[str]) -> bool`：步驟前進的門檻判斷（純函式）。

- [ ] **Step 1: 寫失敗測試 tests/test_wizard.py**

```python
"""精靈步驟門檻的純邏輯測試(UI 互動靠實機驗證)。"""
from src.ui.wizard import STEP_API, STEP_TEST, can_advance


def test_step_api_requires_valid_form():
    assert not can_advance(STEP_API, api_test_passed=False, api_errors=["請輸入 API 金鑰"])
    assert can_advance(STEP_API, api_test_passed=False, api_errors=[])


def test_step_test_requires_passed_test():
    assert not can_advance(STEP_TEST, api_test_passed=False, api_errors=[])
    assert can_advance(STEP_TEST, api_test_passed=True, api_errors=[])
```

- [ ] **Step 2: 跑測試確認失敗，然後實作 src/ui/wizard.py**

```python
"""首次設定精靈:選服務商 → 填 API → 測試連線 → 偏好設定,4 步完成寫入 cfg。
中途關閉=取消(不留半套設定),run_wizard 回傳 False。"""
import tkinter as tk
from tkinter import ttk

from src.ui.fields import ApiFields, HotkeyField, LanguageField, validate_api_form

STEP_WELCOME, STEP_API, STEP_TEST, STEP_DONE = 0, 1, 2, 3
_TITLES = ["歡迎使用", "API 設定", "測試連線", "偏好設定"]


def can_advance(step: int, api_test_passed: bool, api_errors: list[str]) -> bool:
    """該步驟是否允許按「下一步」。"""
    if step == STEP_API:
        return not api_errors
    if step == STEP_TEST:
        return api_test_passed
    return True


class SetupWizard:
    """精靈視窗本體。completed 屬性表示是否走完全部步驟。"""

    def __init__(self, root: tk.Tk, cfg: dict):
        self._cfg = cfg
        self.completed = False
        self._step = STEP_WELCOME
        self._skip_test = False

        self._win = tk.Toplevel(root)
        self._win.title("Wizard101 聊天翻譯助手 — 首次設定")
        self._win.geometry("520x420")
        self._win.resizable(False, False)
        self._win.protocol("WM_DELETE_WINDOW", self._cancel)

        self._indicator = ttk.Label(self._win, text="")
        self._indicator.pack(pady=(10, 0))
        self._title = ttk.Label(self._win, font=("Microsoft JhengHei", 13, "bold"))
        self._title.pack(pady=(2, 8))
        self._body = ttk.Frame(self._win, padding=16)
        self._body.pack(fill="both", expand=True)

        nav = ttk.Frame(self._win, padding=8)
        nav.pack(side="bottom", fill="x")
        self._back_btn = ttk.Button(nav, text="上一步", command=self._back)
        self._back_btn.pack(side="left")
        self._next_btn = ttk.Button(nav, text="下一步", command=self._next)
        self._next_btn.pack(side="right")

        # 跨步驟保留的欄位元件(建一次,切步驟時搬進/搬出 body)
        self._api_fields = ApiFields(self._body, cfg["api"], on_change=self._refresh_nav)
        self._language = LanguageField(self._body, cfg["target_language"])
        self._hotkey = HotkeyField(self._body, cfg["hotkey"])
        self._show_step()

    # --- 導航 ---
    def _show_step(self) -> None:
        for w in self._body.winfo_children():
            w.pack_forget()
        self._indicator.configure(
            text="  ".join("●" if i <= self._step else "○" for i in range(4)))
        self._title.configure(text=_TITLES[self._step])

        if self._step == STEP_WELCOME:
            ttk.Label(self._body, wraplength=440, text=(
                "本工具會即時翻譯 Wizard101 的遊戲聊天，並可用熱鍵輸入你的語言、"
                "翻成英文送進遊戲。\n\n首先，請選擇要使用的翻譯服務：")).pack(anchor="w")
            self._api_fields.pack(fill="x", pady=(12, 0))
        elif self._step == STEP_API:
            self._api_fields.pack(fill="x")
        elif self._step == STEP_TEST:
            ttk.Label(self._body, wraplength=440, text=(
                "按「測試連線」確認設定可用（會實際翻譯一句測試文字）。")).pack(anchor="w")
            self._api_fields.pack(fill="x", pady=(12, 0))
            skip = ttk.Label(self._body, text="略過測試", foreground="#888888",
                             cursor="hand2", font=("Microsoft JhengHei", 8))
            skip.pack(anchor="e", pady=(6, 0))
            skip.bind("<Button-1>", lambda e: self._do_skip_test())
        else:  # STEP_DONE
            ttk.Label(self._body, text="翻譯目標語言（收到的訊息翻成什麼語言）").pack(anchor="w")
            self._language.pack(fill="x", pady=(2, 12))
            ttk.Label(self._body, text="呼出輸入框的熱鍵").pack(anchor="w")
            self._hotkey.pack(anchor="w", pady=(2, 0))
            self._next_btn.configure(text="完成")
        if self._step != STEP_DONE:
            self._next_btn.configure(text="下一步")
        self._back_btn.configure(
            state="normal" if self._step > STEP_WELCOME else "disabled")
        self._refresh_nav()

    def _refresh_nav(self) -> None:
        api = self._api_fields.get_values()
        ok = can_advance(self._step,
                         self._api_fields.test_passed or self._skip_test,
                         validate_api_form(api))
        self._next_btn.configure(state="normal" if ok else "disabled")

    def _do_skip_test(self) -> None:
        self._skip_test = True
        self._refresh_nav()

    def _next(self) -> None:
        if self._step == STEP_DONE:
            self._finish()
            return
        self._step += 1
        self._show_step()

    def _back(self) -> None:
        self._step -= 1
        self._show_step()

    def _finish(self) -> None:
        self._cfg["api"] = self._api_fields.get_values()
        if self._language.value():
            self._cfg["target_language"] = self._language.value()
        self._cfg["hotkey"] = self._hotkey.value()
        self.completed = True
        self._win.destroy()

    def _cancel(self) -> None:
        self._win.destroy()


def run_wizard(root: tk.Tk, cfg: dict) -> bool:
    """顯示首次設定精靈並等待關閉;完成回 True(結果已寫入 cfg,呼叫端負責存檔)。"""
    wizard = SetupWizard(root, cfg)
    root.wait_window(wizard._win)
    return wizard.completed
```

- [ ] **Step 3: main.py 啟動整合**

`src/main.py` 的 `main()` 開頭改為：

```python
def main() -> None:
    cfg = load_config(CONFIG_PATH)

    root = tk.Tk()
    root.withdraw()

    if not is_configured(cfg):
        from src.ui.wizard import run_wizard
        if not run_wizard(root, cfg):
            root.destroy()
            return  # 使用者取消首次設定
        save_config(CONFIG_PATH, cfg)

    translator = Translator(**cfg["api"], target_language=cfg["target_language"])
    ui_queue: queue.Queue = queue.Queue()
```

（原本的 `if not cfg["api"]["model"]: sys.exit(...)` 刪除；import 區加 `from src.config import CONFIG_PATH, is_configured, load_config, save_config`。`root = tk.Tk()` 原本在 translator 之後，移到最前如上。）

- [ ] **Step 4: 開發模式實機驗證**

把專案根目錄的 `config.json` 暫時改名（模擬新使用者），跑 `uv run run.py`：精靈應出現；走完 4 步（可用真金鑰或指到本機伺服器）→ config.json 產生、主程式續跑。中途關閉 → 程式直接退出、不產生 config。驗完把原 config 改回來。

- [ ] **Step 5: 跑測試確認通過後 commit**

```bash
uv run pytest
git add src/ui/wizard.py src/main.py tests/test_wizard.py
git commit -m "feat: 首次設定精靈(4 步)與啟動流程整合"
```

---

### Task 6: 一般設定視窗、overlay 齒輪與套用整合

**Files:**
- Create: `src/ui/settings.py`
- Modify: `src/reader/overlay.py`（齒輪按鈕、`set_limits`）
- Modify: `src/main.py`（設定視窗開啟與套用）
- Test: `tests/test_settings.py`（新檔）

**Interfaces:**
- Consumes: `ApiFields`／`HotkeyField`／`LanguageField`／`validate_api_form`（Task 4）、`Translator.reconfigure`（Task 3）。
- Produces:
  - `settings.SettingsWindow(root, cfg, on_save)`：單例視窗；儲存時把新值寫進 `cfg`（就地更新）後呼叫 `on_save(game_path_changed: bool)`。`open()` 顯示或帶到前景。
  - `settings.clamp_advanced(values: dict) -> dict`：進階數值夾限（純函式，`poll_interval` 0.1–5.0、`fade_seconds` 0–3600、`max_messages` 10–1000、`type_delay` 0.0–0.5）。
  - `OverlayWindow.__init__` 新增參數 `on_settings=None`；新方法 `set_limits(max_messages: int, fade_seconds: int)`。

- [ ] **Step 1: 寫失敗測試 tests/test_settings.py**

```python
"""設定視窗純邏輯與 overlay set_limits 測試。"""
from src.reader.overlay import OverlayWindow
from src.ui.settings import clamp_advanced


def test_clamp_advanced_limits_ranges():
    v = clamp_advanced({"poll_interval": 0.01, "fade_seconds": -5,
                        "max_messages": 99999, "type_delay": 9.0})
    assert v == {"poll_interval": 0.1, "fade_seconds": 0,
                 "max_messages": 1000, "type_delay": 0.5}


def test_clamp_advanced_passes_valid_values():
    v = {"poll_interval": 0.4, "fade_seconds": 0, "max_messages": 200, "type_delay": 0.02}
    assert clamp_advanced(dict(v)) == v


def test_overlay_set_limits_trims_messages(root):
    ov = OverlayWindow(root, x=0, y=0, max_messages=5, fade_seconds=0)
    for i in range(5):
        ov.add_message(f"o{i}", f"t{i}", now=100.0)
    ov.set_limits(max_messages=3, fade_seconds=0)
    assert len(ov.visible_messages()) == 3
    assert ov.visible_messages()[0] == ("o2", "t2")  # 移除最舊
```

- [ ] **Step 2: 跑測試確認失敗，然後改 src/reader/overlay.py**

`OverlayWindow.__init__` 簽名加 `on_settings=None`；標題列 `self._status_label.pack(...)` 之前插入齒輪（在右側、狀態字左邊）：

```python
        if on_settings is not None:
            gear = tk.Label(bar, text="⚙", bg=BAR, fg=FG_BAR,
                            font=("Microsoft JhengHei", 9), cursor="hand2")
            gear.pack(side="right", padx=(0, 2))
            gear.bind("<Button-1>", lambda e: on_settings())
```

（齒輪不綁拖曳事件，點擊才不會被拖曳吃掉。）新增方法：

```python
    def set_limits(self, max_messages: int, fade_seconds: int) -> None:
        """套用新的訊息上限與淡出秒數;超出上限的最舊訊息立即移除。"""
        self._max = max_messages
        self._fade = fade_seconds
        while len(self._messages) > self._max:
            _, _, _, old_row = self._messages.pop(0)
            old_row.destroy()
        self._refresh_placeholder()
```

- [ ] **Step 3: 實作 src/ui/settings.py**

```python
"""一般設定視窗:分「基本/進階」分頁,儲存即套用(不需重啟)。
遊戲路徑例外:重掛 hook 需重啟,儲存後提示下次啟動生效。"""
import tkinter as tk
from tkinter import filedialog, messagebox, ttk

from src.ui.fields import ApiFields, HotkeyField, LanguageField, validate_api_form

_ADVANCED_LIMITS = {
    "poll_interval": (0.1, 5.0),
    "fade_seconds": (0, 3600),
    "max_messages": (10, 1000),
    "type_delay": (0.0, 0.5),
}


def clamp_advanced(values: dict) -> dict:
    """把進階數值夾在合理範圍,避免填出爆炸值。"""
    for key, (lo, hi) in _ADVANCED_LIMITS.items():
        values[key] = min(hi, max(lo, values[key]))
    return values


class SettingsWindow:
    """設定視窗(單例):open() 顯示或帶到前景;儲存時就地更新 cfg 並呼叫 on_save。"""

    def __init__(self, root: tk.Tk, cfg: dict, on_save):
        self._root = root
        self._cfg = cfg
        self._on_save = on_save
        self._win: tk.Toplevel | None = None

    def open(self) -> None:
        if self._win is not None and self._win.winfo_exists():
            self._win.lift()
            self._win.focus_force()
            return
        cfg = self._cfg
        self._win = tk.Toplevel(self._root)
        self._win.title("Wizard101 聊天翻譯助手 — 設定")
        self._win.geometry("560x520")
        self._win.attributes("-topmost", True)

        nb = ttk.Notebook(self._win)
        nb.pack(fill="both", expand=True, padx=8, pady=8)

        # --- 基本 ---
        basic = ttk.Frame(nb, padding=12)
        nb.add(basic, text="基本")
        self._api = ApiFields(basic, cfg["api"])
        self._api.pack(fill="x")
        self._api.set_target_language_fn(lambda: self._language.value())
        ttk.Label(basic, text="翻譯目標語言").pack(anchor="w", pady=(12, 0))
        self._language = LanguageField(basic, cfg["target_language"])
        self._language.pack(fill="x", pady=(2, 8))
        ttk.Label(basic, text="呼出輸入框的熱鍵").pack(anchor="w")
        self._hotkey = HotkeyField(basic, cfg["hotkey"])
        self._hotkey.pack(anchor="w", pady=(2, 0))

        # --- 進階 ---
        adv = ttk.Frame(nb, padding=12)
        nb.add(adv, text="進階")
        self._poll = self._spin(adv, "輪詢間隔（秒）", cfg["poll_interval"],
                                0.1, 5.0, 0.1, "收訊掃描頻率，小=更即時")
        self._fade = self._spin(adv, "訊息淡出（秒）", cfg["fade_seconds"],
                                0, 3600, 10, "0＝永不淡出，可滾動看歷史")
        self._max_msgs = self._spin(adv, "訊息保留上限", cfg["max_messages"],
                                    10, 1000, 10, "超過移除最舊")
        self._type_delay = self._spin(adv, "鍵入延遲（秒）", cfg["type_delay"],
                                      0.0, 0.5, 0.01, "遊戲漏字就調大")
        path_row = ttk.Frame(adv)
        path_row.pack(fill="x", pady=(8, 0))
        ttk.Label(path_row, text="遊戲路徑", width=14).pack(side="left")
        self._game_path = tk.StringVar(value=cfg["game_path"] or "")
        ttk.Entry(path_row, textvariable=self._game_path).pack(
            side="left", fill="x", expand=True)
        ttk.Button(path_row, text="瀏覽…", width=7,
                   command=self._browse_game_path).pack(side="left", padx=(4, 0))
        ttk.Label(adv, text="留空＝自動偵測執行中的遊戲",
                  foreground="#888888").pack(anchor="w")

        btns = ttk.Frame(self._win, padding=(8, 0, 8, 8))
        btns.pack(side="bottom", fill="x")
        ttk.Button(btns, text="取消", command=self._win.destroy).pack(side="right")
        ttk.Button(btns, text="儲存", command=self._save).pack(side="right", padx=(0, 8))

    def _spin(self, parent, label, initial, lo, hi, step, hint):
        row = ttk.Frame(parent)
        row.pack(fill="x", pady=2)
        ttk.Label(row, text=label, width=14).pack(side="left")
        var = tk.DoubleVar(value=initial) if isinstance(initial, float) \
            else tk.IntVar(value=initial)
        ttk.Spinbox(row, textvariable=var, from_=lo, to=hi, increment=step,
                    width=8).pack(side="left")
        ttk.Label(row, text=hint, foreground="#888888").pack(side="left", padx=8)
        return var

    def _browse_game_path(self) -> None:
        chosen = filedialog.askdirectory(parent=self._win)
        if chosen:
            self._game_path.set(chosen)

    def _save(self) -> None:
        api = self._api.get_values()
        errors = validate_api_form(api)
        if not self._language.value():
            errors.append("請選擇或輸入翻譯目標語言")
        if errors:
            messagebox.showwarning("設定不完整", "\n".join(errors), parent=self._win)
            return
        cfg = self._cfg
        old_game_path = cfg["game_path"]
        cfg["api"] = api
        cfg["target_language"] = self._language.value()
        cfg["hotkey"] = self._hotkey.value()
        advanced = clamp_advanced({
            "poll_interval": float(self._poll.get()),
            "fade_seconds": int(self._fade.get()),
            "max_messages": int(self._max_msgs.get()),
            "type_delay": float(self._type_delay.get()),
        })
        cfg.update(advanced)
        cfg["game_path"] = self._game_path.get().strip() or None
        game_path_changed = cfg["game_path"] != old_game_path
        self._on_save(game_path_changed)
        if game_path_changed:
            messagebox.showinfo("提示", "遊戲路徑將於下次啟動生效", parent=self._win)
        self._win.destroy()
```

- [ ] **Step 4: main.py 整合**

`main()` 內（`input_box` 建立處附近）：

```python
    hotkey_handle = keyboard.add_hotkey(cfg["hotkey"], lambda: ui_queue.put(input_box.show))

    def apply_settings(game_path_changed: bool) -> None:
        nonlocal hotkey_handle
        save_config(CONFIG_PATH, cfg)
        translator.reconfigure(**cfg["api"], target_language=cfg["target_language"])
        keyboard.remove_hotkey(hotkey_handle)
        hotkey_handle = keyboard.add_hotkey(cfg["hotkey"],
                                            lambda: ui_queue.put(input_box.show))
        overlay.set_limits(cfg["max_messages"], cfg["fade_seconds"])

    from src.ui.settings import SettingsWindow
    settings = SettingsWindow(root, cfg, on_save=apply_settings)
```

`OverlayWindow(...)` 建構參數加 `on_settings=lambda: ui_queue.put(settings.open)`。注意順序：`settings` 要在 `overlay` 之前定義，或改用 `on_settings=lambda: ui_queue.put(lambda: settings.open())` 延後取值——採後者，`overlay` 建構處不必搬動。

（`keyboard.add_hotkey(...)` 原本裸呼叫的那行改為上面存 `hotkey_handle` 的版本。）

- [ ] **Step 5: 開發模式實機驗證**

`uv run run.py`：點 overlay 齒輪開設定視窗；改目標語言＋熱鍵並儲存 → 新熱鍵立即可用；把 API 金鑰改成錯的儲存 → 收訊出現「API 設定有誤」橫幅，改回正確金鑰儲存 → 橫幅在下一輪翻譯成功後消失。

- [ ] **Step 6: 跑測試確認通過後 commit**

```bash
uv run pytest
git add src/ui/settings.py src/reader/overlay.py src/main.py tests/test_settings.py
git commit -m "feat: 一般設定視窗(基本/進階)與 overlay 齒輪入口,儲存即套用"
```

---

### Task 7: frozen log、打包定稿、命名與 README

**Files:**
- Modify: `src/main.py`（frozen log 導向）
- Modify: `build.spec`（console=False 定稿）
- Modify: `pyproject.toml`（專案名）
- Modify: `README.md`
- Modify: `config.example.json`（api 區塊加 provider）

**Interfaces:**
- Consumes: `config.app_dir()`（Task 2）。
- Produces: 最終發佈產物 `dist/Wizard101ChatTranslator.exe`。

- [ ] **Step 1: frozen 模式輸出導向 log 檔**

`src/main.py` 的 `main()` 最開頭（`cfg = load_config(...)` 之前）加：

```python
    if getattr(sys, "frozen", False):
        # windowed exe 沒有 stdout/stderr(為 None);全部導到 exe 旁的 app.log,
        # 使用者回報問題時附上此檔即可(每次啟動覆寫,只留本次紀錄)
        log = open(app_dir() / "app.log", "w", encoding="utf-8", buffering=1)
        sys.stdout = sys.stderr = log
```

import 區加 `from src.config import app_dir`（併入既有的 config import 行）。

- [ ] **Step 2: build.spec 定稿**

`console=True` 改為 `console=False`，並把 spike 註解改為定稿說明。

- [ ] **Step 3: 命名全面改 Wizard101**

```bash
grep -rn --include="*.py" --include="*.md" --include="*.toml" --include="*.json" -i "wiz101" .
```

逐一處理（排除 `.venv/`、`uv.lock`、repo 資料夾路徑本身）：
- `pyproject.toml`：`name = "wizard101-chat-translator"`，改完跑 `uv sync` 更新 lock。
- README、docstring、UI 字串中的「Wiz101」→「Wizard101」（`overlay.py` 標題已是 Wizard101，確認即可）。

- [ ] **Step 4: config.example.json 同步新 schema**

`api` 區塊改為：

```json
  "api": {
    "provider": "custom",
    "base_url": "http://127.0.0.1:8000",
    "model": "your-model-name",
    "api_key": "",
    "thinking": false
  },
```

- [ ] **Step 5: README 更新**

加「一般使用者」章節（置於開發說明之前）：
- 下載 `Wizard101ChatTranslator.exe`，放到任意資料夾（設定與 log 會存在 exe 旁）
- 雙擊執行 → 首次設定精靈（選翻譯服務 → 填金鑰 → 測試連線 → 完成）
- 遊戲需已登入進世界內；overlay 齒輪（⚙）可隨時改設定
- 防毒誤判說明：本工具讀取遊戲聊天記憶體＋全域熱鍵，可能被防毒軟體標記；請自行評估並將程式加入白名單
- 遊戲若以系統管理員身分執行，本工具也需以系統管理員身分執行

開發者章節加打包指令：`uv run pyinstaller build.spec --noconfirm`，產物在 `dist/`。

- [ ] **Step 6: 最終打包與實機驗證**

```bash
uv run pyinstaller build.spec --noconfirm
```

把 `dist/` 清乾淨只留 exe（模擬新使用者），雙擊執行驗收：
1. 無黑窗；首次設定精靈出現
2. 走完精靈 → `config.json`＋`app.log` 出現在 exe 旁
3. 遊戲內收訊翻譯、熱鍵發話、齒輪改設定全部可用
4. 關閉重開 exe → 不再出精靈、直接進主流程

- [ ] **Step 7: 跑測試確認通過後 commit**

```bash
uv run pytest
git add src/main.py build.spec pyproject.toml uv.lock README.md config.example.json
git commit -m "feat: 打包定稿(windowed+log 檔)、命名統一 Wizard101、README 使用者說明"
```

---

## Self-Review 紀錄

- Spec 覆蓋：§1 架構（T4–T6）、§2 精靈（T5）、§3 設定視窗（T6）、§4 provider／錯誤分類／temperature 0／timeout 60（T3）、§5 打包／路徑／log／README（T1、T2、T7）、舊設定遷移（T2）、命名 Wizard101（T7）——無缺漏。
- 佔位掃描：`fields.py` 的 OpenAI 模型清單佔位字已在 Task 4 Step 1／Step 3 明確要求以查證結果替換，非遺留 TBD。
- 型別一致：`Translator.reconfigure` 簽名（T3 定義、T6 使用）、`run_wizard(root, cfg) -> bool`（T5 定義與使用）、`SettingsWindow.open`／`on_save(game_path_changed)`（T6）、`set_limits`（T6 定義與測試）已核對一致。
