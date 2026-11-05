# 程式內更新 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 點更新橫幅或「關於」頁的新版連結時，開程式內的「更新內容」視窗逐版列出 release notes，按「立即更新」就在程式內下載、驗證、替換 exe 並自動重啟。

**Architecture:** `runtime::updater` 改抓 `/releases` 列表回傳所有較新的 release；新增 `runtime::self_update`（檔案替換、殘留清除、重啟交接）與 `runtime::update_job`（下載狀態機，不碰 UI）。UI 端新增 `ui::settings::update_window`（純畫面＋動作）與 `ui::update_host`（獨立 viewport，沿用 `settings_host.rs` 的 `HostedWindow`），由 `App` 接線。重啟時 `App` 替換 exe、登記要重啟的路徑、關閉根 viewport，`shutdown()` 跑完關閉流程後啟動新版。

**Tech Stack:** Rust 2024、eframe/egui 0.37、`egui_commonmark` 0.26（新增）、reqwest、tokio、wiremock／egui_kittest（測試）。

**Spec:** `docs/superpowers/specs/2026-11-04-in-app-update-design.md`

## Global Constraints

- 新增相依只有 `egui_commonmark = { version = "0.26", default-features = false, features = ["pulldown_cmark"] }`（不要 `load-images`：release 內文沒有圖片）。
- log 一律英文、前綴 `[update]`，經 `crate::logs::log`；不可 `println!`／`eprintln!`。
- 介面文字走 i18n key；新 key 加進 `src/i18n/locales/en-US.json`、`zh-TW.json`、`zh-CN.json`（目前只有這三個語系翻了 `update.*`）。CJK 文案用全形標點。
- `update.available` 改為：zh-TW `⬆  有新版本 v{version}`、zh-CN `⬆  有新版本 v{version}`、en-US `⬆  Version {version} is available`。
- 註解預設不寫；寫就一行講 WHY。公開項目一行 `///`、模組頂端 `//!`。
- 整合測試寫成 `tests/it/` 底下的模組並在 `tests/it/main.rs` 宣告，不另開 `tests/*.rs`。
- cargo 一律限並行：`cargo build -j 4`、`cargo test -j 4 -- --test-threads=4`；單一模組用 `cargo test -j 4 --test it <模組名>:: -- --test-threads=4`。
- 每個 task commit 前跑 `cargo fmt --check`、`cargo clippy -j 4 --all-targets -- -D warnings`；完整 `cargo test` 每個 task 只在最後跑一次。commit message 英文、conventional commits、結尾加 attribution 行。
- 錯誤訊息只放系統或伺服器原文，不加推測原因。
- 下載檔放 exe 旁：`<exe>.new`、半成品 `<exe>.new.part`、舊檔 `<exe>.old`。

## Review Focus

- **最新版沒有 exe asset 或沒有 `digest`**：視窗照常顯示，按「立即更新」直接進 ⑥，錯誤寫明缺什麼，「前往 GitHub 下載」可用。→ Task 3 測 `start_without_verifiable_asset_fails_immediately`。
- **下載中途又查到更新的版本**：不打斷進行中的下載；下載完的 `.new` 是舊版就丟掉並回到 ①。→ Task 3 測 `newer_release_discards_ready_download`、`newer_release_during_download_discards_on_finish`。
- **`.old` 被占用（上一個舊程序還沒完全退出、防毒鎖住）**：`replace_exe` 先刪舊 `.old` 失敗時直接回錯，不動目前的 exe。→ Task 2 測 `replace_fails_without_touching_exe_when_old_cannot_be_removed`。
- **`--after-update` 等鎖逾時**：走原本「已有實例」的處理（聚焦既有視窗後退出），不會兩個都跑。→ Task 2 測 `acquire_waiting_gives_up_after_the_deadline`。
- **使用者在 ⑤ 之後又點橫幅或關於頁**：視窗已在重啟流程中，只帶到前景，不重設狀態。→ Task 3 測 `open_does_not_reset_phase`（`UpdateJob` 不因開窗重設）。

---

### Task 1: 抓所有較新的 release

**Files:**
- Modify: `src/runtime/updater.rs`
- Modify: `src/main.rs`（`schedule_update_checks` 的常數與回呼）
- Modify: `src/ui/events.rs`（`UiEvent::UpdateFound`）
- Modify: `src/ui/app.rs:464`、`src/ui/app.rs:620`（取 `releases[0]` 給橫幅，行為暫不變）
- Modify: `src/ui/settings/env.rs`（`SettingsEvent::UpdateChecked`、`check_update`）
- Modify: `src/ui/settings/window/mod.rs:97`（`SettingsAction::UpdateFound`）
- Modify: `src/ui/settings/window/about.rs`（結果型別）
- Test: `tests/it/updater.rs`（改寫 `fetch_latest_release_*` 系列）

