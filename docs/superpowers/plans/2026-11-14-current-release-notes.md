# 關於頁查看本版更新內容 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 關於頁版本號後加「本版更新內容」連結，點下去開獨立視窗，顯示目前執行版本在 GitHub 上的版本說明。

**Architecture:** `runtime/updater.rs` 新增查單一 release 的函式。更新視窗的 release 卡片抽成共用模組。新視窗分成兩部分：`ui/settings/release_notes_window.rs` 只負責畫面，`ui/release_notes_host.rs` 負責托管視窗與背景請求，結果以 `UiEvent` 回到 UI 執行緒。App 串接的方式比照 `UpdateHost`，但不依賴除錯建置／exe 路徑的條件。

**Tech Stack:** Rust、egui／eframe（immediate viewport）、egui_commonmark、reqwest、tokio、wiremock 與 egui_kittest（測試）。

**Spec:** `docs/superpowers/specs/2026-11-14-current-release-notes-design.md`

## Global Constraints

- 回答、註解、docstring、UI 文字用繁體中文全形標點；log 訊息一律英文，前綴 `[update]`／`[ui]`；commit message 用英文 conventional commits。
- 註解預設一行、只寫 WHY；模組 `//!` 與公開項目 `///` 一行摘要。
- 新字串補到 `src/i18n/locales/` 裡已有 `update.heading` 的語系：`en-US.json`、`zh-TW.json`、`zh-CN.json`（其他語系沒有這區的翻譯，不補）。英文有句號中文也要有（本計畫的字串都不帶句尾標點）。
- 不寫死語言；版本一律讀 `env!("CARGO_PKG_VERSION")`。
- 錯誤訊息只顯示原文（非 200 為 `HTTP {code}`），不加推測原因。
- cargo 一律 `-j 4`、測試 `--test-threads=4`；全套 `cargo test` 每個 task 只在 commit 前跑一次，開發中只跑相關測試。
- commit 前：`cargo fmt --check`、`cargo clippy -j 4 --all-targets -- -D warnings`、`cargo test -j 4 -- --test-threads=4` 全過。
- 新整合測試放 `tests/it/`，在 `tests/it/main.rs` 宣告模組。
- 視窗尺寸 620×600、最小 480×360；載入中用 `widgets::skeleton`，失敗用 `widgets::error_with_retry`。

## Review Focus

- **載入中又點一次連結**：只叫到前景，不重複發請求 → Task 4 `open_while_loading_does_not_request_again`。
- **還沒回應就關窗**：回來的結果照樣記住，下次開窗直接顯示，不再請求 → Task 4 `result_after_close_is_cached`。
- **失敗後按〔重試〕成功**：卡片換成內文 → Task 4 `retry_after_failure_shows_the_notes`。
- **設定視窗關了、本版說明還開著**：本版說明不跟著關，overlay 維持一般層級 → Task 5 `release_notes_window_keeps_the_overlay_at_normal_level`。
- **發布日期解析不了**：原樣顯示（沿用 `local_date` 的行為）→ Task 3 `unparsable_date_is_shown_as_is`。

---

### Task 0: 開分支

- [ ] **Step 1:** 在原目錄（不建 worktree）`git switch -c feat/current-release-notes`。
- [ ] **Step 2:** 跑 commit 前三項檢查，再 commit spec 與本計畫：`docs(spec): add current release notes design and plan`。

### Task 1: 查單一 release

**Files:**
- Modify: `src/runtime/updater.rs`（`parse_releases` 附近、`check_for_update` 之後）
- Test: `tests/it/updater.rs`

**Interfaces:**
- Produces:
  - `pub fn release_tag_url(api: &str, version: &str) -> String`：回傳 `format!("{api}/tags/v{version}")`。
  - `pub async fn fetch_release(client: &reqwest::Client, url: &str) -> Result<Option<Release>, UpdateCheckError>`：200 → `Ok(Some)`、404 → `Ok(None)`、其他狀態 → `Err(UpdateCheckError(format!("HTTP {code}")))`、JSON 壞掉或缺 `tag_name` → `Err(UpdateCheckError("unexpected response: …"))`。
  - 私有 `fn parse_release(item: &Value) -> Option<Release>`：從 `parse_releases` 迴圈抽出來（缺 `tag_name` 回 `None`），`parse_releases` 改用它。草稿／pre-release／`is_newer` 過濾留在 `parse_releases`。

