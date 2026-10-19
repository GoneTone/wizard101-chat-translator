# Rust 重寫：第 1 段留給後續各段的待辦

第 1 段（`2026-10-17-rust-rewrite-phase1.md`）已實作完成，使用者實機驗收通過（2026-10-17）。以下是執行過程中延到後段處理的事項，寫各段計畫時要納入。

## 第 1 段尚未驗收的項目

- **斷網時的離線橫幅**：使用者暫時沒測。行為：暫時斷網後，等新訊息進來，overlay 出現離線橫幅；恢復網路後自動補翻。第 2 段驗收時一併確認。

## 第 2 段

原列的兩項（主控台 Ctrl+C 走關閉流程、新 viewport 不靠標題找 HWND）已在第 2 段完成。第 2 段實作完成、待使用者實機驗收（`2026-10-17-rust-rewrite-phase2.md` 檔尾清單）。

## 第 3 段

第 3 段（框選翻譯）已實作完成，使用者 2026-10-18 實機驗收通過（`2026-10-17-rust-rewrite-phase3.md` 檔尾清單）。原列三項（框選快捷鍵佔位、`selecting` 旗標、等觸發鍵放開）都已完成。

- **OCR 截圖驗證**（spec 規定）：2026-10-18 用使用者的 5 張截圖跑 `tools/ocr_compare` 與 `examples/ocr_compare.rs` 新舊對照：2 張新參數讀對（`It ems`→`Items`、`Welcome to i the`→`Welcome to the`），3 張（含中英混排的任務卡與 NPC 對話）相同，沒有任何一張變差。Rust 版辨識速度約為 Python 版的 5～10 倍。之後若有更多截圖，原本的新舊對照工具已在第 5 段整個移除（Python 與 Rust 兩側都刪了），之後沒有現成的對照工具可用。
- **刻意與 Python 不同**：調整框時重新擷取或裁切失敗，會撤銷進行中的請求並進入新 session，晚到的結果不再蓋掉錯誤卡片（使用者 2026-10-18 要求，Python 版仍有此問題）。
- **只能實機確認**：WGC 擷取的顏色、DWM 外框偏移與高 DPI、截整顆螢幕時 CAPTUREBLT 是否讓游標閃爍、混合 DPI 下選取畫面是否蓋滿螢幕、框的拖曳手感、卡片邊框在 125%／150% 是否有接縫。
- **已於第 5 段處理**（`c3105b8..9e57646`）：OCR 模型舊目錄在解出新模型時清掉；選取畫面「找不到視窗」那行 log 有斷言了（新增 `logs::capture`）；視窗查找測試改成直接斷言已知清單，不再自己重寫一份比對邏輯。
- **保留**（需要實機才能驗證）：`HostedWindow::place` 比對的是上次設定的矩形，不是視窗實際矩形；混合 DPI 下 winit 改了大小時不會再拉正。實機遇到再改成與 `window_rect` 比對。

## 第 4 段

第 4 段（設定視窗、首次設定精靈、服務管理、更新檢查）已實作完成，使用者 2026-10-18 實機驗收通過（`2026-10-18-rust-rewrite-phase4.md` 檔尾清單；驗收中修正的問題見 git 歷史 a0da784..fabf62f）。原列各項的處理：

