# 框選擷取移到背景執行緒 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 開框與調整框時 UI 執行緒不再做 WGC／GDI 擷取與影像轉換，改由背景執行緒擷取後以 `UiEvent::RegionCaptured` 送回。

**Architecture:** 比照 `RegionRunner`：`RegionFlow` 把擷取交給新的 `RegionCapturer` trait 物件，自己記住一個「等擷取中」的 job（遞增編號），結果回來時編號相符才用，不符就丟掉。正式版每個 job 開一條 `std::thread`。`RegionDesktop` 拿掉三個擷取方法。

**Tech Stack:** Rust 2024、egui/eframe 0.37（`ColorImage`）、windows-capture（WGC）、`std::thread`。

**Spec:** `docs/superpowers/specs/2026-10-20-region-background-capture-design.md`

## Global Constraints

- 在原目錄從 `refactor/rust-rewrite` 開分支 `perf/region-background-capture`，不建 worktree。
- log 一律英文，前綴用 `[region]`，經 `crate::logs::log`，不可用 `println!`／`eprintln!`。
- 註解規則：
  - 實作註解預設一行、只寫 WHY；
  - `///` 一行摘要；
  - 中文用全形標點。
- 測試不可啟動真的 WGC、截桌面或搶前景，一律用替身。
- 整合測試放 `tests/it/`。
- commit message 用英文 conventional commits。commit 前要通過：
  - `cargo fmt --check`；
  - `cargo clippy --all-targets -- -D warnings`；
  - `cargo test` 全綠。
- 不 push。
- 既有 log 原字串保留，不可改字：
  - `[region] frame captured (…)`
  - `[region] selection started (game_hwnd=…)`
  - `[region] selector placed: …`
  - `[region] box adjusted, recapturing (…)`
  - `[region] frame capture failed (hwnd=…): …`
- 新增的 log 字串（逐字照用）：
  - `[region] capture started (id={id}, hwnd={hwnd:#x}, purpose=select|recapture)`
  - `[region] stale capture dropped (id={id}, current={current})`（`current` 是目前等待中的編號，沒有在等就寫 `none`）
  - `[region] selection cancelled while capturing`
  - `[region] capture thread panicked (id={id}): {message}`

## Review Focus

1. **擷取中連按兩下熱鍵：** 不應開出選取畫面，`selecting` 回到 false，遊戲前景不動。Task 2 有測試 `a_second_toggle_while_capturing_cancels_without_touching_the_foreground`。
2. **調整框後、重拍回來前關掉卡片：** 結果回來不應重開卡片，也不送 runner。Task 2 有測試 `closing_the_card_while_recapturing_drops_the_late_capture`。
3. **快速調整框兩次：** 只有第二次的結果送出，而且用第二次的矩形。Task 2 有測試 `only_the_latest_recapture_is_sent`。
4. **重拍途中按熱鍵開新框：** 重拍結果被丟掉，選取畫面用開框那次的結果。Task 2 有測試 `a_new_selection_drops_a_pending_recapture`。
5. **背景執行緒 panic：** 流程不能卡在「等擷取中」，否則熱鍵永遠只會送取消。panic 要轉成 `CaptureError::Failed` 送回。Task 2 有測試 `a_panicking_capture_becomes_a_failure`。

---

### Task 1: `Selector` 改吃轉好的 `ColorImage`

**Files:**
- Modify: `src/ui/region/selector.rs`（`Selector::new`、`color_image`、測試裡的建構）
- Modify: `src/ui/region/flow.rs:129`（唯一的正式呼叫端）

**Interfaces:**
- Produces:
  - `pub fn images(frame: &Frame, backdrop: &RgbImage) -> [ColorImage; 2]`，位置在 `src/ui/region/selector.rs`，回傳順序是 `[backdrop, frame.image]`；
  - `pub fn new(monitor: Rect, frame: Arc<Frame>, images: [ColorImage; 2]) -> Selector`。

- [ ] **Step 1: 開分支**

```bash
git switch -c perf/region-background-capture
```

