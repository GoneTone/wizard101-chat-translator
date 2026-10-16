# Rust 重寫第 1 段（基礎＋翻譯核心＋收訊＋overlay）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 `refactor/rust-rewrite` 分支做出一個 Rust 版 exe：讀使用者現有的 `config.json`，掛入遊戲收訊、翻譯，並在 overlay 顯示，行為與 Python v0.6.0 一致。

**Architecture:** 單一 crate，放在過渡目錄 `rust/`（lib `wizard101_chat_translator`＋bin `Wizard101ChatTranslator`），模組與 Python 套件同名。執行緒分工：
- reader 用 `std::thread`，每個客戶端一條；
- 網路用 `tokio` runtime；
- UI 用 `eframe`／`egui`，跑在主執行緒，其他執行緒透過 `UiEvent` channel 把事件交給它。

純邏輯以 Python 產生的 golden fixture 對照，確保行為一致。

**Tech Stack:** Rust（edition 2024、rustc 1.95）、`windows`、`eframe`/`egui`（wgpu 後端）、`tokio`、`tokio-util`（CancellationToken）、`reqwest`（rustls、stream）、`serde`/`serde_json`（`preserve_order`）、`regex`、`chrono`、`sha1`；測試用 `wiremock`、`tempfile`、`egui_kittest`。各 crate 取撰寫當下最新的穩定版，鎖在 `Cargo.lock`。

**Spec:** `docs/superpowers/specs/2026-10-17-rust-rewrite-design.md`（以下簡稱「spec」，§ 指功能對照清單的章節）。每個 task 的行為以「對應的 Python 原始碼」為準；spec 已註明舊 spec 與程式碼衝突時，以程式碼為準。

## Global Constraints

- Rust crate 放在 `rust/`；Python 程式碼在本段**一行都不刪、不改行為**（第 5 段才刪）。
- 每次 commit 前，以下全部都要通過：
  - `uv run ruff check src tests tools`
  - `uv run pytest`
  - `cargo fmt --check --manifest-path rust/Cargo.toml`
  - `cargo clippy --manifest-path rust/Cargo.toml --all-targets -- -D warnings`
  - `cargo test --manifest-path rust/Cargo.toml`

  不可以用 `--no-verify`。為了不干擾執行中的遊戲，Python 全套測試每個 task 只跑一次。
- **log**：
  - 一律呼叫 `crate::logs::log(...)`，不直接 `println!`／`eprintln!`。
  - 訊息內容用英文；前綴沿用 Python 的模組名（`[app]`、`[reader]`、`[translate]`、`[ui]`、`[config]`、`[update]`、`[settings]`）。
  - API 金鑰絕不可寫進 log。
  - 文字盡量與 Python 的同一行 log 相同，方便使用者比對新舊 `app.log`。
- **註解與 doc comment**：
  - 模組頂端用 `//!`、公開項目用 `///`，寫一行繁體中文摘要；只有呼叫者必須知道的限制才多加一個短段落。
  - 實作註解預設不寫；要寫就只寫 WHY，一行為原則，三行是上限。
  - 中文用全形標點，英文用半形。
- 不可寫死語言：用方向命名（`translate_incoming`／`translate_outgoing`）；發話語言是常數 `prompts::OUTGOING_LANGUAGE = "English"`。
- 介面文字一律走 `i18n::t`，沿用 `src/i18n/*.json` 的 key（`include_str!` 編進 exe），**不新增 key**。
- 路徑與格式：
  - `local_state_dir()` = `%LOCALAPPDATA%\wizard101-chat-translator`。
  - `app_dir()`：release 建置是 exe 所在資料夾；debug 建置是 repo 根目錄（`concat!(env!("CARGO_MANIFEST_DIR"), "/..")`），與 Python 開發時共用同一份 `config.json`。
  - 所有持久化檔案的格式要與 Python 寫出的**逐 byte 相同**，包括換行。例如 `config.json` 由 Python `Path.write_text` 寫成，在 Windows 上是 CRLF。
- **測試不可干擾使用者桌面**：不開可見視窗、不搶前景、不送按鍵。UI 測試用 `egui_kittest`（headless）；Win32 視窗行為列入實機驗收。
- **移植測試**：Python 端每個對應的測試，在 Rust 端都要有同等測試，或已被 golden 涵蓋。Tk 專屬、無法對應的測試，在該 task 的 commit body 列出名稱與理由。
- Commit message 用英文 conventional commits，結尾附上 session 規定的 `Co-Authored-By` 與 `Claude-Session` 兩行。

## Review Focus

1. **遊戲或本程式在 hook 掛著時被強制結束**：下次啟動要依 hookstate 檔修復，**包含 Python 版寫的檔案**。測試放在 Task 14：`repairs_from_python_written_hookstate`。
2. **環形緩衝的 1000 字截斷把 UTF-16 surrogate pair 切半**，或長度是奇數 byte：解碼不 panic，結果與 Python `decode("utf-16-le", "replace")` 相同。測試放在 Task 15：`ring_record_split_surrogate_matches_python`。
3. **reader 執行緒 panic**：仍要釋放 slot、解除 hook，與 Python 的 `finally` 相同。測試放在 Task 17：`reader_panic_still_drops_slot_and_closes`。
4. **舊 `config.json` 存的 overlay／泡泡座標落在目前所有螢幕外**（高 DPI 的邏輯座標、拔掉的第二螢幕）：拉回可見範圍、不改大小。測試放在 Task 19：`offscreen_geometry_is_pulled_back`。
5. **1000 則訊息時拖曳縮放 overlay**：每幀排版不可卡頓。測試放在 Task 21：`relayout_1000_messages_within_budget`。

---

### Task 1: Crate 骨架、golden 匯出機制、CI

**Files:**
- Create: `rust/Cargo.toml`, `rust/src/lib.rs`, `rust/src/main.rs`, `rust/tests/common/mod.rs`, `rust/.gitignore`（`target/`）
- Create: `tools/golden/__init__.py`, `tools/golden/export.py`, `tests/tools/test_golden_export.py`
- Modify: `.github/workflows/ci.yml`（新增 `rust` job）

**Interfaces:**
- Produces:
  - Python 端：`tools.golden.export.EXPORTERS: dict[str, Callable[[], list[dict]]]`。每個 exporter 回傳 `[{"input": ..., "output": ...}, ...]`。
  - CLI：
    - `uv run python -m tools.golden.export [name ...]`：寫出 `rust/tests/golden/<name>.json`（UTF-8、`ensure_ascii=False`、`indent=1`、LF）。
    - `--check`：重新產生後與已提交的檔案比對，不同就以非零結束。
  - Rust 端：`rust/tests/common/mod.rs` 提供 `pub fn golden(name: &str) -> Vec<serde_json::Value>`，讀 `tests/golden/<name>.json`。
  - 其他 task 都透過「在 `EXPORTERS` 加一項、產生 fixture、Rust 測試讀它」來對照。

- [ ] **Step 1:** 寫 `tests/tools/test_golden_export.py::test_check_detects_drift`：用 `monkeypatch` 註冊一個回傳 `[{"input": 1, "output": 2}]` 的假 exporter，輸出目錄指到 `tmp_path`。
  - 先 export 一次，再把檔案內容改掉；
  - 斷言 `main(["--check", "fake"])` 回傳非零；
  - 未修改時回傳 0。
- [ ] **Step 2:** `uv run pytest tests/tools/test_golden_export.py -n 0`，預期 FAIL（模組不存在）。
- [ ] **Step 3:** 實作 `tools/golden/export.py` 的 `main(argv: list[str] | None = None, out_dir: Path = ...) -> int` 與 `EXPORTERS = {}`。
- [ ] **Step 4:** 再跑同一個測試，預期 PASS。
- [ ] **Step 5:** 建立 `rust/Cargo.toml`：
  - `name = "wizard101-chat-translator"`、`version = "0.6.0"`、`edition = "2024"`；
  - `[[bin]] name = "Wizard101ChatTranslator", path = "src/main.rs"`；
  - 相依先只加 `serde_json`（開 `preserve_order`）。

  `main.rs` 開頭加 `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`，`main` 先留空。`rust/tests/common/mod.rs` 實作 `golden()`。
- [ ] **Step 6:** CI 新增 `rust` job（`runs-on: windows-latest`），依序執行：
  1. `dtolnay/rust-toolchain@stable`（含 clippy、rustfmt）
  2. `Swatinem/rust-cache`（`workspaces: rust`）
  3. `cargo fmt --check`
  4. `cargo clippy --all-targets -- -D warnings`
  5. `cargo test`

  前三個 cargo 指令都在 `rust/` 目錄執行。另一步跑 `uv sync --locked && uv run python -m tools.golden.export --check`，讓 master 改到 Python 行為時，分支 CI 會報出 fixture 過期。
- [ ] **Step 7:** 跑 Global Constraints 的五項檢查，全部通過後 commit：`chore(rust): scaffold Rust crate, golden fixture export and CI job`。

### Task 2: logs

**Files:**
- Create: `rust/src/logs/mod.rs`, `rust/src/logs/files.rs`
- Test: 同檔的 `#[cfg(test)]`，加上 `rust/tests/logs_golden.rs`
- Modify: `tools/golden/export.py`（新增 `logs_trim`、`logs_stamp`）

**Interfaces:**
- Produces:
  - `pub fn log(message: impl AsRef<str>)`：加上時戳後寫到目前的 sink。執行緒安全，每行獨立寫出。
  - `pub fn init(sink: Sink)`，其中 `pub enum Sink { Stderr, File(std::fs::File) }`。
  - `pub fn utc_stamp(now: DateTime<Utc>) -> String`：格式 `2026-10-17T01:02:03.456Z`。
  - `pub fn session_header(now) -> String`
  - `pub fn trim_log_sessions(text: &str, now) -> String`
  - `pub fn open_session_log(path: &Path, now) -> io::Result<File>`：先修剪再 append，寫入 session header。
  - `pub struct TimestampedWriter`：`messages.log` 用，每行加時戳。
  - `pub fn install_panic_hook()`：panic 時以 `[app] panic: ...` 加上 backtrace 寫進 log。
