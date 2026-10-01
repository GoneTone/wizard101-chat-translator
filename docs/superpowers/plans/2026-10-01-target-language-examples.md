# 目標語言範例動態生成 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓收訊、系統訊息、框選三條翻譯路徑的「譯名（原文）」範例由模型改寫成目標語言、驗證後快取，並以 few-shot 對話輪送出；沒有範例時退回不放範例。

**Architecture:** 新增 `src/translation/examples.py`，負責範例的資料結構、生成請求、驗證、磁碟快取（`ExampleStore`）與觸發協調（`ExampleCoordinator`）。它不 import `translator.py`；送請求與重試的迴圈放在 `Translator.generate_examples()`，以免循環 import。`prompts.py` 移除內嵌的中文範例，改由 `example_turns()` 轉成對話輪；`main.py` 在啟動與儲存設定時呼叫協調器，測試連線則透過 `test_translate(..., example_store=)` 生成。

**Tech Stack:** Python 3、httpx、anthropic SDK、tkinter、pytest、uv。

**Spec:** `docs/superpowers/specs/2026-10-01-target-language-examples-design.md`

## Global Constraints

- 程式碼命名、註解、提示詞邏輯不可寫死 `zh`／`en`／繁體中文；語言一律當參數傳入（CLAUDE.md）。
- 註解預設一行、只寫 WHY，三行是上限；模組與公開函式寫一行摘要 docstring。CJK 語境用全形標點。
- log 一律英文、`[translate]` 前綴，帶足夠 context（provider、model、target、attempt、耗時），**絕不寫入 API 金鑰**；不得直接 `print`。
- Commit message 英文、conventional commits，結尾加上：
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` 與
  `Claude-Session: https://claude.ai/code/session_01BxaQkFyyw8kdBrU4MgeX7n`。不要 push。
- 每個 task commit 前：`uv run ruff check src tests tools` 零錯誤、該 task 相關測試通過；全套 `uv run pytest` 只在最後一個程式 task 跑一次。
- 快取檔：`local_state_dir() / "translation-examples.json"`，上限 32 組指紋，原子寫入（`tempfile.mkstemp` ＋ `os.replace`）。
- 生成：一次請求、五行編號原文；輸出異常（`TranslatorBadOutput`、行數不符、驗證失敗）立即重試，最多 5 次（總共 6 次請求）；生成請求不送 `temperature=0`；上限 `_MAX_TOKENS_REGION`。
- 連線與狀態碼錯誤（`TranslatorOffline`、`TranslatorConfigError`、`TranslatorCancelled`）不重試，同一次執行中不再自動生成同一組指紋。
- 目標是遊戲語言（`is_game_language`）時不送生成請求，使用固定範例。
- `PROMPT_REVISION` 從 5 升到 6；新增 `EXAMPLE_REVISION = 1`。

## Review Focus

