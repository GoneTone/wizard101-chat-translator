# 多帳號一鍵登入 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓使用者管理自己的多個帳號，一鍵直接登入並開進遊戲，略過官方啟動器。

**Architecture:** 新增 `src/launcher/` 模組，內含互相獨立、可單獨測試的部分：帳號儲存（DPAPI）、遊戲位置偵測、登入連線、Steam 票據、用戶端啟動、執行中偵測、一鍵編排。純函式（組參數、解析命令列、解析封包）走 TDD；FFI／網路／實際啟動屬整合行為，照專案慣例以實機（登入進世界、Steam 開著）驗證。UI 沿用既有 egui 設定視窗與 overlay 元件。

**Tech Stack:** Rust、eframe/egui、windows-sys（DPAPI、程序列舉、`steam_api64.dll` FFI）、tokio（TCP）、serde／serde_json（config）、kittest（離屏 UI 測試）。

**Spec:** `docs/superpowers/specs/2026-11-27-account-launcher-design.md`

> 登入連線與 Steam 票據的實際封包／FFI 細節不在本計畫內，實作時依 Subata Lite 2.4.7 反編譯結果與遊戲 `Root.wad/LoginMessages.xml` 的訊息定義推導，寫進對應模組的程式碼與行內註解。本計畫列介面、測試與驗證方式。

## Global Constraints

逐條複製自 spec 與 CLAUDE.md，每個 task 的要求都隱含這一節：

- 只支援 Windows、64 位元用戶端。
- **密碼、ck2、Steam 票據絕不寫進 log、錯誤訊息或任何明文檔案**；讀命令列只擷取帳號名後即丟棄含 ck2 的字串。
- 登入失敗只顯示伺服器回傳原文，不自行推測原因。
- log 一律用 `log()`（`src/logs/mod.rs`），不 `println!`／`eprintln!`；訊息英文、帶足 context；前綴 `[launch]`／`[login]`。
- 註解預設一行、只寫 WHY；模組 `//!`、公開項目 `///`。
- CJK 語境全形標點（註解／docstring／UI／README／設計文件）；程式碼與 log 半形。
- UI 顯示文字繁體中文；新字串要改每個有翻譯的語系，測試一律經 key 取字。
- commit message 英文、conventional commits；PR 以 merge commit 合併。
- 用 cargo：commit 前 `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`（零警告）、`cargo test` 全綠；建置與測試限 `-j 4`／`--test-threads=4`。
- 整合測試放 `tests/it/` 並在 `tests/it/main.rs` 宣告；純函式單元測試用 `#[cfg(test)] mod tests` 內嵌。
- DRY／YAGNI：先確認專案有沒有現成可用的再動手。

## Review Focus

spec 隱含、但各 task 的正常路徑測試未必涵蓋、最可能咬到使用者的輸入，依序：

1. **遊戲沒更新 → 伺服器拒絕登入。** 使用者期望：顯示伺服器原文，並有提示／按鈕導去官方啟動器或 Steam 更新一次，不自行猜原因。→ 測試落在 Task 7（編排錯誤分支，fake login 回伺服器錯誤）。
2. **Steam 沒開或登入的不是綁定的 Steam 使用者**（開 Steam 帳號時）。期望：該帳號明確失敗、顯示原因，其餘帳號不受影響。→ 測試落在 Task 7（fake steam provider 回錯誤，確認其餘帳號照常）。
3. **帳號類型與可用安裝不符**：Steam 帳號但只有官網安裝。期望：提示改用 Steam 版，不硬開。官網帳號則任一安裝都能開。→ 測試落在 Task 3（launch 選目錄／參數）。
4. **重複開啟**：同一帳號已在執行（我們、Subata 或官方啟動器開的）。期望：跳過、不重開。→ 測試落在 Task 4（命令列 `-U` 解析）＋ Task 7（已執行集合排除）。
5. **帳號名稱空白或重複時存檔。** 期望：紅字擋存、不靜默填預設名。→ 測試落在 Task 8（設定頁驗證，沿用既有命名欄位規則）。

---

### Task 1: 帳號儲存與密碼加密

**Files:**
- Create: `src/launcher/mod.rs`（模組根、公開型別）
- Create: `src/launcher/accounts.rs`
- Modify: `src/config/mod.rs`（把帳號清單與「上次勾選」掛進 `Config`）
- Modify: `src/lib.rs`（`pub mod launcher;`）
- Test: `src/launcher/accounts.rs` 內嵌 `#[cfg(test)]`；config 整合進 `tests/it/config_golden.rs`

