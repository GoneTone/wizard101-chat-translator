# Rust 重寫第 3 段（框選翻譯：WGC 擷取、選取畫面、框與結果卡、OCR）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 Rust 版補齊框選翻譯，從觸發到出結果的流程與 Python v0.6.0 一致：
- 用快捷鍵或 ⛶ 觸發，凍結一張遊戲畫面；
- 在全螢幕選取畫面上框選；
- 框選後，框留在畫面上可調整；
- 本機 OCR 辨識後交給翻譯，結果顯示在卡片上。

OCR 偵測正規化改用官方值，這是 spec 唯一刻意的行為改變。

**Architecture:**
- **非 UI 部分**放在 `region/`，各自可單獨測試：
  - 擷取：WGC 抓遊戲畫面、GDI 抓整顆螢幕；
  - OCR：rusto-rs；
  - 辨識加翻譯的流程。
- **介面部分**放在 `ui/region/`：選取畫面、框、結果卡三個元件只負責狀態與 egui 內容，由 `RegionFlow` 串成流程。
- **視窗**：`App` 用 egui immediate viewport 托管這三個視窗，做法同第 2 段的輸入框。
- **耗時的工作**：辨識與翻譯在 tokio 上執行，OCR 走 `spawn_blocking`，結果經 `UiEvent` 回到 UI 執行緒。

**Tech Stack:** 沿用前兩段。
- 新增 crate：
  - `rusto-rs = "0.3.1"`：OCR，內含純 Rust 的 rten 推論；
  - `image`：PNG 編解碼與裁切，版本跟 rusto-rs 用的 0.25 一致，避免重複編譯。
- `windows-capture 2.0.1` 從 dev-dependencies 移到 dependencies。
- windows-sys 視需要新增 `Win32_Graphics_Dwm`（DWM 外框座標）；GDI 截圖用既有的 `Win32_Graphics_Gdi`。

**Spec:** `docs/superpowers/specs/2026-10-17-rust-rewrite-design.md`，驗收依據是 §G。

- 「唯一刻意的行為改變」那一節規定 OCR 要用一批截圖驗證，由 Task 2 的對照工具支援。
- 「單一 exe 與資源」那一節規定模型編進 exe、第一次框選時才解出來。
- 每個 task 的行為以對應的 Python 原始碼為準：`src/region/*.py`、`src/ui/region/*.py`。
- 也要納入 `docs/superpowers/plans/2026-10-17-rust-rewrite-followups.md` 的「第 3 段」三項，以及記憶裡「前景切換會吃掉按鍵放開」的教訓。

## Global Constraints

- **提交前檢查依改動範圍**：
  - 只動 `rust/` 時，跑三項 cargo 檢查：`fmt --check`、`clippy --all-targets -- -D warnings`、`test`。
  - 有動到 `src/`、`tests/`、`tools/` 或 `pyproject.toml` 時，另外跑 `uv run ruff check src tests tools` 與 `uv run pytest`。
  - 只動文件時不必跑。
  - 不可用 `--no-verify`，不可 push。
- 第 1 段計畫的其他 Global Constraints 照樣沿用：log 規則、註解規則、i18n 不新增 key、移植測試、commit 格式。
- **log**：
  - 前綴是 `[region]`；文字盡量與 Python 同一行 log 相同。
  - 截圖內容與 OCR 原文不可寫進 log，只記長度與座標。這點與 Python 相同：Python 只記 `source_chars`。
- **測試不可干擾桌面**：
  - 不可呼叫真的 WGC、GDI 抓螢幕、`SetForegroundWindow`，也不可開可見視窗。這些一律經可注入的介面處理。
  - 唯一例外是標成 `#[ignore]` 的螢幕外 probe，做法沿用 `rust/tests/ui_probe.rs`。
- **熱鍵開出、需要搶前景的視窗**（選取畫面），要等觸發鍵放開才顯示並搶前景。
  - 判斷用 `input::hotkeys::swallowed_key_held()`，最多等 1 秒；邏輯沿用 `ui/input_host.rs` 的 `defer_for_release`。
  - 不可以先顯示、晚點才給焦點：這段空檔打的字會進遊戲。
- **新的 egui viewport**：
  - 一律無邊框，建好就呼叫 `remove_system_frame` 與 `hide_from_taskbar`；
  - HWND 用 `win32::pick_new_window`，比對建立前後的視窗清單，`known` 要列出所有已知的 viewport HWND；
  - 不搶焦點的視窗（框、結果卡）要呼叫 `make_non_activating`。
- **OCR 模型與字典**：
  - 放在 `rust/assets/ocr/` 進版控：`PP-OCRv6_det_small.onnx`、`PP-OCRv6_rec_small.onnx`、`ch_ppocr_mobile_v2.0_cls_mobile.onnx`、`ppocrv6_keys.txt`。
  - 前三個從 `.venv/Lib/site-packages/rapidocr/models/` 複製，與 Python 版用的是同一批檔案。
  - 字典從 `%LOCALAPPDATA%\Temp\w101-ocr-spike\models\ppocrv6_keys.txt` 複製。
