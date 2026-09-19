# 多組翻譯服務設定 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把固定三家的 API 設定改成可新增多組的具名翻譯服務清單，附一組預設服務與收訊／發話／區域三個可各自指定的用途插槽。

**Architecture:** 新增 `src/services.py` 收攏「服務是什麼、有哪些、怎麼找」（服務商欄位表、清單操作、`resolve()`、設定檔遷移與校驗），`config.py` 回歸單純的 JSON 讀寫。UI 端把現有的 `ApiFields` 拆成可重用的單筆表單 `ServiceForm`，外面包一個「翻譯服務」分頁 `ServicePane`（預設下拉＋摺疊的用途分派＋卡片清單＋新增／編輯對話框）。執行期由 `build_translation()` 依三個插槽各建一個 `Translator`。

**Tech Stack:** Python 3.12＋uv、tkinter／ttk、pytest、ruff。

**Spec:** `docs/superpowers/specs/2026-09-19-multi-service-config-design.md`

## Global Constraints

- **語言**：回答與文件用繁體中文（台灣）。程式碼註解／docstring／UI 文字用繁體中文全形標點；**log 訊息一律英文**（含 `key=value` 診斷欄位）；commit message 與 PR 標題一律英文、conventional commits。
- **註解節制**：預設不寫實作層 `#` 註解；要寫就只寫 WHY，**預設一行**，三行是上限不是目標。模組與公開函式寫**一行摘要**的 docstring；測試 docstring 一行。
- **不重複造輪子**：動手寫工具函式前先找專案裡有沒有現成的（`form.py` 的 `hint_label`／`BackgroundButton`／`show_outcome`、`ScrollableFrame`、`ModelField`、`richtext.RichLabel` 都要沿用）。
- **翻譯語言不可寫死**：命名用方向（`incoming`／`outgoing`），不得出現 `zh`／`en`／繁體中文之類的硬編。
- **敏感資料**：API 金鑰絕不寫入 log，只記 `has_key=true/false`。
- **指令**：安裝 `uv sync`；執行 `uv run run.py`；lint `uv run ruff check src tests tools`（必須零錯誤）；測試 `uv run pytest`（必須全綠）。
- **提交前**：lint 與測試都過才 `git commit`，不得 `--no-verify`。**不要主動 `git push`。**
- **全套測試每個 task 只在收尾時跑一次**，步驟中間跑單檔或單一測試即可。
- **Commit 結尾附**：
  ```
  Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_011xvdKtdzM7NktE8gPSVKgD
  ```

---

## 檔案結構

| 檔案 | 責任 | 動作 |
|---|---|---|
| `src/services.py` | 服務商欄位表、服務清單操作、`resolve()`、遷移與校驗 | 新增 |
| `src/ui/providers.py` | （內容搬進 `src/services.py`） | 刪除 |
| `src/ui/service_form.py` | 單筆服務的編輯表單（精靈與對話框共用） | 新增 |
| `src/ui/service_list.py` | 「翻譯服務」分頁 ＋ 新增／編輯對話框 | 新增 |
| `src/ui/fields.py` | 只留熱鍵與語言欄位（`ApiFields` 移除） | 修改 |
| `src/ui/form.py` | 加 `collapsible()` | 修改 |
| `src/config.py` | JSON 讀寫與非服務欄位；服務相關全部委派 `services.py` | 修改 |
| `src/ui/settings.py` | 新增「翻譯服務」分頁，基本分頁移除 API 欄位 | 修改 |
| `src/ui/wizard.py` | 第二步改用 `ServiceForm` | 修改 |
| `src/main.py` | 三個 `Translator`、`apply_settings` 迴圈、啟動摘要 | 修改 |
| `src/translation/translator.py` | `EFFORT_AUTO` 改從 `services` 匯入 | 修改 |
| `src/ui/model_field.py` | `validate_endpoint_fields` 改從 `services` 匯入 | 修改 |
| `tests/config_helpers.py` | `configured_cfg()` 測試輔助 | 新增 |
| `tests/test_services.py` | 服務資料層測試 | 新增 |
| `tests/test_service_form.py` | `ServiceForm` 測試 | 新增 |
| `tests/test_service_list.py` | `ServicePane` 與對話框測試 | 新增 |

---

### Task 1: 測試輔助 `configured_cfg()`

先把散落在測試裡的「弄一份設定完整的 cfg」收成一支函式，**此時 schema 還沒變**，整套測試必須維持全綠。之後 Task 8 換 schema 時只要改這一支。

**Files:**
- Create: `tests/config_helpers.py`
- Modify: `tests/test_settings.py`、`tests/test_scrollable.py`、`tests/test_ui_pump.py`、`tests/test_main.py`

**Interfaces:**
- Consumes: 現有的 `src.config.DEFAULT_CONFIG`
- Produces: `configured_cfg(provider: str = "custom", **fields) -> dict` —— 回傳一份通過 `is_configured()` 的 cfg 深副本

- [ ] **Step 1: 建立輔助模組**

沿用專案既有的 `tests/chatlog_helpers.py` 模式（同層模組、直接 `from tests.config_helpers import ...`）。

```python
# tests/config_helpers.py
"""測試用的設定輔助：一行生出一份 API 設定完整的 cfg。"""
import copy

from src.config import DEFAULT_CONFIG

# 各服務商「剛好填滿必填欄位」的最小值；自訂端點不需金鑰，欄位最少，故為預設。
_MINIMAL = {
    "custom": {"base_url": "http://x", "model": "m"},
    "openai": {"api_key": "k", "model": "m"},
    "claude": {"api_key": "k", "model": "m"},
}


def configured_cfg(provider: str = "custom", **fields) -> dict:
    """一份 API 設定完整的 cfg 深副本；`fields` 覆寫該服務商的欄位。"""
    cfg = copy.deepcopy(DEFAULT_CONFIG)
    cfg["api"]["provider"] = provider
    cfg["api"][provider].update({**_MINIMAL[provider], **fields})
    return cfg
```

- [ ] **Step 2: 寫輔助本身的測試**

```python
# tests/test_services.py（本檔在 Task 3 會繼續長大，先放這一則）
"""翻譯服務資料層測試。"""
from src.config import is_configured
from tests.config_helpers import configured_cfg


def test_configured_cfg_passes_the_completeness_check():
    assert is_configured(configured_cfg())
    assert is_configured(configured_cfg("openai"))
    assert is_configured(configured_cfg("claude"))


def test_configured_cfg_returns_an_independent_copy():
    first = configured_cfg()
    first["api"]["custom"]["model"] = "mutated"
    assert configured_cfg()["api"]["custom"]["model"] == "m"
```

- [ ] **Step 3: 跑測試確認通過**

Run: `uv run pytest tests/test_services.py -v`
Expected: 2 passed

- [ ] **Step 4: 掃出所有呼叫點**

Run: `uv run python -c "import re,pathlib; [print(p, i+1, l.rstrip()) for p in pathlib.Path('tests').glob('*.py') for i,l in enumerate(p.read_text(encoding='utf-8').splitlines()) if 'cfg[\"api\"]' in l]"`

Expected: 列出 `tests/test_settings.py`、`tests/test_scrollable.py`、`tests/test_ui_pump.py`、`tests/test_main.py`、`tests/test_config.py`、`tests/test_fields.py` 的命中行。

- [ ] **Step 5: 換手**

把下列形狀（出現約 20 處，多在 `tests/test_settings.py`）：

```python
cfg = copy.deepcopy(DEFAULT_CONFIG)
cfg["api"]["provider"] = "custom"
cfg["api"]["custom"].update(base_url="http://x", model="m")
```

一律換成：

```python
cfg = configured_cfg()
```

並在檔頭加 `from tests.config_helpers import configured_cfg`，移除因此不再需要的 `copy` 與 `DEFAULT_CONFIG` 匯入。

**不要動這兩個檔**：`tests/test_config.py`（測的就是 config 本身的結構，要直接操作原始 dict）與 `tests/test_fields.py`（有自己的 `_initial()`，Task 8 會整批搬走）。

- [ ] **Step 6: 跑全套確認沒改壞**

Run: `uv run pytest`
Expected: 全綠、測試則數與換手前相同

- [ ] **Step 7: Lint 並 commit**

```bash
uv run ruff check src tests tools
git add tests/config_helpers.py tests/test_services.py tests/test_settings.py tests/test_scrollable.py tests/test_ui_pump.py tests/test_main.py
git commit -m "test: add configured_cfg helper and use it across the suite"
```

---

### Task 2: `src/services.py` —— 服務商資料搬家

純搬家，行為不變。把 `src/ui/providers.py` 整個移進新模組，順便把服務商欄位表與 effort 常數從 `config.py` 搬過來，並把 `validate_api_form` 更名為 `validate_service`。

**Files:**
- Create: `src/services.py`
- Delete: `src/ui/providers.py`
- Modify: `src/config.py`、`src/translation/translator.py:19`、`src/ui/fields.py:10`、`src/ui/model_field.py:12`、`src/ui/settings.py:26`、`src/ui/wizard.py:13`
- Test: `tests/test_services.py`、`tests/test_fields.py`

**Interfaces:**
- Produces:
  - `API_PROFILE_FIELDS: dict[str, dict]`、`API_PROVIDERS: tuple[str, ...]`
  - `EFFORT_AUTO = "auto"`、`EFFORT_LOW = "low"`、`API_EFFORTS = (EFFORT_AUTO, EFFORT_LOW)`
  - `needs_base_url(provider: str) -> bool`
  - `Provider`（`key`、`label_key`、`key_url`、`brand`；`has_field(name)`、`needs_base_url`、`short_name`）、`PROVIDERS: dict[str, Provider]`
  - `validate_endpoint_fields(api: dict) -> list[str]`
  - `validate_service(api: dict) -> list[str]`（原 `validate_api_form`）

- [ ] **Step 1: 寫失敗的測試**

```python
# tests/test_services.py（接在 Task 1 的兩則之後）
from src import i18n
from src.services import (
    API_PROFILE_FIELDS,
    API_PROVIDERS,
    PROVIDERS,
    needs_base_url,
    validate_endpoint_fields,
    validate_service,
)


def test_every_provider_has_ui_metadata():
    assert set(PROVIDERS) == set(API_PROVIDERS)
    assert needs_base_url("custom")
    assert not needs_base_url("openai")


def test_short_names_are_brands_except_the_custom_endpoint():
    i18n.set_language("en-US")
    assert PROVIDERS["openai"].short_name == "ChatGPT"
    assert PROVIDERS["claude"].short_name == "Claude"
    # 自訂端點沒有品牌名，短名要走語言檔（此處只確認它被翻過，不釘字面）
    assert PROVIDERS["custom"].short_name != "provider.custom_short"


def test_validate_service_reports_every_missing_required_field():
    errors = validate_service({"provider": "openai", "model": "", "api_key": ""})
    assert set(errors) == {"error.need_model", "error.need_api_key"}


def test_validate_endpoint_fields_ignores_the_model():
    assert validate_endpoint_fields(
        {"provider": "custom", "base_url": "http://x", "model": ""}) == []


def test_profile_fields_cover_what_the_translator_takes():
    assert set(API_PROFILE_FIELDS["claude"]) == {"model", "api_key", "effort"}
    assert "base_url" not in API_PROFILE_FIELDS["openai"]
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `uv run pytest tests/test_services.py -v`
Expected: FAIL，`ModuleNotFoundError: No module named 'src.services'`

- [ ] **Step 3: 建立 `src/services.py`**

```python
"""翻譯服務：服務商的欄位表與顯示資料，以及表單必填欄位的判定。

「服務」是使用者建立的一筆具名設定 —— 某家服務商加上金鑰、模型等欄位。
"""
from dataclasses import dataclass

