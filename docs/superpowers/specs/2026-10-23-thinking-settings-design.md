# 各服務商的思考開關與思考深度 —— 設計

## 為什麼

目前的思考設定在四家服務商之間不一致，選項也少：

| 服務商 | 現況 |
|---|---|
| OpenAI／自訂端點 | 只有思考開關（`thinking`）；關閉時送 `reasoning_effort: none`，自訂端點另送幾種後端的關閉參數 |
| Claude／Gemini | 只有思考深度兩檔「自動／精簡」（`EFFORTS = [auto, low]`），沒有開關 |

使用者希望每家都能設定該服務商 API 支援的思考深度，有模型能關閉思考的服務商也提供開關。同一家不同模型支援的參數不同，這件事接受：選到不支援的組合時，測試連線會顯示錯誤。能從模型清單 API 事先判斷的，就在選模型時直接反映在介面上。

## 目標

- 四家服務商的介面一致：一律顯示**思考開關**；開啟時顯示**思考深度**選單，關閉時隱藏。
- 思考深度選單的第一項是「自動」（不帶任何深度參數，交給模型決定），其餘是該服務商 API 有的等級。
- 能從模型清單判斷的，選了模型就立即反映：
  - 開關鎖定開啟（模型一律思考）或鎖定關閉（模型不支援思考）；
  - 深度選單只列出該模型支援的等級。
- 判斷不了的，開關可切換、深度全部列出；不支援時由 API 報錯，測試連線看得到。
- 使用者主動選的深度被 API 拒絕時，一律報錯，不默默拿掉。
- 開啟設定裡的服務表單時自動載入模型清單；〔更新〕按鈕保留。
- 既有使用者升級後，送出的參數與現在相同（翻譯速度、行為不變）。

## 非目標

- 像沉浸式翻譯那樣維護「模型名稱 → 參數」對照表：每出新模型就要更新，與「選項開完整」相反。
- 自訂請求內容 JSON（進階欄位）。
- 思考 token 預算（`budget_tokens`／`thinkingBudget` 數值）讓使用者填：只用等級。唯一例外是 Gemini 關閉思考送 `thinkingBudget: 0`，見下。
- 調整 `MAX_TOKENS_THINKING`（4096）：深度選高可能被截斷，靠提示文字告知「越高越慢、越耗 Token」，被截斷時沿用既有的截斷錯誤。

## 做法比較

| 做法 | 內容 | 取捨 |
|---|---|---|
| A. 開關＋深度選單並列 | 每家同時顯示兩個控制項 | 會出現「關閉＋深度高」這種矛盾組合 |
| B. 單一深度選單 | 「自動／關閉／各等級」合成一個選單 | 不會矛盾，但四家的選單內容各不相同 |
| **C. 開關在前、開啟才顯示深度（採用）** | 一律顯示開關，開啟時才出現深度選單；判斷得出時鎖住開關 | 不會矛盾，四家長得一樣，使用者換服務商不必重新理解 |
| D. 依模型給預設（沉浸式翻譯的做法） | 維護模型對照表，使用者不用選 | 維護成本高，見非目標 |

另外比較了「判斷能不能關閉」的兩種方式：一律送出、讓 API 報錯；或先查模型清單。採用**能查就查、查不到就送出再報錯**：Claude 的清單資訊最完整，Gemini 可判斷「不支援思考」，OpenAI 與自訂端點查不到。

## 各服務商的 API 事實（2026-10-23 查證）

### 思考深度等級

| 服務商 | 送出的參數 | 等級 |
|---|---|---|
| OpenAI／自訂 | `reasoning_effort` | `minimal`、`low`、`medium`、`high`、`xhigh`、`max` |
| Claude | `output_config.effort` | `low`、`medium`、`high`、`xhigh`、`max` |
| Gemini | `generationConfig.thinkingConfig.thinking_level`（現有寫法，送大寫值） | `minimal`、`low`、`medium`、`high` |

各模型支援的子集不同，例如 GPT-6.1 Sol 不支援 `none`／`minimal`，Gemini 3.1 Pro 不支援 `minimal`。

### 關閉思考

