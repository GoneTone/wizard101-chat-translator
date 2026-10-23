# 各服務商的思考開關與思考深度 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 四家服務商一律有思考開關、開啟時有思考深度選單；能從模型清單判斷的就鎖住開關、過濾深度；開啟表單自動載入模型清單。

**Architecture:**
- **設定層**：`config::services` 的欄位表讓四家都有 `thinking`＋`effort`，並記住各家的深度等級。
- **Client 層**：照「開關 × 深度」送出參數；`list_models` 一併帶回每個模型的思考能力（`ThinkingCaps`）。
- **介面層**：一個純函式模組依能力決定開關狀態與選單項目，`ServiceForm` 只負責畫。

**Tech Stack:** Rust、egui（kittest 測 UI）、reqwest、wiremock（client 測試）、serde_json。

**Spec:** `docs/superpowers/specs/2026-10-23-thinking-settings-design.md`

## Global Constraints

- 回答、註解、docstring、UI 文字用繁體中文全形標點；log 訊息一律英文，前綴沿用 `[settings]`／`[translate]`／`[config]`。
- 註解預設一行、只寫 WHY；公開項目一行 `///` 摘要。
- 不可寫死語言；新 UI 字串要加進 `src/i18n/zh-TW.json`、`zh-CN.json`、`en-US.json` 三份（只有這三份翻了這一區）。不上傳 Crowdin。
- 金鑰絕不進 log、絕不印到終端機。
- 新整合測試寫在 `tests/it/` 現有模組裡，不在 `tests/` 另開檔。
- cargo 一律 `-j 4`，測試 `-- --test-threads=4`。
- 每個 task 的 commit 前跑：`cargo fmt --check`、`cargo clippy -j 4 --all-targets -- -D warnings`、該 task 相關的測試；全套 `cargo test` 只在最後一個 task 跑。
- Commit message 英文、conventional commits，結尾加：
  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_0195s5qydrgZUTvcGVFSv4qY
  ```
- 不 push。
- 預設值（spec「設定格式」）：
  - OpenAI `thinking=false, effort="auto"`；
  - 自訂 `thinking=false, effort="auto"`；
  - Claude `thinking=true, effort="auto"`；
  - Gemini `thinking=true, effort="low"`。
- 各家深度等級（顯示順序，`auto` 一律在最前）：
  - OpenAI／自訂：`minimal, low, medium, high, xhigh, max`；
  - Claude：`low, medium, high, xhigh, max`；
  - Gemini：`minimal, low, medium, high`。

## Review Focus

1. **舊 config 升級後送出的參數不變**：每家各一筆缺 `thinking` 或 `effort` 的舊服務，正規化後組出的請求 body 要與改版前相同。→ Task 2 的 `legacy_services_keep_their_request_parameters`。
2. **手改 config 寫了不認得的深度**（例如 Claude 寫 `"minimal"`）：當成 `auto`，不送出。→ Task 2 的 `unknown_effort_falls_back_to_auto`。
3. **清單回來前或之後使用者改了模型欄**：能力要跟著目前輸入的 id 走，不能沿用上一個模型的。→ Task 7 的 `caps_follow_the_typed_model_id`。
4. **鎖定期間存檔，換回可切換的模型後再存檔**：要恢復使用者原本的開關值。→ Task 7 的 `locked_switch_restores_the_users_choice`。
5. **自訂端點沒有 `/v1/models`（`NoModelList`）**：自動載入不跳錯、狀態顯示「不提供清單」，開關可切換。→ Task 8 的 `auto_load_without_model_list_keeps_switch_usable`。

---

### Task 1: 真實 API 實測（spike）

不寫產品程式碼；結果決定 Task 4、Task 3 的條件步驟與 Task 9 要不要做。

**Files:**
- Create（scratchpad，不進 repo）：探測腳本
- Modify: `docs/superpowers/specs/2026-10-23-thinking-settings-design.md`（「待實測」各項補上結論）

- [ ] **Step 1: 寫探測腳本**：從 `config.json` 的 `services` 取各家金鑰與 llama-swap 的 `base_url`，在程式內讀取，不印出、不寫進檔案。直接打 API，對每個 case 只印 HTTP 狀態碼、錯誤訊息，以及思考 token 數（Gemini `usageMetadata.thoughtsTokenCount`、OpenAI `usage.completion_tokens_details.reasoning_tokens`）與譯文前 80 字。

- [ ] **Step 2: 跑下列 case，記錄結果**

| # | 服務商／模型 | 送出 | 要回答的問題 |
|---|---|---|---|
| 1a | Gemini `gemini-2.5-flash` | `thinkingConfig.thinking_level: "LOW"` | 2.5 收不收 `thinking_level`（spec 待實測 1） |
| 1b | Gemini `gemini-2.5-pro` | 同上 `"MEDIUM"` | 同上 |
| 2 | Claude `claude-opus-5` | `thinking: {type: "disabled"}` | 回應有沒有 `<thinking>` 混進文字（待實測 2） |
| 3a | Gemini 任一 `thinking: false` 的模型（從 `models.list` 挑） | `thinkingBudget: 0` | 會不會被拒（待實測 3） |
| 3b | Claude 任一 `capabilities.thinking.supported: false` 的模型（若有） | `thinking: disabled` | 同上 |
| 4 | Gemini `gemini-3-flash-preview`、`gemini-3.1-pro-preview` | `thinkingBudget: 0` | 400 還是默默忽略（思考 token > 0）（待實測 4） |
| 5 | Claude `/v1/models` | GET | 回應裡 `capabilities.thinking.types.disabled`、`capabilities.effort.*` 的實際形狀，把兩三筆模型的回應存成精簡樣本放在 scratchpad，給 Task 5 寫 fixture |
| 6 | Gemini `/v1beta/models` | GET | 確認 `thinking` 欄位存在 |
| 7 | llama-swap | `reasoning_effort: "low"`、`reasoning_effort: "none"` | 後端接不接受、會不會默默忽略 |

- [ ] **Step 3: 把結論寫回 spec「待實測」各項**（一行結論＋採取哪個分支），commit：

```bash
git add docs/superpowers/specs/2026-10-23-thinking-settings-design.md
git commit -m "docs(spec): record live API findings for thinking settings"
```

**分支判定（後續 task 照這張表）：**
- 1a 或 1b 被拒 → 做 Task 4 Step 5（2.5 改送 `thinkingBudget`）。
- 2 有 `<thinking>` 外漏 → 做 Task 3 Step 5（`strip_think` 補 `<thinking>`）。
- 3a 被拒 → 做 Task 4 Step 6（Gemini 關閉被拒時拿掉 `thinkingConfig` 重送一次）。
- 3b 被拒 → 停下來回報使用者，spec 沒有涵蓋。
- 4 任一默默忽略 → 做 Task 9。
- 7 的結果只記錄，不改計畫：自訂端點被拒會照新規則報錯。

---

### Task 2: 設定層：四家都有開關與深度

**Files:**
- Modify: `src/config/services.rs`（`Provider`、`PROVIDERS`、`EFFORTS`、`ServiceSpec` 文件註解）
- Modify: `src/translation/clients/mod.rs:148`（`build_client` 的深度先過 `known_effort`）
- Modify: `src/ui/settings/service_form.rs`（只改 `EFFORTS` 的引用，讓它編得過）
- Test: `tests/config_services.rs`、`tests/golden/config_*.json`

**Interfaces:**
- Produces:
  - `Provider` 新增公開欄位 `pub efforts: &'static [&'static str]`（不含 `auto`）；
  - `pub fn known_effort(provider: &Provider, value: &str) -> &'static str`：`value` 是 `EFFORT_AUTO` 或在 `provider.efforts` 裡就回那個值，否則回 `EFFORT_AUTO`；
  - `EFFORT_AUTO`、`EFFORT_LOW` 保留；刪除 `EFFORTS`。
  - 四家 `fields` 都以 `("thinking", Flag(..)), ("effort", Text(..))` 結尾，預設值見 Global Constraints。

