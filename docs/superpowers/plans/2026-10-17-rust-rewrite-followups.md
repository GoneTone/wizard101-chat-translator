# Rust 重寫：第 1 段留給後續各段的待辦

第 1 段（`2026-10-17-rust-rewrite-phase1.md`）已實作完成，使用者實機驗收通過（2026-10-17）。以下是執行過程中延到後段處理的事項，寫各段計畫時要納入。

## 第 1 段尚未驗收的項目

- **斷網時的離線橫幅**：使用者暫時沒測。行為：暫時斷網後，等新訊息進來，overlay 出現離線橫幅；恢復網路後自動補翻。第 2 段驗收時一併確認。

## 第 2 段

- 開發版從主控台按 Ctrl+C 會直接結束，不走關閉流程，hook 要等下次啟動才修復。Python 版會接住 `KeyboardInterrupt`，再走 `finally`。
- 輸入框使用 `app_name()` 當視窗標題。泡泡改用「建立前後比對視窗」來找自己的 HWND，所以不會撞名；新增的 viewport 也不可以靠標題找 HWND。

## 第 4 段

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
