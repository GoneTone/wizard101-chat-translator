# 第三輪稽核修正計畫（2026-10-20）

來源：補查前兩輪沒讀到的模組＋效能檢查，主控已對照原始碼抽查。分支 `fix/audit-3`，從 `refactor/rust-rewrite`（4e760e1）切出。使用者決定全部處理；第 6、8 項改變 Python 版刻意保留的行為，使用者已同意，第 8 項採「看名稱第一個字母」的規則（理由見 Task 1）。

## Global Constraints（每個 task 都適用）

- 先讀 `CLAUDE.md`。實作註解預設一行、只寫 WHY，三行是上限不是目標；docstring 一行摘要；CJK 全形標點；log 一律英文、沿用既有前綴；不可寫死語言；API 金鑰不可進 log。
- 行號是查證當下的參考值，動手前以實際程式為準；描述與程式不符時以程式為準並在回報說明。
- 修 bug 先寫會失敗的測試、確認失敗，再修。純刪除靠現有測試把關。golden 檔（`tests/golden/*.json`）只改這次刻意改變行為的案例，並在回報列出改了哪些。
- 測試不可碰真實桌面、鍵盤 hook、剪貼簿或執行中的遊戲；不可啟動 GUI 或截圖。新的整合測試放 `tests/it/`。
- 每個 task 結束前：`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、完整 `cargo test` 各跑一次，全綠才 commit。迭代時只跑聚焦測試。
- Commit：英文 conventional commits，可分多個；結尾附派工說明給的 attribution 兩行。只 `git add` 改到的檔案。不 push。

---

## Task 1：翻譯後處理（src/translation/postprocess.rs）

1. **發送者重複**：`restore_sender` 以開頭中括號組數判斷模型是否丟掉發送者。原文 `[Amy] [WTS] gear`、譯文 `[Amy] 出售裝備`（或 `[Amy] 【出售】裝備`）時組數 1 < 2，結果變 `[Amy] [Amy] 出售裝備`。修法：比對組數之前，若譯文已以原文的同一個 `[發送者]` 開頭（容許其後空白），就換成原文那一個並保留其餘內容，不再補。補 golden／單元測試：上述兩個案例＋既有「模型丟掉發送者」「內容本身以 `[WTS]` 開頭」案例照舊。
2. **只有結尾 `</think>`**：`strip_think` 只移除成對的 `<think>…</think>`。自架推理模型（如 DeepSeek-R1 系列、未設 reasoning parser 的 vLLM）常只輸出 `推理…</think>\n\n譯文`，整段推理被當成譯文。修法：先移除成對區塊；之後若仍有 `</think>`（大小寫與 `ı/İ` 規則同 `THINK_BLOCK`），丟掉最後一個 `</think>` 以前的全部內容與其後空白。更新 golden 中 `"no open </think> tag"` 案例為新行為，並加「成對區塊＋孤立結尾」「沒有任何標籤」不變的案例。
3. **標點造成誤判漏翻**：`has_stray_latin` 的 `LATIN_RUN` 把 `.`、`,`、空白吃進同一段，原文 `Kai and Bob joined`、譯文 `Kai、Bob 加入了` 或 `Kai, Bob님이…` 時片段 `kai, bob` 不在原文裡，被判漏翻 → 多一次重翻且永不進快取。修法：拿每段拉丁片段去比對前，拆成單字（以非字母數字切開、去空），只要有一個單字不在原文（正規化後）才算漏翻；整段都是拉丁、沒有目標語言字的規則不變。補測試；確認 `tests/golden/postprocess_stray.json` 既有案例結果不變（若有案例因此改變，列出並說明為何新結果才正確）。
4. **目標語言名稱夾帶英文**：`uses_latin_script` 只要名稱有任何拉丁字母就算拉丁語言，`中文 (Chinese)` 會關掉漏翻檢查。修法：看名稱裡**第一個字母**（`\p{L}`，略過空白、數字、標點）屬於哪種文字：是拉丁字母（含 `\u{24F}` 以內與 `\u{1E00}-\u{1EFF}` 的帶重音字母，沿用 `NON_LATIN_LETTER` 的範圍定義）才算拉丁語言；沒有任何字母時維持現狀回 `false`。理由：`English (英文)` 這類拉丁開頭的名稱仍是拉丁，不會讓每句英文譯文都被判漏翻；`中文 (Chinese)`、`日本語 (Japanese)`、`繁體中文 (zh-TW)` 改成非拉丁。更新 `tests/golden/postprocess_latin.json` 的 `中文 (Chinese)` 為 `false`，並加 `English (英文)` → `true`、`日本語 (Japanese)` → `false`。內建語言清單（`ui/settings/language.rs` 的 `COMMON_LANGUAGES`）每一項的結果都必須不變（寫測試逐項斷言與舊規則相同）。

## Task 2：框選結果卡與選取層（src/ui/region/）

1. **點掉複製選單時卡片也關掉**：`card.rs` 按下迴圈只略過落在選單內的按下；選單開著時在選單外按一下，放開被當成點擊 → 關閉卡片，譯文要重跑 OCR＋翻譯。修法：迴圈前記下選單是否開著（`CopyMenu` 需要一個 `is_open()`，一行），開著時這次按下不算點卡片（`continue`），讓選單自己處理關閉。overlay 選取的同類行為已正確，不動。補測試：右鍵開選單 → 在卡片內文外按放 → 沒有 `Close`，選單關閉。
2. **連結一按下就開**：`card.rs` 在按下當下就 `open_url`，拖曳選取網址時瀏覽器會先跳出來搶焦點。修法：按下時只記住連結（欄位存 URL），放開且 `is_click` 成立才開；拖曳就不開。更新 `clicking_a_link_opens_it_and_closes_the_card`（開啟改在放開後斷言；行為：點連結開網址並關卡片照舊），加「在連結上按下並拖曳選字不會開網址」測試。log 文字照舊。
3. **寬或高為 0 的框**：`selector.rs` `finish` 只在兩軸都小於門檻時取消，拖出 200×0 會變成 `Selected` 而後回報「不在遊戲畫面內」。修法：`w == 0 || h == 0` 時回 `Cancelled`。補測試。
4. **刪只有測試在用的公開項目**：`src/ui/richtext.rs` 的 `LINK_COLOR`（全樹沒有引用就刪）；`src/ui/overlay/model.rs` 的 `OverlayModel::client()` 只有測試用 → 改 `#[cfg(test)]` 或讓測試改讀既有欄位後刪除（擇較少改動者）。

