# 第四輪稽核修正計畫（2026-10-21）

來源：補查前三輪沒讀到的 overlay 選取／外框、設定視窗、翻譯核心，共 16 項（無必修），標 ✔ 的主控已對照原始碼確認。使用者決定全部處理。分支 `fix/audit-4`，從 `refactor/rust-rewrite`（a17a195）切出。

## Global Constraints（每個 task 都適用）

- 先讀 `CLAUDE.md`。實作註解預設一行、只寫 WHY，三行是上限不是目標；docstring 一行摘要；CJK 全形標點；log 一律英文、沿用既有前綴；不可寫死語言；API 金鑰不可進 log。
- 行號是查證當下的參考值，動手前以實際程式為準；描述與程式不符、或查證後問題不存在時，以程式為準並在回報說明（不存在就不改）。
- 修 bug 先寫會失敗的測試、確認失敗，再修。純精簡（刪 `pub`、抽小函式）靠現有測試把關。golden 檔（`tests/golden/*.json`）不改。
- 新增 UI 字串：每個有翻譯到該區的語系檔都要補（`src/i18n/*.json`，比照同區既有 key 在哪些檔出現）。
- 測試不可碰真實桌面、鍵盤 hook、剪貼簿或執行中的遊戲；不可啟動 GUI 或截圖。新的整合測試放 `tests/it/`。
- 每個 task 結束前：`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、完整 `cargo test` 各跑一次，全綠才 commit。迭代時只跑聚焦測試。
- Commit：英文 conventional commits，可分多個；結尾附派工說明給的 attribution 兩行。只 `git add` 改到的檔案。不 push。

---

## Task 1：收訊上下文、快取、框選空白（reader／translation／region）

1. ✔ **多分頁訊息只進第一個分頁的上下文**：`src/reader/run_loop.rs` `dispatch` 只對 `tabs.first()` 取快照並 push。伺服器廣播的玩家訊息 `ChatEvent.tabs` 可能有多個分頁，使用者在另一個分頁回話時上下文缺這句。修法：快照照舊取第一個分頁（送進 pool 的那份不變），但這句要 push 進 `tabs` 裡每個分頁的上下文（`tabs` 為空時照舊用分頁 0）。補 reader loop 測試：`tabs=[0,1]` 的玩家訊息兩個分頁的上下文都有這句。
2. ✔ **換目標語言 A→B→A 時系統訊息快取被清空**：`src/translation/cache.rs` `rebind` 落盤後清空記憶體，換回 A 時已沒有 A 的條目（磁碟檔也被 B 的空快取蓋掉）。修法：`Inner` 加一格暫存 `previous: Option<(String, 條目)>`；`rebind` 把目前指紋與條目移進暫存，新指紋等於暫存的指紋就把條目換回來（`unflushed` 設成會在下次 flush 寫回的狀態，確保磁碟檔回到 A 的內容）。只救同一次執行，不做每個指紋一個檔案。log 照舊並註明是否從暫存換回。補測試：A 放入條目 → rebind B → rebind A → `get` 拿得到 A 的條目，且 flush 後的檔案是 A 的指紋與條目。
3. **框選 OCR 結果只有空白仍送翻譯請求**：`src/region/pipeline.rs` 只檢查 `source.is_empty()`，`"\n"` 這種結果會送出空的請求（Claude 回 400、卡片顯示錯誤）。修法：改 `source.trim().is_empty()`（沿用專案 `py::strip` 若周圍是用它）。補測試：recognize 回 `"\n "` 時得到空結果、翻譯函式沒被呼叫。

## Task 2：overlay 外框與選取（src/ui/overlay/、src/ui/geometry.rs）

1. ✔ **標題列只點一下也寫 config**：`frame.rs` `drag_stopped` 一律呼叫 `on_geometry_change`；egui 0.37 只感應拖曳的元件一按下就算開始拖，單擊放開也會觸發，每次都在 UI 執行緒寫檔＋fsync。修法：放開時 `shell.rect()` 與 `Gesture.start` 相同就不呼叫（resize end 的 log 照舊）。補測試：標題列按下放開不動 → 回呼沒被呼叫；有移動 → 呼叫一次。
2. **非整數縮放時最外圈一排像素抓不到縮放**：`frame.rs` `edge_under` 用 `area.width() as i32` 截斷，125% 下寬 512.8 變 512，最右／最下一排（floor 後 = 512）落在範圍外。修法：寬高改 `.ceil() as i32`。補測試：寬 512.8 時 x=512.0 判為右邊緣。
3. **左下角縮放帶蓋到最新一行第一個字**：`CORNER`（14 DIP）的西南角範圍與 `message_list.rs` 內容區（底部只內縮 `EDGE` 6、文字邊距 `TEXT_MARGIN` 8）重疊，在最新一行第一個字的下半部按下會變成縮放。修法：讓最新一行文字的底部不落在角的範圍內（例如內容區底部內縮改用 `CORNER`），擇改動最小、不讓其他邊出現「看得到但選不到」的做法。補測試：在最新一行第一個字、距底部 8～14 DIP 處按下拖曳會選字而非縮放。
4. **拖到不同縮放的螢幕後可能小於最小尺寸**：移動保留實體像素寬高，100% 的 200×90 拖到 150% 變成 133×60 DIP，低於 `MIN_WIDTH`／`MIN_HEIGHT`，下次啟動 `initial_rect` 用的是建立視窗那個螢幕的縮放，也不會修正。修法：(a) 啟動時 `initial_rect` 的 scale 改用 `shell.scale_at(x, y)`（存檔的 x、y 都有值時；否則照舊用目前 ppp）；(b) 移動拖曳放開時，若寬高小於放開位置那個螢幕縮放下的 `min_size`，放大到最小尺寸再存。補測試（用測試 Shell 的 `scale_at`）：兩種情況各一。實機效果留給使用者驗收。
5. **只在檔內使用的 `pub`**：`selection.rs` 的 `Selection::extend`、`Selection::finish`、`selected_text`、`STATUS_COLORS`（以實際所在檔為準）全樹 grep（含 tests）確認只在本檔使用後改為私有；`Selection::begin` 有測試用，保留。

## Task 3：設定視窗（src/ui/settings/、src/translation/cache.rs `clear`）

1. ✔ **熱鍵設成 Ctrl+V 沒擋**：`hotkeys.rs` `conflict()` 只比對兩把熱鍵互撞。「遊戲內貼上」開著時，輸入框或框選熱鍵設成 `ctrl+v`，啟動時熱鍵生效、按過儲存後改成貼上生效（掛載順序反轉），熱鍵默默失效。修法：設定視窗的儲存（`collect()`）與精靈的完成（`finish()`）在貼上開著且任一熱鍵為 `ctrl+v`（以 `input` 模組的正規化名稱比對）時擋下，顯示新的錯誤字串（每個有翻譯的語系都補）。補測試：兩個入口各一；貼上關著時不擋。
2. ✔ **測試連線時目標語言為空白**：`window/mod.rs` `page_ui` 把去空白後的目標語言傳給服務頁，空白時測試請求的提示詞沒有語言、結果仍顯示已連線。修法：空白時改用 `language_name(&current_language())`，比照精靈（`wizard/steps.rs` 的同類處理）。補測試。
3. **進階數字欄位超範圍時存檔默默修正**：輸入 5000 的訊息上限，存成 1000，使用者重開才看到。修法：`fields.rs` 數字欄位在失去焦點時把文字修正到範圍內並重排格式，沿用 −/+ 按鈕已有的 clamp＋格式化寫法（不重寫一份）。補測試：輸入超範圍值後移開焦點，欄位文字變成上限。
4. **自訂服務的 base_url 格式沒檢查**：貼上 `http://localhost:1234/v1` 會打到 `/v1/v1/chat/completions` 得到 404；沒有 `http://`／`https://` 開頭則建立請求就失敗。裁定：**只在服務表單擋**（按確定與測試連線時），不改 `config::services::validate_endpoint_fields`／`validate_service` —— 它們也決定啟動時要不要進精靈（`config/mod.rs`），並有 Python 一致性 golden。條件：需要 base_url 的服務商，去空白後不以 `http://` 或 `https://` 開頭（不分大小寫），或去掉結尾 `/` 後以 `/v1` 結尾。兩種各一個新錯誤字串（每個有翻譯的語系都補；文案說明正確寫法，不寫死某家服務）。補測試。
5. **effort 不合法值顯示「自動」卻存回原值**：`service_form.rs` 讀 effort 時，不在 `EFFORTS` 裡的值（例如手改成 `""`）畫面顯示自動但按確定保留原值，Claude 可能回 400。修法：建構表單時先過濾 `EFFORTS.contains`，否則用 `EFFORT_AUTO`。補測試。
6. **清快取刪檔失敗仍顯示成功、log 重複**：`cache.rs` `clear` 刪檔失敗只記 log，關於頁顯示「已清除 N 筆」，重開又回來；`env.rs` 與 `cache.rs` 各記一次清除。修法：`clear` 回 `Option<usize>`（刪檔失敗回 `None`，`NotFound` 算成功），呼叫端失敗時顯示失敗訊息（沿用關於頁既有的 `outcome(false, …)` 樣式與字串；沒有合適字串才新增）；刪掉 `env.rs` 那條重複 log。補測試：刪檔失敗回 `None`。
7. **精簡：熱鍵錄製中止重複 6 處**：`hotkeys.stop(); hotkeys.release(env);`（設定視窗、`hotkeys.rs`、精靈）抽成 `HotkeyEditor::abort(env)`，全部改用。
8. **精簡：`service_list.rs` 兩處相同的 `Dialog::Form { form, picking: false, errors: vec![] }`**：抽成一個建構函式，兩處改用。
