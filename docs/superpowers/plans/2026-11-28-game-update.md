# 啟動前自動更新遊戲 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 按〔啟動〕時先檢查要用的遊戲安裝：官網安裝自動下載變動檔，Steam 安裝只檢查；通過才登入開遊戲，進度以「每份安裝一段」顯示在啟動視窗與設定頁。

**Architecture:** 新增 `src/launcher/patch/`（更新伺服器請求、清單解析、CRC 與檢查紀錄、單檔下載、整份安裝的檢查流程），登入與更新共用從 `login.rs` 抽出的 KI 訊息框架（`src/launcher/wire.rs`）。`orchestrate::launch_accounts` 在登入前對每份用到的安裝過一道「同一安裝同時只跑一次」的閘門；安裝狀態以新的 `UiEvent::PatchStatus` 送到啟動視窗與設定頁，與帳號進度的送法相同。

**Tech Stack:** Rust、tokio、reqwest（`net::download_client`）、egui、egui_kittest；CRC 用 `crc32fast`、`.gz` 用 `flate2`（兩者已在 `Cargo.lock` 裡，改成直接相依，版本照 lock：crc32fast 1.5、flate2 1.1）。

**Spec:** `docs/superpowers/specs/2026-11-28-game-update-design.md`

## Global Constraints

- 檢查／建置／測試一律限制並行：`cargo clippy -j 4 --all-targets -- -D warnings`、`cargo test -j 4 -- --test-threads=4`；每個 commit 前跑 `cargo fmt --check`＋clippy＋test。
- Commit message 英文 conventional commits，結尾 `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`；不 push。
- 註解與 docstring 用繁中全形標點，實作註解預設一行、只寫 WHY；模組 `//!`、公開項目 `///` 一行摘要。
- log 一律英文、前綴 `[patch]`（登入編排沿用 `[launch]`），用 `crate::logs::log`；不寫密碼、ck2、Steam 票據。
- 使用者看得到的字一律經 `i18n::t`，新 key 補進 `zh-TW.json`、`en-US.json`、`zh-CN.json`（目前有翻 `launch.*` 的三個語系）；錯誤只給固定說明句＋原文，不加推測。
- 無法檢查（連不上、逾時、解析失敗、清單 CRC 不符）＝不開遊戲，不提供略過檢查的退路。
- 不寫入 Steam 管理的遊戲檔；Steam 安裝只寫 `PatchInfo\`。
- 新整合測試放 `tests/it/`（在 `tests/it/main.rs` 宣告）。
- 測試不連官方伺服器；唯一連線的是 Task 5 的 `#[ignore]` 驗證，**執行前要先問使用者**。
- 測試不可開視窗、不可動執行中的遊戲。

## Review Focus

1. **下載中途失敗（網路斷、CRC 不符、磁碟寫入失敗）**：原檔不動、暫存檔不換上，檢查紀錄不寫成「已最新」。測試：Task 7 `download_failure_keeps_original_and_record`。
2. **同一份安裝被兩批（啟動視窗＋設定頁）同時啟動**：只跑一次檢查／下載，兩批都拿到同一個結果。測試：Task 8 `concurrent_batches_share_one_check`。
3. **檔案大小、修改時間對得上但內容被改過**：照官方規則視為最新（不重算），這是刻意行為；但紀錄 CRC 與清單 CRC 不同時必須重算。測試：Task 4 `stale_record_crc_forces_recompute`。
4. **清單裡出現未知欄位型別或截斷的記錄**：回 `PatchFailure::Protocol`，不 panic、不開遊戲。測試：Task 2 `unknown_field_type_is_an_error`、`truncated_record_is_an_error`。
5. **需要下載時官方啟動器或遊戲還開著**：不下載、不開遊戲；不需要下載時照常開。測試：Task 7 `busy_install_blocks_only_when_download_needed`。

---

### Task 1: 抽出共用的 KI 訊息框架

**Files:**
- Create: `src/launcher/wire.rs`
- Modify: `src/launcher/login.rs:27-140`（框架、讀框、offer、accept 移走）、`src/launcher/mod.rs`（`pub mod wire;`）

