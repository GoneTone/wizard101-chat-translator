# 第五輪稽核修正計畫（2026-10-21）

來源：補查三家 AI 服務的串流解析（`src/translation/clients/`）、選服務商畫面（`src/ui/settings/provider_picker.rs`）與 `src/ui/settings/theme.rs`。標 ✔ 的主控已對照原始碼確認。使用者決定全部處理，報讀軟體念說明那項不做。分支 `fix/audit-5`，從 `refactor/rust-rewrite`（295d95f）切出。

## Global Constraints（每個 task 都適用）

- 先讀 `CLAUDE.md`。實作註解預設一行、只寫 WHY，三行是上限不是目標；docstring 一行摘要；CJK 全形標點；log 一律英文、沿用 `[translate]` 等前綴並帶足 context（provider、model、狀態碼、finish_reason）；不可寫死語言；API 金鑰不可進 log 或錯誤訊息。
- 行號是查證當下的參考值，動手前以實際程式為準；描述與程式不符、或查證後問題不存在時，以程式為準並在回報說明（不存在就不改）。涉及 reqwest／hyper-util 行為時看 `~/.cargo/registry` 裡實際版本的原始碼。
- 修 bug 先寫會失敗的測試、確認失敗，再修。純精簡靠現有測試把關。golden 檔（`tests/golden/*.json`）不改；若某個 golden 依賴被改的行為，停下回報。
- 測試不寫死翻譯句子：UI 文字一律經 `t(key)`／`tr(key)` 取值。
- 新增 UI 字串：每個有翻譯到該區的語系檔都要補，句尾標點跨語言一致（英文有句號，中文也要有全形「。」）。
- 測試不可碰真實桌面、鍵盤 hook、剪貼簿、執行中的遊戲或真實網路（沿用 `tests/it` 的本機假伺服器，如 `stream_server`）；不可啟動 GUI 或截圖。新的整合測試放 `tests/it/`。
- 每個 task 結束前：`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、完整 `cargo test` 各跑一次，全綠才 commit。迭代時只跑聚焦測試。
- Commit：英文 conventional commits，可分多個；結尾附派工說明給的 attribution 兩行。只 `git add` 改到的檔案。不 push。

---

## Task 1：代理、錯誤分類與重試時機（clients／net／pool／errors）

1. ✔ **Windows 系統代理被忽略**：`Cargo.toml` 的 reqwest 關掉預設功能，連 `system-proxy` 一起關了，`Matcher::from_system()` 只讀 `HTTP(S)_PROXY` 環境變數，不讀 Windows 登錄檔的系統代理（Clash／v2rayN「系統代理」模式就是設這裡）。Python 版（httpx → `urllib.request.getproxies()`）會讀，這是退步。修法：reqwest 加 `system-proxy` feature。確認 `net::bypass_proxy_for_loopback` 對 loopback 一律 `no_proxy()`（目前就是），本機 LLM 伺服器不受影響；它的 log 文字只提環境變數，改成不限來源的說法。補測試：loopback 網址建出的 client 不走代理的既有測試照過；能測就加一個「設定了代理時非 loopback 會走代理」的本機測試（用假代理伺服器），測不了就在回報說明。
2. ✔ **串流中途錯誤沒有數字 code 就被丟掉**：`clients/mod.rs` `chunk_error` 只用 `as_u64` 讀 `code`，OpenAI 標準伺服器錯誤 `{"code": null, "type": "server_error"}`、OpenRouter 的 `"code": "server_error"` 都變 `Unexpected`，pool 直接丟掉那句不重試。修法：`code` 接受數字或數字字串；否則把 `type`（或字串 `code`）經狀態對照表轉成狀態碼。對照表目前在 `claude.rs`（`server_error`／`api_error` → 500、`rate_limit_error` → 429、`overloaded_error` → 529 等），搬到 `clients/mod.rs` 三家共用，並補 `rate_limit_exceeded` → 429、`timeout_error` → 504（以實際表為準增補）。補測試：`code: null` + `type: server_error`、`code: "server_error"` 都成為可重試的錯誤。
3. **手改 config 的錯網址被當成斷線**：`clients/mod.rs` `connection_error` 把 `send()` 的所有 `reqwest::Error` 都當 `Offline`，包括網址無法解析的 builder 錯誤（例如 `localhost:1234/v1` 被解析成 scheme `localhost`），結果亮離線橫幅、重試 5 分鐘。修法：`error.is_builder()` 時回 `TranslateError::Config { status: None, detail: error_chain(error) }`。補測試。
4. **只有 Claude 照 `Retry-After` 等待**：OpenAI 相容與 Gemini 的 429／503 變成 `Offline` 但沒有等待提示，pool 一律用 `BACKOFF_STEPS`（5／15／30 秒）。修法：
   - `TranslateError::Offline` 加可選的重試等待（例如 `retry_after: Option<Duration>`）；pool `note_failure` 用 `max(該步退避, retry_after)`，上限沿用 60 秒（`MAX_RETRY_AFTER` 搬到共用處）。
   - 把 Claude 的 `retry_after(headers)`（含 `retry-after-ms`、秒數、HTTP 日期）搬到 `clients/mod.rs` 共用；OpenAI 相容讀回應 header；Gemini 另讀錯誤 body 的 `RetryInfo.retryDelay`（如 `"37s"`）。
   - Claude 自己的 SDK 式重試照舊，最後一次失敗時也把等待時間帶給 pool。
   - log：pool 套用伺服器等待時間時記一行（含秒數與 provider）。
   - 補測試：OpenAI 429 帶 `retry-after: 20` 時 pool 下一次重試不早於 20 秒（用既有可控時鐘）；Gemini body 的 `retryDelay` 被讀到；超過 60 秒被夾住。
5. **精簡**：`clients/sse` 模組與其項目只在 `clients/` 內使用（grep 全樹含 tests 確認），改成 `pub(crate)` 或更窄。

## Task 2：串流完整性（clients／sse）

1. ✔ **被服務商擋下的回應被當成翻譯成功**：
   - Gemini（`gemini.rs`）只把 `MAX_TOKENS` 當失敗；`finishReason` 為 `STOP` 以外的值（`SAFETY`、`RECITATION`、`PROHIBITED_CONTENT`、`BLOCKLIST`、`SPII`、`OTHER` 等）時，已收到的半句被當成譯文，系統訊息還會進快取。修法：有 `finishReason` 且不是 `STOP`（`MAX_TOKENS` 照舊走截斷）→ 回 `BadOutput`，訊息帶 `finish_reason` 與樣本（沿用 `truncated` 的樣本寫法）。沒有候選、只有 `promptFeedback.blockReason` 時，把 `blockReason` 帶進錯誤／log（目前 log 成 `finish_reason=unknown`）。
   - OpenAI 相容（`openai.rs`）：`finish_reason == "content_filter"` 同樣回 `BadOutput`。
   - 補測試：各分支一個。
2. **串流沒收到結束訊號就斷了**：代理／閘道提前正常關閉連線時——OpenAI 沒有 `[DONE]` 也沒有 `finish_reason`、Gemini 沒有 `finishReason` → 半句當成功；Claude 沒有 `message_delta`／`message_stop` → `BadOutput`，pool 不重試；最後一行斷在半截 → `Unexpected`，也不重試。這些都是暫時性中斷。修法：
   - 各家追蹤是否收到結束訊號：OpenAI＝`[DONE]` 或任一非空 `finish_reason`；Gemini＝`finishReason`；Claude＝`message_stop` 或 `stop_reason`。沒收到 → `TranslateError::offline(...)`（訊息說明 stream ended before completion，帶 provider 與已收字數），log 一行。
   - `sse::for_each_data` 收尾時，最後一行半截而解析失敗 → 也當 `Offline`。
   - 既有測試多半已送 `[DONE]`／`STOP`；檢查以 `None` finish reason 組串流的 Gemini 測試，必要時讓它們帶上結束訊號（這是測試資料補齊，不是放寬斷言）。
   - 補測試：沿用 `stream_server`，加一個會送出 chunked 結尾 `0\r\n\r\n` 正常關閉的變體，三家各測「沒有結束訊號 → 可重試錯誤」。
3. **保持連線的空訊息讓請求永遠卡住**：reqwest 的讀取逾時每收到一個 frame 就重新計時（`reqwest-0.13.5/src/async_impl/body.rs` `ReadTimeoutBody`），閘道持續送 SSE 註解行（`: OPENROUTER PROCESSING`）或 `ping` 事件時請求永不結束、一直占著並行名額。修法：
   - `for_each_data` 對 `stream.next()` 套閒置期限（沿用 `clients` 既有的讀取逾時常數），只在送出一筆 `data:` 給呼叫端時重新計時，註解行、空行不算；Claude 的 `ping` 事件雖是 `data:`，也不應重新計時（由 Claude 端告知，或以事件類型判斷，擇簡單者）。逾時 → `Offline("stream idle ...")`，log 一行。
   - Claude 收到 `message_stop` 時回 `ControlFlow::Break` 主動停止讀取。
   - 補測試：假伺服器每隔一小段時間送註解行、從不送資料 → 在閒置期限後得到 `Offline`（測試裡把期限調短，或用可注入的期限；不要讓測試真的等 60 秒）。
4. **HTTP 200 但內容不是事件串流**：`sse.rs` 把非 `data:` 行都丟掉，閘道回 200 的錯誤 JSON（`{"error":{"message":"insufficient quota"}}`）、非串流的 `chat.completion` 或 HTML 頁面時，OpenAI／Gemini 回空字串 → 「模型沒有回傳文字」，Claude 回「stream ended without a message」，app.log 都沒有回應內容。修法：`for_each_data` 記下送出的 `data:` 行數，保留非 data 行的前約 200 bytes；結束時一筆 data 都沒有且樣本非空 → `Unexpected(format!("response is not an event stream: {}", error_detail(&sample)))`（沿用 `error_detail` 取 `error.message`、去 HTML）。補測試：200 + JSON 錯誤 body → 錯誤文字含 `insufficient quota`。

## Task 3：選服務商畫面（src/ui/settings/provider_picker.rs、theme.rs）

1. **長文字換行沒有測試**：卡片高度與文字寬度是手算的（`CardText::new`／`height`、格狀列高取最大值），沒有測試檢查各語言的標題／說明是否超出卡片。補一個測試：逐一切換每個有翻譯的介面語言（照 `tests/` 裡會切換全域語言的測試的做法——獨立測試檔或語言鎖），兩種排版（格狀、清單）、實際寬度（精靈 600、對話框實際寬度），斷言標題與說明的 galley 都在卡片 rect 內，格狀同一列卡片等高。若測試抓到真的超出，一併修。
2. **邊距數字重複**：`CardText::new`／`CardText::height` 與 `provider_card` 各寫一次 14、12、16、箭頭寬等數字，只改一邊就會讓文字壓到箭頭或高度算錯。修法：抽成具名常數（如 `LIST_PAD`、`LIST_GAP`、`ARROW_W`、`GRID_PAD`），量與畫共用。
3. **字型大小寫死**：清單標題 `14.0`（等於 `theme::TEXT_SIZE`）改用常數；格狀 `14.5` 給一個 theme 常數或統一成同一個大小（擇外觀不變者）。
4. **每幀重複查翻譯字串**：`provider_card` 的無障礙標籤再呼叫一次 `t(provider.label_key)`，改用 `CardText` 已算好的標題文字。
5. **誤導的寫法**：
   - 說明 galley 繪製時傳入的後備顏色 `theme::TEXT` 實際不生效（galley 自帶 `TEXT_SECONDARY`），改傳 `theme::TEXT_SECONDARY`（或 `Color32::PLACEHOLDER`），畫面不變。
   - `theme.rs` 的 `HOT_BG` 說明只寫「服務商卡片 hover」，實際也用在元件 hover／active 底色（`apply`、`fields.rs`、`hotkeys.rs`），改寫說明。