**Interfaces:**
- Produces:
  - `pub struct Release { pub version: String, pub url: String, pub published_at: String, pub body: String, pub asset: Option<Asset> }`（`published_at` 為 API 原字串 RFC 3339；`#[derive(Debug, Clone, PartialEq, Eq)]`）
  - `pub struct Asset { pub url: String, pub size: u64, pub sha256: String }`（`sha256` 已去掉 `sha256:` 前綴；asset 沒有 `digest` 時 `Release.asset` 為 `None`）
  - `pub const RELEASES_API: &str = "https://api.github.com/repos/GoneTone/wizard101-chat-translator/releases";`（取代 `LATEST_API`）
  - `pub const EXE_ASSET: &str = "Wizard101ChatTranslator.exe";`
  - `pub fn parse_releases(json: &Value, current: &str) -> Result<Vec<Release>, UpdateCheckError>`：排除 `draft`／`prerelease`、只留 `is_newer(tag, current)`，依版本由新到舊；根不是陣列回 `Err("unexpected response: ...")`，個別項目缺 `tag_name` 就跳過並記 log。
  - `pub async fn check_for_update(client: &reqwest::Client, url: &str, current: &str) -> Result<Vec<Release>, UpdateCheckError>`：空 vec＝沒有新版；404 回 `Ok(vec![])`；其餘非 200 回 `Err("HTTP {code}")`。log `[update] newer releases: count={n} latest={v}`。
  - `schedule_update_checks(..., found: Arc<dyn Fn(Vec<Release>) + Send + Sync>)`：非空才呼叫。
  - `UiEvent::UpdateFound(Vec<Release>)`、`SettingsEvent::UpdateChecked { round, result: Result<Vec<Release>, String> }`、`SettingsAction::UpdateFound(Vec<Release>)`，vec 皆新到舊且非空（`UpdateChecked` 的 `Ok(vec![])`＝已是最新）。
  - 刪掉 `fetch_latest_release` 與 `LATEST_API`。

- [ ] **Step 1: 改寫測試**

在 `tests/it/updater.rs` 把 `release_json()` 換成三筆的陣列 fixture（`v1.2.0`、`v1.1.0` 正式版；`v1.3.0-rc.1` prerelease；外加一筆 `draft: true` 的 `v9.0.0`），每筆含 `html_url`、`published_at`、`body`、`assets: [{name, browser_download_url, size, digest}]`。測試：

```rust
#[test]
fn parse_releases_keeps_newer_published_releases_newest_first() {
    let list = parse_releases(&releases_json(), "1.0.0").unwrap();
    let versions: Vec<_> = list.iter().map(|r| r.version.as_str()).collect();
    assert_eq!(versions, ["1.2.0", "1.1.0"]);
    let asset = list[0].asset.as_ref().unwrap();
    assert_eq!(asset.sha256, "be76267b7f18060bab1fe7ca9d63822c84974f5aed16e4688b1c0de575e84d79");
    assert_eq!(asset.size, 61598208);
    assert_eq!(list[0].published_at, "2026-11-03T11:58:24Z");
}

#[test]
fn parse_releases_drops_the_asset_without_a_digest() { /* 把 digest 拿掉 → asset == None，release 仍在 */ }

#[test]
fn parse_releases_ignores_other_assets() { /* 多一個 name 不是 EXE_ASSET 的 asset → 取到的是 exe 那個 */ }

#[test]
fn parse_releases_is_empty_when_current_is_latest() { /* current = "1.2.0" → 空 */ }

#[test]
fn parse_releases_rejects_a_non_array() { /* json!({}) → Err，訊息含 "unexpected response" */ }
```

`fetch_latest_release_*` 的 wiremock 測試改成打 `check_for_update`（path 改 `/releases`）：讀到清單、404 回空、其他狀態碼回 `HTTP {code}`、連不上回錯、payload 壞掉回錯、轉址跟隨。`schedule_update_checks` 那組改收 `Vec<Release>`，`found_is_not_called_without_a_newer_release` 照舊。

- [ ] **Step 2: 跑測試確認失敗**

Run: `cargo test -j 4 --test it updater:: -- --test-threads=4`
Expected: 編譯失敗（`parse_releases`、`RELEASES_API` 不存在）

- [ ] **Step 3: 實作 `updater.rs` 並修好所有呼叫端**

依 Interfaces 改。`App`、`about.rs`、`banners` 這個 task 只取 `releases[0]`，點擊行為仍開 `releases[0].url`（Task 6 才換成開視窗）。

- [ ] **Step 4: 跑測試確認通過**

Run: `cargo test -j 4 --test it updater:: -- --test-threads=4`
Expected: PASS

- [ ] **Step 5: lint、全套測試、commit**

```bash
git commit -m "feat(update): fetch every newer release with notes and exe asset"
```

---

### Task 2: 替換 exe、清除殘留、重啟交接