- 規則照 `src/logs/files.py`：
  - 保留 7 天；
  - 超過 5 MB 時先只留最後 1 MB；
  - header `===== session started YYYY-MM-DDTHH:MM:SSZ =====`；
  - 檔案用 CRLF。

- [ ] **Step 1:** golden exporter：
  - `logs_trim`：多組含新舊 session、超過 7 天、無 header 前言的文字，並固定 `now`，對 `trim_log_sessions` 取輸出；
  - `logs_stamp`：數個 datetime 的 `utc_stamp` 輸出。

  執行 export。
- [ ] **Step 2:** 寫 `rust/tests/logs_golden.rs`：
  - `trim_matches_python`、`stamp_matches_python`：逐筆斷言輸出相等；
  - 單元測試 `open_session_log_caps_oversized_file`：6 MB 檔案打開後 ≤ 1 MB 加上 header 長度；
  - 單元測試 `lines_are_crlf`。
- [ ] **Step 3:** `cargo test --manifest-path rust/Cargo.toml logs`，預期 FAIL。
- [ ] **Step 4:** 實作。
- [ ] **Step 5:** 再跑同一指令，預期 PASS。
- [ ] **Step 6:** 跑五項檢查後 commit：`feat(rust): port logs`。

### Task 3: i18n 與字型選擇

**Files:**
- Create: `rust/src/i18n/mod.rs`, `rust/src/ui/mod.rs`, `rust/src/ui/fonts.rs`
- Test: `rust/tests/i18n_golden.rs`
- Modify: `tools/golden/export.py`（新增 `i18n_best_match`、`i18n_available`、`i18n_t`、`fonts_family`）

**Interfaces:**
- Produces:
  - `pub fn t(key: &str, args: &[(&str, &str)]) -> String`：查找順序為目前語言 → en-US → zh-TW → key 本身；格式化錯誤也往下一層退。`{name}` 佔位對應 Python `str.format`。
  - `pub fn set_language(code: &str)`、`pub fn current_language() -> String`
  - `pub fn available_languages() -> Vec<(String, String)>`：覆蓋率 ≥ 0.6 且有 `language.name`，依 Python 的順序。
  - `pub fn language_name(code) -> String`、`pub fn translators(code) -> String`
  - `pub fn best_match(tags: &[String]) -> String`、`pub fn detect_system_language() -> String`（`GetUserPreferredUILanguages`）
  - `pub fn font_family(code: &str) -> &'static str`
- 30 個語言檔以 `include_str!` 編進 exe；載入時拒絕重複 key，規則與 Python `_no_duplicates` 相同。

- [ ] **Step 1:** golden exporter：
  - `i18n_best_match`：至少 60 組系統語言清單，涵蓋 30 個 locale 本身、地區變體（`zh-HK`、`zh-SG`、`pt-BR`、`en-GB`、`es-419`…）、不認得的 tag、空清單；輸出 `best_match`。
  - `i18n_available`：`available_languages()` 的輸出。
  - `i18n_t`：在 3 種語言下，對所有 key 與幾組帶參數的 key 取 `t()` 的輸出。
  - `fonts_family`：每個 locale 的 `font_family`。
- [ ] **Step 2:** Rust 測試逐筆對照：`best_match_matches_python`、`available_languages_match_python`、`t_matches_python`、`font_family_matches_python`，另加 `duplicate_keys_are_rejected`。
- [ ] **Step 3:** 跑測試，預期 FAIL。
- [ ] **Step 4:** 實作。語言比對不必移植 langcodes 全套，只要讓 golden 全數通過即可，例如用明確的地區對應表。
- [ ] **Step 5:** 跑測試，預期 PASS。
- [ ] **Step 6:** 五項檢查通過後 commit：`feat(rust): port i18n and font family selection`。

### Task 4: config（載入、遷移、服務、驗證）

**Files:**
- Create: `rust/src/config/mod.rs`, `rust/src/config/services.rs`
- Test: `rust/tests/config_golden.rs`
- Modify: `tools/golden/export.py`（新增 `config_load`、`config_normalize`、`config_resolve`、`config_validate`、`config_summary`、`config_bootstrap_language`）

**Interfaces:**
- Consumes：`i18n::detect_system_language`、`i18n::language_name`、`logs::log`
- Produces：
  - `pub type Config = serde_json::Map<String, serde_json::Value>;`
  - `pub fn app_name() -> String`、`pub fn app_dir() -> PathBuf`、`pub fn local_state_dir() -> PathBuf`、`pub fn config_path() -> PathBuf`
  - `pub fn load_config(path: &Path) -> Config`：處理流程依序為
    1. 預設值深度合併在檔案值之下；
    2. `clamp_advanced`；
    3. 丟掉 `poll_interval`；
    4. `normalize`；
    5. 有變動才重寫。

    壞檔時回傳預設值，且不覆寫原檔。
  - `pub fn save_config(path: &Path, cfg: &Config) -> io::Result<()>`：輸出要與 Python `json.dumps(ensure_ascii=False, indent=2)` 經 `write_text` 寫成的檔案逐 byte 相同（CRLF、無結尾換行）。
  - `pub fn is_configured(cfg) -> bool`
  - `pub fn bootstrap_language(cfg: &mut Config, config_existed: bool) -> String`
  - `pub fn config_summary(cfg) -> String`
  - `services::normalize(cfg: &mut Config) -> bool`
  - `services::resolve(cfg: &Config, slot: ServiceSlot) -> ServiceSpec`
  - `services::validate_service(&Value) -> Vec<String>`
  - `pub enum ServiceSlot { Incoming, Outgoing, Region }`：用 `ServiceSlot` 這個名字，避免和 reader 的客戶端編號 `slot: u32` 混淆。
  - `pub struct ServiceSpec { pub id: String, pub name: String, pub provider: String, pub model: String, pub api_key: String, pub base_url: String, pub thinking: bool, pub effort: String }`：欄位不存在時為空字串或 `false`。
  - `pub struct Settings`：執行期唯讀檢視，`Settings::from_config(&Config)` 取出
    - `fade_seconds: u32`、`max_messages: usize`、`max_parallel_translations: usize`
    - `translate_system_messages: bool`、`target_language: String`
    - `overlay: Geometry { x: Option<i32>, y: Option<i32>, width: i32, height: i32 }`
    - `bubble_position: (Option<i32>, Option<i32>)`、`overlay_alpha: f32`
- 預設值、範圍、遷移規則全部照 spec §D 與 `src/config/__init__.py`、`src/config/services.py`。

- [ ] **Step 1:** golden exporter。
  - `config_load` 至少涵蓋以下輸入，輸入是寫進暫存檔的原始 bytes：
    - 空檔與壞 JSON；
    - 缺欄位；
    - 型別錯誤（bool 混成 int、字串數字）；
    - 超出範圍的進階值；
    - 平面舊版 `api`、分服務商的舊版 `api`；
    - 未知服務商、重複 id／名稱、slots 指向不存在的服務；
    - 含 `poll_interval`；
    - 非 ASCII 名稱。

    每組輸出 `load_config` 的結果，以及寫回檔案的 bytes（hex），沒寫回時為 `null`。
  - 其餘 exporter 各自對應 `normalize`、`resolve`、`validate_service`、`config_summary`、`bootstrap_language`（`detect` 改成注入固定值）。
- [ ] **Step 2:** Rust 測試逐筆對照：
  - `load_matches_python`：比對結果與寫回的 bytes；
  - `corrupt_file_is_never_overwritten`；
  - `save_bytes_match_python`；
  - `normalize_matches_python`、`resolve_matches_python`、`validate_matches_python`、`summary_matches_python`。

  `config_summary` 的輸出不可含 API 金鑰：加一個斷言，確認金鑰字串不出現在輸出中。
- [ ] **Step 3:** 跑測試，預期 FAIL。
- [ ] **Step 4:** 實作。遷移一律操作 `Value`，保留未知欄位與鍵的順序；bool 不可當成 int。
- [ ] **Step 5:** 跑測試，預期 PASS。
- [ ] **Step 6:** 五項檢查通過後 commit：`feat(rust): port config loading, migration and services`。

### Task 5: runtime（單一實例、權限、內嵌資源）

**Files:**
- Create: `rust/src/runtime/mod.rs`, `rust/src/runtime/single_instance.rs`, `rust/src/runtime/resources.rs`

**Interfaces:**
- Produces：
  - `pub fn acquire_single_instance() -> Option<InstanceLock>`：named mutex `wizard101-chat-translator.single-instance`；`InstanceLock` 在 Drop 時釋放。
  - `pub fn focus_running_instance(title: &str) -> bool`：找標題完全相同的可見頂層視窗，`SW_RESTORE` 後強制取得前景。
  - `pub fn is_elevated() -> bool`
  - `resources::ICON_ICO: &[u8]`、`resources::icon_png(size: u32) -> &'static [u8]`（16／48）、`resources::provider_icon(key: &str) -> Option<&'static [u8]>`：全部以 `include_bytes!` 從 `src/assets/` 取。
- 不移植 `_MEI` 監看（spec 的非目標）。

- [ ] **Step 1:** 寫測試：
  - `second_acquire_fails_while_first_held`：用測試專用的 mutex 名稱參數化，內部函式為 `acquire_named(name)`；
  - `released_on_drop`；
  - `provider_icons_exist_for_all_providers`：openai、claude、gemini、custom。
- [ ] **Step 2:** 跑測試，預期 FAIL。
- [ ] **Step 3:** 實作。
- [ ] **Step 4:** 跑測試，預期 PASS。
- [ ] **Step 5:** 五項檢查通過後 commit：`feat(rust): port single instance, elevation check and embedded resources`。

### Task 6: 提示詞、上下文、後處理

