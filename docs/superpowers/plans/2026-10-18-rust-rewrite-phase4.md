# Rust 重寫第 4 段（設定視窗、首次設定精靈、服務管理、更新檢查）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 Rust 版補齊所有設定介面與更新檢查，讓功能與操作和 Python v0.6.0 一致：
- 設定視窗：基本、翻譯服務、進階、關於四頁；
- 首次設定精靈：沒有可用服務時先跑完三步才進主畫面；
- 服務管理：預設服務、各用途指派、新增／編輯／刪除服務、測試連線、模型清單；
- 更新檢查：啟動時與每 60 分鐘自動查，關於頁可手動查。

外觀換成使用者核可的新設計（深色、左側導覽、卡片式設定列），見設計稿 https://claude.ai/artifact/WDdwXMcjrXrDJP9EmEbV4o 。

**Architecture:**
- **非 UI 邏輯**先補齊，各自可單獨測試：
  - 服務操作、進階數值解析（`config/`）；
  - 熱鍵錄製（`input/`）；
  - 測試連線、模型清單、範例生成（`translation/`）；
  - 更新檢查（`runtime/updater.rs`）；
  - 套用設定後重設翻譯端（`app_main.rs`）。
- **介面**放在 `ui/settings/`：每個元件是「狀態＋`ui(&mut Ui)`」，不碰視窗，用 `egui_kittest` 測。耗時工作交給可注入的 `SettingsEnv`，結果經 `UiEvent::Settings` 回到 UI 執行緒。
- **視窗**：`App` 用 immediate viewport 托管設定視窗與精靈，做法同第 2、3 段，但這兩個是**有系統標題列**的一般視窗。
- **精靈與主程式**：winit 一個程序只能開一次事件迴圈，所以精靈也在同一個 eframe 裡跑。沒有可用服務時，overlay 根視窗先藏著，只開精靈；完成後才建立翻譯端、讀取端與熱鍵，再顯示 overlay。

**Tech Stack:** 沿用前三段，不新增 crate。
- 圖示用 Windows 內建的 Segoe Fluent Icons（Win11 的 `SegoeIcons.ttf`），沒有時退到 Win10 的 Segoe MDL2 Assets（`segmdl2.ttf`）。
- 粗體用各語系字型的粗體檔。
- HTTP 測試用既有的 `wiremock`，UI 測試用既有的 `egui_kittest`。

**Spec:** `docs/superpowers/specs/2026-10-17-rust-rewrite-design.md`。

- 驗收依據是 §H（設定介面），以及 §A 的「首次設定精靈」與「更新檢查」兩項。
- 每個 task 的行為以對應的 Python 原始碼為準：
  - `src/ui/settings/*.py`、`src/config/services.py`；
  - `src/runtime/updater.py`；
  - `src/main.py` 的 `apply_settings`、`reconfigure_translation`、`schedule_update_checks` 與首次設定流程。
- 也要納入 `docs/superpowers/plans/2026-10-17-rust-rewrite-followups.md` 的「第 4 段」各項。
- **外觀以設計稿為準**：顏色、尺寸、版面、圖示碼位都寫在本計畫的 Global Constraints，設計稿是對照用的畫面。
- **使用者核可的兩個行為差異**（2026-10-18）：
  - 測試連線成功時多顯示耗時（秒，小數一位、整數毫秒四捨五入）與原文，共三行。
  - 模型清單按鈕在繁中改叫「更新清單」。

## Global Constraints

- **提交前檢查依改動範圍**：
  - 只動 `rust/` 時，跑三項 cargo 檢查：`fmt --check`、`clippy --all-targets -- -D warnings`、`test`。linker 報 1102 時改用 `cargo test -j 2`。
  - 有動到 `src/`（含 `src/i18n/*.json`）、`tests/`、`tools/` 或 `pyproject.toml` 時，另外跑 `uv run ruff check src tests tools` 與 `uv run pytest`。
  - 只動文件時不必跑。
  - 不可用 `--no-verify`，不可 push。
- 第 1 段計畫的其他 Global Constraints 照樣沿用：log 規則、註解規則、移植測試、commit 格式（英文 conventional commits，結尾附 trailer）。
- **i18n**：
  - 只新增三個 key，其餘文字一律用現有 key。
  - 有翻譯這幾個 key 的語系都要改（使用者 2026-10-18 要求）：目前是 `zh-TW`、`en-US`、`zh-CN`，其餘語系檔沒有翻到這一區。動手前再用 `grep '"button.refresh"' src/i18n/*.json` 確認一次，有新語系翻到就一併補上。

  | key | zh-TW | en-US | zh-CN |
  |---|---|---|---|
  | `test.connected`（新） | `連線成功（耗時 {seconds} 秒）` | `Connected ({seconds}s)` | `连接成功（耗时 {seconds} 秒）` |
  | `test.source`（新） | `原文` | `Source` | `原文` |
  | `test.translation`（新） | `譯文` | `Translation` | `译文` |
  | `button.refresh`（改） | `更新清單` | `Refresh list` | `刷新列表` |
  | `hint.model_idle`（改） | `按「更新清單」從伺服器取得可用模型清單；也可以自行輸入模型名稱。` | `Click Refresh list to fetch the model list from the server, or type a model name yourself.` | `点“刷新列表”从服务器获取可用模型列表；也可以自行输入模型名称。` |

  - 新 key 放在各檔 `test.success` 的下一行，三個檔案位置一致。
  - `test.success` 保留不刪（Python 版還在用）。
  - 改完若 `rust/tests/golden/i18n_t.json` 涵蓋到改過的 key，重跑 `uv run python -m tools.golden.export i18n_t`。
- **log**：
  - 前綴對齊 Python：`[settings]`、`[ui]`、`[update]`、`[app]`、`[hotkey]`、`[translate]`。
  - 文字盡量與 Python 同一行 log 相同。
  - **API 金鑰絕不可寫進 log**，只記 `has_key=True/False`。
- **測試不可干擾桌面**：
  - 不可裝真的鍵盤 hook、呼叫 `SetForegroundWindow`，也不可開可見視窗。
  - 視窗相關一律經可注入的介面處理；UI 內容用 `egui_kittest::Harness` 測。
  - 唯一例外是標成 `#[ignore]` 的螢幕外 probe，做法沿用 `rust/tests/ui_probe.rs`。
  - 切換全域介面語言的 UI 測試要自成一個測試執行檔，做法同 `rust/tests/overlay_relabel.rs`。
- **設定視窗與精靈的視窗**：
  - 一般視窗：有系統標題列、可縮放、有工作列按鈕、不擁有者（owner）。
  - **不呼叫** `remove_system_frame`、`hide_from_taskbar`、`make_non_activating`。
  - HWND 用 `win32::pick_new_window` 找。
  - 找到 HWND 後呼叫新的 `win32::dark_caption(hwnd)`。
  - 視窗圖示用 `icon_png(48)`。
- **深色標題列**：`DwmSetWindowAttribute` 設兩個屬性。
  - `DWMWA_USE_IMMERSIVE_DARK_MODE`（20）＝1；
  - `DWMWA_CAPTION_COLOR`（35）＝`#101018`，COLORREF 是 `0x00181010`。
  - 失敗時記一次 log，例如 Win10 不支援 35。
- **配色**（`ui/settings/theme.rs`；能沿用 `ui/palette.rs` 的就引用它）：

  | 名稱 | 色碼 | 用途 |
  |---|---|---|
  | `WINDOW_BG` | `#15151f` | 內容區底 |
  | `SIDEBAR_BG` | `#101018`（`palette::BG`） | 側欄、精靈步驟條以外的深底 |
  | `NAV_SELECTED` | `#23233a`（`palette::BAR`） | 選中的導覽項 |
  | `CARD_BG` | `#1c1c2a` | 卡片、對話框 |
  | `CARD_BORDER` | `#2a2a3e` | 卡片外框、分隔線 |
  | `CONTROL_BG` | `#262638` | 按鈕、輸入框、下拉 |
  | `CONTROL_BORDER` | `#3a3a55` | 控制項外框、捲軸 |
  | `CONTROL_UNDERLINE` | `#6a6a88` | 輸入框底線（聚焦時改 `ACCENT`、2px） |
  | `ACCENT` | `#6fa8dc`（`palette::TAB_ACCENT`） | 主要按鈕、開關、選中指示、焦點框 |
  | `ON_ACCENT` | `#0b1220` | 主要按鈕與開關上的字 |
  | `TEXT` | `#f2f2f7` | 一般文字 |
  | `TEXT_SECONDARY` | `#a3a7b8` | 說明文字 |
  | `TEXT_MUTED` | `#8a8aa0` | 版本號、步驟未到 |
  | `LINK` | `#7fb2ff` | 連結 |
  | `OK` | `#6fd39a` | 成功 |
  | `ERROR` | `#ff8a8a` | 失敗 |
  | `DANGER` / `ON_DANGER` | `#ff7a7a` / `#1a0b0b` | 刪除確認按鈕 |
  | `WARN_BG` / `WARN_BORDER` / `WARN_TEXT` | `#3a2a14` / `#6b4a1c` / `#ffd9a3` | 儲存被擋下的提示 |
  | `PILL_BG` / `PILL_TEXT` | `#22314a` / `#a9cdf5` | 服務卡片的角色標記 |
  | `KEYCAP_BG` / `KEYCAP_BORDER` | `#2b2b42` / `#40405c` | 熱鍵按鍵帽 |
  | `HOT_BG` | `#24304a` | 服務商卡片 hover（外框改 `ACCENT`） |
  | `MODAL_FOOT` | `#181826` | 對話框按鈕列底、精靈步驟條底 |
  | `SCRIM` | `rgba(6,6,12,0.62)` | 對話框背後的遮罩 |