- **模型只在 `rust/src/main.rs` 用 `include_bytes!` 編進 exe**，經 `region::ocr::register_models` 交給函式庫。
  - 函式庫與測試一律讀 `rust/assets/ocr/` 的檔案，不嵌入，否則每個測試執行檔都會多 31 MB。
- **使用者的遊戲截圖不進版控**；OCR 的實圖測試標成 `#[ignore]`，從環境變數指定的資料夾讀圖。

## Review Focus

1. **只框一行字的細長區域**（例如 420×18）：這是 RapidOCR 補垂直留白的路徑，不可 panic，結果要與 Python 同一套前處理一致。測試放在 Task 1：`thin_strip_goes_through_vertical_padding_without_panicking`。
2. **WGC 一直等不到影格**（遊戲最小化、被遮住）：2 秒內回擷取錯誤，UI 不可卡死超過這段時間。測試放在 Task 3：`grab_times_out_after_two_seconds`，用注入的 grabber。
3. **模型解不出來**（`%LOCALAPPDATA%` 唯讀或磁碟滿）：卡片顯示「本機文字辨識無法啟動」，程式繼續跑；下一次框選時重試解出。測試放在 Task 1：`extraction_failure_is_ocr_unavailable_and_retried`。
4. **框選很大的區域**（整顆 4K 螢幕）：OCR 不可在 UI 執行緒上跑，overlay 照常重繪。測試放在 Task 4：`recognize_runs_off_the_async_worker`。
5. **在遊戲裡按住 Ctrl+Shift+Space**：選取畫面要等放開才搶前景，下一次熱鍵仍要有效，與第 2 段的教訓相同。測試放在 Task 9：`selector_takes_foreground_only_after_the_trigger_is_released`。

---

### Task 1: 本機 OCR（rusto-rs、模型解出、merge_lines）

**Files:**
- Create: `rust/src/region/mod.rs`, `rust/src/region/ocr.rs`, `rust/assets/ocr/`（四個檔案，見 Global Constraints）
- Modify: `rust/src/lib.rs`、`rust/Cargo.toml`（`rusto-rs = "0.3.1"`、`image`）、`rust/src/logs/library.rs`（壓低 rusto-rs／rten 的 log）
- Modify: `tools/golden/export.py`（新增 `ocr_merge_lines`）
- Test: `rust/tests/ocr.rs`、`rust/tests/ocr_golden.rs`

**Interfaces:**
- Produces（`crate::region::ocr`）：
  - `pub struct Models { pub det: &'static [u8], pub rec: &'static [u8], pub cls: &'static [u8], pub keys: &'static [u8] }`
  - `pub fn register_models(models: Models)`：由 main 呼叫一次；沒註冊時（測試）改從 `rust/assets/ocr/` 讀檔。
  - `#[derive(Debug, Clone, PartialEq)] pub struct OcrUnavailable(pub String);`
  - `pub struct OcrBox { pub points: [(f32, f32); 4], pub text: String }`
  - `pub fn merge_lines(items: &[OcrBox]) -> Vec<String>`
  - `pub fn recognize(png: &[u8]) -> Result<String, OcrUnavailable>`：各行以 `\n` 連接，沒有文字回空字串。
  - `pub fn ensure_extracted(dir: &Path, models: &Models) -> io::Result<PathBuf>`：純檔案操作，可測。

- [ ] **Step 1:** 在 `tools/golden/export.py` 加 `ocr_merge_lines` exporter，input 是 `(box, text)` 清單，output 是 `src.region.ocr.merge_lines` 的結果。案例至少涵蓋：
  - 空清單；
  - 同一行被切成三個框（商店標題）；
  - 兩行；
  - y 差距剛好等於、與剛好小於中位數一半的邊界；
  - 行內的框順序打亂；
  - 傾斜的框。

  跑 `uv run python -m tools.golden.export ocr_merge_lines`。
- [ ] **Step 2:** 寫測試：
  - `rust/tests/ocr_golden.rs::merge_lines_matches_python`
  - `rust/tests/ocr.rs`：
    - `extracts_models_once_and_rewrites_on_hash_mismatch`：目錄名稱含四個檔案合起來的 sha1 前 12 碼；內容相同就不重寫；檔案被改壞會重寫。
    - `extraction_failure_is_ocr_unavailable_and_retried`（Review Focus 3）：目標放在唯讀路徑時回 `OcrUnavailable`；之後換成可寫路徑，再呼叫一次就成功，失敗的結果不可被快取。
    - `thin_strip_goes_through_vertical_padding_without_panicking`（Review Focus 1）：以 `rust/assets/ocr/` 的模型辨識一張 420×18 的全黑 PNG，回 `Ok("")`。
    - `#[ignore] samples_match_expected_text`：讀環境變數 `W101_OCR_SAMPLES` 指定的資料夾，每張 `*.png` 與同名 `*.txt` 的期望文字比對。