**Files:**
- Create: `rust/src/translation/mod.rs`, `rust/src/translation/prompts.rs`, `rust/src/translation/context.rs`, `rust/src/translation/postprocess.rs`
- Test: `rust/tests/prompts_golden.rs`, `rust/tests/postprocess_golden.rs`
- Modify: `tools/golden/export.py`

**Interfaces:**
- Produces：
  - `pub struct Turn { pub role: Role, pub content: String }`、`pub enum Role { User, Assistant }`
  - `prompts`：
    - `OUTGOING_LANGUAGE`、`PROMPT_REVISION = 6`
    - `build_incoming_system(target)`、`build_outgoing_system(lang)`、`build_system_message_system(target, strict: bool)`、`build_region_system(target)`，回傳 `String`
    - `build_turns(context: &[String], text: &str, intro: &str, examples: &[Turn]) -> Vec<Turn>`
    - `example_turns(pair: Option<&(String, String)>) -> Vec<Turn>`
    - `is_game_language(target) -> bool`
    - `FEWSHOT_OUTGOING`、`CONTEXT_INTRO_INCOMING`、`CONTEXT_INTRO_OUTGOING`、`CONTEXT_ACK`
  - `context`：`CONTEXT_LINES = 8`；`ChatContext { push, snapshot }`；`ClientContexts { for_tab(tab: i32), current(), current_tab }`
  - `postprocess`：
    - `strip_think`、`tidy_parentheses`、`sender_of`、`restore_sender`
    - `uses_latin_script`、`has_stray_latin`
    - `nonblank_lines`、`number_lines`、`numbered_entries`、`unnumber_lines`

  簽名對應 `src/translation/postprocess.py`。

- [ ] **Step 1:** golden exporter。
  - `prompts_*`：目標語言取「繁體中文 (台灣)」、「English」、「日本語」、「Español」，加上 strict 開或關；輸出四種 system prompt 與數組 `build_turns`，要求**逐 byte 相同**。
  - `postprocess_*`：每個函式至少 15 組輸入，涵蓋全形與半形括號、巢狀 `[Sender]`、emoji、組合字元、`\u2028`、`\x0b` 等 `splitlines` 會切的字元、帶重音的拉丁字母、`casefold` 會改變的字（ß）。
- [ ] **Step 2:** Rust 測試逐筆對照，另加 `context_keeps_last_8_lines_per_tab`。
- [ ] **Step 3:** 跑測試，預期 FAIL。
- [ ] **Step 4:** 實作。Python 的 `\w`、`\W`、`\d` 預設是 Unicode 語意，Rust `regex` 的 `\w` 也是 Unicode，但 `\d` 要寫成 `\p{Nd}` 才一致；`str.split()`、`splitlines()`、`casefold()` 自己寫，不靠 std 的近似函式。
- [ ] **Step 5:** 跑測試，預期 PASS。
- [ ] **Step 6:** 五項檢查通過後 commit：`feat(rust): port prompts, chat context and postprocessing`。

### Task 7: 錯誤型別與 OpenAI 相容 client

**Files:**
- Create: `rust/src/translation/errors.rs`, `rust/src/translation/clients/mod.rs`, `rust/src/translation/clients/openai.rs`, `rust/src/translation/clients/sse.rs`
- Test: `rust/tests/clients_openai.rs`（wiremock）、`rust/tests/clients_golden.rs`

**Interfaces:**
- Consumes：`config::ServiceSpec`、`translation::Turn`
- Produces：
  - `pub enum TranslateError { Offline { status: Option<u16>, detail: String }, Config { status: Option<u16>, detail: String }, BadOutput(String), Cancelled, NoModelList }`
  - `pub fn error_detail(text: &str) -> String`、`pub fn rejected_parameter(detail: &str) -> Option<String>`
  - 常數：`TIMEOUT = 60s`、`MAX_TOKENS = 512`、`MAX_TOKENS_THINKING = 4096`、`MAX_TOKENS_REGION = 4096`、`OPENAI_BASE_URL = "https://api.openai.com"`
  - `pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;`（放在 `translation/mod.rs`，Task 10、12 共用）
  - `pub trait ChatBackend: Send + Sync`，由 `Client` 實作；Task 10 用它注入假的 backend：
    - `fn chat<'a>(&'a self, req: ChatRequest<'a>, cancel: Option<&'a CancellationToken>) -> BoxFuture<'a, Result<String, TranslateError>>`
    - `fn list_models(&self) -> BoxFuture<'_, Result<Vec<String>, TranslateError>>`
    - `fn model(&self) -> &str`
  - `pub struct ChatRequest<'a> { pub system: &'a str, pub turns: &'a [Turn], pub max_tokens: Option<u32>, pub deterministic: bool }`
  - `pub enum Client { OpenAiCompat(OpenAiCompatClient), Claude(ClaudeClient), Gemini(GeminiClient) }`（後兩個在 Task 8）
    - `pub async fn chat(&self, req: ChatRequest<'_>, cancel: Option<&CancellationToken>) -> Result<String, TranslateError>`
    - `pub async fn list_models(&self) -> Result<Vec<String>, TranslateError>`
    - `pub fn model(&self) -> &str`
  - `pub fn build_client(spec: &ServiceSpec) -> Client`
  - `OpenAiCompatClient::new(base_url, api_key, model, thinking, official: bool)`

  學到的參數放在 `Mutex` 裡，留在 client 實例上。
- 請求內容、官方模式與 custom 的差異、思考開關、截斷判定（`length`）、參數學習（遇到 400 時拿掉參數，或互換 `max_tokens`／`max_completion_tokens` 後重送）、`/v1/models` 的排序與 404／405 處理，全部照 `src/translation/clients.py:198-370` 與 spec §C。

- [ ] **Step 1:** golden exporter：
  - `clients_error_detail`：JSON error 物件、字串 error、頂層 message、HTML 頁、多行文字；
  - `clients_rejected_parameter`；
  - `clients_openai_body`：用 Python 的 `_body` 對各種組合產生請求 JSON（official 與 custom、thinking 開關、deterministic、max_tokens）。
- [ ] **Step 2:** wiremock 測試（對應 `tests/translation/test_clients.py` 的 OpenAI 部分）：
  - `streams_and_joins_deltas`
  - `sends_bearer_only_with_key`
  - `official_uses_max_completion_tokens_and_no_temperature`
  - `length_finish_is_bad_output_with_sample`
  - `400_naming_param_drops_it_and_retries_once`
  - `learned_params_persist_on_instance`
  - `429_and_5xx_are_offline`、`4xx_is_config_with_detail`
  - `cancel_mid_stream_returns_cancelled_and_closes_connection`
  - `models_sorted`、`models_404_is_no_model_list`
  - `logs_prompt_cache_usage`：檢查 log 內容中有 `[translate] prompt cache:`。

  另加 golden 測試：`request_body_matches_python`、`error_detail_matches_python`。
- [ ] **Step 3:** 跑測試，預期 FAIL。
- [ ] **Step 4:** 實作。SSE 解析放在 `sse.rs`，供 Task 8 共用。
- [ ] **Step 5:** 跑測試，預期 PASS。
- [ ] **Step 6:** 五項檢查通過後 commit：`feat(rust): port OpenAI-compatible translation client`。

### Task 8: Claude 與 Gemini client

**Files:**
- Create: `rust/src/translation/clients/claude.rs`, `rust/src/translation/clients/gemini.rs`
- Test: `rust/tests/clients_claude.rs`, `rust/tests/clients_gemini.rs`

**Interfaces:**
- Consumes：Task 7 的 `Client`、`ChatRequest`、`TranslateError`、`sse`
- Produces：
  - `ClaudeClient::new(api_key, model, effort, endpoint: &str)`，`CLAUDE_BASE_URL = "https://api.anthropic.com"`
  - `GeminiClient::new(api_key, model, effort, endpoint: &str)`，`GEMINI_BASE_URL = "https://generativelanguage.googleapis.com"`

  兩者都掛進 `Client` 與 `build_client`。

**行為依據：** 先讀 `.venv/Lib/site-packages/anthropic/` 與 `.venv/Lib/site-packages/google/genai/`，確認 SDK 實際送出的 REST 請求內容。包括 headers、`anthropic-version`、`output_config` 的位置、`thinking_config`、`?alt=sse`、`x-goog-api-key`，以及**預設重試策略**（anthropic `max_retries=2` 的觸發條件、退避公式、`retry-after`；genai 是否預設重試）。Rust 版逐項照做，並在測試中鎖住。

- [ ] **Step 1:** wiremock 測試：
  - Claude：
    - `claude_sends_structured_output_and_parses_translation`
    - `claude_effort_only_when_not_auto`、`claude_no_temperature`
    - `claude_system_has_cache_control_at_end`
    - `claude_retries_429_twice_then_offline`：斷言請求次數與退避遵守 `retry-after`
    - `claude_max_tokens_stop_is_bad_output`
    - `claude_models_paginate`
    - `claude_logs_cache_read_write_uncached`
  - Gemini：
    - `gemini_maps_roles_and_system_instruction`
    - `gemini_empty_key_sent_as_space`
    - `gemini_thinking_level_only_when_not_auto`
    - `gemini_skips_thought_parts`
    - `gemini_models_filtered_and_prefix_stripped`
    - `gemini_retry_behavior_matches_sdk`
  - 兩者都要有 `cancel_mid_stream_returns_cancelled`。
- [ ] **Step 2:** 跑測試，預期 FAIL。
- [ ] **Step 3:** 實作。
- [ ] **Step 4:** 跑測試，預期 PASS。
- [ ] **Step 5:** 五項檢查通過後 commit：`feat(rust): port Claude and Gemini translation clients`。

### Task 9: 範例（ExampleSet、驗證、儲存、協調）

**Files:**
- Create: `rust/src/translation/examples.rs`, `rust/src/translation/pyrepr.rs`
- Test: `rust/tests/examples_golden.rs`

