# 必填欄位標示星號 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 設定視窗與精靈裡的必填欄位，標籤後接錯誤紅的 `*`。

**Architecture:** `widgets.rs` 新增共用的 `required_label`（把標籤和紅色星號組成同一個 `LayoutJob`，再把無障礙名稱覆寫回不含星號的原文），另外新增兩個 `required_setting_row*` 變體。服務表單的 `form_row`、精靈的 `heading`、術語表欄名改用這個 helper。API 金鑰要不要標星號，跟存檔驗證共用新的 `Provider::requires_api_key`。

**Tech Stack:** Rust、eframe／egui 0.37（accesskit）、egui_kittest。

**Spec:** `docs/superpowers/specs/2026-11-02-required-field-asterisk-design.md`

## Global Constraints

- 星號是 `*`，顏色 `theme::ERROR`，字型與大小沿用該標籤，與標籤隔四分之一字寬。
- 標籤的無障礙名稱（accesskit Label 的 value）維持原文，不含星號；既有 `get_by_label(tr(...))`／`get_by_role_and_label(Role::Label, ...)` 測試一律不改。
- 不動語系檔、不改驗證邏輯（`Provider::requires_api_key` 是把既有條件抽出共用，行為不變）。
- 純畫面改動，不新增 log。
- 註解、docstring 依 CLAUDE.md：繁中全形、一行為原則；commit message 用英文 conventional commits。
- cargo 一律 `-j 4`，測試加 `-- --test-threads=4`；全套 `cargo test` 每個 task 只跑一次。
- 要標的欄位（en-US 文字）：服務表單 `Name`、`Provider`、`Model`、`API key`（只在官方服務商）、`Server URL`（自訂端點）；基本設定 `Translate messages into`；服務頁 `Default service`；術語表編輯 `Name`、`Source`、`Target`；精靈 `Target language (what incoming messages are translated into)`。其他標籤都不標。

## Review Focus

1. 無障礙名稱被星號污染 → 螢幕報讀多念符號、`get_by_label` 找不到欄位。Task 1 測 Label 節點的 value，既有測試當迴歸。
2. 服務商從官方切到自訂端點 → `API key` 的星號要消失，`Server URL` 要出現。Task 2 測。
3. 本地模型表單沒有金鑰欄 → 只有 `Name`、`Provider`、`Model` 三個星號。Task 2 測。
4. 標籤很長、視窗很窄時換行 → 星號跟在最後一行後面，不能單獨掉到下一行開頭或撐寬版面。既有的 `every_page_fits_the_narrowest_window` 當迴歸；精靈那行長標籤留給實機目視。
5. 術語表欄名改用 helper 後 → 跟輸入框的對齊不能跑掉。既有的 `column_captions_line_up_with_their_fields` 當迴歸。

---

### Task 1: 共用的星號標籤與 `required_setting_row`

**Files:**
- Modify: `src/ui/settings/widgets.rs`（`setting_row`／`setting_row_with` 附近，約 :582-:690）
- Modify: `tests/common/kittest.rs`
- Test: `tests/it/settings_widgets.rs`

**Interfaces:**
- Produces:
  - `pub(super) fn required_label(ui: &mut Ui, text: RichText) -> Response`：`text` 用 `RichText::append_to` 寫進 `LayoutJob`，後面接同字型、`theme::ERROR` 色的 `"*"`，以 `Label::new(job).wrap()` 加入。之後呼叫 `response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, ui.is_enabled(), <text 原文>))`，把 accesskit value 覆寫回原文。
  - `pub fn required_setting_row(ui: &mut Ui, label: &str, hint: Option<&str>, control: impl FnOnce(&mut Ui))`
  - `pub fn required_setting_row_with(ui: &mut Ui, label: &str, details: impl FnOnce(&mut Ui), control: impl FnOnce(&mut Ui))`
  - `setting_row`、`setting_row_with` 的簽名不變；四個函式都委派給私有的 `fn labeled_row(ui, label: &str, required: bool, details, control)`。`required` 為真時，在 :648 用 `required_label(ui, RichText::new(label))` 取代 `Label::new(label).wrap()`。
  - 測試 helper `pub fn starred_labels<S>(h: &Harness<S>) -> BTreeSet<String>`（`tests/common/kittest.rs`）：畫面上最後一段文字是 `"*"`、顏色是 `theme::ERROR` 的文字形狀，回傳去掉星號的文字。