- **尺寸**（單位是 egui point）：

  | 項目 | 值 |
  |---|---|
  | 設定視窗 | 預設 840×660，最小 640×360 |
  | 精靈 | 預設 600×760，最小 600×380 |
  | 側欄 | 寬 200；導覽項高 40、圓角 6；選中項左側有 3 寬的 `ACCENT` 指示條（上下各縮 12） |
  | 內容區 | 內距上 20、右 28、下 16、左 24；頁標題 22 粗體 |
  | 卡片 | 圓角 8、外框 1；卡片間距 10 |
  | 設定列 | 最小高 48，內距 10×16，列間 1 寬分隔線；左邊名稱 14、說明 12.5 |
  | 控制項 | 高 32、圓角 6；下拉寬 240；數值欄寬 132（含 −／＋）；滑桿寬 200；開關 40×20 |
  | 底部按鈕列 | 高 60（精靈 64）、上方 1 寬分隔線 |
  | 對話框 | 圓角 10；服務表單寬 540、服務商選擇寬 460、刪除確認寬 280 |
  | 字級 | 內文 14、說明 12.5 |

- **圖示碼位**（Segoe Fluent Icons／MDL2 共用）：

  | 用途 | 碼位 | 用途 | 碼位 |
  |---|---|---|---|
  | 基本 | `E713` | 翻譯服務 | `E774` |
  | 進階 | `E9E9` | 關於 | `E946` |
  | 下拉 | `E70D` | 收合 | `E70E` |
  | 下一步 | `E76C` | 上一步 | `E76B` |
  | 成功 | `E73E` | 更新清單 | `E72C` |
  | 外部連結 | `E8A7` | 開啟資料夾 | `E838` |
  | 檢查更新 | `E895` | 顯示金鑰 | `E890` |
  | 失敗 | `E783` | 警告 | `E7BA` |
  | 減少 | `E738` | 增加 | `E710` |
  | 關閉 | `E8BB` | | |

- **鍵盤操作**：
  - 所有可點的元件都要能用 Tab 走到、Enter／Space 觸發，聚焦時畫 `ACCENT` 外框。自訂元件包括導覽項、開關、服務商卡片、略過測試、收合列。
  - 對話框按 Esc＝取消，點遮罩不關。
- **不可先顯示、晚點才給焦點**：設定視窗與精靈在找到 HWND 的那一幀就 `force_foreground`。它們不是熱鍵開的，不必等觸發鍵放開。

## Review Focus

1. **小螢幕**（1366×768、150% 縮放，工作區約 910×485 point）：視窗初始大小夾在工作區內，內容區可捲動，儲存鈕與下一步永遠看得到。測試放在 Task 11：`initial_size_fits_the_work_area`。
2. **測試連線或抓模型清單還沒回來，就改了欄位、換了服務商或關了對話框**：晚到的結果丟掉，不可把新設定標成「已測過」；按鈕要恢復可按。測試放在 Task 7：`a_result_after_editing_is_discarded_and_the_button_recovers`。
3. **進階數值欄**打了非數字、空白、`NaN`、`inf` 或超出範圍：非數字擋下儲存並提示 `error.advanced_not_number`，範圍外夾到範圍內，設定檔不可寫進非有限值。測試放在 Task 1：`parse_advanced_rejects_non_numbers_and_non_finite`。
4. **錄熱鍵時按 Esc、關掉視窗或換頁**：錄製器從分派器移除，熱鍵維持原值，之後按鍵不再被記錄。測試放在：
   - Task 2：`dropping_a_recording_removes_its_handler`；
   - Task 9：`closing_while_recording_cancels_it`。
5. **斷網或 GitHub 回 403／非 JSON 時的自動更新檢查**：只記 log、不跳橫幅，一小時後照常再查。使用者關掉過的版本，只有出現更新的版本才再提醒。測試放在 Task 5：
   - `scheduled_check_survives_failures_and_runs_again`；
   - `offer_update_skips_versions_not_newer_than_dismissed`。

---

### Task 1: 服務操作與進階數值解析（config）

**Files:**
- Modify: `rust/src/config/services.rs`、`rust/src/config/mod.rs`
- Modify: `tools/golden/export.py`（新增 `config_unique_name`、`config_is_auto_name`）
- Test: `rust/tests/config_services.rs`（新）、`rust/tests/config_golden.rs`
- 移植 `tests/config/test_services.py` 中 UI 相關的測試：
  - provider metadata、short name；
  - `validate_endpoint_fields`；
  - `new_service`、`unique_name`、`is_auto_name`、`describe`。
- 移植 `tests/ui/settings/test_window.py` 的 `parse_advanced_values` 與 `clamp_advanced` 測試。

**Interfaces:**
- Produces（`crate::config::services`）：
  - `pub struct Provider`，新增欄位：`label_key: &'static str`、`desc_key: &'static str`、`key_url: Option<&'static str>`。
    - 欄位 `key`、`label_key`、`desc_key`、`key_url` 改為 `pub`。
    - `has_field`、`short_name` 改為 `pub`，新增 `pub fn needs_base_url(&self) -> bool`。
    - 值照 Python `PROVIDERS`：`provider.<key>`、`provider.<key>_desc` 與三家的取金鑰網址。
  - `pub fn providers() -> &'static [Provider]`；`pub fn provider(key: &str) -> Option<&'static Provider>`，現有的私有函式改為 `pub`。
  - `pub const EFFORT_LOW: &str = "low"`；`pub const EFFORTS: [&str; 2] = [EFFORT_AUTO, EFFORT_LOW]`。
  - `pub fn new_service(provider: &str, services: &[Value]) -> Value`：現有私有版本改成吃 key，內部呼叫端跟著改。
  - `pub fn unique_name(name: &str, services: &[Value], ignore_id: Option<&str>) -> String`：現有呼叫端傳 `None`。
  - `pub fn is_auto_name(name: &str, base: &str) -> bool`
  - `pub fn describe(service: &Value) -> String`：`"{short_name} · {model}"`。
  - `pub fn validate_endpoint_fields(service: &Value) -> Vec<String>`；`validate_service` 改成「`model` 檢查＋這一個」。
- Produces（`crate::config`）：
  - `pub struct AdvancedText { pub fade_seconds: String, pub max_messages: String, pub max_parallel_translations: String, pub type_delay: String, pub overlay_alpha: f64 }`
  - `pub fn parse_advanced(text: &AdvancedText) -> Result<Config, &'static str>`：
    - 回傳五個鍵，已經過 `clamp_advanced`，錯誤時回 `"error.advanced_not_number"`。
    - 整數欄位照 Tk `IntVar.get`：先試整數，不行就以浮點數解析後向零截斷。
    - 前後空白忽略；`NaN`／`inf` 視為非數字。
  - `pub fn advanced_limits(key: &str) -> (f64, f64)`：設定頁的範圍說明用。

- [ ] **Step 1:** 在 `tools/golden/export.py` 加兩個 exporter，跑 `uv run python -m tools.golden.export config_unique_name config_is_auto_name`。
  - `config_unique_name`：input 是 `(name, services, ignore_id)`。案例至少涵蓋：無衝突、`X`／`X (2)` 已存在、`ignore_id` 指向自己、服務沒有 `name` 欄位。
  - `config_is_auto_name`：案例至少涵蓋 `X`、`X (2)`、`X (10)`、`X (1)`、`X (02)`、`X (２)`（全形）、`X(2)`、`X (2) `。