| 服務商 | 關閉時送出 | 備註 |
|---|---|---|
| OpenAI | `reasoning_effort: none` | 部分模型不支援 `none`（400） |
| 自訂 | 沿用現有：`reasoning_effort: none`＋`chat_template_kwargs.enable_thinking`／`think`／`enable_thinking` 皆 false | 不動 |
| Claude | `thinking: {type: "disabled"}` | Haiku 5.5、Sonnet 5、Opus 5 接受；Opus 5.5、Sonnet 5.5、Fable 等會 400 |
| Gemini | `thinkingConfig: {thinkingBudget: 0}` | 只有 2.5 Flash／Flash-Lite 能關；`thinking_level` 與 `thinkingBudget` 不可同時送（400） |

### 模型清單可判斷的資訊

| 服務商 | 欄位 | 能判斷 |
|---|---|---|
| Claude | `capabilities.thinking.supported`、`capabilities.thinking.types.disabled.supported`、`capabilities.effort.{low,medium,high,xhigh,max}.supported` | 不支援思考、能否關閉、支援哪些深度 |
| Gemini | Model 資源的 `thinking`（布林） | 只能判斷「不支援思考」 |
| OpenAI | 只有 `id`、`created`、`owned_by` | 無 |
| 自訂 | 無統一格式 | 無 |

Claude 的 `capabilities` 就在現有 `list_models` 抓的 `/v1/models` 回應裡，不必多打請求。Gemini 的 `thinking` 也在現有 `models.list` 回應裡。

## 介面行為

### 開關的四種狀態

由「目前模型的思考能力」決定（能力怎麼來見下節）：

| 能力 | 開關 | 深度選單 | 開關下方提示 |
|---|---|---|---|
| 不支援思考 | 關閉、禁用 | 隱藏 | 此模型不支援思考。 |
| 一律思考（不能關閉） | 開啟、禁用 | 顯示 | 此模型一律會進行思考，無法關閉。 |
| 可切換 | 依設定值 | 開啟才顯示 | 沿用 `hint.thinking`（建議關閉：開啟可能會讓每則翻譯慢上數秒、也更耗 Token。） |
| 不知道 | 同「可切換」 | 同「可切換」 | 同「可切換」 |

- 表單開著時記住使用者自己的開關選擇；換到鎖定的模型再換回可切換的模型，恢復原本的選擇。
- **存檔與測試連線都用畫面上顯示的值**：鎖定關閉存 `thinking: false`，鎖定開啟存 `thinking: true`。翻譯時 client 不查清單，所以存下的就是實際生效的值。
- 鎖定關閉時，client 照 `thinking: false` 送關閉參數。不支援思考的模型收到關閉參數是否會被拒，見「待實測」第 3 項。
- 目前沒有任何一家服務商在「不知道模型」時整家鎖定開啟，鎖定開啟只會來自 Claude 清單的逐模型判斷。使用者明確要求兩種鎖定狀態都做齊。

### 深度選單

- 第一項「自動」：不帶深度參數。
- 其餘列出該服務商的全部等級；若清單提供了該模型支援的等級（目前只有 Claude），只列支援的。
- 清單說該模型不支援任何深度（Claude 的 `effort.supported = false`，例如 Haiku 4.5），選單只剩「自動」。
- 換模型後，目前選的深度若不在新選單裡，改回「自動」（改的是表單值，存檔後生效）。
- 深度下方提示（清單不知道模型支援哪些等級時）：

  > 各模型支援的思考深度不同，選到不支援的思考深度時，測試會顯示錯誤；如果您不確定可選「自動」。思考深度越高，翻譯越慢、越耗 Token。

- 清單已列出模型支援的等級時（選單已濾掉不支援的），改用：

  > 思考深度越高，翻譯越慢、越耗 Token；如果您不確定可選「自動」。

### 模型能力從哪來

- 模型清單由 `list_models` 一併帶回每個模型的思考能力。目前模型欄的值與清單中某個 id **完全相同**時，採用該筆能力；否則（手動輸入、清單沒載入、載入失敗、服務商不提供清單）一律視為「不知道」。
- 能力只用在設定表單。翻譯時（`build_client`）不查清單，照存下的設定送出 —— 介面已經擋掉的組合本來就不會存下來。

### 自動載入模型清單

