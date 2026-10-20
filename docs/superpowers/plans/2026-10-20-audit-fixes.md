# 稽核修正計畫（2026-10-20）

來源：全專案稽核後逐項查證（讀程式、git 歷史、官方文件、old-python 分支）。分支 `fix/audit-2026-10-20`，從 `refactor/rust-rewrite`（58f0bda）切出。查證不成立而不做的：未讀數不隨淡出遞減（與 Python 版相同、有測試釘住）。使用者決定不做的：翻譯中的同範本系統訊息去重。

## Global Constraints（每個 task 都適用）

- 先讀 `CLAUDE.md`。實作註解預設一行、只寫 WHY，三行是上限不是目標；docstring 一行摘要；CJK 全形標點；log 一律英文、沿用 `[reader]`／`[translate]`／`[ui]`／`[composer]`／`[input]`／`[log]` 等前綴；不可寫死語言；API 金鑰不可進 log。
- 行號是查證當下的參考值，動手前以實際程式為準；若發現描述與程式不符，以程式為準並在回報中說明。
- 修 bug 先寫會失敗的測試、確認它真的失敗，再修。純刪除與機械重構靠現有測試把關。
- 新增介面字串時，`src/i18n/` 中有內容的語系（en-US、zh-CN、zh-TW）都要補上譯文。
- 測試不可碰真實桌面、鍵盤 hook、剪貼簿或執行中的遊戲；不可啟動 GUI 或截圖。
- 新的整合測試放 `tests/it/`（在 `tests/it/main.rs` 宣告）；會切換全域介面語言的測試例外。
- 每個 task 結束前：`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、完整 `cargo test` 各跑一次，全綠才 commit。迭代時只跑聚焦測試。
- Commit：英文 conventional commits，可分多個 commit；結尾附派工說明給的 attribution 兩行。不 push。

---

## Task 1：記錄檔與啟動順序

1. **取消記錄檔大小上限，只留天數**（使用者決定，app.log 與 messages.log 都適用）：刪 `src/logs/files.rs` 的 `LOG_HARD_CAP`／`LOG_KEEP_TAIL` 與 `prepare_log` 裡的截斷分支；`raw` 不再需要 `mut`，`original` 的 clone 可省（直接比 `out != raw`）。刪測試 `open_session_log_caps_oversized_file`。兩份 log 都走 `open_session_log`（`src/main.rs` 的 app.log、`src/app_main.rs` 的 messages.log），改一處即可。README 只寫「保留 7 天」，不用改；歷史設計文件不動。
2. **第二份程式不可動到執行中那份的 app.log**：`src/main.rs` 目前先 `open_session_log(app.log)`（會 `prepare_log` 重寫檔案並寫 session 標頭），才 `claim_instance`。改成 `main` 第一行先 `let lock = acquire_single_instance();`，只有 `lock.is_some()` 才開 app.log；之後 `claim_instance(|| lock, focus_running_instance)`（`claim_instance` 與其測試不變；`focus_running_instance` 需要的本地化 `app_name()` 仍在設定與語言載入之後才用到）。第二份程式的 log 走 stderr、會消失，可接受。
3. **缺字 log 去重**：`src/i18n/mod.rs` 的 `translate` 在缺 key 與格式化失敗時每次都記 log，而 `t()` 每幀都跑。加一個 `static` 的 `Mutex<HashSet<(String, String)>>`（語言、key），只在這兩個失敗分支查表，同一組只記一次；命中路徑不付成本。補一則測試：同一缺 key 呼叫兩次只記一次 log（若 log 難以攔截，可測去重集合的判斷函式）。

## Task 2：reader 掛入安全

1. **等 hook 生效時要能中止**：`WizChatReader` 加 `pub stop: CancellationToken`（`new` 給預設值），`src/app_main.rs` spawn reader 的 closure 設成 `shared.stop.clone()`。`wait_root_window_ready`（`src/reader/mem_reader.rs`）迴圈裡 `if self.stop.is_cancelled() || !conn.mem.is_alive()` 就回新的 `AttachError::Cancelled`（`connect` 的 match 既有 else 分支會走一般退出流程，teardown 仍會解除 hook；確認這點）。補測試：`tests/it/mem_reader.rs` 在 `on_sleep` 裡 cancel，斷言遠早於 `HOOK_READY_TIMEOUT` 返回、且 hook 已解除／狀態檔已清。`readiness_timeout_is_a_hook_not_firing_mismatch` 保持不變。
2. **修復殘留 hook 時等執行點離開 stub**：`src/reader/hook_state.rs` 的 `repair` 加參數 `wait: impl FnOnce(Duration)`；先寫 `ops[..n-1]`，`wait(500ms)`（與 `AutobotCave::uninstall_with` 同一個值，抽常數共用），再寫最後一筆（cave）。呼叫端 `mem_reader.rs` 傳 `|d| (self.clock.sleep)(d)`；`tests/it/hooks_golden.rs` 各 `repair(` 呼叫傳 `|_| {}`。補一則測試斷言 cave 在 wait 之後才寫。
3. **聊天框重建時清掉輸入框快取**：`refind_chatbox` 位址改變的分支加 `self.edit_node = None;`，更新該函式的文件註解。擴充 `tests/it/mem_reader.rs` 的 `chatbox_moving_clears_foreign_and_button_caches`：新聊天框底下放一個 chatEditContainer，斷言輸入框偵測讀到的是新的節點。
4. **找不到輸入框時節流**：`edit_container` 目前只快取命中，找不到時每次輪詢（約每秒 30 次）都整棵 UI 樹走一遍。加負向快取時間戳（例如 `edit_missing_at: Option<Instant>`，1 秒內不重找），沿用既有 `foreign_checked`／`FOREIGN_RECHECK` 的作法與 `self.clock`；teardown 時重設。補測試：用 spy 的讀取次數，比照 `foreign_address_is_rechecked_after_the_window`。
5. **指標加位移用 wrapping_add**：debug 建置下遊戲記憶體讀到的指標接近 `u64::MAX` 時 `+` 會溢位 panic。改 `wrapping_add`：`src/reader/ui_tree.rs` 的 CHILDREN／NAME／PARENT／FLAGS／RECT 讀取（含 RECT 後續 `at + 4/8/12`）、`src/reader/memory.rs` `read_std_string` 的 `addr + 16`、`src/reader/chat_tabs.rs` 的 `chatbox + CUR_TAB_OFFSET`（兩處）與 `button.addr + LABEL_OFFSET`。動手前再 grep 一次 `src/reader/` 確認沒有漏掉同類（以遊戲讀來的位址加常數）。補一則 `addr = u64::MAX` 的測試確認回錯誤而非 panic。
6. **cave 清空後立刻存修復狀態**：`attach` 目前在 `AutobotCave::install`（清空 cave）之後、root pattern 掃描之後才第一次 `save_hook_state`；掃描期間被強制結束就沒有狀態檔，cave pattern 之後再也找不到。改成 cave 裝好立刻 `self.save_hook_state()`，再裝 root hook；AddLine hook 寫入跳轉後同樣立即存（確認現有順序，縮小「狀態缺 AddLine 跳轉」的窗口）。補測試：在 cave 之後第一次掃描時取狀態快照，斷言等於 `[(CAVE, 原始 208 bytes)]`。`connect_order_matches_python` 需要時一併調整斷言。
7. **合併重複的等待函式**：`src/reader/run_loop.rs` 的 `wait` 與 `wait_for_retry` 只差一個 `window_exists` 條件。`wait` 改成 `wait(clock, stop, duration, mut keep_going: impl FnMut() -> bool)`，`supervisor.rs` 傳 `|| true`，`wait_for_retry` 改呼叫 `wait(..., || (self.env.window_exists)(self.hwnd))` 或直接刪掉改在呼叫端寫。`wait_watching_input` 不動。

## Task 3：翻譯池與服務商 client

1. **翻譯工作不可無限重試**（`src/translation/pool.rs` `Job::run`）：
   - 年齡上限：`Job` 建立時記 `born = tokio::time::Instant::now()`（測試用 paused clock）。每輪 `wait_for_backoff` 之後若 `born.elapsed() > MAX_JOB_AGE`（5 分鐘）就記 log `line dropped after N attempt(s), gave up after outage` 並回報失敗。
   - 單則專屬的 Config 錯誤：`State` 加 `successes: u64`，`note_success` 時遞增。每次送出前讀一次；遇到 Config 錯誤時若計數在這次嘗試期間或之後有變（代表別則成功、設定沒問題），就把這則當失敗放棄，且**不**呼叫 `note_failure`（不動閘門與橫幅）。金鑰過期時沒有任何成功，不會誤丟。
   - 測試（`tests/it/pool.rs`，已是 `start_paused`）：`offline_line_past_max_age_reports_failed`、`config_error_after_another_line_succeeded_is_dropped_without_arming_the_gate`。
   - 更新 `CONFIG_ERROR_INTERVAL` 與 `TranslateError::Config` 文件中「持續重試」的描述。
2. **寫檔先落盤再改名**：`src/util.rs` `write_atomic` 改成 `File::create` + `write_all` + `sync_all()` 再 `fs::rename`。既有測試 `write_atomic_leaves_no_temp_behind` 照舊通過即可。
3. **OpenAI 相容串流中途的錯誤**：把 `src/translation/clients/gemini.rs` 的 `chunk_error` 移到 `clients/mod.rs`（`pub(crate)`），Gemini 改用共用版。`openai.rs` `stream_once` 的 chunk 處理加 `if let Some(e) = chunk.get("error").filter(|e| !e.is_null()) { return Err(chunk_error(e)); }`（OpenRouter 文件：200 之後的錯誤以 `data:` chunk 送出，含頂層 `error` 與 `finish_reason: "error"`；有些後端會送 `"error": null`）。測試（`tests/it/clients_openai.rs`）：先送部分內容再送 error chunk，斷言回 Err 而非把部分譯文當成功；另一則 `"error": null` 不影響正常結果。
4. **Claude 串流 error 事件依類型分類**：`claude.rs` 讀到 `event: error` 時，依 `error.type` 對應狀態碼：`rate_limit_error`→429、`api_error`→500、`timeout_error`→504、`overloaded_error`→529，經既有的 `status_error(code, message)` 分類（成為可重試的 Offline）；其他類型維持 Unexpected。更新 `tests/it/clients_claude.rs` 的 `claude_stream_error_event_is_unexpected`（改名並預期 `Offline{status:Some(529), detail:"Overloaded"}`），加一則 `invalid_request_error` 仍是 Unexpected；修正 `read_message` 的文件註解。
5. **400 學習不可拿掉核心參數**：`openai.rs` `learn_rejection` 的判斷加上 `&& !matches!(param.as_str(), "model" | "messages" | "stream")`，這三個被拒時直接回錯誤。測試 `rejected_core_param_is_returned_not_dropped`：`rejects("stream")` 只發一次請求並回 Config 400。

## Task 4：鍵盤輸入與打字

1. **目標視窗不在時不打字**：`src/composer/paste.rs` 的共用 `type_chars` 開頭：`hwnd == 0 || !(env.is_window)(hwnd)` 就記 `[composer] typing skipped: target window gone (hwnd=..)` 並 return；拿掉 `guarded`（之後一律檢查前景）。`type_into_window` 保留 `target_alive` log。更新文件註解中「`hwnd` 為 0 則打到底」。測試：`paste.rs` 內傳 hwnd 0 並預期有輸出的測試，`setup()` 改給活著的假目標（`set_foreground(&[T])`）；`tests/it/app_main.rs` 約 970 行的 0 改成與 `typing_env` 前景相同的值（如 0x77）；新增「目標為 0／已關閉時什麼都不打」測試。
2. **鎖定螢幕後修飾鍵卡住**：WH_KEYBOARD_LL 只收同一桌面的事件，Win+L／Ctrl+Alt+Del／UAC 安全桌面期間的放開事件會遺失，`pressed` 永遠留著該修飾鍵。新增 `Dispatcher::drop_released_modifiers(&mut self, current: Code, is_down: impl Fn(<修飾鍵群組>) -> bool)`：走過修飾鍵群組（Ctrl／Shift／Alt／Win，左右合併），跳過包含 `current` 的群組（文件：callback 在非同步狀態更新前被呼叫），`!is_down` 的群組就把它的 code 從 `pressed` 移除。`src/input/hook.rs` `should_pass` 在 key-down、取得鎖之後呼叫，傳入以 `GetAsyncKeyState`（通用 VK：VK_CONTROL／VK_SHIFT／VK_MENU、VK_LWIN|VK_RWIN）實作的函式。`held` 不改（一次按放就會自癒）。單元測試在 `dispatcher.rs` 以 closure 注入。有清掉時記一行 log。
3. **hook 被系統移除後自動重掛**：Windows 7 起 callback 逾時會被靜默移除。`hook.rs` 偵測到 `took >= SLOW_CALLBACK` 時 `PostThreadMessageW(GetCurrentThreadId(), WM_APP, 0, 0)`；`GetMessageW` 迴圈收到 `WM_APP` 時 `UnhookWindowsHookEx`（失敗忽略）後用同一個安裝函式重掛，並記 log。多重掛一次無害。若可抽出純函式（例如「是否該重掛」）就補測試，否則在回報說明靠實機驗證。
4. **Ctrl+V 貼上上限 1000 字**（使用者決定）：`paste.rs` 加 `PASTE_MAX_CHARS: usize = 1000`，在 CRLF 正規化之後 `chars().take(n)`；有截斷時記 `[composer] paste truncated chars=.. limit=..`。補一則測試。
5. **貼上只等 Ctrl 放開**：觸發貼上的 V 一定被攔截器吞掉、不會進 `pressed`，`wait_for_paste_keys_released` 等 V 的那一半永遠不成立。改成只等 Ctrl；更新文件註解、log 字樣，以及 `paste_clipboard_waits_for_ctrl_and_v_to_be_released_before_typing`（改名、輪詢次數隨之調整）。
6. **AltGr 常數單一來源**：在 `src/input/keys.rs` 定義 `pub const ALT_GR: Code = 541` 與 `VK_LCONTROL`，`keys.rs`（約 271 行 `(541, 162, …)`）、`hook.rs`（`ALT_GR_SCAN`、`VK_LCONTROL`）、`inject.rs`（`ALT_GR`）、`recorder.rs` 都改用它；刪掉各處重複定義。

## Task 5：overlay 與設定介面

1. **模型下拉選單可捲動**：`src/ui/settings/fields.rs` `editable_combo` 的選項迴圈包進 `egui::ScrollArea::vertical().max_height(ui.spacing().combo_height)`（與 egui `ComboBox` 相同）；方向鍵移動時對目前反白的項目 `scroll_to_me(None)`。補測試：大量選項時彈出區高度不超過上限（或反白項目可見）。
2. **單純點擊不搶焦點**：`src/ui/overlay/selection.rs` 按下時不再 `take_focus()`；改在放開、`finish()` 確認選到文字（`span().is_some()`）時才搶（Ctrl+C 與右鍵選單都需要有選取）。這是刻意偏離 Python 版的改動（使用者已同意）。測試：`press_on_a_line_starts_a_selection_and_takes_focus` 改成按下→拖曳→放開後 `focused == 1`；`a_bare_press_is_not_a_selection` 加斷言 `focused == 0`。
3. **從泡泡展開時拉回螢幕內**：`src/ui/app.rs` `transition()` 回傳 `overlay_rect` 前呼叫 `geometry::pull_onscreen(x, y, w, h, &self.shell.monitors())`，位置有變時記 log（比照 `initial_rect`）。補一則 App 測試：`overlay_rect` 在螢幕外時展開會被拉回。
4. **刪只給測試用的公開方法**：`Banners::clear_update`（`src/ui/overlay/banners.rs`）與 `Selection::dragging()`（`selection.rs`）沒有正式呼叫者；刪掉，同模組測試改讀私有欄位；`clear_update_does_not_count_as_dismissing` 若只為測該方法存在就刪。`region_host.rs` 的 `.dragging()` 是 `RegionBox` 的，不動。
5. **預覽語言後存檔重建兩次字型**：`app.rs` 的 `relabel()` 裡加 `self.ui_language = language.clone();`（或等價），讓 `apply_settings` 的語言比較在預覽之後成為 no-op。測試放 `tests/settings_host.rs`（會切換語言、已獨立成檔）：預覽→存檔後 relabel 只發生一次。

## Task 6：app 主流程的錯誤回饋

1. **存設定失敗要讓使用者看到**：存檔失敗目前只記 log（`src/app_main.rs` `set_then_save`／`save_field`、`apply_settings`，`src/main.rs` 精靈完成）。比照 `hotkeys_unavailable`（`banners.rs` 的旗標＋`error_text()` 附加、`UiEvent::HotkeysUnavailable`）新增 `config_unsaved` 旗標、`UiEvent::ConfigSaveFailed` 與 i18n key `notice.config_unsaved`（en-US／zh-CN／zh-TW 都要翻）。下一次存檔成功時清掉旗標（避免一次性檔案鎖定留下過時橫幅）。`save_field` 的失敗回報方式自行選最小改動（傳 poster 或回傳結果給呼叫端）。`config/mod.rs` 啟動時的正規化重寫維持只記 log。測試：`banners.rs` 單元測試比照 `the_hotkey_line_survives_the_reader_clearing_its_error`；`tests/it/app_main.rs` 加 `apply_settings` 存檔失敗會送出事件的案例。
2. **發話翻譯 panic 不可讓輸入框卡住**：`app_main.rs` `outgoing_translate` 的 task 比照 `PipelineRunner::run` 用 `AssertUnwindSafe(..).catch_unwind()` 包住，panic 轉成 `Err(t("input.failed", error = panic_message))` 並記 log，確保一定會送出 `OutgoingDone`。測試放 `tests/it/app_main.rs`，用會 panic 的假 `ChatBackend`。
