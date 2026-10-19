# 全專案稽核修正計畫（2026-10-19）

來源：兩輪全專案稽核（第二輪逐行讀完各區並以臨時測試重現）。分支 `refactor/audit-cleanup`，從 `refactor/rust-rewrite`（cc1ecbd）切出。

## 共通規則（每個 task 都適用）

- 先讀 `CLAUDE.md`。註解預設一行、只寫 WHY；docstring 一行摘要；CJK 全形標點；log 一律英文、沿用 `[reader]`／`[translate]`／`[ui]` 等前綴；不可寫死語言。
- 行號是稽核當下的參考值，動手前以實際程式為準；同一 task 內若發現稽核描述與程式不符，以程式為準並在回報中說明。
- 修 bug 先寫會失敗的測試，再修。純刪除與機械重構靠現有測試把關。
- 測試不可碰真實桌面、鍵盤 hook、剪貼簿或執行中的遊戲；不可啟動 GUI 或截圖。
- 每個 task 結束前：`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、完整 `cargo test` 各跑一次，全綠才 commit。
- Commit：英文 conventional commits，可分多個 commit；結尾附 attribution 兩行（見派工說明）。不 push。

---

## T1 翻譯 client 的 bug 與重複

1. **空譯文當成功**：`Translator::chat`（`src/translation/translator.rs` 約 196 行）拿到 client 結果、`tidy_parentheses` 之後、`restore_sender` 之前，`pystr::strip(..).is_empty()` 就記 log 並回 `TranslateError::BadOutput("model returned no text")`。同步改 `translator.rs` 模組文件與 `errors.rs` 中「BadOutput＝截斷」的描述。新增測試：fake backend 回 `""` → 收訊得 BadOutput、系統訊息不寫入快取。
2. **OpenAI 3xx 當成功**：`clients/openai.rs` `stream_once` 的 `if status >= 400` 改成非 2xx 一律走 `status_failure(...)`（或與 Claude／Gemini 相同的 Unexpected 路徑），拿掉 `.expect`。`tests/it/clients_openai.rs` 約 550 行鎖住舊行為的測試改成預期 `Unexpected("HTTP 302…")`。
3. **錯誤說明解析統一**：Claude（`claude.rs` 的 `detail`）與 Gemini（`gemini.rs` 的 `detail`＋`message_in`）的非 JSON 分支改呼叫 `errors::error_detail`（會剝 HTML）；Gemini 的「單元素陣列先拆」搬進 `error_detail`。刪掉 Claude 模仿 SDK 的 `"Error code: …"` 前綴。各補一則 HTML 502 測試；更新 `tests/golden/clients_error_detail.json` 若有影響。
4. **Claude 模型清單第二頁以後的錯誤要分類**（`claude.rs` 約 179-183）：照第一頁的錯誤處理（`?` 往上傳分類後的錯誤），刪掉照抄 Python 的註解。
5. **Claude 重試用完顯示真正原因**：刪 `CONNECTION_ERROR`／`TIMEOUT_ERROR` 常數，重試用完直接回 `connection_error(err)`（與 OpenAI／Gemini 一致）；改 `tests/it/clients_claude.rs` 約 337、347 兩則測試。
6. **Claude retry-after 上限**：`MAX_RETRY_AFTER` 改為 60 秒；超過 60 秒的 retry-after 視同沒有、改走指數退避（交給池子的 5／15／30 秒退避）。改 `claude.rs` 約 453-456 的測試。
7. **408／409 不是設定錯誤**：`errors.rs` 約 130 行把 408（逾時）與 409 改歸暫時性錯誤（與 429／5xx 同類，會重試、不顯示「設定錯誤」）；改 `clients_claude.rs` 約 304-305 的斷言。先確認三家 client 共用這個分類。
8. **client 間近似複製**：在 `clients/mod.rs` 放共用的 `parse_object(text, what)`（取代 6 處 `from_str().ok().filter(is_object).ok_or_else`）、一份 `Streamed`（取代 `openai.rs`／`gemini.rs` 各一份）、`finish_text()`（取代 3 處 `strip(strip_think(..))`）。
9. **`ChatBackend::list_models` 沒人透過 trait 呼叫**：從 trait 刪掉，`Client::list_models` 保留；刪三個 fake 的樁實作（`tests/common/fake_backend.rs`、`tests/it/cache_golden.rs`、`tests/it/app_main.rs`）。
10. **「用 RandomState 當亂數」兩份**（`claude.rs` 約 331、`config/services.rs` 約 311）：抽到 `util.rs` 一個函式共用。
11. **Claude 金鑰含非法 header 字元時的誤導訊息**（顯示 "Could not resolve authentication method"）：改成明確的金鑰格式錯誤（走既有的金鑰錯誤分類）。

## T2 JSON 寫檔回歸 serde＋原子寫檔統一

Python 版已刪，讀這些檔的只剩 Rust 的 serde；逐 byte 對齊 Python 已無讀者，且 `dumps_indent2` 會寫出讀不回的 `Infinity`（手改設定成 `1e400` 後存檔 → 下次啟動整份退回預設 → 精靈存檔會蓋掉含金鑰的設定）。

1. `config/mod.rs` 的 `save_config` 改用 `serde_json::to_string_pretty`（專案開了 `preserve_order`、`arbitrary_precision`，正常設定位元組相同；`1e400` 會照原字面值寫回）。刪 `config/py.rs` 的 `dumps_indent2` 與只為它存在的輔助函式；保留讀取端容錯（`truthy`、int／float 保留）。新增回歸測試：含 `1e400` 的設定存檔後可再讀回。
2. `cache.rs` 的 `flush`、`examples.rs` 的 `render_entries`、`reader/hook_state.rs` 的寫檔改用 `serde_json::to_string`（建 `Value`／`json!`，保持鍵順序）。舊格式檔案仍讀得回來（加一則讀舊格式的測試）。
3. 只鎖 Python 位元組的 golden（`cache_file_bytes.json`、`examples_store_bytes.json`、`hookstate_bytes.json`、`config_load.json` 中的寫回位元組）改成「寫出 → 讀回 → 內容相等」的測試；仍有讀取語意價值的 case 保留。
4. **注意**：`ExampleSet::py_repr`／`repr_str` 用於範例指紋 sha1，**不可動**，否則使用者的範例與譯文快取全部作廢。
5. **原子寫檔兩套**：把 `examples.rs` 的 `write_atomic`（獨立暫存檔名、會建資料夾）移到 `util.rs`，`save_config`（固定 `.tmp` 名，兩次同時存檔會互踩）改呼叫它；`config/mod.rs` 約 539 的 `a_failed_save_keeps_the_previous_config` 改用別的方法觸發寫入失敗。
6. **小包裝**：刪 `py::is_blank`（＝`pystr::strip(s).is_empty()`）與 `py::str_repr`（轉呼叫 `repr_str`），呼叫端直接用。
7. **`Settings.translate_system_messages` 沒人讀**：刪欄位（`config/mod.rs` 約 386、417）。
8. **進階設定頁**：`advanced_page` 開頭取一次 `default_config()` 往下傳；`bound()` 判斷浮點改讀 `ADVANCED_LIMITS` 的 `is_float`（必要時公開，連帶改 `tests/config_services.rs`）。

## T3 範例生成重複付費

1. `generate_and_store`（`translator.rs` 約 470-487）與 `ExampleCoordinator::generate`（約 625-651）共用一個「生成 → 全空不存 → 寫入」函式。
2. pending 登記要跨兩條路共享（測試連線用的是新的 `Translator`／`ExampleStore`，見 `ui/settings/env.rs` 約 122）：例如以檔案路徑＋指紋為鍵的 process 內 static 集合，同一指紋生成中時另一條路等它或直接跳過。補一則併發測試：兩條路同時對同一指紋生成，只發一輪請求。
3. `generate_and_store` 的回傳值只有測試在用就改掉。

## T4 reader／runtime／logs

1. **版本不符時的重試**（A1）：`run_loop.rs` 遇到 `GameNotRunning::VersionMismatch` 時的重試間隔拉長到 60 秒（其餘原因維持原間隔）；`mem_reader.rs` 的 `[reader] game path:` 只在 PID 改變時記；`AddLine hook install failed`／root 沒觸發這類每輪都寫的 log 改成原因改變才記。補測試（虛擬時鐘）。
2. **狀態來回跳**（A2）：`run_loop.rs` `read()` 開頭那次 `board.report(listening_status())` 換成 `self.report_activity()`；`tests/it/reader_loop.rs` 約 601 改成斷言完整狀態序列（只出現 listening → translating → …，不交替）。
3. **未知圖示去重被清**（A3）：`forget_warned_icons()` 從 `teardown()` 移到 `connect()` 成功結尾；修正 `markup.rs` 約 45 與實際相反的註解。補一則測試。
4. **找遊戲視窗兩套**（A5）：`find_game_window` 改為 `game_windows()` 中第一個可見的視窗；刪 `first_game_window`、`find_game_window_in`、`enum_windows`、`collect_window` 與對應測試（`process.rs` 內 4 則、`tests/it/mem_reader.rs` 約 711-720 與 import）。`game_windows` 的 EnumWindows 失敗時記 log。保留 `process_exe_path`、`is_game_process_path`。
5. **`window_exe`**（C9）：`composer/paste.rs` 的 `foreground_exe`／`exe_of_window` 搬到 `reader::process`（新增 `window_exe(hwnd)`，給 `input/dispatch.rs` 用），連同 `paste.rs` 的相關測試。
6. **死碼**：`ProcessMemory::read_wstring`（測試改呼叫 `decode_utf16le`）、`AutobotCave::uninstall`（文件連結改指 `uninstall_with`，修正「（測試用）」的錯誤描述）、`updater::parse_version` 與其測試、`ChatEvent.seq`／`.raw`（改測試建構式）、`ICON_ICO` 的 `include_bytes` 移進測試。`HookRecord.ret` 保留。
7. **轉手包裝**：刪 `reader_loop`→`reader_loop_with`、`supervise`→`supervise_with` 的包裝，`_with` 改回原名；呼叫端（`app_main.rs`）直接傳預設值。刪 `tests/it/app_main.rs` 約 553-565 比對原始碼字串的脆弱測試。
8. **仿 Python 的 log 格式**（A7）：刪 4 份 `type_name()`（`memory.rs`、`pattern.rs`、`hooks.rs`、`chat_tabs.rs`）與 `mem_error`，log 直接 `{err}`；刪 `"OSError: "`／`"error: "`／`"ValueError: "`／`"JSONDecodeError: "` 前綴（`mem_reader.rs`、`hook_state.rs`）、Python struct 錯誤文字（`hooks.rs` 約 47-49）、`_PLAYER_IMG_PREFIXES` 舊名、字面 `None`（`mem_reader.rs` 約 571、`run_loop.rs` 約 383 改成可讀字樣）。同步改 `tests/it/mem_reader.rs`、`hooks_golden.rs`、`reader_golden.rs` 的斷言。wizwalker 原文的 Display（`memory.rs`、`pattern.rs` 等）不動。`repr_str` 不動。
9. **等待迴圈**（A8）：`wait_watching_input` 改成單一截止迴圈 `sleep(min(remaining, INPUT_POLL_INTERVAL)); poll_input()`。
10. **重複常數**（A9）：`hooks::MODULE` 改為引用 `process::PROCESS_NAME`；`util` 抽 `hex()` 給 `hook_state.rs` 與 `sha1_head` 共用。
11. **ContextMap 不清**（A10）：`app_main.rs` 插入新 hwnd 前 `retain` 掉已不存在的視窗。**updater** 的「next automatic check」log 移到 sleep 前。
12. **`status.rs` 約 48-49 到不了的 None 分支**：刪除（確認 `pool.rs` 約 220 的前提）。
13. **i18n**：`i18n/mod.rs` 約 199、212 的 `lock().unwrap()` 改 `util::lock`；內容為 `{}` 的空語言檔跳過且不逐檔寫「not offered」log（改成一行彙總）。
14. **過時文件**：`runtime/mod.rs` 模組文件補 updater；`util.rs` 約 13「讀取器用的時鐘」更正。

## T5 統一使用 `util::lock`／`read`／`write`

全 `src` 約 21 處手寫 `.lock()/.read()/.write().unwrap_or_else(|e| e.into_inner())` 換成 `util::lock`／`read`／`write`（含 `status.rs` 的私有 `lock()`、`pool.rs`、`gate.rs`、`translator.rs`、`cache.rs`、`examples.rs`、`clients/openai.rs`、`logs/*`、`reader/*`、`ui/events.rs`、`app_main.rs`）。`#[cfg(test)]` 內的同類寫法也一併換。完成後 grep 確認 `src` 無殘留。

## T6 熱鍵與輸入

1. **打字期間放開 V 被吞**（C1）：`input/hotkeys.rs` `should_swallow` 的 override 改成 `ev.down && override()`；更新 `swallow_override` 的 doc（「觸發鍵按下一律吞」）；`composer/paste.rs` 約 540 的測試改成斷言放開會放行；新增 Dispatcher 層級回歸測試：打字中放開 V 後 `pressed` 不殘留 V、被動熱鍵仍觸發。
2. **兩個 `Desktop`**（C3）：`overlay/selection.rs` 的 `trait Desktop` 改名（例如 `SelectionShell`），更新 `app.rs`、`tests/it/ui_probe.rs` 等引用。

## T7 托管視窗與 overlay／app 清理

1. **托管視窗共用純函式**（C2／D1）：在 `src/ui/viewport.rs` 放 `windows_before(hwnd)`、`find_new_window(before, known, title) -> isize`（或放 `win32.rs`）、`borderless_builder(creating, rough, scale)`、`frameless_setup(hwnd, owner)`（拿掉外框＋`disable_transitions`＋由 overlay 擁有）。`region_host.rs`、`input_host.rs` 改用；`settings_host.rs` 的 `WindowOps::real` 用 `find_new_window`／`windows_before`。**不要**合併三套「顯示／搶前景」時序，也不要把 owner／APPWINDOW 處理套到設定視窗（見記憶 taskbar-button-owner-order）。
2. **輸入框補 `disable_transitions`**（N1）：經由上面的 `frameless_setup` 涵蓋；回報中標註此為行為改變、需實機確認。
3. `win32::set_cloaked` 改用現成的 `set_dwm_attribute`；`win32.rs` 約 398 的 `set_window_rect` 文件註解移到正確的函式上。
4. **死 API**：`OverlayModel::minimized()`、`is_empty()`（測試改 `texts(&model).is_empty()`）；`InputBox::content` 回傳的 `Option<BoxAction>`（正式碼丟掉）——改成不回傳，`BoxAction` 若只剩測試在用一併處理；`OnSave` 回傳 tuple 的第一個值（`app.rs`、`app_main.rs`、`tests/it/app_main.rs`、`tests/settings_host.rs`、`tests/it/ui_probe.rs`），測試改讀已寫回的設定。
5. **`App` 的 5 個 `with_*`**（只有測試用）刪除，測試改呼叫 `set_*`；`app.rs` 測試裡與 `app_harness` 相同的 inline `App::new` 改用 helper。`with_first_run` 是正式碼在用，保留。
6. **自動捲動節拍**（C4）：`text_select.rs` 加 `autoscroll_tick(...)`，`overlay/selection.rs` 與 `ui/region/card.rs` 各呼叫（注意兩邊方向相反）。
7. **選單範圍重算**（N7）：`copy_menu::show` 的 `MenuOutcome::Open` 直接帶回 rect，`selection.rs`／`card.rs` 不再呼叫 `copy_menu::rect`。
8. **輸入框色碼**（N4）：`input_box.rs` 的 `FG`／`HINT_FG` 改引用 `palette` 既有常數（數值相同才引用；沒有就收進 `palette`）。

## T8 剪貼簿寫入改用 egui

1. 寫入（overlay 選取複製、框選卡片複製、`region_host.rs` 每幀重建的 owner 閉包）改用 `ctx.copy_text`；刪 `clipboard.rs` 的寫入實作。`[ui] copied selection` log 保留（改成記錄已送出複製請求與字數）。
2. 讀取（`composer/paste.rs` 背景執行緒讀剪貼簿）egui 沒有對應 API：保留 `clipboard.rs` 的讀取函式，或改用 eframe 已帶入的 `arboard`（在 `Cargo.toml` 直接宣告，不新增編譯的 crate）——擇行數少者。
3. 結果：App 層的選取 shell 不再直接碰 Win32 剪貼簿，測試不可能寫入使用者真正的剪貼簿。回報中標註需實機驗證複製行為。

## T9 框選

1. **最小化的遊戲**：`region/capture.rs` 的 `client_geometry`（或擷取入口）遇到 `IsIconic` 直接回錯誤（不等 2 秒 WGC 逾時），錯誤卡片沿用既有的擷取失敗訊息；補一則注入幾何的測試。
2. **暗化改繪製時疊色**：`ui/region/selector.rs` 刪 `dim()` 與 `dim_frame` 貼圖，畫遊戲畫面時以 `Color32::from_gray(115)` 著色（≈0.451 倍）；`Textures` 由 3 張變 2 張；改 `backdrop_and_dimmed_frame_are_placed_in_order`、`texture_ids` 等測試為檢查頂點顏色，刪 `dim_scales_each_channel`（PIL 逐位元對齊已無意義）。
3. **`RegionBox::drag`** 改為不回傳值，文件註解更正，測試改檢查 `rect()`。
4. **測試替身**：crate 內 `RegionDesktop` 的兩份 fake（`flow.rs`、`region_host.rs` 測試）合併成一份。

## T10 設定視窗與精靈

1. **HotkeyEditor 生命週期**（D3）：待釋放旗標搬進 `HotkeyEditor`（`apply`／`stop` 自己設、新增 `release(&mut self, env)`），加 `from_config`、`write_into`、`conflict()`；`window/mod.rs`、`wizard/mod.rs`、`wizard/steps.rs` 改用；更新 `hotkeys.rs` 的文件註解。
2. **服務下拉**（D6）：`service_list.rs` 抽 `service_combo(ui, salt, current, follow: Option<&str>)`，命名與焦點框留給呼叫端。
3. **現成查找與圖示**（D7）：`basic.rs` 改用 `ServiceDraft::find`；精靈改用 `config::services::find`、`service_draft::id_of`，減少重複的 `ServiceDraft::from_config(..).services`；`provider_picker.rs` 加 `provider_icon(ui, key, size)` 取代 3 處「配置空間再畫圖示」。
4. **頁面捲動外殼與 App 圖示**（D8）：`widgets.rs` 加 `scroll_page(...)`，`thin_scrollbar` 搬到 `widgets.rs`；`theme::CONTENT_MARGIN` 改成 `Margin` 常數；App 圖示改成像 `provider_texture` 一樣快取在 ctx，刪 `about.rs`、`wizard`、`overlay/frame.rs`、`overlay/bubble.rs` 各自的快取欄位。
5. **模型欄**（D4）：`service_form.rs` 約 322-353 手寫的右對齊改用 `right_aligned`／`right_aligned_width`（比照 `service_list.rs`）。
6. **`friendly_error` 參數轉換三份**（N4）：在 `translation::errors` 加 `friendly_text(...)`，`service_form.rs`、`region/pipeline.rs`、`overlay/banners.rs` 改用。
7. **測試名稱**（N3）：`tests/it/service_list.rs` 約 784 `the_cancel_button_closes_the_picker` 改名為 close、刪過時註解。

## T11 測試程式整理

1. 新增 `tests/common/kittest.rs`（`tr`、`run`、捲過去再點的 `click`、`has_label`、`opened_urls`、`painted_texts`、`click_at`、`language_lock` 等）與 `tests/common/settings_window.rs`（設定視窗 harness／State、「一筆 custom 服務的完整設定」），各測試檔改用。
2. reader 測試替身：`tests/it/mem_reader.rs` 的 `Shared` 與 `reader_golden.rs` 的 `SpyMemory`、`put_record`／`put` 收進 `tests/common`。
3. `log_lines_with("")` 當初始化的地方改 `capture_logs()`，避免 O(n²) 重讀。`logs::capture()` 在閉包 panic 時也要把 sink 換回。
4. `tests/it/app_main.rs` 的 `Backend` 改用 `common::fake_backend::Fake`；`reader_fixture.rs` 的 `pool()` 與 `tests/it/pool.rs` 的 `make_pool` 合併。
5. 刪無作用的 `#[allow(dead_code)] use crate::common;`。
6. `tests/it/layout_audit.rs` 會切換全域語言，依 CLAUDE.md 移出成 `tests/` 底下獨立檔並在檔頭註明原因。

## 不處理（驗證後不成立或刻意設計）

- `RegionHost::finish`：刻意的外觀層。
- 泡泡模式每 300 ms 輪詢前景：刻意且有測試。
- `Registration`／`Recording` 合併、「沿用上一幀尺寸」通用 helper、`fonts.rs` 兩段式對應：省不到或有 golden 價值。
- `join_until`：語意不同，不算重複。
- 框選擷取移到背景執行緒：新資料流，T11 之後另寫 spec 再實作（使用者 2026-10-19 確認要做；要保住「選取畫面與送辨識是同一幀」與觸發鍵放開的等待）。
- `app.log` 執行期無上限：主要來源是 T4-1 的每輪 log，修掉後不再成長；輪替屬新功能，有需求再做。
- 伺服器廣播暫留只看尾巴 2～3 筆（`chat_hook.rs` 約 235-245）：未驗證的邊界推論，T4 執行者若能以 fake 重現再修，否則記錄於回報。

## T12 伺服器廣播被拆在兩輪讀取（先確認再修）

使用者 2026-10-19 指示：先用假記憶體重現，確認會重複顯示再修。邏輯與 Python 版（`old-python` 的 `chat_hook.py` `pending_broadcast`）相同，屬一直存在的邊界。

1. **確認**：以 `tests/common/fake_game.rs`／`reader_fixture.rs` 的假記憶體，讓一則廣播的分頁 0～3 在第一輪讀取、頻道分頁（5 以上）在下一輪才寫入，斷言目前的行為（頻道那幾份是否被當成額外訊息投遞）。把結果寫進回報；若不會重複，停在這裡，只留下這則測試鎖住行為。
2. **修正（僅在第 1 步確認會重複時）**：`chat_hook.rs` 的暫留也涵蓋「結尾已是連號、同字、分頁 0 起、湊滿 4 筆以上且仍可能接頻道分頁」的廣播，下一輪再一起分組；暫留不可無限延長（下一輪沒有接續就照常投遞）。第 1 步的測試改斷言只投遞一則。