- [ ] **Step 2:** 寫測試：兩個 golden 測試，加上移植的 Python 測試。另外加：
  - `parse_advanced_rejects_non_numbers_and_non_finite`（Review Focus 3）：`"abc"`、`""`、`"NaN"`、`"inf"` 都回 `Err("error.advanced_not_number")`；`"1.9"` 的整數欄位得 1；`"99999"` 的淡出夾到 3600。
  - `every_provider_label_and_description_resolve`：四家的 `label_key`、`desc_key` 在 zh-TW 都翻得出來，不是 key 本身。
- [ ] **Step 3:** 跑 `cargo test --test config_services --test config_golden`，預期 FAIL。
- [ ] **Step 4:** 實作。
- [ ] **Step 5:** 跑三項 cargo 檢查，以及 `uv run ruff check src tests tools` 與 `uv run pytest`（動到 `tools/`），全過。
- [ ] **Step 6:** Commit：`feat(rust): expose service and advanced-value helpers for the settings UI`

---

### Task 2: 熱鍵錄製

**Files:**
- Modify: `rust/src/input/dispatcher.rs`
  - `KeyEvent` 新增 `pub raw: Option<RawKey>`；
  - 新增 `pub fn handler_count(&self) -> usize`；
  - 測試裡的 `KeyEvent { .. }` 補 `raw: None`。
- Modify: `rust/src/input/hook.rs`：`to_event` 填 `raw`。
- Modify: `rust/src/input/keys.rs`：新增 `event_name`、`hotkey_name`；`Buffers` 改放在 `static Mutex`，跨呼叫保留內容，同 Python 的全域緩衝。
- Create: `rust/src/input/recorder.rs`
- Modify: `tools/golden/export.py`（新增 `keyboard_hotkey_name`、`keyboard_event_names`）
- Test: `rust/tests/keys_golden.rs`、`rust/tests/hotkey_recorder.rs`（新）

**Interfaces:**
- Produces：
  - `#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub struct RawKey { pub vk: u32, pub extended: bool }`（`crate::input::dispatcher`）
  - `pub fn event_name(scan: u32, vk: u32, extended: bool, modifiers: &[&str]) -> Option<String>`（`crate::input::keys`）：`_winkeyboard.get_event_names` 的第一個名稱，經 `normalize_name`。
  - `pub fn hotkey_name(names: &[String]) -> String`（`crate::input::keys`）：`keyboard.get_hotkey_name` 的移植，見 `.venv/Lib/site-packages/keyboard/__init__.py`。
  - `crate::input::recorder`：
    - `pub fn record(d: &SharedDispatcher, done: Box<dyn FnOnce(String) + Send>) -> Recording`
    - `pub struct Recording`：`Drop` 時從分派器移除 handler。`done` 在 hook 執行緒上呼叫，只會呼叫一次。

- [ ] **Step 1:** 在 `tools/golden/export.py` 加兩個 exporter，跑 `uv run python -m tools.golden.export keyboard_hotkey_name keyboard_event_names`。
  - `keyboard_hotkey_name`：餵名稱清單給 `keyboard.get_hotkey_name`。案例至少涵蓋：
    - `["space","ctrl"]`；
    - `["left ctrl","right shift","a"]`；
    - `["+","ctrl"]`（得 `ctrl+plus`）；
    - 重複名稱；
    - 只有修飾鍵；
    - `["windows","alt","shift","ctrl","f1"]` 的排序。
  - `keyboard_event_names`：對一批 `(scan, vk, extended, modifiers)` 輸出 `_winkeyboard.get_event_names(...)[0]` 經 `normalize_name` 的結果。案例至少涵蓋：空白鍵、`a`、shift＋`1`、caps lock＋`a`、數字鍵盤的 `5`、右 ctrl（extended）、F1。
  - 與 `keyboard_names` 一樣和鍵盤配置有關，在本機產生。
- [ ] **Step 2:** 寫測試：
  - `keys_golden.rs`：
    - `hotkey_name_matches_python`
    - `event_name_matches_python`
  - `hotkey_recorder.rs`（用 `Dispatcher` 直接 `process`，不裝 hook）：
    - `records_ctrl_space_on_the_first_release`：ctrl down、space down、space up 得 `"ctrl+space"`；之後的事件不再回呼。
    - `does_not_swallow_keys`：錄製期間 `process` 一律回 true。
    - `shift_changes_the_recorded_name_like_python`：shift down、`1` down、`1` up，得出與 golden 相同的名稱（US 配置是 `shift+!`）。
    - `dropping_a_recording_removes_its_handler`（Review Focus 4）：`handler_count` 回到錄製前的值；之後的按鍵不回呼。
- [ ] **Step 3:** 跑測試，預期 FAIL。
- [ ] **Step 4:** 實作，修飾鍵狀態照 `_winkeyboard.process_key`：
  - shift 由 vk `0x10`／`0xA0`／`0xA1` 的按下／放開追蹤；
  - Alt Gr 由 scan 541＋vk 162 追蹤；
  - num lock／caps lock／scroll lock 在事件當下讀 `GetKeyState(...) & 1`。
  - 錄製器自己追蹤 shift 與 Alt Gr，只看它開始之後的事件。
  - 名稱在 hook 執行緒上算；`done` 只做投遞，不做 I/O。
- [ ] **Step 5:** 跑三項 cargo 檢查以及 ruff／pytest，全過。
- [ ] **Step 6:** Commit：`feat(rust): record hotkeys through the keyboard hook like keyboard.read_hotkey`

---

### Task 3: 測試連線、模型清單與範例生成（translation）

**Files:**
- Modify: `rust/src/translation/translator.rs`
- Test: `rust/tests/translator.rs`（沿用 `tests/common/fake_backend.rs`）、`rust/tests/clients_openai.rs`（`list_models` 走 `wiremock`）
- 移植 `tests/ui/settings/test_service_form.py::test_test_connection_also_generates_examples` 的語意。

**Interfaces:**
- Produces（`crate::translation::translator`）：
  - `pub async fn list_models(spec: &ServiceSpec) -> Result<Vec<String>, TranslateError>`：`build_client(spec).list_models()`。
  - `#[derive(Debug, Clone, PartialEq, Eq)] pub struct TestOutcome { pub translated: String, pub elapsed: Duration }`
  - `pub async fn test_translate(tr: Arc<Translator>, store: Option<Arc<ExampleStore>>) -> Result<TestOutcome, TranslateError>`
    - `elapsed` 只算 `translate_incoming(TEST_SAMPLE, &[])` 這一次。
    - 成功後若有 `store`，接著跑 `generate_and_store`。失敗只記 `[translate] example generation after the connection test failed: {class_name}: {e}`，不影響回傳。
  - `pub async fn generate_and_store(tr: &Translator, store: &ExampleStore) -> Result<Option<ExampleSet>, TranslateError>`
    - 依 `tr.examples_fingerprint()` 查快取，命中就記 `[translate] examples loaded from cache: {summary}` 並回傳。
    - 否則 `generate_examples`，全空回 `None` 且不存，有內容就 `store.put`。

- [ ] **Step 1:** 寫測試：
  - `test_translate_times_only_the_sample_translation`：fake backend 第一次回譯文，`on_call` 讓第二次（範例生成）睡 200ms；`elapsed` 要小於 150ms。
  - `test_translate_generates_and_stores_examples`
  - `example_failure_after_a_good_test_still_returns_the_translation`
  - `generate_and_store_uses_the_cache_and_skips_empty_results`
  - `list_models_goes_through_the_built_client`：wiremock 的 `/v1/models`。
- [ ] **Step 2:** 跑測試，預期 FAIL。
- [ ] **Step 3:** 實作。
- [ ] **Step 4:** 跑三項 cargo 檢查，全過。
- [ ] **Step 5:** Commit：`feat(rust): port test_translate, list_models and generate_and_store`

---

### Task 4: 套用設定後重設翻譯端

**Files:**
- Modify: `rust/src/app_main.rs`
- Test: `rust/tests/app_main.rs`
- 移植 `tests/test_main.py` 中所有 reconfigure／fingerprint 相關的測試：
  - `test_changing_only_the_region_slot_*`、`test_changing_the_incoming_slot_invalidates_the_cache`；
  - `test_reconfigure_*`；
  - `test_a_settings_change_applies_the_new_fingerprints_examples_and_never_the_old`；
  - `test_saving_unrelated_settings_rebuilds_no_translator`；
  - `test_editing_the_service_a_slot_uses_rebuilds_it`；
  - `test_changing_the_target_language_rebuilds_every_translator`；
  - `test_switching_the_incoming_slot_between_same_named_models_invalidates_the_cache`。