**Interfaces:**
- Produces（`wire.rs`）：
  - `pub const FRAME_MAGIC: [u8; 2]`、`pub const MESSAGE_PREFIX: [u8; 4]`
  - `pub enum WireError { Io(String), Protocol(String) }`（`Clone, Debug, PartialEq, Eq`）
  - `pub struct Offer { pub sid: u32, pub seconds: u32, pub millis: u32 }`
  - `pub fn frame(body: &[u8]) -> Vec<u8>`
  - `pub async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> Result<Vec<u8>, WireError>`
  - `pub fn parse_session_offer(body: &[u8]) -> Result<Offer, WireError>`
  - `pub fn build_session_accept(offer: &Offer, now_millis: u64) -> Vec<u8>`
  - `pub async fn handshake<S: AsyncRead + AsyncWrite + Unpin>(s: &mut S) -> Result<Offer, WireError>`：讀 offer、送 accept（用目前時間），回 offer。
  - `pub fn u16_at / u32_at / u64_at(b: &[u8], at: usize) -> Option<…>`
- `login.rs` 以 `impl From<WireError> for LoginError`（`Io`→`Connect`、`Protocol`→`Protocol`）銜接，行為不變。

- [ ] **Step 1:** 把 `login.rs` 既有的框架／offer／accept 單元測試搬到 `wire.rs` 的 tests，改呼叫 `wire::*`；加 `handshake_reads_offer_and_sends_accept`：用 `tokio::io::duplex` 餵一個 `frame(offer_body)`，斷言回傳的 `Offer.sid` 等於餵入值，且對端讀到的框本體長度 26、`[18..20]` 為 sid。
- [ ] **Step 2:** `cargo test -j 4 --lib launcher::wire -- --test-threads=4` → 編譯失敗（模組不存在）。
- [ ] **Step 3:** 建 `wire.rs`，從 `login.rs` 移入上列項目；`login.rs` 改用 `wire`，`sign_in_on` 的握手改呼叫 `wire::handshake`。
- [ ] **Step 4:** `cargo test -j 4 --lib launcher:: -- --test-threads=4` → 全過（登入既有測試不改也要過）。
- [ ] **Step 5:** fmt／clippy／全套 test 後 commit：`refactor(launcher): share KI message framing between login and patch`。

### Task 2: CRC 與清單解析

**Files:**
- Create: `src/launcher/patch/mod.rs`（先只宣告子模組與 Task 2 型別）、`src/launcher/patch/crc.rs`、`src/launcher/patch/list.rs`
- Modify: `src/launcher/mod.rs`（`pub mod patch;`）、`Cargo.toml`（`crc32fast = "1.5"`、`flate2 = "1.1"`）

**Interfaces:**
- Produces：
  - `crc::ki_crc(data: &[u8]) -> u32`：`crc32fast::Hasher::new_with_initial(0xFFFF_FFFF)` 更新後 `finalize() ^ 0xFFFF_FFFF`。
  - `crc::ki_crc_reader(r: impl Read) -> io::Result<u32>`：分塊計算整檔，不整檔讀進記憶體。
  - `list::FileEntry { pub package: String, pub src: String, pub tar: String, pub file_type: u32, pub size: u32, pub crc: u32 }`（`Clone, Debug, PartialEq, Eq`），`fn local_path(&self) -> &str`（`tar` 非空用 `tar`，否則 `src`）。
  - `list::parse_file_list(bytes: &[u8]) -> Result<Vec<FileEntry>, String>`：只回有 `CRC` 欄位的表格的列，`package` 是表格名，順序同檔案。
  - `list::select(entries: &[FileEntry], source: InstallSource, local_packages: &[String]) -> Vec<&FileEntry>`：官網＝`Base`＋`local_packages`；Steam＝全部；兩者都排除 `PatchClient`。
  - `list::read_local_packages(root: &Path) -> Vec<String>`：讀 `LocalPackagesList.txt`，去 `\r`、空行；檔案不存在回空。

