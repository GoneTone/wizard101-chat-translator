# 收訊平行翻譯 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 以工作池取代 `reader_loop` 的序列翻譯迴圈，使單則翻譯卡住時不再堵住後續訊息，同時維持顯示順序與上下文品質不變。

**Architecture:** 上下文從 `Translator` 內部搬到獨立、執行緒安全的 `ChatContext`，由 reader 的**讀取順序**推進（而非「翻譯成功」）；`reader_loop` 退化為「讀新行 → 推進 context → overlay 佔位 → 提交 pool」；`TranslationPool` 包一層 `ThreadPoolExecutor`，以全域退避閘門節流失敗重試；overlay 訊息一讀到就先顯示原文，譯文由 `update_message(msg_id, ...)` 稍後就地填入，因此顯示順序在佔位當下即定案，不需要重排序緩衝。

**Tech Stack:** Python 3.11+、`concurrent.futures.ThreadPoolExecutor`、`threading`、tkinter、pytest、uv。

**Spec:** `docs/superpowers/specs/2026-08-24-parallel-translation-design.md`

## Global Constraints

以下為專案層級要求，每個 task 的驗收都隱含包含這一節（值一律照抄自 spec 與 `CLAUDE.md`）：

- **平行度**：config 欄位 `max_parallel_translations`，範圍 `(1, 8)`，預設 `4`；設 1 必須完全等同現行序列行為。
- **退避**：沿用 `BACKOFF_STEPS = [5, 15, 30]`（秒）與 `CONFIG_ERROR_INTERVAL = 15.0`（秒）。
- **重試語義**：`TranslatorOffline` 與 `TranslatorConfigError` 無限重試；`TranslatorBadOutput` 與其他例外不重試。
- **上下文容量**：`CONTEXT_LINES = 8`。
- **語言不可寫死**：命名用方向（`translate_incoming`／`translate_outgoing`），語言一律當參數傳入，程式碼不得出現 `zh`／`en`／繁體中文之類的硬編碼判斷。
- **UI 文字**繁體中文（台灣）全形標點；**log 訊息一律英文**，前綴沿用 `[reader]`／`[translate]`／`[ui]`；**API 金鑰絕不寫入 log**。
- **註解節制**：只寫 WHY 不顯而易見處與段落導引；模組與公開 function／class 寫 docstring。
- **Commit message 英文**、conventional commits、半形標點。
- 每個 task 結束前 `uv run pytest` 必須全綠。

---

### Task 1: ChatContext

**Files:**
- Create: `src/context.py`
- Test: `tests/test_context.py`

**Interfaces:**
- Consumes: 無
- Produces:
  - `CONTEXT_LINES: int = 8`
  - `class ChatContext: __init__(self, max_lines: int = CONTEXT_LINES)`、`push(self, line: str) -> None`、`snapshot(self) -> list[str]`

- [ ] **Step 1: Write the failing test**

`tests/test_context.py`：

```python
"""ChatContext：跨執行緒共用的近期原文行緩衝。"""
import threading

from src.context import CONTEXT_LINES, ChatContext


def test_snapshot_returns_pushed_lines_in_order():
    ctx = ChatContext()
    ctx.push("[A] one")
    ctx.push("[B] two")
    assert ctx.snapshot() == ["[A] one", "[B] two"]


def test_snapshot_is_a_copy():
    # 呼叫端拿到的 snapshot 之後不得被新 push 影響——worker 帶著它重試時上下文不能漂移
    ctx = ChatContext()
    ctx.push("[A] one")
    snap = ctx.snapshot()
    ctx.push("[B] two")
    assert snap == ["[A] one"]


def test_oldest_lines_drop_beyond_capacity():
    ctx = ChatContext()
    for i in range(CONTEXT_LINES + 3):
        ctx.push(f"[A] m{i}")
    lines = ctx.snapshot()
    assert len(lines) == CONTEXT_LINES
    assert "[A] m0" not in lines
    assert f"[A] m{CONTEXT_LINES + 2}" in lines


def test_concurrent_pushes_do_not_lose_lines():
    # reader 執行緒、翻譯 worker、發話執行緒共用同一份，併發 push 不得掉行
    ctx = ChatContext(max_lines=1000)
    start = threading.Event()

    def worker(tag):
        start.wait()
        for i in range(100):
            ctx.push(f"[{tag}] {i}")

    threads = [threading.Thread(target=worker, args=(t,)) for t in "ABCD"]
    for t in threads:
        t.start()
    start.set()
    for t in threads:
        t.join()
    assert len(ctx.snapshot()) == 400
```

- [ ] **Step 2: Run test to verify it fails**

Run: `uv run pytest tests/test_context.py -v`
Expected: FAIL —「ModuleNotFoundError: No module named 'src.context'」

- [ ] **Step 3: Write minimal implementation**

`src/context.py`：

```python
"""收訊與發話共用的近期聊天原文行緩衝。

上下文由 reader 的「讀取順序」推進，不是由「翻譯成功」推進——平行翻譯時
同一批訊息才看得到彼此（提示詞用的是原文，不需要等前面幾則翻完）。
reader 執行緒、翻譯 worker、發話執行緒共用同一份，故所有存取持鎖。
"""
import threading
from collections import deque

# 帶進提示詞的近期對話行數：短窗涵蓋眼前的對話線，避免遠處舊話題污染判斷。
CONTEXT_LINES = 8


class ChatContext:
    """近期聊天原文行的環狀緩衝，執行緒安全。"""

    def __init__(self, max_lines: int = CONTEXT_LINES):
        self._lines: deque[str] = deque(maxlen=max_lines)
        self._lock = threading.Lock()

    def push(self, line: str) -> None:
        """把一行原文加入上下文（超出容量時擠掉最舊的）。"""
        with self._lock:
            self._lines.append(line)

    def snapshot(self) -> list[str]:
        """取得目前上下文的複本。呼叫端可長期持有：後續 push 不會影響已取出的快照。"""
        with self._lock:
            return list(self._lines)
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `uv run pytest tests/test_context.py -v && uv run pytest -q`
Expected: 全數 PASS

- [ ] **Step 5: Commit**

```bash
git add src/context.py tests/test_context.py
git commit -m "feat(context): add thread-safe ChatContext for translation context"
```

---

### Task 2: Translator 改吃明確 context 參數

**Files:**
- Modify: `src/translator.py`（移除 `_history`，兩個 translate 方法加 `context` 參數，移除 `CONTEXT_LINES` 與 `deque` import）
- Modify: `tests/test_translator.py`（改寫依賴內部 history 的測試）

**Interfaces:**
- Consumes: Task 1 的 `src.context.CONTEXT_LINES`（僅測試用到）
- Produces:
  - `Translator.translate_incoming(self, text: str, context: list[str]) -> str`
  - `Translator.translate_outgoing(self, text: str, context: list[str]) -> str`
  - `test_translate(api: dict, target_language: str) -> str`（簽名不變，內部改傳 `[]`）

- [ ] **Step 1: Write the failing test**

在 `tests/test_translator.py`，**刪除** `test_incoming_history_feeds_next_translation`、`test_failed_translation_not_recorded_to_history`、`test_history_caps_at_context_lines`、`test_outgoing_gets_context_but_does_not_record`、`test_truncated_output_not_recorded_to_history` 這五個依賴內部 history 的測試，改為：

```python
def test_incoming_uses_given_context():
    fake = FakeHttpxClient()
    _make(fake).translate_incoming("[B] two", ["[A] one"])
    turns = _turns(fake.last_body)
    assert len(turns) == 3                      # user(背景)+assistant(ack)+user(待翻)
    assert "[A] one" in turns[0]["content"]
    assert turns[-1] == {"role": "user", "content": "[B] two"}


