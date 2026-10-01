# Wizard101 聊天翻譯助手：目標語言範例動態生成設計

日期：2026-10-01
狀態：設計完成，待實作。

## 目標

收訊、系統訊息、框選區域三條翻譯路徑的「譯名（原文）」示範範例，目前固定以中文寫成並內嵌在
系統提示詞裡。範例的譯文側是中文，會把非中文目標語言的譯文帶成中文；而目標語言是使用者可自由
輸入的字串，無法用固定的語言範例表涵蓋。

本案改成：**請使用者設定的模型把範例改寫成目標語言，驗證後快取，並以 few-shot 對話輪送出**。

1. 任何目標語言（包括自由輸入的 `Japanese`）都拿到該語言的範例，不再被中文範例帶偏。
2. 範例改用 few-shot 對話輪，對小模型的約束力比內嵌在系統提示詞強。
3. 生成只在「目標語言或模型改變」時發生一次並存到磁碟，平常不額外送請求。
4. 生成完成前、或生成失敗時，翻譯照常進行，只是不放範例（退路）。

## 非目標

- **不改發話路徑**：發話固定翻成英文，`FEWSHOT_OUTGOING` 的目標側本來就是英文，沒有被帶偏的問題。
- **不做譯名表**：同一專有名詞在不同句子譯法不一致（Darkmoor → 黑暗沼澤／黑暗荒原）是另一個問題，
  本案不處理。
- **不改測試連線的介面**：範例生成的成敗只記 log，不影響測試連線的結果顯示。
- **不提供範例的手動編輯或檢視介面**。

## 實測依據（2026-10-01，qwen3.6-35b-a3b，自架端點，思考關閉）

真實對話取自使用者提供的 `messages.log`（92 句不重複玩家對話）；日文測試把發送者換成英文代號，
模擬英文伺服器上日文使用者的真實情境。

| 範例形式 | 日文目標：整句變成中文 | 繁中目標：該附原文 | 繁中目標：誤加括號 |
|---|---|---|---|
| 改寫前的原始提示詞 | 26/92 | 不附原文 | — |
| 中文範例內嵌在系統提示詞（目前） | 47/92 | 7/7 | 4/9 |
| 不放範例 | 26/92 | 3/7 | 4/9 |
| 中文範例改成 few-shot 對話輪 | 83/92 | 7/7 | 1/9 |
| **模型生成的目標語言範例＋few-shot 對話輪** | **約 0/92** | 7/7（以繁中範例實測） | 1/9 |

另外，以一次編號行請求生成全部範例，在日本語、`Japanese`、한국어、Español、Deutsch、繁體中文
六種目標上都產出通過驗證的結果。gpt-6-luna 在目前的中文範例下也會把範例用語（「笑死」）帶進
日文譯文，問題不只出現在小模型。

## 決策摘要

| 決策 | 選擇 | 理由 |
|---|---|---|
| 範例的語言 | 由模型改寫成目標語言 | 目標語言可自由輸入，固定的語言範例表涵蓋不完 |
| 範例的形式 | few-shot 對話輪（user 原文、assistant 譯文） | 實測約束力明顯比內嵌在系統提示詞強；發話路徑已在用 |
| 生成方式 | 一次請求，所有範例以編號行一起生成 | 每組設定只多 1 次請求；沿用框選的 `number_lines`／`unnumber_lines` |
| 退路 | 不放範例 | 譯文變成錯的語言比少了括號原文嚴重；程式無法得知目標語言是否為中文 |
| 遊戲語言（English） | 沿用固定的中文 → 英文範例，不生成 | 英文翻英文沒有可示範的內容；固定範例的譯文側本來就是英文 |
| 生成時機 | 啟動、儲存設定、測試連線 | 測試連線是使用者主動觸發的請求，結果存快取後儲存時直接命中 |
| 換模型 | 重新生成 | 不同模型產出的範例品質不同 |
| 快取鍵 | 服務商、端點、模型、目標語言、範例版次 | 與系統訊息快取的指紋同一套思路，不含金鑰 |
| 輸出異常 | 立即重試，最多 5 次，跨次合併通過的行 | 使用者指定 |
| 連線與狀態碼錯誤 | 不立即重試，同一次執行中不再試同一組指紋 | 與一般翻譯的錯誤處理一致，避免付費端點重複扣費 |
| 生成請求的 temperature | 不送 `temperature=0` | 自架端點固定 `temperature=0` 時重送必得同一結果，重試無意義 |

