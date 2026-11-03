# 設定視窗儲存流程 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 術語表編輯畫面與服務對話框的〔儲存〕馬上把那一筆寫進設定檔、視窗不關；按 ✕／Alt+F4 關窗時，有沒存的改動先確認。

**Architecture:** 視窗多記一份「已存檔的基準」`saved: Config`，也就是開窗時 `collect().0` 的值。子編輯器存檔時，以 `saved` 為底只換掉那一筆，算出那幾個鍵，再以新的 `SettingsAction::Commit(Config)` 交給 App。App 只走後端的 `on_save`，不關窗。術語表編輯畫面改編輯一份獨立副本。分頁停在編輯畫面時，視窗的底部按鈕列交給 `GlossaryPane::editor_footer` 畫。關窗確認由 `SettingsWindow::request_close` 決定。

**Tech Stack:** Rust、eframe/egui、egui_kittest（測試）、serde_json。

**Spec:** `docs/superpowers/specs/2026-11-03-glossary-save-flow-design.md`

## Global Constraints

- 回覆、註解、docstring、UI 文字用繁體中文全形標點；log 一律英文、前綴 `[settings]`／`[ui]`，帶 id；**API 金鑰不得進 log**。
- 註解預設一行，只寫 WHY；公開項目一行 `///`。
- 新字串補進 `src/i18n/locales/` 下**有翻譯術語表區段的三個語系**：`zh-TW.json`、`en-US.json`、`zh-CN.json`（簡中用 `“”`、英文用 `"`、繁中用 `「」`；三語的句尾標點要一致）。
- 測試一律經 key 取字（`tr(...)`／`t(...)`），不綁翻譯句；新的整合測試放 `tests/it/` 既有模組。
- 測試不可搶焦點、不可開真視窗。
- cargo 一律 `-j 4`，測試加 `-- --test-threads=4`。每個 task commit 前：`cargo fmt --check`、`cargo clippy -j 4 --all-targets -- -D warnings`；全套 `cargo test` 每個 task 只跑一次，平常只跑相關模組。
- Commit message 用英文 conventional commits，結尾附：
  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01WreXjqseqKvtmt9caNxWDh
  ```
- 不要 `git push`。

## Review Focus

1. 在清單刪了 A，再編輯 B 並按儲存：`Commit` 裡的 `glossaries` 仍要有 A，只有視窗〔儲存〕才會刪掉 A。測試放在 Task 3。
2. 新增空白術語表後馬上按〔取消〕：草稿與 `saved` 都不能出現這一份；之後按視窗〔儲存〕也不能出現。測試放在 Task 2。
3. 子編輯器存檔後馬上按 ✕，其他地方都沒改：不能跳確認。也就是 `collect().0` 要和更新後的 `saved` 相等，包括名稱去頭尾空白之後的結果。測試放在 Task 5。
4. 介面語言預覽中按 ✕：要跳確認；按〔放棄〕後語言還原。測試放在 Task 5。
5. 服務對話框開著、但什麼都沒改就按 ✕：不跳確認。也就是剛開啟時 `form.values()` 要等於記下的初值。測試放在 Task 5。

---

### Task 1: GlossaryDraft 拆出單份操作

**Files:**
- Modify: `src/ui/settings/glossary_draft.rs`
- Test: `tests/it/glossary_settings.rs`

**Interfaces:**
- Produces:
  - `GlossaryDraft::prepare_blank(&self) -> StoredGlossary`：配好新 id 與去重名稱的空白術語表，**不加進清單**。
  - `GlossaryDraft::prepare_from_file(&self, name: &str, file: GlossaryFile) -> StoredGlossary`：同上，帶入檔案內容。
  - `GlossaryDraft::upsert(&mut self, glossary: StoredGlossary)`：同 id 就取代，否則加在最後。
  - `pub fn glossary_errors(glossary: &StoredGlossary, index: usize) -> Vec<SaveError>`：單一份的檢查，順序為名稱空白、來源重複、只填一側。`errors()` 改用它；整份清單的順序維持「先所有名稱空白，再所有重複，再所有只填一側」。
  - `pub fn commit_glossary(cfg: &mut Config, glossary: &StoredGlossary)`：`GlossaryDraft::from_config(cfg)`、`upsert`、`write_into(cfg)`。
  - `add_blank`／`add_from_file`／`duplicate` 改成內部呼叫 `prepare_*` 再 push。

- [ ] **Step 1: 寫失敗測試**
  - `prepare_blank_does_not_add_to_the_list`：草稿有一份 `"New glossary"`。`prepare_blank()` 回傳的名稱是 `"New glossary (2)"`（實際寫法依 `unique_name` 的輸出），`d.glossaries.len()` 仍為 1。
  - `upsert_replaces_by_id_or_appends`：對 `"a"` 做 upsert 後，名稱變更且長度不變；對新 id `"c"` 做 upsert 後加在最後。
  - `commit_glossary_replaces_only_that_glossary`：`cfg.glossaries = [a, b]`，提交一份改了名稱的 `b`（名稱前後有空白）。結果 `a` 不變，`b` 的名稱已去頭尾空白；`active_glossary` 不變。
  - `glossary_errors_reports_one_glossary`：名稱空白，加上一條只填一側的條目。回傳的 key 依序為 `error.glossary_name_blank`、`error.glossary_incomplete_entry_unnamed`，`index` 為傳入值加 1。
  - 既有的 `save_blocks_on_glossary_errors` 維持全綠。
- [ ] **Step 2: 跑測試確認失敗**：`cargo test -j 4 --test it glossary_settings -- --test-threads=4`，預期出現編譯錯誤（函式不存在）。
- [ ] **Step 3: 實作上述介面**（`push` 拆成 `prepare(&self, StoredGlossary) -> StoredGlossary` 加 push）。
- [ ] **Step 4: 跑測試確認通過**（同一行指令，全綠）。
- [ ] **Step 5: Commit**：`refactor(settings): split single-glossary helpers out of the glossary draft`

### Task 2: 術語表編輯畫面改用副本，加上自己的底部按鈕

**Files:**
- Modify: `src/ui/settings/glossary_pane.rs`、`src/ui/settings/glossary_editor.rs`
- Modify: `src/i18n/locales/{zh-TW,en-US,zh-CN}.json`
- Test: `tests/it/glossary_settings.rs`

**Interfaces:**
- Consumes: Task 1 的 `prepare_blank`、`prepare_from_file`、`upsert`、`glossary_errors`。
- Produces（`GlossaryPane`）：
  - `pub fn editing(&self) -> bool`
  - `pub fn editing_glossary(&self) -> Option<&StoredGlossary>`：目前副本，給測試與視窗用。
  - `pub fn unsaved(&self) -> Option<SaveError>`：副本與進入時不同，或是新的（進入時沒有原版），就回傳 `("error.glossary_unsaved_edit", [("name", 名稱)])`；名稱空白時回傳 `("error.glossary_unsaved_edit_unnamed", [])`。不在編輯中時回傳 `None`。
  - `pub fn editor_footer(&mut self, ui: &mut Ui, buttons: Id) -> Option<StoredGlossary>`：畫底部按鈕列。左側在被擋過時顯示錯誤 callout（標題 `dialog.incomplete_title`），否則顯示 `glossary.editing`；右側用 `right_aligned(ui, buttons, ...)` 畫〔取消〕（`button.cancel`）、〔儲存〕（`button.save`）。〔儲存〕通過 `glossary_errors` 時，先 `upsert` 進草稿、回到清單，再回傳存好的那份；擋下時記住錯誤，之後隨編輯即時重算，比照視窗的 `errors`。〔取消〕丟掉副本並回到清單。
- 副本存放方式：`Screen::Edit` 改成帶 `{ editor: Box<GlossaryEditor>, copy: StoredGlossary, original: Option<StoredGlossary>, errors: Vec<SaveError> }`（欄位排法由實作者決定）。`GlossaryEditor::ui` 收 `&mut copy`。
- 移除：`EditorAction::Back`、編輯畫面左上的「返回清單」按鈕、字串 `glossary.back_to_list`（三語都刪）。
- 行為：
  - 新增空白、從檔案新增時，用 `prepare_*` 建副本，`original = None`。〔編輯〕時，`original = Some(clone)`。
  - 編輯畫面裡的匯入合併改到副本；匯出用副本內容。
  - log：`[settings] glossary {id} edit saved`、`[settings] glossary {id} edit cancelled`、`[settings] glossary {id} not saved: {keys}`。
- 新字串：

  | key | zh-TW | en-US | zh-CN |
  |---|---|---|---|
  | `glossary.editing` | 正在編輯「{name}」 | Editing "{name}" | 正在编辑“{name}” |
  | `error.glossary_unsaved_edit` | 「{name}」還有沒儲存的編輯。 | "{name}" has unsaved edits. | “{name}”还有没保存的编辑。 |
  | `error.glossary_unsaved_edit_unnamed` | 編輯中的術語表還有沒儲存的編輯。 | The glossary being edited has unsaved edits. | 编辑中的术语表还有没保存的编辑。 |

  名稱空白時，`glossary.editing` 的 `{name}` 帶入 `glossary.default_name`。

- [ ] **Step 1: 改測試 harness**：`harness_with` 的 closure 在 `s.pane.ui(...)` 之後，當 `s.pane.editing()` 為真時，接著呼叫 `s.pane.editor_footer(ui, Id::new("footer"))`，並把回傳值推進 `State` 的新欄位 `committed: Vec<StoredGlossary>`。`custom_entries` 改成讀 `editing_glossary()`。`back_returns_to_the_list` 改名為 `cancel_returns_to_the_list`，改按 `button.cancel`。
- [ ] **Step 2: 寫失敗測試**
  - `editing_does_not_touch_the_draft_until_saved`：在副本加一條條目；`pane.draft()` 仍是原樣。按〔儲存〕後，草稿有新條目、回到清單，`committed` 長度為 1。
  - `cancel_discards_the_edit`：改名後按〔取消〕；草稿維持原名，`committed` 為空。
  - `cancelling_a_new_blank_glossary_adds_nothing`：`add_blank` 後馬上按〔取消〕；`draft().glossaries` 長度不變。（Review Focus 2）
  - `save_with_a_blank_name_stays_and_lists_the_reason`：清空名稱按〔儲存〕；仍在編輯中，畫面上看得到 `localized("error.glossary_name_blank", …)`，`committed` 為空。
  - `unsaved_tracks_changes_and_new_glossaries`：剛按〔編輯〕時 `unsaved()` 為 `None`；改一個字後 key 變成 `error.glossary_unsaved_edit`；`add_blank` 後立刻就是 `Some`。
  - `merge_import_changes_only_the_copy`：在編輯畫面送 `GlossaryFilePicked` 並完成合併；草稿不變，副本多出條目。（沿用既有匯入測試的檔案與事件寫法。）
- [ ] **Step 3: 跑測試確認失敗**：`cargo test -j 4 --test it glossary_settings -- --test-threads=4`
- [ ] **Step 4: 實作**
- [ ] **Step 5: 跑測試確認通過**；`cargo test -j 4 --test it -- --test-threads=4`，確認 `no_hardcoded_ui_text` 與 i18n 相關測試也都綠。
- [ ] **Step 6: Commit**：`feat(settings): edit glossaries on a working copy with their own save and cancel`

### Task 3: 視窗接上術語表即時寫檔

**Files:**
- Modify: `src/ui/settings/window/mod.rs`、`src/ui/app.rs`（`SettingsAction` 的處理，約 `:621`）
- Test: `tests/it/glossary_settings.rs`（用 `window_harness`）

**Interfaces:**
- Consumes: Task 1 的 `commit_glossary`；Task 2 的 `editing`、`editor_footer`、`unsaved`。
- Produces:
  - `SettingsAction::Commit(Config)`：只帶這次提交的鍵，視窗不關。docstring 要寫明：App 只呼叫 `on_save`，**不呼叫** overlay 端的 `App::apply_settings`（部分設定會讓 overlay 的透明度等被重設成預設值）。
  - `SettingsWindow` 的新欄位 `saved: Config`，在 `new()` 結尾設成 `collect().0`，每次 `Commit` 與 `Save` 時更新。
  - `fn commit(&mut self, keys: &[&str])`（私有）：從 `saved` 取出 `keys`，推出 `Commit`，並寫 log `[settings] committed {keys}`。
- 行為：
  - `footer()`：`self.page == Page::Glossaries && self.glossaries.editing()` 時改畫 `self.glossaries.editor_footer(ui, buttons)`。拿到 `Some(g)` 就 `commit_glossary(&mut self.saved, &g)`，再 `commit(&["glossaries", "active_glossary"])`。
  - `collect()`：`self.glossaries.unsaved()` 有值時加進 errors，所以從其他分頁按〔儲存〕會被擋。
  - `save()` 成功時，也把 `saved` 設成這次的 cfg。
  - `app.rs`：`SettingsAction::Commit(cfg) => (wiring.on_save)(cfg)`，不 `close_settings`、不呼叫 `self.apply_settings`。（純轉發，不加測試。）

- [ ] **Step 1: 寫失敗測試**（`window_harness`、`custom_cfg()` 加兩份術語表 `a`、`b`）
  - `saving_an_edit_commits_only_that_glossary`：在清單刪掉 `a`，編輯 `b`、改名後按底部〔儲存〕。動作裡恰好有一個 `Commit(cfg)`，且沒有 `Save`／`Cancel`。這個 cfg 的鍵只有 `glossaries`、`active_glossary`；`glossaries` 裡 `a` 還在，`b` 已改名。（Review Focus 1）
  - `editor_footer_replaces_the_window_buttons`：在編輯畫面時，看得到 `glossary.editing`，而且〔取消〕〔儲存〕各只有一顆；切到基本分頁後，就恢復視窗按鈕。
  - `window_save_is_blocked_by_an_unsaved_glossary_edit`：編輯 `b` 並改一個字，切到基本分頁按〔儲存〕。沒有 `Save`，畫面上看得到 `localized("error.glossary_unsaved_edit", …)`。
  - `window_save_goes_through_when_the_editor_is_unchanged`：開了 `b` 的編輯但沒改，切到基本分頁按〔儲存〕；有 `Save`。
- [ ] **Step 2: 跑測試確認失敗**：`cargo test -j 4 --test it glossary_settings -- --test-threads=4`
- [ ] **Step 3: 實作**
- [ ] **Step 4: 跑測試確認通過**；再加跑 `settings_window`、`local_settings` 兩個模組。
- [ ] **Step 5: Commit**：`feat(settings): write a saved glossary to the config file without closing the window`

### Task 4: 服務對話框即時寫檔

**Files:**
- Modify: `src/ui/settings/service_list.rs`、`src/ui/settings/window/mod.rs`
- Test: `tests/it/service_list.rs`、`tests/it/settings_window.rs`

**Interfaces:**
- Consumes: Task 3 的 `saved`、`commit`。
- Produces（`ServicePane`）：
  - `ui` 多一個參數：`check: &dyn Fn(&Value) -> Vec<SaveError>`，放在最後。表單欄位檢查過了之後，拿要存的服務呼叫它；有錯誤時，留在對話框並列出原因。
  - `pub fn take_committed(&mut self) -> Option<Value>`：這一幀存好的服務，讀走就清掉。
  - `pub fn unsaved(&self) -> bool`：表單開著，且 `form.values()` 不等於開啟當下記下的 `form.values()`。
  - `Dialog::Form` 的 `errors` 改成 `Vec<SaveError>`，用 `localized(key, args)` 畫。即時重算時，用欄位錯誤加上欄位全過時的 `check(&form.values())`。
  - 〔確定〕改用 `button.save`。`button.ok` 若沒有別處在用，三語都刪。
- 視窗：
  - `fn with_service(saved: &Config, service: &Value) -> Config`（私有）：clone、`ServiceDraft::from_config`、`apply_dialog_result`、`write_into`。
  - `check` = `|s| { let cfg = with_service(&self.saved, s); service_errors(&cfg) 轉成 SaveError，再接上 local_errors(&cfg, &self.snapshot) }`。
  - `take_committed()` 拿到服務時，`self.saved = with_service(&self.saved, &s)`，再 `commit(&["services", "default_service", "service_slots"])`。
  - log：`[settings] service {id} committed`（**不記 api_key**）。

- [ ] **Step 1: 改既有測試**：`tests/it/service_list.rs` 的 harness 傳 `&|_| Vec::new()`；原本找 `button.ok` 的地方改成 `button.save`。
- [ ] **Step 2: 寫失敗測試**
  - `service_list.rs`：`a_failing_check_keeps_the_dialog_open`。`check` 固定回傳 `[("error.need_service".into(), vec![])]`；按〔儲存〕後對話框還在、看得到該錯誤，`take_committed()` 為 `None`。
  - `service_list.rs`：`saving_hands_the_service_over_once`。通過後 `take_committed()` 第一次是 `Some`，第二次是 `None`。
  - `settings_window.rs`：`saving_a_service_commits_only_the_service_keys`。先在基本分頁改目標語言，再編輯服務名稱並按〔儲存〕。恰好有一個 `Commit`，鍵只有那三個，且不含 `target_language`；沒有 `Save`／`Cancel`。
- [ ] **Step 3: 跑測試確認失敗**：`cargo test -j 4 --test it -- service_list settings_window --test-threads=4`
- [ ] **Step 4: 實作**
- [ ] **Step 5: 跑測試確認通過**；再加跑 `wizard`、`local_settings`，確認精靈沒被波及。
- [ ] **Step 6: Commit**：`feat(settings): save a service to the config file from its dialog`

### Task 5: 關窗前確認

**Files:**
- Modify: `src/ui/settings/window/mod.rs`、`src/ui/settings_host.rs:283-292`
- Modify: `src/i18n/locales/{zh-TW,en-US,zh-CN}.json`
- Test: `tests/it/settings_window.rs`

**Interfaces:**
- Consumes: Task 2 的 `GlossaryPane::unsaved`；Task 4 的 `ServicePane::unsaved`；Task 3 的 `saved`。
- Produces（`SettingsWindow`）：
  - `pub fn has_unsaved(&self) -> bool`：`collect().0 != self.saved`，或 `glossaries.unsaved().is_some()`，或 `services.unsaved()`。
  - `pub fn request_close(&mut self, env: &dyn SettingsEnv) -> Vec<SettingsAction>`：沒有未存的改動時，等同 `cancel(env)`。有的話，設 `confirm_close = true` 並回傳空 Vec。已經在問時重複呼叫，結果不變。
  - `ui()` 在 `confirm_close` 時畫 `dialog::confirm(ui, "confirm_close", t("dialog.discard_changes", &[]), |ui| danger_button(ui, &t("button.discard", &[])))`。`Done` 時把 `cancel(env)` 的動作推進 `pending`；`Cancel` 時清掉旗標。
  - log：`[settings] close blocked: unsaved changes`、`[settings] unsaved changes discarded`、`[settings] close cancelled`。
- `settings_host.rs`：`closing && !done` 改呼叫 `self.request_close(env)`。結果裡沒有 `Cancel` 時，送 `ui.ctx().send_viewport_cmd(ViewportCommand::CancelClose)`。視窗〔取消〕按鈕仍走 `cancel()`，不確認。精靈（`Wizard`）不動。
- 新字串：

  | key | zh-TW | en-US | zh-CN |
  |---|---|---|---|
  | `dialog.discard_changes` | 有變更還沒儲存，要放棄這些變更並關閉設定嗎？ | You have unsaved changes. Discard them and close settings? | 有更改还没保存，要放弃这些更改并关闭设置吗？ |
  | `button.discard` | 放棄 | Discard | 放弃 |

- [ ] **Step 1: 寫失敗測試**（直接呼叫 `h.state_mut().win.request_close(&env)`，不模擬 OS 關窗）
  - `closing_an_unchanged_window_cancels_at_once`：回傳 `[SettingsAction::Cancel]`。
  - `closing_with_a_changed_field_asks_first`：改目標語言後呼叫；回傳空 Vec，畫面看得到 `dialog.discard_changes`。按〔放棄〕後動作含 `Cancel`。
  - `keeping_the_window_closes_only_the_question`：同上，但按對話框的〔取消〕；動作裡沒有 `Cancel`，問句消失，目標語言的改動還在。
  - `closing_right_after_saving_a_glossary_does_not_ask`：編輯術語表、名稱打成 `"  b2 "` 後按〔儲存〕，接著 `request_close`；回傳 `[Cancel]`。（Review Focus 3）
  - `closing_during_a_language_preview_asks_and_discard_restores_it`：沿用 `tests/settings_language.rs` 的寫法，放在**那個檔案**（會切全域語言）。預覽另一種語言後 `request_close` 回傳空 Vec；按〔放棄〕後動作是 `[PreviewLanguage, Cancel]`。（Review Focus 4）
  - `an_untouched_service_dialog_does_not_count`：開服務的〔編輯〕對話框但不改，`has_unsaved()` 為 false；改名稱後變成 true。（Review Focus 5）
  - `a_new_glossary_counts_as_unsaved`：`add_blank` 後，`has_unsaved()` 為 true。
- [ ] **Step 2: 跑測試確認失敗**：`cargo test -j 4 --test it settings_window -- --test-threads=4`，以及 `cargo test -j 4 --test settings_language -- --test-threads=4`
- [ ] **Step 3: 實作**
- [ ] **Step 4: 跑測試確認通過**；最後跑一次全套 `cargo test -j 4 -- --test-threads=4`，加上 fmt 與 clippy。
- [ ] **Step 5: Commit**：`feat(settings): ask before closing the window with unsaved changes`

### 實機驗證（交給使用者）

全部 task 完成後，`cargo build --release`，把 exe 複製到 `_exe_test`，請使用者實測以下幾項：
- 按 ✕ 被擋下時，視窗確實留著，不會閃一下、也不會消失。
- 術語表或服務存檔後，翻譯馬上套用新內容。
- 存檔過程中錄熱鍵不受影響。

不自己操作桌面。