def test_incoming_without_context_is_single_turn():
    fake = FakeHttpxClient()
    _make(fake).translate_incoming("[A] one", [])
    assert _turns(fake.last_body) == [{"role": "user", "content": "[A] one"}]


def test_translator_keeps_no_internal_history():
    # 上下文改由呼叫端（ChatContext）維護：translator 連續翻兩則也不得自行累積
    fake = FakeHttpxClient()
    t = _make(fake)
    t.translate_incoming("[A] one", [])
    t.translate_incoming("[B] two", [])
    assert _turns(fake.last_body) == [{"role": "user", "content": "[B] two"}]
    assert not hasattr(t, "_history")


def test_outgoing_uses_given_context_and_skips_fewshot():
    from src.translator import FEWSHOT_OUTGOING
    fake = FakeHttpxClient()
    _make(fake).translate_outgoing("好啊", ["[A] want to trade?"])
    turns = _turns(fake.last_body)
    assert turns[0] != FEWSHOT_OUTGOING[0]                           # 有上下文就不加範例
    assert any("[A] want to trade?" in m["content"] for m in turns)
    assert turns[-1] == {"role": "user", "content": "好啊"}
```

同時把檔案中其餘呼叫改成兩參數形式（`translate_incoming("[A] hi", [])`、`translate_outgoing("哈囉", [])`）——包含 `test_openai_compat_sends_temperature_zero`、`test_openai_compat_connection_error_maps_to_offline`、`test_openai_compat_auth_or_model_error_maps_to_config_error`、`test_openai_compat_retryable_status_maps_to_offline`、`test_claude_provider_returns_text`、`test_claude_connection_error_maps_to_offline`、`test_claude_status_error_mapping`、`test_openai_compat_sends_max_tokens_by_thinking_mode`、`test_openai_compat_truncated_output_maps_to_bad_output`、`test_openai_compat_missing_finish_reason_is_accepted`、`test_claude_sends_max_tokens`、`test_claude_truncated_output_maps_to_bad_output`、`test_openai_provider_disables_thinking_with_official_param_only`、`test_openai_provider_thinking_on_sends_no_thinking_params`、`test_outgoing_uses_fewshot_when_no_context`。

保留 `test_outgoing_uses_fewshot_when_no_context`，但改為顯式傳空 context：

```python
def test_outgoing_uses_fewshot_when_no_context():
    from src.translator import FEWSHOT_OUTGOING
    fake = FakeHttpxClient()
    _make(fake).translate_outgoing("在嗎", [])   # 無背景上下文：帶 few-shot 強制翻譯模式
    turns = _turns(fake.last_body)
    assert turns[:len(FEWSHOT_OUTGOING)] == FEWSHOT_OUTGOING
    assert turns[-1] == {"role": "user", "content": "在嗎"}
    assert any("提供" in m["content"] for m in FEWSHOT_OUTGOING if m["role"] == "user")
```

刪除 `test_outgoing_skips_fewshot_when_context_present`（已由 `test_outgoing_uses_given_context_and_skips_fewshot` 取代）。

- [ ] **Step 2: Run test to verify it fails**

Run: `uv run pytest tests/test_translator.py -v`
Expected: FAIL —「TypeError: translate_incoming() takes 2 positional arguments but 3 were given」

- [ ] **Step 3: Write minimal implementation**

`src/translator.py` 改三處。

其一，模組頂端移除 `from collections import deque` 與 `CONTEXT_LINES` 常數定義（`CONTEXT_LINES` 已移至 `src/context.py`），並更新模組 docstring 末段：

```python
失敗分三類：TranslatorOffline（可重試）、TranslatorConfigError（等使用者修設定）、
TranslatorBadOutput（譯文被截斷，重試無用、該行應跳過）。
上下文由呼叫端提供（見 src/context.py）：本類別不持有狀態，可安全平行呼叫。
```

其二，`Translator.__init__` 刪掉 `self._history` 那一行（含其上方註解）。

其三，兩個 translate 方法：

```python
    def translate_incoming(self, text: str, context: list[str]) -> str:
        """收訊：把遊戲聊天（任何語言）翻成使用者設定的目標語言。
        context 為該行之前的原文行，由呼叫端依讀取順序維護（見 ChatContext）。"""
        return self._impl.chat(
            build_incoming_system(self._target_language),
            build_turns(context, text, CONTEXT_INTRO_INCOMING))

    def translate_outgoing(self, text: str, context: list[str]) -> str:
        """發話：把玩家輸入（任何語言）翻成遊戲聊天語言（固定）。
        發話內容不寫入上下文——送出後遊戲會回顯成聊天行，由收訊路徑記錄。
        few-shot 只在無背景上下文時帶：有上下文時多輪結構已足夠，避免範例與
        背景 turn 交錯干擾弱模型。"""
        return self._impl.chat(
            build_outgoing_system(OUTGOING_LANGUAGE),
            build_turns(context, text, CONTEXT_INTRO_OUTGOING,
                        examples=None if context else FEWSHOT_OUTGOING))
```

其四，檔尾 `test_translate` 內改為：

```python
    return Translator(**api, target_language=target_language).translate_incoming(TEST_SAMPLE, [])
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `uv run pytest tests/test_translator.py -v`
Expected: PASS（`tests/test_reader_loop.py` 與 `tests/test_input_box.py` 此時可能失敗，Task 7 會修好；若要確認範圍，跑 `uv run pytest tests/test_translator.py tests/test_context.py -q`）

- [ ] **Step 5: Commit**

```bash
git add src/translator.py tests/test_translator.py
git commit -m "refactor(translator): take context as an explicit parameter"
```

---

### Task 3: Overlay 佔位與就地填入譯文

**Files:**
- Modify: `src/reader/overlay.py`（`_messages` 元素改為 NamedTuple、`add_message` 收 `msg_id`、新增 `update_message`）
- Test: `tests/test_overlay.py`

**Interfaces:**
- Consumes: 無
- Produces:
  - `OverlayWindow.add_message(self, original: str, translated: str, now: float | None = None, msg_id: int | None = None) -> None`
  - `OverlayWindow.update_message(self, msg_id: int, translated: str) -> None`

- [ ] **Step 1: Write the failing test**

追加到 `tests/test_overlay.py`：

```python
def test_update_message_fills_translation_in_place(root):
    # 佔位：訊息一讀到就先顯示原文，譯文稍後填入同一個位置（順序不因翻譯先後而變）
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ov.add_message("[A] one", "翻譯中…", msg_id=1)
    ov.add_message("[B] two", "翻譯中…", msg_id=2)
    ov.update_message(2, "乙")          # 後到的先翻完
    ov.update_message(1, "甲")
    assert ov.visible_messages() == [("[A] one", "甲"), ("[B] two", "乙")]


def test_update_message_ignores_unknown_id(root):
    # 佔位訊息可能已被 max_messages 擠掉或被 prune 清除：晚到的譯文安靜忽略，不得拋錯
    ov = OverlayWindow(root, x=0, y=0, width=460, height=300, max_messages=1, fade_seconds=0)
    ov.add_message("[A] one", "翻譯中…", msg_id=1)
    ov.add_message("[B] two", "翻譯中…", msg_id=2)   # 擠掉 msg_id=1
    ov.update_message(1, "甲")
    assert ov.visible_messages() == [("[B] two", "翻譯中…")]
```

- [ ] **Step 2: Run test to verify it fails**

Run: `uv run pytest tests/test_overlay.py -k update_message -v`
Expected: FAIL —「TypeError: add_message() got an unexpected keyword argument 'msg_id'」