**Files:**
- Create: `src/runtime/self_update.rs`（並在 `src/runtime/mod.rs` 宣告）
- Modify: `src/runtime/single_instance.rs`（`acquire_waiting`）
- Modify: `src/main.rs`（啟動時等鎖、清殘留）
- Modify: `src/app_main.rs:918`（`shutdown` 在 `exit` 前呼叫 `relaunch_if_requested`）
- Test: `tests/it/self_update.rs`（新模組，在 `tests/it/main.rs` 宣告）

**Interfaces:**
- Produces（`crate::runtime::self_update`）：
  - `pub const AFTER_UPDATE_ARG: &str = "--after-update";`
  - `pub fn new_path(exe: &Path) -> PathBuf`（`<exe>.new`）、`pub fn part_path(exe: &Path) -> PathBuf`（`<exe>.new.part`，須與 `local::download` 的 `.part` 規則一致）、`pub fn old_path(exe: &Path) -> PathBuf`（`<exe>.old`）
  - `pub fn replace_exe(exe: &Path) -> io::Result<()>`：刪舊 `.old`（不存在不算錯）→ `exe`→`.old` → `.new`→`exe`；最後一步失敗就把 `.old` 改回 `exe` 再回原錯誤。每步記 log。
  - `pub fn remove_leftovers(exe: &Path)`：刪 `.old`、`.new`、`.new.part`，不存在略過，刪不掉記 log（`[update] could not delete leftover {path}: {err}`）。
  - `pub fn request_relaunch(exe: PathBuf)`：存進 process 內的 `static Mutex<Option<PathBuf>>`。
  - `pub fn relaunch_if_requested()`：有登記就 `Command::new(exe).arg(AFTER_UPDATE_ARG).spawn()`（沿用目前權限），記成功或失敗。
- Produces（`crate::runtime::single_instance`）：
  - `pub fn acquire_waiting<L>(clock: &Clock, wait: Duration, acquire: impl FnMut() -> Result<L, AcquireError>) -> Result<L, AcquireError>`：只在 `AlreadyRunning` 時以 `poll_until`（`crate::util`）每 200 ms 重試到 `wait`；逾時回最後一次的結果。
- Consumes: `crate::util::{Clock, poll_until}`。

- [ ] **Step 1: 寫測試**

```rust
#[test]
fn replace_swaps_in_the_new_exe_and_keeps_the_old_one() {
    // tempdir 裡建 app.exe("old")、app.exe.new("new")
    replace_exe(&exe).unwrap();
    assert_eq!(fs::read_to_string(&exe).unwrap(), "new");
    assert_eq!(fs::read_to_string(old_path(&exe)).unwrap(), "old");
    assert!(!new_path(&exe).exists());
}

#[test]
fn replace_overwrites_a_stale_old_file() { /* 先放 app.exe.old("stale") → 成功後 .old 內容是 "old" */ }

#[test]
fn replace_restores_the_exe_when_the_new_file_is_missing() {
    // 沒有 .new → Err，app.exe 內容仍是 "old"，.old 不存在
}

#[test]
fn replace_fails_without_touching_exe_when_old_cannot_be_removed() {
    // .old 以 OpenOptions + std::os::windows::fs::OpenOptionsExt::share_mode(0) 開著不放
    // （std 預設的共用模式允許刪除，一定要關掉）→ Err，app.exe 仍是 "old"
}

#[test]
fn remove_leftovers_deletes_all_three_and_ignores_missing() { /* 只放 .old 與 .new.part → 兩個都刪掉、不 panic */ }

#[test]
fn acquire_waiting_retries_until_the_lock_frees() {
    // acquire 前兩次回 AlreadyRunning、第三次 Ok(7)；假 Clock 每次 sleep 推進時間 → Ok(7)，呼叫 3 次
}

#[test]
fn acquire_waiting_gives_up_after_the_deadline() { /* 一直 AlreadyRunning、wait = 1s → Err(AlreadyRunning) */ }

#[test]
fn acquire_waiting_returns_create_failures_immediately() { /* Err(CreateFailed(5)) → 只呼叫一次 */ }
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `cargo test -j 4 --test it self_update:: -- --test-threads=4`
Expected: 編譯失敗（模組不存在）

- [ ] **Step 3: 實作 `self_update.rs` 與 `acquire_waiting`**

- [ ] **Step 4: 接進 `main.rs` 與 `shutdown`**

`main.rs` 開頭 `let lock = acquire_single_instance();` 改成：參數含 `AFTER_UPDATE_ARG` 時用 `acquire_waiting(&Clock::default(), Duration::from_secs(10), acquire_single_instance)`，否則照舊。這段必須在開 `app.log` 之前（現有註解說明了原因）。`claim_instance` 成功之後、`sweep_stale_temps` 旁邊呼叫 `remove_leftovers(&std::env::current_exe()?)`（`current_exe` 失敗就記 log 略過）。`shutdown()` 在 `std::process::exit(0)` 前呼叫 `relaunch_if_requested()`。log 等鎖結果：`[update] after-update: instance lock acquired after {ms} ms` 或 `... still held after 10 s`。

- [ ] **Step 5: 跑測試確認通過**

Run: `cargo test -j 4 --test it self_update:: -- --test-threads=4`
Expected: PASS

- [ ] **Step 6: lint、全套測試、commit**

```bash
git commit -m "feat(update): replace the running exe and hand over to the new one on restart"
```

---

### Task 3: 下載狀態機

**Files:**
- Create: `src/runtime/update_job.rs`（在 `src/runtime/mod.rs` 宣告）
- Modify: `src/net.rs`（或 `net` 模組所在檔）、`src/local/hf.rs:64`：把 `hf_client` 的建構（只設 `connect_timeout`／`read_timeout`、轉址、User-Agent、本機不走 proxy）搬成 `pub fn download_client(base: &str) -> reqwest::Client`，`hf_client` 改呼叫它。大檔下載不可用 `update_client`：它的 `timeout` 是整個請求含內文的總時間（10 秒），60 MB 必定逾時。
- Test: `tests/it/update_job.rs`（新模組）

**Interfaces:**
- Consumes: Task 1 的 `Release`／`Asset`；Task 2 的 `new_path`／`part_path`；`crate::local::download::{download, Progress, DownloadError}`。
- Produces（`crate::runtime::update_job`）：

```rust
pub enum Phase {
    Idle,                                   // ①
    Downloading,                            // ②；total > 0 && done == total 時 UI 顯示 ④
    Ready,                                  // ⑧（驗證通過、.new 在磁碟上）
    Restarting,                             // ⑤
    Failed { error: String, replace: bool }, // ⑥；replace = 替換那步失敗（重試不重新下載）
    Cancelled,                              // ⑦
}