## 元件設計

### `src/translation/examples.py`（新增）

範例的生成、驗證與快取，跟翻譯本身分開。

**`ExampleSet`**（不可變）

```python
@dataclass(frozen=True)
class ExampleSet:
    incoming: tuple[str, str] | None   # (原文, 譯文)
    system: tuple[str, str] | None
    region: tuple[str, str] | None     # (原文行, 譯文行)，不帶編號（見下方「後續修訂」）
```

每條路徑各自可缺；缺的路徑翻譯時不放範例。`digest()` 回傳內容的短雜湊，供系統訊息快取的指紋使用。

**範例的原文與中文示範**：模組內的常數。原文一律是英文（遊戲語言），共五行：

```
1. [Amy] idk, Kai and I got new armor and learned Fire Cat at Colossus Boulevard lol, brb my wand is trash
2. Kai taught you Fire Cat! Gained {0} gold at Colossus Boulevard.
3. Talk to the Fire Cat
4. Go to Colossus Boulevard
5. and then you must
```

第 1 行是收訊範例，第 2 行是系統訊息範例，第 3～5 行是框選範例（第 5 行示範截斷句停在原處）。
中文示範沿用目前 `prompts.py` 內嵌的版本，只作為生成請求的格式示範，不再直接送進翻譯提示詞。
遊戲語言的固定範例（中文原文 → 英文譯文）也從 `prompts.py` 搬到這裡。

**`generate_examples(client, target_language) -> ExampleSet`**

- 目標是遊戲語言（`is_game_language`）時不送請求，直接回傳固定範例。
- 否則送一次請求：系統提示詞說明「逐行翻成目標語言、保留編號、專有名詞譯名後括號照抄英文原文、
  `[Amy]`／`Kai`／`{0}` 照抄、最後一行停在截斷處」並附中文示範；user 輪是五行編號原文。
- 用 `unnumber_lines` 拆回五行，逐行驗證：

| 行 | 必須包含 |
|---|---|
| 1（收訊） | `[Amy]`、`Kai`，且 `Fire Cat` 與 `Colossus Boulevard` 各自出現在括號（全形或半形）裡 |
| 2（系統訊息） | `Kai`、恰好一個 `{0}`，且 `Fire Cat` 與 `Colossus Boulevard` 各自出現在括號裡 |
| 3、4、5（框選） | 第 3 行 `Fire Cat` 在括號裡、第 4 行 `Colossus Boulevard` 在括號裡、第 5 行非空 |

**後續修訂（2026-10-01，實機測試框選後）**：

- **原文改為六行**：截斷句前插入介面標籤 `OPTIONS`（中文示範「選項」）。
  - 只示範「加括號」時，qwen 會替整排選單、設定標籤都附原文。
  - 這行驗證「有翻譯且不含括號」。
  - 部分語言拼法與英文相同（法文 OPTIONS），模型確實寫了這行時，照抄也算譯文。
- **框選翻譯不再加編號**：
  - 編號擋不住模型合併折行的句子，缺的編號又只能補回原文，卡片上會多出已經翻過的英文。
  - 框選範例因此以不帶編號的行儲存。生成請求本身仍用編號，以便逐行驗證。
- **括號原文的比對範圍**：行數對得上時，比對該行與上下相鄰行；行數對不上時，比對整段。
- 以上兩次調整各遞增一次 `EXAMPLE_REVISION`，目前為 3。

- **重試**：輸出被截斷（`TranslatorBadOutput`）、行數對不上、或有路徑沒通過驗證時，立即重試，
  最多 5 次（總共最多 6 次請求）。每次只採用新通過的路徑，已通過的保留；三條路徑都通過就提早結束。
- **不重試的錯誤**：`TranslatorOffline`、`TranslatorConfigError`、`TranslatorCancelled` 直接往上拋，
  由呼叫端記 log。
- 生成請求不送 `temperature=0`（見下方 `translator.py` 的修改），上限用 `_MAX_TOKENS_REGION`。

**`examples_fingerprint(api, target_language) -> str`**

`provider|base_url|model|target_language|e{EXAMPLE_REVISION}`，不含金鑰。`EXAMPLE_REVISION` 是
本模組的常數，改動範例原文、中文示範或生成提示詞時 +1。

**`ExampleStore`**