- [ ] **Step 3: Write minimal implementation**

`src/reader/overlay.py`：

其一，模組 import 區加入 `from typing import NamedTuple`，並在 `MIN_WIDTH` 等常數附近新增：

```python
class _Message(NamedTuple):
    """overlay 中的一則訊息。msg_id 為 None 代表不需要就地更新（例如測試直接塞完成品）。"""
    ts: float
    original: str
    translated: str
    row: "tk.Frame"
    msg_id: int | None
```

其二，`__init__` 的型別註記改為：

```python
        self._messages: list[_Message] = []
```

其三，`add_message` 的 append 與其後的擠出改為：

```python
        self._messages.append(_Message(now if now is not None else time.time(),
                                       original, translated, row, msg_id))
        while len(self._messages) > self._max:
            self._messages.pop(0).row.destroy()
```

其四，`set_limits` 內的擠出同樣改為 `self._messages.pop(0).row.destroy()`。

其五，`prune` 改為：

```python
        keep = []
        for entry in self._messages:
            if entry.ts <= cutoff:
                entry.row.destroy()
            else:
                keep.append(entry)
        self._messages = keep
```

其六，`_apply_wrap` 內的迴圈改為 `for entry in self._messages:` 並以 `entry.row` 取代 `row`。

其七，`visible_messages` 改為：

```python
        return [(m.original, m.translated) for m in self._messages]
```

其八，在 `add_message` 之後新增：

```python
    def update_message(self, msg_id: int, translated: str) -> None:
        """把某則佔位訊息的譯文就地填入（原文與位置不動）。
        找不到 msg_id 代表該則已被 prune 或 max_messages 擠掉，安靜忽略。"""
        for i, m in enumerate(self._messages):
            if m.msg_id != msg_id:
                continue
            stick = should_stick_to_bottom(self._canvas.yview()[1])
            line = m.row.winfo_children()[1]  # 0＝原文行，1＝譯文行
            line.itemconfigure("txt", text=translated)
            _fit_line_height(line)
            self._messages[i] = m._replace(translated=translated)
            self._canvas.update_idletasks()
            self._canvas.configure(scrollregion=self._canvas.bbox("all"))
            if stick:
                self._canvas.yview_moveto(1.0)
            return
```

其九，`add_message` 的簽名改為：

```python
    def add_message(self, original: str, translated: str, now: float | None = None,
                    msg_id: int | None = None) -> None:
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `uv run pytest tests/test_overlay.py -v`
Expected: 全數 PASS（既有測試不傳 `msg_id`，預設 `None` 不影響）

- [ ] **Step 5: Commit**

```bash
git add src/reader/overlay.py tests/test_overlay.py
git commit -m "feat(overlay): support placeholder messages filled in later"
```

---

### Task 4: TranslationPool 派發與結果回報

**Files:**
- Create: `src/translation_pool.py`
- Test: `tests/test_translation_pool.py`

**Interfaces:**
- Consumes: Task 2 的 `Translator.translate_incoming(text, context)`
- Produces:
  - `BACKOFF_STEPS: list[int] = [5, 15, 30]`、`CONFIG_ERROR_INTERVAL: float = 15.0`
  - `class TranslationPool: __init__(self, translator, on_result, workers: int, failed_notice: str)`
  - `submit(self, line: str, context: list[str], msg_id: int) -> None`
  - `in_flight` property `-> int`、`error_state` property `-> str | None`
  - `resize(self, workers: int) -> None`、`shutdown(self, wait: bool = False) -> None`

本 task 只做「派發、回報結果、in_flight」；退避與重試在 Task 5。

- [ ] **Step 1: Write the failing test**

`tests/test_translation_pool.py`：

```python
"""TranslationPool：平行翻譯、單則卡住不擋後續、失敗退避與重試。"""
import threading

from src.translation_pool import TranslationPool

FAILED = "⚠  這則訊息翻譯不出來"


class Collector:
    """收集 on_result 回報，供測試等待與斷言。"""

    def __init__(self):
        self.results: dict[int, str] = {}
        self._lock = threading.Lock()
        self._event = threading.Event()

    def __call__(self, msg_id, text):
        with self._lock:
            self.results[msg_id] = text
        self._event.set()

    def wait_for(self, count, timeout=5.0):
        """等到收滿 count 則結果並回傳；逾時即斷言失敗（不靠 sleep 猜時間）。"""
        while True:
            with self._lock:
                if len(self.results) >= count:
                    return dict(self.results)
            if not self._event.wait(timeout):
                with self._lock:
                    raise AssertionError(f"只收到 {len(self.results)} 則，預期 {count}")
            self._event.clear()


class OkTranslator:
    def translate_incoming(self, text, context):
        return f"譯:{text}|ctx={len(context)}"


def _pool(translator, collector, workers=4):
    return TranslationPool(translator=translator, on_result=collector,
                           workers=workers, failed_notice=FAILED)


def test_submitted_lines_are_translated_and_reported():
    c = Collector()
    pool = _pool(OkTranslator(), c)
    try:
        for i in range(4):
            pool.submit(f"[A] m{i}", ["[Z] ctx"], msg_id=i)
        assert c.wait_for(4) == {i: f"譯:[A] m{i}|ctx=1" for i in range(4)}
    finally:
        pool.shutdown(wait=True)


def test_blocked_line_does_not_block_the_others():
    # 需求核心：一則卡住時，其餘各則仍須照常完成
    release = threading.Event()

    class BlockingTranslator:
        def translate_incoming(self, text, context):
            if text == "[A] stuck":
                release.wait(5.0)
            return f"譯:{text}"

    c = Collector()
    pool = _pool(BlockingTranslator(), c, workers=4)
    try:
        pool.submit("[A] stuck", [], msg_id=0)
        for i in range(1, 4):
            pool.submit(f"[A] m{i}", [], msg_id=i)
        got = c.wait_for(3)                       # 卡住那則還沒回來，其餘三則已完成
        assert set(got) == {1, 2, 3}
        release.set()
        assert c.wait_for(4)[0] == "譯:[A] stuck"
    finally:
        release.set()
        pool.shutdown(wait=True)


def test_in_flight_counts_outstanding_work():
    release = threading.Event()
    started = threading.Event()

    class SlowTranslator:
        def translate_incoming(self, text, context):
            started.set()
            release.wait(5.0)
            return "譯"

    c = Collector()
    pool = _pool(SlowTranslator(), c, workers=1)
    try:
        assert pool.in_flight == 0
        pool.submit("[A] one", [], msg_id=1)
        assert started.wait(5.0)
        assert pool.in_flight == 1
        release.set()
        c.wait_for(1)
        assert pool.in_flight == 0
    finally:
        release.set()
        pool.shutdown(wait=True)


def test_resize_keeps_pool_usable():
    c = Collector()
    pool = _pool(OkTranslator(), c, workers=2)
    try:
        pool.submit("[A] before", [], msg_id=1)
        c.wait_for(1)
        pool.resize(4)
        pool.submit("[A] after", [], msg_id=2)
        assert c.wait_for(2)[2] == "譯:[A] after|ctx=0"
    finally:
        pool.shutdown(wait=True)
```

- [ ] **Step 2: Run test to verify it fails**

Run: `uv run pytest tests/test_translation_pool.py -v`
Expected: FAIL —「ModuleNotFoundError: No module named 'src.translation_pool'」

- [ ] **Step 3: Write minimal implementation**

`src/translation_pool.py`：

```python
"""收訊翻譯工作池：以固定數量的 worker 平行翻譯，單則卡住不影響其他則。

呼叫端只負責提交（line, context, msg_id）與接收 on_result(msg_id, text)；
顯示順序不由完成順序決定——overlay 在提交當下就已佔好位置（見 reader_loop）。
"""
import sys
import threading
from concurrent.futures import ThreadPoolExecutor

