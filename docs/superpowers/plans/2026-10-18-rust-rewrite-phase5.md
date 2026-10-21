# Rust 重寫第 5 段（收尾：刪 Python、搬到最終目錄、CI、發佈流程、文件）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓 `refactor/rust-rewrite` 可以合併並發佈 v1.0.0：
- 刪掉 Python 版；
- Rust crate 搬到 repo 根目錄；
- CI 與發佈流程改用 cargo，exe 帶版本資訊；
- 規則檔與三份 README 改成 Rust 對應；
- 合併前的小問題清掉。

**Architecture:**
- 刪 Python 之前，先做兩件需要 Python 原始碼的事：測試對照稽核、合併前小問題清理。
- 接著在 `rust/` 裡加上 exe 版本資訊與 toolchain 鎖定。
- 再一次性搬目錄。i18n JSON 留在 `src/i18n/`，與 Rust 的 `src/i18n/mod.rs` 放一起，Crowdin 路徑不變；圖示與 OCR 模型集中到根目錄 `assets/`。
- 最後改 CI、發佈 workflow 與文件。

**Tech Stack:** Rust 1.99.1（`rust-toolchain.toml` 鎖定）、cargo、`embed-resource`（已在用）、GitHub Actions（windows-latest）。

**Spec:** `docs/superpowers/specs/2026-10-17-rust-rewrite-design.md`。

- 驗收依據是 §J，以及「分段順序與驗收」表的第 5 列：用 CI 產出的 exe 跑一輪完整功能對照清單。
- 「對照的方法」規定：現有 Python 測試檔的行為都要有對應的 Rust 測試；匯出腳本與 fixture 進版控，Python 刪除後 fixture 留下。
- 也要納入 `docs/superpowers/plans/2026-10-17-rust-rewrite-followups.md` 的「第 5 段」與「合併前」各節。

## Global Constraints

- **提交前檢查依改動範圍**：
  - 只動 `rust/` 時，跑三項 cargo 檢查：`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`。linker 報 1102 時用 `cargo test -j 2`。
  - Task 4 刪掉 Python 之前，有動到 `src/`、`tests/`、`tools/`、`pyproject.toml` 時，另外跑 `uv run ruff check src tests tools` 與 `uv run pytest`。
  - Task 4 之後只剩 cargo 檢查，在 repo 根目錄跑。
  - 只動文件時不必跑。
  - 不可用 `--no-verify`，不可 push。
- **commit**：英文 conventional commits，結尾附 trailer。
- **合併**：PR 以 merge commit 合併；開 PR 前先問使用者。
- **測試不可干擾桌面**：不可裝真的鍵盤 hook、注入按鍵、搶前景、開可見視窗；`#[ignore]` 的 probe 除外。
- **CLAUDE.md 與 AGENTS.md 逐字相同**：改一份就改另一份，同一個 commit，`diff CLAUDE.md AGENTS.md` 無輸出。
- **開發指令同步 9 處**：
  - 2 個 workflow；
  - CLAUDE.md、AGENTS.md 各 2 處；
  - 3 份 README 各 1 處。

  改完用 `grep -rn` 確認舊指令（`uv run`、`ruff`、`pytest`、`pyinstaller`）在這 9 處都沒有了。`docs/superpowers/` 與 `.superpowers/` 是歷史紀錄，不改。
- **README 規則**：
  - 不寫 config 鍵名、不引用設定視窗的選項文字、不寫可自訂的快捷鍵組合；
  - 三份 README 各用自己的語言寫，不要只翻譯英文版。
- **標點依語言慣例**：繁中、簡中用全形，英文用半形。
- **版本號唯一來源**：`Cargo.toml` 的 `version`，程式內用 `env!("CARGO_PKG_VERSION")`。
- **Rust 版本**：`rust-toolchain.toml` 釘 `1.99.1`（含 clippy、rustfmt）；`Cargo.toml` 的 `rust-version = "1.88"`。
- **搬目錄後的測試資料夾**：使用者已決定把 `rust\target\release\_test` 移到 `target\release\_test`。
  - 由 controller 在 Task 4 完成後手動搬，內容不動；
  - 搬之前確認沒有從那裡執行中的 `Wizard101ChatTranslator.exe`；
  - 實作者不碰這個資料夾。