**Interfaces:**
- Produces:
  - `enum AccountKind { Official, Steam }`
  - `struct Account { id: String, name: String, kind: AccountKind }`（`name` 即登入帳號名，也是 `-U` 末段比對鍵）
  - `fn encrypt_password(plain: &str) -> Result<String, ...>` / `fn decrypt_password(stored: &str) -> Result<String, ...>`（DPAPI，輸出為 base64）
  - `Config` 新增 `accounts: Vec<Account>`、每帳號加密密碼的存放、`launch_selection: Vec<String>`（上次勾選的 account id）

- [ ] **Step 1: 寫失敗測試** — `accounts.rs`：`encrypt_then_decrypt_round_trips`（`decrypt_password(encrypt_password("hunter2")?)? == "hunter2"`）；`encrypted_blob_is_not_plaintext`（輸出不含 `"hunter2"`）。`config_golden.rs`：`config_round_trips_accounts`（含兩筆不同 kind 的 `Account` 的 `Config` 序列化再反序列化相等）。
- [ ] **Step 2: 跑測試確認失敗** — `cargo test -j 4 accounts:: -- --test-threads=4`、`cargo test -j 4 --test it config_round_trips_accounts -- --test-threads=4`；預期 FAIL（型別未定義）。
- [ ] **Step 3: 實作** — 在 `accounts.rs` 定義型別與 DPAPI 封裝（`CryptProtectData`／`CryptUnprotectData`，帶 `CRYPTPROTECT_UI_FORBIDDEN`，blob 以 base64 存）；在 `config/mod.rs` 把 `accounts`、密碼欄位、`launch_selection` 加進 `Config` 與其讀寫，沿用既有 serde 與驗證路徑。密碼欄位命名要能一眼看出是加密後的（如 `password_dpapi`）。
- [ ] **Step 4: 跑測試確認通過** — 同 Step 2 指令；預期 PASS。
- [ ] **Step 5: Commit** — `feat(launcher): add account model and DPAPI password storage`

---

### Task 2: 遊戲位置偵測

**Files:**
- Create: `src/launcher/install.rs`
- Test: `install.rs` 內嵌 `#[cfg(test)]`（VDF 解析、目錄驗證）；註冊表讀取與磁碟列舉屬整合、實機驗證

**Interfaces:**
- Produces:
  - `enum InstallSource { Steam, Official }`
  - `struct Install { source: InstallSource, root: PathBuf }`（`root` 為安裝根，`root/Bin/WizardGraphicalClient.exe` 存在才算有效）
  - `fn detect_installs() -> Vec<Install>`（可能同時回傳 Steam 與官網兩筆）
  - `fn parse_steam_libraries(vdf: &str) -> Vec<PathBuf>`（純函式，解析 `libraryfolders.vdf` 的 `path`）
  - `fn is_valid_install(root: &Path) -> bool`

- [ ] **Step 1: 寫失敗測試** — `parse_steam_libraries_extracts_paths`（給一段含兩個 `"path"` 的 vdf fixture，回兩個路徑）；`is_valid_install_requires_client_exe`（用 tempdir：只有建出 `Bin/WizardGraphicalClient.exe` 才為 true）。
- [ ] **Step 2: 跑測試確認失敗** — `cargo test -j 4 install:: -- --test-threads=4`；預期 FAIL。
- [ ] **Step 3: 實作** — `parse_steam_libraries` 用現有解析慣例（regex 或簡單掃描；專案已依賴 `regex`）；`is_valid_install` 檢查 `Bin/WizardGraphicalClient.exe` 與 `Data/GameData`（沿用 `process.rs::install_root` 的判斷精神）；`detect_installs` 讀 `HKCU\Software\Valve\Steam` 的 `SteamPath` → `libraryfolders.vdf` → 各收藏庫找 `steamapps/common/Wizard101`，再列舉各磁碟機找 `ProgramData\KingsIsle Entertainment\Wizard101`，各自驗證後回傳。埋 `[launch]` log 記偵測到的來源與路徑。
- [ ] **Step 4: 跑測試確認通過** — 同 Step 2；預期 PASS。
- [ ] **Step 5: Commit** — `feat(launcher): detect Steam and official game installs`

---

### Task 3: 啟動參數組法（核心純函式）