- [ ] **Step 1: 寫失敗測試**（`tests/config_services.rs`）
  - `every_provider_has_thinking_and_effort`：四家 `has_field("thinking") && has_field("effort")`。
  - `provider_defaults_keep_current_behavior`：`defaults()` 的 `thinking`／`effort` 等於 Global Constraints 的表。
  - `provider_effort_levels`：四家 `efforts` 等於 Global Constraints 的表。
  - `unknown_effort_falls_back_to_auto`：`known_effort(claude, "minimal") == "auto"`、`known_effort(gemini, "xhigh") == "auto"`、`known_effort(openai, "max") == "max"`、`known_effort(gemini, "") == "auto"`。
  - `legacy_services_keep_their_request_parameters`：
    - 舊格式服務正規化後補上預設：Claude `{effort:"low"}` → `thinking:true`；Gemini `{effort:"auto"}` → `thinking:true`；OpenAI `{thinking:true}` → `effort:"auto"`。
    - 斷言正規化後的欄位值。實際送出的 body 由 Task 3／4 斷言。

- [ ] **Step 2: 跑測試確認失敗**：`cargo test -j 4 --test config_services -- --test-threads=4`，預期 compile error（沒有 `efforts`／`known_effort`）。

- [ ] **Step 3: 實作**：
  - 改 `PROVIDERS` 欄位表；`build_client` 傳給各 client 的深度改用 `known_effort(provider, &spec.effort)`；`ServiceForm::new` 的過濾改用 `known_effort`。
  - 移除 `services.rs:90` 那行「Claude 的 effort 維持 auto」的註解：預設值本身已表達，原因移到 Claude 預設值旁一行註解。