from src.i18n import t

# 每家服務商有哪些欄位與預設值：官方端點的網址寫死在 translator，Claude 不吃 thinking
# 開關而是 effort。這張表是服務商清單與欄位的單一真實來源。
API_PROFILE_FIELDS: dict[str, dict] = {
    "openai": {"model": "", "api_key": "", "thinking": False},
    "claude": {"model": "", "api_key": "", "effort": "auto"},
    "custom": {"base_url": "", "model": "", "api_key": "", "thinking": False},
}

API_PROVIDERS = tuple(API_PROFILE_FIELDS)

# Claude 的思考深度：auto＝不帶參數、由模型自行決定；low＝壓到最低。
# Claude 沒有「完全不思考」這個選項，故意不與另兩家的 thinking 開關共用欄位名。
EFFORT_AUTO = "auto"
EFFORT_LOW = "low"
API_EFFORTS = (EFFORT_AUTO, EFFORT_LOW)


def needs_base_url(provider: str) -> bool:
    """這家服務商要不要自己填端點網址（官方端點的網址寫死在 translator）。"""
    return "base_url" in API_PROFILE_FIELDS[provider]


@dataclass(frozen=True)
class Provider:
    """服務商的顯示資料。該畫哪些欄位一律問 API_PROFILE_FIELDS（has_field）。"""
    key: str
    label_key: str
    key_url: str | None = None
    brand: str | None = None

    def has_field(self, name: str) -> bool:
        return name in API_PROFILE_FIELDS[self.key]

    @property
    def needs_base_url(self) -> bool:
        return needs_base_url(self.key)

    @property
    def short_name(self) -> str:
        """新建一筆服務時的預設名稱；品牌名不進語言檔，只有自訂端點要翻譯。"""
        return self.brand or t(f"provider.{self.key}_short")


PROVIDERS: dict[str, Provider] = {p.key: p for p in (
    Provider(key="openai", label_key="provider.openai", brand="ChatGPT",
             key_url="https://platform.openai.com/api-keys"),
    Provider(key="claude", label_key="provider.claude", brand="Claude",
             key_url="https://console.anthropic.com/settings/keys"),
    Provider(key="custom", label_key="provider.custom"),
)}


def validate_endpoint_fields(api: dict) -> list[str]:
    """檢查連上端點所需的欄位（不含模型），回傳錯誤文案 key 列表（空＝通過）。
    取模型清單時模型欄本來就還沒填，故與 validate_service 分開。"""
    errors = []
    provider = PROVIDERS[api["provider"]]
    if not provider.needs_base_url and not api["api_key"].strip():
        errors.append("error.need_api_key")
    if provider.needs_base_url and not api["base_url"].strip():
        errors.append("error.need_base_url")
    return errors


def validate_service(api: dict) -> list[str]:
    """檢查一筆服務的必填欄位，回傳錯誤文案 key 列表（空＝通過）。"""
    errors = [] if api["model"].strip() else ["error.need_model"]
    return errors + validate_endpoint_fields(api)