1. **儲存設定但什麼都沒改**：範例必須保留、不送任何生成請求。→ Task 6 `test_reconfigure_without_changes_keeps_examples_and_does_not_ensure`。
2. **目標語言前後帶空白**（`" 日本語 "`）：應視為同一個目標語言，不重新生成。→ Task 1 `test_fingerprint_ignores_surrounding_whitespace_in_the_target`。
3. **模型在編號行前加了前言或包了 code fence**（`以下是翻譯：`、```` ``` ````）：仍要正確解析。→ Task 1 `test_parse_generated_ignores_a_preamble_and_code_fences`。
4. **連按兩次測試連線**：第二次快取命中，不再送生成請求。→ Task 4 `test_generate_and_store_reuses_a_cached_set_without_a_request`。
5. **生成途中又切換設定**：舊設定的結果存進快取但不套用到新設定。→ Task 5 `test_a_result_for_an_outdated_fingerprint_is_stored_but_not_applied`。

---

### Task 1: 範例資料結構、生成請求與驗證

**Files:**
- Create: `src/translation/examples.py`
- Create: `tests/test_examples.py`

**Interfaces:**
- Consumes: `src.translation.postprocess.number_lines`、`unnumber_lines`（既有）。
- Produces:
  - `EXAMPLE_REVISION: int = 1`
  - `SOURCE_LINES: tuple[str, ...]`（五行英文原文，見下）
  - `DEMO_LINES: tuple[str, ...]`（五行中文示範，見下）
  - `@dataclass(frozen=True) class ExampleSet`：欄位 `incoming`、`system`、`region`，型別皆為 `tuple[str, str] | None`（`(原文, 譯文)`；region 是 `(編號後的三行原文, 編號後的三行譯文)`）。方法：`merge(other) -> ExampleSet`（自己已有的保留、缺的用 other 補）、屬性 `complete -> bool`、屬性 `empty -> bool`、`digest() -> str`（內容的 sha1 前 12 碼）、`summary() -> str`（如 `incoming=ok system=ok region=failed`）。
  - `EMPTY = ExampleSet(None, None, None)`
  - `GAME_LANGUAGE_EXAMPLES: ExampleSet`（固定的中文 → 英文範例）
  - `examples_fingerprint(api: dict, target_language: str) -> str`
  - `generation_request(target_language: str) -> tuple[str, list[dict]]`（system 提示詞, 對話輪）
  - `parse_generated(output: str) -> tuple[ExampleSet, list[str]]`（通過驗證的範例, 失敗說明清單）

**固定內容**（照抄）：

`SOURCE_LINES`：
```
[Amy] idk, Kai and I got new armor and learned Fire Cat at Colossus Boulevard lol, brb my wand is trash
Kai taught you Fire Cat! Gained {0} gold at Colossus Boulevard.
Talk to the Fire Cat
Go to Colossus Boulevard
and then you must
```

`DEMO_LINES`：
```
[Amy] 不知道耶，我和 Kai 拿到新護甲，還在巨像大道（Colossus Boulevard）學會了火貓（Fire Cat），笑死，等我一下，我的法杖超爛
Kai 教會了你火貓（Fire Cat）！在巨像大道（Colossus Boulevard）獲得了 {0} 金幣。
和火貓（Fire Cat）談談
前往巨像大道（Colossus Boulevard）
然後你必須
```

`GAME_LANGUAGE_EXAMPLES`：把目前 `prompts.py` 三個 `_example(...)` 呼叫的第二個 tuple（native，中文 → 英文）原樣搬過來；region 的原文與譯文各自是三行編號文字（沿用 prompts.py 現有內容）。

`generation_request` 的 system 提示詞（`{t}` 代入目標語言，`{demo}` 是 `number_lines(DEMO_LINES)` 的編號文字）：
```
把使用者給的每一行翻成 {t}，逐行對應、保留行首編號，只輸出譯文。格式照下面的中文示範：遊戲專有名詞翻成 {t} 後緊接括號照抄英文原文；[Amy]、人名 Kai、{0} 照抄不翻；縮寫與一般名詞直接翻、不加括號；最後一行是被截斷的句子，譯文也停在同一處。
中文示範：
{demo}
```
對話輪：`[{"role": "user", "content": number_lines(SOURCE_LINES) 的編號文字}]`。

**驗證規則**（括號全形半形皆可：`[（(]\s*名稱\s*[)）]`）：

| 行 | 必須 |
|---|---|
| 1 → incoming | 含 `[Amy]`、`Kai`；`Fire Cat`、`Colossus Boulevard` 各在括號裡 |
| 2 → system | 含 `Kai`；`{0}` 恰好一次；`Fire Cat`、`Colossus Boulevard` 各在括號裡 |
| 3–5 → region | 第 3 行 `Fire Cat` 在括號裡、第 4 行 `Colossus Boulevard` 在括號裡、第 5 行非空；三行都通過才算 region 通過 |

`parse_generated` 先用 `unnumber_lines(output, list(SOURCE_LINES))` 拆行；行數不是 5 就回傳 `(EMPTY, ["line count ..."])`。region 的原文用 `number_lines(SOURCE_LINES[2:])`、譯文用 `number_lines(第 3–5 行)` 重新編號為 1–3。

- [ ] **Step 1: Write the failing tests** in `tests/test_examples.py`

```python
GOOD = "\n".join([
    "1. [Amy] 知らないよ、Kai と新しい鎧を手に入れて、巨像大道（Colossus Boulevard）で火猫（Fire Cat）を覚えた、笑",
    "2. Kai が火猫（Fire Cat）を教えてくれた！巨像大道（Colossus Boulevard）で {0} ゴールドを獲得した。",
    "3. 火猫（Fire Cat）に話しかける",
    "4. 巨像大道（Colossus Boulevard）へ行く",
    "5. そしてあなたは",
])

def test_parse_generated_accepts_every_path():
    examples, failures = parse_generated(GOOD)
    assert examples.complete and failures == []
    assert examples.incoming == (SOURCE_LINES[0], GOOD.splitlines()[0][3:])
    assert examples.region[0].startswith("1. Talk to the Fire Cat")
    assert examples.region[1].splitlines()[2] == "3. そしてあなたは"

def test_parse_generated_drops_only_the_failing_path():
    bad_system = GOOD.replace("{0} ゴールド", "ゴールド")
    examples, failures = parse_generated(bad_system)
    assert examples.system is None and examples.incoming is not None and examples.region is not None
    assert any("line 2" in f for f in failures)

def test_parse_generated_requires_the_original_in_parentheses():
    examples, _ = parse_generated(GOOD.replace("火猫（Fire Cat）を覚えた", "火猫を覚えた"))
    assert examples.incoming is None

def test_parse_generated_rejects_a_wrong_line_count():
    examples, failures = parse_generated("\n".join(GOOD.splitlines()[:3]))
    assert examples == EMPTY and failures