## Task 3：效能與資源（OCR、泡泡、讀取、翻譯池、建置、圖示）

1. **OCR 佔滿所有核心**：rten 執行緒池預設用全部實體核心，每次框選 OCR 0.3～1.9 秒內與遊戲搶 CPU。修法：`src/main.rs` 最開頭（任何執行緒建立之前）若使用者沒設 `RTEN_NUM_THREADS`，就設成 `max(1, available_parallelism / 2)`；記一行 log（英文，`[region]` 或 `[app]` 前綴）。確認 rten 0.27 確實讀這個環境變數、且是在第一次建立執行緒池時讀（看 `~/.cargo/registry` 下 rten 的 `threading.rs`）。
2. **OCR 把小圖放大十幾倍**：`src/region/ocr.rs` 的 `with_limit_side_len(736)` + `with_limit_type("min")` 讓 625×60 的裁切放大到約 7680×736。先用樣本驗證：`W101_OCR_SAMPLES=%LOCALAPPDATA%\Temp\w101-ocr-spike\images` 跑 `cargo test --release --test it ocr::samples_match_expected_text -- --ignored`，記錄現況（應全過）與每張耗時。再試改成放大倍數上限（例如短邊目標不變，但整體放大後長邊不超過 2000 或倍數不超過 3；以 rusto-rs 0.3.1 能設定的參數為準，必要時在我們這端先縮放影像），4 張樣本**全部仍與期望文字相同**才保留，並在回報附前後耗時；任何一張改變就放棄這一項，只留第 1 點，回報結果。
3. **泡泡每秒重繪 3.3 次**：`src/ui/overlay/bubble.rs` `POLL_INTERVAL` 300 ms 改為 2 s（啟用事件本來就當幀觸發展開，輪詢只是保險）。更新受影響的測試期望值；docstring 照舊說明用途。
4. **讀取畫面期間每 0.4 秒整棵 UI 樹搜尋**：`src/reader/mem_reader.rs` `refind_chatbox` 在上次沒找到時，比照 `edit_container` 的 `edit_missing_at`／`INPUT_RECHECK` 加負向快取（1 秒內不重找；找到或 teardown 時重設）。補測試（用 spy 讀取次數，比照既有的輸入框節流測試）。
5. **服務中斷時排隊工作每 0.5 秒醒來**：`src/translation/pool.rs` `wait_for_backoff` 改成睡到 `gate_until`（仍可被 stop 取消、閘門被延後時重新計算），不再固定 500 ms 切片輪詢；行為（重試時機、年齡上限、Config 讓出）不變，既有 pool 測試全過。若改動需要大幅重寫等待邏輯，就停在最小改動並回報。
6. **release 最佳化**：`Cargo.toml` 加 `[profile.release]` `lto = "fat"`、`codegen-units = 1`（**不要**加 `panic = "abort"`：鍵盤 hook 與各處理器依賴 `catch_unwind`）。量改前後 exe 大小與 `cargo build --release` 時間寫進回報；同步檢查 `docs/releasing.md`／CI 是否有需要說明的地方（只在確實受影響時改）。
7. **泡泡圖示高縮放模糊**：`assets/icon.ico` 內含 64、128 等尺寸。從 ico 取出 96px（或用 128px 縮到 96，以清晰為準）存成 `assets/icon_96.png`，`src/runtime/resources.rs` `icon_png` 加 96，`icon_pngs_have_native_sizes` 測試一併涵蓋；`src/ui/overlay/bubble.rs` 在 `pixels_per_point > 1` 時用 96px。