- [ ] **Step 4: 更新 golden**：跑 `cargo test -j 4 --test it -- config_golden --test-threads=4`。失敗的 fixture（`tests/golden/config_normalize.json`、`config_load.json`、`config_resolve.json`、`config_summary.json`、`config_validate.json` 中有影響的）用 scratchpad 的一次性腳本更新期望輸出。規則：每個期望輸出中的服務物件，缺 `thinking` 就補該家預設、缺 `effort` 就補該家預設，鍵序為該家欄位表順序（`thinking` 在 `effort` 前）。只動期望輸出，不動 input。

- [ ] **Step 5: 跑測試確認通過**：`cargo test -j 4 --test config_services -- --test-threads=4` 與 `cargo test -j 4 --test it -- config --test-threads=4`，PASS。

- [ ] **Step 6: Commit**：`feat(config): give every provider a thinking switch and effort levels`

---

### Task 3: Claude client 的送出規則

**Files:**
- Modify: `src/translation/clients/claude.rs`（`ClaudeClient::new`、`request_body`、結構文件註解）
- Modify: `src/translation/clients/mod.rs`（`build_client` 傳 `spec.thinking`）
- Test: `tests/it/clients_claude.rs`

**Interfaces:**
- Consumes: Task 2 的 `known_effort`。
- Produces: `ClaudeClient::new(api_key: &str, model: &str, thinking: bool, effort: &str, endpoint: &str) -> Self`。

- [ ] **Step 1: 寫失敗測試**：`client()` 輔助函式加 `thinking` 參數。
  - `claude_thinking_off_sends_disabled_without_effort`：`thinking=false, effort="high"` → body 有 `"thinking": {"type": "disabled"}`，`output_config` 等於 `{"format": output_format()}`（沒有 effort）。
  - `claude_thinking_on_auto_sends_neither`：`thinking=true, effort="auto"` → 沒有 `thinking` 鍵，`output_config` 沒有 effort。
  - `claude_thinking_on_level_sends_effort`：`thinking=true, effort="xhigh"` → `output_config.effort == "xhigh"`，沒有 `thinking` 鍵。
  - 既有的 `claude_effort_only_when_not_auto` 改用 `thinking=true`。

- [ ] **Step 2: 跑測試確認失敗**：`cargo test -j 4 --test it -- clients_claude --test-threads=4`。

- [ ] **Step 3: 實作**：照 spec「Client 送出規則」Claude 那一列改。`claude.rs:55-56` 的文件註解改成：Claude 4.6 之後的模型不接受 temperature；關閉時送 `disabled`，能不能關由設定介面依模型清單判斷。

- [ ] **Step 4: 跑測試確認通過**。

- [ ] **Step 5（只在 Task 1 case 2 有外漏時）：`strip_think` 補 `<thinking>`**：
  - 在 `tests/it/postprocess_golden.rs` 加 `strip_think_removes_thinking_blocks`：`strip_think("<thinking>x</thinking>Hello") == "Hello"`。
  - `src/translation/postprocess.rs` 的 `THINK_BLOCK`／`THINK_PREFIX` 改成也認 `thinking`，例如 `<th[iıİ]nk(?:[iıİ]ng)?>`。

- [ ] **Step 6: Commit**：`feat(translate): let Claude services turn thinking off`

---