def test_parse_generated_ignores_a_preamble_and_code_fences():
    examples, _ = parse_generated("以下是翻譯：\n```\n" + GOOD + "\n```")
    assert examples.complete

def test_merge_keeps_existing_paths_and_fills_missing_ones():
    a = ExampleSet(("s", "o"), None, None)
    b = ExampleSet(("x", "y"), ("s2", "o2"), None)
    assert a.merge(b) == ExampleSet(("s", "o"), ("s2", "o2"), None)

def test_summary_and_digest():
    assert ExampleSet(("s", "o"), None, None).summary() == "incoming=ok system=failed region=failed"
    assert ExampleSet(("s", "o"), None, None).digest() != EMPTY.digest()

def test_fingerprint_contains_service_target_and_revision_but_no_key():
    api = {"provider": "custom", "base_url": "https://x", "model": "m", "api_key": "secret"}
    fp = examples_fingerprint(api, "日本語")
    assert fp == f"custom|https://x|m|日本語|e{EXAMPLE_REVISION}" and "secret" not in fp

def test_fingerprint_ignores_surrounding_whitespace_in_the_target():
    api = {"provider": "openai", "model": "m"}
    assert examples_fingerprint(api, " 日本語 ") == examples_fingerprint(api, "日本語")

def test_generation_request_names_the_target_and_numbers_the_sources():
    system, turns = generation_request("Deutsch")
    assert "Deutsch" in system and "1. [Amy]" in system
    assert turns == [{"role": "user", "content": "\n".join(f"{i}. {s}" for i, s in enumerate(SOURCE_LINES, 1))}]

def test_game_language_examples_translate_into_english():
    assert "Fire Cat (火" in GAME_LANGUAGE_EXAMPLES.incoming[1]
    assert GAME_LANGUAGE_EXAMPLES.complete
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `uv run pytest tests/test_examples.py -q`
Expected: FAIL（`ModuleNotFoundError: src.translation.examples`）

- [ ] **Step 3: Implement the Task 1 interfaces in `src/translation/examples.py`**

模組 docstring 一行摘要。code fence 與前言交給 `unnumber_lines`（它只取有編號的行）；若前言本身帶編號導致行數不符，就當驗證失敗。

- [ ] **Step 4: Run tests to verify they pass**

Run: `uv run pytest tests/test_examples.py -q && uv run ruff check src tests tools`
Expected: PASS、`All checks passed!`

- [ ] **Step 5: Commit**

```bash
git add src/translation/examples.py tests/test_examples.py
git commit -m "feat(translate): add example set, generation request and validation"
```

---

### Task 2: `ExampleStore` 磁碟快取

**Files:**
- Modify: `src/translation/examples.py`
- Modify: `tests/test_examples.py`

**Interfaces:**
- Consumes: `ExampleSet`（Task 1）、`src.config.local_state_dir`。
- Produces:
  - `EXAMPLES_PATH = local_state_dir() / "translation-examples.json"`、`MAX_ENTRIES = 32`
  - `class ExampleStore`：`__init__(self, path: Path = EXAMPLES_PATH)`、`get(self, fingerprint: str) -> ExampleSet | None`、`put(self, fingerprint: str, examples: ExampleSet) -> None`

**行為：** 不在記憶體保存內容，`get`／`put` 都直接讀檔，讓測試連線與主程式各自建立的實例看到同一份資料。模組層一把 `threading.Lock` 保護 `put` 的「讀－改－寫」。格式 `{"entries": {fingerprint: {"incoming": [s, o] | null, "system": ..., "region": ...}}}`，依插入順序；`put` 把該指紋移到最後，超過 `MAX_ENTRIES` 時捨棄最前面的。`examples.empty` 為 True 時不寫入。讀檔失敗或格式不符時當成空快取並記 `[translate] example cache unreadable (...)`。

- [ ] **Step 1: Write the failing tests**（用 `tmp_path`）

```python
def test_store_round_trips_and_keeps_several_fingerprints(tmp_path):
    store = ExampleStore(tmp_path / "ex.json")
    a, b = ExampleSet(("s", "o"), None, None), ExampleSet(None, ("s", "o"), None)
    store.put("fp-a", a); store.put("fp-b", b)
    assert ExampleStore(tmp_path / "ex.json").get("fp-a") == a
    assert store.get("fp-b") == b and store.get("fp-c") is None

def test_store_drops_the_oldest_beyond_the_limit(tmp_path):
    examples = ExampleSet(("s", "o"), None, None)
    store = ExampleStore(tmp_path / "ex.json")
    for i in range(MAX_ENTRIES + 1):
        store.put(f"fp-{i}", examples)
    assert store.get("fp-0") is None and store.get(f"fp-{MAX_ENTRIES}") is not None

def test_store_treats_a_corrupt_file_as_empty(tmp_path):
    path = tmp_path / "ex.json"
    path.write_text("{not json", encoding="utf-8")
    assert ExampleStore(path).get("fp") is None

def test_store_does_not_write_an_empty_set(tmp_path):
    path = tmp_path / "ex.json"
    ExampleStore(path).put("fp", EMPTY)
    assert not path.exists()
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `uv run pytest tests/test_examples.py -q -k store`
Expected: FAIL（`ExampleStore` 未定義）

- [ ] **Step 3: Implement `ExampleStore`**（原子寫入照 `TranslationCache.flush` 的 `tempfile.mkstemp` ＋ `os.replace`）

- [ ] **Step 4: Run tests to verify they pass**

Run: `uv run pytest tests/test_examples.py -q && uv run ruff check src tests tools`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/translation/examples.py tests/test_examples.py
git commit -m "feat(translate): persist generated examples per service and target"
```