- [ ] **Step 1: 寫失敗測試**（`list.rs` tests；以測試內的小 encoder 組位元組，不放大檔進版控）
  - `ki_crc_matches_known_values`：`ki_crc(b"") == 0`、`ki_crc(b"123456789") == 0x2DFD_2D88`（以本機 `CRC_Base.dat` 紀錄驗證過的同一公式算出）。檢查紀錄的檔名 CRC 用的是正斜線路徑，例如 `Bin/xxx.dll`。
  - `parses_package_rows_with_schema`：一個 `Base` 表格（STR `SrcFileName`、STR `TarFileName`、UINT `FileType`、UINT `Size`、UINT `HeaderSize`、UINT `CompressedHeaderSize`、UINT `CRC`、UINT `HeaderCRC`，最後 `_TargetTable`＋名稱）＋兩列、中間夾 `01 00 00 00` 標記，加一個沒有 `CRC` 欄位的 `About` 表格；斷言回兩筆、欄位值正確、`About` 不在結果裡。
  - `unknown_field_type_is_an_error`（型別 7）、`truncated_record_is_an_error`（記錄長度超過剩餘位元組）→ `Err`。
  - `select_official_uses_base_and_local_packages`、`select_steam_takes_all_but_patch_client`。
- [ ] **Step 2:** `cargo test -j 4 --lib launcher::patch -- --test-threads=4` → FAIL。
- [ ] **Step 3:** 實作。格式：開頭 u32 略過；迴圈看兩位元組：`02 01`＝結構（u16 總長含 4 位元組標頭；欄位＝u16 名長＋名＋型別 u8＋旗標 u8，最後一個名字之後沒有型別即為表名）、`02 02`＝資料列（依最近結構解，型別 9＝u16 長＋latin1 字串、3＝u32；解完位置必須剛好等於記錄尾）、其他＝4 位元組標記略過。
- [ ] **Step 4:** 同 Step 2 → PASS。另加 `#[ignore]` 的 `real_list_counts`：讀環境變數 `W101_FILE_LIST` 指的檔案，斷言表格內有 CRC 的列總數與 `FileType` 分布（3：3,695、1：154、4：84、5：40）、`select(Steam)` 的套件數 3,735（以 `package` 去重計數）。實作者用 scratchpad 既有清單手動跑一次，不進 CI。
- [ ] **Step 5:** commit：`feat(patch): parse the patch file list and compute KI CRC`。

### Task 3: 向更新伺服器要清單位置

**Files:**
- Create: `src/launcher/patch/server.rs`

**Interfaces:**
- Consumes: `wire::{frame, read_frame, handshake, MESSAGE_PREFIX, WireError}`
- Produces：
  - `pub struct ListLocation { pub version: u32, pub size: u32, pub crc: u32, pub url: String, pub prefix: String, pub suffix: String }`
  - `pub fn patch_server(root: &Path) -> Result<(String, u16), PatchFailure>`：讀 `PatchClient\BankA\PatchConfig.xml` 的 `PatchServerHostname host` 與 `PatchServerPort port`（regex，不加 XML 套件）。
  - `pub fn build_list_request() -> Vec<u8>`：本文 `MESSAGE_PREFIX`＋`[8, 2]`＋u16(資料長＋4)＋資料；資料依序 u32 0、STR ""、u32×4 皆 0、STR ""×3、STR "English"。
  - `pub fn parse_list_response(body: &[u8]) -> Option<Result<ListLocation, PatchFailure>>`：不是 service 8 的框回 `None`（略過通知），欄位順序同請求。
  - `pub async fn list_location(host: &str, port: u16) -> Result<ListLocation, PatchFailure>`：連線＋`handshake`＋送請求＋最多讀 `MAX_RESPONSE_FRAMES`（沿用登入的 30）則；整體逾時 30 秒。
- `PatchFailure` 在本 task 定義於 `patch/mod.rs`：`SteamOutdated`、`GameOpen`、`Connect(String)`、`Protocol(String)`、`Download(String)`、`NoSpace { need: u64, free: u64 }`、`Io(String)`（`Clone, Debug, PartialEq, Eq`），`From<WireError>`：`Io`→`Connect`、`Protocol`→`Protocol`。

- [ ] **Step 1:** 測試：`list_request_matches_known_bytes`（寫死請求本文 hex：`00000000 0802` ＋長度＋各欄位，"English" 結尾）、`parses_list_response`（自組一個回應本文，斷言各欄位）、`ignores_other_services`、`patch_server_reads_host_and_port`（tempdir 寫一份含兩個標籤的 xml；缺檔回 `Io`、缺標籤回 `Protocol`）、`list_location_over_fake_server`（`tokio::net::TcpListener` 綁 127.0.0.1，先送 offer 框、讀 accept 與請求、回應本文，斷言拿到 `ListLocation`）。
- [ ] **Step 2:** `cargo test -j 4 --lib launcher::patch::server -- --test-threads=4` → FAIL。
- [ ] **Step 3:** 實作上列函式；log `[patch] list location: host={host}:{port} version={} size={} crc={}`。
- [ ] **Step 4:** 同 Step 2 → PASS。
- [ ] **Step 5:** commit：`feat(patch): request the latest file list location`。