- [ ] **Step 2: 改簽名與呼叫端**

`Selector::new` 不再自己轉圖，直接收 `images`。原本 `new` 裡兩次 `color_image` 的轉換搬進新的 `images()`。`flow.rs:129` 暫時改成 `Selector::new(monitor, Arc::clone(&frame), selector::images(&frame, &backdrop))`，selector 測試的建構方式也一起改。這是純機械重構，不新增測試；既有的 `backdrop_size` 相關測試必須照樣通過。

- [ ] **Step 3: 驗證**

Run: `cargo test --lib region::`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/ui/region/selector.rs src/ui/region/flow.rs
git commit -m "refactor(region): build the selector from pre-converted images"
```

---

### Task 2: 背景擷取 —— `RegionCapturer`、流程狀態、事件與接線

這是一個 task：trait 一改，所有實作者都要同步改才編得過。

**Files:**
- Modify: `src/ui/region/flow.rs`：
  - 新增型別與 trait；
  - `RegionDesktop` 刪除 `capture_window`／`capture_screen`／`monitor_at`；
  - 加入流程狀態並改寫測試。
- Modify: `src/ui/region/fakes.rs`：新增 `FakeCapturer`，`frames`／`fallback`／`backdrop` 從 `FakeDesktop` 搬過去；新增 `deliver`／`delivery`。
- Modify: `src/ui/events.rs`：新增 `UiEvent::RegionCaptured`。
- Modify: `src/ui/region_host.rs`：新增 `captured`，並調整測試。
- Modify: `src/ui/app.rs`：分派新事件，並調整框選測試。
- Modify: `src/app_main.rs`：
  - 新增 `region_capturer`、`ThreadCapturer`、`shoot`、`guarded`；
  - `WinRegionDesktop` 刪除三個擷取方法。
- Modify: `src/main.rs:215-226`：把 capturer 接上 `RegionFlow::new`。
- Modify: `src/input/dispatch.rs`：`on_region_hotkey` 的文件註解改成「選取層開著或正在為它擷取就取消」。
- Modify: `tests/it/region_flow.rs`、`tests/it/ui_probe.rs`：配合新 trait，並補上新 log 的測試。

**Interfaces:**
- Consumes: Task 1 的 `selector::images`、`Selector::new(monitor, frame, images)`。
- Produces（都放在 `src/ui/region/flow.rs`，`pub`）：

```rust
/// 背景擷取的用途。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureKind { Select, Recapture }

/// 背景擷取的成果；開框時另帶所在螢幕與轉好的 [backdrop, 遊戲畫面]。
#[derive(Clone)]
pub struct Shot {
    pub frame: Arc<Frame>,
    pub selector: Option<(Rect, [ColorImage; 2])>,
}
// 手寫 PartialEq：frame 用 Arc::ptr_eq 比，selector 只比 monitor；手寫 Debug：印 client 大小與 monitor。
// 理由：UiEvent 要 derive Debug/Clone/PartialEq，但不該逐像素比較或印出整張圖。

/// 在背景擷取遊戲畫面：完成後投遞 `UiEvent::RegionCaptured { id, .. }`。
pub trait RegionCapturer {
    fn capture(&self, id: u64, hwnd: isize, kind: CaptureKind);
}