### Task 4: Gemini client 的送出規則

**Files:**
- Modify: `src/translation/clients/gemini.rs`（`GeminiClient::new`、`request_body`）
- Modify: `src/translation/clients/mod.rs`（`build_client`）
- Test: `tests/it/clients_gemini.rs`

**Interfaces:**
- Produces: `GeminiClient::new(api_key: &str, model: &str, thinking: bool, effort: &str, endpoint: &str) -> Self`。

- [ ] **Step 1: 寫失敗測試**：
  - `gemini_thinking_off_sends_zero_budget`：`thinking=false, effort="low"` → `generationConfig.thinkingConfig == {"thinkingBudget": 0}`。
  - `gemini_thinking_on_auto_sends_no_config`：`thinking=true, effort="auto"` → `generationConfig` 沒有 `thinkingConfig`。
  - `gemini_thinking_on_level_sends_upper_level`：`thinking=true, effort="medium"` → `{"thinking_level": "MEDIUM"}`。
  - 既有的 `gemini_thinking_level_only_when_not_auto` 改用 `thinking=true`。

- [ ] **Step 2: 跑測試確認失敗**：`cargo test -j 4 --test it -- clients_gemini --test-threads=4`。

- [ ] **Step 3: 實作**：照 spec「Client 送出規則」Gemini 那一列改。

- [ ] **Step 4: 跑測試確認通過**。

- [ ] **Step 5（只在 Task 1 case 1a／1b 被拒時）：Gemini 2.5 改送 `thinkingBudget`**：
  - 測試 `gemini_25_maps_levels_to_budgets`：模型 `gemini-2.5-flash`，`minimal`／`low` → `{"thinkingBudget": 1024}`、`medium` → `8192`、`high` → `24576`。
  - 測試 `gemini_3_keeps_thinking_level`：模型 `gemini-3-flash-preview`，`low` → `thinking_level`。
  - 實作：以模型 id 前綴 `gemini-2.` 判斷（去掉 `models/` 前綴後比對），一行註解說明 2.5 不收 `thinking_level`。

- [ ] **Step 6（只在 Task 1 case 3a 被拒時）：關閉被拒時拿掉 `thinkingConfig` 重送一次**：
  - 測試 `gemini_rejected_off_retries_without_thinking_config`：wiremock 第一次回 400 `thinkingBudget` 相關錯誤、第二次 200 → 成功；第二個請求沒有 `thinkingConfig`。
  - 測試 `gemini_rejected_level_is_an_error`：`thinking=true, effort="minimal"` 回 400 → `Err(TranslateError::Config{..})`，只送一次。
  - log：`[translate] endpoint rejected thinkingBudget=0 (model=…); retrying without thinkingConfig`。

- [ ] **Step 7: Commit**：`feat(translate): let Gemini services pick any thinking level or turn it off`

---

### Task 5: OpenAI 相容 client 的送出規則與被拒處理

**Files:**
- Modify: `src/translation/clients/openai.rs`（`new`、`request_body`、`chat` 的重送迴圈）
- Modify: `src/translation/clients/mod.rs`（`build_client`）
- Test: `tests/it/clients_openai.rs`、`tests/it/clients_golden.rs`（只改建 client 的呼叫）

**Interfaces:**
- Produces: `OpenAiCompatClient::new(base_url: &str, api_key: &str, model: &str, thinking: bool, effort: &str, official: bool) -> Self`。

- [ ] **Step 1: 寫失敗測試**：
  - `thinking_on_level_sends_reasoning_effort`：official 與自訂各一，`thinking=true, effort="high"` → `reasoning_effort == "high"`，且沒有 `chat_template_kwargs`／`think`／`enable_thinking`。
  - `thinking_on_auto_sends_no_reasoning_effort`：`thinking=true, effort="auto"` → 沒有 `reasoning_effort`。
  - `rejected_user_effort_is_an_error`：`thinking=true, effort="minimal"`，wiremock 回 400 指名 `reasoning_effort` → `Err(TranslateError::Config{status: Some(400), ..})`，只送一次。
  - `rejected_none_is_still_dropped`：`thinking=false` 回 400 指名 `reasoning_effort` → 第二次請求沒有 `reasoning_effort`，成功（既有行為，若已有同義測試則沿用）。

- [ ] **Step 2: 跑測試確認失敗**：`cargo test -j 4 --test it -- clients_openai clients_golden --test-threads=4`。