### Task 4: 檢查紀錄（`PatchInfo\CRC_*.dat`）

**Files:**
- Create: `src/launcher/patch/cache.rs`

**Interfaces:**
- Produces：
  - `pub struct CacheRecord { pub name_crc: u32, pub size: u32, pub crc: u32, pub mtime_ms: u64 }`
  - `pub fn parse_records(bytes: &[u8]) -> Vec<CacheRecord>`（24 bytes 一筆：u32、u32、u32、u32 保留 0、u64；尾端不足 24 丟掉）、`pub fn encode_records(records: &[CacheRecord]) -> Vec<u8>`
  - `pub fn record_path(root: &Path, package: &str) -> PathBuf`（`PatchInfo\CRC_<package>.dat`）
  - `pub fn file_stamp(path: &Path) -> Option<(u64, u64)>`：（大小, 修改時間 Unix 毫秒）；不存在回 `None`。
  - `pub fn is_fresh(record: Option<&CacheRecord>, entry: &FileEntry, stamp: Option<(u64, u64)>) -> bool`：紀錄存在、`record.crc == entry.crc`、`record.size as u64 == stamp.0 == entry.size as u64`、`record.mtime_ms == stamp.1`。
  - `pub fn record_for(entry: &FileEntry, stamp: (u64, u64)) -> CacheRecord`（`name_crc = ki_crc(entry.src.as_bytes())`）
  - `pub struct PackageRecords`：`load(root, package)`、`get(&self, entry) -> Option<&CacheRecord>`（以 `name_crc` 查）、`set(&mut self, record)`、`save(&self, root, package) -> io::Result<()>`（先寫暫存檔再改名）。

- [ ] **Step 1:** 測試：`records_round_trip`、`fresh_when_size_mtime_and_crc_match`、`stale_record_crc_forces_recompute`（紀錄 CRC 與清單不同 → false）、`missing_file_is_not_fresh`、`package_records_save_and_reload`（tempdir）。
- [ ] **Step 2:** `cargo test -j 4 --lib launcher::patch::cache -- --test-threads=4` → FAIL。
- [ ] **Step 3:** 實作。
- [ ] **Step 4:** → PASS。
- [ ] **Step 5:** commit：`feat(patch): read and write the PatchInfo CRC records`。

### Task 5: 驗證下載規則（需使用者同意才連線）

**Files:**
- Create: `src/launcher/patch/url.rs`
- Test: `tests/it/patch_download_probe.rs`（`#[ignore]`）＋ `tests/it/main.rs` 宣告

**Interfaces:**
- Produces：`pub fn file_url(loc: &ListLocation, entry: &FileEntry) -> String`＝`prefix + "/" + src`（`src` 的 `\` 換 `/`），`file_type == 4` 加 `.gz`，最後接 `suffix`；`pub fn is_gzip(entry: &FileEntry) -> bool`（`file_type == 4`）。

- [ ] **Step 1:** 單元測試 `file_url_appends_gz_for_type_4`、`file_url_uses_forward_slashes`。
- [ ] **Step 2:** → FAIL；實作；→ PASS。
- [ ] **Step 3:** 寫 `#[ignore]` 的 `probe_download_rules`：連官網 port 12500 取清單位置與清單，`FileType` 1、3、4、5 各挑 2 個最小的檔，下載到 `tempfile::tempdir()`，`.gz` 用 `flate2::read::GzDecoder` 解，斷言 `ki_crc` 與清單相符；印出每個檔的 URL、HTTP 狀態、大小。不寫入遊戲目錄。
- [ ] **Step 4: 停下來問使用者**能否執行 `cargo test -j 4 --test it patch_download_probe -- --ignored --test-threads=1 --nocapture`（會連官方伺服器、下載數個小檔）。得到同意才跑。
- [ ] **Step 5:** 依結果定案：若 `FileType` 5 直接 GET 整檔就能過，Task 6 照此實作；若不行，把觀察到的回應記進本 task 的結果並在 Task 6 補分段規則，再回報使用者。把確認後的規則一句話寫回 spec 的「下載網址」段（拿掉「未實測」）。
- [ ] **Step 6:** commit：`feat(patch): build file download URLs and add a manual download probe`。