**Interfaces:**
- Produces：
  - `pub struct ExampleSet { pub incoming: Option<(String, String)>, pub system: Option<(String, String)>, pub region: Option<(Vec<String>, Vec<String>)> }`

    `region` 的實際型別以 Python `ExampleSet` 為準，對應 Python 的 None、tuple 結構。方法有 `merge`、`complete`、`empty`、`digest() -> String`、`summary()`。
  - `pyrepr::repr_str(&str) -> String`：逐字重現 Python `repr(str)` 的引號選擇與跳脫規則。`digest()` 用 `pyrepr` 組出與 Python 相同的 `repr()` 後取 sha1。
  - `EXAMPLE_REVISION = 4`、`MAX_ENTRIES = 32`、`GAME_LANGUAGE_EXAMPLES`
  - `examples_fingerprint(spec: &ServiceSpec, target: &str) -> String`
  - `generation_request(target) -> (String, Vec<Turn>)`
  - `parse_generated(output: &str) -> (ExampleSet, Vec<String>)`
  - `ExampleStore { path }`：
    - `get(&self, fp) -> Option<ExampleSet>`
    - `put(&self, fp, &ExampleSet)`：LRU，原子寫入（暫存檔加 rename）。
  - `ExampleCoordinator` 依賴 `Translator`，放在 Task 10。

- [ ] **Step 1:** golden exporter：
  - `pyrepr`：200 組字串，涵蓋單雙引號混用、反斜線、控制字元、`\x7f`、非 BMP、`\u2028`、不可列印的 Unicode（`\u200b`、`\ufeff`）；
  - `examples_digest`：各種 ExampleSet 的 `digest()`；
  - `examples_parse`：各種模型輸出的 `parse_generated`；
  - `examples_store_bytes`：Python `ExampleStore.put` 寫出的檔案 bytes。
- [ ] **Step 2:** Rust 測試：
  - `repr_matches_python`、`digest_matches_python`、`parse_matches_python`、`store_bytes_match_python`
  - `store_evicts_lru_beyond_32`
  - `reads_python_written_store`
- [ ] **Step 3:** 跑測試，預期 FAIL。
- [ ] **Step 4:** 實作。
- [ ] **Step 5:** 跑測試，預期 PASS。
- [ ] **Step 6:** 五項檢查通過後 commit：`feat(rust): port translation examples and Python repr digest`。

### Task 10: Translator

**Files:**
- Create: `rust/src/translation/translator.rs`
- Test: `rust/tests/translator.rs`（假 client）

**Interfaces:**
- Consumes：Task 6 到 9
- Produces：`pub struct Translator`，以 `Arc` 共享，內部是 `RwLock<Binding>`。
  - `Translator::new(spec: ServiceSpec, target_language: String) -> Arc<Self>`
  - `async fn translate_incoming(&self, text: &str, context: &[String]) -> Result<String, TranslateError>`
  - `async fn translate_system_message(&self, text: &str) -> Result<String, TranslateError>`：遇到 `has_stray_latin` 時，以 strict 模式重試一次。
  - `async fn translate_outgoing(&self, text, context, cancel: Option<&CancellationToken>)`
  - `async fn translate_region_text(&self, text, cancel)`：逐行編號送出，行數對得上才逐行處理，空結果為 `BadOutput`。
  - `async fn generate_examples(&self) -> Result<(String, ExampleSet), TranslateError>`：最多 6 次。
  - `fn reconfigure(&self, spec, target) -> bool`、`fn describe(&self) -> String`
  - `fn examples(&self) -> Option<ExampleSet>`、`fn examples_fingerprint(&self) -> String`、`fn set_examples(&self, Option<ExampleSet>, fp: &str) -> bool`
  - `fn target_language(&self) -> String`
  - 測試注入：`Translator::with_client(client: Arc<dyn ChatBackend>, ...)`，`trait ChatBackend` 包住 `Client::chat`。
  - `TEST_SAMPLE = "[Tester] Hello! How are you?"`
  - `ExampleCoordinator::new(store: Arc<ExampleStore>, rt: tokio::runtime::Handle, on_applied: Arc<dyn Fn(&Translator) + Send + Sync>)`、`ensure(&Arc<Translator>)`
    - 行為照 `src/translation/examples.py:250-331`：快取有就直接套用，沒有就在背景生成、存檔、套用。
    - Translator 本身執行緒安全，所以套用不必繞回 UI 執行緒（Python 版的 `post` 在這裡不需要）。

- [ ] **Step 1:** 移植 `tests/translation/test_examples.py` 中與協調器相關的案例，再移植 `tests/translation/test_translator.py` 的每一個案例，改用假 backend 記錄收到的 system 與 turns，並斷言輸出。至少要有：
  - `incoming_uses_context_and_examples`
  - `system_message_strict_retry_on_stray_latin_only_once`
  - `outgoing_uses_fixed_fewshot_and_english`
  - `region_numbered_lines_roundtrip`、`region_mismatched_count_returns_block`、`region_empty_is_bad_output`
  - `reconfigure_untouched_slot_returns_false`
  - `english_target_uses_game_language_examples`
- [ ] **Step 2:** 跑測試，預期 FAIL。
- [ ] **Step 3:** 實作。
- [ ] **Step 4:** 跑測試，預期 PASS。
- [ ] **Step 5:** 五項檢查通過後 commit：`feat(rust): port Translator`。

### Task 11: 系統訊息快取

**Files:**
- Create: `rust/src/translation/cache.rs`
- Test: `rust/tests/cache_golden.rs`

**Interfaces:**
- Produces：
  - `normalize(text) -> (String, Vec<String>)`、`restore(template, &[String]) -> String`、`placeholders_match(template, translated) -> bool`
  - `fingerprint_of(provider, model, target, base_url, examples_digest) -> String`：格式 `provider|base_url|model|target|p6|x<digest 前 12 碼或 ->`，以 Python 為準。
  - `TranslationCache::new(path, fingerprint)`，方法有 `load`、`get(&str) -> Option<String>`、`put(text, translated_template, fingerprint) -> bool`、`rebind(fp)`、`clear() -> usize`、`flush()`；`MAX_ENTRIES = 2000`、`FLUSH_EVERY = 20`；內部用 `Mutex`。
  - `async fn translate_and_cache(tr: &Translator, cache: &TranslationCache, text: &str) -> Result<String, TranslateError>`

- [ ] **Step 1:** golden exporter：`cache_normalize`（數字、千分位、小數、版本號樣式）、`cache_fingerprint`、`cache_file_bytes`（Python flush 寫出的 bytes）。
- [ ] **Step 2:** Rust 測試：
  - `normalize_matches_python`、`fingerprint_matches_python`
  - `flush_bytes_match_python`、`loads_python_written_cache`
  - `fingerprint_change_clears_on_rebind`
  - `does_not_store_on_placeholder_mismatch_and_translates_original`
  - `does_not_store_when_fingerprint_changed_mid_flight`
  - `does_not_store_stray_latin`
  - `flushes_every_20_puts`、`lru_caps_at_2000`
- [ ] **Step 3:** 跑測試，預期 FAIL。
- [ ] **Step 4:** 實作。
- [ ] **Step 5:** 跑測試，預期 PASS。
- [ ] **Step 6:** 五項檢查通過後 commit：`feat(rust): port system message translation cache`。

### Task 12: 併發閘門與翻譯池

**Files:**
- Create: `rust/src/translation/gate.rs`, `rust/src/translation/pool.rs`
- Test: `rust/tests/pool.rs`

**Interfaces:**
- Produces：
  - `ConcurrencyGate::new(limit)`、`async fn acquire(&self, stop: &CancellationToken) -> Option<GatePermit>`（Drop 時釋放）、`set_limit(n)`、`in_use()`
  - 型別別名：
    - `pub type TranslateFn = Arc<dyn Fn(String, Vec<String>) -> BoxFuture<Result<String, TranslateError>> + Send + Sync>`
    - `pub type ResultFn = Arc<dyn Fn(u64, String, bool) + Send + Sync>`
  - `TranslationPool::new(translate: TranslateFn, on_result: ResultFn, gate: Arc<ConcurrencyGate>, failed_notice: Arc<dyn Fn() -> String + Send + Sync>, rt: tokio::runtime::Handle)`
    - `submit(&self, line: String, context: Vec<String>, msg_id: u64)`：同步呼叫，供 reader 執行緒使用。
    - `in_flight() -> usize`
    - `error_state() -> Option<ErrorState>`，其中 `enum ErrorState { Offline, Config }`
    - `error_detail() -> Option<(Option<u16>, String)>`
    - `resize(n)`、`shutdown()`
  - 常數：`BACKOFF_STEPS = [5, 15, 30]` 秒、`CONFIG_ERROR_INTERVAL = 15` 秒。
  - 重試規則照 spec §C：
    - Offline 類無限重試，套用全域退避；
    - Config 類每 15 秒重試；
    - BadOutput 或非預期錯誤時放棄，回傳 `failed_notice()` 並標記 `failed = true`；
    - 任一次成功就重置退避。

- [ ] **Step 1:** 移植 `tests/translation/test_gate.py`、`test_pool.py`、`tests/test_parallel_order.py`。時間相關的測試用 `tokio::time::pause`。必須有：
  - `display_order_independent_of_completion`：斷言 `on_result` 依 msg_id 回填，與完成順序無關；
  - `offline_backoff_5_15_30_then_stays_30`
  - `config_error_retries_every_15s`
  - `success_resets_backoff`
  - `bad_output_gives_failed_notice`
  - `resize_changes_parallelism`
  - `shutdown_stops_waiting_acquires`
- [ ] **Step 2:** 跑測試，預期 FAIL。
- [ ] **Step 3:** 實作。
- [ ] **Step 4:** 跑測試，預期 PASS。
- [ ] **Step 5:** 五項檢查通過後 commit：`feat(rust): port concurrency gate and translation pool`。

### Task 13: 程序記憶體存取與 pattern 掃描

**Files:**
- Create: `rust/src/reader/mod.rs`, `rust/src/reader/memory.rs`, `rust/src/reader/pattern.rs`, `rust/src/reader/process.rs`
- Test: 同檔的單元測試，加上 `rust/tests/pattern_golden.rs`