BACKOFF_STEPS = [5, 15, 30]   # 翻譯伺服器離線時的重試間隔（秒）
CONFIG_ERROR_INTERVAL = 15.0  # API 設定錯誤時的重試間隔（秒）；使用者修正後自動恢復


class TranslationPool:
    """平行收訊翻譯。`on_result(msg_id, text)` 於 worker 執行緒呼叫，
    呼叫端負責把它轉交回 UI 執行緒。"""

    def __init__(self, translator, on_result, workers: int, failed_notice: str):
        self._translator = translator
        self._on_result = on_result
        self._failed_notice = failed_notice
        self._workers = workers
        self._lock = threading.Lock()
        self._stop = threading.Event()
        self._in_flight = 0
        self._executor = ThreadPoolExecutor(max_workers=workers,
                                            thread_name_prefix="translate")

    @property
    def in_flight(self) -> int:
        """已提交但尚未回報結果的則數（供狀態指示判斷是否顯示『翻譯中…』）。"""
        with self._lock:
            return self._in_flight

    @property
    def error_state(self) -> str | None:
        """目前的翻譯錯誤狀態：None／"offline"／"config"（Task 5 起有實質值）。"""
        return None

    def submit(self, line: str, context: list[str], msg_id: int) -> None:
        """提交一則翻譯。context 為提交當下的快照，重試時沿用同一份、不隨後續訊息漂移。"""
        if self._stop.is_set():
            return
        with self._lock:
            self._in_flight += 1
            executor = self._executor
        executor.submit(self._work, line, context, msg_id)

    def resize(self, workers: int) -> None:
        """變更平行度。舊 executor 放生（手上的工作跑完仍會經 on_result 回報，
        msg_id 不受影響），本物件身分不變——持有本 pool 參考的呼叫端不需更新。"""
        with self._lock:
            if workers == self._workers:
                return
            old = self._executor
            self._workers = workers
            self._executor = ThreadPoolExecutor(max_workers=workers,
                                                thread_name_prefix="translate")
        old.shutdown(wait=False)
        print(f"[translate] pool resized to {workers} workers", file=sys.stderr)

    def shutdown(self, wait: bool = False) -> None:
        """停止接受新工作並要求 worker 盡快收手。"""
        self._stop.set()
        with self._lock:
            executor = self._executor
        executor.shutdown(wait=wait, cancel_futures=True)

    def _work(self, line: str, context: list[str], msg_id: int) -> None:
        try:
            translated = self._translator.translate_incoming(line, context)
        except Exception as exc:
            print(f"[translate] line dropped, unexpected error ({exc}): {line}",
                  file=sys.stderr)
            self._on_result(msg_id, self._failed_notice)
        else:
            self._on_result(msg_id, translated)
        finally:
            with self._lock:
                self._in_flight -= 1
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `uv run pytest tests/test_translation_pool.py -v`
Expected: 全數 PASS

- [ ] **Step 5: Commit**

```bash
git add src/translation_pool.py tests/test_translation_pool.py
git commit -m "feat(pool): add translation worker pool with result callback"
```

---

### Task 5: TranslationPool 退避閘門與重試

**Files:**
- Modify: `src/translation_pool.py`
- Test: `tests/test_translation_pool.py`

**Interfaces:**
- Consumes: Task 4 的 `TranslationPool`
- Produces: `error_state` 回傳 `None` / `"offline"` / `"config"`；offline 與 config error 無限重試、由全域閘門節流

- [ ] **Step 1: Write the failing test**

追加到 `tests/test_translation_pool.py`（檔頭 import 補上 `import pytest` 與三個例外）：

```python
import pytest

from src.translator import TranslatorBadOutput, TranslatorConfigError, TranslatorOffline


@pytest.fixture(autouse=True)
def fast_backoff(monkeypatch):
    """把退避縮到毫秒級：測的是「有沒有退避與重置」，不是真的等 5 秒。"""
    import src.translation_pool as pool_module
    monkeypatch.setattr(pool_module, "BACKOFF_STEPS", [0.01, 0.01, 0.01])
    monkeypatch.setattr(pool_module, "CONFIG_ERROR_INTERVAL", 0.01)


class FailThenOk:
    """前 n 次拋出指定例外，之後成功。"""

    def __init__(self, exc, failures):
        self._exc = exc
        self._left = failures
        self._lock = threading.Lock()
        self.calls = 0

    def translate_incoming(self, text, context):
        with self._lock:
            self.calls += 1
            fail = self._left > 0
            if fail:
                self._left -= 1
        if fail:
            raise self._exc
        return f"譯:{text}"


def test_offline_retries_until_success_and_clears_error_state():
    tr = FailThenOk(TranslatorOffline("down"), failures=2)
    c = Collector()
    pool = _pool(tr, c, workers=1)
    try:
        pool.submit("[A] one", [], msg_id=1)
        assert c.wait_for(1)[1] == "譯:[A] one"
        assert tr.calls == 3                  # 兩次失敗 + 一次成功
        assert pool.error_state is None       # 成功後狀態清除
    finally:
        pool.shutdown(wait=True)


def test_offline_sets_error_state_while_failing():
    blocked = threading.Event()

    class AlwaysOffline:
        def translate_incoming(self, text, context):
            blocked.set()
            raise TranslatorOffline("down")

    c = Collector()
    pool = _pool(AlwaysOffline(), c, workers=1)
    try:
        pool.submit("[A] one", [], msg_id=1)
        assert blocked.wait(5.0)
        deadline = time.monotonic() + 5.0
        while pool.error_state != "offline" and time.monotonic() < deadline:
            time.sleep(0.01)
        assert pool.error_state == "offline"
    finally:
        pool.shutdown(wait=True)


def test_config_error_retries_and_reports_config_state():
    tr = FailThenOk(TranslatorConfigError("bad key", status=401), failures=1)
    c = Collector()
    pool = _pool(tr, c, workers=1)
    try:
        pool.submit("[A] one", [], msg_id=1)
        assert c.wait_for(1)[1] == "譯:[A] one"
        assert tr.calls == 2
        assert pool.error_state is None
    finally:
        pool.shutdown(wait=True)


def test_bad_output_is_not_retried():
    # temperature=0 下重試必得同一結果：截斷一律放棄，直接回失敗提示
    tr = FailThenOk(TranslatorBadOutput("truncated"), failures=99)
    c = Collector()
    pool = _pool(tr, c, workers=1)
    try:
        pool.submit("[A] one", [], msg_id=1)
        assert c.wait_for(1)[1] == FAILED
        assert tr.calls == 1
    finally:
        pool.shutdown(wait=True)


def test_backoff_gate_is_shared_across_workers():
    # 伺服器離線是全域事實：四個 worker 不得以四倍速重打
    hits = []
    lock = threading.Lock()

    class CountingOffline:
        def translate_incoming(self, text, context):
            with lock:
                hits.append(time.monotonic())
            raise TranslatorOffline("down")

    c = Collector()
    pool = _pool(CountingOffline(), c, workers=4)
    try:
        for i in range(4):
            pool.submit(f"[A] m{i}", [], msg_id=i)
        deadline = time.monotonic() + 2.0
        while time.monotonic() < deadline:
            with lock:
                if len(hits) >= 8:
                    break
            time.sleep(0.01)
        with lock:
            assert len(hits) >= 8       # 有持續重試
    finally:
        pool.shutdown(wait=True)
```

檔頭再補 `import time`。

- [ ] **Step 2: Run test to verify it fails**