- [ ] **Step 3:** 跑測試，預期 FAIL。
- [ ] **Step 4:** 實作 `region/ocr.rs`。

  引擎設定照 spike（`%LOCALAPPDATA%\Temp\w101-ocr-spike\rusto\src\main.rs`）：
  - `InitializeConfig::ppv6(det, rec, keys)`
  - `with_det_box_thresh(0.5)`、`with_unclip_ratio(1.6)`、`with_use_dilation(true)`
  - `with_limit_side_len(736)`、`with_limit_type("min")`、`with_det_thresh(0.3)`
  - `with_cls_threshold(cls, 0.9)`、`with_text_score(0.5)`
  - `with_det_mean([0.485, 0.456, 0.406])`、`with_det_std([0.229, 0.224, 0.225])`

  Global 前處理照 RapidOCR 3.9.2 的 `config.yaml`，rusto-rs 預設不是這些值時要明確設定：
  - `min_side_len 30`、`max_side_len 2000`
  - `min_height 30`、`width_height_ratio 8`（垂直留白）

  引擎與模型目錄：
  - 引擎放在 `Mutex<Option<RustO>>`，第一次 `recognize` 才建立。
  - 模型目錄是 `local_state_dir()/ocr/<sha1 前 12 碼>/`。

  log 文字照 Python：
  - `[region] local OCR engine ready (rusto)`
  - `[region] local OCR done (boxes=N, lines=M, ms=T)`
- [ ] **Step 5:** 跑測試，預期 PASS；另跑一次 ignored 的 `samples_match_expected_text`，`W101_OCR_SAMPLES` 指向 spike 的 `images` 資料夾。期望文字由 Step 6 的工具產生，`2.png` 與 `4.png` 期望的是讀對的版本。
- [ ] **Step 6:** 跑提交前檢查後 commit：`feat(rust): add local OCR with rusto-rs`。

### Task 2: OCR 新舊參數對照工具

**Files:**
- Create: `tools/ocr_compare.py`、`rust/examples/ocr_compare.rs`
- Test: `tests/tools/test_ocr_compare.py`

**Interfaces:**
- `uv run python -m tools.ocr_compare baseline <dir>`：用 Python 版（現行參數）辨識 `<dir>/*.png`，寫出 `<dir>/python.json`，格式為 `{檔名: merged_text}`。
- `cargo run --release --manifest-path rust/Cargo.toml --example ocr_compare -- <dir>`：用 Task 1 的 `recognize` 辨識，寫出 `<dir>/rust.json`。
- `uv run python -m tools.ocr_compare report <dir>`：讀兩份 json，寫出 `<dir>/report.md`。逐張列出兩邊原文；不同的標 `⚠`，相同的標 `=`；檔尾統計張數。

- [ ] **Step 1:** 寫 `test_report_marks_differences_and_counts`：在 tmp 目錄放兩份 json，斷言 `report.md` 的內容與統計。
- [ ] **Step 2:** 跑 `uv run pytest tests/tools/test_ocr_compare.py -n 0`，預期 FAIL。
- [ ] **Step 3:** 實作兩個工具，`baseline` 直接呼叫 `src.region.ocr.recognize`。
- [ ] **Step 4:** 用 spike 的 4 張圖跑一輪，`2.png` 與 `4.png` 應該標 `⚠`，而且是 Rust 讀對的那一邊。把結果貼進 commit body，並為 Task 1 的 ignored 測試產生 4 個 `*.txt`，放在 spike 資料夾，不進版控。
- [ ] **Step 5:** 跑提交前檢查（這次有動 Python，五項全跑）後 commit：`feat(tools): add OCR old-vs-new comparison tool`。

### Task 3: 畫面擷取（WGC、螢幕截圖、裁切）

**Files:**
- Create: `rust/src/region/capture.rs`
- Modify: `rust/Cargo.toml`（`windows-capture` 移到 dependencies）、`tools/golden/export.py`（新增 `region_window_region`）
- Test: `rust/tests/capture.rs`、`rust/tests/capture_golden.rs`

**Interfaces:**
- Produces（`crate::region::capture`）：
  - `pub struct Frame { pub image: image::RgbImage, pub client_origin: (i32, i32), pub client_size: (i32, i32) }`
  - `#[derive(Debug, Clone, PartialEq)] pub enum CaptureError { OutsideGame(String), Failed(String) }`：實作 Display，文字照 Python 例外訊息。
  - `pub fn window_region(screen_rect: Rect, client_origin: (i32, i32), client_size: (i32, i32)) -> Option<Rect>`
  - `pub fn crop_frame(frame: &Frame, screen_rect: Rect) -> Result<Vec<u8>, CaptureError>`：回傳 PNG bytes。
  - `pub fn capture_window(hwnd: isize) -> Result<Frame, CaptureError>`：真的 WGC。
  - `pub fn capture_window_with(hwnd: isize, grab: impl FnOnce(isize, Duration) -> Result<image::RgbImage, CaptureError>, client: ClientGeometry) -> Result<Frame, CaptureError>`：測試注入用。
  - `pub struct ClientGeometry { pub client_origin: (i32, i32), pub client_size: (i32, i32), pub frame_origin: (i32, i32) }`
  - `pub fn capture_screen(monitor: Rect) -> image::RgbImage`：用 GDI BitBlt，失敗回全黑。
  - `pub const FRAME_TIMEOUT: Duration = Duration::from_secs(2);`