**Interfaces:**
- Produces：
  - `pub trait ProcessMemory: Send`，方法如下：
    - `read(&self, addr: u64, len: usize) -> Result<Vec<u8>, MemError>`、`write(&self, addr: u64, bytes: &[u8]) -> Result<(), MemError>`
    - `alloc(&self, size: usize) -> Result<u64, MemError>`（`MEM_COMMIT`、RWX）、`free(&self, addr: u64)`
    - `module_base(&self, name: &str) -> Result<u64, MemError>`（名稱不分大小寫）
    - `regions(&self, module: &str) -> Result<Vec<Region>, MemError>`
    - `is_alive(&self) -> bool`（`GetExitCodeProcess == 259`）

    再加上便利函式：`read_u64`、`read_i32`、`read_u32`、`read_wstring(addr, chars)`、`read_std_string`。
  - `pub struct Region { base: u64, size: usize, protect: u32, state: u32 }`
  - `pub struct WinProcess`：`WinProcess::open(pid) -> Result<Self, MemError>`，先嘗試啟用 SeDebugPrivilege，再用 `PROCESS_ALL_ACCESS` 開啟；Drop 時關閉 handle。
  - `MemError` 的 `AccessDenied` 對應 Python `CouldNotOpenProcess`，`Closed` 對應 `ClientClosedError`。
  - `pub struct FakeMemory`（放在 `#[cfg(any(test, feature = "test-support"))]` 下，供 Task 14 到 17 使用）：可預置區段與內容，記錄寫入順序。
    - `Cargo.toml` 加上 `[features] test-support = []`，並在 dev-dependencies 加一筆指向自己的相依：`wizard101-chat-translator = { path = ".", features = ["test-support"] }`，`rust/tests/` 底下的整合測試才看得到它。
  - `pattern::scan(mem: &dyn ProcessMemory, module: &str, pattern: &str) -> Result<u64, ScanError>`
    - `pattern` 是與 Python 相同的跳脫字串（例如 `r"\x48\x89\x5C..."`），編譯成 `regex::bytes::Regex::new(&format!("(?s-u){pattern}"))`；
    - 只掃 `MEM_COMMIT` 且保護屬性恰為 `PAGE_EXECUTE_READ`、`PAGE_EXECUTE_READWRITE`、`PAGE_READWRITE`、`PAGE_READONLY` 的區段；
    - 比對不跨區段；
    - 結果：0 筆時為 `ScanError::NotFound`，多於 1 筆時為 `ScanError::Multiple(n)`。
  - `process`：
    - `game_windows() -> Vec<isize>`：class 名稱為 `Wizard Graphical Client`；
    - `window_exists(hwnd)`、`window_pid(hwnd)`、`pid_alive(pid)`、`foreground_window()`
    - `is_game_process_path(path)`：結尾為 `WizardGraphicalClient.exe`，不分大小寫，以 Python 為準。
    - `client_to_screen`、`client_size`

- [ ] **Step 1:** golden exporter `pattern_scan`：取 wizwalker 與 `chat_hook.PATTERN` 的實際 pattern 字串，加上 3 個合成 pattern，對數組合成 buffer（包含換行 byte 0x0A，用來驗證 DOTALL）用 Python `regex` 搜尋，輸出命中位置。
- [ ] **Step 2:** Rust 測試：
  - `scan_matches_python_regex_semantics`
  - `scan_skips_non_listed_protections`
  - `scan_does_not_span_regions`
  - `scan_reports_multiple`
  - `fake_memory_records_write_order`
- [ ] **Step 3:** 跑測試，預期 FAIL。
- [ ] **Step 4:** 實作。
- [ ] **Step 5:** 跑測試，預期 PASS。
- [ ] **Step 6:** 五項檢查通過後 commit：`feat(rust): add process memory access and pattern scanning`。

### Task 14: Hook（code cave、root window、AddLine）與 hookstate

**Files:**
- Create: `rust/src/reader/hooks.rs`, `rust/src/reader/chat_hook.rs`（只放 stub 的部分；記錄解析在 Task 15），`rust/src/reader/hook_state.rs`
- Test: `rust/tests/hooks_golden.rs`

**Interfaces:**
- Consumes：Task 13
- Produces：
  - `hooks::AutobotCave::install(mem) -> Result<AutobotCave, HookError>`：掃 `AUTOBOT_PATTERN`，從 `wizwalker/memory/handler.py:31` 原樣抄過來。保存原本的 **208 bytes**，寫入 **4100 個 0**，並提供 `alloc_stub(size) -> u64`（bump allocator）。
  - `hooks::RootWindowHook::install(mem, cave) -> Result<Self, HookError>`、`export_addr()`、`uninstall(mem)`：細節照 spec §B。
  - `chat_hook::AddLineHook::install(mem, cave) -> Result<Self, HookError>`、`ring_addr()`、`uninstall(mem)`；`chat_hook::stub_bytes(ring: u64, ...) -> Vec<u8>`。
  - 常數：`SLOTS = 256`、`SLOT_SIZE = 0x800`、`HEADER = 0x40`、`DATA_OFF = 0x30`，`PATTERN` 照抄。
  - `AutobotCave::uninstall(mem)`：依序解每個 hook，等 0.5 秒，只還原那 208 bytes，而且只在內容不同時還原。
  - `hook_state`：
    - `save_state(pid, base, ops: &[(u64, Vec<u8>)])`
    - `load_state(pid) -> (Option<u64>, Vec<(u64, Vec<u8>)>)`
    - `clear_state(pid)`、`sweep(is_alive: impl Fn(u32) -> bool)`
    - `repair(mem, pid, base) -> usize`：base 相同才套用。

    hookstate 格式與 Python `json.dumps` 逐 byte 相同。

- [ ] **Step 1:** golden exporter：
  - `hook_stubs`：對數組 ring／export／跳回位址，取 Python 產生的 `chat_hook.stub_bytes` 與 wizwalker RootWindowHook 的 stub、跳轉 bytes；
  - `hookstate_bytes`：Python `save_state` 寫出的檔案。
- [ ] **Step 2:** Rust 測試：
  - `addline_stub_bytes_match_python`、`root_window_stub_and_jump_match_python`
  - `cave_saves_208_and_zeroes_4100`
  - `uninstall_restores_only_208_when_different`
  - `ring_is_never_freed`
  - `hookstate_bytes_match_python`
  - `repairs_from_python_written_hookstate`（Review Focus 1）：把 Python 寫的 fixture 放進暫存的 state 目錄，對 `FakeMemory` 執行 `repair`。斷言：
    - 依檔案順序寫回原 bytes（hook 跳轉在前、autobot 208 bytes 最後）；
    - base 不符時完全不寫；
    - 完成後刪除檔案。
  - `sweep_removes_dead_pids_only`
- [ ] **Step 3:** 跑測試，預期 FAIL。
- [ ] **Step 4:** 實作。stub 的組譯不另引入 assembler crate，照 Python 以 bytes 拼接。
- [ ] **Step 5:** 跑測試，預期 PASS。
- [ ] **Step 6:** 五項檢查通過後 commit：`feat(rust): port code cave, root window and AddLine hooks with hookstate`。

### Task 15: UI 樹、分頁、聊天標記、環形紀錄解析

**Files:**
- Create: `rust/src/reader/ui_tree.rs`, `rust/src/reader/chat_tabs.rs`, `rust/src/reader/ui_rect.rs`, `rust/src/reader/markup.rs`
- Modify: `rust/src/reader/chat_hook.rs`（新增記錄解析與廣播收合）
- Test: `rust/tests/reader_golden.rs`

**Interfaces:**
- Produces：
  - `ui_tree::Window { addr: u64 }`，方法有 `children(mem)`（+0x70，上限 1000）、`name(mem)`（+0x50）、`parent(mem)`（+0x88）、`flags(mem)`（+0x9C）、`rect(mem)`（+0xA0），以及 `find_by_name(mem, root, name) -> Vec<Window>`（完整 DFS，讀取錯誤直接略過）。
  - `chat_tabs`：
    - `TabState { tabs: Vec<(i32, String)>, current: i32 }`
    - `visible_label(raw: &str) -> Option<String>`
    - `read_tab_state(mem, chatbox) -> Option<TabState>`、`read_current_tab(mem, chatbox) -> Option<i32>`
    - 常數 `CUR_TAB_OFFSET = 0x7A0`、`LABEL_OFFSET = 0x2E0`、`SSO_CAPACITY = 8`、`MAX_LABEL_CHARS = 4096`
  - `ui_rect::client_rect(offsets: &[(i32, i32)], size: (i32, i32), root_size, client_size) -> (i32, i32, i32, i32)`：兩軸都用寬度比例。
  - `markup`：
    - `ChatLine { text: String, color: Option<String>, system: bool, ... }`，欄位對應 Python 的 `ChatLine`；
    - `clean`、`line_color`、`parse_line(raw) -> Option<ChatLine>`、`is_server_broadcast`。
  - `chat_hook`：
    - `HookRecord { seq: u64, ret: u64, this: u64, tab: i32, text: String }`
    - `read_ring(mem, ring, last_seq) -> RingRead`
    - `decode_record(bytes: &[u8]) -> HookRecord`
    - `group_broadcasts(&[HookRecord]) -> Vec<(Vec<i32>, HookRecord)>`
    - `pending_broadcast(&[HookRecord]) -> usize`

- [ ] **Step 1:** golden exporter：
  - `markup_parse`：200 行以上的原始聊天行，可從 `tests/reader/` 的 helper 與測試資料收集，再加上表情、轉義、`\x00`、各種圖示；
  - `hook_records`：各種 slot bytes 的 `decode`，包含切半的 surrogate pair 與奇數長度；
  - `broadcast_groups`：各種記錄序列的 `group_broadcasts` 與 `pending_broadcast`；
  - `tab_labels`：`visible_label` 的輸出；
  - `ui_rect`：`client_rect` 的輸出。
