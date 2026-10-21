# 規則

- 回答使用**繁體中文 (台灣)**。
- **嚴禁重複造輪子**：動手寫任何工具函式前，先確認專案內有沒有現成可用的；有則直接用，不要再寫一個類似的。若某段邏輯散在別處可共用，抽出來共用。
- **專案要好維護**：架構清楚、讓其他協助開發者一眼看懂在做什麼；維持模組單一職責、小而聚焦（reader／composer／translator 各司其職，透過清楚介面溝通、可獨立測試）。需要時可引入套件寫得更簡潔，不要自行造輪子。
- **翻譯語言不可寫死**：本工具支援使用者設定收訊的**目標語言**（config 頂層 `target_language`，人讀名稱字串直接帶入提示詞）；發話固定翻成英文（`translation::prompts::OUTGOING_LANGUAGE`）；來源語言一律自動判斷。**程式碼命名、註解、提示詞邏輯都不可寫死 `zh`／`en`／繁體中文之類** —— 用方向命名（`translate_incoming`／`translate_outgoing`）、把語言當參數／常數傳入。
- **新增 AI 服務商要處理 prompt cache**：先查該家的快取是自動生效，還是要明確標記（Claude 要帶 `cache_control`，OpenAI 自動），以及最低可快取長度。要標記時，斷點放在固定前綴（系統提示詞）結尾，不要放在最後 —— 後面的上下文每句都變，放在最後只會一直寫入、讀不到；每句會變的內容也不可放進斷點之前。沿用 `[translate] prompt cache:` log 記下快取用量（見 `src/translation/clients/`）。
- **記憶體掛入須知**：收訊靠自訂 AddLine hook（`src/reader/hook/add_line.rs` 的 `PATTERN`）讀聊天事件，輸入框偵測與分頁狀態靠 `src/reader/hook/cave.rs` 的 root_window hook（`ROOT_WINDOW_PATTERN`）；不啟 player／duel／quest 等其他 hook。掛入需遊戲已**登入進世界內**；本專案不依賴 wizwalker 或其 fork，遊戲改版報 `PatternFailed` 時，要更新上述對應檔案裡的 pattern（AddLine 那組需重新反組譯 `ChatBox::AddLine`）。只跑純解析函式的測試不需遊戲，整合行為靠實機驗證。
- **註解節制使用**：預設不寫實作層 `//` 註解；寫了就要有資訊增益。可寫的情境：(1) **WHY 不顯而易見** —— 隱藏限制、微妙 invariant、bug workaround、會讓讀者意外的行為；(2) **結構/段落導引** —— 在較長函式內標出段落意圖。判準：拿掉註解後讀者是否需要多花時間理解？需要 → 留，不需要 → 刪。**預設一行**，只寫 WHY，不敘述做了什麼；真的講不清才用到兩三行，三行是上限不是目標。反例：純粹重述下一行、檔頭路徑 banner、被註解掉的舊程式碼（直接刪）、變更歷史與設計取捨的長篇論述（那些放 spec／設計文件，不進程式碼）。
- **模組與公開項目寫文件註解**：模組頂端用 `//!`、公開的 fn／struct／enum／trait 用 `///` 說明其作用（沿用現有風格，如 `src/translation/translator.rs`／`src/reader/mem_reader.rs`）；簽名已自明的極簡函式可略。**長度：一行摘要**；只有呼叫者必須知道的非顯而易見限制才多加一個短段落（幾行內）。測試的文件註解一行即可。
- **標點依語言慣例**：CJK 語境（繁體中文 (台灣)、日文等）一律用全形標點 `，。！？；：（）「」『』、——`，涵蓋註解、docstring、UI 文字、README、設計文件。英文／純程式碼語境維持半形 `. , ( ) ! ? : ;`。中英混排以**主語言**為準（中文句子內含英文短語，標點仍用全形）。破折號 `——` 前後各留一個半形空格（`本軟體 —— 疊在遊戲上`），行尾的 `——` 不補尾隨空格。
- **新功能要埋 log**：實作新功能或修改既有邏輯時，在關鍵節點（記憶體掛入／讀取、hook 修復、翻譯 API 請求與錯誤分支、設定套用、重試／重置判定等）呼叫 `log(...)`（`src/logs/mod.rs`）記錄 —— 打包版寫進 exe 旁的 `app.log`，原始碼執行時寫到終端機（stderr）；不要直接 `println!`／`eprintln!`，打包版沒有主控台，這些輸出會直接消失。內容要帶足夠 context（行數、HTTP 狀態碼、provider、PID、例外訊息等）讓使用者匯出 `app.log` 後能直接定位問題，而不是 `"failed"` 這種沒有上下文的訊息。**log 訊息內容一律用英文**（方便搜尋、不受 locale 影響）：涵蓋所有寫進 stdout／stderr 的訊息字串（`key=value` 診斷欄位同理）；**唯獨程式碼註解／docstring 與 UI 顯示文字不受此條影響** —— 註解依「標點依語言慣例」用中文全形，overlay 橫幅／設定視窗／錯誤提示等使用者看得到的介面文字維持繁體中文。log 前綴對齊既有慣例（`[reader]`／`[translate]`／`[ui]` 等模組名）。**敏感資料（API 金鑰）絕不可寫入 log**。
- **Commit message 與 PR 標題一律英文**：commit message（subject 與 body）與 PR 標題使用英文、半形標點，沿用 conventional commits 格式（`feat(...)`、`fix(...)`、`docs(...)`、`chore(...)`、`refactor(...)` 等），subject 簡潔。PR 描述、設計文件、程式碼註解／docstring、UI 文字（overlay 橫幅／設定視窗等）不受此條影響，仍依「標點依語言慣例」維持繁體中文全形。
- **合併 PR 保留歷史**：PR 一律以 merge commit 合併（`gh pr merge --merge`，或 GitHub 上選 Create a merge commit），**不 squash、不 rebase** —— 每個 task 的 commit 與審查修正都要留在 `master` 的歷史裡可追。
- **簡單改動免 spec**：純字串／提示詞小改、單檔 typo、單一 bug fix、純機械重構（重新命名、抽常數）、config 欄位增減等，可直接動手，不必走 brainstorming → spec → plan；使用者已自帶明確設計判斷（what + why）時同樣免 spec。需要走流程的判準：跨多檔影響架構、新增元件／資料流、驗收條件超過「`cargo test` 全綠」的功能。
- **版本號單一真實來源**：`Cargo.toml` 的 `version`（程式內一律讀 `env!("CARGO_PKG_VERSION")`，不另存一份）。SemVer、tag 用 `v` 前綴、發布版本的步驟見 `docs/releasing.md`。
- **用 `cargo`**：本專案以 cargo 管理（`Cargo.toml` + `Cargo.lock`），toolchain 由 `rust-toolchain.toml` 鎖定。建置用 `cargo build`（發行版 `cargo build --release`）；執行用 `cargo run`；lint 用 `cargo fmt --check` 與 `cargo clippy --all-targets -- -D warnings`；測試用 `cargo test`。翻譯設定與 API 金鑰在 `config.json`（gitignore、不進版控）。
- **整合測試放 `tests/it/`**：新的整合測試寫成 `tests/it/` 底下的模組（在 `tests/it/main.rs` 宣告），不要在 `tests/` 另開檔 —— 每個 `tests/*.rs` 都會各自連結整個相依樹，檔案一多完整 `cargo test` 就回到十幾分鐘。例外只有會切換全域介面語言的測試：它們跟同程序的其他測試互相干擾，才在 `tests/` 獨立成檔，並在檔頭註明原因。

## 提交前品質檢查

執行 `git commit` 前：

1. **Lint**：`cargo fmt --check` 與 `cargo clippy --all-targets -- -D warnings` — 必須零錯誤、零警告。
2. **測試**：`cargo test` — 必須全部通過。
3. 若改到收訊／掛入相關，盡量開著遊戲（登入進世界內）`cargo run` 實跑一次確認。

任何一項失敗就先修，不要用 `--no-verify` 跳過。**不要主動 git push。**

## YAGNI 原則（You Ain't Gonna Need It）

只實作當前需要的功能，不要預先設計「未來可能用到」的東西。

| ❌ 不要          | ✅ 應該    |
|---------------|---------|
| 預先建立抽象層或介面    | 需要時再抽象  |
| 加入「以後可能用到」的參數 | 只加必要參數  |
| 過度設計可擴展架構     | 簡單直接的實作 |
| 預留未使用的設定選項    | 有需求再加   |
