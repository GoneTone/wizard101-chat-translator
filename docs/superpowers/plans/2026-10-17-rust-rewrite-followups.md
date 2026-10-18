# Rust 重寫：第 1 段留給後續各段的待辦

第 1 段（`2026-10-17-rust-rewrite-phase1.md`）已實作完成，使用者實機驗收通過（2026-10-17）。以下是執行過程中延到後段處理的事項，寫各段計畫時要納入。

## 第 1 段尚未驗收的項目

- **斷網時的離線橫幅**：使用者暫時沒測。行為：暫時斷網後，等新訊息進來，overlay 出現離線橫幅；恢復網路後自動補翻。第 2 段驗收時一併確認。

## 第 2 段

原列的兩項（主控台 Ctrl+C 走關閉流程、新 viewport 不靠標題找 HWND）已在第 2 段完成。第 2 段實作完成、待使用者實機驗收（`2026-10-17-rust-rewrite-phase2.md` 檔尾清單）。

## 第 3 段

第 3 段（框選翻譯）已實作完成，使用者 2026-10-18 實機驗收通過（`2026-10-17-rust-rewrite-phase3.md` 檔尾清單）。原列三項（框選快捷鍵佔位、`selecting` 旗標、等觸發鍵放開）都已完成。

- **OCR 截圖驗證**（spec 規定）：2026-10-18 用使用者的 5 張截圖跑 `tools/ocr_compare` 新舊對照：2 張新參數讀對（`It ems`→`Items`、`Welcome to i the`→`Welcome to the`），3 張（含中英混排的任務卡與 NPC 對話）相同，沒有任何一張變差。Rust 版辨識速度約為 Python 版的 5～10 倍。之後若有更多截圖，用同一套工具再跑。
- **刻意與 Python 不同**：調整框時重新擷取或裁切失敗，會撤銷進行中的請求並進入新 session，晚到的結果不再蓋掉錯誤卡片（使用者 2026-10-18 要求，Python 版仍有此問題）。
- **只能實機確認**：WGC 擷取的顏色、DWM 外框偏移與高 DPI、截整顆螢幕時 CAPTUREBLT 是否讓游標閃爍、混合 DPI 下選取畫面是否蓋滿螢幕、框的拖曳手感、卡片邊框在 125%／150% 是否有接縫。
- **可保留的小問題**（附理由）：
  - `HostedWindow::place` 比對的是上次設定的矩形，不是視窗實際矩形；混合 DPI 下 winit 改了大小時不會再拉正。實機遇到再改成與 `window_rect` 比對。
  - OCR 模型目錄以雜湊命名，換模型會留下舊目錄（約 31 MB）；模型目前不會變。
  - 選取畫面「找不到視窗、未取得焦點」那行 log 沒有自動化斷言（lib 單元測試沒有 log 擷取）。
  - 視窗查找測試有一半是套套邏輯，有實質意義的部分（`viewport_hwnds` 列出全部六個角色）已涵蓋。

## 第 4 段

第 4 段（設定視窗、首次設定精靈、服務管理、更新檢查）已實作完成，使用者 2026-10-18 實機驗收通過（`2026-10-18-rust-rewrite-phase4.md` 檔尾清單；驗收中修正的問題見 git 歷史 a0da784..fabf62f）。原列各項的處理：

- 設定視窗裡顯示 API 錯誤的地方，網址都能點：測試連線結果、模型清單狀態一律經 `ui::settings::widgets` 走 `ui::richtext::show`。
- `generate_and_store`、`list_models`、`test_translate` 已移植（`translation/translator.rs`）。
- `offer_update` 改用 SemVer `is_newer` 跟已關閉的版本比較；自動更新檢查（啟動時與每 60 分鐘）以 `UiEvent::UpdateFound` 接到 `offer_update`。
- 套用設定時先寫共用設定，`reconfigure_translation` 再重新綁定系統訊息快取的指紋。
- 啟動摘要改在接線時（精靈完成之後）才記；被擋下的第二份實例不再寫 startup。
- 無邊框視窗呼叫 `remove_system_frame` 的規則照舊；設定視窗與精靈是有系統標題列的一般視窗，不適用。
- redirection bitmap：設定視窗與精靈都用 egui 繪圖，沒有 GDI。
- 泡泡建立的第一幀可能閃一下舊外框：沒處理，仍待實機確認。
- **鍵盤 raw input 與低階 hook**（實機驗收時錄熱鍵沒反應的根因）：winit 建立事件迴圈時會為滑鼠與鍵盤註冊 raw input。鍵盤 raw input 註冊著的時候，只要前景是我們自己的視窗，Windows 就不呼叫我們的 `WH_KEYBOARD_LL` hook。現在啟動時以 `win32::stop_raw_keyboard_input` 取消鍵盤的註冊（滑鼠保留；egui 的鍵盤事件來自一般的 `WM_KEYDOWN`，不受影響）。第 2 段「等觸發鍵放開才讓輸入框搶前景」的 `defer_for_release`，繞的是同一個原因（前景換成我們的視窗後，觸發鍵的放開不會進 hook）；它沒有害處，先保留，之後確認不需要再拿掉。