- [ ] **Step 2:** Rust 測試：
  - `parse_line_matches_python`、`group_broadcasts_matches_python`、`visible_label_matches_python`、`client_rect_matches_python`
  - `ring_record_split_surrogate_matches_python`（Review Focus 2）
  - `ui_tree_find_by_name_skips_unreadable_nodes`（用 `FakeMemory`）
  - `std_string_heap_when_length_ge_16`
- [ ] **Step 3:** 跑測試，預期 FAIL。
- [ ] **Step 4:** 實作。
- [ ] **Step 5:** 跑測試，預期 PASS。
- [ ] **Step 6:** 五項檢查通過後 commit：`feat(rust): port UI tree, chat tabs, markup and ring record parsing`。

### Task 16: WizChatReader

**Files:**
- Create: `rust/src/reader/mem_reader.rs`
- Test: `rust/tests/mem_reader.rs`（`FakeMemory`）

**Interfaces:**
- Consumes：Task 13 到 15
- Produces：
  - `pub struct ChatEvent { pub line: ChatLine, pub tabs: Vec<i32>, pub seq: u64, pub raw: String }`
  - `pub enum GameNotRunning { Missing(String), AccessDenied(String), VersionMismatch(String) }`
  - `WizChatReader::new(hwnd, open: Box<dyn Fn(u32) -> Result<Box<dyn ProcessMemory>, MemError> + Send>)`：測試時注入 `FakeMemory`。
  - 方法：
    - `emit_system: bool`
    - `read_new(&mut self) -> Result<Vec<ChatEvent>, GameNotRunning>`：第一次呼叫時連線。
    - `anchored() -> bool`
    - `tab_state()`、`current_tab()`、`input_open() -> bool`
    - `input_box_screen_rect() -> Option<(i32, i32, i32, i32)>`
    - `close(&mut self)`：解除 hook、清掉 hookstate。
  - `INPUT_CONTAINER = "chatEditContainer"`、`HOOK_READY_TIMEOUT = 15s`、`HOOK_READY_POLL = 0.3s`、`FOREIGN_RECHECK_SECONDS = 5`
  - 連線順序與事件過濾照 spec §B 與 `src/reader/mem_reader.py:94-564`。
  - 不讀 `Root.wad`；不呼叫 `SetProcessDpiAwareness`（改由 manifest 處理，見 Task 18）。

- [ ] **Step 1:** 移植 `tests/reader/test_reader_connect.py` 與 `test_reader_events.py` 的所有案例，改用 `FakeMemory`。必須有：
  - `connect_order_matches_python`：斷言寫入與 hookstate 的順序。
  - `pattern_failure_is_version_mismatch_and_leaves_no_partial_hook`：AddLine pattern 找不到時，已掛的 root hook 與 cave 的處理方式與 Python 一致。
  - `open_failure_is_access_denied`
  - `only_wizardchatbox_this_is_accepted`
  - `unknown_this_relooks_chatbox_once_per_round`
  - `partial_broadcast_held_one_round`
  - `system_lines_suppressed_but_seq_advances`
  - `readiness_timeout_is_version_mismatch`
- [ ] **Step 2:** 跑測試，預期 FAIL。
- [ ] **Step 3:** 實作。
- [ ] **Step 4:** 跑測試，預期 PASS。
- [ ] **Step 5:** 五項檢查通過後 commit：`feat(rust): port WizChatReader`。

### Task 17: reader 迴圈、supervisor、狀態彙整、messages.log

**Files:**
- Create: `rust/src/reader/run_loop.rs`（`loop` 是 Rust 保留字）、`rust/src/reader/supervisor.rs`、`rust/src/reader/status.rs`、`rust/src/reader/message_log.rs`
- Create: `rust/src/ui/events.rs`
- Test: `rust/tests/reader_loop.rs`, `rust/tests/supervisor.rs`, `rust/tests/status.rs`

**Interfaces:**
- Consumes：Task 11、12、16，以及 `logs::TimestampedWriter`
- Produces：
  - `ui::events::UiEvent`：
    ```rust
    pub enum UiEvent {
        AddMessage { slot: u32, tabs: Vec<i32>, original: String, translated: String,
                     msg_id: Option<u64>, pending: bool, color: Option<String> },
        UpdateMessage { msg_id: u64, translated: String, failed: bool },
        SetTabs { slot: u32, state: Option<TabState> },
        SetForegroundSlot(Option<u32>),
        SetStatus(&'static str),               // status.* 的 state key
        SetError { key: &'static str, args: Vec<(&'static str, String)> },
        ClearError,
    }
    ```
    `pub struct UiPoster`（Clone）：`post(UiEvent)` 會送進 `std::sync::mpsc`，並呼叫 `egui::Context::request_repaint`。測試時改用 `UiPoster::for_test() -> (UiPoster, Receiver<UiEvent>)`。
  - `MessageIds::next() -> u64`
  - `ReaderShared { settings: Arc<RwLock<Config>>, ui: UiPoster, stop: CancellationToken, pool: Arc<TranslationPool>, system_pool: Arc<TranslationPool>, board: Arc<StatusBoard>, msg_ids: Arc<MessageIds>, cache: Arc<TranslationCache> }`
  - `pub trait ChatSource: Send`，由 `WizChatReader` 實作；測試注入假的 reader（例如會 panic 的）：
    - `read_new`、`anchored`、`current_tab`、`tab_state`、`input_open`、`input_box_screen_rect`
    - `set_emit_system(bool)`、`close`
  - `reader_loop<R: ChatSource>(shared: Arc<ReaderShared>, hwnd: isize, slot: u32, reader: R, contexts: Arc<Mutex<ClientContexts>>, message_log: MessageLog)`
    - 以 Drop guard 保證結束時一定會 `board.drop(slot)` 並 `reader.close()`，panic 時也一樣。
    - 常數：`POLL_INTERVAL = 0.1s`、`INPUT_POLL_INTERVAL = 0.05s`、`TAB_POLL_INTERVAL = 0.4s`、`GAME_MISSING_INTERVAL = 5s`。
    - 輸入框開關事件在第 2 段才接上，本段先保留 `_inputs.poll()` 的取樣與 log。
  - `supervisor::supervise(stop, spawn: impl FnMut(isize, u32) -> JoinHandle<()>, windows: impl Fn() -> Vec<isize>, board)`、`allocate_slot`；`SCAN_INTERVAL = 5s`、`JOIN_TIMEOUT = 8s`。
  - `status::StatusBoard::new(ui: UiPoster, pool, system_pool)`：`report(slot, state, issue)`、`drop(slot)`、`refresh()`；還有 `banner_for`、`aggregate_state`、`aggregate_game_issue`，規則照 `src/reader/status.py`。
  - `MessageLog::new(writer, slot)`：`record(seq, tab, raw, verdict)`、`note(text)`，並略過 `\s*\[[A-Z]{3,5}\]` 開頭的行。

- [ ] **Step 1:** 移植 `tests/reader/test_loop.py`、`test_supervisor.py`、`test_status.py`、`test_message_log.py`。必須有：
  - `system_cache_hit_shown_without_pool`
  - `player_line_context_snapshot_before_push`
  - `foreground_slot_posted_on_each_full_sample`
  - `game_missing_retries_every_5s_and_logs_once_per_issue`
  - `window_gone_stops_reader`
  - `reader_panic_still_drops_slot_and_closes`（Review Focus 3）：假 reader 在 `read_new` 中 panic，斷言 `board` 收到 drop、`close` 被呼叫。
  - `smallest_free_slot_reused`
  - `join_budget_shared_8s`
  - `permanent_issue_wins_and_attached_suppresses_missing`
- [ ] **Step 2:** 跑測試，預期 FAIL。
- [ ] **Step 3:** 實作。
- [ ] **Step 4:** 跑測試，預期 PASS。
- [ ] **Step 5:** 五項檢查通過後 commit：`feat(rust): port reader loop, supervisor, status board and message log`。

### Task 18: UI 骨架與視窗架構驗證（含退場判斷）

這一個 task 先把 spec 列出的 egui 風險逐項驗證。任一項做不到，就**停下來回報使用者**（spec 的退場條件），不繼續做 Task 19 之後的內容。

**Files:**
- Create: `rust/src/ui/app.rs`, `rust/src/ui/win32.rs`, `rust/src/ui/palette.rs`, `rust/src/ui/fonts_egui.rs`
- Create: `rust/build.rs`, `rust/app.manifest`（per-monitor DPI aware，加上 Common Controls v6）
- Modify: `rust/Cargo.toml`（加入 eframe/egui，用 wgpu 後端並指定 DX12；加入 `windows` 所需的 features、`embed-resource` 或 `winresource`）

**Interfaces:**
- Produces：
  - `ui::app::App`：實作 `eframe::App`，持有 `Receiver<UiEvent>` 與 overlay／bubble 狀態，每幀先 drain 所有事件。
  - `ui::app::run(native: NativeOptions, build: impl FnOnce(&egui::Context) -> App) -> Result<(), eframe::Error>`
  - `ui::win32`：`hwnd_of(frame) -> isize`、`make_non_activating(hwnd)`、`enable_taskbar_button(hwnd)`（照「先設 owner 再啟用 APPWINDOW」的順序）、`set_topmost(hwnd)`、`work_area_at(x, y)`、`monitor_rect_at(x, y)`、`all_monitors()`、`foreground_window()`、`force_foreground(hwnd)`（`AttachThreadInput` 加上 `SetForegroundWindow`）
  - `palette`：照抄 `src/ui/palette.py` 的所有色碼，型別為 `egui::Color32`。
  - `fonts_egui::install(ctx, language_code)`：從 `C:\Windows\Fonts` 載入字型。
    - 主字型依 `font_family`：`msjh.ttc`、`msyh.ttc`、`YuGothM.ttc`、`malgun.ttf`、`segoeui.ttf`；
    - 其餘 CJK 字型依序作為 fallback，最後是 `seguisym.ttf`，讓 ⛶⚙─✕ 顯示得出來；
    - 讀不到的檔案略過並記 log。
  - 字級換算：Tk 的 `N pt` 等於 egui 的 `N * 96 / 72` point，例如 9pt 為 12.0、11pt 為 14.67。