**Files:**
- Create: `src/launcher/launch.rs`
- Test: `launch.rs` 內嵌 `#[cfg(test)]`；實際 spawn 屬整合、實機驗證

**Interfaces:**
- Consumes: `Install`、`InstallSource`（Task 2）
- Produces:
  - `struct Credentials { user_id: String, ck2: String, account: String }`
  - `fn build_args(install: &Install, steam_running: bool, creds: &Credentials) -> Vec<String>`（純函式）
  - `fn select_install<'a>(installs: &'a [Install], kind: AccountKind) -> Result<&'a Install, LaunchError>`（Steam 帳號必須 Steam 安裝、否則錯誤提示改用 Steam 版；官網帳號優先官網安裝、否則 Steam 安裝）
  - `fn spawn_client(install: &Install, args: &[String]) -> Result<u32, LaunchError>`（工作目錄設 `Bin\`，回 PID）
  - `enum LaunchError { NoSteamInstall, ClientMissing, Spawn(String) }`

- [ ] **Step 1: 寫失敗測試** — 針對 `build_args`：
  - `steam_install_running_adds_st_and_p0`：Steam install＋`steam_running=true` → 參數含 `-ST`、`-P 0`、`-PT 0`。
  - `steam_install_not_running_drops_st`：Steam install＋`false` → 含 `-P 0 -PT 0`、**不含** `-ST`。
  - `official_install_minimal`：官網 install → 含 `-PT 0`、**不含** `-ST`、**不含** `-P`。
  - 三者都驗證骨架 `-L login.us.wizard101.com 12000`、`-A en-US`、末段 `-U ..{user_id} {ck2} {account}` 順序正確。
  - 針對 `select_install`：`steam_account_without_steam_install_errors`（只有官網 install → `Err(NoSteamInstall)`）；`official_account_prefers_official`（兩種都在 → 回官網）。
- [ ] **Step 2: 跑測試確認失敗** — `cargo test -j 4 launch:: -- --test-threads=4`；預期 FAIL。
- [ ] **Step 3: 實作** — `build_args` 依 spec「本工具採用的啟動參數規則」組 `Vec<String>`；`select_install` 依帳號類型選安裝；`spawn_client` 用 `std::process::Command`，`current_dir` 設 `root/Bin`，回 PID；埋 `[launch]` log（PID、來源；不記 ck2）。
- [ ] **Step 4: 跑測試確認通過** — 同 Step 2；預期 PASS。
- [ ] **Step 5: Commit** — `feat(launcher): build client launch arguments per install source`

---

### Task 4: 執行中帳號偵測

**Files:**
- Create: `src/launcher/running.rs`
- Modify: `src/game/process.rs`（新增讀某 PID 命令列的能力）
- Test: `running.rs` 內嵌 `#[cfg(test)]`（命令列字串 → 帳號名的純解析）；實際程序列舉屬整合、實機驗證

**Interfaces:**
- Produces:
  - `fn account_in_command_line(cmdline: &str) -> Option<String>`（純函式：取 `-U` 後第三段的帳號名；取不到回 `None`，**絕不回傳 ck2**）
  - `fn running_accounts() -> Vec<String>`（列舉 `WizardGraphicalClient.exe`，回各程序的帳號名）
- Consumes: `process.rs` 既有的程序列舉；新增 `fn process_command_line(pid: u32) -> Option<String>`

- [ ] **Step 1: 寫失敗測試** — `account_in_command_line_extracts_name`（給一條完整參數字串 → 回末段帳號名）；`account_in_command_line_ignores_ck2`（回傳值不等於、也不包含那段 88 字元 ck2）；`account_in_command_line_handles_missing_u`（沒有 `-U` → `None`）。
- [ ] **Step 2: 跑測試確認失敗** — `cargo test -j 4 running:: -- --test-threads=4`；預期 FAIL。
- [ ] **Step 3: 實作** — `account_in_command_line` 以 `CommandLineToArgvW` 或等價切分、取 `-U` 後第三個 token；`process_command_line(pid)` 走 PEB＋`ReadProcessMemory`（沿用 `mem_reader` 路數）或 WMI，實作者定；`running_accounts` 串起來。埋 `[launch]` log 記偵測到幾個執行中帳號（只記數量與名稱，不記命令列全文）。
- [ ] **Step 4: 跑測試確認通過** — 同 Step 2；預期 PASS。
- [ ] **Step 5: Commit** — `feat(launcher): detect already-running accounts from command lines`