- **刻意與 Python 不同**：
  - 測試連線成功時多顯示耗時與原文，共三行；模型清單按鈕在繁中改叫「更新清單」（使用者 2026-10-18 核可）。
  - 改了 API 欄位後，已顯示的「連線成功」會清掉（避免留著過期的綠字）；失敗訊息照 Python 保留。
  - 首次設定精靈與 overlay 在同一個 eframe 事件迴圈裡跑（winit 一個程序只能開一次），精靈期間根視窗建立時就藏著。
  - 套用設定時新的熱鍵先註冊、再解除舊的（`Registration` 只能整個交出去解除）；結果與 Python 相同，只是 log 先後不同。
  - Python 的 `[settings] hotkey capture failed` 不會出現：Rust 的錄製器沒有失敗路徑。
- **只能實機確認**：精靈與設定視窗的深色標題列、工作列按鈕與搶前景；小螢幕與高縮放下的初始大小；`main.rs` 的 `start_main` 接線（兩條路：設定完整、精靈完成）沒有自動化測試，依實機驗收清單確認。
- **可保留的小問題**（附理由）：
  - **重複程式碼**：
    - `SettingsHost` 與 `WizardHost` 幾乎相同（可改成泛型 host）；
    - 設定視窗的預設值 helper 與 `Settings::from_config` 重複；
    - `updater::http_client` 與 `clients::http_client` 同名但設定不同（呼叫端都帶模組前綴）。
  - **檔案偏大**：`ui/app.rs` 約 1550 行（`native_options`／`run*` 可搬出去）、`service_list.rs` 約 750 行。
  - **與 Python 的小差異**：
    - `parse_int` 不接受 Tcl 的 `0x10` 寫法；`unique_name` 遇到非字串 id 的處理不同（`sanitize` 已正規化，實際不會發生）；
    - 錄製器遇到沒有名稱的鍵不出聲（Python 會拋例外）；錄製開始前就按著的修飾鍵，放開時不算進組合（例如按著 Ctrl 點〔更改〕再按 K，得 `k`）。
  - **邊界情況**：
    - `schedule_update_checks` 的 `found` 回呼 panic 時，排程 task 會無聲結束（回呼只投遞事件）；
    - `number_field` 在 min>max 時 clamp 會 panic（呼叫端都給合法範圍）；
    - 疊開服務商選擇的同一幀按 Esc 會連表單一起關（實際按不出來）；
    - 混合 DPI 下設定視窗置中沒有再以實體像素擺正；
    - 套用設定後重新註冊的熱鍵排在 Ctrl+V 攔截器之後，鍵入進行中含 `v` 的熱鍵會被貼上攔截器先吞掉（Python 相同）；
    - 精靈中途取消時鍵盤 hook 不另外卸除，程序隨即結束、由系統移除。
  - **測試覆蓋缺口**：
    - combo 的滑鼠操作、`icon_button` 提示框的配色、精靈完成時同步 `ui_language` 與 `with_first_run` 的 log；
    - 範例過期的測試沒有分別釘住 `finish()` 與 `set_examples()` 兩道防線；
    - 錄製器測試讀真實的 `GetKeyState`（Caps Lock 開著時結果與機器有關）；
    - `up_to_date_log_lock` 是約定，新增不加鎖的 `Ok(None)` 測試會打破 +1 的計數。
  - **外觀細節**：精靈的測試列放在表單卡片內（設計稿在卡片下方）、下拉高 32（設計稿 36）、48px 圖示放大到 64；提示框顯示時每幀重算 `collect()`；設定頁連結的 log 寫成「from message」。
  - `parse_advanced` 每次都建一份 `default_config()`：只在儲存與提示框顯示時跑，成本可忽略。

## 第 5 段