- [ ] **Step 1:** 建立最小的 overlay 加泡泡 viewport，每項都在 `app.log` 留下 `[ui]` log，並由實作者在本機確認以下五項：
  1. overlay 用 per-pixel alpha 顯示半透明底與不透明文字；
  2. `WS_EX_NOACTIVATE` 生效時，點 overlay 不會搶走遊戲的前景，同時仍有工作列按鈕；
  3. 點擊 overlay 不會穿透到後方視窗；
  4. overlay 隱藏、只剩泡泡顯示時，`UiEvent` 仍會被處理（看 log）；
  5. 設 `WGPU_ADAPTER` 或用 `force_fallback_adapter` 強制走 WARP 時，畫面能正常顯示。
- [ ] **Step 2:** 寫 `egui_kittest` 測試 `app_drains_events_each_frame`：送 3 個 `UiEvent`，跑一幀，斷言狀態已套用。
- [ ] **Step 3:** 跑測試，預期 FAIL。接著實作，再跑測試，預期 PASS。
- [ ] **Step 4:** 五項驗證若有任何一項失敗：commit 目前的進度，在 commit body 寫明失敗項目與觀察結果，然後**停止並回報**。全部通過才進行下一步。
- [ ] **Step 5:** 五項檢查通過後 commit：`feat(rust): add egui app shell and Win32 window helpers`。

### Task 19: overlay 視窗外框（標題列、拖曳、縮放、記憶位置、關閉）

**Files:**
- Create: `rust/src/ui/geometry.rs`, `rust/src/ui/overlay/mod.rs`, `rust/src/ui/overlay/frame.rs`
- Test: `rust/tests/geometry_golden.rs`，加上 `frame.rs` 內的 kittest 測試
- Modify: `tools/golden/export.py`（新增 `geometry`）

**Interfaces:**
- Produces：
  - `geometry`：以下函式照 `src/ui/geometry.py` 逐一移植，常數 `EDGE = 6`、`CORNER = 14`、`CLICK_THRESHOLD = 5`、`EDGE_MARGIN = 4`。
    - `moved_to`、`centered_position`、`clamped_position`、`point_in_rect`、`is_click`
    - `edge_at(px, py, x, y, w, h) -> Option<Edge>`、`resized_edge(...)`
    - `anchored_position`、`anchored_geometry`、`beside_geometry`、`Placement`
  - 新函式 `geometry::pull_onscreen(x, y, w, h, monitors: &[(i32, i32, i32, i32)]) -> (i32, i32)`：整個視窗都不在任何螢幕內時，拉回最近的螢幕，大小不變（spec 的高 DPI 決定）。
  - `overlay::frame::TitleBar`：高度 20px，包含 16px 圖示、名稱、狀態字、⛶⚙─✕ 四個按鈕。被截掉的是名稱，不是按鈕。
    - ⛶ 與 ⚙ 在本段只記一行 `[ui]` log，第 3、4 段才接上；
    - ─ 縮成泡泡（Task 24）；✕ 走關閉流程。
  - 最小尺寸 200×90；四邊與四角都可縮放；頂端邊緣帶是縮放、不是拖曳。
  - 拖曳或縮放放開時呼叫 `on_geometry_change(x, y, w, h)`。
  - 首次啟動（x、y 為 null）時置中。
  - 底色 `BG` 的透明度取 `Settings.overlay_alpha`；`set_alpha(f32)` 要能即時套用（第 4 段的設定預覽會用到）。
  - `MIN_WIDTH = 200`、`MIN_HEIGHT = 90`、`STATUS_COLORS` 照 `src/ui/overlay/window.py:42`。

- [ ] **Step 1:** golden exporter `geometry`：每個函式 30 組以上的輸入與輸出。
- [ ] **Step 2:** Rust 測試：
  - `geometry_matches_python`
  - `offscreen_geometry_is_pulled_back`（Review Focus 4）：螢幕為 `(0,0,1920,1080)`，視窗位於 `(2500,300,640,420)`，回傳值要在螢幕內，且大小不變；部分可見的視窗不移動。
  - kittest：
    - `title_buttons_never_clipped_by_long_name`
    - `release_after_drag_reports_geometry_once`
    - `resize_respects_min_size`
    - `top_edge_band_resizes_not_drags`
- [ ] **Step 3:** 跑測試，預期 FAIL。
- [ ] **Step 4:** 實作。
- [ ] **Step 5:** 跑測試，預期 PASS。
- [ ] **Step 6:** 五項檢查通過後 commit：`feat(rust): add overlay frame with title bar, drag, resize and geometry`。

### Task 20: overlay 訊息模型與分頁列

**Files:**
- Create: `rust/src/ui/overlay/model.rs`, `rust/src/ui/overlay/tab_bar.rs`
- Test: `model.rs` 內的單元測試、`tab_bar.rs` 內的 kittest 測試

**Interfaces:**
- Produces：
  - `model::OverlayModel`：純邏輯，不依賴 egui。
    - `add_message(slot, tabs, original, translated, msg_id, pending, color, now: Instant)`
    - `update_message(msg_id, translated, failed)`
    - `set_tabs(slot, Option<TabState>)`、`set_foreground_slot(Option<u32>)`
    - `set_limits(max, fade)`、`prune(now)`
    - `current_view() -> Option<&View>`、`current_tabs() -> (&[(i32, String)], Option<i32>)`
    - `minimized`、`unread`：縮小時只有顯示中客戶端的訊息計入；廣播進多個分頁也只算一則。
  - `Message { original, translated, pending, failed, color, added_at }`
    - 顏色：原文為遊戲色 ×0.71，譯文為遊戲色；pending 用 `FG_PENDING`，failed 用 `FG_ERROR`；沒有遊戲色時用 `FG_ORIGINAL`／`FG_TRANSLATED`。
    - `dimmed(color, factor)` 照 `message_list.py:47`。
  - `client_decided` 的行為照 `window.py:560-596`：第一則訊息或第一次 `set_tabs` 就決定要顯示哪個客戶端。
  - `tab_bar`：
    - 只顯示、不可點；
    - 目前分頁用 `TAB_FG_ACTIVE`，下方加 2px `TAB_ACCENT` 底線；
    - 沒有分頁時隱藏。

- [ ] **Step 1:** 移植 `tests/ui/overlay/test_window.py` 與 `test_message_list.py` 中屬於資料面的案例。必須有：
  - `messages_go_to_each_tab_view`
  - `update_fills_in_place_keeps_order`
  - `max_messages_drops_oldest`
  - `fade_prunes_after_seconds_and_zero_disables`
  - `unread_counts_once_per_broadcast_only_for_shown_client`
  - `first_message_decides_client`
  - `set_tabs_for_shown_client_switches_view`
  - `dimmed_matches_python`：數組色碼，與 Python 輸出比對，可直接寫成字面值。
- [ ] **Step 2:** 跑測試，預期 FAIL。
- [ ] **Step 3:** 實作。
- [ ] **Step 4:** 跑測試，預期 PASS。
- [ ] **Step 5:** 五項檢查通過後 commit：`feat(rust): add overlay message model and tab bar`。

### Task 21: 訊息清單繪製（外框文字、黏底、錨定、捲軸、效能）

**Files:**
- Create: `rust/src/ui/overlay/message_list.rs`, `rust/src/ui/overlay/outlined.rs`, `rust/src/ui/overlay/scrollbar.rs`
- Modify: `.github/workflows/ci.yml`（release 效能測試）
- Test: 各檔內的 kittest 與單元測試

**Interfaces:**
- Consumes：Task 20 的 `OverlayModel`
- Produces：
  - `outlined::paint(painter, pos, galley, color)`：先在 8 個方向、各偏移 1px 畫 `OUTLINE` 色，再畫本色；`TEXT_ORIGIN = 2`。
  - `message_list::MessageList`：
    - 每則兩行：原文 9pt、譯文 11pt；每列上下各留 2px；
    - 用虛擬化排版：只排可見範圍附近，其餘用估計高度；
    - `should_stick_to_bottom(fraction, ...)`：門檻 `0.999`，只在使用者捲動時重新判定；
    - 不黏底時用「頂端訊息加位移」錨定，移除或插入訊息時畫面不跳；
    - 拖曳選取中暫停自動捲到底；
    - 沒有訊息時，狀態文字以大字置中。
  - `scrollbar`：
    - 寬 8px、圓角、外框、最小 20px；
    - `THUMB`／`THUMB_HOVER` 照抄；
    - 不需捲動時隱藏；
    - `thumb_span`、`scroll_fraction` 照 `thin_scrollbar.py` 移植。
  - 滾輪在清單、捲軸、底窗上都有效。

- [ ] **Step 1:** 移植 `tests/ui/overlay/test_message_list.py` 中屬於捲動與排版的案例，以及 `test_outlined_text.py`、`test_perf.py`。必須有：
  - `new_message_follows_when_at_bottom`
  - `does_not_follow_after_user_scrolls_up`
  - `anchor_keeps_reading_position_on_prune`
  - `placeholder_shown_when_empty`
  - `thumb_span_matches_python`：字面值對照。
  - `relayout_1000_messages_within_budget`（Review Focus 5）：1000 則訊息，連續 20 幀改變寬度，每幀排版時間中位數低於 8ms（release 測試，`#[ignore]` 但由 CI 以 `cargo test --release -- --ignored relayout` 執行）。
- [ ] **Step 2:** 跑測試，預期 FAIL。
- [ ] **Step 3:** 實作。
- [ ] **Step 4:** 跑測試，預期 PASS。CI 多加一步 `cargo test --release -- --ignored relayout`。
- [ ] **Step 5:** 五項檢查通過後 commit：`feat(rust): render overlay message list with outlined text and scrolling`。