- 檔案：`local_state_dir() / "translation-examples.json"`，與系統訊息快取同一個資料夾。
- 格式：`{"entries": {fingerprint: {"incoming": [...] | null, "system": ..., "region": ...}}}`。
- 可同時保存多組指紋（收訊與框選可能用不同服務、切換目標語言後切回、測試連線測別的服務），
  上限 32 組，超過時捨棄最舊的。
- `get(fp)`、`put(fp, examples)`；`put` 立即寫檔，沿用 `TranslationCache.flush` 的
  `tempfile.mkstemp` ＋ `os.replace` 原子寫入。
- 讀檔失敗或格式不符時當成空快取並記 log，不拋例外。
- 內部用一把鎖保護；另外記錄「正在生成中」與「本次執行已失敗」的指紋集合，
  供協調器去重與避免重試（見下方資料流）。

### `src/translation/prompts.py`（修改）

- `build_incoming_system`、`build_system_message_system`、`build_region_system` 移除內嵌範例
  （`_example` 與中文示範常數刪除，遊戲語言的固定範例搬到 `examples.py`）。`_closing` 保留。
- 新增 `example_turns(pair: tuple[str, str] | None) -> list[dict]`：`None` 回傳空清單，
  否則回傳 user／assistant 兩輪。翻譯時把它當成 `build_turns` 的 `examples` 傳入，放在背景上下文之前，
  與發話的 `FEWSHOT_OUTGOING` 走同一條路。
- 系統訊息提示詞改變，`PROMPT_REVISION` 從 5 升到 6。

### `src/translation/translator.py`（修改）

- `Translator` 新增：
  - `examples_fingerprint` 屬性：依目前的 `_api` 與目標語言計算。
  - `set_examples(examples: ExampleSet | None, fingerprint: str) -> bool`：`fingerprint` 與目前的
    不符時忽略並回傳 False（生成途中設定已改變）。
  - `examples` 屬性：目前套用的 `ExampleSet | None`。
- `reconfigure` 實際重建 client 時，**一律先把範例清成 `None`**，由協調器決定改用快取或背景生成。
  這保證切換目標語言後絕不沿用舊語言的範例。
- `translate_incoming`、`translate_system_message`、`translate_region_text` 在請求開始時把
  `self._examples` 綁成區域變數，再組對話輪，避免請求途中換範例造成混用（與現有 `_chat` 綁定 `impl` 同理）。
- 後端 client 的 `chat()` 新增參數 `deterministic: bool = True`；`False` 時自架端點不送
  `temperature=0`。只有生成範例會傳 `False`。

### 協調：`examples.py` 的 `ExampleCoordinator`（新增）＋ `main.py`、設定視窗（修改）

`ExampleCoordinator` 負責「何時用快取、何時背景生成、結果交給誰」，`main.py` 只負責接線：

- 建構參數：`ExampleStore`、啟動背景執行緒的函式、把結果交回 UI 執行緒的函式（`ui_queue.put`）。
- 記住呼叫過 `ensure` 的 translator（收訊、框選兩個長期實例），生成完成時才能套用到所有指紋相符者。
- `ensure(translator)`：
  1. 取 `translator.examples_fingerprint`。
  2. 快取命中 → 直接 `set_examples`。
  3. 未命中、且該指紋不在「生成中」或「本次執行已失敗」集合 → 標記生成中，在背景執行緒呼叫
     `generate_examples`；完成後 `put` 進快取，回到 UI 執行緒對**所有**指紋相符的 translator
     呼叫 `set_examples`（收訊與框選共用同一服務時只生成一次）。
  4. 生成失敗 → 記入「本次執行已失敗」，維持退路。
- `generate_now(api, target_language)`：給測試連線用，同步執行並存進快取，**不檢查**「本次執行已失敗」
  集合（使用者主動觸發）。

接線點：

- **啟動**：`build_translation` 建好 translator 後，對收訊與框選兩個 translator 呼叫 `ensure`。
  發話 translator 不需要範例。
- **儲存設定**：`reconfigure_translation` 中 `reconfigure` 回傳 True 的收訊、框選 translator 呼叫 `ensure`。
- **測試連線**：`test_translate` 成功後，在同一個 `BackgroundButton` 的背景工作裡呼叫 `generate_now`。
  測試連線的總耗時會增加一次生成的時間（輸出異常時最多 6 次請求）。生成失敗只記 log，不改變測試結果。
- **系統訊息快取**：`incoming_fingerprint(cfg)` 加上收訊 translator 目前範例的 `digest()`（`None` 時為固定字串）。
  收訊 translator 的範例被替換時，在 UI 執行緒呼叫 `cache.rebind(...)`，退路期間翻出的譯文因此失效。