- 服務表單開啟時（設定視窗的新增／編輯服務、首次設定精靈），若連線欄位已填妥（`validate_endpoint_fields` 通過），自動觸發一次與〔更新〕相同的載入。
- 新服務開啟時金鑰還是空的：金鑰或伺服器網址欄位**失去焦點**時，若連線欄位與上次自動載入時不同且已填妥，再自動載入一次。不在每次按鍵時載入。（我的決定：使用者要的是「不用手動按更新」，只在開啟時載入的話，新增服務永遠用不到。）
- 欄位沒填妥就不載入，也不顯示錯誤；狀態列維持 `hint.model_idle`。
- 載入失敗照現有方式顯示在模型欄下方（`ModelStatus::Error`），不跳視窗。
- 只載入目前開啟的那一個表單，不對清單裡的其他服務發請求。

## 設定格式

四家的欄位表都改成同時有 `thinking`（布林）與 `effort`（字串）：

| 服務商 | `thinking` 預設 | `effort` 預設 | 新服務送出的效果 |
|---|---|---|---|
| OpenAI | `false` | `auto` | `reasoning_effort: none`（同現在） |
| 自訂 | `false` | `auto` | 現有的一組關閉參數（同現在） |
| Claude | `true` | `auto` | 不帶 effort（同現在；Haiku 4.5 帶 effort 會 400） |
| Gemini | `true` | `low` | `thinking_level: LOW`（同現在） |

- **遷移**：既有服務缺 `thinking` 或 `effort` 時補上該家預設值即可，值的語意不變，不需要轉換：
  - OpenAI／自訂既有 `thinking: true` 加上 `effort: auto` → 不帶深度，同現在；
  - Claude／Gemini 補 `thinking: true`，原本的 `effort` 保留 → 同現在。
- `effort` 的合法值依服務商不同（見上表）；讀到不認得的值（例如手改 config）當成 `auto`。現有的 `EFFORTS` 常數改成依服務商取等級清單。

## Client 送出規則

| 服務商 | `thinking: false` | `thinking: true` + `auto` | `thinking: true` + 等級 |
|---|---|---|---|
| OpenAI | `reasoning_effort: none` | 不帶 | `reasoning_effort: <等級>` |
| 自訂 | 現有關閉參數組 | 不帶 | `reasoning_effort: <等級>` |
| Claude | `thinking: {type: "disabled"}`，不帶 effort | 不帶 | `output_config.effort: <等級>` |
| Gemini | `thinkingConfig: {thinkingBudget: 0}` | 不帶 `thinkingConfig` | `thinkingConfig: {thinking_level: <大寫等級>}` |

- Claude 關閉時固定送 `disabled`，不依模型改送 `between_tools`（Sonnet 5.5 要的關閉方式）：Sonnet 5.5 的清單能力會標示不能 `disabled`，介面直接鎖定開啟，不會走到這裡。
- `max_tokens`：現有邏輯是 OpenAI 相容端點思考關閉時用 `MAX_TOKENS`、開啟時用 `MAX_TOKENS_THINKING`；Claude／Gemini 一律 `MAX_TOKENS_THINKING`。維持不變。

### 被拒參數的處理（OpenAI 相容 client）

現有 `learn_rejection` 遇到 400 指名某參數被拒，就拿掉重送、記住以後不送。改成：

- **使用者選的深度**（`reasoning_effort` 的值不是 `none`）被拒：**不重送，直接回錯誤**。
- **關閉參數**（`reasoning_effort: none` 與自訂端點的其他關閉參數）被拒，分兩種：
  - 錯誤是 `Unsupported value`（認得參數、但模型關不掉思考，例如 gpt-5-mini）：**直接回錯誤**。2026-10-23 實測，拿掉參數重送等於讓模型照預設大量思考（gpt-5-nano 每句 768 個以上思考 token），與「關閉」相反。
  - 其他情況（例如 `Unrecognized request argument`，模型本來就不會思考，如 gpt-4.1-mini）：維持拿掉重送，預設設定（思考關閉）不能因此突然失敗。
- `max_tokens`／`max_completion_tokens` 互換、其他非思考參數：不變。

Claude 與 Gemini client 沒有自動拿掉參數的機制，被拒時本來就回錯誤，不用改。

## 文案（語系檔）

只有 `zh-TW`、`zh-CN`、`en-US` 翻了這一區，三份都要改：