- [ ] **Step 3: 實作**：
  - `request_body`：`thinking` 為真且深度不是 `auto` 時送 `reasoning_effort`；關閉時照舊。
  - `chat` 迴圈：被拒參數是 `reasoning_effort` 且 `self.thinking` 為真時，不呼叫 `learn_rejection`，直接回錯誤，並記 `[translate] endpoint rejected the chosen reasoning_effort={effort} (model=…): {detail}`。
  - 結構文件註解補一句：使用者選的深度不在「聽伺服器的」範圍內。

- [ ] **Step 4: 跑測試確認通過**；`clients_golden` 的 16 個 case 輸出不變。

- [ ] **Step 5: Commit**：`feat(translate): send the chosen reasoning effort and surface its rejection`

---

### Task 6: 模型清單帶回思考能力

**Files:**
- Modify: `src/translation/clients/mod.rs`（新型別、`Client::list_models`）
- Modify: `src/translation/clients/claude.rs`、`gemini.rs`、`openai.rs`（`list_models`）
- Modify: `src/translation/translator.rs:461`（`list_models` 回傳型別）
- Modify: `src/ui/settings/env.rs:58`（`SettingsEvent::ModelsDone` 的 `result` 型別）
- Modify: `src/ui/settings/service_form.rs`（`listed` 存清單；表單其他行為不動）
- Test: `tests/it/clients_claude.rs`、`tests/it/clients_gemini.rs`、`tests/it/clients_openai.rs`、`tests/it/service_form.rs`、`tests/layout_audit.rs`（只改型別）

**Interfaces:**
- Produces（`translation::clients`）：
  ```rust
  /// 模型清單的一筆：id 與清單透露的思考能力。
  #[derive(Clone, Debug, PartialEq, Eq)]
  pub struct ModelInfo { pub id: String, pub thinking: ThinkingCaps }

  /// 清單透露的思考能力；`None` 是清單沒說。
  #[derive(Clone, Debug, Default, PartialEq, Eq)]
  pub struct ThinkingCaps {
      pub supported: Option<bool>,
      pub can_disable: Option<bool>,
      /// 支援的深度等級（不含 auto）；`Some(vec![])` 是不接受任何深度。
      pub efforts: Option<Vec<String>>,
  }
  ```
  - `list_models` 一律回 `Result<Vec<ModelInfo>, TranslateError>`，依 id 排序。
  - `ServiceForm` 內部改存 `models: Vec<ModelInfo>`；給 `editable_combo` 的 id 清單由它導出。

- [ ] **Step 1: 寫失敗測試**（fixture 照 Task 1 case 5 的實際形狀）：
  - `claude_models_carry_thinking_caps`：
    - 一筆 `thinking.supported=true, types.disabled.supported=false, effort.supported=true`，只有 `low/medium/high` 為 true → `ThinkingCaps{supported:Some(true), can_disable:Some(false), efforts:Some(["low","medium","high"])}`；
    - 一筆 `effort.supported=false` → `efforts: Some(vec![])`；
    - 一筆沒有 `capabilities` → `ThinkingCaps::default()`。
  - `gemini_models_carry_thinking_flag`：`"thinking": false` → `supported: Some(false)`；沒有該鍵 → `None`；其餘欄位 `None`。
  - `openai_models_have_unknown_caps`：每筆 `ThinkingCaps::default()`。
  - 既有清單測試改成比對 `.id`。

- [ ] **Step 2: 跑測試確認失敗**：`cargo test -j 4 --test it -- clients_ service_form --test-threads=4`。

- [ ] **Step 3: 實作**：
  - 解析寬鬆：欄位缺或型別不對就是 `None`。
  - `[settings] model list fetched` 的 log 加 `with_thinking_caps=N`（`supported`、`can_disable`、`efforts` 任一為 `Some` 的筆數）。

- [ ] **Step 4: 跑測試確認通過**（`tests/layout_audit.rs` 也要編得過：`cargo test -j 4 --test layout_audit --no-run`）。

- [ ] **Step 5: Commit**：`feat(translate): read thinking capabilities from model lists`

---

### Task 7: 開關狀態判定與表單 UI