---

### Task 5: 登入連線

**Files:**
- Create: `src/launcher/login.rs`
- Test: `login.rs` 內嵌 `#[cfg(test)]`（封包組／解析純函式，fixtures 由分析產物得出）；實際連線登入屬整合、以實機（登入伺服器可達）驗證

**Interfaces:**
- Consumes: `Credentials`（Task 3，login 產出其 `user_id`／`ck2`）
- Produces:
  - `struct SteamTicket { steam_id: String, ticket: Vec<u8> }`（共用型別，放 `src/launcher/mod.rs`；Task 6 產出、此處作為 `sign_in` 輸入）
  - `async fn sign_in(account: &str, password: &str, steam: Option<SteamTicket>) -> Result<LoginOutcome, LoginError>`
  - `struct LoginOutcome { user_id: String, ck2: String }`
  - `enum LoginError { Connect(String), Rejected { raw: String }, Protocol(String) }`（`Rejected.raw` 是伺服器原文，直接給 UI）
- 內部純函式（框架組／解析、認證訊息組法、回應解析）各自可測；wire 細節寫在本檔程式碼與註解，依 Subata Lite 參照推導。

- [ ] **Step 1: 寫失敗測試** — 對內部純函式寫 fixture 測試：`parse_session_offer_reads_fields`（給一段握手主體 fixture → 取出 sid／時間欄位）、`build_session_accept_is_26_bytes`、`auth_response_surfaces_server_reason`（給一段帶錯誤碼的回應 fixture → `LoginError::Rejected { raw }` 且 raw 等於 fixture 內的原文）。fixtures 由分析產物得出，放測試模組常數，不在本計畫內嵌位元組。
- [ ] **Step 2: 跑測試確認失敗** — `cargo test -j 4 login:: -- --test-threads=4`；預期 FAIL。
- [ ] **Step 3: 實作** — 依 Subata Lite 參照與 `LoginMessages.xml` 實作框架、握手、認證訊息、回應解析與 `sign_in`（tokio `TcpStream`）；連線逾時與各失敗分支回對應 `LoginError`。埋 `[login]` log 記 provider、伺服器回應碼／錯誤分支（**不記帳密、ck2**）。
- [ ] **Step 4: 跑測試確認通過** — 同 Step 2；預期 PASS。
- [ ] **Step 5: 實機驗證** — 以一個真帳號實跑 `sign_in`，確認回 `user_id`／`ck2`；登入伺服器拒絕時確認 `Rejected.raw` 是原文。記錄於 commit message。
- [ ] **Step 6: Commit** — `feat(launcher): implement login protocol sign-in`

---

### Task 6: Steam 票據

**Files:**
- Create: `src/launcher/steam.rs`
- Test: FFI 行為以實機（Steam 開著、已登入）驗證；無純單元測試

**Interfaces:**
- Consumes: `Install`（需 Steam 安裝的 `root/Bin/steam_api64.dll`）、`SteamTicket`（Task 5，定義於 `mod.rs`）
- Produces:
  - `fn acquire_ticket(install: &Install) -> Result<SteamTicket, SteamError>`（回 Task 5 定義的 `SteamTicket`，供 `sign_in` 使用）
  - `enum SteamError { NotRunning, InterfaceUnavailable, NoSession, Timeout }`

- [ ] **Step 1: 實作** — 在本程序 `LoadLibrary` `root/Bin/steam_api64.dll`，設環境變數 `SteamAppId`／`SteamGameId=799960`，`SteamAPI_InitFlat`，取 `GetAuthTicketForWebApi("Wizard101")` 與 SteamID；各前提不符回對應 `SteamError`。依 Subata Lite `SteamSession` 參照。埋 `[launch]` log（**不記票據內容**）。
- [ ] **Step 2: 確認編譯與 lint** — `cargo build -j 4`、`cargo clippy --all-targets -- -D warnings`。
- [ ] **Step 3: 實機驗證** — Steam 開著、登入綁定帳號時 `acquire_ticket` 回票據；Steam 關閉時回 `NotRunning`。記錄於 commit message。
- [ ] **Step 4: Commit** — `feat(launcher): acquire Steam web-api ticket in-process`

---

### Task 7: 一鍵編排

**Files:**
- Create: `src/launcher/orchestrate.rs`
- Test: `orchestrate.rs` 內嵌 `#[cfg(test)]`，以 fake login／steam／launch 函式注入驗證流程；實際全流程實機驗證