- [ ] **Step 1:** 寫測試。
  - golden：
    - `window_region_matches_python`：exporter 涵蓋完全在內、部分超出四邊、完全在外、client 原點為負，以及第二螢幕的座標。
  - 從 `tests/region/test_capture.py` 移植：
    - `crop_frame_cuts_the_selected_rect_out_of_the_frame`
    - `crop_frame_clips_to_the_client_area`
    - `crop_frame_outside_the_game_is_outside_game`
    - `capture_offsets_the_client_area_inside_the_dwm_frame`
    - `capture_logs_when_the_frame_does_not_contain_the_client`
    - `blank_capture_is_an_error`：整張單色時，錯誤文字含 `blank capture` 與顏色。
    - `capture_failures_are_wrapped_as_failed`
  - 新增：
    - `grab_times_out_after_two_seconds`（Review Focus 2）：注入的 grab 等滿 `FRAME_TIMEOUT` 才回 `Failed("no frame within 2.0s ...")`，測試以假時鐘或縮短的逾時驗證。
- [ ] **Step 2:** 跑測試，預期 FAIL。
- [ ] **Step 3:** 實作。
  - **WGC**：用 `windows-capture` 的 free-threaded session 抓一張影格；`cursor_capture` 與 `draw_border` 都關；BGRA 轉 RGB；超過 `FRAME_TIMEOUT` 就停止並回錯誤。
  - **座標**：DWM 外框原點用 `DwmGetWindowAttribute(DWMWA_EXTENDED_FRAME_BOUNDS)`；客戶區用 `reader::process::client_to_screen` 與 `client_size`。
  - **螢幕截圖**：`capture_screen` 用 `GetDC(0)`、`BitBlt`、`GetDIBits`。
  - **log**：照 Python 的 `frame captured`、`frame ... does not contain client`、`region cropped`、`screen grab failed` 四行。
- [ ] **Step 4:** 跑測試，預期 PASS。
- [ ] **Step 5:** 跑提交前檢查後 commit：`feat(rust): add region capture via WGC`。

### Task 4: 辨識加翻譯的流程，與錯誤文案

**Files:**
- Create: `rust/src/region/pipeline.rs`
- Modify: `rust/src/translation/errors.rs`（新增 `friendly_error`）、`tools/golden/export.py`（新增 `friendly_error`）
- Test: `rust/tests/region_pipeline.rs`、`rust/tests/errors_golden.rs`（新增案例）

**Interfaces:**
- Produces：
  - `pub fn friendly_error(e: &TranslateError) -> (&'static str, Vec<(&'static str, String)>)`：放在 `crate::translation::errors`，第 4 段的設定視窗也會用到。
  - `crate::region::pipeline`：
    - `#[derive(Debug, Clone, PartialEq)] pub struct RegionResult { pub text: String, pub source: String }`
    - `pub enum RegionError { Ocr(OcrUnavailable), Translate(TranslateError) }`
    - `pub struct RegionPipeline`
      - `new(translator: Arc<Translator>, recognize: Arc<dyn Fn(&[u8]) -> Result<String, OcrUnavailable> + Send + Sync>)`
      - `async fn run(&self, png: Vec<u8>, rect: Rect, cancel: &CancellationToken) -> Result<RegionResult, RegionError>`
      - `describe() -> String`
    - `pub fn describe_error(e: &RegionError) -> String`：已經 `t()` 過的一句話。

- [ ] **Step 1:** 寫測試。
  - golden：`friendly_error_matches_python`，涵蓋 config／offline 錯誤有無 detail、有無狀態碼、401／403／404、其他狀態碼。
  - 從 `tests/region/test_pipeline.py` 移植：
    - `run_returns_translation_and_source`
    - `no_text_skips_translation`：回 `RegionResult { text: "", source: "" }`，translator 沒被呼叫。
    - `cancel_is_passed_to_the_translator`
    - `errors_propagate`
  - 移植 `test_describe_error_maps_each_failure_kind`，四種對應如下：
    - OCR 失敗對應 `region.ocr_unavailable`；
    - BadOutput 對應 `region.failed`；
    - config／offline 錯誤走 `friendly_error`；
    - Unexpected 對應 `error.unexpected`。
  - 新增：
    - `recognize_runs_off_the_async_worker`（Review Focus 4）：注入的 recognize 記下執行緒名稱，斷言它不是呼叫 `run` 的那條 tokio worker，也就是走了 `spawn_blocking`。
- [ ] **Step 2:** 跑測試，預期 FAIL。
- [ ] **Step 3:** 照 `pipeline.py` 與 `form.py::friendly_error` 實作。
  - 翻譯呼叫 `Translator::translate_region_text(text, Some(cancel))`。
  - log 照 Python：
    - `[region] local OCR found no text (rect=..)`
    - `[region] done in X.Xs (rect=.., source_chars=N, chars=M)`
- [ ] **Step 4:** 跑測試，預期 PASS。
- [ ] **Step 5:** 跑提交前檢查後 commit：`feat(rust): add region OCR-then-translate pipeline`。