---

### Task 3: 提示詞改用 few-shot 範例輪

**Files:**
- Modify: `src/translation/prompts.py`（刪除 `_example` 與三個 `build_*_system` 裡的 `_example(...)` 呼叫；新增 `example_turns`；`PROMPT_REVISION = 6`）
- Modify: `tests/test_translator.py`（`test_every_parenthesis_prompt_shows_the_format_and_restates_the_target`、`test_game_language_example_translates_into_english_with_the_original_in_parentheses` 改寫或移除）

**Interfaces:**
- Produces: `example_turns(pair: tuple[str, str] | None) -> list[dict]`：`None` 回傳 `[]`，否則回傳 `[{"role": "user", "content": 原文}, {"role": "assistant", "content": 譯文}]`。

`_closing` 保留；`_game_noun_rule` docstring 中提到範例的句子若因此不成立就一併修正。

- [ ] **Step 1: Write the failing tests**（取代上面兩個舊測試）

```python
def test_system_prompts_no_longer_embed_an_example():
    for build in (build_incoming_system, build_system_message_system, build_region_system):
        prompt = build("日本語")
        assert "範例" not in prompt and "Fire Cat" not in prompt
        assert prompt.rstrip().endswith("譯文一律使用 日本語，不論原文或本說明是什麼語言。")

def test_example_turns_become_a_user_and_assistant_pair():
    assert example_turns(None) == []
    assert example_turns(("src", "out")) == [{"role": "user", "content": "src"},
                                             {"role": "assistant", "content": "out"}]

def test_prompt_revision_was_bumped_for_the_example_change():
    assert PROMPT_REVISION == 6
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `uv run pytest tests/test_translator.py -q -k "embed_an_example or example_turns or prompt_revision"`
Expected: FAIL

- [ ] **Step 3: Implement**（`GAME_LANGUAGE_EXAMPLES` 已在 Task 1 搬走，這裡只刪除）

- [ ] **Step 4: Run tests to verify they pass**

Run: `uv run pytest tests/test_translator.py tests/test_cache.py -q && uv run ruff check src tests tools`
Expected: PASS（`test_cache.py` 若有斷言 `p5` 的測試，改成 `p{PROMPT_REVISION}`）

- [ ] **Step 5: Commit**

```bash
git add src/translation/prompts.py tests/test_translator.py tests/test_cache.py
git commit -m "refactor(translate): move prompt examples out of the system prompts"
```

---

### Task 4: Translator 帶上範例、生成範例與測試連線

**Files:**
- Modify: `src/translation/translator.py`
- Modify: `tests/test_translator.py`

**Interfaces:**
- Consumes: `ExampleSet`、`EMPTY`、`GAME_LANGUAGE_EXAMPLES`、`examples_fingerprint`、`generation_request`、`parse_generated`、`ExampleStore`（Task 1–2）；`example_turns`（Task 3）。
- Produces:
  - `_OpenAICompatClient.chat(..., deterministic: bool = True)`：`False` 時自架端點不送 `temperature`；`_ClaudeClient.chat(..., deterministic: bool = True)` 接受並忽略。
  - `Translator.examples -> ExampleSet | None`、`Translator.examples_fingerprint -> str`、`Translator.set_examples(examples: ExampleSet | None, fingerprint: str) -> bool`（指紋不符回傳 False 且不套用）。
  - `Translator.reconfigure` 實際重建時把範例清成 `None`。
  - `Translator.generate_examples() -> tuple[str, ExampleSet]`：開頭就綁定 `impl`、目標語言與指紋；遊戲語言直接回傳 `(fp, GAME_LANGUAGE_EXAMPLES)` 不送請求；否則最多 6 次請求（`deterministic=False`、`max_tokens=_MAX_TOKENS_REGION`），`TranslatorBadOutput` 與驗證失敗都重試並 `merge`，`complete` 就提早結束；其他例外往上拋。
  - `generate_and_store(api: dict, target_language: str, store: ExampleStore) -> ExampleSet | None`：快取命中直接回傳、不送請求；否則建臨時 `Translator` 生成，非 `empty` 才 `put`，最後 `close()`。
  - `test_translate(api: dict, target_language: str, example_store: ExampleStore | None = None) -> str`：翻譯成功後若有 `example_store` 就呼叫 `generate_and_store`；生成的任何例外只記 log，不影響回傳值。

翻譯路徑：`translate_incoming` 把 `example_turns(ex.incoming)` 當成 `build_turns` 的 `examples`；`translate_system_message` 在方法開頭綁定一次範例，兩次嘗試（含 strict）都用同一份；`translate_region_text` 的對話輪是 `example_turns(ex.region) + [編號後的待翻內容]`。`ex` 為 `None` 時一律不放範例輪。

log（英文）：`[translate] generating examples (provider=…, model=…, target=…, attempt=N)`；完成 `[translate] examples generated in X.Xs after N attempt(s): {summary}`；每次驗證失敗 `[translate] example validation failed (attempt=N): {failures}; output={output!r}`。

- [ ] **Step 1: Write the failing tests**

```python
JA = ...  # 與 Task 1 tests/test_examples.py 的 GOOD 相同的五行字串，在本檔另外定義一份