## Review Focus

1. **CI 產出的 exe 少了資源**（圖示、DPI manifest、版本資訊）：使用者下載後看到預設圖示、模糊放大，或檔案內容沒有版本。測試放在 Task 5：CI 步驟 `Verify exe resources`，讀建出來的 exe 的版本資訊與 manifest。
2. **預發布 tag**（`v1.0.0-rc.1`）：`FileVersion` 四位數字是 `1.0.0.0`，`ProductVersion` 字串保留 `1.0.0-rc.1`；workflow 拒絕格式不對的 tag。測試放在：
   - Task 3：`file_version_strips_the_prerelease_suffix`；
   - Task 5：workflow 的 tag 驗證沿用現有正規式。
3. **搬目錄後開發版找不到 `config.json`**：`cargo run` 時 `app_dir()` 要指向 repo 根目錄，不是它的上一層。測試放在 Task 4：`app_dir_in_debug_is_the_repo_root`。
4. **CI runner 的鍵盤配置與本機不同**，`keyboard_*` golden 失敗。處理放在 Task 5：CI 第一次跑若 `keys_golden` 失敗，依失敗內容調整 fixture 或把配置相依的案例標成只在本機跑，並在 commit 說明原因。
5. **文件與 workflow 指令不一致**：貢獻者照 README 跑的指令和 CI 不同。驗證放在 Task 6 最後一步：grep 9 處的指令與 ci.yml 逐字一致。

---

### Task 1: Python 測試對照稽核

**Files:**
- Create: `docs/superpowers/plans/2026-10-18-python-test-parity.md`
- Modify／Create: `rust/tests/*.rs`（補缺的測試）

**Interfaces:** 無（只補測試與文件）。

- [ ] **Step 1:** 列出 `tests/` 底下所有 `test_*.py` 的每一個測試函式（目前 67 個檔案），逐一找出對應的 Rust 測試（檔名::測試名）。
  - 一個 Python 測試可以對應多個 Rust 測試；
  - 已由 golden fixture 覆蓋的，寫出對應的 golden 測試名；
  - 寫成 `docs/superpowers/plans/2026-10-18-python-test-parity.md`：每個 Python 檔一節，每列「Python 測試 → Rust 測試，或『不移植：理由』」。
- [ ] **Step 2:** 下列三類照原樣標「不移植：理由」：
  - Tk 專屬（pack 順序、wraplength、grab）；
  - PyInstaller／splash 專屬；
  - Python 套件專屬（wizwalker 匯入、`test_startup_path_does_not_import_anthropic`）。

  其餘沒有對應的，補 Rust 測試。測試名沿用 Python 名稱去掉 `test_` 前綴，放到負責該模組的測試檔。
- [ ] **Step 3:** 跑三項 cargo 檢查（在 `rust/`），全過。
- [ ] **Step 4:** Commit：`test(rust): close the gaps from the Python test parity audit`

---

### Task 2: 合併前的小問題

**Files:** 依項目而定（`rust/src/**`、`rust/tests/**`、`docs/superpowers/plans/2026-10-17-rust-rewrite-followups.md`）。

**Interfaces:** 無對外變更。

處理範圍：followups 文件的下列各節逐項處理：
- 「第 5 段」的 `real_fingerprint.rs` 文件、`fonts_dir` 空字串 `WINDIR`；
- 「合併前清掉的小問題（第 1 段）」；
- 「第 2 段審查列為可保留的小問題」；
- 「第 3 段」與「第 4 段」的「可保留的小問題」。

使用者偏好是殘留的小問題修掉、不留尾巴。

- [ ] **Step 1:** 逐項判斷「修」或「保留」。保留只限下列兩種，理由寫進 followups 文件的該項：
  - 需要實機才能驗證，例如混合 DPI；
  - 要改架構才能做，例如訊息清單的背景補排版。
- [ ] **Step 2:** 要修的逐項先寫測試再修。其中這幾項一定要修：
  - `fonts_dir` 過濾空字串的 `WINDIR`；測試 `fonts_dir_ignores_an_empty_windir`。
  - `real_fingerprint.rs` 改成先複製到 tempdir 再讀，文件寫實。
  - 泡泡未讀數改用粗體：第 4 段已載入 `FontFamily::Name("bold")`。
  - 分頁列未選取的顏色、palette 其餘顏色補測試。
  - `TimestampedWriter` 邊界、3xx `/v1/models`、exporter exit 2 這類測試缺口補上。exporter 的 Python 端在 Task 4 刪掉，無法補就標保留並寫理由。
  - log 文字與 Python 的細節差異：能對齊的對齊；Python 特有的（例如 `repr` 引號）照 spec「log 不逐字相同」保留。