Run: `uv run pytest tests/test_translation_pool.py -v`
Expected: FAIL — `test_offline_retries_until_success_and_clears_error_state` 得到 `FAILED` 而非譯文（目前任何例外都直接放棄）

- [ ] **Step 3: Write minimal implementation**

`src/translation_pool.py`：`import time`，並把 `__init__` 的狀態欄位、`error_state`、`_work` 換成下列版本，另新增三個私有方法。

`__init__` 內 `self._in_flight = 0` 之後補：

```python
        self._error_state: str | None = None
        self._backoff_index = 0
        self._gate_until = 0.0   # time.monotonic() 之前不得送出新請求
```

`error_state` 改為：

```python
    @property
    def error_state(self) -> str | None:
        """目前的翻譯錯誤狀態：None／"offline"／"config"，供上層決定錯誤橫幅。"""
        with self._lock:
            return self._error_state
```

`_work` 改為：

```python
    def _work(self, line: str, context: list[str], msg_id: int) -> None:
        attempts = 0
        try:
            while not self._stop.is_set():
                if not self._wait_for_gate():
                    return          # 關閉中：放棄這則，不回報
                attempts += 1
                try:
                    translated = self._translator.translate_incoming(line, context)
                except TranslatorOffline as exc:
                    self._note_failure("offline", exc)
                    continue        # 該則留著重試，伺服器恢復就補上
                except TranslatorConfigError as exc:
                    self._note_failure("config", exc)
                    continue        # 等使用者修正設定後自動恢復
                except Exception as exc:
                    # 譯文被截斷或回傳格式異常：temperature=0 下重試必得同一結果，
                    # 直接放棄該則，讓 overlay 至少顯示原文而不是無聲消失。
                    reason = ("bad model output" if isinstance(exc, TranslatorBadOutput)
                              else "unexpected error")
                    print(f"[translate] line dropped after {attempts} attempt(s), "
                          f"{reason} ({exc}): {line}", file=sys.stderr)
                    self._on_result(msg_id, self._failed_notice)
                    return
                self._note_success()
                self._on_result(msg_id, translated)
                return
        finally:
            with self._lock:
                self._in_flight -= 1
```

新增：

```python
    def _wait_for_gate(self) -> bool:
        """等到退避閘門開啟；關閉中回傳 False。分段等待讓 shutdown 能及時打斷。"""
        while not self._stop.is_set():
            with self._lock:
                remaining = self._gate_until - time.monotonic()
            if remaining <= 0:
                return True
            self._stop.wait(min(remaining, 0.5))
        return False

    def _note_failure(self, state: str, exc: Exception) -> None:
        """推進全域退避閘門並記錄狀態。閘門是全域的——伺服器離線本就是全域事實，
        否則 N 個 worker 會以 N 倍速重打同一台掛掉的伺服器。"""
        with self._lock:
            if state == "config":
                delay = CONFIG_ERROR_INTERVAL
            else:
                steps = BACKOFF_STEPS
                delay = steps[min(self._backoff_index, len(steps) - 1)]
                self._backoff_index += 1
            self._gate_until = time.monotonic() + delay
            changed = self._error_state != state
            self._error_state = state
        if changed:
            print(f"[translate] provider {state} error: {exc}; "
                  f"retrying with backoff", file=sys.stderr)

    def _note_success(self) -> None:
        """任何一次成功都代表伺服器與設定已恢復：清狀態、重置退避、放開閘門。"""
        with self._lock:
            changed = self._error_state is not None
            self._error_state = None
            self._backoff_index = 0
            self._gate_until = 0.0
        if changed:
            print("[translate] recovered, resuming normal speed", file=sys.stderr)
```

檔頭 import 補上：

```python
from src.translator import TranslatorBadOutput, TranslatorConfigError, TranslatorOffline
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `uv run pytest tests/test_translation_pool.py -v`
Expected: 全數 PASS

- [ ] **Step 5: Commit**

```bash
git add src/translation_pool.py tests/test_translation_pool.py
git commit -m "feat(pool): add shared backoff gate and per-line retry"
```

---

### Task 6: 平行度設定

**Files:**
- Modify: `src/config.py`、`src/ui/settings.py`、`config.example.json`
- Test: `tests/test_config.py`、`tests/test_settings.py`

**Interfaces:**
- Consumes: 無
- Produces: `cfg["max_parallel_translations"]`（int，1–8，預設 4）；`parse_advanced_values` 多收一個 Tk 變數

- [ ] **Step 1: Write the failing test**

追加到 `tests/test_config.py`：

```python
def test_parallel_translations_defaults_and_clamps(tmp_path):
    from src.config import DEFAULT_CONFIG, clamp_advanced, load_config, save_config
    assert DEFAULT_CONFIG["max_parallel_translations"] == 4
    assert clamp_advanced({"max_parallel_translations": 99, **_others()})[
        "max_parallel_translations"] == 8
    assert clamp_advanced({"max_parallel_translations": 0, **_others()})[
        "max_parallel_translations"] == 1
    path = tmp_path / "config.json"
    save_config(path, {"max_parallel_translations": 50})
    assert load_config(path)["max_parallel_translations"] == 8


def _others():
    """clamp_advanced 會遍歷所有 ADVANCED_LIMITS 的鍵，補齊其餘欄位避免 KeyError。"""
    from src.config import DEFAULT_CONFIG
    return {k: DEFAULT_CONFIG[k] for k in
            ("poll_interval", "fade_seconds", "max_messages", "type_delay", "overlay_alpha")}
```

`tests/test_settings.py` 已有 `_vars(root, ...)` helper 回傳 5 個 Tk 變數，並被
`test_parse_advanced_values_returns_error_on_non_numeric_input` 與
`test_parse_advanced_values_clamps_valid_numeric_input` 使用。把 helper 擴充成 6 個：

```python
def _vars(root, poll=0.4, fade=0, max_msgs=200, type_delay=0.02, alpha=0.84, parallel=4):
    return (tk.DoubleVar(root, value=poll), tk.IntVar(root, value=fade),
            tk.IntVar(root, value=max_msgs), tk.DoubleVar(root, value=type_delay),
            tk.DoubleVar(root, value=alpha), tk.IntVar(root, value=parallel))
```

兩個既有測試的解包與呼叫同步改為 6 個變數（例如
`poll, fade, max_msgs, type_delay, alpha, parallel = _vars(root)`、
`parse_advanced_values(poll, fade, max_msgs, type_delay, alpha, parallel)`），
並追加：

```python
def test_parse_advanced_values_clamps_parallel(root):
    poll, fade, max_msgs, type_delay, alpha, parallel = _vars(root, parallel=99)
    values, error = parse_advanced_values(poll, fade, max_msgs, type_delay, alpha, parallel)
    assert error is None
    assert values["max_parallel_translations"] == 8
```

- [ ] **Step 2: Run test to verify it fails**

Run: `uv run pytest tests/test_config.py tests/test_settings.py -v`
Expected: FAIL —「KeyError: 'max_parallel_translations'」

- [ ] **Step 3: Write minimal implementation**

`src/config.py`：`DEFAULT_CONFIG` 在 `max_messages` 之後插入

```python
    # 同時進行的收訊翻譯則數。實測 4 併發後幾乎無額外收益，只讓單則延遲更差；
    # 設 1 等同逐則排隊（本功能之前的行為）。
    "max_parallel_translations": 4,
```

`ADVANCED_LIMITS` 加入

```python
    "max_parallel_translations": (1, 8),
```

`src/ui/settings.py`：

`parse_advanced_values` 簽名改為

```python
def parse_advanced_values(poll_var, fade_var, max_messages_var, type_delay_var,
                          alpha_var, parallel_var) -> tuple[dict | None, str | None]:
```

其 docstring 的「四個進階數值」改為「進階數值」，字典中加入

```python
            "max_parallel_translations": int(parallel_var.get()),
```

進階分頁在 `self._max_msgs` 之後加：

```python
        self._parallel = self._spin(adv, "同時翻譯則數", cfg["max_parallel_translations"],
                                    "max_parallel_translations", 1,
                                    "1＝逐則排隊；大於 1 時卡住的訊息不會擋住後續")
```

`_save` 內的呼叫改為

```python
        advanced, advanced_error = parse_advanced_values(
            self._poll, self._fade, self._max_msgs, self._type_delay, self._alpha_var,
            self._parallel)
```

`config.example.json` 在 `max_messages` 之後補 `"max_parallel_translations": 4,`。

- [ ] **Step 4: Run tests to verify they pass**

Run: `uv run pytest tests/test_config.py tests/test_settings.py -v`
Expected: 全數 PASS

- [ ] **Step 5: Commit**

```bash
git add src/config.py src/ui/settings.py config.example.json tests/test_config.py tests/test_settings.py
git commit -m "feat(config): add max_parallel_translations setting"
```

---

### Task 7: reader_loop 改為派發，main 接線整合

**Files:**
- Modify: `src/main.py`（`reader_loop` 重寫、常數整理、`main()` 接線、`apply_settings`、關閉流程）
- Modify: `src/composer/input_box.py`（無需改動程式碼，但 `translate_fn` 的呼叫方改傳 context——確認 `_worker` 仍是 `self._translate(text)` 單參數呼叫）
- Test: `tests/test_reader_loop.py`（大幅改寫）

**Interfaces:**
- Consumes: Task 1 `ChatContext`、Task 3 `overlay.add_message(..., msg_id=)` 與 `update_message`、Task 4/5 `TranslationPool`、Task 6 `cfg["max_parallel_translations"]`
- Produces:
  - `PENDING_NOTICE: str = "翻譯中…"`、`GAME_MISSING_NOTICE`、`OFFLINE_NOTICE`、`CONFIG_ERROR_NOTICE`
  - `banner_for(game_missing: bool, error_state: str | None) -> str | None`
  - `reader_loop(cfg, overlay, ui_queue, stop, context, pool, on_input_open=None, on_input_close=None) -> None`

- [ ] **Step 1: Write the failing test**

把 `tests/test_reader_loop.py` 中依賴「reader_loop 自己翻譯」的測試（`test_new_lines_translated_in_order_including_repeats`、`test_non_http_error_skips_line_and_keeps_going`、`test_failed_line_stays_pending_and_retried`、`test_config_error_line_stays_pending_and_retried`、`test_truncated_line_is_dropped_and_does_not_block_queue`、`OkTranslator`／`OneBadTranslator`／`FlakyTranslator`／`ConfigErrorTranslator`／`TruncatingTranslator`／`NeverTranslator`）整批刪除，改為下列內容；`run_scripted` 與 `_run_with_input` 的簽名同步調整：

```python
"""reader_loop 行為：WizChatReader.read_new() → 推進 context → overlay 佔位 → 提交 pool。
翻譯本身與其重試改由 TranslationPool 負責（見 test_translation_pool.py）。"""
import queue
import threading

import src.main as main_module
from src.context import ChatContext
from src.main import PENDING_NOTICE, banner_for, reader_loop
from src.reader.mem_reader import GameNotRunning


class FakePool:
    """記錄提交內容；error_state／in_flight 由測試直接設定。"""

    def __init__(self):
        self.submitted: list[tuple[str, list[str], int]] = []
        self.error_state: str | None = None
        self.in_flight = 0

    def submit(self, line, context, msg_id):
        self.submitted.append((line, list(context), msg_id))


def run_scripted(cfg, overlay, reads, monkeypatch, pool=None, context=None):
    ui_queue: queue.Queue = queue.Queue()
    stop = threading.Event()
    pool = pool or FakePool()
    context = context or ChatContext()
    monkeypatch.setattr(main_module, "WizChatReader",
                        lambda **kw: FakeReader(reads, stop))
    reader_loop(cfg, overlay, ui_queue, stop, context, pool)
    _drain(ui_queue)
    return pool, context


def test_new_lines_are_placeheld_and_submitted_in_order(monkeypatch):
    cfg = {"poll_interval": 0.01}
    ov = FakeOverlay()
    reads = [[], ["[A] a"], ["[B] hi", "[B] hi"], []]
    pool, _ = run_scripted(cfg, ov, reads, monkeypatch)
    # overlay 依讀取順序先佔位（譯文位置為「翻譯中…」），順序不由翻譯完成先後決定
    assert ov.messages == [("[A] a", PENDING_NOTICE), ("[B] hi", PENDING_NOTICE),
                           ("[B] hi", PENDING_NOTICE)]
    assert [line for line, _, _ in pool.submitted] == ["[A] a", "[B] hi", "[B] hi"]
    assert [msg_id for _, _, msg_id in pool.submitted] == [1, 2, 3]


def test_context_advances_by_read_order_not_by_completion(monkeypatch):
    # 每則帶到的 context 是它「之前」的行——平行翻譯時同批訊息仍看得到彼此
    cfg = {"poll_interval": 0.01}
    reads = [["[A] one", "[B] two", "[C] three"], []]
    pool, context = run_scripted(cfg, FakeOverlay(), reads, monkeypatch)
    assert [ctx for _, ctx, _ in pool.submitted] == [
        [], ["[A] one"], ["[A] one", "[B] two"]]
    assert context.snapshot() == ["[A] one", "[B] two", "[C] three"]


def test_banner_follows_pool_error_state(monkeypatch):
    cfg = {"poll_interval": 0.01}
    ov = FakeOverlay()
    pool = FakePool()
    pool.error_state = "offline"
    run_scripted(cfg, ov, [[], []], monkeypatch, pool=pool)
    assert ov.errors == [main_module.OFFLINE_NOTICE]


def test_banner_prefers_game_missing_over_translation_error():
    # 連不上遊戲時翻譯狀態已無意義，橫幅顯示遊戲未就緒
    assert banner_for(True, "offline") == main_module.GAME_MISSING_NOTICE
    assert banner_for(False, "config") == main_module.CONFIG_ERROR_NOTICE
    assert banner_for(False, "offline") == main_module.OFFLINE_NOTICE
    assert banner_for(False, None) is None


def test_status_shows_translating_while_pool_busy(monkeypatch):
    cfg = {"poll_interval": 0.01}
    ov = FakeOverlay()
    pool = FakePool()
    pool.in_flight = 2
    run_scripted(cfg, ov, [[], []], monkeypatch, pool=pool)
    assert "●  翻譯中…" in ov.statuses
```

`FakeOverlay` 需新增 `update_message`，並讓 `add_message` 接受 `msg_id`：

```python
class FakeOverlay:
    def __init__(self):
        self.messages: list[tuple[str, str]] = []
        self.errors: list[str] = []
        self.clears = 0
        self.statuses: list[str] = []

    def add_message(self, original, translated, msg_id=None):
        self.messages.append((original, translated))

    def update_message(self, msg_id, translated):
        pass

    def set_error(self, text):
        self.errors.append(text)

    def clear_error(self):
        self.clears += 1

    def set_status(self, text, color=None):
        self.statuses.append(text)