- 設定視窗裡顯示 API 錯誤的地方，網址都能點：測試連線結果、模型清單狀態一律經 `ui::settings::widgets` 走 `ui::richtext::show`。
- `generate_and_store`、`list_models`、`test_translate` 已移植（`translation/translator.rs`）。
- `offer_update` 改用 SemVer `is_newer` 跟已關閉的版本比較；自動更新檢查（啟動時與每 60 分鐘）以 `UiEvent::UpdateFound` 接到 `offer_update`。
- 套用設定時先寫共用設定，`reconfigure_translation` 再重新綁定系統訊息快取的指紋。
- 啟動摘要改在接線時（精靈完成之後）才記；被擋下的第二份實例不再寫 startup。
- 無邊框視窗呼叫 `remove_system_frame` 的規則照舊；設定視窗與精靈是有系統標題列的一般視窗，不適用。
- redirection bitmap：設定視窗與精靈都用 egui 繪圖，沒有 GDI。
- 泡泡建立的第一幀可能閃一下舊外框：已由 1e63cbd 解決 —— 泡泡不再是另建的視窗，而是 overlay 根視窗本身縮成泡泡大小。
- **鍵盤 raw input 與低階 hook**（實機驗收時錄熱鍵沒反應的根因）：winit 建立事件迴圈時會為滑鼠與鍵盤註冊 raw input。鍵盤 raw input 註冊著的時候，只要前景是我們自己的視窗，Windows 就不呼叫我們的 `WH_KEYBOARD_LL` hook。現在啟動時以 `win32::stop_raw_keyboard_input` 取消鍵盤的註冊（滑鼠保留；egui 的鍵盤事件來自一般的 `WM_KEYDOWN`，不受影響）。第 2 段「等觸發鍵放開才讓輸入框搶前景」的 `defer_for_release`，繞的是同一個原因（前景換成我們的視窗後，觸發鍵的放開不會進 hook）；它沒有害處，先保留，之後確認不需要再拿掉。

- **刻意與 Python 不同**：
  - 測試連線成功時多顯示耗時與原文，共三行；模型清單按鈕在繁中改叫「更新清單」（使用者 2026-10-18 核可）。
  - 改了 API 欄位後，已顯示的「連線成功」會清掉（避免留著過期的綠字）；失敗訊息照 Python 保留。
  - 首次設定精靈與 overlay 在同一個 eframe 事件迴圈裡跑（winit 一個程序只能開一次），精靈期間根視窗建立時就藏著。
  - 套用設定時新的熱鍵先註冊、再解除舊的（`Registration` 只能整個交出去解除）；結果與 Python 相同，只是 log 先後不同。
  - Python 的 `[settings] hotkey capture failed` 不會出現：Rust 的錄製器沒有失敗路徑。
- **只能實機確認**：精靈與設定視窗的深色標題列、工作列按鈕與搶前景；小螢幕與高縮放下的初始大小；`main.rs` 的 `start_main` 接線（兩條路：設定完整、精靈完成）沒有自動化測試，依實機驗收清單確認。
- **已於第 5 段處理**（`c3105b8..9e57646`）：
  - 設定視窗與精靈共用同一個泛型 host（`Host<C: HostedContent>`）；`ServiceDraft` 搬到 `service_draft.rs`（`service_list.rs` 剩約 590 行）、eframe 啟動碼搬到 `ui/run.rs`（`ui/app.rs` 剩約 1550 行，其中測試佔大半）；`updater::http_client` 改名 `update_client`。
  - `unique_name` 的 id 比對改成同 Python（缺欄位、null 當 `None`，非字串 id 不相等）；`number_field` 在範圍反向時不再 panic；`schedule_update_checks` 的回呼 panic 會記 log 並繼續排程；設定缺 `hotkey` 鍵時不再 panic。
  - 錄製器遇到沒有名稱的鍵會記 log；精靈中途取消時鍵盤 hook 會卸除。
  - 補測試：combo 的滑鼠操作、`icon_button` 提示框配色、精靈完成時同步 `ui_language`、`with_first_run` 的 log、範例過期的兩道防線、錄製器改注入鎖定鍵狀態（不再讀真實的 `GetKeyState`）、`up_to_date_log_lock` 改成單一 helper（新增測試不會再破壞計數）。
  - Esc 與開啟服務商選擇在同一幀：實測只會關掉選擇框、表單留著，已用測試釘住。