pub struct UpdateJob { /* releases, phase, progress: Arc<Progress>, cancel: Option<CancellationToken>, exe: PathBuf, downloading: Option<String> 版本 */ }

impl UpdateJob {
    pub fn new(exe: PathBuf) -> Self;
    pub fn releases(&self) -> &[Release];          // 新到舊
    pub fn phase(&self) -> &Phase;
    pub fn progress(&self) -> (u64, u64);          // (done, total)
    /// 新清單；最新版比 Ready 的那份新就刪 `.new` 回 Idle。下載中先暫存不換（標題才不會在下載 1.2.0 時寫 1.3.0），`finish` 時再換上並丟棄過期的 `.new`。
    pub fn set_releases(&mut self, releases: Vec<Release>);
    /// ①⑦⑥(下載失敗) → ②；最新版沒有 asset 直接進 Failed。`done` 在下載任務結束時被呼叫（任何執行緒）。
    pub fn start(&mut self, rt: &Handle, http: reqwest::Client, done: Box<dyn FnOnce(Result<(), String>) + Send>);
    /// 下載任務的結果（App 從 UI 事件轉進來）：Ok → Ready；Err → Failed{replace:false}；phase 已是 Cancelled 時不改 phase，只刪 `.new.part`。
    pub fn finish(&mut self, result: Result<(), String>);
    /// ② → ⑦：只取消 token。`.new.part` 等任務結束、`finish` 進來時才刪（任務可能還開著那個檔）。
    pub fn cancel(&mut self);
    /// 視窗被關：沒到 Ready 就刪 `.new.part`（Ready 保留 `.new`）；Failed/Cancelled 回 Idle，Ready 維持。
    pub fn window_closed(&mut self);
    /// Ready（或 Failed{replace:true}）→ Restarting。
    pub fn begin_restart(&mut self);
    /// 替換失敗 → Failed{replace:true}。
    pub fn restart_failed(&mut self, error: String);
}
```

- `start` 收到的 `http` 是 `download_client` 建的 client。
- 下載用 `download(&http, &asset.url, &new_path(exe), asset.size, Some(&asset.sha256), &progress, &cancel)`；`DownloadError` 以 `to_string()` 轉成錯誤字串。
- log：開始（版本、大小）、完成、失敗原文、取消、刪 `.part`／`.new`、丟棄過期下載。

- [ ] **Step 1: 寫測試**

wiremock 伺服一個小檔（例如 `b"fake exe"`），fixture 的 `Asset.sha256` 用它真正的 sha256；`exe` 指向 tempdir 裡的 `app.exe`。`done` 回呼用 `tokio::sync::oneshot` 把結果送回測試，再呼叫 `job.finish(result)`。

```rust
#[tokio::test(flavor = "multi_thread")]
async fn download_ends_ready_with_the_new_file() { /* start → finish → Phase::Ready、new_path 內容 == b"fake exe" */ }

#[tokio::test(flavor = "multi_thread")]
async fn checksum_mismatch_fails_without_a_new_file() { /* sha256 給錯 → Failed{replace:false}、錯誤含 "sha256 mismatch"、.new 不存在 */ }