def test_translate_incoming_sends_the_example_before_the_context():
    fake = FakeHttpxClient()
    tr = _make(fake)
    fp = tr.examples_fingerprint
    assert tr.set_examples(ExampleSet(("src", "out"), None, None), fp)
    tr.translate_incoming("[A] hi", ["[B] yo"])
    turns = _turns(fake.last_body)
    assert turns[:2] == [{"role": "user", "content": "src"}, {"role": "assistant", "content": "out"}]
    assert turns[-1] == {"role": "user", "content": "[A] hi"}

def test_without_examples_no_example_turns_are_sent():
    fake = FakeHttpxClient()
    _make(fake).translate_system_message("你获得了 {0} 金币！")
    assert _turns(fake.last_body) == [{"role": "user", "content": "你获得了 {0} 金币！"}]

def test_region_text_sends_the_region_example_first():
    fake = FakeHttpxClient(response=FakeResponse(content="1. 譯文"))
    tr = _make(fake)
    tr.set_examples(ExampleSet(None, None, ("1. a", "1. b")), tr.examples_fingerprint)
    tr.translate_region_text("hello")
    assert _turns(fake.last_body)[:2] == [{"role": "user", "content": "1. a"},
                                          {"role": "assistant", "content": "1. b"}]

def test_set_examples_ignores_a_stale_fingerprint():
    tr = _make(FakeHttpxClient())
    assert not tr.set_examples(ExampleSet(("s", "o"), None, None), "other")
    assert tr.examples is None

def test_reconfigure_clears_the_examples():
    tr = _make(FakeHttpxClient())
    tr.set_examples(ExampleSet(("s", "o"), None, None), tr.examples_fingerprint)
    tr.reconfigure(provider="custom", base_url="http://x", model="m", target_language="日本語")
    assert tr.examples is None

def test_generate_examples_retries_bad_output_and_merges():
    good_but_no_system = JA.replace("{0} ゴールド", "ゴールド")
    fake = _contents("garbage", good_but_no_system, JA)
    tr = Translator(target_language="日本語", client=fake, provider="custom", base_url="http://x", model="m")
    fp, examples = tr.generate_examples()
    assert examples.complete and len(fake.bodies) == 3 and fp == tr.examples_fingerprint
    assert all("temperature" not in body for body in fake.bodies)

def test_generate_examples_gives_up_after_six_requests():
    fake = _contents(*["garbage"] * 7)
    tr = Translator(target_language="日本語", client=fake, provider="custom", base_url="http://x", model="m")
    _, examples = tr.generate_examples()
    assert examples == EMPTY and len(fake.bodies) == 6

def test_generate_examples_does_not_retry_a_config_error():
    fake = FakeHttpxClient(response=FakeResponse(status_code=401, text='{"error":{"message":"bad key"}}'))
    tr = Translator(target_language="日本語", client=fake, provider="custom", base_url="http://x", model="m")
    with pytest.raises(TranslatorConfigError):
        tr.generate_examples()

def test_generate_examples_for_the_game_language_sends_nothing():
    fake = FakeHttpxClient()
    tr = Translator(target_language="English", client=fake, provider="custom", base_url="http://x", model="m")
    assert tr.generate_examples()[1] == GAME_LANGUAGE_EXAMPLES and fake.last_body is None

def test_generate_and_store_reuses_a_cached_set_without_a_request(tmp_path, monkeypatch):
    store = ExampleStore(tmp_path / "ex.json")
    api = {"provider": "custom", "base_url": "http://x", "model": "m"}
    cached = ExampleSet(("s", "o"), None, None)
    store.put(examples_fingerprint(api, "日本語"), cached)
    monkeypatch.setattr(Translator, "generate_examples", lambda self: pytest.fail("no request expected"))
    assert generate_and_store(api, "日本語", store) == cached