**Files:**
- Create: `src/ui/settings/thinking.rs`（純函式，`//!` 一行模組說明）
- Modify: `src/ui/settings/mod.rs`（宣告模組）
- Modify: `src/ui/settings/service_form.rs`（`thinking_row`、`effort_row`、`api_map`、模型變動時的深度重設、能力判定 log）
- Modify: `src/i18n/zh-TW.json`、`zh-CN.json`、`en-US.json`
- Test: `src/ui/settings/thinking.rs` 的 `#[cfg(test)]` 單元測試、`tests/it/service_form.rs`

**Interfaces:**
- Consumes: Task 2 的 `Provider::efforts`、`EFFORT_AUTO`；Task 6 的 `ThinkingCaps`、`ModelInfo`。
- Produces（`ui::settings::thinking`）：
  ```rust
  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub enum ThinkingSwitch { Unsupported, Always, Toggle }
  pub fn switch_state(caps: Option<&ThinkingCaps>) -> ThinkingSwitch;
  pub fn effort_options(provider: &Provider, caps: Option<&ThinkingCaps>) -> Vec<&'static str>;
  pub fn effective_thinking(switch: ThinkingSwitch, chosen: bool) -> bool;
  ```
  - `switch_state`：`supported == Some(false)` → `Unsupported`；否則 `can_disable == Some(false)` → `Always`；否則 `Toggle`（含 `None`）。
  - `effort_options`：`[EFFORT_AUTO]` 接上 `provider.efforts`；`caps.efforts` 為 `Some` 時只留其中有的。
  - `effective_thinking`：`Unsupported` → false、`Always` → true、`Toggle` → `chosen`。

- [ ] **Step 1: 寫 `thinking.rs` 單元測試**：
  - `unsupported_wins_over_can_disable`：`supported=Some(false), can_disable=Some(true)` → `Unsupported`。
  - `cannot_disable_is_always`、`unknown_is_toggle`（`None` 與 `ThinkingCaps::default()`）。
  - `options_start_with_auto_and_follow_caps`：Claude＋`efforts=Some(["low","high"])` → `["auto","low","high"]`；Claude＋`efforts=Some([])` → `["auto"]`；Gemini＋`None` → `["auto","minimal","low","medium","high"]`。
  - `caps_level_unknown_to_provider_is_ignored`：Gemini＋`efforts=Some(["xhigh"])` → `["auto"]`。

- [ ] **Step 2: 跑確認失敗，實作 `thinking.rs`，跑確認通過**：`cargo test -j 4 --lib ui::settings::thinking -- --test-threads=4`。

- [ ] **Step 3: 寫 `tests/it/service_form.rs` 的失敗測試**（以 `refresh_models` 餵 `ModelInfo`，經 `tr(key)` 取字）：
  - `every_provider_shows_the_switch`：四家表單都有 `field.thinking` 開關。
  - `effort_hidden_while_switch_off`：開關關閉時找不到 `field.effort`，開啟後出現。
  - `unsupported_model_locks_switch_off`：模型 id 對到 `supported=Some(false)` → 開關為關、禁用，顯示 `hint.thinking_unsupported`，`values()["thinking"] == false`。
  - `always_thinking_model_locks_switch_on`：`can_disable=Some(false)` → 開、禁用，顯示 `hint.thinking_always`，深度選單出現。
  - `caps_follow_the_typed_model_id`：清單有 A（Always）、B（無能力）；模型欄打 A → 鎖定；改成 B → 可切換；改成清單沒有的 C → 可切換。
  - `locked_switch_restores_the_users_choice`：使用者選關閉 → 換成 Always 的模型（`values()` 為 true）→ 換回 Toggle 模型（`values()` 為 false）。
  - `effort_resets_when_new_model_lacks_it`：選 `xhigh` → 換到 `efforts=Some(["low"])` 的模型 → `values()["effort"] == "auto"`。
  - `effort_hint_text`：深度選單下方顯示 `hint.effort`。

- [ ] **Step 4: 跑確認失敗**：`cargo test -j 4 --test it -- service_form --test-threads=4`。

- [ ] **Step 5: 實作表單**：
  - `ServiceForm` 保留使用者的 `thinking`；畫面與 `api_map` 用 `effective_thinking`。
  - 開關禁用用 `add_enabled_ui(false, ..)`；提示依狀態選 `hint.thinking_unsupported`／`hint.thinking_always`／`hint.thinking`。
  - 深度選單在 `effective_thinking` 為真時畫，項目用 `effort_options`。
  - 每幀算出（模型 id → 能力 → 狀態＋選項），與上次記錄的不同才記 log：`[settings] thinking caps resolved (provider=…, model=…, switch=unsupported|always|toggle|unknown, efforts=[…])`。`unknown` 是沒對到清單的 `Toggle`。
  - 目前深度不在選項裡時改回 `auto`，記 `[settings] effort reset to auto (model=…, was=…)`。