- 把 `rust/` 搬到 repo 根目錄時，使用者的測試資料夾 `rust/target/release/_test`（不進版控，裡面有使用者的 config 和 log）要一起搬。搬之前先問使用者。
- `rust/tests/real_fingerprint.rs` 的文件寫「只讀副本」，但 `load_config` 在正規化有變動時會寫回檔案。要改文件，或改成測試自己先複製再讀。
- `fonts_dir` 遇到空字串的 `WINDIR` 時沒有過濾掉。
- **鎖定 Rust 版本**（使用者 2026-10-18 決定放在這段）：
  - 現況：沒有 `rust-toolchain.toml`，`Cargo.toml` 沒寫 `rust-version`，CI 用 `dtolnay/rust-toolchain@stable`；本機是 1.99.1（`f16e7ce` 已配合 clippy 1.99 的新規則修過一次）。
  - 加 `rust-toolchain.toml` 釘在當時使用的版本，本機與 CI 一致，新版 clippy 規則不會讓 `-D warnings` 無預警失敗。
  - `Cargo.toml` 加 `rust-version`（程式用到 let chains，至少 1.88）。
  - CI 改讀 toolchain 檔，不再固定抓 stable。
- **`best_match` 對同語言多地區的語言檔沒有區分**（Python 對照稽核發現的已知缺口）：
  - 現況：同語言有多份語言檔時，Rust 取第一個，Python 以 CLDR 取地區最接近者（pt-MZ 在 Python 得 pt-PT、在 Rust 得 pt-BR）。
  - 觸發條件：同一語言的第二份地區語言檔（例如 pt-BR 與 pt-PT）達到覆蓋率門檻、進入語言選單。目前每個語言只有一份，沒有影響。
  - 處理方向：同語言內改依地區取最接近者，例如用 CLDR 的 parent locales 資料；並補 golden 案例。

## 合併前清掉的小問題（第 1 段審查列為可延後）

- **log 文字與 Python 的細節差異**：多數只是診斷用字串，例如：
  - `null` 與 `None` 的寫法；
  - 截斷樣本用 `{:?}` 而非 `repr`；
  - hookstate 壞檔的例外名稱；
  - auto-follow／relaid-out 的欄位。
- **測試覆蓋缺口**：
  - exporter 的 exit 2；
  - TimestampedWriter 的邊界情況；
  - 3xx `/v1/models`；
  - 黏底時最後一則譯文長高；
  - 左緣縮放帶的拖曳；
  - 廣播訊息 msg_id 跨分頁更新；
  - 分頁列未選取的顏色；
  - palette 只驗了 2 種顏色。
- **語言比對**：目前只用主語言標籤比對。新語系的完成度超過 60% 時，要補 langcodes 距離比對的守門測試，例如 nb／nn、iw、pt 的地區、sr-Latn。
- **訊息清單**：沒有在背景補排版，從沒進過可視範圍的列，捲軸位置只是估計值。
- **其他**：
  - 泡泡未讀數不是粗體，因為沒有載入粗體字型；
  - 混合 DPI 下，拖曳泡泡或 overlay 跨螢幕時，實體尺寸的處理要再實機確認；
  - `config::save_config` 不是原子寫入，與 Python 相同。

## 第 2 段審查列為可保留的小問題（附理由，合併前再看一次）

- **CI 上的鍵盤名稱 fixture**：`keyboard_names` 等 fixture 與鍵盤配置有關，在本機（US 實體配置）產生。push 後若 CI 的 golden `--check` 或 `keys_golden` 失敗，是配置差異，要調整 fixture。
- **golden 抓不到「Rust 接受、Python 不認得」的鍵名**：名稱表建法逐行相同，只影響手改成 Python 會拒絕的設定字串。
- **handler panic 時，全域 panic hook 會在鍵盤 hook 裡抓 backtrace**：可能超過 hook 逾時。handler 裡實際可能 panic 的只有資源耗盡時的 `thread::spawn`。
- **鍵盤 hook 一個程序只能裝一次**，第一次失敗後無法重試；Python 也沒有重試。
- **`SendInput` 回傳值沒檢查**：與 Python 的 `keybd_event` 相同。
- **lib 單元測試會把假剪貼簿的 `clipboard open failed: error=5` 印到 stderr**：只是雜訊。
- **設定缺 `hotkey`／`region_hotkey` 時 `region_hotkey_to_register` 會 panic**：與 Python 的 KeyError 等價，`load_config` 一定會補上預設。
- **`scale_at` 不能注入**，沒有單元測試證明用的是錨點所在的螢幕；列入混合 DPI 的實機驗收。
- **overlay 的 HWND 還是 0 時開輸入框，會沒有擁有者而出現工作列按鈕**：overlay 第一幀就拿到 HWND，實際不會發生。
- **`register_hotkeys`／`install_paste` 寫死 `Foreground::real()`**（原 `install_input`）：測試會唯讀查詢真實前景，斷言因此兩個分支都接受。
- **`on_paste_hotkey` 用 `std::thread::spawn`**：資源耗盡時 panic，但會被 handler 的 `catch_unwind` 接住並放行；回傳 `JoinHandle` 是計畫定的簽名。