def test_test_translate_survives_a_failing_example_generation(tmp_path, monkeypatch):
    monkeypatch.setattr(Translator, "translate_incoming", lambda self, text, ctx: "譯文")
    monkeypatch.setattr(Translator, "generate_examples",
                        lambda self: (_ for _ in ()).throw(TranslatorOffline("down")))
    api = {"provider": "custom", "base_url": "http://x", "model": "m"}
    assert test_translate(api, "日本語", example_store=ExampleStore(tmp_path / "ex.json")) == "譯文"
```

（`FakeResponse(status_code=, text=)`、`_contents`、`_turns`、`_make` 皆為檔內既有 helper。）

- [ ] **Step 2: Run tests to verify they fail**

Run: `uv run pytest tests/test_translator.py -q -k "example or test_translate_survives"`
Expected: FAIL

- [ ] **Step 3: Implement the Task 4 interfaces in `src/translation/translator.py`**

- [ ] **Step 4: Run tests to verify they pass**

Run: `uv run pytest tests/test_translator.py tests/test_examples.py -q && uv run ruff check src tests tools`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/translation/translator.py tests/test_translator.py
git commit -m "feat(translate): send target-language examples as few-shot turns"
```

---

### Task 5: `ExampleCoordinator`

**Files:**
- Modify: `src/translation/examples.py`
- Modify: `tests/test_examples.py`

**Interfaces:**
- Consumes: `ExampleStore`（Task 2）；translator 以鴨子型別使用：`examples_fingerprint`、`examples`、`set_examples(examples, fp) -> bool`、`generate_examples() -> tuple[str, ExampleSet]`、`describe() -> str`。
- Produces: `class ExampleCoordinator`：
  - `__init__(self, store: ExampleStore, post: Callable[[Callable[[], None]], None], spawn: Callable[[Callable[[], None]], None] | None = None, on_applied: Callable[[object], None] | None = None)`；`spawn` 預設開 daemon `threading.Thread`。
  - `ensure(self, translator) -> None`（只在 UI 執行緒呼叫）。
  - 快取存在 `self._store`（測試直接用它預先寫入）。

**行為：**
1. 記住 translator（同一物件只記一次）。
2. 快取命中 → `set_examples` 並呼叫 `on_applied(translator)`，記 `[translate] examples cache hit (...)`。
3. 指紋在「生成中」或「本次執行已失敗」集合 → 不再生成，記 `[translate] examples unavailable, translating without them (...)`（維持退路）。
4. 否則標記生成中，`spawn` 背景工作：`fp, ex = translator.generate_examples()`；非 `empty` 就 `store.put(fp, ex)` 並 `post` 回 UI 執行緒套用到**所有**記住的、`examples_fingerprint == fp` 的 translator（套用成功者各呼叫 `on_applied`）；沒有任何符合者記 `[translate] discarded examples for an outdated fingerprint`。`empty` 或例外 → `post` 回 UI 執行緒把請求的指紋加入失敗集合並記 log。兩種情況都把請求的指紋移出生成中集合。

- [ ] **Step 1: Write the failing tests**（同步的 `spawn`／`post`：`lambda job: job()`；假 translator 自訂，計數 `generate_examples` 呼叫次數）