### Task 5: 選取畫面（狀態與 egui 內容）

**Files:**
- Create: `rust/src/ui/region/mod.rs`, `rust/src/ui/region/selector.rs`
- Modify: `rust/src/ui/mod.rs`
- Test: 同檔 `#[cfg(test)]`（純函式與 kittest）

**Interfaces:**
- Produces（`crate::ui::region::selector`）：
  - `pub const DIM_FACTOR: f32 = 0.45;`、`pub const BAND_WIDTH: f32 = 2.0;`、`pub const HINT_Y: f32 = 40.0;`
  - `pub enum SelectorAction { Selected(Rect), Cancelled }`：`Rect` 是螢幕實體像素。
  - `pub struct Selector`
    - `new(monitor: Rect, frame: Arc<Frame>, backdrop: image::RgbImage)`
    - `content(&mut self, ui: &mut egui::Ui) -> Option<SelectorAction>`
  - `pub fn spotlight_rect(drag: Rect, frame_pos: (i32, i32), frame_size: (i32, i32)) -> Option<Rect>`：拖曳矩形與遊戲畫面範圍的交集，用 monitor 內的座標。
  - `pub fn dim(image: &image::RgbImage, factor: f32) -> image::RgbImage`：與 PIL `ImageEnhance.Brightness` 相同，每個通道乘上係數。

- [ ] **Step 1:** 寫測試，移植 `tests/ui/region/test_select.py`：
  - `drag_reports_the_screen_rect`：monitor 原點加上拖曳的左上角，寬高取絕對值；反向拖曳也正確。
  - `a_click_without_drag_cancels`：使用 `geometry::is_click`。
  - `escape_cancels`、`right_click_cancels`
  - `spotlight_is_clipped_to_the_game_frame`：拖出遊戲範圍的部分不聚光；完全在外時回 `None`。
  - `size_label_shows_w_x_h_while_dragging`：kittest 查得到 `120×40`。
  - `dim_scales_each_channel`：`(200, 100, 50)` 乘 0.45 得 `(90, 45, 22)`，取整與 PIL 相同。
  - `hint_text_is_shown`：`region.hint`。
- [ ] **Step 2:** 跑 `cargo test --manifest-path rust/Cargo.toml --lib ui::region`，預期 FAIL。
- [ ] **Step 3:** 實作。
  - 背景依序疊三層：
    - 暗化的 backdrop；
    - 暗化的遊戲畫面，放在 `frame.client_origin - monitor` 的位置；
    - 拖曳中的聚光區，是原亮度的裁切圖。
  - 三張圖在 `new` 時轉成 egui texture，只轉一次；聚光區用同一張原亮度 texture 搭配 UV 子矩形，不必每幀裁圖。
  - 框線與尺寸文字的顏色用 `FG_UPDATE`。
  - **裁定：提示文字直接畫在選取畫面頂端置中**，底色 `BAR`、1px 邊框 `GRIP`，不另開視窗。spec 寫的「另一個不搶焦點的小窗」是 Tk 的做法，使用者看到的結果相同。
- [ ] **Step 4:** 跑測試，預期 PASS。
- [ ] **Step 5:** 跑提交前檢查後 commit：`feat(rust): add the region selector`。

### Task 6: 框選框（拖曳移動與縮放）

**Files:**
- Create: `rust/src/ui/region/region_box.rs`
- Modify: `rust/src/ui/geometry.rs`（`edge_at` 改為可指定判定帶寬度）
- Test: 同檔 `#[cfg(test)]`、`rust/tests/geometry_golden.rs`（不得退步）

**Interfaces:**
- Produces：
  - `crate::ui::geometry::edge_at_bands(px, py, x, y, w, h, edge: i32, corner: i32) -> Option<Edge>`：原本的 `edge_at` 改成呼叫它，傳入現有的常數。
  - `crate::ui::region::region_box`：
    - 常數：`MARGIN = 6`、`HANDLE = 10`、`MIN_SIZE = 20`、`INNER_BAND = 4`、`LINE = 3.0`
    - `pub enum Hit { Move, Edge(Edge) }`
    - `pub fn hit_at(px: i32, py: i32, w: i32, h: i32) -> Option<Hit>`：視窗座標；判定帶 `edge = MARGIN + INNER_BAND`、`corner = MARGIN + INNER_BAND + HANDLE / 2`。
    - `pub fn handles(w: i32, h: i32) -> [(Edge, (i32, i32)); 8]`
    - `pub struct RegionBox`
      - `new(rect: Rect)`、`rect() -> Rect`
      - `press(screen: (i32, i32), window_point: (i32, i32))`
      - `drag(screen: (i32, i32)) -> Option<Rect>`：回傳新 rect，供宿主搬移視窗。
      - `release(screen: (i32, i32)) -> Option<Rect>`：有改變才回 `Some`，點擊則還原。
      - `paint(ui: &mut egui::Ui)`