- 新增：鎖定開啟提示（此模型一律會進行思考，無法關閉。）、鎖定關閉提示（此模型不支援思考。）、各等級名稱 `effort.minimal`／`low`／`medium`／`high`／`xhigh`／`max`。
- 修改：`hint.effort` 換成上面定案的句子。`effort.low` 原本叫「精簡」，現在是多個等級之一，名稱要改成表示「低」的字樣。
- 改到既有字串（`hint.effort`、`effort.low`）不先上傳 Crowdin：推上 GitHub 後由 GitHub 整合同步過去；之後 Crowdin 的 PR 若把舊字蓋回來，才手動處理。
- en-US、zh-CN 的譯文由實作者依 zh-TW 定案文字撰寫（我的決定）。en-US 現在把 `effort.low` 叫做「Minimal」，會跟新的 `effort.minimal` 撞名，要改成「Low」。
- 「可切換」狀態四家都沿用 `hint.thinking`（建議關閉…）（我的決定）：Claude／Gemini 預設開啟是為了在不知道模型能力時不出錯，建議關閉仍是正確的建議。

zh-TW 定案用詞：

| key | 文字 |
|---|---|
| 鎖定開啟提示 | 此模型一律會進行思考，無法關閉。 |
| 鎖定關閉提示 | 此模型不支援思考。 |
| `hint.effort` | 各模型支援的思考深度不同，選到不支援的思考深度時，測試會顯示錯誤；如果您不確定可選「自動」。思考深度越高，翻譯越慢、越耗 Token。 |

## Log

依專案規則用英文、帶 context：

- 模型清單載入：除了現有的 count，記下有多少模型帶回思考能力（例如 `with_thinking_caps=N`）；自動載入與按〔更新〕要分得出來（`trigger=auto|manual`）。
- 選中模型的能力判定結果：`provider`、`model`、能力（`unsupported|always|toggle|unknown`）、可選等級。只在判定結果改變時記，不要每幀記。
- 換模型時深度被改回自動：記下舊值與模型。
- OpenAI 相容 client：使用者選的深度被拒時記 `model`、`reasoning_effort` 值與 API 的錯誤細節；關閉參數被拒照現有 log。

## 待實測（實作時要用真實 API 驗證）

實作一開始先做這幾項，用 `config.json` 裡使用者現有的金鑰（四家都有）直接打 API，不印出金鑰。自訂端點用使用者目前設定的 llama-swap。

**實測結論（2026-10-23）：**

| 項目 | 結果 | 採取的分支 |
|---|---|---|
| 1. Gemini 2.5 吃不吃 `thinking_level` | 測不到：`gemini-2.5-flash`／`gemini-2.5-pro` 對新使用者回 404（「no longer available to new users」） | 不做 2.5 改送 `thinkingBudget`。現行程式本來就對所有 Gemini 模型送 `thinking_level`，既有 2.5 使用者若被拒，測試會看到錯誤 |
| 2. Opus 5 送 `disabled` 會不會外漏 `<thinking>` | 不會：回應只有一個 text 區塊，內容是單純譯文 | 不改 `strip_think`；修正 `ClaudeClient` 文件註解裡的舊說法 |
| 3. 不支援思考的模型收到關閉參數 | 測不到：Gemini 清單沒有任何 `thinking: false` 的模型（不支援思考的模型是**沒有這個欄位**，例如 TTS），Claude 清單沒有 `thinking.supported: false` 的模型 | 不做 Gemini 的關閉重送。解析仍照設計處理 `false`；實務上 Gemini 不會出現鎖定關閉 |
| 4. Gemini 3 收到 `thinkingBudget: 0` | `gemini-3-flash-preview`：200，回應沒有思考 token（真的關掉了）；`gemini-3.5-flash-lite`：400 INVALID_ARGUMENT；`gemini-3.1-pro-preview`：免費額度 429，測不到 | 沒看到默默忽略，**不做**測試連線的「關了仍在思考」檢查 |
| Claude 清單形狀 | `capabilities.thinking.types` 有 `adaptive`／`enabled`／`disabled`／`between_tools`；`capabilities.effort` 有 `supported` 與各等級。例：Sonnet 5.5、Opus 5.5、Fable 系列 `disabled: false`；Haiku 4.5、Sonnet 4.5 `effort.supported: false`；4.6 系列 `xhigh: false` | 照設計解析 |
| llama-swap（qwen3.8-27b） | `reasoning_effort: low` → 200 且有思考；`none` → 200 且沒有思考；不帶 → 有思考 | 不改計畫 |