```python
class FakeTr:
    def __init__(self, fp, result=None, error=None):
        self.examples_fingerprint, self.examples = fp, None
        self.result, self.error, self.calls = result, error, 0
    def set_examples(self, ex, fp):
        if fp != self.examples_fingerprint: return False
        self.examples = ex; return True
    def generate_examples(self):
        self.calls += 1
        if self.error: raise self.error
        return self.examples_fingerprint, self.result
    def describe(self): return "provider=custom, model=m"

SET = ExampleSet(("s", "o"), ("s", "o"), ("s", "o"))

def coordinator(tmp_path, applied=None):
    return ExampleCoordinator(ExampleStore(tmp_path / "ex.json"), post=lambda job: job(),
                              spawn=lambda job: job(), on_applied=(applied or []).append)

def test_cache_hit_applies_without_generating(tmp_path):
    co = coordinator(tmp_path); co._store.put("fp", SET)
    tr = FakeTr("fp", SET); co.ensure(tr)
    assert tr.examples == SET and tr.calls == 0

def test_a_miss_generates_stores_and_applies(tmp_path):
    applied = []
    co = coordinator(tmp_path, applied)
    tr = FakeTr("fp", SET); co.ensure(tr)
    assert tr.examples == SET and tr.calls == 1 and applied == [tr]
    assert ExampleStore(tmp_path / "ex.json").get("fp") == SET

def test_two_translators_on_the_same_service_share_one_generation(tmp_path):
    jobs = []
    co = ExampleCoordinator(ExampleStore(tmp_path / "ex.json"), post=lambda j: j(), spawn=jobs.append)
    a, b = FakeTr("fp", SET), FakeTr("fp", SET)
    co.ensure(a); co.ensure(b)
    assert len(jobs) == 1
    jobs[0]()
    assert a.examples == SET and b.examples == SET

def test_a_failed_fingerprint_is_not_retried_in_the_same_run(tmp_path):
    co = coordinator(tmp_path)
    tr = FakeTr("fp", error=RuntimeError("offline")); co.ensure(tr); co.ensure(tr)
    assert tr.calls == 1 and tr.examples is None

def test_an_empty_result_counts_as_a_failure(tmp_path):
    co = coordinator(tmp_path)
    tr = FakeTr("fp", EMPTY); co.ensure(tr); co.ensure(tr)
    assert tr.calls == 1 and ExampleStore(tmp_path / "ex.json").get("fp") is None

def test_a_result_for_an_outdated_fingerprint_is_stored_but_not_applied(tmp_path):
    jobs = []
    co = ExampleCoordinator(ExampleStore(tmp_path / "ex.json"), post=lambda j: j(), spawn=jobs.append)
    tr = FakeTr("fp-ja", SET); co.ensure(tr)
    tr.examples_fingerprint = "fp-zh"          # 生成途中使用者切換了目標語言
    tr.generate_examples = lambda: ("fp-ja", SET)
    jobs[0]()
    assert tr.examples is None
    assert ExampleStore(tmp_path / "ex.json").get("fp-ja") == SET
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `uv run pytest tests/test_examples.py -q -k "cache_hit or miss or share or failed or empty_result or outdated"`
Expected: FAIL

- [ ] **Step 3: Implement `ExampleCoordinator`**（生成中與失敗集合只在 UI 執行緒讀寫，不需要鎖）

- [ ] **Step 4: Run tests to verify they pass**

Run: `uv run pytest tests/test_examples.py -q && uv run ruff check src tests tools`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/translation/examples.py tests/test_examples.py
git commit -m "feat(translate): coordinate example generation across settings changes"
```

---

### Task 6: 接線（啟動、儲存設定、系統訊息快取、測試連線）

**Files:**
- Modify: `src/translation/cache.py`（`fingerprint_of` 新增參數）
- Modify: `src/main.py`（`incoming_fingerprint`、`build_translation`、`reconfigure_translation`、`build_app`）
- Modify: `src/ui/service_form.py`（`_start_test`）
- Modify: `tests/test_cache.py`、`tests/test_main.py`、`tests/test_service_form.py`

**Interfaces:**
- Consumes: `ExampleCoordinator`、`ExampleStore`（Task 2、5）；`Translator.examples`、`test_translate(..., example_store=)`（Task 4）。
- Produces:
  - `fingerprint_of(provider, model, target_language, base_url="", examples_digest="-") -> str`：在現有結尾後加 `|x{examples_digest}`。
  - `incoming_fingerprint(cfg: dict, examples: ExampleSet | None = None) -> str`：`examples` 為 `None` 時傳 `"-"`，否則傳 `examples.digest()`。
  - `build_translation(cfg, deliver, post) -> tuple[dict[str, Translator], TranslationCache, list[TranslationPool], ExampleCoordinator]`：建立 `ExampleCoordinator(ExampleStore(), post, on_applied=...)`；`on_applied` 只在 translator 是收訊那格時呼叫 `cache.rebind(incoming_fingerprint(cfg, tr.examples))`；最後對收訊、框選兩格呼叫 `ensure`（發話不需要）。
  - `reconfigure_translation(cfg, translators, cache, pools, coordinator) -> None`：只對 `reconfigure` 回傳 True 的收訊、框選格呼叫 `ensure`；最後 `cache.rebind(incoming_fingerprint(cfg, translators[SLOT_INCOMING].examples))`。
  - `build_app` 把 `ui_queue.put` 傳給 `build_translation`，並把 coordinator 傳給 `reconfigure_translation`。
  - `ServiceForm._start_test`：改呼叫 `test_translate(api, target, example_store=ExampleStore())`。

- [ ] **Step 1: Write the failing tests**

`tests/test_cache.py`：
```python
def test_fingerprint_includes_the_examples_digest():
    assert fingerprint_of("custom", "m", "日本語").endswith("|x-")
    assert fingerprint_of("custom", "m", "日本語", examples_digest="abc").endswith("|xabc")
```