- [ ] **Step 1: 寫失敗測試**（沿用檔內 `server_with`、`release_item`、`update_client` 等 helper；mock 路徑 `/releases/tags/v1.2.0`）

```rust
#[test]
fn release_tag_url_appends_the_v_tag() {
    assert_eq!(release_tag_url("https://x/releases", "1.3.1"), "https://x/releases/tags/v1.3.1");
}

#[tokio::test]
async fn fetch_release_reads_one_release() {
    // 200 + release_item("v1.2.0", ...)；結果的 version == "1.2.0"、body/url/published_at 與 item 相同
    // 同時驗證帶 Accept 與 User-Agent 標頭（同 check_for_update_reads_the_list）
}

#[tokio::test]
async fn fetch_release_is_none_on_404() { /* assert_eq!(…, Ok(None)) */ }

#[tokio::test]
async fn fetch_release_errors_on_other_status_codes() {
    // 403／429／500 → Err(UpdateCheckError(format!("HTTP {status}")))
}

#[tokio::test]
async fn fetch_release_errors_on_a_body_without_tag_name() {
    // 200 + json!({"body": "x"}) → Err，訊息以 "unexpected response" 開頭
}
```

- [ ] **Step 2:** `cargo test -j 4 --test it updater -- --test-threads=4` → 編譯失敗（函式不存在）。
- [ ] **Step 3:** 實作上述三個函式。`fetch_release` 的請求標頭與 404／非 200 分支和 `check_for_update` 共用一個私有 helper（例如 `async fn get_json(client, url) -> Result<Option<Value>, UpdateCheckError>`），不要複製一份。log：`[update] fetching release notes: {url}`、成功 `[update] release notes loaded: version={v} published={date}`、404 `[update] release notes not found (404)`。
- [ ] **Step 4:** 同 Step 2 的指令 → 全部 PASS（既有的 `parse_releases`／`check_for_update` 測試不能壞）。
- [ ] **Step 5:** 檢查三項後 commit：`feat(update): fetch a single release by tag`。

### Task 2: 抽出共用的 release 卡片

純重構，行為不變。

**Files:**
- Create: `src/ui/settings/release_card.rs`
- Modify: `src/ui/settings/update_window.rs`、`src/ui/settings/mod.rs`
- Test: 既有的 `tests/it/update_window.rs`（不改）

**Interfaces:**
- Produces（`pub(super)`，供 `update_window.rs` 與 Task 3 的 `release_notes_window.rs` 使用）：
  - `fn show(ui: &mut Ui, cache: &mut CommonMarkCache, release: &Release, latest: bool)`：就是現在的 `UpdateView::release_card` 本體。`latest` 決定要不要畫「最新」標籤。
  - `const HEADER_MARGIN: Margin`、`const LIST_PADDING: i8`、`const FOOTER_MARGIN: Margin`（= `Margin::symmetric(20, 14)`，取代 `update_window.rs` 裡寫死的值）。
  - `flatten_headings`、`local_date`、`MARKDOWN_HEADING_SIZE` 從 `update_window.rs` 移過來（模組私有即可，除非 Task 3 要用）。
- 卡片的外層 `card` + `Frame` 內距、版本列的列高計算一起搬，不要留在呼叫端。

- [ ] **Step 1:** 搬移程式碼，`UpdateView::releases` 改呼叫 `release_card::show`；`mod.rs` 加 `mod release_card;`。
- [ ] **Step 2:** `cargo test -j 4 --test it update_window -- --test-threads=4` → 全部 PASS（版面像素測試也要過，證明外觀沒變）。
- [ ] **Step 3:** 檢查三項後 commit：`refactor(update): share the release card between windows`。

### Task 3: 本版說明視窗的畫面

**Files:**
- Create: `src/ui/settings/release_notes_window.rs`
- Modify: `src/ui/settings/mod.rs`（`pub mod release_notes_window;`）、`src/ui/settings/release_card.rs`（載入中的變體）、三個語系檔
- Test: `tests/it/release_notes_window.rs`（新增，在 `tests/it/main.rs` 宣告）

**Interfaces:**
- Consumes: Task 2 的 `release_card::{show, HEADER_MARGIN, LIST_PADDING, FOOTER_MARGIN}`。
- Produces:

```rust
/// 本版說明的載入狀態。
#[derive(Debug, Clone, PartialEq)]
pub enum NotesState { Loading, Loaded(Release), NotFound, Failed(String) }

/// 使用者在本版說明視窗按下的動作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotesAction { Retry, OpenOnGitHub, Close }

#[derive(Default)]
pub struct ReleaseNotesView { /* markdown: CommonMarkCache */ }
impl ReleaseNotesView {
    pub fn new() -> Self;
    /// `version` 是目前執行的版本（不含 `v`）。
    pub fn ui(&mut self, ui: &mut Ui, version: &str, state: &NotesState) -> Vec<NotesAction>;
}
```

- 新增 `release_card::loading(ui: &mut Ui, version: &str)`：與 `show` 同一套外框與版本列，日期位置畫 `skeleton(ui, 96.0, 日期字高)`；分隔線下畫兩段佔位，每段一條標題列（寬約 120）加兩三條長短不一的內文列（寬度取可用寬度的比例，例如 100%／92%／64%、96%／58%）。
- 版面：`Panel::bottom` 底部列（`FOOTER_MARGIN`、`theme::MODAL_FOOT`）右側依序放〔在 GitHub 上查看〕secondary、〔關閉〕primary；`Panel::top` 標題區（`HEADER_MARGIN`）放粗體 20 的 `release_notes.heading` 與 `hint` 副標 `release_notes.subheading`；中間 `scroll_page`（`Margin::symmetric(20, LIST_PADDING)`）裡依狀態畫：
  - `Loading` → `release_card::loading`
  - `Loaded(r)` → `release_card::show(ui, cache, r, false)`
  - `NotFound` → 卡片內（版本列＋分隔線，與 `loading` 共用）放 `hint(release_notes.not_found, theme::TEXT_SECONDARY)`
  - `Failed(e)` → 卡片內 `error_with_retry(ui, &t("release_notes.failed", &[("error", e)]))`，按下回傳 `Retry`
- 語系字串（en-US／zh-TW／zh-CN）：

| key | en-US | zh-TW | zh-CN |
|---|---|---|---|
| `about.release_notes` | Release notes | 本版更新內容 | 本版更新内容 |
| `release_notes.title` | {app} — Release Notes | {app} — 本版更新內容 | {app} — 本版更新内容 |
| `release_notes.heading` | What's new in v{version} | v{version} 更新內容 | v{version} 更新内容 |
| `release_notes.subheading` | The version you're running | 目前使用的版本 | 当前使用的版本 |
| `release_notes.not_found` | No release notes for v{version} on GitHub | GitHub 上找不到 v{version} 的版本說明 | GitHub 上找不到 v{version} 的版本说明 |
| `release_notes.failed` | Couldn't load release notes: {error} | 載入失敗：{error} | 加载失败：{error} |
| `release_notes.open_github` | View on GitHub | 在 GitHub 上查看 | 在 GitHub 上查看 |

  〔關閉〕沿用 `update.close`；〔重試〕由 `error_with_retry` 自帶。

- [ ] **Step 1: 寫失敗測試**（harness 比照 `tests/it/update_window.rs` 的 `themed_harness(vec2(620.0, 600.0), "en-US", …)`）

```rust
#[test] fn loading_shows_placeholders_not_text() {
    // 標題 tr("release_notes.heading", version=1.3.1) 與卡片版本 "v1.3.1" 可見；
    // 至少一個無障礙名稱為 tr("local.loading") 的節點；沒有 retry 按鈕
}
#[test] fn loaded_shows_the_markdown_body() { /* body "## Fixed\n- a bug" → 可找到 "Fixed" 與 "a bug"；沒有「最新」pill */ }
#[test] fn unparsable_date_is_shown_as_is() { /* published_at = "someday" → tr("update.published", date="someday") 可見 */ }
#[test] fn not_found_has_no_retry() { /* 有 tr("release_notes.not_found", version) 文字；沒有 tr("local.dialog.retry") 按鈕 */ }
#[test] fn failed_offers_retry() { /* 有 tr("release_notes.failed", error="HTTP 403")；按 retry → actions == [NotesAction::Retry] */ }
#[test] fn footer_buttons_send_actions() { /* 按 open_github → [OpenOnGitHub]；按 update.close → [Close] */ }
```