- **保留**（需要實機才能驗證）：
  - 混合 DPI 下設定視窗置中沒有再以實體像素擺正。
  - 精靈的測試列放在表單卡片內（設計稿在卡片下方）、下拉高 32（設計稿 36）、48px 圖示放大到 64：這些尺寸已由使用者實機調整並驗收，改動要再實機確認。
  - 錄製開始前就按著的修飾鍵，放開時不算進組合（例如按著 Ctrl 點〔更改〕再按 K，得 `k`；Python 會得 `ctrl+k`）：要改成納入開始前的按鍵狀態，得改掉錄製器「錄製前按下的 Enter／Space 放開不算」的既有設計（半個架構變更）。
- **刻意與 Python 不同／相同**：
  - `parse_int` 把 `010` 讀成十（Tcl 會讀成八），不接受 `0x10` 這類前綴寫法：使用者打的是十進位，照 Tcl 反而違反直覺。
  - 套用設定後重新註冊的熱鍵排在 Ctrl+V 攔截器之後，鍵入進行中含 `v` 的熱鍵會被貼上攔截器先吞掉（Python 相同）。
  - 設定視窗的預設值 helper 與 `Settings::from_config` 看似重複，其實語意不同：設定視窗沿用 Python 的比較方式（布林當 0／1、顯示用文字），`from_config` 取執行期的型別值，不合併。
  - 設定頁連結的 log 寫成「from message」：Python 的 `RichLabel` 也是同一行。
  - `parse_advanced` 與提示框顯示時的 `collect()` 每次都建一份 `default_config()`：只在儲存與提示框顯示時跑，成本可忽略，不處理。

## 第 5 段

- **已於第 5 段 Task 4 處理**：把 `rust/` 搬到 repo 根目錄時，使用者的測試資料夾 `rust/target/release/_test`（不進版控，裡面有使用者的 config 和 log）要一起搬。搬之前先問使用者。
- **已於第 5 段處理**（`c3105b8..9e57646`）：`real_fingerprint.rs` 改成先複製到暫存資料夾再讀，文件寫實；`fonts_dir` 過濾空字串的 `WINDIR`。
- **鎖定 Rust 版本**（已於第 5 段 Task 3／4 處理；`rust-version = "1.95"` 依裁定 R10，因為測試用到 1.95 的 API，下列為當時的原始需求）（使用者 2026-10-18 決定放在這段）：
  - 現況：沒有 `rust-toolchain.toml`，`Cargo.toml` 沒寫 `rust-version`，CI 用 `dtolnay/rust-toolchain@stable`；本機是 1.99.1（`f16e7ce` 已配合 clippy 1.99 的新規則修過一次）。
  - 加 `rust-toolchain.toml` 釘在當時使用的版本，本機與 CI 一致，新版 clippy 規則不會讓 `-D warnings` 無預警失敗。
  - `Cargo.toml` 加 `rust-version`（程式用到 let chains，至少 1.88）。
  - CI 改讀 toolchain 檔，不再固定抓 stable。
- **已處理**（`fix(i18n): pick the closest regional catalog like the Python version`）：**`best_match` 對同語言多地區的語言檔沒有區分**（Python 對照稽核發現的已知缺口）：
  - 原況：同語言有多份語言檔時，Rust 取第一個，Python 以 CLDR 取地區最接近者（pt-MZ 在 Python 得 pt-PT、在 Rust 得 pt-BR）。
  - 做法：`src/i18n/closest.rs` 移植 langcodes 3.5.1 的 `closest_match` 距離規則（語言距離表、Maghreb／美國屬地／港澳／拉丁美洲／美洲地區群），likely subtags 與別名交給 ICU4X 的 `icu_locale`；不分語言、不寫死 pt。
  - 驗證：`tests/golden/i18n_closest_match.json`（現行選單、全部語言檔、加上假想的 es-MX／es-419／en-GB／fr-CA／zh-HK 等地區語言檔）逐筆與 langcodes 相同，產生方式見 `2026-10-18-python-test-parity.md` 附錄。
  - 同距離時改取語言預設地區的語言檔（fr-BE 在 fr-CA、fr-FR 之間取 fr-FR），刻意與 Python 的字母序不同（`fix(i18n): break distance ties in favour of the language's default region`）；受影響的 golden 案例列在 parity 文件。