**Interfaces:**
- Consumes：`Translator::reconfigure`、`TranslationPool::resize`、`TranslationCache::rebind`、`ExampleCoordinator::ensure`、`incoming_fingerprint`（皆已存在）。
- Produces：`pub fn reconfigure_translation(cfg: &Config, translation: &Translation)`
  - 三格各自 `reconfigure`。有重建的那格記 `[translate] {slot} translator rebuilt: {describe}`，`slot` 是 `ServiceSlot::key()`。
  - 收訊與框選那兩格重建時呼叫 `coordinator.ensure`。
  - 兩條池 `resize(max_parallel_translations)`。
  - 最後 `cache.rebind(incoming_fingerprint(cfg, incoming.examples()))`。
  - 前提：呼叫端已先把新設定寫進共用的 `RwLock<Config>`。`build_translation` 的 `on_applied` 讀的是那份，否則晚到的範例會把快取綁回舊指紋（待辦文件第 4 段第 4 項）。這個前提寫進 doc comment。

- [ ] **Step 1:** 寫移植的測試。另加 `late_examples_after_a_save_bind_the_cache_to_the_new_fingerprint`：先改共用 cfg 並呼叫 `reconfigure_translation`，再讓舊的範例生成晚到完成，快取指紋要是新的。
- [ ] **Step 2:** 跑測試，預期 FAIL。
- [ ] **Step 3:** 實作。
- [ ] **Step 4:** 跑三項 cargo 檢查，全過。
- [ ] **Step 5:** Commit：`feat(rust): reconfigure the translators and cache when settings are saved`

---

### Task 5: 更新檢查

**Files:**
- Create: `rust/src/runtime/updater.rs`；Modify: `rust/src/runtime/mod.rs`
- Modify: `rust/src/ui/overlay/banners.rs`
  - `Release` 搬到 updater，這裡改成 `pub use`；
  - `offer_update` 改用 SemVer，刪掉 `shortcut:` 註解。
- Modify: `rust/src/ui/events.rs`（新增 `UiEvent::UpdateFound(Release)`）、`rust/src/ui/app.rs`（收到就 `banners.offer_update`）
- Modify: `tools/golden/export.py`（新增 `updater_is_newer`）
- Test: `rust/tests/updater.rs`（新，wiremock）、`rust/tests/updater_golden.rs`（新）
- 移植 `tests/runtime/test_updater.py` 全部 21 個測試，其中 announce／schedule 的語意改測 `schedule_update_checks`。

**Interfaces:**
- Produces（`crate::runtime::updater`）：
  - 常數：`PROJECT_URL`、`RELEASES_URL`、`ISSUES_URL`、`AUTHOR_URL`、`CROWDIN_URL`、`LATEST_API`，值同 Python。
  - `pub const UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(3600)`；`pub const TIMEOUT: Duration = Duration::from_secs(10)`
  - `#[derive(Debug, Clone, PartialEq, Eq)] pub struct Release { pub version: String, pub url: String }`
  - `pub fn parse_version(text: &str) -> Option<(u64, u64, u64)>`；`pub fn is_newer(latest: &str, current: &str) -> bool`
  - `#[derive(Debug, Clone, PartialEq, Eq)] pub struct UpdateCheckError(pub String)`，`Display` 顯示內文。
  - `pub fn http_client() -> reqwest::Client`：逾時 10 秒、跟隨轉址。
  - `pub async fn fetch_latest_release(client: &reqwest::Client, url: &str) -> Result<Option<Release>, UpdateCheckError>`
    - 送 `Accept: application/vnd.github+json` 與 `User-Agent: wizard101-chat-translator/<CARGO_PKG_VERSION>`。
    - 404 回 `None`，其他非 200 回 `Err("HTTP {status}")`。
    - `tag_name` 去掉前綴的 `v`／`V`；沒有 `html_url` 時退到 `RELEASES_URL`。
  - `pub async fn check_for_update(client: &reqwest::Client, url: &str, current: &str) -> Result<Option<Release>, UpdateCheckError>`
  - `pub fn schedule_update_checks(rt: &Handle, stop: CancellationToken, url: String, interval: Duration, found: Arc<dyn Fn(Release) + Send + Sync>) -> JoinHandle<()>`
    - 立刻查一次，之後每 `interval` 再查。每輪先記 `[update] next automatic check in {n} min`。
    - 失敗記 `[update] check failed: {e}`；`stop` 觸發就結束。
- Produces（`Banners`）：`offer_update` 在「已有關掉的版本，且新版本不比它新」時略過，log 照舊。

- [ ] **Step 1:** 在 `tools/golden/export.py` 加 `updater_is_newer`：`(latest, current)` 配對，涵蓋數字比較、預發布、build metadata 與無法解析的版本。跑 `uv run python -m tools.golden.export updater_is_newer`。
- [ ] **Step 2:** 寫測試：
  - 移植的 21 個；
  - `is_newer_matches_python`（golden）；
  - `scheduled_check_survives_failures_and_runs_again`（Review Focus 5）：wiremock 第一次回 403、第二次回新版；interval 用 50ms；`found` 只被呼叫一次。
  - `offer_update_skips_versions_not_newer_than_dismissed`（Review Focus 5）：關掉 0.2.0 後，再給 0.2.0 和 0.1.9 都不顯示，給 0.2.1 要顯示。
  - `stop_ends_the_schedule`
- [ ] **Step 3:** 跑測試，預期 FAIL。
- [ ] **Step 4:** 實作。
- [ ] **Step 5:** 跑三項 cargo 檢查以及 ruff／pytest，全過。
- [ ] **Step 6:** Commit：`feat(rust): check GitHub releases for updates on a schedule`

---

### Task 6: 設定介面的視覺基礎、共用元件與背景工作介面

**Files:**
- Modify: `rust/src/ui/fonts_egui.rs`：新增 `FontFamily::Name("bold")` 與 `FontFamily::Name("icons")`。
- Modify: `rust/src/ui/win32.rs`：新增 `dark_caption`。
- Create:
  - `rust/src/ui/settings/mod.rs`
  - `rust/src/ui/settings/theme.rs`：配色、尺寸常數。
  - `rust/src/ui/settings/widgets.rs`：共用元件。
  - `rust/src/ui/settings/env.rs`：耗時工作的介面與 round 追蹤。
- Modify: `rust/src/ui/events.rs`（新增 `UiEvent::Settings(SettingsEvent)`）、`rust/src/ui/mod.rs`
- Test: `rust/tests/settings_widgets.rs`（新）
- 移植 `tests/ui/settings/test_form.py` 的 BackgroundButton、collapsible 與鍵盤操作測試。Tk 專屬的（grab、wraplength、pack 順序）略過，在 commit 訊息列出。

**Interfaces:**
- Consumes：Task 2 的 `record`／`Recording`、Task 3 的 `test_translate`／`list_models`／`TestOutcome`、Task 5 的 `check_for_update`／`http_client`／`Release`。
- Produces（`crate::ui::fonts_egui`）：
  - 粗體檔依主字型：`msjhbd.ttc`、`msyhbd.ttc`、`YuGothB.ttc`、`malgunbd.ttf`、`segoeuib.ttf`；
  - 圖示檔：`SegoeIcons.ttf`，沒有就用 `segmdl2.ttf`；
  - 檔案不存在時，該 family 退回一般字型並記 log。
- Produces（`crate::ui::win32`）：`pub fn dark_caption(hwnd: isize)`。
- Produces（`crate::ui::settings::widgets`）：
  - `card(ui, add_contents)`
  - `setting_row(ui, label: &str, hint: Option<&str>, control: impl FnOnce(&mut Ui))`
  - `toggle(ui, on: &mut bool, label: &str) -> Response`：`label` 只給無障礙名稱，畫面上的文字由 `setting_row` 畫。
  - `primary_button(ui, text) -> Response`、`secondary_button(ui, text) -> Response`
  - `icon_button(ui, glyph: char, tooltip: &str) -> Response`
  - `collapsible(ui, id, title: &str, hint: Option<&str>, default_open: bool, body: impl FnOnce(&mut Ui))`
  - `keycaps(hotkey: &str) -> Vec<Vec<String>>`：以 `, ` 分段、以 `+` 分鍵。每個詞首字母大寫，單一字元整個大寫，例如 `ctrl+shift+space` 得 `[["Ctrl","Shift","Space"]]`。
  - `editable_combo(ui, id, text: &mut String, options: &[String], filter: bool) -> Response`：可自行輸入的下拉；`filter` 時邊打邊篩，篩法同 `filter_models`。方向鍵選、Enter 選定、Esc 收起，點別處收起；沒有相符項目時不展開。
  - `number_field(ui, text: &mut String, step: f64, range: (f64, f64)) -> Response`
  - `callout(ui, title: &str, lines: &[String])`：儲存被擋下的提示。
  - `outcome(ui, ok: bool, text: &str)`：失敗時文字走 `ui::richtext::show`，網址可點（待辦文件第 4 段第 1 項）。
  - `pub fn filter_models(models: &[String], query: &str) -> Vec<String>`：不分大小寫的子字串比對；關鍵字空白時回全部。
