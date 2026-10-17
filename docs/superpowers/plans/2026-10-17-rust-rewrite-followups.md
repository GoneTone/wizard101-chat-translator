# Rust 重寫：第 1 段留給後續各段的待辦

第 1 段（`2026-10-17-rust-rewrite-phase1.md`）已實作完成，使用者實機驗收通過（2026-10-17）。以下是執行過程中延到後段處理的事項，寫各段計畫時要納入。

## 第 1 段尚未驗收的項目

- **斷網時的離線橫幅**：使用者暫時沒測。行為：暫時斷網後，等新訊息進來，overlay 出現離線橫幅；恢復網路後自動補翻。第 2 段驗收時一併確認。

## 第 2 段

原列的兩項（主控台 Ctrl+C 走關閉流程、新 viewport 不靠標題找 HWND）已在第 2 段完成。第 2 段實作完成、待使用者實機驗收（`2026-10-17-rust-rewrite-phase2.md` 檔尾清單）。

## 第 3 段

- 框選快捷鍵目前只在遊戲內攔下按鍵並記 `[ui] region hotkey pressed ...`，要換成真的框選流程；`on_region_hotkey` 的 `selecting` 旗標目前恆為 false，要接上選取層是否開著。
- `Registration::unregister` 目前沒有正式呼叫端；第 4 段套用設定時要用它重新註冊熱鍵。
- 框選選取層由熱鍵開啟時，要等觸發鍵放開才顯示並搶前景（同輸入框的 `defer_for_release`，判斷用 `input::hotkeys::swallowed_key_held()`）。觸發鍵按著時換前景，那次放開不會進到鍵盤 hook，下一次熱鍵會被吞掉。

## 第 4 段

- 設定視窗裡顯示 API 錯誤的地方，網址都要能點（Python 用 `RichLabel`）：測試連線結果（`service_form.py`）、模型清單狀態（`model_field.py`）、`form.py` 的結果呈現。一律走 `ui::richtext::show`，不要另寫。
- `translator.py` 的 `generate_and_store`、`list_models`、`test_translate` 還沒移植，設定視窗的「測試連線」和「重新整理模型」需要它們。
- 更新橫幅的 `offer_update` 目前只擋「同一個版本」。Python 版的規則是用 SemVer `is_newer` 跟已關閉的版本比較，程式碼留有 `shortcut:` 註解。另外，更新檢查要接到 `banners_mut().offer_update`。
- 套用設定（reconfigure）時，改完 cfg 要重新綁定系統訊息快取的指紋。否則背景晚到的範例結果，可能讓快取留在舊指紋上。
- 啟動摘要要移到首次設定精靈之後再記，Python 是在 `build_app` 裡記。目前被擋下的第二份實例也會寫一行 startup。
- 新的無邊框 egui 視窗要呼叫 `ui::win32::remove_system_frame`，才不會出現 Win11 的圓角和灰邊。
- 程序內所有 winit 視窗都經由 `CreateWindowExW` 匯入 shim 拿掉了 redirection bitmap（裁定 R34）。這類視窗不可再用 GDI 繪圖。
- 泡泡建立的第一幀可能閃一下舊外框；這個在螢幕外量不到，需要時在實機確認。

## 第 5 段

- 把 `rust/` 搬到 repo 根目錄時，使用者的測試資料夾 `rust/target/release/_test`（不進版控，裡面有使用者的 config 和 log）要一起搬。搬之前先問使用者。
- `rust/tests/real_fingerprint.rs` 的文件寫「只讀副本」，但 `load_config` 在正規化有變動時會寫回檔案。要改文件，或改成測試自己先複製再讀。
- `fonts_dir` 遇到空字串的 `WINDIR` 時沒有過濾掉。

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
- **`install_input` 寫死 `Foreground::real()`**：測試會唯讀查詢真實前景，斷言因此兩個分支都接受。
- **`on_paste_hotkey` 用 `std::thread::spawn`**：資源耗盡時 panic，但會被 handler 的 `catch_unwind` 接住並放行；回傳 `JoinHandle` 是計畫定的簽名。