- [ ] **Step 3:** followups 文件更新：
  - 修掉的項目刪除，並在該節末尾記「已於第 5 段處理」與 commit 範圍；
  - 保留的項目附理由。
- [ ] **Step 4:** 跑三項 cargo 檢查，全過。
- [ ] **Step 5:** Commit（可分數個）：`fix(rust): clear the pre-merge follow-ups` 等。

---

### Task 3: exe 版本資訊與 Rust 版本鎖定

**Files:**
- Create: `rust/rust-toolchain.toml`
  - Task 4 會搬到根目錄。搬之前 rustup 只在 `rust/` 底下才讀得到它，所以在 `rust/` 跑 cargo 時才生效。
- Create: `rust/build_support/version.rs`：純函式，`build.rs` 以 `include!` 引用，測試以 `#[path]` 引用。
- Modify: `rust/build.rs`、`rust/app.rc`（改成由 build.rs 產生到 `OUT_DIR` 的 rc，含 manifest、icon、VERSIONINFO）
- Modify: `rust/Cargo.toml`（`rust-version = "1.88"`）
- Test: `rust/tests/version_info.rs`（新）

**Interfaces:**
- Produces（`rust/build_support/version.rs`）：
  - `pub fn file_version(version: &str) -> [u16; 4]`：去掉 `-` 之後的預發布後綴，三段補成四段，例如 `"1.0.0-rc.1"` 得 `[1,0,0,0]`。
  - `pub fn version_rc(version: &str, manifest: &str, icon: &str) -> String`：產生完整 rc 內容，manifest 與 icon 用絕對路徑。

- [ ] **Step 1:** 寫測試 `rust/tests/version_info.rs`：
  - `file_version_strips_the_prerelease_suffix`：
    - `"1.0.0"` 得 `[1,0,0,0]`；
    - `"1.0.0-rc.1"` 得 `[1,0,0,0]`；
    - `"0.6.12"` 得 `[0,6,12,0]`。
  - `version_rc_matches_the_pyinstaller_strings`：輸出含下列每一項，與 `build.spec` 逐字相同：
    - `VALUE "CompanyName", "GoneTone"`
    - `"FileDescription", "Wizard101 Chat Translator"`
    - `"FileVersion", "<version>"`
    - `"InternalName", "Wizard101ChatTranslator"`
    - `"LegalCopyright", "© 2026 GoneTone"`
    - `"OriginalFilename", "Wizard101ChatTranslator.exe"`
    - `"ProductName", "Wizard101 Chat Translator"`
    - `"ProductVersion", "<version>"`
    - `"Comments", "Wizard101 Chat Translator — an AI translator for Wizard101's in-game chat, translating conversations both ways in real time."`
    - `BLOCK "040904b0"`
    - `VALUE "Translation", 0x409, 1200`
    - `FILEVERSION`／`PRODUCTVERSION` 四位數字。
- [ ] **Step 2:** 跑 `cargo test --test version_info`，預期 FAIL。
- [ ] **Step 3:** 實作。
  - `build.rs` 用 `CARGO_PKG_VERSION` 產生 rc 寫到 `OUT_DIR`，再交給 `embed_resource::compile`；`rerun-if-changed` 照舊。
  - rc 是 UTF-8：`©` 與 `—` 要正確，必要時加 `#pragma code_page(65001)`。
  - 刪掉舊的 `app.rc`。
- [ ] **Step 4:** 建 `rust-toolchain.toml`：`channel = "1.99.1"`、`components = ["clippy", "rustfmt"]`；`Cargo.toml` 加 `rust-version = "1.88"`。
- [ ] **Step 5:** 跑三項 cargo 檢查與 `cargo build --release`，全過。用 PowerShell 讀出 exe 的版本資訊確認：`(Get-Item target\release\Wizard101ChatTranslator.exe).VersionInfo | Format-List *`。
- [ ] **Step 6:** Commit：`build(rust): embed version info and pin the Rust toolchain`