- Produces（`crate::ui::settings::env`）：
  - `pub trait SettingsEnv`：
    - `fn test_connection(&self, round: u64, spec: ServiceSpec, target: String)`
    - `fn list_models(&self, round: u64, spec: ServiceSpec)`
    - `fn check_update(&self, round: u64)`
    - `fn record_hotkey(&self, round: u64)`、`fn cancel_recording(&self)`
    - `fn open_folder(&self, path: &Path)`
    - `fn has_cache(&self) -> bool`：沒有快取時關於頁不畫譯文快取那一列。
    - `fn clear_cache(&self) -> usize`：回傳清掉的筆數。
  - `pub enum SettingsEvent`：
    - `TestDone { round: u64, result: Result<TestOutcome, TranslateError> }`
    - `ModelsDone { round: u64, result: Result<Vec<String>, TranslateError> }`
    - `UpdateChecked { round: u64, result: Result<Option<Release>, String> }`
    - `HotkeyRecorded { round: u64, combo: String }`
  - `pub struct RealEnv { rt: Handle, ui: UiPoster, dispatcher: SharedDispatcher, cache: Option<Arc<TranslationCache>>, recording: Mutex<Option<Recording>> }`
    - 用 Task 2、3、5 的函式，結果以 `UiEvent::Settings` 投遞；
    - 範例存放用 `Arc::new(ExampleStore::default())`；
    - `open_folder` 用 `ShellExecuteW`，失敗記 `[ui] open log folder failed: path=... error=...`；
    - `clear_cache` 記 `[settings] translation cache cleared by user ({n} entries)`。
  - `pub struct Round`：
    - `fn start(&mut self) -> u64`：用全域遞增序號，回傳本輪編號；
    - `fn invalidate(&mut self)`
    - `fn finish(&mut self, round: u64) -> bool`：只有本輪且沒被作廢才回 true；本輪結束時一律解除忙碌；
    - `fn busy(&self) -> bool`

- [ ] **Step 1:** 寫 `settings_widgets.rs`：
  - `keycaps_splits_steps_and_keys`：
    - `ctrl+space` 得 `[["Ctrl","Space"]]`；
    - `ctrl+shift+a, b` 得 `[["Ctrl","Shift","A"],["B"]]`；
    - `page up` 得 `[["Page Up"]]`。
  - `toggle_is_reachable_by_tab_and_flips_on_space`
  - `collapsible_header_is_keyboard_activatable`
  - `editable_combo_filters_while_typing_and_picks_with_enter`
  - `editable_combo_stays_closed_when_nothing_matches`
  - `filter_models_is_case_insensitive_substring`、`filter_models_blank_query_returns_all`
  - `round_discards_invalidated_and_stale_results`：涵蓋 BackgroundButton 的三個情境（作廢後的結果丟掉但按鈕恢復、慢的一輪不會落到後一輪、正常結果送達）。
  - `outcome_makes_urls_clickable`
- [ ] **Step 2:** 跑測試，預期 FAIL。
- [ ] **Step 3:** 實作。
- [ ] **Step 4:** 跑三項 cargo 檢查，全過。
- [ ] **Step 5:** Commit：`feat(rust): add the settings theme, shared widgets and background work env`

---

### Task 7: 服務表單

**Files:**
- Create: `rust/src/ui/settings/service_form.rs`
- Modify: `src/i18n/zh-TW.json`、`src/i18n/en-US.json`、`src/i18n/zh-CN.json`（見 Global Constraints 的 i18n 表）
- Test: `rust/tests/service_form.rs`（新）
- 移植：
  - `tests/ui/settings/test_service_form.py` 全部；
  - `tests/ui/settings/test_fields.py` 的模型欄位、`friendly_error` 與服務商標籤相關測試；Tk 下拉的 grab／post 測試改測 `editable_combo` 的同等行為。

**Interfaces:**
- Consumes：Task 1 的服務函式、Task 6 的 widgets、`SettingsEnv`、`SettingsEvent`、`Round`。
- Produces（`crate::ui::settings::service_form`）：
  - `pub struct ServiceForm`：
    - `pub fn new(service: Value, services: Vec<Value>) -> Self`
    - `pub fn values(&self) -> Value`：完整的一筆服務（id、名稱、服務商與該家欄位）；名稱空白時退回去重後的短名。
    - `pub fn api_values(&self) -> Value`：去掉 id 與名稱。
    - `pub fn set_provider(&mut self, key: &str)`：規則同 Python——名稱還是上一家的自動名才換，欄位清空並套新服務商預設，測試結果與模型清單作廢，記 `[settings] service form provider switched {old} -> {new}`。
    - `pub fn set_name(&mut self, name: &str)`
    - `pub fn test_passed(&self) -> bool`
    - `pub fn apply(&mut self, event: &SettingsEvent)`
    - `pub fn ui(&mut self, ui: &mut Ui, env: &dyn SettingsEnv, target_language: &str) -> FormResponse`
  - `pub struct FormResponse { pub changed: bool, pub change_provider: bool }`：`changed` 是 API 欄位有變動（精靈用來作廢「略過測試」），`change_provider` 由外層開服務商選擇。
  - `pub fn format_seconds(elapsed: Duration) -> String`：先換成整數毫秒，`(ms + 50) / 100` 得十分位，輸出 `"{整數}.{小數}"`。
- 版面照設計稿的服務表單：
  - 標籤欄寬 96，欄位依序：名稱、服務商（圖示＋名稱＋〔更改〕）、自訂端點的伺服器網址＋`hint.custom_endpoint`。
  - API 金鑰預設遮住，右側眼睛圖示切換顯示（tooltip `button.show`）；官方三家底下有 `link.get_key` 連到該家的 `key_url`；自訂端點用 `field.api_key_optional`。
  - 模型：`editable_combo`（`filter: true`）＋〔`button.refresh`〕，下方狀態說明：
    - 還沒抓：`hint.model_idle`；
    - 抓到：`hint.model_found`，0 個時 `hint.model_empty`；
    - 端點沒清單（`TranslateError::NoModelList`）：`hint.model_no_list`；
    - 失敗：`friendly_error` 的文字，紅字、網址可點。
    - 抓之前先 `validate_endpoint_fields`，有缺就直接顯示錯誤，以 `sep.errors` 連接。
  - 思考深度下拉（Claude／Gemini，選項 `effort.auto`、`effort.low`）＋`hint.effort`；或思考開關＋`hint.thinking`（OpenAI／自訂）。
  - 分隔線下是〔`button.test`〕與結果；測試中按鈕顯示 `button.testing` 並停用，抓清單中顯示 `button.loading`。
- 測試連線：
  - 開始前 `validate_service`，有缺就直接顯示錯誤；
  - 成功三行：`✓ test.connected(seconds)`、`test.source` ＋ `TEST_SAMPLE`、`test.translation` ＋譯文；
  - 失敗一行：`friendly_error` 的文字，網址可點；
  - log 同 Python：`[settings] test connection ok (provider=..., model=...)`、`[settings] test connection failed (provider=..., model=...): {e}`；模型清單的 `[settings] model list fetched/unsupported/failed ...`。
- API 欄位（金鑰、模型、網址、思考深度、思考開關）一變動就作廢測試結果（`Round::invalidate`）並回報 `changed`。

- [ ] **Step 1:** 寫移植的測試，另加：
  - `a_result_after_editing_is_discarded_and_the_button_recovers`（Review Focus 2）：開始測試後改 API 金鑰，再送 `TestDone`：不顯示結果、`test_passed()` 為 false、按鈕可按。換服務商也一樣，模型清單的 `ModelsDone` 也一樣。
  - `a_successful_test_shows_time_source_and_translation`：三行文字都在，1250ms 顯示 `1.3`。
  - `format_seconds_rounds_half_up`：1240ms→`"1.2"`、1250ms→`"1.3"`、49ms→`"0.0"`、50ms→`"0.1"`、61049ms→`"61.0"`。
  - `test_errors_make_urls_clickable`：錯誤訊息裡的網址是 Hyperlink。
  - `the_api_key_is_masked_until_revealed`
- [ ] **Step 2:** 跑測試，預期 FAIL。
- [ ] **Step 3:** 實作，並改 i18n 三個 JSON；若有 Python 測試斷言舊的「重新整理」文字，跟著改。
- [ ] **Step 4:** 跑三項 cargo 檢查以及 ruff／pytest，全過。
- [ ] **Step 5:** Commit：`feat(rust): add the service form with timed connection tests`