### Task 6: 單檔下載與替換

**Files:**
- Create: `src/launcher/patch/fetch.rs`

**Interfaces:**
- Consumes: `net::download_client`、`url::{file_url, is_gzip}`、`crc::ki_crc_reader`
- Produces：
  - `pub fn temp_path(dest: &Path) -> PathBuf`（`<dest>.w101tmp`）
  - `pub async fn fetch_to(http: &reqwest::Client, url: &str, temp: &Path, done: &AtomicU64) -> Result<(), PatchFailure>`：串流寫入 `temp`，每塊累加 `done`；HTTP 非 2xx 回 `Download("HTTP {status}")`，連線錯誤回 `Download(error_chain)`。
  - `pub fn install_file(entry: &FileEntry, temp: &Path, dest: &Path) -> Result<(u64, u64), PatchFailure>`（同步，呼叫端放 `spawn_blocking`）：`.gz` 先解到 `<dest>.w101tmp2`；算 CRC，不符刪暫存回 `Download("CRC mismatch …")`；`file_type == 3` 刪 `<dest>.utd`（不存在不算錯）；`std::fs::rename` 換上；回 `file_stamp(dest)`。

- [ ] **Step 1:** 測試（tempdir）：`install_file_replaces_after_crc_check`、`install_file_rejects_bad_crc_and_keeps_original`（原檔內容不變、暫存檔已刪）、`install_file_gunzips_type_4`、`install_file_removes_utd_for_type_3`；`fetch_to_streams_and_counts`（本機 `TcpListener` 起極簡 HTTP 回 200＋內容，比照 `tests/it/stream_server/` 的做法若可直接用就用）、`fetch_to_reports_http_status`（回 404）。
- [ ] **Step 2:** `cargo test -j 4 --lib launcher::patch::fetch -- --test-threads=4` → FAIL。
- [ ] **Step 3:** 實作。
- [ ] **Step 4:** → PASS。
- [ ] **Step 5:** commit：`feat(patch): download a game file and swap it in after the CRC check`。

### Task 7: 整份安裝的檢查與更新流程

**Files:**
- Modify: `src/launcher/patch/mod.rs`、`src/launcher/running.rs`
- Test: `src/launcher/patch/mod.rs` tests（tempdir 假安裝＋假 env）

**Interfaces:**
- Consumes: Task 2–6 全部。
- Produces（`patch/mod.rs`）：
  - `pub enum PatchStatus { Checking { done: u64, total: u64 }, Downloading { done: u64, total: u64 }, UpToDate, Updated, Failed(PatchFailure) }`（`Clone, Debug, PartialEq, Eq`）
  - `pub struct PatchOutcome { pub downloaded: bool, pub seconds: u64 }`
  - `pub trait PatchEnv: Sync { fn list_location(&self, install: &Install) -> impl Future<Output = Result<ListLocation, PatchFailure>> + Send; fn fetch(&self, url: &str, temp: &Path, done: &AtomicU64) -> impl Future<Output = Result<(), PatchFailure>> + Send; fn install_busy(&self, root: &Path) -> bool; fn free_space(&self, dir: &Path) -> Result<u64, PatchFailure>; }`
  - `pub struct SystemPatchEnv { http: reqwest::Client }`（`list_location` 用 `server::patch_server`＋`server::list_location`、`fetch` 用 `fetch::fetch_to`、`install_busy` 用 `running::install_busy`、`free_space` 用 `local::download::free_space`）
  - `pub async fn check_install<E: PatchEnv>(install: &Install, env: &E, report: &(dyn Fn(PatchStatus) + Sync)) -> Result<PatchOutcome, PatchFailure>`
- Produces（`running.rs`）：`pub fn install_busy(root: &Path) -> bool`：任一程序執行檔在 `root` 底下且檔名是 `WizardGraphicalClient.exe` 或 `WizardLauncher.exe`（不分大小寫）；純比對抽成 `fn busy_paths(root: &Path, exes: &[String]) -> bool` 供測試。