---

### Task 4: 刪除 Python、Rust 搬到根目錄

**Files:**
- Delete:
  - `src/` 底下所有 Python 檔與 `__pycache__`（`src/i18n/*.json` 與 `src/assets/*` 不刪，見下）；
  - `tests/`（Python）；
  - `run.py`、`pyproject.toml`、`uv.lock`、`.python-version`、`build.spec`；
  - `tools/`；
  - `.idea/runConfigurations/*.xml`（三個都是 Python 設定；`.gitignore` 的 `!.idea/runConfigurations/` 一併拿掉）。
- Move（`git mv`）：
  - `rust/Cargo.toml`、`rust/Cargo.lock`、`rust/build.rs`、`rust/build_support/`、`rust/app.manifest`、`rust/rust-toolchain.toml`、`rust/examples/`、`rust/tests/` → 根目錄同名；
  - `rust/src/**` → `src/**`，其中 `rust/src/i18n/mod.rs` → `src/i18n/mod.rs`，與既有的 `src/i18n/*.json` 放一起；
  - `src/assets/*`（圖示）→ `assets/`；
  - `rust/assets/ocr/` → `assets/ocr/`。
- Modify：
  - `src/i18n/mod.rs`：`include_str!` 路徑改成同目錄 `"zh-TW.json"` 等；
  - `src/runtime/resources.rs`：`include_bytes!` 指向 `../../assets/...`；
  - `src/main.rs`：OCR 模型路徑；
  - `build.rs`：icon 與 manifest 路徑；
  - `src/input/keys.rs`：golden 路徑；
  - `src/config/mod.rs`：開發版的 `app_dir()` 改成 `CARGO_MANIFEST_DIR` 本身；
  - `.gitattributes`：`tests/golden/*.json text eol=lf`、`assets/ocr/* -text`；
  - `.gitignore`：拿掉 Python 項目，加 `/target`；保留 `config.json`、`app.log`、`messages.log`、`.idea/*`；
  - README 三份裡的 `src/assets/icon.ico` 改成 `assets/icon.ico`。
- `crowdin.yml` 不變，路徑仍是 `/src/i18n/*.json`。

**Interfaces:** 無對外變更；crate 名稱、模組路徑不變。

- [ ] **Step 1:** 改測試 `app_dir_in_debug_is_the_project_root` 為 `app_dir_in_debug_is_the_repo_root`：
  - `app_dir().join("Cargo.toml")` 存在；
  - `app_dir().join(".git")` 存在；
  - 路徑沒有 `..`。
- [ ] **Step 2:** 搬檔、刪檔、改路徑。
  - 一律 `git mv`，保留歷史；
  - 刪 Python 用 `git rm -r`；
  - `rust/` 最後應該只剩被 gitignore 的 `target/`，不要刪它，`_test` 在裡面。
- [ ] **Step 3:** 在根目錄跑三項 cargo 檢查與 `cargo build --release`，全過。
  - 確認 `rust-toolchain.toml` 在根目錄生效：`rustc --version` 是 1.99.1。
- [ ] **Step 4:** `git status` 確認沒有漏掉的未追蹤檔；`git grep -n "\.\./\.\./src\|rust/" -- '*.rs' build.rs` 沒有殘留的舊路徑。
- [ ] **Step 5:** Commit：`refactor: remove the Python version and move the Rust crate to the repo root`
- [ ] **Step 6（controller，非實作者）：**
  - 確認沒有從 `rust\target\release\_test` 執行中的 exe，把整個資料夾移到 `target\release\_test`；
  - 複製新的 release exe 進去；
  - 更新記憶「Rust exe 測試資料夾」。

---

### Task 5: CI、發佈 workflow、Dependabot

**Files:**
- Modify: `.github/workflows/ci.yml`、`.github/workflows/release-windows.yml`、`.github/dependabot.yml`

**Interfaces:** 無。