---

### Task 8: 服務清單、服務商選擇與對話框

**Files:**
- Create: `rust/src/ui/settings/service_list.rs`、`rust/src/ui/settings/provider_picker.rs`
- Test: `rust/tests/service_list.rs`（新）
- 移植：
  - `tests/ui/settings/test_service_list.py` 全部；
  - `tests/ui/settings/test_provider_picker.py` 全部，Tk 專屬的 geometry／grab 測試改測 egui 的 modal 行為。

**Interfaces:**
- Consumes：Task 1 的服務函式，Task 6 的 widgets 與 `SettingsEnv`，Task 7 的 `ServiceForm`。
- Produces（`crate::ui::settings::service_list`）：
  - `pub struct ServiceDraft { pub services: Vec<Value>, pub default: Option<String>, pub slots: [Option<String>; 3] }`（順序同 `ServiceSlot::ALL`），純資料、可單獨測：
    - `from_config(cfg)`、`write_into(&self, cfg: &mut Config)`
    - `card_label(&self, service: &Value) -> String`：名稱後接「預設」與各用途標記，以 ` · ` 分隔。
    - `can_delete(&self) -> bool`
    - `apply_dialog_result(&mut self, service: Value)`：名稱過 `unique_name`；清單空時接手預設，並記 `[settings] first service added; default={id}`。
    - `delete(&mut self, id: &str)`：指到它的用途改回跟隨預設，被刪的是預設就交給第一筆。log 同 Python。
    - `set_default(&mut self, id)`、`set_slot(&mut self, slot, id: Option<String>)`
  - `pub struct ServicePane`：
    - `pub fn new(draft: ServiceDraft) -> Self`
    - `pub fn draft(&self) -> &ServiceDraft`
    - `pub fn apply(&mut self, event: &SettingsEvent)`
    - `pub fn ui(&mut self, ui: &mut Ui, env: &dyn SettingsEnv, target_language: &str)`
    - 版面：預設服務下拉（含服務商圖示）；可收合的「各用途指定」，有任一用途有指定時預設展開；「我的服務」卡片；〔＋ 新增翻譯服務〕。只剩一筆時刪除鈕變灰，下方加 `service.keep_one` 說明。
  - 對話框都用 `egui::Modal`，Esc 取消：
    - 服務表單：確定時 `validate_service` 不過，就在表單底部顯示 `callout`，不關閉；
    - 服務商選擇：`provider_cards(ui, CardLayout::List)`；精靈用 `CardLayout::Grid`（Task 10）；
    - 刪除確認：`dialog.confirm_title`，問句依「有沒有用途指到它」選 `service.confirm_delete` 或 `service.confirm_delete_in_use`，按鈕是〔取消〕〔刪除〕（`DANGER`）。
  - 新增流程：先選服務商，取消就什麼都不開；選了才開表單對話框。
  - 存檔 log：`[settings] service saved (provider=..., has_key=True|False)`。
- Produces（`crate::ui::settings::provider_picker`）：`pub fn provider_cards(ui: &mut Ui, layout: CardLayout) -> Option<&'static str>`，`pub enum CardLayout { List, Grid }`。服務商圖示用 `runtime::resources::provider_icon`，載入方式同 `frame.rs` 的 `load_icon`。

- [ ] **Step 1:** 寫移植的測試，另加：
  - `deleting_needs_confirmation_and_cancel_changes_nothing`
  - `escape_closes_only_the_top_dialog`：選服務商的對話框疊在服務表單上時，Esc 只關選擇。
  - `cards_are_reachable_and_activatable_by_keyboard`
- [ ] **Step 2:** 跑測試，預期 FAIL。
- [ ] **Step 3:** 實作。
- [ ] **Step 4:** 跑三項 cargo 檢查，全過。
- [ ] **Step 5:** Commit：`feat(rust): add the service list, provider picker and service dialogs`

---

### Task 9: 設定視窗內容

**Files:**
- Create: `rust/src/ui/settings/window.rs`
- Test: `rust/tests/settings_window.rs`（新）；語言預覽的測試另放 `rust/tests/settings_language.rs`（新，自成一個執行檔）。
- 移植 `tests/ui/settings/test_window.py` 全部：
  - Tk 專屬的測試（topmost 釋放、`update()` 重繪順序）移到 Task 11；
  - overlay 的 `set_limits`／`set_alpha` 測試已在第 1 段移植過的就略過。

**Interfaces:**
- Consumes：Task 1、5、6、7、8。
- Produces（`crate::ui::settings::window`）：
  - `pub enum Page { Basic, Services, Advanced, About }`
  - `pub enum SettingsAction`：
    - `PreviewAlpha(f32)`
    - `PreviewLanguage`：視窗已呼叫 `set_language`，App 只需 relabel。
    - `UpdateFound(Release)`
    - `Save(Config)`：完整的新設定，已含 `ui_language`。
    - `Cancel`
  - `pub struct SettingsWindow`：
    - `pub fn new(cfg: &Config) -> Self`：從 cfg 取一份 draft，記下開窗時的語言與透明度。
    - `pub fn page(&self) -> Page`、`pub fn set_page(&mut self, page: Page)`
    - `pub fn apply(&mut self, event: &SettingsEvent)`
    - `pub fn ui(&mut self, ui: &mut Ui, env: &dyn SettingsEnv) -> Vec<SettingsAction>`
    - `pub fn cancel(&mut self) -> Vec<SettingsAction>`：還原預覽中的透明度與語言，給視窗 ✕／Alt+F4 用。
- 各頁照設計稿：
  - **基本**：
    - 卡片一：介面語言（說明列有譯者掛名，以及 `credit.help_translate` 連到 `CROWDIN_URL`）、翻譯目標語言（可輸入下拉，選項 `COMMON_LANGUAGES` 同 Python，說明 `hint.language`）。
    - 卡片二：預設翻譯服務（圖示＋draft 裡預設服務的名稱，沒有就顯示「—」）＋〔管理〕，按下切到翻譯服務頁。
    - 卡片三：輸入框熱鍵（按鍵帽＋〔更改〕）、`field.auto_input` 開關。
    - 卡片四：框選熱鍵。
    - 卡片五：`field.paste_hotkey` 開關。
  - **翻譯服務**：`ServicePane`。
  - **進階**：
    - 卡片一：四個數值欄，說明用 `settings.range_hint`。
    - 卡片二：透明度滑桿（0.3–1.0，顯示兩位小數），拖動就 `PreviewAlpha`。
    - 卡片三：`field.translate_system` 開關。
  - **關於**：
    - 頭卡：圖示 48、名稱、`about.version` ＋ `v{版本}`、〔檢查更新〕與結果。最新是 `update.latest`；失敗是 `update.failed`；有新版是整行可點的 `update.available`，連到 release 網址，同時送出 `UpdateFound`。
    - 連結卡：專案、開發者（`GoneTone`）、協助翻譯、回報問題。
    - 紀錄卡：`about.logs` 路徑（`app_dir()`）＋〔開啟資料夾〕＋說明；`env.has_cache()` 時加譯文快取列：〔清除〕呼叫 `clear_cache()`，旁邊顯示 `about.cache_cleared`。
  - 每一頁的內容區各自可捲動（細捲軸同 overlay 的樣式），底部按鈕列固定不捲。
  - 側欄底部顯示 `v{CARGO_PKG_VERSION}`；底部按鈕列〔取消〕〔儲存〕。
- 熱鍵錄製：
  - 按〔更改〕後那一格變成 `button.press_key`（`ACCENT` 外框），呼叫 `env.record_hotkey`。
  - 收到 `HotkeyRecorded` 時，`esc` 或空字串不改值，其他寫入 draft。
  - 換頁、關窗、開始錄另一格時呼叫 `env.cancel_recording()`。
  - 錄製失敗記 `[settings] hotkey capture failed: ...`。
- 儲存：
  - 依序收集錯誤：`_service_errors`（同 Python：預設服務與三個用途實際指到的服務各自驗證，去重）、兩把熱鍵相同、目標語言空白、進階數值。
  - 有錯就在底部按鈕列左邊顯示 `callout`，標題 `dialog.incomplete_title`，每個錯誤一行，不送 `Save`。
  - 沒錯時送 `Save(cfg)`，`cfg["ui_language"] = current_language()`。
  - 記 `[settings] ...` 的 log 同 Python。