impl RegionFlow {
    pub fn new(desktop: Box<dyn RegionDesktop>, capturer: Box<dyn RegionCapturer>,
               runner: Box<dyn RegionRunner>, selecting: Arc<AtomicBool>) -> Self;
    /// 擷取結果（`UiEvent::RegionCaptured`）：編號是目前在等的才用，其餘丟掉。
    pub fn on_captured(&mut self, id: u64, result: Result<Shot, CaptureError>);
}
```

- `src/ui/events.rs`：`UiEvent::RegionCaptured { id: u64, result: Result<Shot, CaptureError> }`。
- `src/ui/region_host.rs`：`pub fn captured(&mut self, id: u64, result: Result<Shot, CaptureError>)`，內容是 `flow.on_captured` 再 `sync_closed`，跟 `toggle` 一樣。
- `src/app_main.rs`：
  - `pub fn region_capturer(ui: UiPoster) -> Box<dyn RegionCapturer>`；
  - `fn shoot(hwnd: isize, kind: CaptureKind, grab: impl FnOnce(isize) -> Result<Frame, CaptureError>, monitor_at: impl FnOnce(i32, i32) -> Rect, screen: impl FnOnce(Rect) -> RgbImage) -> Result<Shot, CaptureError>`；
  - `fn guarded(id: u64, f: impl FnOnce() -> Result<Shot, CaptureError>) -> Result<Shot, CaptureError>`。
- `src/ui/region/fakes.rs`（`pub(crate)`）：
  - `FakeCapturer { calls, frames, fallback, backdrop }`：
    - `capture` 把 `hwnd` 推進 `calls.captured`；
    - 用 `frames` 佇列（用完回 `fallback`）當場算好結果，`Select` 時附上 `(MONITOR, images(frame, RgbImage::new(backdrop)))`；
    - 結果存進 `calls.pending: VecDeque<(u64, Result<Shot, CaptureError>)>`。
  - `capturer() -> FakeCapturer`：預設值。
  - `delivery(calls: &Shared) -> UiEvent`：取出最早的 pending，包成 `RegionCaptured`。
  - `deliver(flow: &mut RegionFlow, calls: &Shared)`：把 `delivery` 拆開後呼叫 `flow.on_captured`。
  - `flow()` 改成同時接上 `FakeCapturer`。
  - `Calls.monitor_at`／`Calls.screens` 刪除，這兩項改由 `shoot` 的測試涵蓋。

**流程狀態**（`RegionFlow` 的私有欄位，`capture_id: u64` 加上 `pending: Option<(u64, Pending)>`）：

```rust
enum Pending { Select { hwnd: isize }, Recapture { rect: Rect } }
```

各事件的處理：

- **`toggle(hwnd)`**
  - 選取畫面開著，或正在等開框擷取：
    - 選取畫面開著 → 照舊取消，並還前景；
    - 等擷取中 → 清掉 `pending`、`selecting = false`，記 log `selection cancelled while capturing`，不還前景。
  - `hwnd == 0` → 照舊忽略。
  - 其他情況：
    - 收掉框與卡片、撤銷請求、清掉 `pending`；
    - 發出 `Select` job，記 log `capture started`；
    - `selecting = true`。
- **`start_from_button`**：條件改成「選取畫面開著或正在等開框擷取」時呼叫 `toggle(0)`。
- **`on_captured(id, result)`**：先檢查編號。
  - 編號 ≠ 目前在等的 → 記 log `stale capture dropped` 後 return。
  - `Select` 成功 → 建選取畫面，記 log `selection started`。
  - `Select` 失敗 → 記 log `frame capture failed`，在游標處顯示錯誤卡片，`selecting = false`。
  - `Recapture` 成功 → 呼叫 `send(rect, hwnd, &frame)`。
  - `Recapture` 失敗 → 記 log `frame capture failed`，呼叫 `show_error(rect, …)`，框保留。
- **`on_box_changed(rect)`**：
  - 先記 log `box adjusted, recapturing`；
  - 再呼叫 `pending_card(rect)`；
  - 最後發出 `Recapture` job（新編號會讓舊的作廢）。
- **`on_card(Close)`**：除了現有動作，再清掉 `pending`。
- **把現有的 `translate` 拆成兩段：**
  - `pending_card(rect)`：撤銷請求、session +1，再 `Card::new`；
  - `send(rect, hwnd, frame)`：
    - 裁切失敗 → `show_error`，錯誤分類照現有寫法；
    - 裁切成功 → 建新 token，再 `runner.run(png, rect, self.session, token)`。
  - 開框後選好時，`translate` 就是 `pending_card` 加 `send`；重拍則是放開時呼叫 `pending_card`，結果回來時呼叫 `send`。
- **`selecting` 的不變式：** `selecting == selector.is_some() || pending 是 Select`。
  - flow 測試的 `H::selecting()` 要改成檢查這一條；
  - 模組文件與 `new` 的 `///` 也要一起更新。