#[tokio::test(flavor = "multi_thread")]
async fn cancel_deletes_the_partial_file() {
    // 伺服器 set_delay 讓下載卡住；cancel() → Phase::Cancelled；等 done 回呼送回 Err 再 finish(Err)
    // → 仍是 Cancelled、.new.part 不存在
}

#[test]
fn start_without_verifiable_asset_fails_immediately() { /* asset: None → Failed{replace:false}，錯誤含版本與 EXE_ASSET */ }

#[test]
fn closing_after_a_failed_download_deletes_the_partial_file() { /* Failed{replace:false} 狀態＋.new.part → window_closed → 刪掉、Phase::Idle */ }

#[test]
fn closing_when_ready_keeps_the_new_file() { /* Ready＋.new → window_closed → .new 還在、Phase::Ready */ }

#[test]
fn newer_release_discards_ready_download() { /* Ready（1.2.0）→ set_releases([1.3.0, ...]) → .new 刪掉、Phase::Idle */ }

#[test]
fn same_release_list_keeps_ready_download() { /* 再 set 一次 1.2.0 → 維持 Ready */ }

#[tokio::test(flavor = "multi_thread")]
async fn newer_release_during_download_discards_on_finish() {
    // 下載 1.2.0 中 set_releases([1.3.0]) → releases()[0] 仍是 1.2.0 → finish(Ok) → releases()[0] 是 1.3.0、.new 刪掉、Phase::Idle
}

#[test]
fn replace_failure_retry_goes_straight_to_restart() { /* Ready → begin_restart → restart_failed("x") → Failed{replace:true} → begin_restart → Restarting */ }

#[test]
fn open_does_not_reset_phase() { /* Restarting 時 set_releases(同一份) → 仍 Restarting */ }
```

（沒有網路的測試直接用「設定好檔案的 tempdir＋`UpdateJob` 的 test-support 建構子」進到指定 phase：在 `#[cfg(feature = "test-support")]` 下提供 `pub fn with_phase(exe: PathBuf, releases: Vec<Release>, phase: Phase) -> Self`。）

- [ ] **Step 2: 跑測試確認失敗**

Run: `cargo test -j 4 --test it update_job:: -- --test-threads=4`
Expected: 編譯失敗

- [ ] **Step 3: 實作 `update_job.rs`**

- [ ] **Step 4: 跑測試確認通過**

Run: `cargo test -j 4 --test it update_job:: -- --test-threads=4`
Expected: PASS

- [ ] **Step 5: lint、全套測試、commit**

```bash
git commit -m "feat(update): add the download state machine for in-app updates"
```

---

### Task 4: 更新內容視窗的畫面

**Files:**
- Modify: `Cargo.toml`（加 `egui_commonmark`）
- Create: `src/ui/settings/update_window.rs`（在 `src/ui/settings/mod.rs` 宣告 `pub mod update_window;`；放這裡才能用 `widgets` 的 `pub(super)` 元件）
- Modify: `src/ui/settings/local_rate.rs`（把 `Rates::observe` 的取樣核心抽成 `pub fn observe_running(&mut self, now: Instant, running: BTreeMap<String, u64>)`，`observe` 改呼叫它）
- Modify: `src/i18n/locales/{en-US,zh-TW,zh-CN}.json`
- Test: `tests/it/update_window.rs`（新模組，egui_kittest）

**Interfaces:**
- Consumes: Task 1 `Release`；Task 3 `Phase`。
- Produces（`crate::ui::settings::update_window`）：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateAction { Start, Cancel, Retry, OpenReleasePage, Close, Restart }

pub struct UpdateView { /* CommonMarkCache, Rates */ }