- 換介面語言：記 `[ui] settings previewing language {code}`，呼叫 `set_language`，送 `PreviewLanguage`；draft 與目前頁面自然保留。
- 取消：
  - 透明度變過就送 `PreviewAlpha(開窗值)`；
  - 語言變過就 `set_language(開窗值)`、送 `PreviewLanguage`，並記 `[ui] settings language preview reverted to {code}`；
  - 最後送 `Cancel`。

- [ ] **Step 1:** 寫移植的測試，另加：
  - `save_is_blocked_with_a_callout_listing_each_problem`：缺 API 金鑰加上兩把熱鍵相同時，提示列兩行，沒有 `Save`。
  - `closing_while_recording_cancels_it`（Review Focus 4）：開始錄製後呼叫 `cancel()`，fake env 記到 `cancel_recording`。
  - `manage_button_switches_to_the_services_page`
  - `alpha_slider_previews_and_cancel_restores`
  - `settings_language.rs::language_preview_keeps_the_draft_and_cancel_reverts`：改目標語言後換介面語言，草稿還在；取消後語言還原。
- [ ] **Step 2:** 跑測試，預期 FAIL。
- [ ] **Step 3:** 實作。
- [ ] **Step 4:** 跑三項 cargo 檢查，全過。
- [ ] **Step 5:** Commit：`feat(rust): add the settings window content`

---

### Task 10: 首次設定精靈內容

**Files:**
- Create: `rust/src/ui/settings/wizard.rs`
- Test: `rust/tests/wizard.rs`（新）；換語言的測試放 `rust/tests/wizard_language.rs`（新，自成一個執行檔）。
- 移植 `tests/ui/settings/test_wizard.py` 全部：
  - 「重建」的語意改成：換語言後同一個精靈繼續用，畫面換成新語言，狀態照 Python 重建後的結果。
  - `bootstrap_language` 的三個測試已在第 1 段移植過就略過。

**Interfaces:**
- Consumes：Task 6、7、8 的元件（服務商卡片用 Task 8 的 `provider_cards`）。
- Produces（`crate::ui::settings::wizard`）：
  - `pub enum Step { Language, Service, Prefs }`
  - `pub fn can_advance(step: Step, test_passed: bool, api_errors: &[String]) -> bool`
  - `pub enum WizardAction { PreviewLanguage, Finish(Config), Cancel }`
  - `pub struct Wizard`：
    - `pub fn new(cfg: Config) -> Self`：有服務可編輯（預設服務，沒有就第一筆）時直接建表單，否則第二步先出服務商卡片。
    - `pub fn step(&self) -> Step`
    - `pub fn apply(&mut self, event: &SettingsEvent)`
    - `pub fn ui(&mut self, ui: &mut Ui, env: &dyn SettingsEnv) -> Option<WizardAction>`
- 版面照設計稿：
  - 上方步驟條：三步各有編號圓點與名稱 `wizard.step.*`；走過的是 `ACCENT` 底＋勾，目前的是 `ACCENT` 外框＋粗體，之間是連接線。
  - 中間內容可捲動；底部〔上一步〕（第一步停用）與〔下一步〕／〔完成〕。
  - 第一步：圖示 64、`app.name`、介面語言下拉、`wizard.language_hint`、譯者掛名與協助翻譯連結。
  - 第二步：
    - `wizard.intro`；
    - 還沒選服務商時：`wizard.pick_provider` ＋ 2×2 服務商卡片；
    - 選了之後：`wizard.intro_form` ＋服務表單＋右下「略過測試」連結。
  - 第三步：`wizard.target_language` ＋可輸入下拉＋`hint.language`，以及兩張熱鍵卡（同設定的基本頁，輸入框熱鍵卡含 `field.auto_input` 開關）。
- 規則照 Python：
  - 第二步要測試成功或按過「略過測試」才能下一步；改任何 API 欄位就作廢「略過」。
  - 換語言時：
    - 目標語言還是舊介面語言的自稱，就跟著換；
    - 正在編輯的服務名稱還是自動名，就換成新語言的短名並去重；
    - 測試結果作廢；
    - 記 `[ui] wizard restarting with language {code}`。
  - 選服務商記 `[ui] wizard provider picked: {key}`。
  - 完成時：兩把熱鍵相同就顯示 `callout`，不完成；否則把服務依 id 併回清單（不可刪掉其他服務）並設為預設，名稱去重，送 `Finish(cfg)`。
  - 精靈內服務表單測試用的目標語言：`language_name(current_language())`。

- [ ] **Step 1:** 寫移植的測試，另加：
  - `next_is_disabled_until_tested_or_skipped`
  - `finish_keeps_other_services_and_their_keys`
- [ ] **Step 2:** 跑測試，預期 FAIL。
- [ ] **Step 3:** 實作。
- [ ] **Step 4:** 跑三項 cargo 檢查，全過。
- [ ] **Step 5:** Commit：`feat(rust): add the first-run wizard content`

---

### Task 11: 在 App 裡托管設定視窗與精靈

**Files:**
- Create: `rust/src/ui/settings_host.rs`
- Modify: `rust/src/ui/app.rs`
  - ⚙ 開設定；
  - 處理 `UiEvent::Settings`；
  - `viewport_hwnds` 納入新視窗；
  - 精靈模式下根視窗保持隱藏。
- Test: `rust/tests/settings_host.rs`（新）、`rust/tests/ui_probe.rs`（新增 `#[ignore]` 的視窗檢查）

**Interfaces:**
- Consumes：Task 9 的 `SettingsWindow`／`SettingsAction`、Task 10 的 `Wizard`／`WizardAction`、Task 6 的 `SettingsEnv`。
- Produces（`crate::ui::settings_host`）：
  - `pub fn initial_size(want: (f32, f32), work_area_points: (f32, f32)) -> (f32, f32)`：每邊各留 24 的邊距，夾在工作區內。
  - `pub struct SettingsHost`：
    - `pub fn new(env: Box<dyn SettingsEnv>) -> Self`
    - `pub fn open(&mut self, cfg: &Config)`：已開著就 `force_foreground`。
    - `pub fn is_open(&self) -> bool`
    - `pub fn hwnd(&self) -> isize`
    - `pub fn apply(&mut self, event: SettingsEvent)`
    - `pub fn show(&mut self, ctx: &egui::Context, known: &[isize]) -> Vec<SettingsAction>`
  - `pub struct WizardHost`：同樣的形狀，`show` 回傳 `Option<WizardAction>`。
- Produces（`App`）：
  - `pub fn with_settings(mut self, host: SettingsHost, on_save: Box<dyn FnMut(Config) -> (String, Option<String>)>) -> Self`：`on_save` 回傳實際生效的兩把熱鍵。
  - `pub fn with_first_run(mut self, wizard: WizardHost, start: Box<dyn FnOnce(&mut App, Config)>) -> Self`：根視窗一開始就藏著。
  - `pub fn apply_settings(&mut self, ctx: &egui::Context, cfg: &Config, region_hotkey: Option<String>)`：overlay 端的套用，順序照 Python 的 `apply_settings` 後半：
    1. `set_region_hotkey`
    2. 框選卡片的透明度
    3. `set_limits`
    4. `frame.set_alpha`
    5. 介面語言和上次不同時 `relabel`
  - 把現有的 `with_input`、`with_region`、`with_bubble_position`、`with_on_close` 各補一個 `&mut self` 版本（`set_input` 等）；`with_*` 改成呼叫它們。精靈完成後的接線要用這些。
- 視窗：
  - ViewportBuilder：
    - 標題：`settings.title` 或 `wizard.title`，`app` 帶 `app_name()`；語言改了就送 `ViewportCommand::Title`。
    - 大小：`initial_size`，預設值見 Global Constraints；工作區取游標所在的螢幕，置中。
    - 有系統標題列、可縮放、`with_taskbar(true)`、圖示用 `icon_png(48)`。
    - 設定視窗建立時 `with_always_on_top()`，下一幀送 `WindowLevel::Normal`，同 Python「只在第一次顯示時置頂」；精靈不置頂。
  - 找到 HWND 的那一幀：`dark_caption`、`force_foreground`；失敗記 `[ui] settings window could not take the foreground (hwnd=..)`。
  - 視窗關閉要求（✕、Alt+F4）：設定視窗走 `cancel()`，精靈視為 `Cancel`。
  - 內容整塊不透明：根視窗的 `clear_color` 是透明的，要自己鋪 `WINDOW_BG`。
- App 對 `SettingsAction` 的處理：
  - `PreviewAlpha` → `frame.set_alpha`
  - `PreviewLanguage` → `relabel`
  - `UpdateFound` → `banners.set_update`
  - `Save(cfg)` → 呼叫 `on_save(cfg)`，再 `apply_settings`，最後關窗。
  - `Cancel` → 關窗。