流程（照 spec「檢查與更新流程」1–8）：清單 CRC 等於 `PatchInfo\LatestFileList.bin` 的 CRC 就用本機副本，否則 `env.fetch` 清單到暫存、驗 `size`／`crc`；`select`；逐檔 `is_fresh`，不新鮮且檔案存在就 `spawn_blocking` 重算 CRC，相符即 `record_for` 寫回；回報 `Checking` 最多每 100 ms 一次、結束時補一次。Steam：有不符／缺檔 → 存好已更新的紀錄後回 `SteamOutdated`。官網：有要下載的 → `install_busy` 為真回 `GameOpen`；`free_space(root) < 要下載總大小 + 1 GiB` 回 `NoSpace`；逐檔 `fetch`→`install_file`→更新紀錄，`Downloading` 回報同樣節流；全部完成存紀錄與清單副本。`seconds` 為整個 `check_install` 耗時四捨五入。成功時回報 `UpToDate` 或 `Updated`，失敗時回報 `Failed(…)` 並回 `Err`。log：每個階段的數量與位元組、失敗檔名、耗時。

- [ ] **Step 1: 寫失敗測試**（假 env：`list_location` 回固定值，`fetch` 依 URL 從記憶體表寫檔、可設定失敗，`install_busy` 可設定）
  - `fresh_install_reports_up_to_date_without_fetching_files`
  - `official_downloads_changed_file_and_updates_record`（之後 `PackageRecords` 有新紀錄、`outcome.downloaded`）
  - `steam_mismatch_is_outdated_and_writes_nothing_but_records`（遊戲檔內容不變）
  - `steam_recompute_match_writes_record`
  - `busy_install_blocks_only_when_download_needed`（需下載＋busy → `GameOpen`；不需下載＋busy → `UpToDate`）
  - `download_failure_keeps_original_and_record`
  - `list_crc_mismatch_is_protocol_error`
  - `no_space_is_reported`
  - `local_list_copy_is_reused_when_crc_matches`（`fetch` 沒被叫到清單網址）
  - `install_busy_matches_client_and_launcher_under_root`（`busy_paths`）
- [ ] **Step 2:** `cargo test -j 4 --lib launcher::patch -- --test-threads=4` → FAIL。
- [ ] **Step 3:** 實作。
- [ ] **Step 4:** → PASS。
- [ ] **Step 5:** commit：`feat(patch): check and update a whole install`。

### Task 8: 接上一鍵啟動

**Files:**
- Modify: `src/launcher/orchestrate.rs`、`src/launcher/launch.rs:46-72`、`src/app_main.rs:558-700`、`src/ui/events.rs`

**Interfaces:**
- Consumes: `patch::{check_install, PatchStatus, PatchFailure, PatchOutcome, SystemPatchEnv}`
- Produces：
  - `LaunchStatus::Waiting(InstallSource)`（等這份安裝的檢查）；`Failure::Patch`（檢查沒通過而未啟動，原因只在安裝狀態顯示）。
  - `LauncherDeps` 新增 `fn ensure_patched(&self, install: &Install) -> impl Future<Output = Result<PatchOutcome, PatchFailure>> + Send;` 與 `fn report_patch(&self, source: InstallSource, status: Option<&PatchStatus>);`（`None`＝收起）。
  - `build_args(install, kind, steam_running, patch_seconds: u64, creds)`：`-PT` 改填 `patch_seconds`（Steam 安裝呼叫端一律給 0）。
  - `orchestrate::PatchGate`（`static`）：以安裝根目錄為鍵、`futures_util::future::Shared` 的單次檢查＋等待中的帳號計數。`enter(install)` 回 guard；guard drop 時計數歸零且結果為成功就 `report_patch(source, None)`；檢查結束移除鍵（下次按〔啟動〕重新檢查）。
  - `UiEvent::PatchStatus { source: InstallSource, status: Option<PatchStatus> }`；`SystemDeps` 新增 `on_patch: PatchSink`（`Box<dyn Fn(InstallSource, Option<&PatchStatus>) + Send + Sync>`）。

`attempt` 順序改為：跳過執行中 → `select_install` → 取密碼 → `report(Waiting(source))` → `ensure_patched`（失敗 → `Failed(Failure::Patch)`）→ `report(LoggingIn)` → 原本的 Steam／登入／spawn，`build_args` 帶 `outcome.seconds`（Steam 安裝帶 0）。`app_main::status_sink` 把 `Waiting(_)` 也當成仍在進行（不從 `in_flight` 移除）。