impl UpdateView {
    pub fn new() -> Self;
    /// 畫一幀；`unsaved` 是設定視窗有沒有沒儲存的變更（決定 ⑧-1／⑧-2）。
    pub fn ui(&mut self, ui: &mut Ui, releases: &[Release], phase: &Phase, progress: (u64, u64), unsaved: bool) -> Vec<UpdateAction>;
}
```

- 版面與每個狀態的底部列、按鈕、✕ 是否可關，照 spec「已定案的 UI」兩張表。「立即更新」「重試」「重新下載」都回 `Start`／`Retry`（`Retry` 在 `Failed{replace:true}` 時由 App 轉成重啟）；「稍後」「關閉」回 `Close`；⑧-1 紅色按鈕與 ⑧-2「重新啟動」都回 `Restart`。紅色主按鈕用 `widgets::danger_button`。
- 進度文字：`update.downloading`＝「下載中 {done} / {total} MB」（MB 取一位小數），有速度時接 ` · {speed}`（`local_rate::format_speed`）；速度用自己的 `Rates`，key 用 `"update"`。
- 日期：`chrono::DateTime::parse_from_rfc3339(published_at)` 轉本機時區 `%Y-%m-%d`；解析失敗就顯示原字串。
- 視窗是否允許關閉另給 `pub fn can_close(phase: &Phase) -> bool`（② 與 ⑤ 為 false）。

新 i18n key（zh-TW／en-US；zh-CN 照 zh-TW 轉簡體）：

| key | zh-TW | en-US |
|---|---|---|
| `update.title` | `{app} — 更新` | `{app} — Update` |
| `update.heading` | `有新版本 v{version}` | `Version {version} is available` |
| `update.subheading` | `目前版本 v{current} · 共 {count} 個新版本` | `Current version {current} · {count} new versions` |
| `update.published` | `{date} 發布` | `Released {date}` |
| `update.latest_tag` | `最新` | `Latest` |
| `update.later` | `稍後` | `Later` |
| `update.now` | `立即更新` | `Update now` |
| `update.cancel` | `取消下載` | `Cancel download` |
| `update.downloading` | `下載中 {done} / {total} MB` | `Downloading {done} / {total} MB` |
| `update.verifying` | `正在驗證檔案…` | `Verifying the file…` |
| `update.restarting` | `✓ 下載完成，正在重新啟動…` | `✓ Downloaded, restarting…` |
| `update.failed_download` | `更新失敗：{error}` | `Update failed: {error}` |
| `update.open_github` | `前往 GitHub 下載` | `Download from GitHub` |
| `update.retry` | `重試` | `Retry` |
| `update.cancelled` | `已取消下載` | `Download cancelled` |
| `update.close` | `關閉` | `Close` |
| `update.redownload` | `重新下載` | `Download again` |
| `update.unsaved` | `⚠ 設定視窗有變更還沒儲存，重新啟動後會遺失` | `⚠ The settings window has unsaved changes; they will be lost when restarting` |
| `update.discard_restart` | `放棄變更並重新啟動` | `Discard changes and restart` |
| `update.ready` | `✓ 下載完成，可以重新啟動了` | `✓ Downloaded and ready to restart` |
| `update.restart` | `重新啟動` | `Restart` |

（同時把 `update.available` 改成 Global Constraints 寫的文字。）

- [ ] **Step 1: 寫測試**

用 `egui_kittest::Harness::new_ui` 包 `UpdateView::ui`，fixture 兩筆 release（1.2.0、1.1.0），標籤一律經 `t(...)` 取字（不綁翻譯句）。

```rust
#[test] fn idle_lists_every_release_newest_first() { /* 看得到 "v1.2.0" 與 "v1.1.0"、1.2.0 在上；有 update.latest_tag；按 update.now → [Start] */ }
#[test] fn downloading_shows_progress_and_only_cancel() { /* (25 MiB, 61 MiB) → progress 節點存在、update.cancel 可按 → [Cancel]、沒有 update.now */ }
#[test] fn verifying_disables_cancel() { /* Downloading 且 done == total > 0 → update.verifying、update.cancel disabled */ }
#[test] fn downloading_at_zero_bytes_is_not_verifying() { /* (0, 0) → 沒有 update.verifying、update.cancel 可按 */ }
#[test] fn restarting_disables_cancel() { /* Restarting → update.restarting、update.cancel disabled */ }
#[test] fn failed_offers_github_and_retry() { /* Failed{error:"boom"} → 文字含 "boom"；按 update.retry → [Retry]；按 update.open_github → [OpenReleasePage] */ }
#[test] fn cancelled_offers_close_and_redownload() { /* update.close → [Close]；update.redownload → [Start] */ }
#[test] fn ready_with_unsaved_settings_asks_to_discard() { /* unsaved = true → update.unsaved、按 update.discard_restart → [Restart] */ }
#[test] fn ready_without_unsaved_settings_offers_restart() { /* unsaved = false → update.ready、按 update.restart → [Restart] */ }
#[test] fn markdown_body_is_rendered() { /* body "## What's New\n\n- item" → 看得到 "What's New" 與 "item"，看不到 "##" */ }
#[test] fn can_close_only_outside_download_and_restart() { /* Downloading/Restarting → false；其他 → true */ }
```

- [ ] **Step 2: 跑測試確認失敗**

Run: `cargo test -j 4 --test it update_window:: -- --test-threads=4`
Expected: 編譯失敗

- [ ] **Step 3: 加相依、抽 `observe_running`、實作 `update_window.rs`、加 i18n key**

- [ ] **Step 4: 跑測試確認通過**

Run: `cargo test -j 4 --test it update_window:: -- --test-threads=4`，再跑 `cargo test -j 4 --test it no_hardcoded_ui_text:: -- --test-threads=4` 與 `cargo test -j 4 --test i18n_golden -- --test-threads=4`
Expected: PASS

- [ ] **Step 5: lint、全套測試、commit**

```bash
git commit -m "feat(update): add the release notes window view"
```

---

### Task 5: 獨立視窗與 App 接線

**Files:**
- Create: `src/ui/update_host.rs`（在 `src/ui/mod.rs` 宣告）
- Modify: `src/ui/settings_host.rs`（`HostedWindow`、`Spec` 及其欄位、`WindowOps` 改 `pub(crate)`；`impl SettingsHost { pub fn has_unsaved(&self) -> bool }` 轉呼叫 `SettingsWindow::has_unsaved`，沒開窗為 false）
- Modify: `src/ui/events.rs`（`UiEvent::UpdateDownloaded(Result<(), String>)`）
- Modify: `src/ui/app.rs`（持有 `UpdateHost`，接 `UpdateFound`／`UpdateDownloaded`、橫幅點擊、`SettingsAction::OpenUpdate`、重啟）
- Modify: `src/ui/overlay/banners.rs`（點整列改設旗標：`pub fn take_update_click(&mut self) -> bool`，不再 `open_url`）
- Modify: `src/ui/settings/window/mod.rs`、`about.rs`（新版連結改成按鈕樣式的連結，點了送 `SettingsAction::OpenUpdate`）
- Modify: `src/main.rs`（建 `UpdateHost` 時給 runtime handle、`download_client("https://github.com")`（Task 3；**不是** `update_client`，那個只給 `/releases` 查詢用）、`UiPoster`、`current_exe()`）
- Modify: `README.md:46,76`、`README_ZH-TW.md:46,76`、`README_ZH-CN.md:46,76`：「更新版本」段改成在程式內更新（手動換檔留作退路），橫幅描述從「點橫幅開下載頁」改成開更新內容視窗。依專案慣例不寫按鈕文字與鍵名。
- Test: `tests/it/update_host.rs`（新模組）；`src/ui/overlay/banners.rs` 既有測試改點擊斷言

**Interfaces:**
- Consumes: Task 3 `UpdateJob`；Task 4 `UpdateView`／`UpdateAction`／`can_close`；Task 2 `replace_exe`／`request_relaunch`；`updater::RELEASES_URL`。
- Produces（`crate::ui::update_host`）：

```rust
pub struct UpdateHost { /* job: UpdateJob, view: Option<UpdateView>（開著才有）, win: HostedWindow, rt: Handle, http: reqwest::Client, ui: UiPoster, auto_restart_pending: bool */ }