```

`FakeReader`、`InputFakeReader`、`_drain` 三個 helper **原封保留**（它們與翻譯無關）。
`test_game_not_running_shows_banner_once`、`test_status_locating_when_not_anchored`、`test_game_input_edge_triggers_open_and_close`、`test_game_input_detection_disabled_by_config` 保留，但呼叫改為新簽名（不再傳 translator，改傳 `context` 與 `pool`）。其中 `test_game_not_running_shows_banner_once` 的斷言改為 `assert ov.errors == [main_module.GAME_MISSING_NOTICE]`，`test_status_locating_when_not_anchored` 的 `reader_loop(...)` 呼叫改為 `reader_loop(cfg, ov, ui_queue, stop, ChatContext(), FakePool())`。`_run_with_input` 改為：

```python
def _run_with_input(cfg, reads, input_states, monkeypatch):
    events = []
    ui_queue: queue.Queue = queue.Queue()
    stop = threading.Event()
    monkeypatch.setattr(main_module, "WizChatReader",
                        lambda **kw: InputFakeReader(reads, stop, input_states))
    reader_loop(cfg, FakeOverlay(), ui_queue, stop, ChatContext(), FakePool(),
                on_input_open=lambda: events.append("open"),
                on_input_close=lambda: events.append("close"))
    _drain(ui_queue)
    return events
```

- [ ] **Step 2: Run test to verify it fails**

Run: `uv run pytest tests/test_reader_loop.py -v`
Expected: FAIL —「ImportError: cannot import name 'PENDING_NOTICE' from 'src.main'」

- [ ] **Step 3: Write minimal implementation**

`src/main.py`：

其一，import 區改為（移除 `TranslatorConfigError`／`TranslatorOffline`／`TranslatorBadOutput`，改引入新元件）：

```python
import itertools
...
from src.context import ChatContext
from src.translation_pool import TranslationPool
from src.translator import Translator
```

其二，常數區把 `BACKOFF_STEPS`、`CONFIG_ERROR_INTERVAL` 刪除（已移入 `translation_pool.py`），保留 `GAME_MISSING_INTERVAL`，並整理成：

```python
GAME_MISSING_INTERVAL = 5.0  # 找不到遊戲時的重試間隔（秒）

PENDING_NOTICE = "翻譯中…"                        # 佔位期間顯示於譯文位置
TRANSLATE_FAILED_NOTICE = "⚠  這則訊息翻譯不出來"   # 放棄該行時代替譯文顯示
GAME_MISSING_NOTICE = "⚠  遊戲未就緒／連線中斷，等待中…"
OFFLINE_NOTICE = "⚠  翻譯伺服器離線，重試中…"
CONFIG_ERROR_NOTICE = "⚠  API 設定有誤，請開啟設定（⚙）檢查"
```

其三，新增純函式：

```python
def banner_for(game_missing: bool, error_state: str | None) -> str | None:
    """依目前狀況決定該顯示哪一條錯誤橫幅（None＝不顯示）。
    遊戲未就緒優先於翻譯錯誤：連不上遊戲時翻譯狀態已無意義。"""
    if game_missing:
        return GAME_MISSING_NOTICE
    if error_state == "config":
        return CONFIG_ERROR_NOTICE
    if error_state == "offline":
        return OFFLINE_NOTICE
    return None
```

其四，`reader_loop` 整個換成：

```python
def reader_loop(cfg: dict, overlay: OverlayWindow, ui_queue: queue.Queue,
                stop: threading.Event, context: ChatContext, pool: TranslationPool,
                on_input_open=None, on_input_close=None) -> None:
    # 讀遊戲聊天記錄 → 依序推進上下文、在 overlay 佔位 → 交給 pool 平行翻譯。
    # 本迴圈不做翻譯，因此單則翻譯卡住不會延誤後續訊息的讀取與顯示。
    reader = WizChatReader(game_path=cfg.get("game_path"))
    msg_ids = itertools.count(1)
    game_missing = False
    game_input_open = False
    last_status: str | None = None
    last_banner: str | None = None

    def set_status(key: str) -> None:
        nonlocal last_status
        if key == last_status:
            return
        last_status = key
        text, color = STATUS[key]
        ui_queue.put(lambda: overlay.set_status(text, color))

    def set_banner(text: str | None) -> None:
        nonlocal last_banner
        if text == last_banner:
            return
        last_banner = text
        if text is None:
            ui_queue.put(overlay.clear_error)
        else:
            ui_queue.put(lambda t=text: overlay.set_error(t))

    while not stop.is_set():
        set_status("listening" if reader.anchored else "locating")
        try:
            new_lines = reader.read_new()
        except GameNotRunning as exc:
            set_status("waiting_game")
            if not game_missing:
                game_missing = True
                print(f"[reader] game not ready: {exc}", file=sys.stderr)
            set_banner(banner_for(game_missing, pool.error_state))
            if game_input_open:
                game_input_open = False  # 遊戲斷線＝輸入框已不存在，同步收回
                if on_input_close is not None:
                    on_input_close()
            stop.wait(GAME_MISSING_INTERVAL)
            continue
        except Exception as exc:  # 收訊偶發錯誤：略過該輪，不讓執行緒死掉
            print(f"[reader] poll skipped: {exc}", file=sys.stderr)
            stop.wait(cfg["poll_interval"])
            continue

        if game_missing:
            game_missing = False
            print("[reader] game back, resuming", file=sys.stderr)

        for line in new_lines:
            ctx = context.snapshot()   # 該行之前的行；提交後即固定，重試不漂移
            context.push(line)
            msg_id = next(msg_ids)
            ui_queue.put(lambda o=line, m=msg_id:
                         overlay.add_message(o, PENDING_NOTICE, msg_id=m))
            pool.submit(line, ctx, msg_id)

        set_banner(banner_for(game_missing, pool.error_state))
        if pool.in_flight:
            set_status("translating")
        else:
            set_status("listening" if reader.anchored else "locating")

        # 遊戲聊天輸入框開／關的邊緣觸發：開 → 呼出翻譯輸入；關 → 收回
        if on_input_open is not None and cfg.get("auto_show_input", True):
            now_open = reader.input_open()
            if now_open != game_input_open:
                game_input_open = now_open
                print(f"[reader] game chat input {'opened' if now_open else 'closed'}",
                      file=sys.stderr)
                if now_open:
                    on_input_open()
                elif on_input_close is not None:
                    on_input_close()

        stop.wait(cfg["poll_interval"])

    reader.close()  # 停止：解除 wizwalker hook、關閉連線
```

其五，`main()` 內的接線。`translator = Translator(...)` 之後改為：

```python
    translator = Translator(**cfg["api"], target_language=cfg["target_language"])
    context = ChatContext()
    ui_queue: queue.Queue = queue.Queue()
```

`overlay = OverlayWindow(...)` 之後、`input_box` 之前插入：

```python
    pool = TranslationPool(
        translator=translator,
        on_result=lambda mid, text: ui_queue.put(
            lambda: overlay.update_message(mid, text)),
        workers=cfg["max_parallel_translations"],
        failed_notice=TRANSLATE_FAILED_NOTICE)
```

`input_box` 的建構改為：

```python
    input_box = InputBox(root, lambda text: translator.translate_outgoing(
        text, context.snapshot()), ui_queue, on_translated,
        position=cfg["input_position"], on_move=save_input_position)
```

`apply_settings` 內 `translator.reconfigure(...)` 之後補：

```python
        pool.resize(cfg["max_parallel_translations"])
```

並在該函式最後的 log 加上平行度：

```python
        print(f"[settings] applied; provider={cfg['api']['provider']}, "
              f"model={cfg['api']['model']}, hotkey={cfg['hotkey']}, "
              f"parallel={cfg['max_parallel_translations']}", file=sys.stderr)