- [ ] **Step 1: 改 fakes 與 flow 測試（先寫失敗的測試）**

`H::new` 改成接收 `(FakeDesktop, FakeCapturer)`，並新增 `H::toggle(hwnd)` 輔助：先 `flow.toggle`，再 `deliver`。既有的測試把「開框後馬上要有選取畫面」的地方改用 `h.toggle`；`frames:` 改設在 `FakeCapturer` 上。

`the_selector_covers_the_monitor_under_the_middle_of_the_game_client_area` 從 flow 移到 Step 5 的 `shoot` 測試，flow 這邊刪除。

新增下列測試：

```rust
#[test]
fn toggle_waits_for_the_capture_before_opening_the_selector() {
    // flow.toggle(GAME) 後：selector 為 None、selecting == true、calls.captured == [GAME]；deliver 後 selector 開著
}
#[test]
fn a_second_toggle_while_capturing_cancels_without_touching_the_foreground() {
    // flow.toggle(GAME); flow.toggle(GAME); deliver → selector None、selecting false、calls.foreground 為空
}
#[test]
fn adjusting_the_box_shows_the_pending_card_before_the_capture_returns() {
    // h.toggle; pick(RECT); finish_last; on_box_changed(new) → card == (new, pending())、sessions == [1]（尚未送出）；deliver → sessions == [1, 2]
}
#[test]
fn only_the_latest_recapture_is_sent() {
    // on_box_changed(a); on_box_changed(b); 依序 deliver 兩次 → 第一次被丟、runs 只多一筆且矩形為 b
}
#[test]
fn closing_the_card_while_recapturing_drops_the_late_capture() {
    // on_box_changed(a); on_card(Close); deliver → card None、region_box None、runs 沒增加
}
#[test]
fn a_new_selection_drops_a_pending_recapture() {
    // on_box_changed(a); flow.toggle(GAME); deliver（重拍的，丟掉）→ selector 仍 None；deliver（開框的）→ selector 開著
}
```

`fakes::frame()` 改成每次回傳新的 `Arc`，或維持原樣，由實作者決定，前提是 `selection_crops_the_same_frame_that_was_captured` 照樣成立：裁切用的是選取畫面那張 frame 的指標。

- [ ] **Step 2: 跑測試確認失敗**

Run: `cargo test --lib region::flow`
Expected: 編譯失敗（`RegionCapturer`／`on_captured`／`FakeCapturer` 還不存在）。

- [ ] **Step 3: 實作 flow 的型別、trait、狀態，以及 `UiEvent::RegionCaptured`**

依上面的 Interfaces 與「流程狀態」實作。`log` 字串依 Global Constraints 逐字照用：`purpose` 寫 `select` 或 `recapture`，`stale capture dropped` 沒有在等時寫 `current=none`。

- [ ] **Step 4: 跑 flow 測試**

Run: `cargo test --lib region::flow`
Expected: PASS

- [ ] **Step 5: 寫正式版 capturer 的測試（`src/app_main.rs` 的 tests 模組）**

```rust
#[test]
fn shoot_for_a_selection_uses_the_monitor_under_the_client_centre() {
    // grab 回 client_origin (100, 200)、client_size (800, 600) 的 Frame（影像非單色無妨，不經 capture_window_with）
    // monitor_at 記下收到的點並回 (0, 0, 1920, 1080)；screen 回 RgbImage::new(3, 2)
    // 斷言：monitor_at 收到 (500, 500)；shot.selector 的 monitor 為 (0, 0, 1920, 1080)、images[0].size == [3, 2]
}
#[test]
fn shoot_for_a_recapture_skips_the_screen_and_monitor() {
    // monitor_at／screen 被呼叫就 panic；結果 selector 為 None
}
#[test]
fn shoot_passes_a_grab_failure_through() {
    // grab 回 Err(CaptureError::Failed("x")) → shoot 回同一個 Err，screen 不被呼叫
}
#[test]
fn a_panicking_capture_becomes_a_failure() {
    // guarded(7, || panic!("boom")) → Err(CaptureError::Failed(m))，m 含 "boom"；capture_logs 後 logged("[region] capture thread panicked (id=7): boom")
}
```