pub enum HostAction { Restart }   // App 收到才做替換與關根視窗

impl UpdateHost {
    pub fn new(rt: Handle, http: reqwest::Client, ui: UiPoster, exe: PathBuf) -> Self;
    pub fn set_window_ops(&mut self, ops: WindowOps);       // 測試用
    pub fn set_releases(&mut self, releases: Vec<Release>);  // 轉給 job
    pub fn open(&mut self);                                  // 已開就 bring_to_front；清單為空就不開並記 log
    pub fn is_open(&self) -> bool;
    pub fn hwnd(&self) -> isize;
    pub fn downloaded(&mut self, result: Result<(), String>); // 轉 job.finish；進 Ready 時設 auto_restart_pending
    pub fn restart_failed(&mut self, error: String);
    /// 每幀；`unsaved` 由 App 從 SettingsHost 取。
    pub fn show(&mut self, ctx: &egui::Context, known: &[isize], unsaved: bool) -> Vec<HostAction>;
}
```

- `Spec`：`name: "update window"`、`title_key: "update.title"`、`size: vec2(620.0, 600.0)`、`min_size: vec2(480.0, 360.0)`、`topmost_once: true`。
- `show` 內處理動作：`Start` → `job.start(..., done 回呼 post UiEvent::UpdateDownloaded)`；`Cancel` → `job.cancel()`；`Retry` → `Failed{replace:true}` 時同 `Restart`，否則同 `Start`；`OpenReleasePage` → `ctx.open_url(releases[0].url)`（最新 release 的網頁，同現行行為）；`Close` 與 ✕ → `job.window_closed()` 並關窗；`Restart` → 驗證後的設定檢查：`unsaved` 為 false 或使用者按的是放棄鈕都直接 `job.begin_restart()`，然後在**下一幀**（⑤ 已畫出）回傳 `HostAction::Restart`。
- 驗證後的設定檢查在 `show()` 做（`downloaded` 拿不到 `unsaved`）：`auto_restart_pending` 為 true 時取走它，`unsaved == false` 就 `begin_restart`（不停在 ⑧），`true` 就停在 Ready 顯示 ⑧-1。每次下載完成只判斷這一次：之後關窗再打開，Ready 一律顯示 ⑧ 等使用者按，不自動重啟。log `[update] ready; settings unsaved={bool}`。
- ✕ 在 `can_close` 為 false 時送 `ViewportCommand::CancelClose`。
- App 收到 `HostAction::Restart`：`replace_exe(&exe)`；Ok → `request_relaunch(exe)`、`ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close)`（走既有 `on_close` → `shutdown`）；Err → `update.restart_failed(e.to_string())`。
- App 收到 `UiEvent::UpdateFound(list)`：`update.set_releases(list.clone())`、`banners.offer_update(list[0].clone())`；`SettingsAction::UpdateFound(list)` 同理但用 `set_update`。
- 橫幅 `take_update_click()` 為 true 或收到 `SettingsAction::OpenUpdate` → `update.open()`。
- `viewport_hwnds()` 納入 `update.hwnd()`。
- `overlay_level`：更新視窗開著時比照設定視窗，把置頂的 overlay 降為一般層級（`topmost_once` 只管第一幀，之後點一下遊戲，更新視窗就會被 overlay 蓋住）。

- [ ] **Step 1: 寫測試**

`tests/it/update_host.rs` 用 `Context` 跑幀（參考 `settings_host.rs` 測試的 `offscreen_ops` 與 `CLOSE_NEXT` 作法），`UpdateJob` 用 Task 3 的 `with_phase` 建好狀態：

```rust
#[test] fn open_without_releases_stays_closed() {}
#[test] fn open_twice_keeps_one_window() { /* 第二次 open 不重設 phase */ }
#[test] fn close_request_is_cancelled_while_downloading() { /* phase Downloading + 關閉要求 → 仍 is_open、送出 CancelClose */ }
#[test] fn closing_when_failed_deletes_the_partial_file() { /* Failed + .new.part + 關閉要求 → 關窗、.new.part 不存在 */ }
#[test] fn ready_without_unsaved_restarts_on_the_next_frame() { /* downloaded(Ok) 且 unsaved=false → 第一幀 phase Restarting、回傳空；第二幀回傳 [HostAction::Restart] */ }
#[test] fn ready_with_unsaved_waits_for_the_user() { /* unsaved=true → 停在 Ready，不回傳 Restart */ }
#[test] fn reopening_when_ready_does_not_auto_restart() { /* Ready 時關窗再 open、unsaved=false 跑兩幀 → 仍 Ready、不回傳 Restart */ }
```

`banners.rs` 測試：點更新橫幅列後 `take_update_click()` 為 true、第二次為 false，且沒有排入 `open_url`。`app.rs` 既有 harness 測試加兩則：送 `UiEvent::UpdateFound(vec![r])` 後點橫幅 → `update` 視窗 `is_open()`；更新視窗開著時 overlay 層級為 `WindowLevel::Normal`。

- [ ] **Step 2: 跑測試確認失敗**

Run: `cargo test -j 4 --test it update_host:: -- --test-threads=4`
Expected: 編譯失敗

- [ ] **Step 3: 實作 `update_host.rs` 與各處接線**

- [ ] **Step 4: 跑測試確認通過**

Run: `cargo test -j 4 --test it update_host:: -- --test-threads=4` 與 `cargo test -j 4 --lib banners -- --test-threads=4`
Expected: PASS

- [ ] **Step 5: lint、全套測試、commit**

```bash
git commit -m "feat(update): open the release notes window and restart into the new version"
```

---

### Task 6: 實機驗證

不寫新程式；照 CLAUDE.md 品質檢查與 `docs/releasing.md` 的建置方式，把整條流程在真的 exe 上跑一遍。桌面操作交給使用者，不自行操控。

- [ ] **Step 1: 準備兩個版本**

`Cargo.toml` 版本暫改 `0.0.1` 建一份 release exe 複製到 `_exe_test/`（不 commit 這個改動），讓它查到 GitHub 上的 v1.2.0。

- [ ] **Step 2: 請使用者照清單操作並回報 `app.log`**

清單：點橫幅開視窗、卡片與日期正確、立即更新→進度→取消（`.new.part` 消失）、再下載→設定視窗留未存變更→停在 ⑧-1→去存→⑧-2→重新啟動→新版開起來、`_exe_test/` 裡沒有 `.old`／`.new` 殘留、遊戲內 hook 有正確還原（舊程序 log 有 shutdown complete）。

注意：這次重啟進去的是已發佈的 v1.2.0，它還不認得 `--after-update`，可能在舊程序還沒放掉單一實例鎖時就啟動，聚焦舊視窗後自己退出。看到「沒重開」先查 `app.log` 是不是 `another instance is already running`，那不是這次實作的 bug。完整的等鎖交接要等下一次發版（從含本功能的版本更新到更新的版本）才會實際走到。

- [ ] **Step 3: 依回報修正後還原版本號**

---

## Self-Review 紀錄

- Spec 每一節都有對應：清單（T1）、替換重啟與等鎖與清殘留（T2）、下載／取消／清除／過期丟棄（T3）、畫面與 ⑧（T4）、視窗與進入點與文案（T5、T4）、實機（T6）。
- 型別一致：`Release`／`Asset`／`Phase`／`UpdateAction`／`HostAction` 在各 task 名稱相同。
- 留給實作者：`egui_commonmark` 樣式細節（是否開 `regex` feature 讓裸網址變連結，以實測決定）、`UpdateView` 內部排版。