## 資料流

### 切換目標語言或模型

1. 使用者在設定視窗按下儲存 → `apply_settings` → `reconfigure_translation`。
2. 收訊或框選的 `Translator.reconfigure` 回傳 True → 範例被清成 `None`（立即退路）。
3. `ExampleCoordinator.ensure`：新指紋快取命中就立即套用；否則背景生成。
4. 背景生成完成時，比對 translator 目前的指紋：相符才套用；不符（例如生成途中又切換）則只存進快取、不套用。

| 情境 | 結果 |
|---|---|
| 切回用過的語言或模型 | 快取命中，不送請求 |
| 日文生成中途切回繁中 | 日文結果存快取但不套用；繁中依快取命中或另行生成 |
| 快速來回切換 | 同一指紋生成中不重複送請求 |
| 翻譯中的請求 | 沿用請求開始時綁定的範例 |

### 測試連線後儲存

1. 測試連線以表單當下的服務與目標語言呼叫 `generate_now`，結果存進快取。
2. 使用者按下儲存，`ensure` 以相同指紋查快取，直接命中。

## 錯誤處理

| 情況 | 處理 |
|---|---|
| 輸出被截斷、行數對不上、某些路徑沒通過驗證 | 立即重試，最多 5 次；最後仍沒通過的路徑退路 |
| 連線失敗、逾時、429、5xx | 記 log、維持退路，同一次執行中不再試這組指紋 |
| 4xx（金鑰、模型、參數錯誤） | 同上；翻譯本身也會遇到並由現有的錯誤提示告知使用者 |
| 快取檔損壞或讀不到 | 當成空快取，記 log |
| 程式結束時生成未完成 | 背景執行緒隨程序結束；原子寫入保證快取檔不會寫壞 |
| 測試連線時生成失敗 | 只記 log，測試連線的結果照常顯示 |

## Log

英文、`[translate]` 前綴，絕不記金鑰：

- 開始生成：`[translate] generating examples (provider=…, model=…, target=…, attempt=N)`
- 完成：耗時與各路徑結果，例如 `incoming=ok system=ok region=failed`
- 驗證失敗：失敗的行號、缺少的元素，以及該行生成的內容
- 快取命中、丟棄過期結果、改用退路、重試：各一行

## 測試

單元測試使用現有的假 client（`FakeHttpxClient`、`SequenceClient` 等），不送真實請求。

- **生成與驗證**：各路徑的必要元素；英文目標回傳固定範例且不送請求；部分路徑失敗時只有該路徑退路。
- **重試**：輸出異常立即重試、跨次合併、最多 6 次請求；連線錯誤不重試；生成請求不帶 `temperature=0`。
- **快取**：多組指紋並存、上限捨棄最舊、命中不送請求、檔案損壞當成空快取、原子寫入。
- **提示詞**：三條路徑的系統提示詞不再內嵌中文範例；`example_turns` 轉成對話輪；`None` 時沒有範例輪。
- **Translator**：`set_examples` 後三條路徑帶上對應範例；指紋不符時忽略；`reconfigure` 後範例被清空；
  請求途中換範例不影響進行中的請求。
- **協調器與切換目標語言**：切換後不沿用舊範例；切回時快取命中；生成途中切換不套用；同一指紋不重複生成；
  本次執行已失敗的指紋不再自動生成；`generate_now` 不受失敗集合限制。
- **系統訊息快取**：指紋包含範例雜湊；範例替換後 `rebind`。
- **既有測試**：檢查送出對話輪內容的測試（`test_translator.py`、`test_main.py`、`test_cache.py`）配合新結構調整。

## 驗收

| 項目 | 標準 |
|---|---|
| qwen 日文目標（92 句、英文發送者名稱） | 整句變成中文從 47/92 降到接近 0 |
| qwen 繁中目標（92 句） | 該附原文 7/7、誤加括號 ≤ 2/9 |
| qwen 多語言生成 | 日本語、Japanese、한국어、Español、Deutsch、繁體中文都產出通過驗證的範例 |
| gpt-6-luna、claude-sonnet-5（`low`） | 精簡測試集抽樣沒有退步（約 20～40 次付費請求） |
| 實機 | `uv run run.py` 切換目標語言、測試連線後，`app.log` 的範例生成、快取命中、退路紀錄正確 |
| 品質檢查 | `uv run ruff check src tests tools` 零錯誤、`uv run pytest` 全數通過 |