### Task 22: 選取、複製、右鍵選單

**Files:**
- Create: `rust/src/ui/overlay/selection.rs`, `rust/src/ui/clipboard.rs`
- Test: `selection.rs` 內的單元測試與 kittest 測試

**Interfaces:**
- Produces：
  - `selection::Caret { msg: usize, line: usize, index: usize }`、`Selection`，方法有 `begin`、`extend`、`finish`、`clear`、`span`、`text`。
  - `selected_text(messages: &[Vec<String>], start: Caret, end: Caret) -> String`：同一則訊息內的行以 `\n` 連接，訊息之間空一行。
  - hit-test 用 `Galley::cursor_from_pos`；選取以字元為單位，可跨訊息。
  - 拖出可視範圍時自動捲動：`autoscroll_pixels(y, top, bottom) -> i32`，每 40ms 捲動 2 到 24px，照 `window.py:69-77`。
  - 選取開始時，overlay 主動取得鍵盤焦點（`force_foreground`），讓 Ctrl+C 生效；Caps Lock 開著時也要有效。
  - 有選取時，右鍵顯示只有一項「複製」（`t("menu.copy")`）的選單，點 overlay 其他地方就關閉。
  - `clipboard::set_text(&str)`：用 `CF_UNICODETEXT`。

- [ ] **Step 1:** 移植 `tests/ui/overlay/test_selection.py`、`tests/ui/test_popup.py` 中可以對應的案例。必須有：
  - `selected_text_matches_python`：從 Python 測試抄字面值。
  - `autoscroll_pixels_matches_python`
  - `selection_spans_messages`、`reverse_drag_normalizes_span`
  - `ctrl_c_with_caps_lock_copies`：kittest 送出 `Key::C` 加上 `Modifiers::CTRL`，以 Shift 狀態模擬大寫。
  - `context_menu_only_with_selection`
  - `prune_of_selected_message_clears_selection`
- [ ] **Step 2:** 跑測試，預期 FAIL。
- [ ] **Step 3:** 實作。
- [ ] **Step 4:** 跑測試，預期 PASS。
- [ ] **Step 5:** 五項檢查通過後 commit：`feat(rust): add overlay text selection, copy and context menu`。

### Task 23: 橫幅、狀態、提示框、連結文字、語言重繪

**Files:**
- Create: `rust/src/ui/richtext.rs`, `rust/src/ui/overlay/banners.rs`
- Test: `rust/tests/richtext_golden.rs`，加上 kittest 測試
- Modify: `tools/golden/export.py`（新增 `richtext`）

**Interfaces:**
- Produces：
  - `richtext::parse_link_markup(text) -> Vec<(String, Option<String>)>`、`linkify(text) -> String`：只允許 `http://` 與 `https://`；去掉裸網址結尾的 `.,;:!?'`。`LINK_COLOR` 照抄。
  - `richtext::show(ui, text, color)`：以 egui 的 `Hyperlink` 顯示可點的連結。
  - `banners`：
    - 錯誤橫幅：處理 `UiEvent::SetError` 與 `ClearError`；顯示在清單之前，不會被擠掉。
    - 更新橫幅：`offer_update(Release)`、`set_update`、`clear_update`。整列可點；按 ✕ 後，本次執行不再顯示同一版本；右側預留抓取紋的空間。本段只做 UI 與 API，第 4 段才接上更新檢查。
    - `Release { version: String, url: String }`
  - 狀態：`UiEvent::SetStatus(key)` 更新標題列狀態字（顏色取 `STATUS_COLORS`）與空清單的大字。存的是 key，不是文字。
  - 提示框：停留 500ms 後顯示（`ctx.style_mut().interaction.tooltip_delay = 0.5`），每次顯示都重新取文字。⛶ 的提示內容包含 `region_hotkey`。
  - `App::relabel()`：介面語言變更後，標題列、狀態、橫幅、選單文字全部重新翻譯。

- [ ] **Step 1:** golden exporter `richtext`：`parse_link_markup` 與 `linkify` 各 30 組。
- [ ] **Step 2:** Rust 測試：
  - `link_markup_matches_python`
  - `javascript_scheme_is_plain_text`
  - kittest：
    - `error_banner_not_squeezed_by_list`
    - `dismissed_update_version_not_reoffered`
    - `status_key_relabels_on_language_change`
- [ ] **Step 3:** 跑測試，預期 FAIL。
- [ ] **Step 4:** 實作。
- [ ] **Step 5:** 跑測試，預期 PASS。
- [ ] **Step 6:** 五項檢查通過後 commit：`feat(rust): add overlay banners, status, tooltips and link text`。

### Task 24: 泡泡（縮小、展開、未讀、自動展開）

**Files:**
- Create: `rust/src/ui/overlay/bubble.rs`
- Test: `bubble.rs` 內的單元測試

**Interfaces:**
- Produces：
  - `BUBBLE_SIZE = 48`；`bubble_alpha(alpha) -> f32 = max(0.30, alpha * 0.8)`
  - `should_auto_expand(foreground, previous, bubble_hwnd, pointer_over) -> bool`：照 `bubble.py:36-46`，每 300ms 檢查一次。
  - 泡泡的行為：
    - 顯示圓形 app 圖示；
    - 左下角顯示未讀數，上限 `99+`；
    - 移動不到 5px 視為點擊，點擊時展開；拖曳時移動，放開後呼叫 `on_bubble_move(x, y)`；
    - 預設位置在 overlay 右上角；
    - 有自己的工作列按鈕；關閉走同一條路徑。
  - overlay 的 `minimize()`／`expand()`：狀態放在 `OverlayModel` 的 `minimized`／`unread` 欄位。

- [ ] **Step 1:** 寫測試（對應 `tests/ui/overlay/test_window.py` 中與泡泡相關的案例）：
  - `bubble_alpha_floor`
  - `auto_expand_only_on_becoming_foreground_without_pointer`
  - `badge_caps_at_99_plus`
  - `click_vs_drag_threshold_5px`
  - `default_position_top_right_of_overlay`
- [ ] **Step 2:** 跑測試，預期 FAIL。
- [ ] **Step 3:** 實作。
- [ ] **Step 4:** 跑測試，預期 PASS。
- [ ] **Step 5:** 五項檢查通過後 commit：`feat(rust): add overlay bubble with unread badge and auto-expand`。

### Task 25: main 接線、關閉流程、第 1 段驗收交付

**Files:**
- Modify: `rust/src/main.rs`
- Create: `rust/src/app_main.rs`，內容為 `build_app`、`shutdown`、`incoming_fingerprint`、`build_translation`
- Test: `rust/tests/app_main.rs`

**Interfaces:**
- Consumes：前面所有 task 的產出。
- 流程依 `src/main.py:358-423`，但拿掉 splash 與 `_MEI` 監看：
  1. `logs::init`：release 寫 `app.log`，debug 寫 stderr；接著 `install_panic_hook`；
  2. 記 `[app] version=`；
  3. `load_config`；
  4. `set_language(bootstrap_language(...))`，接著記啟動摘要 `[app] startup; elevated=..., active_language=..., <config_summary>`，格式照 `main.py:80-84`，`frozen=` 改成 `build=release|debug`；
  5. 單一實例檢查：取不到鎖就 `focus_running_instance(app_name())`、記 log 後退出；
  6. 開 `messages.log`；
  7. 若 `!is_configured`，記 `[app] config incomplete; first-run wizard arrives in phase 4` 並退出（精靈在第 4 段）；
  8. 建立 tokio runtime（multi-thread）；
  9. `build_translation`（只有 incoming 與 region slot 需要準備範例，與 Python 相同）；
  10. 啟動 supervisor 執行緒；
  11. 記 `[app] running; ...`；
  12. `ui::app::run`。
- `shutdown(...)`：
  1. 設 stop；
  2. 關兩個池；
  3. 等 supervisor 最多 `JOIN_TIMEOUT + 2s`；
  4. `cache.flush()`；
  5. 記 `[app] shutdown complete`；
  6. `std::process::exit(0)`。

  ✕、Alt+F4、從工作列關閉、泡泡關閉都走這裡。
- 每 50ms 呼叫一次 `overlay.prune()`：用 `ctx.request_repaint_after(50ms)` 驅動。

- [ ] **Step 1:** 移植 `tests/test_main.py` 中可以對應的案例。必須有：
  - `incoming_fingerprint_matches_python`：字面值對照。
  - `build_translation_prepares_examples_for_incoming_and_region_only`
  - `shutdown_flushes_cache_after_pools_and_readers`：用假元件記錄呼叫順序。
  - `second_instance_focuses_and_exits`：注入假的 lock 與 focus 函式。
- [ ] **Step 2:** 跑測試，預期 FAIL。
- [ ] **Step 3:** 實作。
- [ ] **Step 4:** 跑測試，預期 PASS。
- [ ] **Step 5:** `cargo build --release --manifest-path rust/Cargo.toml`，記下 exe 大小。用 Python 版目前的 `config.json`，在**不開遊戲**的情況下啟動一次，確認：
  - overlay 出現，狀態為「等待遊戲」；
  - `app.log` 有 version、startup、running 三行；
  - 按 ✕ 能乾淨結束，`app.log` 有 shutdown complete；
  - 從雙擊到 overlay 出現的時間低於 0.5 秒（spec：超過就回報）。
- [ ] **Step 6:** 五項檢查通過後 commit：`feat(rust): wire up phase 1 app and shutdown`。
- [ ] **Step 7:** 交給使用者實機驗收。訊息內容包括：
  - exe 路徑與大小、啟動時間；
  - 要驗收的 spec 章節：§A 的單一實例與關閉；§B 全部，其中輸入框錨點只看 log；§C 收訊與系統訊息；§D 的載入與沿用；§F 全部；§I 的語言與字型；
  - 提醒兩件事：要測多開，以及把 Windows 縮放切到 125% 再測一輪 overlay。

  等使用者驗收通過，才開始第 2 段的計畫。