## 合併前清掉的小問題（第 1 段審查列為可延後）

已於第 5 段處理（`c3105b8..9e57646`）：

- **log 文字與 Python 的細節差異**：截斷樣本、語言標籤、視窗標題改用 Python `repr` 的引號寫法，`follow` 印成 `True`／`False`；`null` 與 `None` 的寫法原本就已對齊。其餘差異是 Python 才有的東西（Tk 的 `region_h`、例外類別名稱），照 spec「log 不逐字相同」保留。
- **測試覆蓋缺口**：exporter 的 exit 2（Python 端新增測試，`tools/` 在搬到根目錄時會一起刪掉）、`TimestampedWriter` 的邊界、3xx `/v1/models`、黏底時最後一則譯文長高、廣播訊息 msg_id 跨分頁更新、分頁列未選取的顏色、整張 palette 的顏色都補了測試；左緣縮放帶的拖曳早已有 `left_edge_resizes_and_keeps_right_edge`。
- **語言比對**：新增 `offered_languages_are_the_ones_match_tag_was_checked_against`，語言選單多出第四種語言就會失敗，提醒補 nb／nn、iw、pt 地區、sr-Latn 的案例；距離比對本身見「第 5 段」的 `best_match` 一項（距離比對完成後，這則測試已由涵蓋全部語言檔的 `i18n_closest_match` golden 取代並刪除）。
- **其他**：泡泡未讀數改用粗體（`FontFamily::Name("bold")`）；`config::save_config` 改成先寫暫存檔再改名，寫到一半當機不會毀掉設定。

保留：

- **訊息清單沒有在背景補排版**（要改架構）：從沒進過可視範圍的列，捲軸位置只是估計值。
- **混合 DPI 下拖曳泡泡或 overlay 跨螢幕時，實體尺寸的處理**（需要實機才能驗證）。

## 第 2 段審查列為可保留的小問題

已於第 5 段處理（`c3105b8..9e57646`）：

- 鍵盤名稱表多出 Python 不認得的名稱：新增測試比對整張名稱表。
- `SendInput` 回傳值：沒送完會記 log（含已送出數與 Win32 錯誤碼）。
- lib 單元測試印到 stderr 的剪貼簿雜訊：測試改用 `logs::capture` 接住並斷言。
- 設定缺 `hotkey`／`region_hotkey` 時 panic：改成不 panic。
- `register_hotkeys`／`install_paste` 寫死 `Foreground::real()`：改由 `InputWiring.foreground` 注入，測試不再讀真實前景，斷言只剩一個分支。
- handler panic 時 backtrace 擷取過久：鍵盤 hook 執行緒上的 panic 不抓 backtrace。
- overlay 的 HWND 還是 0 時開輸入框：記一行 log 說明（正式程式不會發生）。
- `on_paste_hotkey` 的 `std::thread::spawn`：資源耗盡時的 panic 會被 handler 的 `catch_unwind` 接住並放行 Ctrl+V，行為正確，不改簽名。

保留：

- **CI 上的鍵盤名稱 fixture**（需要在 CI 上驗證）：`keyboard_names` 等 fixture 與鍵盤配置有關，在本機（US 實體配置）產生。push 後若 CI 的 `keys_golden` 或 `input::keys::tests::the_name_table_has_no_name_python_does_not_know` 失敗，是配置差異，要調整 fixture。
- **golden fixture 已凍結**：fixture 的匯出工具只存在於 git 歷史裡；有意改變行為時，要直接手動編輯 fixture JSON。
- **鍵盤 hook 一個程序只能裝一次**（要改架構）：第一次失敗後無法重試，要重做 hook 執行緒與分派器的生命週期；Python 也沒有重試。
- **`scale_at` 不能注入**（需要實機才能驗證）：沒有單元測試證明用的是錨點所在的螢幕；列入混合 DPI 的實機驗收。