- [ ] **Step 2:** `cargo test -j 4 --test it release_notes_window -- --test-threads=4` → 編譯失敗。
- [ ] **Step 3:** 實作 `ReleaseNotesView` 與 `release_card::loading`，補語系字串。
- [ ] **Step 4:** 同 Step 2 → PASS；另跑 `cargo test -j 4 --test it no_hardcoded_ui_text i18n -- --test-threads=4` 確認語系相關測試沒壞。
- [ ] **Step 5:** 檢查三項後 commit：`feat(about): add the release notes window view`。

### Task 4: 托管視窗與背景請求

**Files:**
- Create: `src/ui/release_notes_host.rs`
- Modify: `src/ui/mod.rs`（`pub mod release_notes_host;`）、`src/ui/events.rs`
- Test: `tests/it/release_notes_host.rs`（新增，在 `tests/it/main.rs` 宣告）

**Interfaces:**
- Consumes: Task 1 的 `fetch_release`、`release_tag_url`、`update_client`；Task 3 的 `ReleaseNotesView`、`NotesState`、`NotesAction`；`settings_host::{HostedWindow, Spec, WindowOps}`。
- Produces:
  - `UiEvent::ReleaseNotesLoaded(Result<Option<Release>, String>)`。
  - `const RELEASE_NOTES: Spec = Spec { name: "release notes window", title_key: "release_notes.title", size: vec2(620.0, 600.0), min_size: vec2(480.0, 360.0), topmost_once: true }`。
  - 公開方法：

```rust
pub struct ReleaseNotesHost { /* view: Option<ReleaseNotesView>, state: Option<NotesState>, win, rt, ui: UiPoster, api: String */ }
impl ReleaseNotesHost {
    /// `api` 是 releases API 的網址（正式為 `RELEASES_API`，測試換成 wiremock）。
    pub fn new(rt: Handle, ui: UiPoster, api: String) -> Self;
    pub fn set_window_ops(&mut self, ops: WindowOps);
    pub fn open(&mut self);
    pub fn is_open(&self) -> bool;
    pub fn hwnd(&self) -> isize;
    pub fn state(&self) -> Option<&NotesState>;
    pub fn loaded(&mut self, result: Result<Option<Release>, String>);
    pub fn show(&mut self, ctx: &egui::Context, known: &[isize]);
}
```

- 行為（不用輪次，「請求中不再發第二個」保證不會有過期結果）：
  - `open`：已開著 → `win.bring_to_front()`，log `already open; bringing it to the front`。否則建 view；`state` 是 `Some(Loaded)` → log `using cached release notes`，不請求；是 `Some(Loading)` → 不請求；其他（`None`／`NotFound`／`Failed`）→ 設 `Loading` 並發請求。
  - 發請求：`rt.spawn` 打 `fetch_release(&update_client(&url), &release_tag_url(&api, env!("CARGO_PKG_VERSION")))`，錯誤轉 `String` 並 log `[update] release notes fetch failed: {e}`，再 `ui.post(UiEvent::ReleaseNotesLoaded(..))`。
  - `loaded`：`Ok(Some)` → `Loaded`、`Ok(None)` → `NotFound`、`Err` → `Failed`。視窗關著也照樣存。
  - 動作：`Retry` → 設 `Loading` 並重發；`OpenOnGitHub` → `Loaded(r)` 開 `r.url`，否則開 `format!("{PROJECT_URL}/releases")`，log `[update] opening release notes page: {url}`；`Close`（含按 ✕）→ 丟掉 view、`win.reset()`，log `release notes window closed`。

- [ ] **Step 1: 寫失敗測試**（`Frames` 幀驅動比照 `tests/it/update_host.rs`；wiremock mock `/tags/v{CARGO_PKG_VERSION}`，`.expect(n)` 驗證請求次數；從 `UiPoster::for_test()` 的 receiver 取出 `ReleaseNotesLoaded` 再交給 `host.loaded`）

```rust
#[tokio::test(flavor = "multi_thread")] async fn open_fetches_and_shows_the_notes()      // expect(1)；state 變 Loaded
#[tokio::test(flavor = "multi_thread")] async fn open_while_loading_does_not_request_again() // 連續 open 兩次 → expect(1)
#[tokio::test(flavor = "multi_thread")] async fn reopening_uses_the_cache()              // Loaded 後 close、open → expect(1)
#[tokio::test(flavor = "multi_thread")] async fn result_after_close_is_cached()          // open → close → loaded(Ok(Some)) → open → expect(1)、state Loaded
#[tokio::test(flavor = "multi_thread")] async fn not_found_is_retried_on_next_open()     // 404 → NotFound；close、open → expect(2)
#[tokio::test(flavor = "multi_thread")] async fn retry_after_failure_shows_the_notes()   // 先 500 再 200（wiremock up_to_n_times）；按 Retry → Loaded
#[test] fn closing_resets_the_window()                                                   // 送關閉後 is_open() == false
```