- [ ] **Step 6: 字串**（三份語系檔，新增與修改）：

| key | zh-TW | zh-CN | en-US |
|---|---|---|---|
| `hint.thinking_always`（新） | 此模型一律會進行思考，無法關閉。 | 此模型始终会进行思考，无法关闭。 | This model always thinks; thinking can't be turned off. |
| `hint.thinking_unsupported`（新） | 此模型不支援思考。 | 此模型不支持思考。 | This model doesn't support thinking. |
| `hint.effort`（改） | 各模型支援的思考深度不同，選到不支援的思考深度時，測試會顯示錯誤；如果您不確定可選「自動」。思考深度越高，翻譯越慢、越耗 Token。 | 各模型支持的思考深度不同，选到不支持的思考深度时，测试会显示错误；如果您不确定可选“自动”。思考深度越高，翻译越慢、越耗 Token。 | Models support different thinking depths. If you pick one the model doesn't support, the test shows an error; if you're not sure, choose "Auto". Higher depths make translations slower and use more tokens. |
| `effort.minimal`（新） | 最低（minimal） | 最低（minimal） | Minimal (minimal) |
| `effort.low`（改） | 低（low） | 低（low） | Low (low) |
| `effort.medium`（新） | 中（medium） | 中（medium） | Medium (medium) |
| `effort.high`（新） | 高（high） | 高（high） | High (high) |
| `effort.xhigh`（新） | 極高（xhigh） | 极高（xhigh） | Extra high (xhigh) |
| `effort.max`（新） | 最高（max） | 最高（max） | Max (max) |

  等級名稱後面附 API 參數值；中文用全形括號（同 `field.thinking` 的「啟用模型思考（thinking）」）。`effort.auto`（自動）不送參數，不附。新 key 依現有檔內 key 的相鄰位置插入。

- [ ] **Step 7: 跑確認通過**：`cargo test -j 4 --test it -- service_form no_hardcoded_ui_text --test-threads=4`。

- [ ] **Step 8: Commit**：`feat(settings): show a thinking switch for every provider and lock it by model`

---

### Task 8: 開啟表單時自動載入模型清單

**Files:**
- Modify: `src/ui/settings/service_form.rs`（`start_refresh` 加觸發來源、首幀與失焦觸發）
- Test: `tests/it/service_form.rs`、`tests/it/wizard.rs`

**Interfaces:**
- Produces: `fn start_refresh(&mut self, env: &dyn SettingsEnv, trigger: Trigger)`，`enum Trigger { Auto, Manual }`（私有）；`ServiceForm` 記住上次自動載入時的（`api_key`, `base_url`）。

- [ ] **Step 1: 寫失敗測試**（用 `FakeEnv.lists` 數請求）：
  - `opening_a_filled_form_loads_models_once`：有金鑰的 OpenAI 服務，跑三幀 → `lists.len() == 1`。
  - `opening_an_empty_form_does_not_load`：金鑰空 → `lists.len() == 0`，狀態為 `hint.model_idle`，沒有錯誤文字。
  - `key_focus_loss_loads_once`：空表單輸入金鑰、還在焦點內 → 0；焦點移走 → 1；再移回再移走不改金鑰 → 仍 1。
  - `auto_load_without_model_list_keeps_switch_usable`：自訂端點，自動載入回 `NoModelList` → 顯示 `hint.model_no_list`，開關可切換。
  - `refresh_button_still_loads`：自動載入後按〔更新〕→ 2。
  - `wizard_service_step_auto_loads`（`tests/it/wizard.rs`）：精靈填完金鑰並離開欄位後 → 1。

- [ ] **Step 2: 跑確認失敗**：`cargo test -j 4 --test it -- service_form wizard --test-threads=4`。