```

- [ ] **Step 4: 加語言檔的短名 key**

在 `src/i18n/zh-TW.json`、`src/i18n/zh-CN.json`、`src/i18n/en-US.json` 的 `provider.custom` 之後各加一行：

- `zh-TW.json`：`"provider.custom_short": "自訂端點",`
- `zh-CN.json`：`"provider.custom_short": "自定义端点",`
- `en-US.json`：`"provider.custom_short": "Custom endpoint",`

- [ ] **Step 5: 刪掉舊出處並改匯入**

1. 刪除 `src/ui/providers.py`。
2. `src/config.py`：刪除 `API_PROFILE_FIELDS`、`API_PROVIDERS`、`EFFORT_AUTO`／`EFFORT_LOW`／`API_EFFORTS`、`needs_base_url` 的定義，改成 `from src.services import API_PROFILE_FIELDS, API_PROVIDERS, needs_base_url`（`_default_api`、`_prune_profiles`、`is_configured` 仍要用）。
3. `src/translation/translator.py:19`：`from src.config import EFFORT_AUTO` → `from src.services import EFFORT_AUTO`。
4. `src/ui/fields.py:10`：`from src.config import API_EFFORTS, API_PROFILE_FIELDS, API_PROVIDERS, EFFORT_AUTO` → `from src.services import API_EFFORTS, API_PROFILE_FIELDS, API_PROVIDERS, EFFORT_AUTO`；同檔的 `from src.ui.providers import PROVIDERS, validate_api_form` → `from src.services import PROVIDERS, validate_service`，內文 `validate_api_form(` → `validate_service(`。
5. `src/ui/model_field.py`：`from src.ui.providers import validate_endpoint_fields` → `from src.services import validate_endpoint_fields`。
6. `src/ui/settings.py`：`from src.ui.providers import validate_api_form` → `from src.services import validate_service`，內文同步更名。
7. `src/ui/wizard.py`：同上。
8. `tests/test_fields.py`：`from src.ui.providers import PROVIDERS, validate_api_form, validate_endpoint_fields` → `from src.services import PROVIDERS, validate_service, validate_endpoint_fields`，內文同步更名；`from src.config import API_PROVIDERS, ...` 的 `API_PROVIDERS` 改從 `src.services` 匯入；`from src.config import EFFORT_LOW` → `from src.services import EFFORT_LOW`。
9. `tests/test_translator.py:269`：`from src.config import EFFORT_LOW` → `from src.services import EFFORT_LOW`。
10. `tests/test_config.py`：`API_PROFILE_FIELDS`、`API_PROVIDERS` 改從 `src.services` 匯入。

避免遺漏，用這條指令自我檢查（應為空輸出）：

Run: `uv run python -c "import subprocess,sys; sys.exit(subprocess.run(['git','grep','-n','-e','ui.providers','-e','validate_api_form'],capture_output=True,text=True).stdout.strip() != '')"`

- [ ] **Step 6: 跑測試確認通過**

Run: `uv run pytest tests/test_services.py tests/test_fields.py tests/test_config.py tests/test_translator.py -v`
Expected: 全部 PASS

- [ ] **Step 7: Lint 並 commit**

```bash
uv run ruff check src tests tools
git add -A
git commit -m "refactor(services): move provider metadata out of ui into src/services.py"
```

---

### Task 3: 服務清單的資料操作

**Files:**
- Modify: `src/services.py`
- Test: `tests/test_services.py`

**Interfaces:**
- Consumes: Task 2 的 `PROVIDERS`、`API_PROFILE_FIELDS`、`API_PROVIDERS`
- Produces:
  - `SLOT_INCOMING = "incoming"`、`SLOT_OUTGOING = "outgoing"`、`SLOT_REGION = "region"`、`SLOTS = (SLOT_INCOMING, SLOT_OUTGOING, SLOT_REGION)`
  - `new_id(services: list[dict]) -> str`
  - `unique_name(name: str, services: list[dict], ignore_id: str | None = None) -> str`
  - `new_service(provider: str, services: list[dict]) -> dict`
  - `find(cfg: dict, service_id: str | None) -> dict | None`
  - `resolve(cfg: dict, slot: str) -> dict`（攤平副本，含 `provider`，不含 `id`／`name`）
  - `describe(service: dict) -> str`（卡片副標，`短名 · 模型`）

- [ ] **Step 1: 寫失敗的測試**

```python
# tests/test_services.py（續）
from src.services import (
    SLOT_INCOMING,
    SLOT_REGION,
    SLOTS,
    describe,
    find,
    new_service,
    resolve,
    unique_name,
)


def _section(*services, default=None, **slots):
    """一份只含服務三鍵的 cfg 片段（resolve 只讀這三個鍵）。"""
    listed = list(services)
    return {"services": listed,
            "default_service": default or (listed[0]["id"] if listed else None),
            "service_slots": {slot: slots.get(slot) for slot in SLOTS}}


def test_new_service_fills_the_provider_defaults_and_a_name():
    service = new_service("claude", [])
    assert service["provider"] == "claude"
    assert service["name"] == "Claude"
    assert service["model"] == "" and service["effort"] == "auto"
    assert len(service["id"]) == 8


def test_new_services_never_share_an_id():
    services = []
    for _ in range(50):
        services.append(new_service("openai", services))
    assert len({s["id"] for s in services}) == 50


def test_unique_name_numbers_collisions_from_two():
    services = [{"id": "a", "name": "ChatGPT"}]
    assert unique_name("ChatGPT", services) == "ChatGPT (2)"
    services.append({"id": "b", "name": "ChatGPT (2)"})
    assert unique_name("ChatGPT", services) == "ChatGPT (3)"
    assert unique_name("Claude", services) == "Claude"


def test_unique_name_does_not_collide_with_the_entry_being_edited():
    services = [{"id": "a", "name": "ChatGPT"}]
    assert unique_name("ChatGPT", services, ignore_id="a") == "ChatGPT"


def test_resolve_follows_the_default_when_a_slot_is_unset():
    a = new_service("openai", [])
    a.update(model="gpt-x", api_key="k1")
    cfg = _section(a)
    assert resolve(cfg, SLOT_INCOMING)["model"] == "gpt-x"


def test_resolve_prefers_the_slots_own_service():
    a = new_service("openai", [])
    a.update(model="gpt-x")
    b = new_service("claude", [a])
    b.update(model="claude-x")
    cfg = _section(a, b, region=b["id"])
    assert resolve(cfg, SLOT_REGION)["model"] == "claude-x"
    assert resolve(cfg, SLOT_INCOMING)["model"] == "gpt-x"


def test_resolve_drops_the_bookkeeping_fields():
    a = new_service("openai", [])
    resolved = resolve(_section(a), SLOT_INCOMING)
    assert "id" not in resolved and "name" not in resolved
    assert set(resolved) == {"provider", "model", "api_key", "thinking"}


def test_resolve_returns_a_copy():
    a = new_service("openai", [])
    a["model"] = "gpt-x"
    cfg = _section(a)
    resolve(cfg, SLOT_INCOMING)["model"] = "mutated"
    assert cfg["services"][0]["model"] == "gpt-x"


def test_find_returns_none_for_an_unknown_id():
    assert find(_section(new_service("openai", [])), "nope") is None
    assert find(_section(), None) is None


def test_describe_shows_the_brand_and_the_model():
    service = new_service("claude", [])
    service["model"] = "claude-opus-5"
    assert describe(service) == "Claude · claude-opus-5"
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `uv run pytest tests/test_services.py -v`
Expected: FAIL，`ImportError: cannot import name 'SLOT_INCOMING'`

- [ ] **Step 3: 實作**

加到 `src/services.py`（檔頭補 `import copy`、`import uuid`）：

```python
# 用途插槽：三個用途可各自指定服務，None＝跟隨預設。命名用方向而非語言。
SLOT_INCOMING = "incoming"
SLOT_OUTGOING = "outgoing"
SLOT_REGION = "region"
SLOTS = (SLOT_INCOMING, SLOT_OUTGOING, SLOT_REGION)

_ID_LENGTH = 8


def new_id(services: list[dict]) -> str:
    """產生一個不與現有服務相撞的 id。"""
    taken = {s.get("id") for s in services}
    while True:
        candidate = uuid.uuid4().hex[:_ID_LENGTH]
        if candidate not in taken:
            return candidate


def unique_name(name: str, services: list[dict], ignore_id: str | None = None) -> str:
    """撞名就補序號：第一筆無後綴，之後 `ChatGPT (2)`、`ChatGPT (3)`。
    括號一律半形 —— 它是識別用的序號，不隨介面語言換形。"""
    taken = {s["name"] for s in services if s.get("id") != ignore_id}
    if name not in taken:
        return name
    number = 2
    while f"{name} ({number})" in taken:
        number += 1
    return f"{name} ({number})"


def new_service(provider: str, services: list[dict]) -> dict:
    """新的一筆服務：新 id、依服務商短名自動命名、欄位填該家的預設值。"""
    return {"id": new_id(services),
            "name": unique_name(PROVIDERS[provider].short_name, services),
            "provider": provider,
            **copy.deepcopy(API_PROFILE_FIELDS[provider])}


def find(cfg: dict, service_id: str | None) -> dict | None:
    """依 id 取服務本體（不是副本）；找不到回 None。"""
    if service_id is None:
        return None
    return next((s for s in cfg["services"] if s["id"] == service_id), None)


def resolve(cfg: dict, slot: str) -> dict:
    """該用途實際生效的服務，攤平成 Translator 吃的形狀（含 provider，不含 id／name）。
    插槽未指定就跟隨預設；連預設都沒有（精靈尚未完成）時回一份空白設定。"""
    service = find(cfg, cfg["service_slots"].get(slot)) or find(cfg, cfg["default_service"])
    if service is None:
        fallback = API_PROVIDERS[0]
        return {"provider": fallback, **copy.deepcopy(API_PROFILE_FIELDS[fallback])}
    return {key: value for key, value in copy.deepcopy(service).items()
            if key not in ("id", "name")}


def describe(service: dict) -> str:
    """服務卡片的副標：服務商短名與模型。"""
    return f"{PROVIDERS[service['provider']].short_name} · {service['model']}"
```

- [ ] **Step 4: 跑測試確認通過**

Run: `uv run pytest tests/test_services.py -v`
Expected: 全部 PASS

- [ ] **Step 5: Lint 並 commit**

```bash
uv run ruff check src tests tools
git add src/services.py tests/test_services.py
git commit -m "feat(services): add service list primitives and slot resolution"
```

---

### Task 4: `normalize()` —— 遷移與校驗

純函式，還不接進 `config.py`（那是 Task 8）。

**Files:**
- Modify: `src/services.py`
- Test: `tests/test_services.py`

**Interfaces:**
- Consumes: Task 3 的 `new_service`、`new_id`、`unique_name`、`SLOTS`
- Produces: `normalize(cfg: dict) -> bool`（就地整理 `cfg` 的 `services`／`default_service`／`service_slots`，回傳是否有變動）

- [ ] **Step 1: 寫失敗的測試**

```python
# tests/test_services.py（續）
from src.services import normalize


def _empty_section():
    return {"services": [], "default_service": None,
            "service_slots": {slot: None for slot in SLOTS}}


def test_migrates_the_flat_legacy_api_block():
    cfg = _empty_section()
    cfg["api"] = {"provider": "custom", "base_url": "http://127.0.0.1:8000",
                  "model": "gemma", "api_key": "sk-1"}
    assert normalize(cfg) is True
    assert "api" not in cfg
    assert len(cfg["services"]) == 1
    service = cfg["services"][0]
    assert service["provider"] == "custom"
    assert service["base_url"] == "http://127.0.0.1:8000"
    assert service["model"] == "gemma"
    assert cfg["default_service"] == service["id"]


def test_migrates_the_per_provider_api_block_keeping_every_filled_provider():
    cfg = _empty_section()
    cfg["api"] = {"provider": "claude",
                  "openai": {"model": "gpt-x", "api_key": "sk-1", "thinking": False},
                  "claude": {"model": "claude-x", "api_key": "sk-ant", "effort": "auto"},
                  "custom": {"base_url": "", "model": "", "api_key": "",
                             "thinking": False}}
    assert normalize(cfg) is True
    assert [s["provider"] for s in cfg["services"]] == ["openai", "claude"]
    assert [s["name"] for s in cfg["services"]] == ["ChatGPT", "Claude"]
    # 原本選中的那家成為預設，沒填過的自訂端點不留空殼
    assert find(cfg, cfg["default_service"])["provider"] == "claude"


def test_migration_of_an_untouched_config_leaves_an_empty_list():
    cfg = _empty_section()
    cfg["api"] = {"provider": "openai",
                  "openai": {"model": "", "api_key": "", "thinking": False},
                  "claude": {"model": "", "api_key": "", "effort": "auto"},
                  "custom": {"base_url": "", "model": "", "api_key": "",
                             "thinking": False}}
    normalize(cfg)
    assert cfg["services"] == []
    assert cfg["default_service"] is None


def test_drops_a_service_with_an_unknown_provider():
    cfg = _empty_section()
    cfg["services"] = [{"id": "aaaaaaaa", "name": "X", "provider": "gemini",
                        "model": "g"}]
    assert normalize(cfg) is True
    assert cfg["services"] == []


def test_fills_missing_fields_and_drops_stale_ones():
    cfg = _empty_section()
    cfg["services"] = [{"id": "aaaaaaaa", "name": "C", "provider": "claude",
                        "model": "claude-x", "thinking": True}]
    assert normalize(cfg) is True
    assert cfg["services"][0] == {"id": "aaaaaaaa", "name": "C", "provider": "claude",
                                  "model": "claude-x", "api_key": "", "effort": "auto"}


def test_regenerates_duplicate_ids():
    cfg = _empty_section()
    cfg["services"] = [
        {"id": "dup", "name": "A", "provider": "openai", "model": "m", "api_key": "k",
         "thinking": False},
        {"id": "dup", "name": "B", "provider": "openai", "model": "m", "api_key": "k",
         "thinking": False}]
    assert normalize(cfg) is True
    assert cfg["services"][0]["id"] != cfg["services"][1]["id"]


def test_default_service_falls_back_to_the_first_entry():
    cfg = _empty_section()
    cfg["services"] = [new_service("openai", [])]
    cfg["default_service"] = "nope"
    assert normalize(cfg) is True
    assert cfg["default_service"] == cfg["services"][0]["id"]


def test_a_slot_pointing_at_nothing_falls_back_to_the_default():
    cfg = _empty_section()
    cfg["services"] = [new_service("openai", [])]
    cfg["default_service"] = cfg["services"][0]["id"]
    cfg["service_slots"] = {SLOT_INCOMING: "nope", SLOT_OUTGOING: None,
                            SLOT_REGION: None, "bogus": "x"}
    assert normalize(cfg) is True
    assert cfg["service_slots"] == {slot: None for slot in SLOTS}


def test_a_tidy_config_reports_no_change():
    cfg = _empty_section()
    cfg["services"] = [new_service("openai", [])]
    cfg["services"][0].update(model="m", api_key="k")
    cfg["default_service"] = cfg["services"][0]["id"]
    normalize(cfg)
    assert normalize(cfg) is False
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `uv run pytest tests/test_services.py -v`
Expected: FAIL，`ImportError: cannot import name 'normalize'`

- [ ] **Step 3: 實作**

加到 `src/services.py`（檔頭補 `from src.log import log`）：

```python
_LEGACY_FLAT_FIELDS = {key for fields in API_PROFILE_FIELDS.values() for key in fields}
# 遷移時判定「這家使用者填過東西」的欄位；三個都空就不留空殼。
_FILLED_MARKERS = ("model", "api_key", "base_url")


def _unflatten_legacy(api: dict) -> dict:
    """最舊的 api 區塊是扁平的：設定欄位與 provider 並排。整組搬進所屬服務商的子區塊，
    不屬於那家的欄位丟掉；連 provider 都沒有的一律視為自訂端點。"""
    provider = api.get("provider", "custom")
    fields = API_PROFILE_FIELDS.get(provider, {})
    profile = {key: value for key, value in api.items() if key in fields}
    log(f"[config] unflattened the legacy api block into provider={provider} "
        f"(fields={sorted(profile)})")
    return {"provider": provider, provider: profile}


def _migrate_api_block(cfg: dict) -> bool:
    """把舊的 api 區塊（扁平或 per-provider）換成服務清單，並移除該區塊。"""
    api = cfg.pop("api", None)
    if not isinstance(api, dict):
        return False
    if any(key in api for key in _LEGACY_FLAT_FIELDS):
        api = _unflatten_legacy(api)
    services: list[dict] = []
    default_id = None
    for provider in API_PROVIDERS:
        profile = api.get(provider)
        if not isinstance(profile, dict):
            continue
        if not any(str(profile.get(key, "")).strip() for key in _FILLED_MARKERS):
            continue
        service = new_service(provider, services)
        service.update({key: value for key, value in profile.items()
                        if key in API_PROFILE_FIELDS[provider]})
        services.append(service)
        if api.get("provider") == provider:
            default_id = service["id"]
    cfg["services"] = services
    cfg["default_service"] = default_id or (services[0]["id"] if services else None)
    log(f"[config] migrated the api block into {len(services)} service(s); "
        f"default={cfg['default_service']}")
    return True


def _sanitize_services(cfg: dict) -> bool:
    """逐筆補齊欄位、刪掉過期欄位、補上缺漏或重複的 id 與名稱。"""
    before = cfg.get("services")
    clean: list[dict] = []
    for entry in before if isinstance(before, list) else []:
        provider = entry.get("provider") if isinstance(entry, dict) else None
        if provider not in API_PROFILE_FIELDS:
            log(f"[config] dropped a service with unknown provider {provider!r}")
            continue
        service = {"id": str(entry.get("id") or ""),
                   "name": str(entry.get("name") or ""),
                   "provider": provider,
                   **{key: entry.get(key, default)
                      for key, default in API_PROFILE_FIELDS[provider].items()}}
        stale = sorted(key for key in entry if key not in service)
        if stale:
            log(f"[config] dropped stale fields on a {provider} service: "
                f"{', '.join(stale)}")
        if not service["id"] or any(service["id"] == s["id"] for s in clean):
            service["id"] = new_id(clean)
            log(f"[config] regenerated a missing or duplicate service id "
                f"-> {service['id']}")
        if not service["name"]:
            service["name"] = unique_name(PROVIDERS[provider].short_name, clean)
        clean.append(service)
    cfg["services"] = clean
    return clean != before


def _sanitize_pointers(cfg: dict) -> bool:
    """讓 default_service 與三個插槽只指向存在的服務。"""
    ids = {s["id"] for s in cfg["services"]}
    changed = False
    default = cfg.get("default_service")
    wanted = default if default in ids else (
        cfg["services"][0]["id"] if cfg["services"] else None)
    if wanted != default:
        log(f"[config] default_service {default!r} is unknown; using {wanted!r}")
        cfg["default_service"] = wanted
        changed = True
    slots = cfg.get("service_slots")
    slots = slots if isinstance(slots, dict) else {}
    clean = {}
    for slot in SLOTS:
        value = slots.get(slot)
        if value is not None and value not in ids:
            log(f"[config] service_slots.{slot} points at unknown service {value!r}; "
                f"following the default instead")
            value = None
        clean[slot] = value
    if clean != slots:
        changed = True
    cfg["service_slots"] = clean
    return changed


def normalize(cfg: dict) -> bool:
    """就地遷移舊格式、補齊欄位、清掉指不到的參照；回傳是否有變動。
    有變動代表呼叫端該把整理後的結果寫回 config.json。"""
    changed = _migrate_api_block(cfg)
    changed = _sanitize_services(cfg) or changed
    return _sanitize_pointers(cfg) or changed
```

- [ ] **Step 4: 跑測試確認通過**

Run: `uv run pytest tests/test_services.py -v`
Expected: 全部 PASS

- [ ] **Step 5: Lint 並 commit**

```bash
uv run ruff check src tests tools
git add src/services.py tests/test_services.py
git commit -m "feat(services): migrate and sanitise the service list on load"
```

---

### Task 5: `ServiceForm` —— 單筆服務的編輯表單

以現有的 `ApiFields` 為底改寫成吃「一筆服務」。這個 task 只新增檔案與測試，**還不接線**（`ApiFields` 仍在原處供現有 UI 使用），suite 維持全綠。

**Files:**
- Create: `src/ui/service_form.py`
- Test: `tests/test_service_form.py`

**Interfaces:**
- Consumes: Task 3 的 `new_service`、`PROVIDERS`、`API_PROFILE_FIELDS`、`API_EFFORTS`、`EFFORT_AUTO`、`validate_service`；既有的 `ModelField`、`form.BackgroundButton`／`hint_label`／`link_label`／`show_outcome`／`friendly_error`／`LABEL_WIDTH`／`ERROR_COLOR`
- Produces:
  - `ServiceForm(parent, service: dict, on_change=None)`
  - `.values() -> dict`（完整的一筆服務：`id`／`name`／`provider`＋該家欄位）
  - `.api_values() -> dict`（攤平、去掉 `id`／`name`，給測試連線與模型清單用）
  - `.test_passed: bool`
  - `.set_target_language_fn(fn)`
  - `.clear_test_result()`

- [ ] **Step 1: 寫失敗的測試**

```python
# tests/test_service_form.py
"""ServiceForm：單筆服務編輯表單的行為。"""
import pytest

from src.services import EFFORT_LOW, new_service
from src.ui.service_form import ServiceForm


@pytest.fixture
def blank(root):
    return ServiceForm(root, new_service("openai", []))


def test_new_form_starts_from_the_providers_defaults(blank):
    values = blank.values()
    assert values["provider"] == "openai"
    assert values["name"] == "ChatGPT"
    assert values["model"] == ""


def test_editing_keeps_the_id(root):
    service = new_service("claude", [])
    service.update(name="我的 Claude", model="claude-x", api_key="sk-ant")
    form = ServiceForm(root, service)
    assert form.values()["id"] == service["id"]
    assert form.values()["name"] == "我的 Claude"
    assert form.values()["model"] == "claude-x"


def test_values_only_carry_the_fields_that_provider_has(root):
    form = ServiceForm(root, new_service("claude", []))
    assert set(form.values()) == {"id", "name", "provider", "model", "api_key", "effort"}


def test_api_values_drop_the_bookkeeping_fields(blank):
    assert "id" not in blank.api_values() and "name" not in blank.api_values()


def test_switching_provider_rewrites_an_untouched_name(blank):
    blank.set_provider("claude")
    assert blank.values()["name"] == "Claude"


def test_switching_provider_keeps_a_name_the_user_typed(blank):
    blank.set_name("戰鬥用")
    blank.set_provider("claude")
    assert blank.values()["name"] == "戰鬥用"


def test_a_blank_name_falls_back_to_the_provider_short_name(blank):
    blank.set_name("   ")
    assert blank.values()["name"] == "ChatGPT"


def test_switching_provider_does_not_carry_values_across(root):
    service = new_service("openai", [])
    service.update(model="gpt-x", api_key="sk-1")
    form = ServiceForm(root, service)
    form.set_provider("claude")
    assert form.values()["model"] == ""
    assert form.values()["api_key"] == ""


def test_claude_keeps_its_effort_choice(root):
    service = new_service("claude", [])
    service["effort"] = EFFORT_LOW
    form = ServiceForm(root, service)
    assert form.values()["effort"] == EFFORT_LOW
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `uv run pytest tests/test_service_form.py -v`
Expected: FAIL，`ModuleNotFoundError: No module named 'src.ui.service_form'`

- [ ] **Step 3: 實作**

`src/ui/service_form.py` —— 由 `src/ui/fields.py` 的 `ApiFields` 改寫。搬過來時**刪掉** `_profiles`／`_switch_profile`／`_last_provider` 那套跨服務商保留值的機制（一筆只屬於一家），服務商 radio 改 readonly Combobox，最上面多一個名稱欄。

```python
"""單筆翻譯服務的編輯表單：名稱、服務商、該家的欄位、測試連線。
首次精靈（內嵌）與設定視窗的新增／編輯對話框共用同一份。"""
import copy
import tkinter as tk
from tkinter import ttk

from src.i18n import current_language, language_name, t
from src.log import log
from src.services import (
    API_EFFORTS,
    API_PROFILE_FIELDS,
    API_PROVIDERS,
    EFFORT_AUTO,
    PROVIDERS,
    validate_service,
)
from src.translation.translator import test_translate
from src.ui.form import (
    ERROR_COLOR,
    LABEL_WIDTH,
    BackgroundButton,
    friendly_error,
    hint_label,
    link_label,
    show_outcome,
)
from src.ui.model_field import ModelField
from src.ui.richtext import RichLabel, ttk_background


class ServiceForm(ttk.Frame):
    """一筆服務的編輯表單；`values()` 回傳可直接存進清單的服務 dict。"""

    def __init__(self, parent, service: dict, on_change=None):
        super().__init__(parent)
        self._on_change = on_change
        self.test_passed = False
        self._id = service["id"]
        self._provider_keys = list(API_PROVIDERS)
        self._provider = service["provider"]
        self._name = tk.StringVar(value=service.get("name", ""))
        self._api_key = tk.StringVar(value=service.get("api_key", ""))
        self._model = tk.StringVar(value=service.get("model", ""))
        self._base_url = tk.StringVar(value=service.get("base_url", ""))
        self._thinking = tk.BooleanVar(value=service.get("thinking", False))
        self._effort = tk.StringVar(value=service.get("effort", EFFORT_AUTO))
        for var in (self._api_key, self._model, self._base_url, self._effort):
            var.trace_add("write", lambda *_: self._invalidate_test())

        self._name_row()
        self._provider_row()
        self._fields = ttk.Frame(self)
        self._fields.pack(fill="x")
        self._test_row()
        # 精靈階段目標語言還沒選：退到介面語言的自稱（與 main.bootstrap_language 同一套預設）
        self._target_language_fn = lambda: language_name(current_language())
        self._rebuild_fields()

    # --- 值存取 ---
    def values(self) -> dict:
        """完整的一筆服務：id、名稱、服務商與該家的欄位。"""
        return {"id": self._id, "name": self._resolved_name(),
                "provider": self._provider, **self._field_values()}

    def api_values(self) -> dict:
        """攤平成 Translator／測試連線吃的形狀（去掉 id 與名稱）。"""
        return {"provider": self._provider, **self._field_values()}

    def set_name(self, name: str) -> None:
        self._name.set(name)

    def set_provider(self, provider: str) -> None:
        """換服務商（測試與下拉共用同一條路）。"""
        self._provider_shown.set(t(PROVIDERS[provider].label_key))
        self._on_provider_selected()

    def set_target_language_fn(self, fn) -> None:
        """測試連線時取得目標語言的 callback（精靈階段語言還沒選，用預設）。"""
        self._target_language_fn = fn

    def clear_test_result(self) -> None:
        """作廢已顯示的測試結果：那句譯文是用當時的目標語言翻的，語言一改就不算數。"""
        self._invalidate_test()
        self._test_result.set("")
        if self._on_change:
            self._on_change()

    def _resolved_name(self) -> str:
        return self._name.get().strip() or PROVIDERS[self._provider].short_name

    def _field_values(self) -> dict:
        values = {"api_key": self._api_key.get().strip(),
                  "model": self._model.get().strip(),
                  "base_url": self._base_url.get().strip(),
                  "thinking": self._thinking.get(),
                  "effort": self._effort.get()}
        return {key: values[key] for key in API_PROFILE_FIELDS[self._provider]}

    # --- 版面 ---
    def _name_row(self) -> None:
        row = ttk.Frame(self)
        row.pack(fill="x", pady=(0, 6))
        ttk.Label(row, text=t("field.service_name"), width=LABEL_WIDTH).pack(side="left")
        ttk.Entry(row, textvariable=self._name).pack(side="left", fill="x", expand=True)

    def _provider_row(self) -> None:
        row = ttk.Frame(self)
        row.pack(fill="x", pady=(0, 6))
        ttk.Label(row, text=t("field.provider"), width=LABEL_WIDTH).pack(side="left")
        self._provider_names = [t(PROVIDERS[key].label_key) for key in self._provider_keys]
        # 顯示用的變數要留在 self 上：只被 Combobox 參照的話會被 GC，欄位就空掉。
        self._provider_shown = tk.StringVar(
            value=t(PROVIDERS[self._provider].label_key))
        combo = ttk.Combobox(row, textvariable=self._provider_shown,
                             values=self._provider_names, state="readonly")
        combo.pack(side="left", fill="x", expand=True)
        combo.bind("<<ComboboxSelected>>", lambda e: self._on_provider_selected())

    def _test_row(self) -> None:
        row = ttk.Frame(self)
        row.pack(fill="x", pady=(8, 0))
        self._test_btn = ttk.Button(row, text=t("button.test"), command=self._start_test)
        self._test_btn.pack(side="left")
        # 測試還在跑時改了欄位，舊結果回來時要丟掉，不能把新設定標成「已測過」
        self._test_task = BackgroundButton(self._test_btn, "test connection")
        self._test_result = RichLabel(row, fg=ERROR_COLOR, bg=ttk_background(self),
                                      font="TkDefaultFont")
        self._test_result.pack(side="left", fill="x", expand=True, padx=8)

    def _on_provider_selected(self) -> None:
        target = self._provider_keys[self._provider_names.index(self._provider_shown.get())]
        if target == self._provider:
            return
        previous = self._provider
        self._provider = target
        # 名稱還是上一家的自動值就跟著換；使用者取過名字就不覆蓋
        if self._name.get().strip() == PROVIDERS[previous].short_name:
            self._name.set(PROVIDERS[target].short_name)
        for var in (self._api_key, self._model, self._base_url, self._effort):
            var.set("")
        self._thinking.set(False)
        self._effort.set(EFFORT_AUTO)
        self._test_result.set("")
        log(f"[settings] service form provider switched {previous} -> {target}")
        self._rebuild_fields()

    # --- 動態欄位 ---
    def _rebuild_fields(self) -> None:
        for widget in self._fields.winfo_children():
            widget.destroy()
        prov = PROVIDERS[self._provider]
        if prov.needs_base_url:
            # 依填寫順序排：網址→金鑰→模型。模型清單要靠前兩者才取得到。
            self._labeled_entry(t("field.base_url"), self._base_url)
            self._field_hint(t("hint.custom_endpoint"))
            self._labeled_entry(t("field.api_key_optional"), self._api_key, secret=True)
            self._model_row()
            self._thinking_row()
        else:
            self._labeled_entry(t("field.api_key"), self._api_key, secret=True)
            # 取金鑰的連結緊貼金鑰欄：它是這一欄的輔助說明
            link_label(self._fields, t("link.get_key"), prov.key_url).pack(
                anchor="w", pady=(2, 0))
            self._model_row()
            if prov.has_field("thinking"):
                self._thinking_row()
            if prov.has_field("effort"):
                self._effort_row()
        self._invalidate_test()
        if self._on_change:
            self._on_change()
```

其餘的 `_field_hint`／`_model_row`／`_effort_row`／`_thinking_row`／`_labeled_entry`／
`_invalidate_test`／`_start_test`／`_on_tested`／`_show_test_result` 從 `src/ui/fields.py`
的 `ApiFields` **逐字搬過來**，只做兩處替換：

- `self.active_values` → `self.api_values`（`ModelField` 的 `api_getter`、`_start_test`）
- `validate_api_form(api)` → `validate_service(api)`

- [ ] **Step 4: 加語言檔的兩個欄位標籤**

在三份語言檔（`zh-TW.json`／`zh-CN.json`／`en-US.json`）的 `field.model` 附近加：

- `zh-TW`：`"field.service_name": "名稱",` / `"field.provider": "服務商",`
- `zh-CN`：`"field.service_name": "名称",` / `"field.provider": "服务商",`
- `en-US`：`"field.service_name": "Name",` / `"field.provider": "Provider",`

- [ ] **Step 5: 跑測試確認通過**

Run: `uv run pytest tests/test_service_form.py -v`
Expected: 全部 PASS

- [ ] **Step 6: Lint 並 commit**

```bash
uv run ruff check src tests tools
git add src/ui/service_form.py tests/test_service_form.py src/i18n/zh-TW.json src/i18n/zh-CN.json src/i18n/en-US.json
git commit -m "feat(ui): add ServiceForm for editing a single translation service"
```

---

### Task 6: `form.collapsible()` —— 摺疊區塊

**Files:**
- Modify: `src/ui/form.py`
- Test: `tests/test_form.py`

**Interfaces:**
- Produces: `collapsible(parent, text: str, expanded: bool = False) -> ttk.Frame` —— 回傳 body frame，body 上帶 `toggle()` 與 `is_expanded()` 兩個方法供測試與呼叫端使用

- [ ] **Step 1: 寫失敗的測試**

```python
# tests/test_form.py（追加）
from src.ui.form import collapsible


def test_collapsible_starts_collapsed(root):
    body = collapsible(root, "進階")
    assert not body.is_expanded()
    assert not body.winfo_ismapped()


def test_collapsible_can_start_expanded(root):
    body = collapsible(root, "進階", expanded=True)
    root.update_idletasks()
    assert body.is_expanded()


def test_collapsible_toggles(root):
    body = collapsible(root, "進階")
    body.toggle()
    assert body.is_expanded()
    body.toggle()
    assert not body.is_expanded()
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `uv run pytest tests/test_form.py -k collapsible -v`
Expected: FAIL，`ImportError: cannot import name 'collapsible'`

- [ ] **Step 3: 實作**

加到 `src/ui/form.py`：

```python
def collapsible(parent, text: str, expanded: bool = False) -> ttk.Frame:
    """可摺疊的區塊：回傳裝內容的 frame（帶 toggle()／is_expanded()）。
    tkinter 沒有現成的，標準組合是一個可點的標題列加一個 pack／pack_forget 的內容 frame。"""
    holder = ttk.Frame(parent)
    holder.pack(fill="x")
    header = ttk.Label(holder, cursor="hand2")
    header.pack(anchor="w")
    body = ttk.Frame(holder)
    state = {"expanded": False}

    def relabel() -> None:
        header.configure(text=f"{'▾' if state['expanded'] else '▸'} {text}")

    def toggle() -> None:
        state["expanded"] = not state["expanded"]
        if state["expanded"]:
            body.pack(fill="x", padx=(16, 0), pady=(2, 0))
        else:
            body.pack_forget()
        relabel()

    body.toggle = toggle
    body.is_expanded = lambda: state["expanded"]
    header.bind("<Button-1>", lambda e: toggle())
    relabel()
    if expanded:
        toggle()
    return body
```

- [ ] **Step 4: 跑測試確認通過**

Run: `uv run pytest tests/test_form.py -v`
Expected: 全部 PASS

- [ ] **Step 5: Lint 並 commit**

```bash
uv run ruff check src tests tools
git add src/ui/form.py tests/test_form.py
git commit -m "feat(ui): add a collapsible section helper"
```

---

### Task 7: `ServicePane` —— 「翻譯服務」分頁與清單管理

獨立元件，吃／吐的就是 cfg 的那三個鍵，**還不接進設定視窗**（那是 Task 8）。

**Files:**
- Create: `src/ui/service_list.py`
- Test: `tests/test_service_list.py`

**Interfaces:**
- Consumes: Task 3／4 的 `SLOTS`、`new_service`、`unique_name`、`describe`、`find`、`validate_service`；Task 5 的 `ServiceForm`；Task 6 的 `collapsible`；既有的 `ScrollableFrame`
- Produces:
  - `ServicePane(parent, section: dict, target_language_fn=None)`，`section` 形如 `{"services": [...], "default_service": id|None, "service_slots": {slot: id|None}}`
  - `.values() -> dict`（同形，供設定視窗寫回 cfg）
  - `.add_service()` / `.edit_service(service_id)` / `.delete_service(service_id)`（測試直接呼叫，不經滑鼠）
  - `ServiceDialog(parent, service, services, target_language_fn)`，`.result: dict | None`、`._ok()`、`._cancel()`
  - `open_service_dialog(parent, service, services, target_language_fn) -> dict | None`

- [ ] **Step 1: 寫失敗的測試**

```python
# tests/test_service_list.py
"""ServicePane：服務清單分頁的行為。"""
import pytest

from src.services import SLOT_INCOMING, SLOT_OUTGOING, SLOT_REGION, SLOTS, new_service
from src.ui.service_list import ServicePane


def _service(provider, services, **fields):
    service = new_service(provider, services)
    service.update({"model": "m", "api_key": "k", **fields})
    return service


@pytest.fixture
def two(root):
    a = _service("openai", [])
    b = _service("claude", [a])
    section = {"services": [a, b], "default_service": a["id"],
               "service_slots": {slot: None for slot in SLOTS}}
    return ServicePane(root, section), a, b


def test_values_round_trip_unchanged(two):
    pane, a, b = two
    assert pane.values() == {"services": [a, b], "default_service": a["id"],
                             "service_slots": {slot: None for slot in SLOTS}}


def test_cards_are_listed_in_order_with_the_default_marked(two):
    pane, a, b = two
    assert pane.card_labels() == [f"● {a['name']}", f"　{b['name']}"]


def test_deleting_the_default_hands_it_to_the_first_remaining(two):
    pane, a, b = two
    pane.delete_service(a["id"])
    assert pane.values()["default_service"] == b["id"]
    assert [s["id"] for s in pane.values()["services"]] == [b["id"]]


def test_deleting_an_assigned_service_frees_that_slot(root):
    a = _service("openai", [])
    b = _service("claude", [a])
    pane = ServicePane(root, {"services": [a, b], "default_service": a["id"],
                              "service_slots": {SLOT_INCOMING: None,
                                                SLOT_OUTGOING: None,
                                                SLOT_REGION: b["id"]}})
    pane.delete_service(b["id"])
    assert pane.values()["service_slots"][SLOT_REGION] is None


def test_the_last_service_cannot_be_deleted(root):
    a = _service("openai", [])
    pane = ServicePane(root, {"services": [a], "default_service": a["id"],
                              "service_slots": {slot: None for slot in SLOTS}})
    assert pane.delete_button_enabled(a["id"]) is False
    pane.delete_service(a["id"])
    assert len(pane.values()["services"]) == 1


def test_adding_to_an_empty_list_makes_it_the_default(root):
    pane = ServicePane(root, {"services": [], "default_service": None,
                              "service_slots": {slot: None for slot in SLOTS}})
    added = _service("openai", [])
    pane.apply_dialog_result(added)
    assert pane.values()["default_service"] == added["id"]


def test_a_saved_name_never_collides(two):
    pane, a, b = two
    renamed = dict(b, name=a["name"])
    pane.apply_dialog_result(renamed)
    assert pane.values()["services"][1]["name"] == f"{a['name']} (2)"


def test_slots_collapsed_when_everything_follows_the_default(two):
    pane, _a, _b = two
    assert pane.slots_expanded() is False


def test_slots_expanded_when_one_is_assigned(root):
    a = _service("openai", [])
    b = _service("claude", [a])
    pane = ServicePane(root, {"services": [a, b], "default_service": a["id"],
                              "service_slots": {SLOT_INCOMING: None,
                                                SLOT_OUTGOING: None,
                                                SLOT_REGION: b["id"]}})
    assert pane.slots_expanded() is True


def test_slot_options_start_with_follow_the_default(two):
    pane, a, b = two
    assert pane.slot_options() == [None, a["id"], b["id"]]
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `uv run pytest tests/test_service_list.py -v`
Expected: FAIL，`ModuleNotFoundError: No module named 'src.ui.service_list'`

- [ ] **Step 3: 實作對話框**

```python
"""「翻譯服務」分頁：預設服務、用途分派與服務清單，外加新增／編輯用的對話框。"""
import copy
import tkinter as tk
from tkinter import messagebox, ttk

from src.i18n import t
from src.log import log
from src.services import (
    SLOTS,
    describe,
    new_service,
    unique_name,
    validate_service,
)
from src.services import API_PROVIDERS
from src.ui.form import HINT_COLOR, collapsible, hint_label
from src.ui.fonts import ui_font
from src.ui.service_form import ServiceForm

_DIALOG_SIZE = (560, 480)   # 容得下最長的一組欄位（自訂端點）與測試結果訊息


class ServiceDialog:
    """新增／編輯單筆服務的 modal 對話框；`result` 是確定後的服務，取消時為 None。
    測試直接建構本類別並呼叫 `_ok()`／`_cancel()`，不經 `open_service_dialog` 的等待迴圈。"""

    def __init__(self, parent, service: dict, services: list[dict],
                 target_language_fn=None):
        self._services = services
        self.result: dict | None = None
        self.win = tk.Toplevel(parent)
        self.win.title(t("service.dialog_title"))
        self.win.geometry("{}x{}".format(*_DIALOG_SIZE))
        self.win.transient(parent.winfo_toplevel())
        self.win.protocol("WM_DELETE_WINDOW", self._cancel)

        buttons = ttk.Frame(self.win, padding=(8, 0, 8, 8))
        buttons.pack(side="bottom", fill="x")
        ttk.Button(buttons, text=t("button.cancel"), command=self._cancel).pack(
            side="right")
        ttk.Button(buttons, text=t("button.ok"), command=self._ok).pack(
            side="right", padx=(0, 8))

        self.form = ServiceForm(self.win, service)
        if target_language_fn is not None:
            self.form.set_target_language_fn(target_language_fn)
        self.form.pack(fill="both", expand=True, padx=12, pady=12)

    def _ok(self) -> None:
        errors = validate_service(self.form.api_values())
        if errors:
            messagebox.showwarning(t("dialog.incomplete_title"),
                                   "\n".join(t(e) for e in errors), parent=self.win)
            return
        service = self.form.values()
        # 清單裡永遠不會出現兩個一樣的名稱：三個分派下拉只顯示名稱，同名會讓人選錯
        service["name"] = unique_name(service["name"], self._services,
                                      ignore_id=service["id"])
        self.result = service
        log(f"[settings] service saved (provider={service['provider']}, "
            f"has_key={bool(service.get('api_key'))})")
        self.win.destroy()

    def _cancel(self) -> None:
        self.win.destroy()


def open_service_dialog(parent, service: dict, services: list[dict],
                        target_language_fn=None) -> dict | None:
    """顯示對話框並等待關閉；確定回傳服務，取消回傳 None。"""
    dialog = ServiceDialog(parent, service, services, target_language_fn)
    parent.wait_window(dialog.win)
    return dialog.result
```

- [ ] **Step 4: 實作分頁**

接在同一個檔案之後：

```python
class ServicePane(ttk.Frame):
    """翻譯服務分頁：預設服務下拉、摺疊的用途分派、服務卡片清單。
    `values()` 回傳的三個鍵可直接寫回 cfg。"""

    def __init__(self, parent, section: dict, target_language_fn=None):
        super().__init__(parent)
        self._target_language_fn = target_language_fn
        self._services = copy.deepcopy(section["services"])
        self._default = section["default_service"]
        self._slots = dict(section["service_slots"])

        self._default_shown = tk.StringVar()
        self._slot_shown = {slot: tk.StringVar() for slot in SLOTS}
        self._default_row()
        self._slots_body = collapsible(
            self, t("service.slots_title"),
            expanded=any(self._slots[slot] is not None for slot in SLOTS))
        self._slot_rows()
        ttk.Label(self, text=t("service.my_services"), font=ui_font(10, "bold")).pack(
            anchor="w", pady=(12, 4))
        self._cards = ttk.Frame(self)
        self._cards.pack(fill="x")
        ttk.Button(self, text=t("button.add_service"), command=self.add_service).pack(
            pady=(8, 0))
        self._refresh()

    # --- 值存取 ---
    def values(self) -> dict:
        """目前編輯中的三個鍵（設定視窗按儲存時寫回 cfg）。"""
        return {"services": copy.deepcopy(self._services),
                "default_service": self._default,
                "service_slots": dict(self._slots)}

    def card_labels(self) -> list[str]:
        """卡片的標題列文字（預設那張前綴 ●，其餘留等寬空位）。"""
        return [f"{'●' if s['id'] == self._default else '　'} {s['name']}"
                for s in self._services]

    def slot_options(self) -> list[str | None]:
        """分派下拉的選項順序：None（跟隨預設）在最前面。"""
        return [None, *(s["id"] for s in self._services)]

    def slots_expanded(self) -> bool:
        return self._slots_body.is_expanded()

    def delete_button_enabled(self, service_id: str) -> bool:
        """最後一筆不給刪：刪光就無從翻譯，擋在按鈕比擋在儲存清楚。"""
        return len(self._services) > 1

    # --- 清單操作 ---
    def add_service(self) -> None:
        draft = new_service(API_PROVIDERS[0], self._services)
        result = open_service_dialog(self, draft, self._services,
                                     self._target_language_fn)
        if result is not None:
            self.apply_dialog_result(result)

    def edit_service(self, service_id: str) -> None:
        current = next(s for s in self._services if s["id"] == service_id)
        result = open_service_dialog(self, copy.deepcopy(current), self._services,
                                     self._target_language_fn)
        if result is not None:
            self.apply_dialog_result(result)

    def apply_dialog_result(self, service: dict) -> None:
        """把對話框確定的服務併回清單（新增或就地取代），必要時接手預設。"""
        service = dict(service)
        service["name"] = unique_name(service["name"], self._services,
                                      ignore_id=service["id"])
        for index, existing in enumerate(self._services):
            if existing["id"] == service["id"]:
                self._services[index] = service
                break
        else:
            self._services.append(service)
        if self._default is None:
            self._default = service["id"]
            log(f"[settings] first service added; default={service['id']}")
        self._refresh()

    def delete_service(self, service_id: str) -> None:
        if not self.delete_button_enabled(service_id):
            return
        target = next(s for s in self._services if s["id"] == service_id)
        if not messagebox.askyesno(t("dialog.confirm_title"),
                                   t("service.confirm_delete", name=target["name"]),
                                   parent=self.winfo_toplevel()):
            return
        self._services = [s for s in self._services if s["id"] != service_id]
        for slot in SLOTS:
            if self._slots[slot] == service_id:
                self._slots[slot] = None
                log(f"[settings] slot {slot} followed the default after its service "
                    f"was deleted")
        if self._default == service_id:
            self._default = self._services[0]["id"] if self._services else None
            log(f"[settings] default service deleted; default={self._default}")
        self._refresh()
```

`_default_row`／`_slot_rows`／`_refresh` 的實作：

```python
    def _default_row(self) -> None:
        row = ttk.Frame(self)
        row.pack(fill="x", pady=(0, 8))
        ttk.Label(row, text=t("service.default")).pack(side="left")
        self._default_combo = ttk.Combobox(row, textvariable=self._default_shown,
                                           state="readonly")
        self._default_combo.pack(side="left", fill="x", expand=True, padx=(8, 0))
        self._default_combo.bind("<<ComboboxSelected>>",
                                 lambda e: self._pick_default())

    def _slot_rows(self) -> None:
        self._slot_combos = {}
        for slot in SLOTS:
            row = ttk.Frame(self._slots_body)
            row.pack(fill="x", pady=2)
            ttk.Label(row, text=t(f"slot.{slot}")).pack(side="left")
            combo = ttk.Combobox(row, textvariable=self._slot_shown[slot],
                                 state="readonly")
            combo.pack(side="left", fill="x", expand=True, padx=(8, 0))
            combo.bind("<<ComboboxSelected>>",
                       lambda e, s=slot: self._pick_slot(s))
            self._slot_combos[slot] = combo
        hint_label(self._slots_body, t("service.slots_hint")).pack(fill="x",
                                                                   pady=(2, 0))

    def _names(self) -> list[str]:
        return [s["name"] for s in self._services]

    def _pick_default(self) -> None:
        self._default = self._services[self._names().index(
            self._default_shown.get())]["id"]
        log(f"[settings] default service set to {self._default}")
        self._refresh()

    def _pick_slot(self, slot: str) -> None:
        index = [t("service.follow_default"), *self._names()].index(
            self._slot_shown[slot].get())
        self._slots[slot] = self.slot_options()[index]
        log(f"[settings] slot {slot} set to {self._slots[slot] or 'default'}")

    def _refresh(self) -> None:
        """下拉選項與卡片都由清單現況重畫（新增、刪除、改名共用同一條路）。"""
        names = self._names()
        self._default_combo.configure(values=names)
        current = next((s for s in self._services if s["id"] == self._default), None)
        self._default_shown.set(current["name"] if current else "")
        for slot in SLOTS:
            self._slot_combos[slot].configure(
                values=[t("service.follow_default"), *names])
            assigned = next((s for s in self._services
                             if s["id"] == self._slots[slot]), None)
            self._slot_shown[slot].set(
                assigned["name"] if assigned else t("service.follow_default"))
        for widget in self._cards.winfo_children():
            widget.destroy()
        for label, service in zip(self.card_labels(), self._services):
            self._card(label, service)

    def _card(self, label: str, service: dict) -> None:
        card = ttk.Frame(self._cards, relief="solid", borderwidth=1, padding=8)
        card.pack(fill="x", pady=2)
        top = ttk.Frame(card)
        top.pack(fill="x")
        ttk.Label(top, text=label, font=ui_font(10, "bold")).pack(side="left")
        delete = ttk.Button(top, text=t("button.delete"), width=7,
                            command=lambda: self.delete_service(service["id"]))
        delete.pack(side="right")
        if not self.delete_button_enabled(service["id"]):
            delete.configure(state="disabled")
        ttk.Button(top, text=t("button.edit"), width=7,
                   command=lambda: self.edit_service(service["id"])).pack(
            side="right", padx=(0, 4))
        ttk.Label(card, text=describe(service), foreground=HINT_COLOR).pack(anchor="w")
```

- [ ] **Step 5: 加語言檔文案**

三份語言檔（`zh-TW`／`zh-CN`／`en-US`）各加：

| key | zh-TW | zh-CN | en-US |
|---|---|---|---|
| `settings.tab.services` | 翻譯服務 | 翻译服务 | Services |
| `service.default` | 預設服務 | 默认服务 | Default service |
| `service.slots_title` | 各用途指定不同服務（進階） | 各用途指定不同服务（进阶） | Use a different service per feature (advanced) |
| `service.slots_hint` | 未指定的用途一律使用預設服務。 | 未指定的用途一律使用默认服务。 | Anything left unset uses the default service. |
| `service.follow_default` | 跟隨預設 | 跟随默认 | Follow the default |
| `service.my_services` | 我的服務 | 我的服务 | My services |
| `service.dialog_title` | 翻譯服務 | 翻译服务 | Translation service |
| `service.confirm_delete` | 要刪除「{name}」嗎？ | 要删除“{name}”吗？ | Delete “{name}”? |
| `slot.incoming` | 聊天收訊 | 聊天收信 | Incoming chat |
| `slot.outgoing` | 聊天發話 | 聊天发言 | Outgoing chat |
| `slot.region` | 區域翻譯 | 区域翻译 | Region translation |
| `button.add_service` | ＋ 新增翻譯服務 | ＋ 新增翻译服务 | + Add a service |
| `button.edit` | 編輯 | 编辑 | Edit |
| `button.delete` | 刪除 | 删除 | Delete |
| `button.ok` | 確定 | 确定 | OK |
| `dialog.confirm_title` | 確認 | 确认 | Confirm |

- [ ] **Step 6: 跑測試確認通過**

`delete_service` 會跳確認框，測試要先把它擋掉。在 `tests/test_service_list.py` 檔頭加：

```python
@pytest.fixture(autouse=True)
def _always_confirm(monkeypatch):
    """刪除的確認對話框在測試裡一律答「是」，免得測試停在等人按鈕。"""
    monkeypatch.setattr("src.ui.service_list.messagebox.askyesno", lambda *a, **k: True)
```

Run: `uv run pytest tests/test_service_list.py -v`
Expected: 全部 PASS

- [ ] **Step 7: Lint 並 commit**

```bash
uv run ruff check src tests tools
git add src/ui/service_list.py tests/test_service_list.py src/i18n/zh-TW.json src/i18n/zh-CN.json src/i18n/en-US.json
git commit -m "feat(ui): add the translation services pane"
```

---

### Task 8: Schema 切換與全面接線

這是唯一一刀切的 task：`config.py` 換 schema 的同時，設定視窗、精靈與 `main` 都要一起改到新結構，中間沒有能保持全綠的落腳點。做完這個 task 舊的 `ApiFields`、`active_api` 與 `api` 區塊全部消失。

**Files:**
- Modify: `src/config.py`、`src/ui/fields.py`、`src/ui/settings.py`、`src/ui/wizard.py`、`src/main.py`、`tests/config_helpers.py`、`tests/test_config.py`、`tests/test_fields.py`、`tests/test_main.py`、`tests/test_wizard.py`、`tests/test_settings.py`

**Interfaces:**
- Consumes: Task 3／4 的 `SLOTS`／`SLOT_*`／`resolve`／`find`／`normalize`／`validate_service`；Task 5 的 `ServiceForm`；Task 7 的 `ServicePane`
- Produces:
  - `config.DEFAULT_CONFIG` 帶 `services`／`default_service`／`service_slots`，不再有 `api`
  - `config.is_configured(cfg) -> bool`
  - `main.build_translation(cfg, deliver) -> tuple[dict[str, Translator], TranslationCache, list[TranslationPool]]`
  - `main.config_summary(cfg) -> str`（不再吃第二個參數）

- [ ] **Step 1: 改 `src/config.py`**

```python
# 匯入改成
from src.services import SLOTS, find, normalize, validate_service

# DEFAULT_CONFIG 的第一段改成
DEFAULT_CONFIG: dict = {
    "services": [],          # 使用者建立的翻譯服務；空＝尚未設定，啟動時進精靈
    "default_service": None,  # 預設服務的 id；未指定用途的都跟著它走
    "service_slots": {slot: None for slot in SLOTS},  # None＝跟隨預設
    "ui_language": None,
    ...其餘欄位不動
}
```

刪除 `_default_api()`、`_LEGACY_API_FIELDS`、`_is_legacy_api()`、`_migrate_api()`、
`_prune_profiles()`、`active_api()`、`needs_base_url` 的匯入。`load_config()` 中段改成：

```python
    cfg = clamp_advanced(_merge(DEFAULT_CONFIG, data))
    if normalize(cfg):
        # 整理後立刻落地，手開 config.json 看到的就是生效的結構；寫不進去不擋啟動
        try:
            save_config(path, cfg)
            log(f"[config] rewrote {path.name} in the service list format")
        except OSError as exc:
            log(f"[config] could not rewrite {path.name}: {exc}")
    return cfg
```

`is_configured()` 改成：

```python
def is_configured(cfg: dict) -> bool:
    """預設服務存在且設定完整（不完整 → 啟動時進首次設定精靈）。"""
    service = find(cfg, cfg["default_service"])
    return service is not None and not validate_service(service)
```

- [ ] **Step 2: 改 `tests/config_helpers.py`**

```python
"""測試用的設定輔助：一行生出一份設定完整的 cfg。"""
import copy

from src.config import DEFAULT_CONFIG
from src.services import new_service

_MINIMAL = {
    "custom": {"base_url": "http://x", "model": "m"},
    "openai": {"api_key": "k", "model": "m"},
    "claude": {"api_key": "k", "model": "m"},
}


def configured_cfg(provider: str = "custom", **fields) -> dict:
    """一份有單一預設服務、設定完整的 cfg 深副本；`fields` 覆寫該服務的欄位。"""
    cfg = copy.deepcopy(DEFAULT_CONFIG)
    service = new_service(provider, [])
    service.update({**_MINIMAL[provider], **fields})
    cfg["services"] = [service]
    cfg["default_service"] = service["id"]
    return cfg
```

- [ ] **Step 3: 改 `src/ui/fields.py`**

刪掉整個 `ApiFields` 類別與它專用的匯入（`copy`、`API_EFFORTS`、`API_PROFILE_FIELDS`、
`API_PROVIDERS`、`EFFORT_AUTO`、`PROVIDERS`、`validate_service`、`test_translate`、
`ModelField`、`ERROR_COLOR`、`BackgroundButton`、`friendly_error`、`link_label`、
`show_outcome`、`RichLabel`、`ttk_background`、`log`），檔案只留 `COMMON_LANGUAGES`、
`HotkeyField`、`LanguageField`、`UiLanguageField`，模組 docstring 改成：

```python
"""精靈與設定視窗共用的欄位群：熱鍵捕捉、翻譯目標語言與介面語言選擇。
小元件在 form.py，單筆服務的表單在 service_form.py。"""
```

- [ ] **Step 4: 改 `src/ui/settings.py`**

1. 匯入：`from src.ui.fields import HotkeyField, LanguageField, UiLanguageField`、
   `from src.ui.service_list import ServicePane`；刪掉 `from src.services import validate_service`（驗證改由分頁與對話框負責）。
2. `_build_basic()`：刪掉 `self._api = ApiFields(...)` 與 `self._api.set_target_language_fn(...)` 兩行，`LanguageField` 的 `on_change` 改成 `lambda: None`（目標語言一改要作廢的測試結果已經在對話框裡，對話框關掉就沒了）。
3. `open()` 裡在 `self._build_basic(nb, cfg)` 之後插入：

```python
        self._build_services(nb, cfg)
```

4. 新增：

```python
    def _build_services(self, nb, cfg: dict) -> None:
        """翻譯服務分頁：預設服務、用途分派與服務清單。"""
        scroll = ScrollableFrame(nb, padding=12)
        nb.add(scroll, text=t("settings.tab.services"))
        self._services = ServicePane(scroll.body, cfg,
                                     target_language_fn=lambda: self._language.value())
        self._services.pack(fill="x")
```

5. `_form_values()`：把 `"api": self._api.get_values(),` 換成 `**self._services.values(),`。
6. `_save()`：`errors = validate_api_form(self._api.active_values())` 換成：

```python
        errors = [] if values["default_service"] else ["error.need_service"]
```

- [ ] **Step 5: 改 `src/ui/wizard.py`**

1. 匯入 `from src.ui.service_form import ServiceForm`、`from src.services import API_PROVIDERS, SLOTS, new_service, validate_service`，刪掉 `ApiFields` 與 `validate_api_form`。
2. `__init__` 裡：

```python
        self._draft_service = new_service(API_PROVIDERS[0], [])
        self._service_form = ServiceForm(self._body, self._draft_service,
                                         on_change=self._on_api_change)
```

   並把 `persistent` 集合裡的 `self._api_fields` 換成 `self._service_form`。
3. `_show_step()` 的 `STEP_API` 分支把 `self._api_fields.pack(...)` 換成
   `self._service_form.pack(fill="x", pady=(10, 0))`。
4. `_refresh_nav()` 的 `hasattr(self, "_api_fields")` 換成 `hasattr(self, "_service_form")`，
   `api = self._api_fields.active_values()` 換成 `api = self._service_form.api_values()`，
   `validate_api_form(api)` 換成 `validate_service(api)`。
5. `_collect_into_cfg()` 的第一行換成：

```python
        service = self._service_form.values()
        self._cfg["services"] = [service]
        self._cfg["default_service"] = service["id"]
        self._cfg["service_slots"] = {slot: None for slot in SLOTS}
```

- [ ] **Step 6: 改 `src/main.py`**

1. 匯入：`from src.config import ...` 移除 `active_api`；加 `from src.services import SLOT_INCOMING, SLOT_OUTGOING, SLOT_REGION, SLOTS, find, resolve`。
2. `config_summary()` 改成：

```python
def config_summary(cfg: dict) -> str:
    """一行設定摘要（啟動與套用設定時記錄，兩處同一份才不會漏欄位）；金鑰絕不列入。"""
    return (f"services={len(cfg['services'])}, "
            f"default={_service_summary(find(cfg, cfg['default_service']))}, "
            + ", ".join(f"{slot}=" + (_service_summary(find(cfg, cfg['service_slots'][slot]))
                                      if cfg['service_slots'][slot] else "default")
                        for slot in SLOTS) + ", "
            f"target_language={cfg['target_language']!r}, "
            f"ui_language={cfg['ui_language']}, "
            f"hotkey={cfg['hotkey']}, region_hotkey={cfg['region_hotkey']}, "
            f"paste_hotkey={cfg['paste_hotkey']}, "
            f"auto_show_input={cfg['auto_show_input']}, "
            f"poll_interval={cfg['poll_interval']}, "
            f"parallel={cfg['max_parallel_translations']}, "
            f"fade_seconds={cfg['fade_seconds']}, max_messages={cfg['max_messages']}, "
            f"overlay_alpha={cfg['overlay_alpha']}, "
            f"translate_system_messages={cfg['translate_system_messages']}")


def _service_summary(service: dict | None) -> str:
    """一筆服務在 log 裡的樣子；金鑰只記有沒有，絕不記內容。"""
    if service is None:
        return "none"
    return (f"{service['name']}({service['provider']}/{service['model']},"
            f"has_key={bool(service.get('api_key'))})")
```

   `log_startup_summary(cfg)` 同步去掉 `api` 參數。

3. `build_translation()` 改成：

```python
def build_translation(cfg: dict, deliver) -> tuple[dict, TranslationCache,
                                                   list[TranslationPool]]:
    """翻譯端：三個用途各一個翻譯器、系統訊息譯文快取，以及兩條翻譯池（玩家對話吃上下文；
    系統訊息不吃上下文、走快取）。兩條池共用同一個總量閘與 `deliver(msg_id, text, failed)`。"""
    translators = {slot: Translator(**resolve(cfg, slot),
                                    target_language=cfg["target_language"])
                   for slot in SLOTS}
    incoming = resolve(cfg, SLOT_INCOMING)
    gate = ConcurrencyGate(cfg["max_parallel_translations"])
    cache = TranslationCache(fingerprint_of(incoming["provider"], incoming["model"],
                                            cfg["target_language"]))
    cache.load()

    def make_pool(translate_fn=None) -> TranslationPool:
        return TranslationPool(translator=translators[SLOT_INCOMING], on_result=deliver,
                               workers=cfg["max_parallel_translations"],
                               failed_notice_fn=lambda: t("notice.translate_failed"),
                               translate_fn=translate_fn, gate=gate)

    pool = make_pool()
    system_pool = make_pool(
        lambda text, _ctx: translate_and_cache(translators[SLOT_INCOMING], cache, text))
    return translators, cache, [pool, system_pool]
```

4. `build_app()`：刪掉 `api = active_api(cfg)`，`log_startup_summary(cfg)`；
   `translator, cache, pools = build_translation(cfg, api, deliver)` 換成
   `translators, cache, pools = build_translation(cfg, deliver)`；
   `InputBox(...)` 裡的 `translator.translate_outgoing` 換成
   `translators[SLOT_OUTGOING].translate_outgoing`；
   `RegionPipeline(translator)` 換成 `RegionPipeline(translators[SLOT_REGION])`。
5. `apply_settings()` 中段換成：

```python
        for slot, tr in translators.items():
            tr.reconfigure(**resolve(cfg, slot),
                           target_language=cfg["target_language"])
        for p in pools:
            p.resize(cfg["max_parallel_translations"])
        # 收訊的服務商／模型／目標語言任一改變，舊譯文即失效
        incoming = resolve(cfg, SLOT_INCOMING)
        cache.rebind(fingerprint_of(incoming["provider"], incoming["model"],
                                    cfg["target_language"]))
```

   同函式內的 `log(f"[app] settings applied; {config_summary(cfg, applied_api)}")`
   改成 `config_summary(cfg)`。

- [ ] **Step 7: 加缺服務的錯誤文案**

三份語言檔各加 `error.need_service`：

- zh-TW：`"error.need_service": "請先新增一組翻譯服務",`
- zh-CN：`"error.need_service": "请先新增一组翻译服务",`
- en-US：`"error.need_service": "Add a translation service first",`

- [ ] **Step 8: 改測試**

1. `tests/test_fields.py`：刪掉 `_initial()`、`_switch()` 與所有 `ApiFields` 測試（這些在 Task 5 已由 `tests/test_service_form.py` 覆蓋），保留 `friendly_error`、`filter_models`、`ModelField`、`HotkeyField`、`validate_*` 那幾組。
2. `tests/test_config.py`：把 `api` 相關的測試改寫成 services 版本，例如：

```python
def test_load_merges_partial_file_with_defaults(tmp_path: Path):
    p = tmp_path / "config.json"
    p.write_text(json.dumps({"poll_interval": 3.0}), encoding="utf-8")
    cfg = load_config(p)
    assert cfg["poll_interval"] == 3.0
    assert cfg["services"] == []
    assert cfg["default_service"] is None
    assert cfg["hotkey"] == "ctrl+space"


def test_legacy_api_block_is_migrated_and_rewritten(tmp_path: Path):
    p = tmp_path / "config.json"
    p.write_text(json.dumps({"api": {"provider": "claude",
                                     "claude": {"model": "claude-opus-5",
                                                "api_key": "sk-ant-1"}}}),
                 encoding="utf-8")
    cfg = load_config(p)
    assert [s["provider"] for s in cfg["services"]] == ["claude"]
    assert cfg["default_service"] == cfg["services"][0]["id"]
    on_disk = json.loads(p.read_text(encoding="utf-8"))
    assert "api" not in on_disk
    assert on_disk["services"][0]["api_key"] == "sk-ant-1"


def test_is_configured_needs_a_complete_default_service():
    from src.config import DEFAULT_CONFIG
    assert not is_configured(copy.deepcopy(DEFAULT_CONFIG))
    assert is_configured(configured_cfg())
    incomplete = configured_cfg("openai", api_key="")
    assert not is_configured(incomplete)
```

   刪掉 `test_active_api_is_a_flat_copy_of_the_selected_profile` 等測 `active_api` 的則數
   （`resolve` 的對應測試已在 `tests/test_services.py`）。
3. `tests/test_main.py`：`config_summary(cfg, active_api(cfg))` 改成 `config_summary(cfg)`，
   斷言改成檢查金鑰沒外洩且三格都在：

```python
def test_config_summary_lists_every_slot_and_never_leaks_the_key():
    cfg = configured_cfg("custom", base_url="http://x", model="gemma",
                         api_key="sk-secret")
    summary = config_summary(cfg)
    assert "sk-secret" not in summary
    assert "has_key=True" in summary
    assert "incoming=default" in summary and "region=default" in summary
```

4. `tests/test_settings.py`、`tests/test_wizard.py`、`tests/test_scrollable.py`、
   `tests/test_ui_pump.py`：把殘留的 `cfg["api"]` 操作換成 `configured_cfg(...)`；
   精靈測試裡的 `wizard._api_fields` 換成 `wizard._service_form`。

自我檢查（應為空輸出）：

Run: `uv run python -c "import subprocess,sys; out=subprocess.run(['git','grep','-n','-e','active_api','-e','ApiFields','-e','cfg\\[\"api\"\\]'],capture_output=True,text=True).stdout; print(out); sys.exit(out.strip() != '')"`

- [ ] **Step 9: 跑全套測試**

Run: `uv run pytest`
Expected: 全綠

- [ ] **Step 10: Lint 並 commit**

```bash
uv run ruff check src tests tools
git add -A
git commit -m "feat(config): replace the fixed api block with a service list"
```

---

### Task 9: 收尾 —— 防迴歸清單、文件與實機驗證

**Files:**
- Modify: `tests/test_no_hardcoded_ui_text.py`、`README.md`、`README_ZH-TW.md`、`README_ZH-CN.md`

- [ ] **Step 1: 更新硬編文字的掃描清單**

`tests/test_no_hardcoded_ui_text.py` 的 `SCAN_TARGETS`：移除 `"src/ui/providers.py"`，加入
`"src/services.py"`、`"src/ui/service_form.py"`、`"src/ui/service_list.py"`。

- [ ] **Step 2: 跑防迴歸測試**

Run: `uv run pytest tests/test_no_hardcoded_ui_text.py -v`
Expected: PASS（若失敗，代表新模組裡還有中日韓字面量沒走 i18n，改掉而不是把檔案從清單移除）

- [ ] **Step 3: 更新三份 README**

三處要改（依「README 不寫鍵名與選項文字」的既有慣例，只用泛稱，不寫介面上的字）：

1. 功能列表中「每家各存一份設定，換來換去不必重填金鑰」那條 —— 現在是多組並存，改成「可以建立多組服務並隨時切換，也能讓聊天與區域翻譯各用不同的一組」。
   - `README.md:85`、`README_ZH-TW.md:85`、`README_ZH-CN.md:85`
2. 首次精靈第二步的描述（`README.md:55`）：「選翻譯服務商」改成「建立第一組翻譯服務」。
3. 設定視窗截圖的說明文字（`README.md:118` 與另兩份）提到的分頁內容要對得上新版面。

- [ ] **Step 4: 請使用者更新設定視窗截圖**

截圖是實機畫面，交給使用者拍（本專案不自行操控桌面）。在回報時明確列出：`docs/images/`
下的設定視窗截圖已過期，需要重拍一張含「翻譯服務」分頁的。

- [ ] **Step 5: 跑 lint 與全套測試**

```bash
uv run ruff check src tests tools
uv run pytest
```
Expected: 零錯誤、全綠

- [ ] **Step 6: 實機驗證**

開著遊戲（登入進世界內）跑 `uv run run.py`：

1. 首次設定精靈建立一組服務 → 進主流程，收訊正常。
2. 設定視窗「翻譯服務」分頁新增第二組（換一家、換一個明顯不同的模型）→ 展開用途分派，
   把「區域翻譯」指給第二組 → 儲存。
3. 框選一塊畫面翻譯，確認 `app.log` 的 `[app] settings applied` 那行顯示
   `region=<第二組>`，且收訊那行仍是 `incoming=default`。
4. 刪掉被指派的那組，確認該格自動退回「跟隨預設」且 log 有記錄。

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "docs: describe the multi-service settings in the READMEs"
```

---

## Self-Review

**Spec coverage：**

| Spec 章節 | 對應 task |
|---|---|
| 資料模型／config.json 新樣貌 | Task 4、Task 8 Step 1 |
| 模組配置（`services.py`、刪 `ui/providers.py`） | Task 2、Task 3、Task 4 |
| 名稱規則（自動帶入、跟隨服務商、去重、清空退回） | Task 3（`unique_name`／`new_service`）、Task 5（跟隨服務商、清空退回）、Task 7（確定時去重） |
| 遷移（兩段串接、刪舊區塊） | Task 4 |
| 校驗（六條防線）、`is_configured` | Task 4、Task 8 Step 1 |
| UI 版面與分頁順序 | Task 7、Task 8 Step 4 |
| `ServiceForm` | Task 5 |
| `ServicePane` 與對話框 | Task 7 |
| `collapsible()` | Task 6 |
| 互動（新增／編輯／刪除／驗證時機／未儲存編輯） | Task 7、Task 8 Step 4 |
| 精靈 | Task 8 Step 5 |
| 執行期接線（三個 Translator、`apply_settings`、快取、啟動摘要） | Task 8 Step 6 |
| i18n（三份語言檔） | Task 2 Step 4、Task 5 Step 4、Task 7 Step 5、Task 8 Step 7 |
| 測試（`test_services.py`、helper、既有調整、SCAN_TARGETS） | Task 1、Task 3、Task 4、Task 5、Task 7、Task 8 Step 8、Task 9 Step 1 |
| 實機驗證 | Task 9 Step 6 |

**設計文件未涵蓋、實作時補上的兩點**（已寫進計畫）：

- `error.need_service`：清單為空時按儲存的錯誤文案（spec 只說 `is_configured` 會擋啟動，沒說設定視窗怎麼擋）。
- `ServicePane` 的 `apply_dialog_result()`／`card_labels()`／`slot_options()` 等查詢方法：對話框是 modal，測試不能走 `wait_window`，需要可直接呼叫的入口。

**型別一致性檢查：** `resolve()` 在 Task 3 定義為回傳不含 `id`／`name` 的攤平 dict，Task 8
的 `Translator(**resolve(...))` 與 `fingerprint_of(incoming["provider"], incoming["model"], …)`
都只用到 `provider`／`model`，一致。`ServiceForm.values()` 回傳含 `id`／`name` 的完整服務，
`ServicePane.apply_dialog_result()` 吃的正是這個形狀，一致。`normalize()` 與
`is_configured()` 都靠 `find()`，一致。