- [ ] **Step 1:** 寫測試，移植 `tests/ui/region/test_box.py`：
  - `hit_at_reports_edges_corners_and_move`
  - `dragging_the_inside_moves_the_rect`
  - `dragging_an_edge_resizes_only_that_edge`
  - `resizing_stops_at_the_minimum_size`
  - `a_click_restores_the_rect_and_reports_nothing`
  - `release_without_change_reports_nothing`
  - `handles_sit_on_the_corners_and_edge_midpoints`

  另確認既有的 `geometry_golden` 照樣全過。
- [ ] **Step 2:** 跑測試，預期 FAIL。
- [ ] **Step 3:** 照 `box.py` 實作，位移一律用螢幕座標計算，`moved_to`、`resized_edge`、`is_click` 沿用 `ui::geometry`。
  - **裁定：只用一個 viewport。** DirectComposition 視窗以視窗矩形做 hit-test，不看像素透明度，所以框內透明的部分也收得到滑鼠。Python 要「框線層＋滑鼠層」兩層，是 Tk 色鍵的限制。
  - 畫面上只有框線與 8 個把手，顏色是 `FG_UPDATE`，其餘部分透明。
- [ ] **Step 4:** 跑測試，預期 PASS。
- [ ] **Step 5:** 跑提交前檢查後 commit：`feat(rust): add the adjustable region box`。

### Task 7: 結果卡

**Files:**
- Create: `rust/src/ui/region/card.rs`, `rust/src/ui/copy_menu.rs`
- Modify: `rust/src/ui/overlay/selection.rs`（右鍵選單改用 `copy_menu`）
- Test: 同檔 `#[cfg(test)]`

**Interfaces:**
- Produces：
  - `crate::ui::copy_menu::show(ui: &egui::Ui, id: egui::Id, at: egui::Pos2) -> MenuOutcome`，其中 `pub enum MenuOutcome { Copy, Dismissed, Open }`：從 overlay 的 `show_menu` 抽出來共用，行為不變，包括夾在 viewport 內、點選單外就關。
  - `crate::ui::region::card`：
    - 常數：`MIN_WIDTH = 240`、`SIDE_MIN_WIDTH = 360`、`MIN_HEIGHT = 120`、`ANCHOR_GAP = MARGIN + HANDLE / 2 + 2`、`PAD_X = 10`、`PAD_Y = 6`
    - `pub enum CardContent { Pending, Text { text: String, source: String }, Error(String) }`
    - `pub enum CardAction { Close, Copied(usize), FocusForCopy }`
    - `pub struct Card`
      - `new(rect: Rect, alpha: f32)`
      - `set_content(CardContent)`
      - `set_alpha(f32)`
      - `content(&mut self, ui) -> Option<CardAction>`
      - `all_text() -> String`
    - `pub fn place(rect: Rect, width: i32, natural_h: i32, natural_w: i32, area: Rect, sticky: Option<Side>) -> Placement`：包住 `anchored_geometry`／`beside_geometry`；`sticky` 是側邊時沿用 `beside_geometry`。

- [ ] **Step 1:** 寫測試，移植 `tests/ui/region/test_card.py` 裡與 Tk 無關的部分。
  - 三種狀態的文字與顏色：
    - pending 顯示 `region.working`，顏色 `FG_PENDING`；
    - 空譯文顯示 `region.no_text`；
    - 有原文時顯示「原文 9pt 調暗＋空一行＋譯文」；
    - 錯誤顯示 `FG_ERROR`。
  - 位置：
    - `card_sits_below_the_rect_with_its_width`
    - `card_flips_above_when_no_room_below`
    - `card_moves_to_the_wider_side_when_neither_fits`
    - `side_placement_sticks_until_the_content_changes`
  - 互動：
    - `a_click_closes_and_a_drag_does_not`
    - `close_button_closes`
    - `right_click_menu_copies_selection_or_everything`：全部的格式是「原文＋`\n\n`＋譯文」。
    - `drag_selection_asks_to_borrow_focus`
    - `ctrl_c_copies_the_selection_through_our_clipboard`：攔下 egui 的 `OutputCommand::CopyText`，改走 `ui::clipboard::set_text`。eframe 沒開 clipboard feature，不攔下就複製不到。
  - 移植後 overlay 的右鍵選單測試照樣全過。
- [ ] **Step 2:** 跑測試，預期 FAIL。
- [ ] **Step 3:** 照 `card.py` 實作。
  - 文字用 egui 內建的 label 選取，原文與譯文可以一路跨選。
  - 有選取時，「複製」只複製選取範圍，否則複製全部；這需要讀取 egui 的選取狀態。如果 egui 0.37 沒有公開的方式讀取，就回報 controller，不可自行改成別的行為。
  - 背景透明度用 overlay 的 alpha。
  - 內容放不下時用細捲軸，沿用 `overlay::scrollbar`。
- [ ] **Step 4:** 跑測試，預期 PASS。
- [ ] **Step 5:** 跑提交前檢查後 commit：`feat(rust): add the region result card`。

### Task 8: 框選流程 RegionFlow