- [ ] **Step 1: `ci.yml`**：只留一個 job，在根目錄跑。
  - `dtolnay/rust-toolchain` 改成讀 `rust-toolchain.toml`，用 `actions-rust-lang/setup-rust-toolchain` 或 `rustup show`，不要再寫 stable。
  - 依序：
    1. `Swatinem/rust-cache@v2`（不再指定 workspaces）；
    2. `cargo fmt --check`；
    3. `cargo clippy --all-targets -- -D warnings`；
    4. `cargo test`；
    5. relayout 預算步驟照舊：`cargo test --release -- --ignored relayout`；
    6. `cargo build --release`；
    7. `Verify exe resources`：pwsh 讀 `target\release\Wizard101ChatTranslator.exe` 的 `VersionInfo`，`ProductName`、`FileDescription`、`OriginalFilename` 與 Task 3 相同，`ProductVersion` 等於 `Cargo.toml` 的版本；不符就 throw。
    8. `actions/upload-artifact`：exe，`retention-days: 14`，給使用者做最終驗收。
  - 刪掉 uv、golden `--check` 步驟。
  - 註解說明：測試用 kittest 的離屏 UI，不開可見視窗，所以可以平行跑。
- [ ] **Step 2: `release-windows.yml`**：流程不變（tag 驗證 → 改版本號 → 檢查 → 提交 → 建置 → draft release ＋ footer ＋ SHA-256 → 手動發佈），工具換成 cargo。
  - 版本號：
    - pwsh 改 `Cargo.toml` 第一個 `^version = "..."`；
    - 跑 `cargo update --workspace`，只更新本 crate 在 `Cargo.lock` 的版本；
    - `git status --porcelain Cargo.toml Cargo.lock` 判斷有無變更；
    - commit 這兩個檔，訊息照舊。
  - 檢查：fmt、clippy、test。
  - 建置：`cargo build --release --locked`；exe 路徑 `target\release\Wizard101ChatTranslator.exe`；artifact 路徑同步改。
  - 其餘步驟照舊：draft 建立／更新、footer、step summary 含大小與 SHA-256。
- [ ] **Step 3: `dependabot.yml`**：`uv` 生態系換成 `cargo`（`directory: "/"`，排程、commit-message、cooldown 照舊）；拿掉 pytest group。
- [ ] **Step 4:** 本機無法跑 workflow，改做靜態驗證：
  - YAML 解析：`python -c "import yaml,sys;[yaml.safe_load(open(f,encoding='utf-8')) for f in sys.argv[1:]]" .github/workflows/*.yml .github/dependabot.yml`。沒有 Python 時改用 `pwsh` 的 `ConvertFrom-Yaml`，或請 controller 用 `gh workflow view` 檢查。
  - 把 release workflow 的 pwsh 版本號片段抽出來，在本機對 Cargo.toml 的暫存副本實跑一次，確認改得到、`cargo update --workspace` 之後 Cargo.lock 只動本 crate 那一筆，再還原。
- [ ] **Step 5:** Commit：`ci: build, test and release with cargo`
- [ ] **Step 6:** 交接給 controller：push 之後 CI 第一次跑的結果要回報使用者（push 需使用者同意）。若 `keys_golden` 在 runner 上因鍵盤配置失敗，依 Review Focus 4 處理。

---

### Task 6: 規則檔、README、發佈文件

**Files:**
- Modify: `CLAUDE.md`、`AGENTS.md`（逐字相同）
- Modify: `README.md`、`README_ZH-TW.md`、`README_ZH-CN.md`
- Modify: `docs/releasing.md`
- Modify: `.claude/skills/writing-release-notes/SKILL.md`（若提到 Python／uv／pyinstaller）

**Interfaces:** 無。