- App 對 `WizardAction` 的處理：
  - `PreviewLanguage` → `relabel`
  - `Finish(cfg)` → 關精靈、`start(self, cfg)`、顯示根視窗。
  - `Cancel` → 記 `[app] wizard cancelled, exiting`，關根視窗；沒有 `on_close`，程序直接結束。

- [ ] **Step 1:** 寫測試：
  - `initial_size_fits_the_work_area`（Review Focus 1）：
    - 工作區 910×485 時，設定視窗得 862×437、精靈得 600×437；
    - 工作區 1920×1040 時維持 840×660 與 600×760。
  - `settings_host.rs`，用 kittest 跑 App：
    - `gear_opens_settings_and_save_applies_to_the_overlay`：fake env、fake `on_save`；按 ⚙，在視窗內容按儲存後，`on_save` 收到 cfg、overlay 透明度與上限更新、視窗關閉。
    - `cancel_restores_alpha_and_language`
    - `wizard_mode_keeps_the_root_hidden_until_finish`：根視窗的 `Visible(false)` 在第一幀送出；`Finish` 後 `start` 被呼叫一次，根視窗變成可見。
    - `wizard_cancel_closes_the_root`
  - `ui_probe.rs::settings_window_has_a_dark_caption_and_a_taskbar_button`（`#[ignore]`）：在螢幕外開設定視窗。
    - 檢查有 `WS_CAPTION`、有 `WS_EX_APPWINDOW` 或沒有 owner；
    - WGC 擷取標題列取樣顏色接近 `#101018`；
    - 內容區取樣是 `WINDOW_BG`，不透明。
- [ ] **Step 2:** 跑測試，預期 FAIL。
- [ ] **Step 3:** 實作。
- [ ] **Step 4:** 跑三項 cargo 檢查，全過；`ui_probe` 的新測試手動跑一次並附輸出。
- [ ] **Step 5:** Commit：`feat(rust): host the settings window and first-run wizard in the app`

---

### Task 12: 主程式接線

**Files:**
- Modify: `rust/src/main.rs`、`rust/src/app_main.rs`
- Modify: `docs/superpowers/plans/2026-10-17-rust-rewrite-followups.md`：第 4 段各項標完成，新增「第 4 段」完成紀錄與可保留的小問題。
- Test: `rust/tests/app_main.rs`

**Interfaces:**
- Consumes：前面所有 task。
- Produces（`app_main`）：
  - `build_app` 改成接收 `dispatcher: SharedDispatcher` 與 `keyboard: Option<KeyboardHook>`，不再自己裝 hook。
  - `Running` 新增 `pub hotkeys: (Registration, Option<Registration>)` 與 `pub dispatcher: SharedDispatcher`。
  - `install_input` 拆成兩個：
    - `pub fn register_hotkeys(settings, ui, dispatcher, wiring, mask) -> (Registration, Option<Registration>)`：只註冊兩把熱鍵，實際生效的寫回 settings。
    - 掛 Ctrl+V 攔截器那段，只在 `build_app` 跑一次。
  - `pub fn apply_settings(running: &mut Running, settings: &Arc<RwLock<Config>>, new_cfg: Config, ui: &UiPoster) -> (String, Option<String>)`，照 Python `apply_settings` 前半：
    1. 寫進共用 cfg；
    2. `save_config`，失敗記 log；
    3. `reconfigure_translation`；
    4. 解除舊熱鍵、重新註冊（`Registration::unregister`，待辦文件第 4 段最後一項）；
    5. 記 `[settings] applied; {config_summary}`；
    6. 回傳實際生效的熱鍵（框選停用時為 `None`）。
- `main.rs` 的新流程：
  1. 載入設定、定語言、搶單一實例；
  2. 開 messages.log、`register_models`、建 tokio runtime；
  3. 建 `SharedDispatcher` 並裝鍵盤 hook（精靈的熱鍵錄製也要用）；
  4. 設定完整：照舊接線，`log_startup_summary` 移到這裡（待辦文件第 4 段：被擋下的第二份實例不再寫 startup）；
  5. 設定不完整：記 `[app] config incomplete, launching first-run wizard`，以 `with_first_run` 開精靈。`start` 收到 cfg 後依序：
     - 寫進共用 cfg；
     - `save_config`；
     - 記 `[app] wizard completed, config saved`；
     - 跑與步驟 4 相同的接線函式，抽成 `fn start_main(app: &mut App, ...)`，兩條路共用。
  6. 接線時一併：
     - 建 `SettingsHost`（`RealEnv` 帶 cache）；
     - `with_settings` 的 `on_save` 呼叫 `apply_settings`；
     - 啟動 `schedule_update_checks`，`found` 投遞 `UiEvent::UpdateFound`，`stop` 用 `Running::stop`；
     - 記 `[app] running; ...` 那行。
- 關閉流程不變；更新檢查的 task 不等它（Python 也不等）。

- [ ] **Step 1:** 寫測試：
  - `apply_settings_saves_rebinds_and_reregisters`：用 tempdir 的 config 路徑、`Dispatcher` 不裝 hook。
    - 改了輸入框熱鍵後，`handler_count` 不變；
    - 舊熱鍵的組合按下去不觸發、新的會觸發（callback 用計數器）；
    - 設定檔寫入新值；
    - 快取指紋更新。
  - `region_hotkey_equal_to_the_input_hotkey_is_not_registered`
- [ ] **Step 2:** 跑測試，預期 FAIL。
- [ ] **Step 3:** 實作。
- [ ] **Step 4:** 跑三項 cargo 檢查，全過。
- [ ] **Step 5:** 建 release，複製到 `rust\target\release\_test\`：先確認沒有從那個資料夾執行的 `Wizard101ChatTranslator.exe`，只覆蓋 exe，不碰其他檔案。
- [ ] **Step 6:** Commit：`feat(rust): wire the settings window, first-run wizard and update checks`

---

## 實機驗收清單（第 4 段完成後交給使用者）

用 `rust\target\release\_test\` 的 exe 測。

**首次設定精靈**
1. 把 `_test\config.json` 改名備份後啟動：
   - 只出現精靈，overlay 沒出現；
   - 精靈有工作列按鈕、標題列是深色。
2. 第一步換介面語言：精靈立刻換成新語言（含標題列）；第二步的目標語言跟著換。
3. 第二步：
   - 選服務商後出現表單；
   - 沒測試時〔下一步〕是灰的，測試成功或按「略過測試」後才能按；
   - 改了 API 金鑰後「略過」作廢。
4. 測試連線成功時顯示三行：耗時（例如 1.3 秒）、原文、譯文。
5. 第三步把兩把熱鍵設成相同時，〔完成〕被擋下並顯示提示。
6. 完成後 overlay 出現，設定檔寫入；中途按 ✕ 則程式結束、不寫設定檔。
7. 精靈裡錄熱鍵：〔更改〕後按 Ctrl+Space 得 `ctrl+space`，按 Esc 不改值。

**設定視窗**
8. 按 overlay 的 ⚙：設定視窗出現在最前面，可以直接打字；被 overlay 擋住時不會一直壓在其他程式上。
9. 基本頁：
   - 換介面語言時 overlay 立刻換語言；取消後還原；
   - 〔管理〕跳到翻譯服務頁。
10. 翻譯服務頁：
    - 新增：先選服務商，取消就不開表單；
    - 編輯、刪除（有確認）、指定各用途；
    - 刪掉被指定的服務後，那個用途回到「跟隨預設」；
    - 只剩一組時不能刪。
11. 服務表單：
    - 〔更新清單〕抓到模型後，輸入關鍵字會篩選；
    - 抓不到清單時提示自行輸入；
    - 錯誤訊息裡的網址可以點。
12. 進階頁：
    - 拖透明度滑桿時 overlay 即時變化，取消後還原；
    - 數值欄打字母後按儲存，被擋下並提示。
13. 關於頁：
    - 〔檢查更新〕顯示「已是最新版本」；
    - 〔開啟資料夾〕開到 exe 旁的資料夾；
    - 〔清除〕顯示清掉幾筆；
    - 四個連結都能開。
14. 儲存後立即生效，不必重啟：
    - 新熱鍵可用、舊熱鍵失效；
    - 改服務後收訊用新服務翻譯；
    - 淡出秒數與訊息上限套用到 overlay。
15. 鍵盤：用 Tab 走過每個控制項都看得到焦點框，Space／Enter 能操作開關、導覽與卡片。
16. 小螢幕或高縮放（例如 1366×768、150%）：視窗不超出螢幕，內容可捲動，儲存鈕看得到。

**更新檢查**
17. 啟動後 `app.log` 有 `[update] checking latest release`；斷網啟動只記 `check failed`，不跳錯誤。