**Files:**
- Create: `rust/src/ui/region/flow.rs`
- Modify: `rust/src/ui/events.rs`（新增 `RegionDone`）
- Test: 同檔 `#[cfg(test)]`、`rust/tests/region_flow.rs`（需要擷取 log 的測試）

**Interfaces:**
- Consumes：Task 3 的擷取、Task 4 的 pipeline 與 `describe_error`、Task 5～7 的三個元件。
- Produces：
  - `UiEvent::RegionDone { session: u64, result: Result<RegionResult, String> }`：`Err` 是 `describe_error` 的字串。
  - `pub trait RegionDesktop`：Python `RegionFlow` 建構子裡所有可注入的項目都在這裡。
    - `capture_window(&mut self, hwnd: isize) -> Result<Arc<Frame>, CaptureError>`
    - `capture_screen(&mut self, monitor: Rect) -> RgbImage`
    - `monitor_at(&self, x: i32, y: i32) -> Rect`
    - `foreground(&mut self, hwnd: isize)`
    - `find_game(&self) -> Option<isize>`
    - `cursor(&self) -> (i32, i32)`
  - `pub trait RegionRunner { fn run(&self, png: Vec<u8>, rect: Rect, session: u64, cancel: CancellationToken); }`：正式版 spawn 到 tokio，結果投遞 `RegionDone`。
  - `pub struct RegionFlow`
    - `new(desktop: Box<dyn RegionDesktop>, runner: Box<dyn RegionRunner>, selecting: Arc<AtomicBool>)`
    - `toggle(hwnd: isize)`、`start_from_button()`、`set_alpha(f32)`、`finish(session, result)`
    - 給宿主讀寫的狀態：`selector_mut() -> Option<&mut Selector>`、`region_box_mut()`、`card_mut()`
    - 給宿主回報的事件：`on_selector(SelectorAction)`、`on_box_changed(Rect)`、`on_card(CardAction)`

- [ ] **Step 1:** 寫測試，逐一移植 `tests/ui/region/test_flow.py` 的 35 個測試，名稱改成 snake_case、去掉 `test_` 前綴，例如：
  - `toggle_opens_the_selector_and_a_second_toggle_cancels_it`
  - `selection_crops_the_same_frame_that_was_captured`
  - `the_selector_covers_the_monitor_under_the_middle_of_the_game_client_area`
  - `stale_result_is_dropped_after_a_new_selection`
  - `adjusting_the_box_recaptures_the_live_game_and_retranslates`
  - `closing_the_card_cancels_the_request`

  `test_the_failure_log_names_the_service` 改由 Task 10 的 `region_runner_failure_logs_the_service` 涵蓋，因為失敗 log 由 runner 記。`selecting` 旗標要與選取畫面是否開著一致，因為第 2 段的 `on_region_hotkey` 會讀它。
- [ ] **Step 2:** 跑測試，預期 FAIL。
- [ ] **Step 3:** 照 `flow.py` 實作，流程順序與 log 文字照抄。新的 session 會 cancel 舊的 token；過期結果記 `stale result dropped`。
- [ ] **Step 4:** 跑測試，預期 PASS。
- [ ] **Step 5:** 跑提交前檢查後 commit：`feat(rust): add the region translation flow`。

### Task 9: 在 App 裡托管三個視窗

**Files:**
- Create: `rust/src/ui/region_host.rs`
- Modify: `rust/src/ui/app.rs`、`rust/src/ui/input_host.rs`（`defer_for_release` 改成 `pub(crate)` 共用）、`rust/tests/ui_probe.rs`
- Test: `rust/src/ui/app.rs` 的 `#[cfg(test)]`、`region_host.rs` 的 `#[cfg(test)]`

**Interfaces:**
- Consumes：Task 8 的 `RegionFlow`。
- Produces：
  - `App::with_region(self, flow: RegionFlow) -> Self`
  - `App` 處理三種輸入：
    - `UiEvent::RegionHotkey { hwnd }` 呼叫 `flow.toggle(hwnd)`；
    - `RegionDone` 呼叫 `flow.finish`；
    - ⛶ 呼叫 `flow.start_from_button()`。
  - 拿掉第 2 段的佔位 log。

- [ ] **Step 1:** 寫測試：
  - `region_hotkey_toggles_the_flow`
  - `region_button_starts_from_the_button`
  - `region_done_is_forwarded_with_its_session`
  - `selector_takes_foreground_only_after_the_trigger_is_released`（Review Focus 5）：`trigger_held` 注入為 true 時，選取畫面還沒顯示，也沒搶前景；改成 false 後的下一幀才顯示並搶前景。用的是共用的 `defer_for_release`。
  - `known_windows_are_excluded_when_finding_a_new_viewport`：泡泡、輸入框、選取畫面、框、卡片互不認錯。
  - `ui_probe` 新增 ignored 檢查：
    - 框和卡片顯示後前景不變；
    - 四邊沒有 Win11 灰邊；
    - 沒有 `WS_EX_APPWINDOW`；
    - 卡片的背景透明度等於 overlay 的 alpha。