```

執行緒建立改為：

```python
    reader_thread = threading.Thread(
        target=reader_loop, args=(cfg, overlay, ui_queue, stop, context, pool),
        kwargs={"on_input_open": lambda: ui_queue.put(input_box.show),
                "on_input_close": lambda: ui_queue.put(input_box.close)},
        daemon=True)
```

啟動摘要的 log 補上平行度：

```python
    print(f"[app] startup; frozen={getattr(sys, 'frozen', False)}, "
          f"provider={cfg['api']['provider']}, model={cfg['api']['model']}, "
          f"target_language={cfg['target_language']}, hotkey={cfg['hotkey']}, "
          f"poll_interval={cfg['poll_interval']}, "
          f"parallel={cfg['max_parallel_translations']}", file=sys.stderr)
```

`finally` 區塊在 `stop.set()` 之後補：

```python
        pool.shutdown()
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `uv run pytest -q`
Expected: 全數 PASS（含既有的 `test_mem_reader.py` burst guard 測試）

- [ ] **Step 5: Commit**

```bash
git add src/main.py tests/test_reader_loop.py
git commit -m "feat(reader): dispatch translations to the pool instead of translating inline"
```

---

### Task 8: 端到端順序保證與實機驗證

**Files:**
- Test: `tests/test_parallel_order.py`（新增）

**Interfaces:**
- Consumes: Task 1、3、4、5、7 的全部產出
- Produces: 無（純驗證）

- [ ] **Step 1: Write the failing test**

`tests/test_parallel_order.py`：

```python
"""端到端：完成順序與顯示順序脫鉤——後到的訊息先翻完，overlay 仍維持讀取順序。"""
import queue
import threading
import time

from src.context import ChatContext
from src.main import PENDING_NOTICE
from src.reader.overlay import OverlayWindow
from src.translation_pool import TranslationPool

FAILED = "⚠  這則訊息翻譯不出來"


class ReverseOrderTranslator:
    """讓後提交的先完成：第一則等到其餘各則都翻完才放行。"""

    def __init__(self, hold_line: str, others: int):
        self._hold_line = hold_line
        self._others_done = threading.Semaphore(0)
        self._others = others

    def translate_incoming(self, text, context):
        if text == self._hold_line:
            for _ in range(self._others):
                assert self._others_done.acquire(timeout=5.0)
            return f"譯:{text}"
        result = f"譯:{text}"
        self._others_done.release()
        return result


def test_display_order_follows_read_order_not_completion_order(root):
    lines = ["[A] one", "[B] two", "[C] three"]
    overlay = OverlayWindow(root, x=0, y=0, width=460, height=300, fade_seconds=0)
    ui_queue: queue.Queue = queue.Queue()
    context = ChatContext()
    pool = TranslationPool(
        translator=ReverseOrderTranslator("[A] one", others=2),
        on_result=lambda mid, text: ui_queue.put(
            lambda: overlay.update_message(mid, text)),
        workers=3, failed_notice=FAILED)
    try:
        for i, line in enumerate(lines, start=1):
            ctx = context.snapshot()
            context.push(line)
            overlay.add_message(line, PENDING_NOTICE, msg_id=i)
            pool.submit(line, ctx, msg_id=i)
        for _ in range(500):                       # 至多 5 秒，收滿三則就停
            while True:
                try:
                    ui_queue.get_nowait()()
                except queue.Empty:
                    break
            if all(t != PENDING_NOTICE for _, t in overlay.visible_messages()):
                break
            time.sleep(0.01)   # 主執行緒得讓出時間，worker 才有機會回報
        assert overlay.visible_messages() == [
            ("[A] one", "譯:[A] one"),
            ("[B] two", "譯:[B] two"),
            ("[C] three", "譯:[C] three"),
        ]
    finally:
        pool.shutdown(wait=True)
```

- [ ] **Step 2: Run test to verify it fails**

Run: `uv run pytest tests/test_parallel_order.py -v`
Expected: 若前面各 task 都完成，此測試應直接 PASS——它是回歸保護而非驅動實作。若 FAIL，代表佔位或 msg_id 對應有誤，回到 Task 3 或 Task 7 修正。

- [ ] **Step 3: 更新 README 的設定說明**

`README.md` 若有列出 config 欄位表，補上一列：

```
| `max_parallel_translations` | 同時進行的收訊翻譯則數（1–8，預設 4）。1＝逐則排隊；大於 1 時單則卡住不會擋住後續 |
```

若 README 無此表格則跳過本步驟。

- [ ] **Step 4: 全套測試與實機驗證**

Run: `uv run pytest -q`
Expected: 全數 PASS

接著依 `CLAUDE.md` 的提交前品質檢查做實機驗證：開遊戲並**登入進世界內**，`uv run run.py` 跑一輪，確認三件事：

1. 訊息一進來就先出現原文（譯文位置為「翻譯中…」），不必等翻完才看到有人說話
2. 譯文填回正確的位置，多則同時翻譯時**顯示順序未亂**
3. 設定視窗的「同時翻譯則數」改值後即時生效（`app.log` 應出現 `[translate] pool resized to N workers`）

- [ ] **Step 5: Commit**

```bash
git add tests/test_parallel_order.py README.md
git commit -m "test: guard display order against out-of-order completion"
```

---

## Self-Review

**Spec coverage**

| Spec 章節 | 對應 task |
|---|---|
| 1. 元件與職責：`ChatContext` | Task 1 |
| 1. 元件與職責：`Translator` 改介面 | Task 2 |
| 1. 元件與職責：`OverlayWindow` 加 API | Task 3 |
| 1. 元件與職責：`TranslationPool` | Task 4、5 |
| 1. 元件與職責：`reader_loop` 瘦身 | Task 7 |
| 2. 資料流：收訊路徑、msg_id 發號、上下文固定 | Task 7（`test_context_advances_by_read_order_not_by_completion`） |
| 2. 資料流：發話路徑 | Task 7（`input_box` 接線） |
| 2. 資料流：狀態指示 | Task 7（`test_status_shows_translating_while_pool_busy`） |
| 2. 資料流：邊界情況（id 已被擠掉） | Task 3（`test_update_message_ignores_unknown_id`） |
| 3. 錯誤處理：全域退避閘門 | Task 5 |
| 3. 錯誤處理：三類失敗處置 | Task 5 |
| 3. 錯誤處理：職責邊界（pool 不碰 ui_queue） | Task 4（`on_result` 回呼）、Task 7（接線） |
| 3. 錯誤處理：log 只在狀態轉換時印 | Task 5（`_note_failure`／`_note_success` 的 `changed` 判斷） |
| 4. 設定 | Task 6、Task 7（`apply_settings` 的 `resize`） |
| 5. 測試策略 | 各 task 的測試步驟 + Task 8 |

無未覆蓋項。

**Placeholder scan**：全部步驟均含實際程式碼，無 TBD／TODO／「類似 Task N」。Task 8 Step 3 的 README 更新附條件（表格不存在則跳過），非佔位。

**Type consistency**：`translate_incoming(text, context)` 於 Task 2 定義，Task 4／5／8 一致使用；`add_message(..., msg_id=)` 與 `update_message(msg_id, text)` 於 Task 3 定義，Task 7／8 一致使用；`TranslationPool` 的 `submit(line, context, msg_id)`、`in_flight`、`error_state`、`resize`、`shutdown` 於 Task 4 定義，Task 5／7／8 一致；`banner_for(game_missing, error_state)` 於 Task 7 定義並於同 task 測試。`BACKOFF_STEPS`／`CONFIG_ERROR_INTERVAL` 由 `main.py` 移至 `translation_pool.py`，Task 5 的測試 monkeypatch 的是 `src.translation_pool`，與移動後位置一致。