- [ ] **Step 1: 寫失敗測試**（`orchestrate.rs` tests，`Fake` 加 `patch: Result<PatchOutcome, PatchFailure>`、檢查次數計數與可阻塞的 barrier）
  - `accounts_wait_for_the_install_check_before_login`（報告順序含 `Waiting(Official)` 在 `LoggingIn` 前）
  - `failed_check_blocks_every_account_on_that_install_only`（官網安裝失敗，Steam 安裝帳號照開）
  - `concurrent_batches_share_one_check`（兩批同時 `launch_accounts`，檢查次數 1）
  - `patch_seconds_reach_the_pt_argument`（`spawned` 參數含 `-PT 7`）
  - `successful_check_is_hidden_after_its_accounts_finish`（最後一次 `report_patch` 是 `None`）；失敗時不送 `None`。
  - `launch.rs`：`build_args` 既有測試補 `patch_seconds` 參數，新增 `-PT` 帶值的斷言。
- [ ] **Step 2:** `cargo test -j 4 --lib launcher:: -- --test-threads=4` → FAIL。
- [ ] **Step 3:** 實作；`app_main::launch` 建 `SystemPatchEnv`、`on_patch` 投遞 `UiEvent::PatchStatus`；`launch.rs` 刪掉「本工具不做更新」那行註解。
- [ ] **Step 4:** `cargo test -j 4 -- --test-threads=4` → 全過（`app_main` 既有 `claim`／`status_sink` 測試補 `Waiting` 情境）。
- [ ] **Step 5:** commit：`feat(launch): check the game install before signing in`。

### Task 9: 安裝狀態的介面

**Files:**
- Create: `src/ui/settings/patch_card.rs`
- Modify: `src/ui/settings/launch_window.rs`、`src/ui/settings/launch_list.rs`、`src/ui/launch_host.rs`、`src/ui/settings/accounts_pane.rs`、`src/ui/settings/env.rs`、`src/ui/app.rs:667-676`、`src/ui/settings/mod.rs`、`src/i18n/locales/{zh-TW,en-US,zh-CN}.json`
- Test: `tests/it/launch_window.rs`、`tests/it/accounts_settings.rs`、`tests/layout_audit.rs`

**Interfaces:**
- Produces（`patch_card.rs`）：
  - `pub struct PatchBoard { statuses: BTreeMap<InstallSource, PatchStatus> (Steam 在前), rates: Rates }`：`apply(&mut self, source, status: Option<PatchStatus>, now: Instant)`（`Downloading` 時 `rates.observe_running` 以 `"steam"`／`"official"` 為鍵）、`is_empty()`、`status(source)`。
  - `pub fn status_text(status: &PatchStatus, speed: Option<Speed>) -> (String, Color32)`：

    | 狀態 | key／文字 | 色 |
    |---|---|---|
    | `Checking` | `patch.checking`「檢查遊戲檔案中 {done} / {total}」（千分位） | `TEXT_SECONDARY` |
    | `Downloading` | `local_rate::transfer_text(done, total, speed)` | `TEXT_SECONDARY` |
    | `UpToDate` | `patch.up_to_date`「已是最新」 | `OK` |
    | `Updated` | `patch.updated`「已更新到最新版本」 | `OK` |
    | `SteamOutdated` | `patch.steam_outdated`「遊戲版本不是最新，請先在 Steam 更新遊戲。」 | `ERROR` |
    | `GameOpen` | `patch.game_open`「需要更新，請先關閉這份遊戲的所有視窗與官方啟動器。」 | `ERROR` |
    | `Connect` | `patch.failed_connect`「無法連線到更新伺服器：{error}」 | `ERROR` |
    | `Protocol` | `patch.failed_protocol`「更新伺服器回應格式不符：{error}」 | `ERROR` |
    | `Download` | `patch.failed_download`「下載遊戲檔案失敗：{error}」 | `ERROR` |
    | `NoSpace` | `patch.no_space`「磁碟空間不足：需要 {need} GB，剩 {free} GB」 | `ERROR` |
    | `Io` | `patch.failed_io`「寫入遊戲檔案失敗：{error}」 | `ERROR` |

  - `pub fn install_name(source) -> String`：`patch.install_official`「官網版」、`patch.install_steam`「Steam 版」。
  - `pub(super) fn status_block(ui, board, source)`：有進度（`Checking`／`Downloading`）時先畫 `progress_bar(ui, fraction, ui.available_width())` 再畫文字。
  - `pub(super) fn board_card(ui, board)`：`card` 內每份安裝一段（粗體名稱＋`status_block`），段間分隔線；`board.is_empty()` 時不畫。