- [ ] **Step 2:** 跑測試，預期 FAIL。
- [ ] **Step 3:** 實作，做法對照 `input_host.rs`。
  - **選取畫面**：
    - 在 monitor 上全螢幕顯示、置頂、無工作列按鈕；
    - 建立那一幀先藏著；
    - 等觸發鍵放開後才顯示並 `force_foreground`；
    - 關閉後把前景交還遊戲。
  - **框**：
    - 視窗是 rect 往外各擴 `MARGIN`；
    - 不搶焦點；
    - 拖曳中以 `set_window_rect` 跟著移動。
  - **卡片**：
    - 依 `card::place` 擺放；
    - 不搶焦點；
    - 收到 `FocusForCopy` 時借一次焦點。
  - 三個視窗的位置都用實體像素；DIP 的換算用 `win32::scale_at`。
- [ ] **Step 4:** 跑測試與 probe：`cargo test --release --manifest-path rust/Cargo.toml --test ui_probe -- --ignored`，預期全部 PASS。
- [ ] **Step 5:** 跑提交前檢查後 commit：`feat(rust): host the region selector, box and card in the app`。

### Task 10: 主程式接線

**Files:**
- Modify: `rust/src/main.rs`、`rust/src/app_main.rs`、`rust/src/reader/process.rs`（新增 `find_game_window`）
- Test: `rust/tests/app_main.rs`

**Interfaces:**
- Produces：
  - `crate::reader::process::find_game_window() -> Option<isize>`：可見、且屬於遊戲程序的頂層視窗，照 Python `find_game_window`。
  - `app_main::region_runner(translator: Arc<Translator>, rt: Handle, ui: UiPoster) -> Box<dyn RegionRunner>`
  - `install_input` 改為接收共用的 `selecting: Arc<AtomicBool>`，不再自建一個恆為 false 的旗標。

- [ ] **Step 1:** 寫測試：
  - `region_runner_posts_the_result_with_its_session`
  - `region_runner_cancel_posts_nothing`：記 `[region] request cancelled (session=N)`。
  - `region_runner_failure_logs_the_service`：記 `[region] translate failed (rect=.., provider=.., model=..)`。
  - `region_hotkey_cancels_while_selecting`：`selecting` 為 true 時投遞 `RegionHotkey { hwnd: 0 }`。
- [ ] **Step 2:** 跑測試，預期 FAIL。
- [ ] **Step 3:** 實作。
  - `main.rs` 用 `include_bytes!("../assets/ocr/...")` 嵌入四個檔案，在 UI 起來前呼叫 `register_models`。
  - `App` 用 `with_region` 接上 RegionFlow；`set_alpha` 跟著 overlay 透明度。
- [ ] **Step 4:** 跑測試，預期 PASS。另跑 `cargo build --release --manifest-path rust/Cargo.toml`，記下 exe 大小，預期約 50 MB。確認沒有執行中的 `_test\Wizard101ChatTranslator.exe` 後，把 exe 複製到 `rust/target/release/_test/`。
- [ ] **Step 5:** 跑提交前檢查後 commit：`feat(rust): wire region translation into the app`。

---

## 實機驗收清單（第 3 段完成後交給使用者）

1. **快捷鍵**：
   - 遊戲在前景時按 Ctrl+Shift+Space：選取畫面蓋滿遊戲所在的螢幕，遊戲畫面凍結並調暗，桌面其他部分也調暗。
   - 放開按鍵後，選取畫面才拿到焦點；連按幾次都有反應，沒有「第二下沒反應」。
2. **框選**：
   - 拖曳時顯示框線、W×H，框內恢復原亮度。
   - Esc、右鍵、單純點一下都會取消，並把前景還給遊戲。
3. **結果**：
   - 框留在畫面上，卡片先顯示「辨識並翻譯中…」，接著顯示「原文（小字、調暗）＋空一行＋譯文」。
   - 卡片放不下時會依序翻到上方或側邊，換到側邊後不會來回跳。
4. **調整框**：
   - 拖曳框內可以移動，拖邊角可以縮放，放開後重新擷取並重新翻譯。
   - 拖不到 5px 就還原。
   - 調整期間，上一次的請求會被取消。
5. **卡片**：
   - 可以拖曳選字，Ctrl+C 能複製；右鍵「複製」有選取時複製選取範圍，沒有時複製全部。
   - 點一下或按 ✕ 會關閉，框也跟著收掉。
   - 卡片和框都不會搶走遊戲的鍵盤。
6. **⛶ 按鈕**：遊戲不在前景時也能開始框選；找不到遊戲時，在游標處顯示錯誤卡片。
7. **錯誤情況**：
   - 框到遊戲範圍外，提示「請框選遊戲畫面內的區域」；
   - 遊戲最小化時，提示擷取失敗；
   - 斷網時，卡片顯示對應的錯誤文案。
8. **OCR 截圖驗證**（spec 規定）：
   - 提供一批截圖，涵蓋商店標題、NPC 對話框、任務卡、中英混排；
   - 我用 Task 2 的工具產生新舊對照報告；
   - 只要有任何一張新參數讀得比舊參數差，就交給你決定。
9. **第一次框選**：第一次會解出模型，可以稍慢；之後就快。重啟程式後不會再解一次。