1. **Gemini 2.5 能不能吃 `thinking_level`**：API 參考寫「用在 Gemini 3 之前的模型會報錯」，思考指南的表格卻列出 2.5 支援 low／medium／high。目前程式對所有 Gemini 模型都送 `thinking_level`。若 2.5 會報錯（我的決定）：
   - Gemini client 依模型 id 前綴 `gemini-2.` 判斷，改送 `thinkingBudget`。這是整個世代的規則，不是逐模型對照表，不違反非目標。
   - 等級對照沿用官方 OpenAI 相容層的表：`minimal`／`low` → 1024、`medium` → 8192、`high` → 24576。
2. **Opus 5 送 `disabled` 會不會把 `<thinking>` 漏進譯文**：這是 Python 時代的觀察（`claude.rs` 的 `ClaudeClient` 文件註解）。若仍會漏，在 `strip_think` 的正規式補上 `<thinking>` 標籤（現在只認 `<think>`），並修正該註解。
3. **不支援思考的模型收到關閉參數會不會被拒**：Claude 文件寫不支援思考的模型 `disabled.supported` 為 `true`（會接受）；OpenAI 的 `none` 被拒時本來就會拿掉重送；Gemini 不支援思考的模型收到 `thinkingBudget: 0` 尚未確認。若 Gemini 會拒，就讓 Gemini client 在 `thinking: false` 遇到 400 時拿掉 `thinkingConfig` 重送一次（比照 OpenAI 對 `none` 的寬鬆處理）。
4. **Gemini 3 收到 `thinkingBudget: 0` 的回應**：文件寫「Gemini 2.5 Pro 與 3 系列無法關閉思考」，預期回 400，使用者在測試時就會看到錯誤，不用另外處理。若實測是被默默忽略：
   - **測試連線**時，開關為關閉、回應卻有思考 token（Gemini `usageMetadata.thoughtsTokenCount`、OpenAI `usage.completion_tokens_details.reasoning_tokens`）大於 0，判定測試失敗，顯示「此模型無法關閉思考，請開啟思考後再測試。」（使用者定案的用詞）。存檔前就能發現設定沒生效。
   - 不用模型名稱規則鎖開關。
   - 一般翻譯不做這項檢查，只記 log：`[translate] thinking was off but model still thought: provider=… model=… thoughts=…`。
   - 新字串要加進 zh-TW、zh-CN、en-US。

## 驗收

- `cargo test` 全綠，新增的單元測試涵蓋：
  - 各服務商四種開關／深度組合的請求 body；
  - Claude／Gemini 清單回應解析出的思考能力；
  - 能力 → 開關狀態與深度選單項目的判定；
  - 換模型時深度改回自動；
  - 舊設定遷移後送出的參數與現在相同；
  - OpenAI 相容 client：使用者選的深度被拒時回錯誤、`none` 被拒時拿掉重送。
- 實機：四家各用一個能關、一個不能關的模型跑測試連線，確認開關鎖定與錯誤訊息符合預期；「待實測」各項有結論並反映到程式。

**驗收結果（2026-10-23，走 `test_translate`，不開 GUI）：**

| 服務商／模型 | 開關／深度 | 結果 |
|---|---|---|
| OpenAI `gpt-6-luna` | 關 | 成功 |
| OpenAI `gpt-6.1-sol` | 開＋`minimal` | 設定錯誤：「'reasoning_effort' does not support 'minimal' with this model…」，只送一次 |
| Claude `claude-haiku-5-5` | 關 | 成功 |
| Gemini `gemini-3-flash-preview` | 關 | 成功 |
| Gemini `gemini-3.8-flash` | 開＋`minimal` | 設定錯誤：「Thinking level MINIMAL is not supported for this model」 |
| 自訂 llama-swap `qwen3.8-27b` | 開＋`low`／關 | 都成功 |
| Claude 清單判定 | —— | `opus-5-5`、`sonnet-5-5` → 鎖定開啟；`haiku-5-5` → 可切換；`haiku-4-5` → 可切換、深度只剩自動 |

設定視窗的實際操作（換模型時的鎖定、深度選單、自動載入）待使用者驗收。

## 留給實作者決定

- 思考能力的資料型別與放的位置（例如 `list_models` 回傳 `Vec<ModelInfo>` 取代 `Vec<String>`），以及判定函式放在 config 還是 translation。
- 自動載入在 `ServiceForm` 裡怎麼觸發（例如第一次 `ui()` 時檢查旗標）。
- 鎖定狀態的開關用什麼方式畫成禁用（沿用 `add_enabled_ui`）。