`tests/test_main.py`（`_stub_translation` 另外 `monkeypatch.setattr(main, "ExampleCoordinator", _FakeCoordinator)` 與 `monkeypatch.setattr(main, "ExampleStore", lambda: None)`）：
```python
class _FakeCoordinator:
    """範例協調器替身：只記下 ensure 了誰，並留住 on_applied 讓測試直接觸發。"""
    def __init__(self, store, post, spawn=None, on_applied=None):
        self.ensured, self.on_applied = [], on_applied
    def ensure(self, translator):
        self.ensured.append(translator)

def _build(cfg):
    return main.build_translation(cfg, lambda *a: None, lambda job: job())

def test_build_translation_ensures_examples_for_incoming_and_region(monkeypatch):
    _stub_translation(monkeypatch)
    translators, _cache, _pools, coordinator = _build(_three_slot_cfg())
    assert coordinator.ensured == [translators[SLOT_INCOMING], translators[SLOT_REGION]]

def test_reconfigure_ensures_examples_only_for_rebuilt_slots(monkeypatch):
    _stub_translation(monkeypatch)
    cfg = _three_slot_cfg()
    translators, cache, pools, coordinator = _build(cfg)
    coordinator.ensured.clear()
    find(cfg, cfg["service_slots"][SLOT_REGION])["model"] = "region-model-2"
    main.reconfigure_translation(cfg, translators, cache, pools, coordinator)
    assert coordinator.ensured == [translators[SLOT_REGION]]

def test_reconfigure_without_changes_keeps_examples_and_does_not_ensure(monkeypatch):
    _stub_translation(monkeypatch)
    cfg = _three_slot_cfg()
    translators, cache, pools, coordinator = _build(cfg)
    incoming = translators[SLOT_INCOMING]
    examples = ExampleSet(("s", "o"), None, None)
    incoming.set_examples(examples, incoming.examples_fingerprint)
    coordinator.ensured.clear()
    main.reconfigure_translation(cfg, translators, cache, pools, coordinator)
    assert coordinator.ensured == [] and incoming.examples == examples

def test_applying_incoming_examples_rebinds_the_system_message_cache(monkeypatch):
    _stub_translation(monkeypatch)
    translators, cache, _pools, coordinator = _build(_three_slot_cfg())
    incoming = translators[SLOT_INCOMING]
    examples = ExampleSet(("s", "o"), None, None)
    incoming.set_examples(examples, incoming.examples_fingerprint)
    coordinator.on_applied(incoming)
    assert cache.fingerprint.endswith("|x" + examples.digest())
    before = cache.fingerprint
    coordinator.on_applied(translators[SLOT_REGION])
    assert cache.fingerprint == before
```
既有的 `build_translation`／`reconfigure_translation` 測試改成解出四個回傳值、改傳 `post` 與 `coordinator`。

`tests/test_service_form.py`：
```python
def test_test_connection_also_generates_examples(blank, monkeypatch):
    works, calls = [], {}
    monkeypatch.setattr("src.ui.service_form.validate_service", lambda api: [])
    monkeypatch.setattr(blank._test_task, "start", lambda work, on_done, busy: works.append(work))
    monkeypatch.setattr("src.ui.service_form.test_translate",
                        lambda api, target, example_store=None: calls.setdefault("store", example_store))
    blank._start_test()
    works[0]()
    assert isinstance(calls["store"], ExampleStore)
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `uv run pytest tests/test_cache.py tests/test_main.py tests/test_service_form.py -q`
Expected: FAIL

- [ ] **Step 3: Implement the Task 6 interfaces**

- [ ] **Step 4: Run the full suite**

Run: `uv run ruff check src tests tools && uv run pytest -q`
Expected: `All checks passed!`、全部通過

- [ ] **Step 5: Commit**

```bash
git add src/translation/cache.py src/main.py src/ui/service_form.py tests/test_cache.py tests/test_main.py tests/test_service_form.py
git commit -m "feat(translate): generate examples on startup, settings changes and connection tests"
```

---

### Task 7: 實測驗收

不改程式碼；以真實模型驗證 spec 的驗收標準。實測腳本是這次調整提示詞時寫在 session scratchpad 的拋棄式腳本，執行者需改寫成透過 `Translator.generate_examples()`／`set_examples()` 走正式程式路徑。

- [ ] **Step 1: qwen 多語言生成**：對 日本語、Japanese、한국어、Español、Deutsch、繁體中文（台灣）各呼叫一次 `generate_examples()`。Expected：六種都 `complete`，或至少每種都有 incoming 與 system。
- [ ] **Step 2: qwen 日文目標**（92 句真實對話、發送者換成英文代號）。Expected：整句變成中文的句數接近 0（原本 47/92）。
- [ ] **Step 3: qwen 繁中目標**（92 句）。Expected：該附原文 7/7、誤加括號 ≤ 2/9。
- [ ] **Step 4: 付費模型抽樣**：gpt-6-luna（不思考）與 claude-sonnet-5（`low`）各跑精簡測試集一次（各約 21 次翻譯請求＋1 次生成）。Expected：與 commit `7627803` 時的結果相比沒有退步。
- [ ] **Step 5: 實機**：請使用者 `uv run run.py`，切換目標語言、按測試連線、再切回。Expected：`app.log` 依序出現 `generating examples`、`examples generated`、`examples cache hit`，切換後沒有沿用舊語言範例。
- [ ] **Step 6: 回報結果**，未達標的項目列出原因與建議，不自行放寬標準。