- [ ] **Step 6: 實作 `shoot`／`guarded`／`ThreadCapturer`／`region_capturer`，並接線**

- `ThreadCapturer { ui: UiPoster }` 的 `capture` 做的事：
  - 用 `std::thread::Builder::new().name("region-capture".into()).spawn(...)` 開執行緒；
  - 執行緒內呼叫 `guarded(id, || shoot(hwnd, kind, capture::capture_window, monitor_rect_at, capture::capture_screen))`；
  - 結果包成 `UiEvent::RegionCaptured` 投遞。
- spawn 失敗時，記 log `[region] cannot spawn capture thread: {e}`，並直接投遞 `Err(CaptureError::Failed(…))`，避免流程卡在等待中。
- `guarded` 用 `catch_unwind(AssertUnwindSafe(f))`，訊息沿用現有的 `panic_message`。
- `WinRegionDesktop` 刪掉三個擷取方法。
- `main.rs` 改成 `RegionFlow::new(Box::new(WinRegionDesktop), region_capturer(ui.clone()), region_runner(..), ..)`。
- 在 `region_host.rs` 新增 `captured`；在 `app.rs` 的 `UiEvent::RegionDone` 旁加上 `UiEvent::RegionCaptured { id, result } => host.captured(id, result)`。
- `dispatch.rs` 只改文件註解。

- [ ] **Step 7: 調整 host、app、整合測試**

**`src/ui/region_host.rs` 的測試：** 凡是 `host.toggle(GAME)` 後就期待選取畫面的地方，都要接著呼叫 `deliver(&mut host.flow, &calls)`。`harness()` 改用 `fakes::flow()` 回傳的 `calls`。

**`src/ui/app.rs` 的框選測試：** 在 `RegionHotkey`（或 ⛶）的 `step` 之後，加上 `poster.post(fakes::delivery(&calls)); harness.step();`，順便證明新事件確實轉發到 host。`region_hotkey_toggles_the_flow` 的取消要在 deliver 之後才做，這樣才還前景。

**`tests/it/region_flow.rs`：**
- 原本的 `Desktop` 刪掉三個擷取方法；
- 新增 `Capturer { fail: bool, done: Rc<RefCell<VecDeque<(u64, Result<Shot, CaptureError>)>>> }`，用 `frame()` 與 `selector::images` 組出結果；
- `select()` 改成 toggle → 取出結果 → `on_captured` → `on_selector`；
- 新增下列測試：

```rust
#[test]
fn capture_start_stale_drop_and_cancel_while_capturing_are_logged() {
    // toggle(0x5555) → logged("[region] capture started (id=1, hwnd=0x5555, purpose=select)")
    // toggle(0x5555) → logged("[region] selection cancelled while capturing")
    // on_captured(1, …) → logged("[region] stale capture dropped (id=1, current=none)")
}
```

`frame_capture_failure_is_logged` 改成在 `on_captured` 之後才斷言原本的字串。

**`tests/it/ui_probe.rs`：** `RegionFake` 只保留桌面的部分。新增 `RegionShots(UiPoster)` 實作 `RegionCapturer`，當場投遞成功的 `RegionCaptured`，`Select` 時附上 `REGION_CLIENT` 的 frame 與 1×1 的 backdrop。這是 `#[ignore]` 測試，只要求能編譯，`clippy --all-targets` 會檢查到。

- [ ] **Step 8: 完整驗證**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
Expected: 全綠，零警告。

- [ ] **Step 9: Commit**

```bash
git add -A src tests
git commit -m "perf(region): capture the game frame on a background thread"
```

- [ ] **Step 10: 打包給使用者實測**

```bash
cargo build --release && cp target/release/Wizard101ChatTranslator.exe _exe_test/
```

依 spec 的「實機驗收」五項，交給使用者驗收。