**Interfaces:**
- Consumes: Task 3（`select_install`／`build_args`／`spawn_client`）、Task 4（`running_accounts`）、Task 5（`sign_in`）、Task 6（`acquire_ticket`）
- Produces:
  - `enum LaunchStatus { Opened, LoggingIn, Failed { reason: String }, SkippedRunning }`
  - `async fn launch_accounts(selected: &[Account], deps: &LauncherDeps) -> Vec<(String, LaunchStatus)>`（官網並行、Steam 逐一；回每帳號 id 與結果）
  - `struct LauncherDeps { ... }`（把 login／steam／launch／running／installs 以函式指標或 trait 注入，讓測試替換）

- [ ] **Step 1: 寫失敗測試** — 用 fake deps：
  - `official_accounts_run_in_parallel_and_report_opened`。
  - `running_account_is_skipped`（`running_accounts` 回含某帳號 → 該帳號 `SkippedRunning`、未呼叫 spawn）。【Review Focus 4】
  - `login_rejection_surfaces_server_text_and_others_continue`（fake login 對一個帳號回 `Rejected{raw}` → 該帳號 `Failed{reason==raw}`，其餘照常 `Opened`）。【Review Focus 1】
  - `steam_error_fails_only_that_account`（fake steam 回 `NotRunning` → 僅該 Steam 帳號 `Failed`，官網帳號不受影響）。【Review Focus 2】
- [ ] **Step 2: 跑測試確認失敗** — `cargo test -j 4 orchestrate:: -- --test-threads=4`；預期 FAIL。
- [ ] **Step 3: 實作** — `launch_accounts`：先 `running_accounts` 取已執行集合並排除；官網帳號 `tokio` 並行（各自 `sign_in`→`build_args`→`spawn_client`），Steam 帳號逐一（`acquire_ticket`→`sign_in`→…）；逐帳號回 `LaunchStatus`，失敗帶伺服器原文。埋 `[launch]` log 記每帳號結果。
- [ ] **Step 4: 跑測試確認通過** — 同 Step 2；預期 PASS。
- [ ] **Step 5: Commit** — `feat(launcher): orchestrate one-click multi-account launch`

---

### Task 8: 設定「帳號」頁與新增／編輯對話框

**Files:**
- Create: `src/ui/settings/accounts_pane.rs`、`src/ui/settings/account_dialog.rs`
- Modify: `src/ui/settings/window/mod.rs`（`Page` 新增 `Accounts`、title_key、icon、ALL）、`src/ui/settings/mod.rs`、`src/ui/icon.rs`（新圖示）
- Modify: 各語系檔 `src/i18n/locales/*.json`（新字串，所有有翻譯的語系都補）
- Test: `tests/it/accounts_settings.rs`（新檔，於 `tests/it/main.rs` 宣告）用 kittest；驗證字串經 key 取

**Interfaces:**
- Consumes: Task 1（`Account`／加密）、Task 2（`detect_installs`）、Task 4（`running_accounts`）
- Produces: 設定頁的新增／編輯／刪除；遊戲位置卡（顯示偵測結果、可手動變更）

- [ ] **Step 1: 寫失敗測試** — kittest：`accounts_page_lists_saved_accounts`；`blank_name_blocks_save`（名稱清空 → 顯示紅字、存檔鈕停用）【Review Focus 5】；`duplicate_name_blocks_save`；`running_account_shows_badge_and_disables_open`（`running_accounts` 含該帳號 → 顯示「執行中」、單獨開啟停用）。另在 `tests/it/no_hardcoded_ui_text.rs` 的既有掃描涵蓋新字串（經 key）。
- [ ] **Step 2: 跑測試確認失敗** — `cargo test -j 4 --test it accounts_settings -- --test-threads=4`；預期 FAIL。
- [ ] **Step 3: 實作** — `accounts_pane.rs` 用既有 `card`／`list_card`／命名欄位紅字擋存慣例畫清單與遊戲位置卡；`account_dialog.rs` 用既有對話框元件（類型分段切換、名稱、密碼可顯示、依類型顯示說明），沿用 `Tk 對話框要主動啟用` 的既有啟用規則；`Page::Accounts` 接進側欄。字串加進**所有**有翻譯的語系，半形／全形依語言慣例、句尾標點跨語言一致。
- [ ] **Step 4: 跑測試確認通過** — 同 Step 2；預期 PASS。
- [ ] **Step 5: Commit** — `feat(ui): add accounts settings page and editor dialog`