- [ ] **Step 1: 開分支並寫失敗測試**

`git switch -c feat/required-field-asterisk`。在 `settings_widgets.rs` 新增測試：

```rust
#[test]
fn required_setting_row_stars_the_label_but_keeps_its_name() {
    let mut h = harness(String::new(), |ui, text: &mut String| {
        card(ui, |ui| {
            required_setting_row(ui, "Target language", None, |ui| {
                editable_combo(ui, Id::unique("target"), text, &[]);
            });
            setting_row(ui, "Interface language", None, |ui| {
                ui.label("English");
            });
        });
    });
    h.run_steps(2);
    assert_eq!(starred_labels(&h), BTreeSet::from(["Target language".to_owned()]));
    let sections = text_sections(&h, "Target language*");
    assert_eq!(sections.last().unwrap(), &("*".to_owned(), theme::ERROR));
    assert_eq!(h.get_by_label("Target language").accesskit_node().role(), Role::TextInput);
    h.get_by(|n| n.role() == Role::Label && n.value().as_deref() == Some("Target language"));
}
```

- [ ] **Step 2: 確認測試失敗**

Run: `cargo test -j 4 --test it settings_widgets::required_setting_row -- --test-threads=4`
Expected: 編譯失敗，找不到 `required_setting_row`／`starred_labels`。

- [ ] **Step 3: 實作上面 Interfaces 列出的函式**

- [ ] **Step 4: 確認測試通過**

Run: 同 Step 2。Expected: PASS。

- [ ] **Step 5: 跑完整檢查後 commit（spec 與本計畫一起進這個 commit）**

Run: `cargo fmt --check`、`cargo clippy -j 4 --all-targets -- -D warnings`、`cargo test -j 4 -- --test-threads=4`。Expected：都零錯誤、零警告、全綠。

```bash
git add src/ui/settings/widgets.rs tests/common/kittest.rs tests/it/settings_widgets.rs docs/superpowers/specs/2026-11-02-required-field-asterisk-design.md docs/superpowers/plans/2026-11-02-required-field-asterisk.md
git commit -m "feat(settings): add a red asterisk label for required setting rows"
```

---

### Task 2: 服務表單的星號

**Files:**
- Modify: `src/config/services.rs:48-55, :292-305`
- Modify: `src/ui/settings/service_form.rs`（`form_row` :923 與所有呼叫點 :320、:327、:422、:447、:469、:517、:610、:645；`key_row` :441）
- Test: `tests/it/service_form.rs`

**Interfaces:**
- Consumes: `required_label`（Task 1）、`starred_labels`（Task 1）。
- Produces:
  - `Provider::requires_api_key(&self) -> bool`：`needs_api_key() && !needs_base_url()`。`validate_endpoint_fields` 原本用 `needs_api_key && !needs_base_url` 判斷，改成 `p.is_none_or(Provider::requires_api_key)`；未知服務商仍視為要金鑰，行為不變。
  - `fn form_row<R>(ui: &mut Ui, label: &str, required: bool, field: impl FnOnce(&mut Ui, Id) -> R) -> R`：`required` 為真時，標籤改用 `required_label(ui, RichText::new(label).size(theme::CONTROL_TEXT_SIZE))`。
  - 傳 `true` 的有：名稱、伺服器網址、服務商、兩處模型列。金鑰列傳 `self.provider.requires_api_key()`，標籤 key 也改由同一個判斷挑（`field.api_key` 或 `field.api_key_optional`）。思考開關（`""`）與思考深度傳 `false`。

- [ ] **Step 1: 寫失敗測試**

在 `service_form.rs` 新增三個測試，沿用檔內的 `harness`／`blank`／`form`／`local_harness`：

- `official_provider_stars_name_provider_key_and_model`：`harness(blank())`，斷言 `starred_labels(&h) == {"Name", "Provider", "API key", "Model"}`。
- `custom_endpoint_stars_the_url_but_not_the_optional_key`：`harness(form(new_service("custom", &[]), vec![]))`，斷言集合 `== {"Name", "Provider", "Server URL", "Model"}`，而且 `painted_texts(&h)` 含有 `tr("field.api_key_optional")`。
- `local_form_stars_only_name_provider_and_model`：`local_harness(vec![])`，斷言集合 `== {"Name", "Provider", "Model"}`。