- [ ] **Step 3: 實作**：
  - 第一次 `body()` 時若 `validate_endpoint_fields` 通過就自動載入。
  - 金鑰欄與伺服器網址欄 `lost_focus()` 時，若（金鑰, 網址）與上次自動載入不同且通過驗證，就再載入。
  - 不通過驗證時不設 `ModelStatus::Error`（與按〔更新〕不同）。
  - log 的 `[settings] model list fetched`／`failed` 加 `trigger=auto|manual`。

- [ ] **Step 4: 跑確認通過**。

- [ ] **Step 5: Commit**：`feat(settings): load the model list when a service form opens`

---

### Task 9（只在 Task 1 case 4 有默默忽略時）：測試連線抓「關了仍在思考」

**Files:**
- Modify: `src/translation/errors.rs`（新錯誤與 `friendly_error` 對應）
- Modify: `src/translation/clients/gemini.rs`、`openai.rs`（讀思考 token）
- Modify: `src/translation/translator.rs`（`test_translate`）
- Modify: 三份語系檔
- Test: `tests/it/clients_gemini.rs`、`tests/it/clients_openai.rs`、`tests/it/translator.rs`

**Interfaces:**
- Produces: `TranslateError::ThinkingNotDisabled { model: String, thoughts: u64 }`，`friendly_text` → `error.thinking_not_disabled`。
- 行為：
  - 只有測試連線（`test_translate`）會回這個錯誤；一般翻譯照常回譯文，只記 `[translate] thinking was off but model still thought: provider=… model=… thoughts=…`。
  - 讓 client 知道「這次是測試」的接線方式由實作者決定，限制兩條：不改 `ChatBackend::chat` 的既有呼叫端行為；不在一般翻譯路徑多做請求。

- [ ] **Step 1: 寫失敗測試**：
  - `gemini_off_with_thoughts_fails_the_test`：`thinking=false`，串流 `usageMetadata.thoughtsTokenCount=37` → `test_translate` 回 `ThinkingNotDisabled{thoughts:37,..}`。
  - `gemini_off_with_thoughts_still_translates_normally`：同條件一般翻譯成功。
  - `openai_off_with_reasoning_tokens_fails_the_test`：`usage.completion_tokens_details.reasoning_tokens=12`（official）→ 同上。
  - `thinking_on_with_thoughts_is_fine`：`thinking=true` 時不報錯。

- [ ] **Step 2: 跑確認失敗 → 實作 → 跑確認通過**：`cargo test -j 4 --test it -- clients_gemini clients_openai translator --test-threads=4`。

- [ ] **Step 3: 字串**：

| key | zh-TW | zh-CN | en-US |
|---|---|---|---|
| `error.thinking_not_disabled` | 此模型無法關閉思考，請開啟思考後再測試。 | 此模型无法关闭思考，请开启思考后再测试。 | This model can't turn thinking off. Turn thinking on and test again. |

- [ ] **Step 4: Commit**：`feat(settings): fail the connection test when thinking cannot be turned off`

---

### Task 10: 全套驗證與實機驗收

**Files:**
- Modify: `docs/superpowers/specs/2026-10-23-thinking-settings-design.md`（驗收結果附在「驗收」節）

- [ ] **Step 1: 全套檢查**：`cargo fmt --check`、`cargo clippy -j 4 --all-targets -- -D warnings`、`cargo test -j 4 -- --test-threads=4`，全部零錯誤、全綠。

- [ ] **Step 2: 真實 API 驗收**（沿用 Task 1 的腳本，改成建 `Translator` 走 `test_translate`，不開 GUI）：

| 服務商 | 模型 | 開關／深度 | 預期 |
|---|---|---|---|
| OpenAI | 支援 `none` 的模型 | 關 | 成功 |
| OpenAI | GPT-6.1 Sol | 開＋`minimal` | 報錯（設定錯誤） |
| Claude | Haiku 5.5 | 關 | 成功 |
| Claude | Opus 5.5 | 從清單判定 | `switch_state == Always` |
| Gemini | 2.5 Flash | 關 | 成功 |
| Gemini | 3.1 Pro | 開＋`minimal` | 報錯 |
| 自訂（llama-swap） | 使用者目前的模型 | 開＋`low`／關 | 與 Task 1 case 7 一致 |

- [ ] **Step 3: 請使用者開設定視窗實際操作一次**（不自行操控桌面）：Claude 換模型時開關的鎖定、深度選單項目、自動載入。結果記到 spec。

- [ ] **Step 4: Commit**：`docs(spec): record acceptance results for thinking settings`