---

### Task 9: overlay 開啟按鈕與帳號選單

**Files:**
- Create: `src/ui/overlay/launch_menu.rs`（選單純狀態：可勾集合、全選三態、開啟數）
- Modify: `src/ui/overlay/frame.rs`（標題列 ▶ 鈕、等待中的中央大按鈕）、`src/ui/overlay/mod.rs`
- Modify: 各語系檔 `src/i18n/locales/*.json`
- Test: `launch_menu.rs` 內嵌 `#[cfg(test)]`（選單狀態純邏輯）；`tests/it/overlay_launch_menu.rs`（新檔）kittest 驗畫面；定位／視覺以實機驗證

**Interfaces:**
- Consumes: Task 1（`Account`、`launch_selection`）、Task 4（`running_accounts`）、Task 7（`launch_accounts`）
- Produces:
  - `struct MenuState { ... }` 與 `fn checkable(&self, running: &[String]) -> ...`、`fn open_count(&self) -> usize`（排除執行中）、`fn select_all_tristate(...)`
  - 標題列 ▶ 與中央大按鈕都開同一個選單；勾選寫回 `launch_selection`

- [ ] **Step 1: 寫失敗測試** — `launch_menu.rs`：`open_count_excludes_running`（勾了 3 個、其中 1 個在執行 → `open_count()==2`）；`select_all_only_checks_non_running`；`all_running_disables_open`（`open_count()==0`）。kittest：`waiting_shows_center_button`、`menu_lists_accounts_with_running_badge`。
- [ ] **Step 2: 跑測試確認失敗** — `cargo test -j 4 launch_menu:: -- --test-threads=4`、`cargo test -j 4 --test it overlay_launch_menu -- --test-threads=4`；預期 FAIL。
- [ ] **Step 3: 實作** — `launch_menu.rs` 純狀態；`frame.rs` 加標題列 ▶ 與等待中中央大按鈕，兩者開同一選單；選單定位：中央按鈕優先貼按鈕下方、放不下上移蓋按鈕、最多佔滿標題列以下；標題列 ▶ 貼標題列下方；超高走捲動（頭尾固定、沿用 overlay 細捲軸）、名稱過長「…」截斷；內容區過矮走精簡版（「開啟上次勾選的 N 個帳號」＋「把視窗拉高即可選擇帳號」），門檻用實際排版高度算。按「開啟」呼叫 `launch_accounts` 並顯示逐帳號進度。字串補所有語系。
- [ ] **Step 4: 跑測試確認通過** — 同 Step 2；預期 PASS。
- [ ] **Step 5: 實機驗證** — 開著遊戲／等待遊戲兩種狀態各看一次選單定位、捲動、精簡版、進度；截圖確認（顯示類 bug 以截圖驗，沿用既有規則）。
- [ ] **Step 6: Commit** — `feat(overlay): add launch button and account selection menu`

---

### Task 10: 接進 App、README 風險說明、全流程驗證

**Files:**
- Modify: `src/app_main.rs`（把 launcher 接進 App 事件與 overlay／設定的互動）
- Modify: 各語系 README（新增功能說明與**服務條款風險**，鍵名／選項文字不寫死，依既有 README 慣例）
- Test: 全流程實機驗證；確保全套 `cargo test` 綠

**Interfaces:**
- Consumes: Task 7（`launch_accounts`）、Task 8／9（UI 入口）

- [ ] **Step 1: 接線** — App 收到「開啟」事件時呼叫 `launch_accounts`、把進度回饋給 overlay；設定頁的偵測／儲存接上 `Config`。
- [ ] **Step 2: README** — 各語系 README 加本功能與風險段落（自行實作登入協定、違反服務條款的程度、密碼本機加密存放、Steam 帳號限制），泛稱不寫死鍵名／快捷鍵。
- [ ] **Step 3: Lint 與測試** — `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test -j 4 -- --test-threads=4` 全綠。
- [ ] **Step 4: 實機全流程** — 登入進世界、Steam 開著：一鍵開 2～3 個自己的帳號（含官網與 Steam），確認各自登入進遊戲、overlay 各視窗都翻得到、重複開啟被擋、Steam 關閉時該帳號失敗訊息正確。
- [ ] **Step 5: Commit** — `feat(launcher): wire account launcher into app and document risk`