- [ ] **Step 2: 確認測試失敗**

Run: `cargo test -j 4 --test it -- --test-threads=4 stars`
Expected: FAIL，集合是空的。

- [ ] **Step 3: 實作 `requires_api_key` 與 `form_row` 的 `required` 參數，並更新所有呼叫點**

- [ ] **Step 4: 確認新測試與 `service_form`／`service_list`／`config_golden` 既有測試都通過**

Run: `cargo test -j 4 --test it service_ -- --test-threads=4`，再跑 `cargo test -j 4 --test it config_golden -- --test-threads=4`。
Expected: PASS（既有的 `get_by_label(tr("field.service_name"))` 等不需修改）。

- [ ] **Step 5: 跑完整檢查後 commit**

Run: fmt／clippy／完整 `cargo test`（同 Task 1 Step 5）。

```bash
git add src/config/services.rs src/ui/settings/service_form.rs tests/it/service_form.rs
git commit -m "feat(settings): star required fields in the service form"
```

---

### Task 3: 目標語言、預設服務、術語表與精靈的星號

**Files:**
- Modify: `src/ui/settings/window/basic.rs:24`（目標語言改 `required_setting_row`）
- Modify: `src/ui/settings/service_list.rs:95`（預設服務改 `required_setting_row`）
- Modify: `src/ui/settings/glossary_editor.rs:74`（名稱改 `required_setting_row_with`）、`:206-:211`（`caption` 改用 `required_label(ui, RichText::new(...).size(theme::HINT_SIZE).color(theme::TEXT_SECONDARY))`，仍包在 `fixed_cell` 裡）
- Modify: `src/ui/settings/wizard/steps.rs:248`（`heading` 加 `required: bool` 參數；只有 :194 的目標語言傳 `true`）
- Test: `tests/it/settings_window.rs`、`tests/it/glossary_settings.rs`、`tests/it/wizard.rs`

**Interfaces:**
- Consumes: `required_setting_row`、`required_setting_row_with`、`required_label`、`starred_labels`（Task 1）。

- [ ] **Step 1: 寫失敗測試**

- `settings_window.rs` 的 `basic_page_stars_only_the_target_language`：`harness(configured_cfg())` 在基本頁，斷言 `starred_labels == {tr("settings.target_language")}`。
- `settings_window.rs` 的 `services_page_stars_the_default_service`：`go(&mut h, "settings.tab.services")` 切到服務頁，斷言集合 `== {tr("service.default")}`。
- `glossary_settings.rs` 的 `glossary_editor_stars_name_and_both_columns`：`editing(vec![entry("Hollo", "哈樓")], None)`，斷言集合 `== {tr("glossary.name"), tr("glossary.source"), tr("glossary.target")}`。
- `wizard.rs` 的 `prefs_step_stars_the_target_language`：`to_prefs(blank_cfg())`，斷言集合含 `tr("wizard.target_language")`，而且不含 `tr("field.ui_language")`。

- [ ] **Step 2: 確認測試失敗**

Run: `cargo test -j 4 --test it -- --test-threads=4 stars`
Expected: 上述四個 FAIL，Task 2 的三個仍 PASS。

- [ ] **Step 3: 把上面 Files 列出的四處改成必填版本**

- [ ] **Step 4: 確認新測試與既有的對齊、窄視窗測試都通過**

Run: `cargo test -j 4 --test it -- --test-threads=4 stars column_captions every_page_fits`（libtest 接受多個過濾字串）
Expected: PASS。

- [ ] **Step 5: 跑完整檢查後 commit**

Run: fmt／clippy／完整 `cargo test`（同 Task 1 Step 5）。

```bash
git add src/ui/settings/window/basic.rs src/ui/settings/service_list.rs src/ui/settings/glossary_editor.rs src/ui/settings/wizard/steps.rs tests/it/settings_window.rs tests/it/glossary_settings.rs tests/it/wizard.rs
git commit -m "feat(settings): star the target language, default service and glossary fields"
```