- [ ] **Step 2:** `cargo test -j 4 --test it release_notes_host -- --test-threads=4` → 編譯失敗。
- [ ] **Step 3:** 實作 host 與 `UiEvent` 變體。
- [ ] **Step 4:** 同 Step 2 → PASS。
- [ ] **Step 5:** 檢查三項後 commit：`feat(about): host the release notes window`。

### Task 5: 關於頁連結與 App 串接

**Files:**
- Modify: `src/ui/settings/window/about.rs`（`hero_text`）、`src/ui/settings/window/mod.rs`（`SettingsAction`）、`src/ui/app.rs`、`src/main.rs`
- Test: `tests/it/settings_window.rs`、`src/ui/app.rs` 的 `mod tests`

**Interfaces:**
- Consumes: Task 4 的 `ReleaseNotesHost`、`UiEvent::ReleaseNotesLoaded`。
- Produces: `SettingsAction::OpenReleaseNotes`；`App::set_release_notes(&mut self, host: ReleaseNotesHost)`。

- 關於頁：版本那行改成 `ui.horizontal`，左邊保留原本的版本 `Label`（文字不變，既有測試 `about_tab_shows_version_and_links` 靠它），接著是同字級、`TEXT_SECONDARY` 色的 ` · `，再接 `Link`（`about.release_notes`、`HINT_SIZE`、`theme::LINK`）。點下去 log `[ui] release notes requested from the about tab`，並 push `SettingsAction::OpenReleaseNotes`。`hero_text` 需要改成 `&mut self`。
- App：
  - 新增欄位 `release_notes: Option<ReleaseNotesHost>` 與 setter。
  - `SettingsAction::OpenReleaseNotes` → `host.open()`；沒接上時 log `[update] release notes requested but no host is wired`。
  - `UiEvent::ReleaseNotesLoaded(r)` → `host.loaded(r)`。
  - 每幀在 `show_update` 之後呼叫 `host.show(ctx, &self.viewport_hwnds())`。
  - `viewport_hwnds` 加入它的 hwnd。
  - `sync_overlay_level` 的 `open` 條件加上它，log 文字改成 `(settings, update or release notes window open={open})`。
- main.rs：在 `schedule_update_checks` 把 `ui` move 走之前，不受 `cfg!(debug_assertions)`／`current_exe` 條件影響，呼叫 `app.set_release_notes(ReleaseNotesHost::new(rt.clone(), ui.clone(), RELEASES_API.to_owned()))`。

- [ ] **Step 1: 寫失敗測試**

```rust
// tests/it/settings_window.rs
#[test]
fn release_notes_link_opens_the_window() {
    let mut h = harness(configured_cfg());
    go(&mut h, "settings.tab.about");
    assert!(click_link(&mut h, &tr("about.release_notes")).is_empty());
    assert_eq!(h.state().actions, [SettingsAction::OpenReleaseNotes]);
}

// src/ui/app.rs mod tests（比照 wire_update 寫 wire_release_notes，api 指向 http://127.0.0.1:1）
#[test] fn open_release_notes_action_opens_the_window()           // 送 OpenReleaseNotes → host.is_open()
#[test] fn release_notes_window_keeps_the_overlay_at_normal_level() // 開設定＋本版說明，關設定 → overlay_level() == WindowLevel::Normal，本版說明仍 is_open()
```

- [ ] **Step 2:** 跑上述測試 → 失敗。
- [ ] **Step 3:** 實作。
- [ ] **Step 4:** `cargo test -j 4 --test it settings_window -- --test-threads=4` 與 `cargo test -j 4 --lib app::tests -- --test-threads=4` → PASS。
- [ ] **Step 5:** 檢查三項（全套 `cargo test`）後 commit：`feat(about): link to the current release notes`。
- [ ] **Step 6:** 請使用者在 exe 或 `cargo run` 實機看一次（目前 `Cargo.toml` 的版本已打 tag，應看到該版內文；要看「找不到」畫面得暫時改版本號，看失敗畫面可斷網）。不要自行開 GUI。