- [ ] **Step 1: CLAUDE.md／AGENTS.md**，只改技術對應，規則精神不變：
  - 「wizwalker 掛入須知」改成自寫的掛入模組：
    - 收訊靠自訂 AddLine hook（`src/reader/chat_hook.rs` 的 `PATTERN`）；
    - 輸入框偵測與分頁狀態靠 `src/reader/hooks.rs` 的 root_window hook（`ROOT_WINDOW_PATTERN`）；
    - 遊戲改版報 `PatternFailed` 時，更新對應檔案裡的 pattern（已不依賴 wizwalker fork，也沒有 `uv sync`）。
  - 「新功能要埋 log」：`log(...)` 改指 `src/logs/mod.rs`；其餘照舊。
  - 「模組與公開函式寫 docstring」：改成 Rust 的 `//!` 與 `///` 文件註解，規則（一行摘要、只寫呼叫者必須知道的限制）照舊。
  - 「版本號單一真實來源」：`Cargo.toml` 的 `version`，程式內 `env!("CARGO_PKG_VERSION")`；發佈步驟見 `docs/releasing.md`。
  - 「優先用 uv」改成「用 cargo」：
    - 建置 `cargo build`／`cargo build --release`；
    - 執行 `cargo run`；
    - lint `cargo fmt --check` 與 `cargo clippy --all-targets -- -D warnings`；
    - 測試 `cargo test`；
    - toolchain 由 `rust-toolchain.toml` 鎖定。
  - 「提交前品質檢查」：
    1. fmt＋clippy；
    2. `cargo test`；
    3. 改到收訊／掛入時，開著遊戲用 `cargo run` 實跑。
  - 「新增 AI 服務商要處理 prompt cache」裡指向 `translation/clients.py` 的路徑改成 `src/translation/clients/`。
  - 「翻譯語言不可寫死」裡的 `translator.OUTGOING_LANGUAGE` 改成 Rust 的實際常數名稱，先 grep 確認。
- [ ] **Step 2: 三份 README**：
  - 「開發」章節改成 rustup／cargo：
    - 先決條件：rustup，版本由 `rust-toolchain.toml` 自動切換；
    - 安裝：`cargo build`；
    - 從原始碼執行：`cargo run`，`config.json` 在 repo 根目錄；
    - 檢查：fmt、clippy、test 三行，逐字同 ci.yml；
    - 打包：`cargo build --release`，exe 在 `target\release\Wizard101ChatTranslator.exe`；
    - 紀錄檔：開發版 log 寫到終端機（stderr），`messages.log` 照寫。
  - 「已知限制」裡關於 `uv sync`／fork 的句子改成：遊戲改版時要等本專案更新 pattern 後的新版本。
  - 「使用方式」第 7 點刪掉「遊戲路徑」那段括號，這個設定不存在。
  - 圖示路徑在 Task 4 已改。
- [ ] **Step 3: `docs/releasing.md`**：
  - 版本號來源改 `Cargo.toml`；
  - workflow 步驟描述改成 cargo；
  - 在「確認後按 Publish」之前加一步：下載 draft 的 exe，用 Windows Defender 掃描（`MpCmdRun.exe -Scan -ScanType 3 -File <exe>`，或檔案總管右鍵掃描）。被誤判就到 Microsoft 的誤判提交頁送審，連結 <https://www.microsoft.com/en-us/wdsi/filesubmission>，通過後再發佈。
- [ ] **Step 4:** 驗證：
  - `diff CLAUDE.md AGENTS.md` 無輸出；
  - `git grep -nE "uv run|uv sync|ruff|pytest|pyinstaller|run\.py|pyproject" -- ':!docs/superpowers' ':!.superpowers'` 沒有結果；
  - 9 處的檢查指令與 `ci.yml` 逐字一致。
- [ ] **Step 5:** Commit：`docs: switch the project rules and READMEs to the Rust toolchain`
- [ ] **Step 6（controller）：** 更新記憶「開發指令散在 9 處」「Rust 重寫進行中」等提到 uv／Python 的記憶。

---

## 實機驗收清單（第 5 段完成後交給使用者）

1. 本機 `target\release\_test` 的 exe 跑一輪日常操作：收訊翻譯、發話、框選、設定、首次設定精靈。確認搬目錄沒有改到任何行為，`_test` 裡的設定照舊讀得到。
2. 檔案總管對 exe 按右鍵 → 內容 → 詳細資料：產品名稱、檔案描述、版本、著作權都正確；工作列與 exe 圖示正確。
3. push 分支後（需你同意），CI 全綠；從 CI 的 artifact 下載 exe，照 spec「功能對照清單」A～I 跑一輪完整功能對照：
   - 收訊、多開、切分頁；
   - overlay 全部操作；
   - 發話與貼上；
   - 框選翻譯；
   - 設定與精靈；
   - 更新檢查；
   - 換介面語言；
   - 關閉流程不讓遊戲當掉。
4. 以上都過後，開 PR（merge commit 合併），合併後用 release workflow 發佈 v1.0.0；發佈前照 `docs/releasing.md` 做 Defender 掃描。