- 帳號列狀態（`launch_window::status_text`／`status_color`）新增：`Waiting(Official)`→`launch.waiting_update`「等待遊戲更新」、`Waiting(Steam)`→`launch.waiting_check`「等待遊戲檢查」、`Failed(Failure::Patch)`→`launch.not_started`「未啟動」；三者皆 `TEXT_SECONDARY`。`Failure::Patch` 在 `LaunchList::settle` 仍算失敗（視窗不自動關）。
- `LaunchList::checkable` 與 `AccountsPane` 的 `busy` 把 `Waiting(_)` 視同 `LoggingIn`。
- 事件：`app.rs` 收 `UiEvent::PatchStatus` 轉給 `LaunchHost::set_patch_status` 與 `SettingsEvent::PatchStatus { source, status }`；兩邊各持一個 `PatchBoard`。
- 版面：啟動視窗在 `header` 的 `Panel::top` 內、「全選」下方畫 `board_card`；設定頁 `location_card` 每個安裝路徑 `hint` 之後呼叫 `status_block`。
- 刪掉 `launch.update_hint`（三個語系檔與 `accounts_pane.rs:199`）。

- [ ] **Step 1: 寫失敗測試**
  - `tests/it/launch_window.rs`：`patch_card_shows_progress_and_waiting_rows`（餵 `Downloading` 與兩個 `Waiting(Official)` 帳號，斷言有 `Role::ProgressIndicator`、文字「等待遊戲更新」出現兩次、兩列〔啟動〕停用）；`steam_outdated_shows_reason_once_and_rows_not_started`；`patch_card_hidden_when_board_empty`。
  - `tests/it/accounts_settings.rs`：`install_card_shows_patch_status_under_its_path`。
  - `patch_card.rs` 單元：`status_text_for_every_variant`（經 key 取字，不寫死譯文）、`board_clears_on_none`。
- [ ] **Step 2:** `cargo test -j 4 --test it -- launch_window accounts_settings --test-threads=4` → FAIL。
- [ ] **Step 3:** 實作；新 key 補三個語系（en-US 與 zh-CN 由實作者翻，句尾標點與繁中一致）。
- [ ] **Step 4:** → PASS；`tests/layout_audit.rs` 的 `launch_states`／`accounts_states` 各加一個更新中與一個 Steam 不是最新的狀態，執行 `LAYOUT_AUDIT_DIR=<scratchpad> cargo test -j 4 --test layout_audit -- --ignored --test-threads=1`，人工看 PNG：整個視窗上下左右與最小尺寸都沒被擠壞（含 `LAUNCH_MIN_SIZE` 下仍看得到兩列帳號）。
- [ ] **Step 5:** 全套 test 後 commit：`feat(ui): show per-install update status in the launch window and settings`。

### Task 10: 文件

**Files:**
- Modify: `README.md`、`README_ZH-TW.md`、`README_ZH-CN.md`（第 94 行附近「不會幫你更新遊戲」那條）、`docs/superpowers/specs/2026-11-27-account-launcher-design.md`（「之後的項目」的遊戲自動更新改成指向新 spec）

- [ ] **Step 1:** README 三份改寫該條：官網安裝在啟動前自動更新、Steam 安裝只檢查並提示到 Steam 更新、無法檢查時不開遊戲。不寫設定鍵名、介面按鈕文字、出處；只寫已驗證的行為（若 Task 5 時 `FileType` 5 未驗證通過，不宣稱官網安裝能完整更新）。
- [ ] **Step 2:** 舊 spec 的「之後的項目」該條改成一句「已另立 spec：`2026-11-28-game-update-design.md`」。
- [ ] **Step 3:** fmt／clippy／全套 test 後 commit：`docs: describe automatic game updates before launch`。

---

## 實機驗證（全部 task 完成後，交給使用者）

- 官網安裝：本機目前已是最新，按〔啟動〕應顯示「檢查遊戲檔案中」後「已是最新」並照常開遊戲；`app.log` 有 `[patch]` 的數量與耗時。
- Steam 安裝：同上；第一次可能要重算較多檔案。
- 真正下載遊戲檔的情境等下次遊戲改版，或使用者同意的其他方式，由使用者決定時機。
