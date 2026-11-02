# 本地模型 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 新增「本地」翻譯服務商：本程式下載 llama.cpp 官方 Windows 執行檔與 Hugging Face 上的 GGUF 模型，在背景跑 `llama-server.exe` 子程序，翻譯經本機埠走現有的 `OpenAiCompatClient`。

**Architecture:** 新模組 `src/local/` 分成純邏輯（HF 檔案分類、GGUF 檔頭、硬體解析、相容判斷、推薦清單）與 I/O（下載、引擎安裝、模型存放、子程序、執行期），由 `local::Local` 這個 hub 統一對外：設定介面每幀讀它的快照並送指令，翻譯層的 `LocalClient` 每次請求向它要 base_url。引擎與已下載模型以磁碟為準（每個資料夾一份 manifest），`config.json` 只存使用者設定（預先載入、閒置卸載、各模型 MTP 開關）。

**Tech Stack:** Rust 2024、tokio、reqwest（stream）、eframe/egui、wiremock（測試）；新增 `zip`、`sha2`、`ggus`。

**Spec:** `docs/superpowers/specs/2026-10-25-local-model-design.md`

## Global Constraints

- 回答、註解、docstring、UI 文字用繁體中文全形標點；log 訊息一律英文、前綴 `[local]`；**commit message 英文** conventional commits（`feat(local): …`）。
- 註解節制：預設不寫實作 `//`，只寫 WHY，一行為原則；模組 `//!`、公開項目 `///` 一行摘要。
- 不得 `println!`／`eprintln!`，一律 `crate::logs::log`。log 不得含 API 金鑰、HF CDN 轉址網址的查詢字串（`Policy`／`Signature`）。
- 錯誤訊息只顯示伺服器／系統原文，不加推測。
- 翻譯語言不寫死；本功能不碰提示詞。
- 新 UI 字串加進 `src/i18n/locales/zh-TW.json`、`zh-CN.json`、`en-US.json`（只有這三個語系有內容）；鍵名前綴 `local.`；測試經 key 取字（`t(...)`），不綁翻譯句；句尾標點跨語言一致。`tests/it/no_hardcoded_ui_text.rs` 會擋 UI 模組裡的中日韓字面量。
- 整合測試寫在 `tests/it/` 底下的模組並在 `tests/it/main.rs` 宣告，不另開 `tests/*.rs`。
- 測試不可干擾執行中的遊戲：不搶焦點、不開真視窗。
- cargo 一律限制並行：`cargo build -j 4`、`cargo test -j 4 -- --test-threads=4`、`cargo clippy -j 4 --all-targets -- -D warnings`。
- 每個 task 結束前只跑一次全套：`cargo fmt --check`、`cargo clippy -j 4 --all-targets -- -D warnings`、`cargo test -j 4 -- --test-threads=4`，全綠才 commit；不 `--no-verify`、不 push。
- 推薦清單（2026-10-25 查證，Apache-2.0、非 gated）：Qwen3.5-4B `unsloth/Qwen3.5-4B-GGUF`、Gemma 4 E4B `unsloth/gemma-4-E4B-it-GGUF`、Qwen3.5-9B `unsloth/Qwen3.5-9B-GGUF`、Gemma 4 12B QAT `google/gemma-4-12B-it-qat-q4_0-gguf`、Gemma 4 26B-A4B `unsloth/gemma-4-26B-A4B-it-GGUF`、Qwen3.8-27B `unsloth/Qwen3.8-27B-GGUF`；後兩個不讓精靈自動挑。
- 介面不顯示入門／主流／進階／高階；預估記憶體寫明種類：一般模型「顯示卡記憶體（VRAM）」、MoE「顯示卡＋系統記憶體（VRAM＋RAM 合計）」、內顯電腦「系統記憶體（RAM）」。
- 預先載入開關的說明文字定稿：「本工具開啟時就在背景預先載入本地服務用到的模型，第一句翻譯不用等待模型載入；關掉的話等第一句翻譯時才會載入模型，會需要等待一點時間。」
- 中斷的下載按鈕文字是「繼續下載」，不是「續傳」。

## 與 spec 的落差（實作時照這裡）

- spec「設定與資料」寫引擎版本、已下載模型清單存 `config.json`；本計畫改為**以磁碟為準**：`llama/active.json` 與 `models/<id>/manifest.json`。理由：這些是立即生效的磁碟操作，不在設定視窗的 Save／Cancel 草稿裡，寫進 `config.json` 會跟視窗的儲存互相覆蓋。`config.json` 只存 `local_models` 設定區塊。
- llama-server 啟動固定帶 `-c 8192 --parallel 1`：context 與 VRAM 用量可預估，多個用途共用一個子程序時請求排隊。

## Review Focus

0. **使用者自選的 llama.cpp 版本不認得本程式帶的參數**：低於 `MIN_BUILD` 的不列出；新版改了參數名稱導致啟動失敗時，自動退回上一版並顯示 llama.cpp 原文錯誤 —— Task 7／9／10 有測試。
1. **HF 轉址到 CDN 後才回 `Range` 結果**：下載中斷再按「繼續下載」，伺服器若回 200 而非 206，必須從頭寫、不能把整檔接在 `.part` 後面 —— Task 6 有測試。
2. **llama-server 在 `/health` 就緒前就結束**（防毒擋下、模型架構不支援、VRAM 不夠）：使用者要看到含 exit code 與最後幾行輸出的錯誤，不是無限「載入中」 —— Task 9 有測試。
3. **使用者在設定視窗刪掉正在跑的模型**：子程序要先停掉才刪檔（Windows 檔案鎖），服務標成「模型不存在」 —— Task 8／13 有測試。
4. **新版引擎啟動失敗**：自動退回上一版並保留可用狀態，不讓使用者卡在壞 build —— Task 7／10 有測試。
5. **GGUF 檔頭比初始抓取量大**（詞表數 MB）：要逐步加大 `Range` 讀到 tensor 資訊為止，不能因 `Eos` 直接判定壞檔 —— Task 3 有測試。

---

### Task 1: 相依、`local_models` 設定區塊、「本地」服務商

**Files:**
- Modify: `Cargo.toml`（`[dependencies]` 加 `zip = { version = "9.1", default-features = false, features = ["deflate"] }`、`sha2 = "0.11"`、`ggus = "0.5"`；`windows-sys` features 加 `"Win32_System_JobObjects"`、`"Win32_Storage_FileSystem"`）
- Create: `src/config/local.rs`
- Modify: `src/config/mod.rs`（`pub mod local;`、`default_config` 加 `"local_models"`、`load_config` 呼叫 `local::normalize`）
- Modify: `src/config/services.rs`（`PROVIDERS` 加 `local`、`Provider::needs_api_key`、`validate_endpoint_fields`）
- Modify: `.gitignore`（加 `/llama`、`/models`）
- Test: `src/config/local.rs` 內 `#[cfg(test)]`、`tests/it/config_golden.rs`（既有，補案例）

**Interfaces:**
- Produces:
  - `pub const LOCAL_PROVIDER: &str = "local";`（`services.rs`）
  - `impl Provider { pub fn needs_api_key(&self) -> bool }` = `has_field("api_key")`
  - `pub struct LocalSettings { pub preload: bool, pub idle_minutes: u32, pub mtp: BTreeMap<String, bool> }`，`idle_minutes == 0` 代表不卸載
  - `pub fn local::settings(cfg: &Config) -> LocalSettings`、`pub fn local::write_into(cfg: &mut Config, s: &LocalSettings)`、`pub fn local::normalize(cfg: &mut Config) -> bool`
  - `pub const IDLE_CHOICES: [u32; 4] = [0, 5, 15, 30];`、預設 `preload: true`、`idle_minutes: 15`

- [ ] **Step 1: 寫失敗測試**

```rust
#[test]
fn defaults_preload_and_fifteen_minutes() {
    let s = settings(&default_config());
    assert_eq!(s, LocalSettings { preload: true, idle_minutes: 15, mtp: BTreeMap::new() });
}
#[test]
fn normalize_snaps_unknown_idle_values_to_default() {
    let mut cfg = default_config();
    cfg.insert("local_models".into(), json!({"preload": "yes", "idle_minutes": 7, "mtp": {"a": true, "b": 1}}));
    assert!(normalize(&mut cfg));
    assert_eq!(settings(&cfg), LocalSettings { preload: true, idle_minutes: 15, mtp: [("a".into(), true)].into() });
}
#[test]
fn local_service_needs_only_a_model() {
    let mut s = new_service("local", &[]);
    assert_eq!(validate_service(&s), vec!["error.need_model".to_string()]);
    s["model"] = "unsloth__Qwen3.5-4B-GGUF__Q4_K_M".into();
    assert!(validate_service(&s).is_empty());
}
```

- [ ] **Step 2: 跑測試確認失敗** — `cargo test -j 4 --lib config::local -- --test-threads=4`，預期編譯失敗（`settings` 未定義）。
- [ ] **Step 3: 實作**
  - `local` 服務商：`key: "local"`、`label_key: "provider.local"`、`desc_key: "provider.local_desc"`、`key_url: None`、`brand: None`（`short_name` 會讀 `provider.local_short`）、`fields: &[("model", Field::Text("")), ("thinking", Field::Flag(false))]`、`efforts: &[]`；放在 `custom` 之前。
  - `validate_endpoint_fields`：`needs_api_key()` 為假時不要求金鑰。
  - `default_config` 的 `"local_models": {"preload": true, "idle_minutes": 15, "mtp": {}}`；`normalize` 把非 bool 的 preload、不在 `IDLE_CHOICES` 的 idle 改回預設、`mtp` 只留 bool 值，有改就回 `true` 並 log `[config] normalized local_models`。
  - 三個語系補 `provider.local`、`provider.local_desc`、`provider.local_short`。
- [ ] **Step 4: 跑測試確認通過**（同 Step 2 指令，加跑 `cargo test -j 4 --test it config -- --test-threads=4`）。
- [ ] **Step 5: 全套檢查後 commit** — `feat(local): add local provider and settings block`

---

### Task 2: Hugging Face repo 解析與檔案分類（`src/local/hf.rs`）

**Files:**
- Create: `src/local/mod.rs`（`//!` 模組說明、`pub mod hf;`，之後各 task 逐一加）、`src/local/hf.rs`
- Modify: `src/lib.rs`（`pub mod local;`）
- Test: `tests/it/local_hf.rs`（並在 `tests/it/main.rs` 宣告 `mod local_hf;`）

**Interfaces:**
- Produces:
  - `pub fn parse_repo(input: &str) -> Option<String>` —— 接受 `org/repo`、`https://huggingface.co/org/repo`、`huggingface.co/org/repo`，可帶 `/tree/main…`、結尾 `/`、前後空白；回 `org/repo`。其他（含 `org/repo/extra` 以外的非 HF 網域、少於兩段）回 `None`。
  - `pub struct RepoFile { pub path: String, pub size: u64, pub sha256: Option<String> }`
  - `pub async fn list_files(http: &reqwest::Client, base: &str, repo: &str) -> Result<Vec<RepoFile>, String>` —— `GET {base}/api/models/{repo}/tree/main?recursive=true`，只留 `type == "file"`；`sha256` 取 `lfs.oid`；非 2xx 回 `"HTTP {status}: {body 原文}"`。`base` 正式為 `HF_BASE = "https://huggingface.co"`。
  - `pub fn resolve_url(base: &str, repo: &str, path: &str) -> String` = `{base}/{repo}/resolve/main/{path}`
  - `pub struct Quant { pub label: String, pub bits: Option<u8> }`
  - `pub struct Choice { pub quant: Quant, pub files: Vec<RepoFile> }`（主模型；分檔依序）
  - `pub struct MtpChoice { pub quant: Quant, pub file: RepoFile, pub shared: bool }`
  - `pub struct Catalog { pub choices: Vec<Choice>, pub mtp: Vec<MtpChoice> }`
  - `pub fn classify(files: &[RepoFile]) -> Catalog` —— 只看 `.gguf`；檔名（不分大小寫）含 `mmproj` 排除；basename 以 `mtp-` 開頭歸 MTP（`shared` = 檔名含 `-shared-`）；`-NNNNN-of-NNNNN` 分檔合成一個 `Choice`；`choices` 依 `bits` 再依總大小排序。
  - `pub fn parse_quant(file_name: &str) -> Quant` —— 去掉 `.gguf` 與分檔尾巴後，以 `(?i)(?:^|[-_.])((?:UD-)?(?:I?Q\d(?:_[A-Z0-9]+)*|MXFP4(?:_MOE)?|BF16|F16|F32))$` 取量化名（`label` 保留原大小寫）；`bits`：`Q`/`IQ` 後第一個數字、`MXFP4*` = 4、`BF16`／`F16` = 16、`F32` = 32。對不到就 `label` = 原檔名、`bits: None`。

- [ ] **Step 1: 寫失敗測試**

```rust
#[test] fn parse_repo_accepts_urls_and_bare_ids() {
    for input in ["unsloth/Qwen3.5-4B-GGUF", " https://huggingface.co/unsloth/Qwen3.5-4B-GGUF/tree/main ",
                  "huggingface.co/unsloth/Qwen3.5-4B-GGUF/", "https://huggingface.co/unsloth/Qwen3.5-4B-GGUF"] {
        assert_eq!(parse_repo(input).as_deref(), Some("unsloth/Qwen3.5-4B-GGUF"), "{input}");
    }
    for bad in ["", "Qwen", "https://example.com/a/b", "a/b c"] { assert_eq!(parse_repo(bad), None, "{bad}"); }
}
#[test] fn parse_quant_covers_unsloth_and_google_names() {
    let q = |n| { let q = parse_quant(n); (q.label, q.bits) };
    assert_eq!(q("Qwen3.5-4B-UD-Q4_K_XL.gguf"), ("UD-Q4_K_XL".into(), Some(4)));
    assert_eq!(q("Qwen3.5-4B-IQ4_NL.gguf"), ("IQ4_NL".into(), Some(4)));
    assert_eq!(q("gemma-4-26B-A4B-it-MXFP4_MOE.gguf"), ("MXFP4_MOE".into(), Some(4)));
    assert_eq!(q("gemma-4-12b-it-qat-q4_0.gguf"), ("q4_0".into(), Some(4)));
    assert_eq!(q("Qwen3.5-4B-BF16.gguf"), ("BF16".into(), Some(16)));
    assert_eq!(q("big-UD-Q2_K_XL-00002-of-00003.gguf"), ("UD-Q2_K_XL".into(), Some(2)));
}
#[test] fn classify_drops_mmproj_and_splits_mtp_and_shards() {
    let f = |p: &str, s| RepoFile { path: p.into(), size: s, sha256: None };
    let cat = classify(&[f("mmproj-F16.gguf", 1), f("gemma-4-26B-it-mmproj.gguf", 1), f("README.md", 1),
        f("MTP/mtp-X-Q4_K_M.gguf", 2), f("MTP/mtp-X-shared-Q8_0.gguf", 3),
        f("Q2/X-Q2_K-00001-of-00002.gguf", 10), f("Q2/X-Q2_K-00002-of-00002.gguf", 5), f("X-Q4_K_M.gguf", 20)]);
    assert_eq!(cat.choices.iter().map(|c| (c.quant.label.as_str(), c.files.len())).collect::<Vec<_>>(),
               [("Q2_K", 2), ("Q4_K_M", 1)]);
    assert_eq!(cat.mtp.iter().map(|m| (m.quant.label.as_str(), m.shared)).collect::<Vec<_>>(),
               [("Q4_K_M", false), ("Q8_0", true)]);
}
#[tokio::test] async fn list_files_reads_the_recursive_tree() { /* wiremock: GET /api/models/o/r/tree/main?recursive=true
   回 [{"type":"directory","path":"MTP"},{"type":"file","path":"a.gguf","size":5,"lfs":{"oid":"ab","size":5}}]
   → assert_eq!(files, [RepoFile{path:"a.gguf".into(), size:5, sha256:Some("ab".into())}]);
   另一個 mock 回 404 "Repository Not Found" → Err 含 "HTTP 404" 與 "Repository Not Found" */ }
```

- [ ] **Step 2: 確認失敗** — `cargo test -j 4 --test it local_hf -- --test-threads=4`
- [ ] **Step 3: 實作上述函式**；HTTP client 用 Task 6 前先在本檔放 `pub fn hf_client() -> reqwest::Client`（`connect_timeout(60s)`、`read_timeout(60s)`、`redirect(Policy::limited(10))`、`User-Agent: wizard101-chat-translator/{CARGO_PKG_VERSION}`、經 `bypass_proxy_for_loopback`）——Task 6、7 共用它。
- [ ] **Step 4: 確認通過**（同 Step 2）。
- [ ] **Step 5: 全套檢查後 commit** — `feat(local): parse Hugging Face repos and classify GGUF files`

---

### Task 3: GGUF 檔頭（`src/local/gguf.rs`）

**Files:**
- Create: `src/local/gguf.rs`
- Test: `tests/it/local_gguf.rs`

**Interfaces:**
- Consumes: `hf::hf_client`
- Produces:
  - `#[derive(Clone, Debug, PartialEq, Default)] pub struct ModelMeta { pub arch: String, pub block_count: u32, pub head_count: u32, pub head_count_kv: u32, pub key_length: u32, pub value_length: u32, pub embedding_length: u32, pub expert_count: u32, pub has_mtp: bool }`
  - `pub fn parse(bytes: &[u8]) -> Result<ModelMeta, ParseError>`；`pub enum ParseError { NeedMore, Invalid(String) }` —— 用 `ggus::GGufReader`：`read_header` → 逐筆 `read_meta_kv` → 逐筆 `read_tensor_meta`；`GGufReadError::Eos` 對應 `NeedMore`。`{arch}.attention.head_count_kv` 缺時用 `head_count`；`key_length`／`value_length` 缺時用 `embedding_length / head_count`；`expert_count` 缺時 0；`has_mtp` = 有 tensor 名為 `blk.{block_count-1}.nextn.eh_proj.weight`。
  - `pub async fn fetch(http: &reqwest::Client, url: &str) -> Result<ModelMeta, String>` —— `Range: bytes=0-{n-1}`，`n` 從 4 MiB 起每次 `NeedMore` 加倍，上限 64 MiB（超過回錯誤）。

- [ ] **Step 1: 寫失敗測試** —— 測試內寫一個小工具 `fn gguf_bytes(kv: &[(&str, Val)], tensors: &[&str]) -> Vec<u8>`（依 GGUF v3 格式：magic `GGUF`、version 3、tensor_count、kv_count、KV、tensor info；`Val` 支援 u32、string、string 陣列）。

```rust
#[test] fn reads_attention_shape_and_mtp() {
    let bytes = gguf_bytes(&[("general.architecture", s("qwen35")), ("qwen35.block_count", u(4)),
        ("qwen35.attention.head_count", u(16)), ("qwen35.attention.head_count_kv", u(4)),
        ("qwen35.embedding_length", u(2048))], &["blk.3.nextn.eh_proj.weight"]);
    let m = parse(&bytes).unwrap();
    assert_eq!((m.block_count, m.head_count_kv, m.key_length, m.value_length, m.expert_count, m.has_mtp),
               (4, 4, 128, 128, 0, true));
}
#[test] fn truncated_header_asks_for_more() {
    let bytes = gguf_bytes(&[("general.architecture", s("x")), ("tokenizer.ggml.tokens", strs(10_000))], &[]);
    assert_eq!(parse(&bytes[..bytes.len() / 2]), Err(ParseError::NeedMore));
}
#[tokio::test] async fn fetch_grows_the_range_until_the_header_fits() {
    // wiremock：檔案 = 6 MiB 的檔頭（大詞表）+ 填充；mock 依 Range 回 206 與對應切片
    // assert：成功取得 meta，且收到的 Range 依序為 bytes=0-4194303、bytes=0-8388607
}
#[test] fn non_gguf_bytes_are_invalid() { assert!(matches!(parse(b"PK\x03\x04...."), Err(ParseError::Invalid(_)))); }
```

- [ ] **Step 2: 確認失敗** — `cargo test -j 4 --test it local_gguf -- --test-threads=4`
- [ ] **Step 3: 實作**（ggus 讀取 API：`GGufReader::new`、`read_header`、`read_meta_kv`、`read_tensor_meta`；KV 值的取法查 ggus 0.5 文件）。
- [ ] **Step 4: 確認通過**。
- [ ] **Step 5: 全套檢查後 commit** — `feat(local): read GGUF metadata over HTTP ranges`

---

### Task 4: 硬體偵測（`src/local/hardware.rs`）

**Files:**
- Create: `src/local/hardware.rs`
- Test: 檔內 `#[cfg(test)]`

**Interfaces:**
- Produces:
  - `#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum GpuKind { Discrete, Integrated, None }`
  - `#[derive(Clone, Debug, PartialEq)] pub struct Gpu { pub name: String, pub vendor: u32, pub kind: GpuKind }`
  - `#[derive(Clone, Debug, PartialEq, Default)] pub struct Hardware { pub gpu: Option<Gpu>, pub vram_total: u64, pub vram_free: u64, pub ram_total: u64, pub ram_free: u64 }`（位元組；`vram_*` 為 0 代表引擎還沒量）
  - `pub fn detect_gpu() -> Option<Gpu>` —— 用既有的 `wgpu` 列舉 adapter（`Backends::DX12`），取第一個 `DiscreteGpu`，沒有就 `IntegratedGpu`；`Cpu`／`VirtualGpu` 不算。
  - `pub fn system_memory() -> (u64, u64)` —— `GlobalMemoryStatusEx` 的 `(ullTotalPhys, ullAvailPhys)`。
  - `pub fn parse_list_devices(output: &str) -> Vec<(String, u64, u64)>` —— 解析 `llama-server --list-devices` 的 `  Vulkan0: NAME (8188 MiB, 7000 MiB free)` 行，回 `(名稱, total, free)` 位元組。
  - `pub fn effective(hw: &Hardware) -> (u64, u64)` —— 相容判斷用的 `(vram, ram)` 可用量：內顯回 `(0, ram_free)`（共享記憶體只算一次、當成 RAM）；其餘回 `(vram_free, ram_free)`。

- [ ] **Step 1: 寫失敗測試**

```rust
#[test] fn parses_list_devices_lines() {
    let out = "ggml_vulkan: Found 1 Vulkan devices:\nAvailable devices:\n  Vulkan0: NVIDIA GeForce RTX 4060 (8188 MiB, 7000 MiB free)\n";
    assert_eq!(parse_list_devices(out), vec![("NVIDIA GeForce RTX 4060".into(), 8188 << 20, 7000 << 20)]);
    assert!(parse_list_devices("Available devices:\n").is_empty());
}
#[test] fn integrated_gpu_counts_memory_once_as_ram() {
    let hw = Hardware { gpu: Some(Gpu { name: "Intel".into(), vendor: 0x8086, kind: GpuKind::Integrated }),
        vram_total: 16 << 30, vram_free: 10 << 30, ram_total: 16 << 30, ram_free: 10 << 30 };
    assert_eq!(effective(&hw), (0, 10 << 30));
}
```

- [ ] **Step 2: 確認失敗** — `cargo test -j 4 --lib local::hardware -- --test-threads=4`
- [ ] **Step 3: 實作。** `detect_gpu`／`system_memory` 不寫自動測試（依實機），在 Task 10 的偵測流程 log 結果。
- [ ] **Step 4: 確認通過**。
- [ ] **Step 5: 全套檢查後 commit** — `feat(local): detect GPU and memory`

---

### Task 5: 相容判斷、自動挑選、推薦清單（`src/local/fit.rs`、`src/local/catalog.rs`）

**Files:**
- Create: `src/local/fit.rs`、`src/local/catalog.rs`
- Test: 檔內 `#[cfg(test)]`

**Interfaces:**
- Consumes: `hf::{Catalog, Choice, MtpChoice}`、`gguf::ModelMeta`、`hardware::{Hardware, effective}`
- Produces（`fit.rs`）：
  - `pub const CONTEXT_TOKENS: u64 = 8192;`、`const OVERHEAD: u64 = 512 << 20;`、`const RAM_RESERVE: u64 = 2 << 30;`
  - `pub fn kv_bytes(meta: &ModelMeta) -> u64` = `block_count × CONTEXT_TOKENS × head_count_kv × (key_length + value_length) × 2`（f16；SWA 不扣，寧可高估）
  - `pub fn required(meta: &ModelMeta, main_bytes: u64, mtp_bytes: u64) -> u64` = `main + mtp + kv + OVERHEAD + main / 20`
  - `#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)] pub enum Fit { NoFit, Slow, Offload, Smooth }`（`Offload` = MoE 專家放 RAM；排序越大越好；`Unknown` 不放進 enum，引擎未量到 VRAM 時呼叫端以 `Option<Fit>` 為 `None` 表示）
  - `pub fn fit(required: u64, moe: bool, hw: &Hardware) -> Fit` —— 取 `(vram, ram) = effective(hw)`：`required <= vram` → `Smooth`（內顯 `vram` 為 0，故永不 `Smooth`）；`required <= vram + ram.saturating_sub(RAM_RESERVE)` → MoE 為 `Offload`、否則 `Slow`；否則 `NoFit`。
  - `pub enum MemoryKind { Vram, VramPlusRam, Ram }`、`pub fn memory_kind(moe: bool, hw: &Hardware) -> MemoryKind`（內顯 → `Ram`；MoE → `VramPlusRam`；其餘 `Vram`）
  - `pub struct Pick { pub choice: usize, pub mtp: Option<usize>, pub fit: Fit, pub required: u64 }`
  - `pub fn auto_pick(cat: &Catalog, meta: &ModelMeta, hw: &Hardware) -> Option<Pick>` —— 規則照 spec：`Smooth` 裡最大；沒有就 `Slow`/`Offload` 裡最小；全部 `NoFit` 挑最小。MTP：非 `shared` 裡量化 `bits` 最接近主模型的那個，加上後仍為 `Smooth` 才帶。
  - `pub fn combined_fit(requirements: &[(u64, bool)], hw: &Hardware) -> Fit` —— 多個模型同時載入：所需量加總，任一為 MoE 則以 MoE 判；給 Task 13 的儲存檢查用。
- Produces（`catalog.rs`）：
  - `pub struct Recommended { pub repo: &'static str, pub name: &'static str, pub wizard: bool }`
  - `pub const RECOMMENDED: &[Recommended]`，依 Global Constraints 的順序（陣列順序即「定位由低到高」）。
  - `pub fn wizard_pick<'a>(fits: &[(&'a Recommended, Fit)]) -> Option<&'a Recommended>` —— 只看 `wizard: true`，挑 `Fit` 最好、同等級取順序最後（定位最高）。

- [ ] **Step 1: 寫失敗測試**

```rust
fn hw(vram: u64, ram: u64) -> Hardware { Hardware { gpu: Some(discrete()), vram_total: vram, vram_free: vram, ram_total: ram, ram_free: ram } }
const G: u64 = 1 << 30;
#[test] fn dense_model_tiers() {
    assert_eq!(fit(5 * G, false, &hw(6 * G, 16 * G)), Fit::Smooth);
    assert_eq!(fit(7 * G, false, &hw(6 * G, 16 * G)), Fit::Slow);
    assert_eq!(fit(30 * G, false, &hw(6 * G, 16 * G)), Fit::NoFit);
}
#[test] fn moe_spilling_to_ram_is_offload() { assert_eq!(fit(18 * G, true, &hw(8 * G, 32 * G)), Fit::Offload); }
#[test] fn integrated_gpu_never_smooth() { assert_eq!(fit(G, false, &integrated(16 * G)), Fit::Slow); }
#[test] fn auto_pick_takes_largest_smooth_then_smallest_slow() { /* catalog Q2 3G / Q4 5G / Q8 9G；6G VRAM → Q4；4G VRAM → Q2 Slow；0.5G VRAM+1G RAM → Q2 NoFit */ }
#[test] fn mtp_only_when_still_smooth() { /* 主模型 Q4 5G、MTP Q4 1G：VRAM 7G → mtp Some；VRAM 6.2G → None */ }
#[test] fn combined_sums_requirements() { assert_eq!(combined_fit(&[(5 * G, false), (5 * G, false)], &hw(8 * G, 4 * G)), Fit::NoFit); }
#[test] fn wizard_skips_high_end_models() { /* 六個推薦都 Smooth → 回 Gemma 4 12B QAT */ }
```

- [ ] **Step 2: 確認失敗** — `cargo test -j 4 --lib local::fit local::catalog -- --test-threads=4`
- [ ] **Step 3: 實作**。
- [ ] **Step 4: 確認通過**。
- [ ] **Step 5: 全套檢查後 commit** — `feat(local): estimate memory needs and pick quantizations`

---

### Task 6: 可續傳下載（`src/local/download.rs`）

**Files:**
- Create: `src/local/download.rs`
- Test: `tests/it/local_download.rs`

**Interfaces:**
- Consumes: `hf::hf_client`
- Produces:
  - `#[derive(Default)] pub struct Progress { pub done: AtomicU64, pub total: AtomicU64 }`
  - `pub enum DownloadError { Cancelled, Http(String), Io(String), Checksum { expected: String, actual: String }, NoSpace { need: u64, free: u64 } }`（`Display` 給 UI 顯示原文）
  - `pub async fn download(http: &reqwest::Client, url: &str, dest: &Path, size: u64, sha256: Option<&str>, progress: &Progress, cancel: &CancellationToken) -> Result<(), DownloadError>` —— 寫到 `dest` 加 `.part`；`.part` 存在就帶 `Range: bytes={len}-`，回 206 接著寫、回 200 截斷從頭寫；完成後（有 `sha256` 時）算 sha256 比對，不符刪 `.part` 回 `Checksum`；通過才改名成 `dest`。取消時保留 `.part`。
  - `pub fn free_space(dir: &Path) -> io::Result<u64>`（`GetDiskFreeSpaceExW`）；`download` 開始前若 `free < size - 已下載 + 1 GiB` 回 `NoSpace`。
  - log：開始（url 去掉查詢字串、size、續傳位置）、完成（耗時、平均速度）、失敗原因。

- [ ] **Step 1: 寫失敗測試**（wiremock）

```rust
#[tokio::test] async fn resumes_with_range_and_verifies_sha256() { /* .part 已有前 4 bytes；mock 檢查 Range: bytes=4- 回 206 其餘；完成後 dest 內容完整、.part 消失 */ }
#[tokio::test] async fn a_200_reply_to_a_range_request_restarts_from_zero() { /* .part 有 4 bytes 垃圾；mock 回 200 全檔 → dest == 全檔 */ }
#[tokio::test] async fn checksum_mismatch_deletes_the_part_file() { /* sha256 給錯 → Err(Checksum{..})、.part 不存在、dest 不存在 */ }
#[tokio::test] async fn cancel_keeps_the_part_file() { /* mock 延遲回應中途 cancel → Err(Cancelled)、.part 存在 */ }
#[tokio::test] async fn follows_redirects_to_the_cdn() { /* /resolve 回 302 Location: /cdn/x；/cdn/x 回內容 → 成功 */ }
```

- [ ] **Step 2: 確認失敗** — `cargo test -j 4 --test it local_download -- --test-threads=4`
- [ ] **Step 3: 實作**（`sha2::Sha256`；寫檔用 `tokio::fs` 或 `std::fs` 於 `spawn_blocking`，二擇一；串流用 `bytes_stream()`）。
- [ ] **Step 4: 確認通過**。
- [ ] **Step 5: 全套檢查後 commit** — `feat(local): resumable verified downloads`

---

### Task 7: llama.cpp 引擎（`src/local/engine.rs`）

**Files:**
- Create: `src/local/engine.rs`
- Test: `tests/it/local_engine.rs`

**Interfaces:**
- Consumes: `download::{download, Progress, DownloadError}`、`hf::hf_client`、`hardware::Gpu`
- Produces:
  - `pub const RELEASES_API: &str = "https://api.github.com/repos/ggml-org/llama.cpp/releases?per_page=30";`
  - `#[derive(Clone, Debug, PartialEq, Eq, Hash)] pub enum Variant { Vulkan, Cpu, Cuda(String) }`（`Cuda` 帶版本如 `"13.4"`）；`pub fn auto_variant(gpu: Option<&Gpu>) -> Variant`（有 GPU → `Vulkan`，否則 `Cpu`）
  - `#[derive(Clone, Debug, PartialEq)] pub struct Asset { pub name: String, pub url: String, pub size: u64 }`
  - `#[derive(Clone, Debug, PartialEq)] pub struct Build { pub tag: String, pub published: String, pub variants: Vec<(Variant, Vec<Asset>)> }` —— `Cuda` 的 asset 清單含 `cudart-llama-bin-win-cuda-{ver}-x64.zip`。
  - `pub const MIN_BUILD: u32 = 11871;`（spec 的最低支援版本 `b11871`）
  - `pub fn parse_releases(json: &Value) -> Vec<Build>` —— 只收 tag 符合 `^b(\d+)$` 且數字 ≥ `MIN_BUILD`、且至少有一個 `^llama-b\d+-bin-win-(vulkan|cpu|cuda-[\d.]+)-x64\.zip$` 的 release；依 API 順序（新到舊）。`install` 遇到低於 `MIN_BUILD` 的 tag 直接回錯誤。
  - `pub async fn list_builds(http: &reqwest::Client, url: &str) -> Result<Vec<Build>, String>`（帶 `Accept: application/vnd.github+json` 與 User-Agent，同 `updater::fetch_latest_release`）
  - `#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)] pub struct EngineInstall { pub build: String, pub variant: Variant }`（`Variant` 序列化成 `"vulkan"`／`"cpu"`／`"cuda-13.4"`）
  - `pub struct ActiveFile { pub current: Option<EngineInstall>, pub previous: Option<EngineInstall> }`，存在 `<root>/llama/active.json`
  - `pub fn engine_dir(root: &Path, i: &EngineInstall) -> PathBuf` = `<root>/llama/{build}-{variant}`；`pub fn server_exe(dir: &Path) -> Option<PathBuf>`（遞迴找 `llama-server.exe`）
  - `pub async fn install(http, root: &Path, build: &Build, variant: &Variant, progress: &Progress, cancel) -> Result<EngineInstall, String>` —— 下載各 asset 到 `<root>/llama/.downloads/`，`zip` 解到 `engine_dir`（先解到 `…tmp` 再改名），刪 zip；**不改** `active.json`。
  - `pub fn activate(root: &Path, new: EngineInstall) -> io::Result<()>` —— `previous = current`、`current = new`。
  - `pub fn confirm(root: &Path) -> io::Result<()>` —— 新版首次 `/health` 就緒後呼叫：刪 `previous` 的資料夾並清掉 `previous`。
  - `pub fn rollback(root: &Path) -> io::Result<Option<EngineInstall>>` —— `current` 啟動失敗時：刪 `current` 資料夾、`current = previous`、`previous = None`，回退回後的版本。
  - `pub fn remove_all(root: &Path) -> io::Result<()>`（刪引擎）

- [ ] **Step 1: 寫失敗測試**

```rust
#[test] fn parse_releases_keeps_windows_builds_only() { /* fixture：b11871（含 vulkan/cpu/cuda-13.4/cudart 13.4）與 v0.7.0（只有 source）→ 只有 b11871，Cuda("13.4") 的 assets 兩個 */ }
#[test] fn builds_older_than_the_minimum_are_hidden() { /* fixture 再加 b11000（有 Windows 資產）→ 結果不含 b11000 */ }
#[test] fn rollback_restores_the_previous_build() { /* tempdir：activate(A)、confirm、activate(B)；rollback → current A、B 資料夾刪除 */ }
#[test] fn confirm_deletes_the_previous_build() { /* activate(A)、activate(B)、confirm → A 資料夾不存在、previous None */ }
#[tokio::test] async fn install_extracts_the_zip() { /* 測試內用 zip crate 造一個含 bin/llama-server.exe 的 zip，wiremock 提供 → server_exe(engine_dir) 存在 */ }
```

- [ ] **Step 2: 確認失敗** — `cargo test -j 4 --test it local_engine -- --test-threads=4`
- [ ] **Step 3: 實作**。
- [ ] **Step 4: 確認通過**。
- [ ] **Step 5: 全套檢查後 commit** — `feat(local): install, switch and roll back llama.cpp builds`

---

### Task 8: 已下載模型的存放（`src/local/store.rs`）

**Files:**
- Create: `src/local/store.rs`
- Test: 檔內 `#[cfg(test)]`（tempdir）

**Interfaces:**
- Consumes: `hf::{RepoFile, Quant}`
- Produces:
  - `pub fn model_id(repo: &str, quant_label: &str) -> String` —— `repo` 的 `/` 換 `__`，接 `__{quant}`，再把 Windows 檔名不允許的字元換成 `_`。例：`unsloth/Qwen3.5-4B-GGUF` + `Q4_K_M` → `unsloth__Qwen3.5-4B-GGUF__Q4_K_M`。
  - `#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)] pub struct Manifest { pub repo: String, pub quant: String, pub files: Vec<ManifestFile>, pub mtp: Option<ManifestFile>, pub moe: bool, pub has_builtin_mtp: bool, pub required: u64 }`；`pub struct ManifestFile { pub path: String, pub size: u64, pub sha256: Option<String> }`（`path` 為 repo 內路徑；本地檔名取 basename）
  - `pub struct Installed { pub id: String, pub manifest: Manifest, pub complete: bool }` —— `complete` = 所有檔都在且不是 `.part`。
  - `pub fn models_dir(root: &Path) -> PathBuf` = `<root>/models`；`pub fn write_manifest(root, id, &Manifest) -> io::Result<()>`；`pub fn list(root: &Path) -> Vec<Installed>`（壞 manifest 略過並 log）；`pub fn main_file(root, &Installed) -> PathBuf`（第一個分檔）；`pub fn mtp_file(root, &Installed) -> Option<PathBuf>`；`pub fn delete(root: &Path, id: &str) -> io::Result<()>`；`pub fn total_size(list: &[Installed]) -> u64`。

- [ ] **Step 1: 寫失敗測試**

```rust
#[test] fn model_id_is_a_safe_folder_name() { assert_eq!(model_id("unsloth/Qwen3.5-4B-GGUF", "Q4_K_M"), "unsloth__Qwen3.5-4B-GGUF__Q4_K_M"); assert_eq!(model_id("a/b", "x:y"), "a__b__x_y"); }
#[test] fn partial_files_mark_the_model_incomplete() { /* manifest 兩個檔，一個存在、一個只有 .part → complete false；補上後 true */ }
#[test] fn delete_removes_the_folder() { /* write_manifest 後 delete → list 為空 */ }
```

- [ ] **Step 2–4: 確認失敗 → 實作 → 確認通過** — `cargo test -j 4 --lib local::store -- --test-threads=4`
- [ ] **Step 5: 全套檢查後 commit** — `feat(local): on-disk model manifests`

---

### Task 9: llama-server 子程序（`src/local/server.rs`）

**Files:**
- Create: `src/local/server.rs`
- Test: `tests/it/local_server.rs`

**Interfaces:**
- Produces:
  - `#[derive(Clone, Debug, PartialEq)] pub enum Mtp { Off, Builtin, Draft(PathBuf) }`
  - `#[derive(Clone, Debug, PartialEq)] pub struct ServerPlan { pub model_id: String, pub exe: PathBuf, pub model: PathBuf, pub mtp: Mtp, pub idle_minutes: u32 }`
  - `pub fn args(plan: &ServerPlan, port: u16) -> Vec<String>` —— 依序：`-m <model> --host 127.0.0.1 --port <port> -c 8192 --parallel 1`；`idle_minutes > 0` 加 `--sleep-idle-seconds <分×60>`；`Builtin` 加 `--spec-type draft-mtp`；`Draft(p)` 加 `--spec-type draft-mtp --spec-draft-model <p>`。
  - `pub fn free_port() -> io::Result<u16>`（`TcpListener::bind("127.0.0.1:0")`）
  - `pub trait Process: Send { fn exited(&mut self) -> Option<i32>; fn kill(&mut self); fn tail(&self) -> Vec<String>; }`
  - `pub trait Launcher: Send + Sync { fn launch(&self, plan: &ServerPlan, port: u16) -> io::Result<Box<dyn Process>>; }`
  - `pub struct RealLauncher;` —— `Command` 加 `creation_flags(CREATE_NO_WINDOW)`、stdout／stderr piped，背景執行緒把每行寫進 log（`[local] llama-server[{model_id}]: {line}`）並保留最後 20 行給 `tail`；程序指派到全域 Job Object（`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`，`OnceLock` 存 handle 到程式結束）。
  - `pub async fn wait_ready(http: &reqwest::Client, base_url: &str, process: &mut dyn Process, timeout: Duration) -> Result<(), String>` —— 每 250 ms `GET /health`；200 → Ok；程序先結束 → `Err("llama-server exited with code {code}:\n{tail}")`；逾時 → kill 後 `Err` 含 tail。正式逾時 `READY_TIMEOUT = 180s`。

- [ ] **Step 1: 寫失敗測試**

```rust
#[test] fn args_with_draft_mtp_and_idle_sleep() {
    let plan = ServerPlan { model_id: "m".into(), exe: "x".into(), model: r"C:\m\a.gguf".into(), mtp: Mtp::Draft(r"C:\m\mtp.gguf".into()), idle_minutes: 15 };
    assert_eq!(args(&plan, 4321), ["-m", r"C:\m\a.gguf", "--host", "127.0.0.1", "--port", "4321", "-c", "8192", "--parallel", "1",
        "--sleep-idle-seconds", "900", "--spec-type", "draft-mtp", "--spec-draft-model", r"C:\m\mtp.gguf"]);
}
#[test] fn never_unload_omits_sleep() { /* idle_minutes 0、Mtp::Off → 不含 --sleep-idle-seconds、--spec-type */ }
#[tokio::test] async fn ready_when_health_returns_200() { /* wiremock /health 先 503 兩次再 200；假 Process 不結束 → Ok */ }
#[tokio::test] async fn early_exit_reports_code_and_tail() { /* 假 Process exited() 回 Some(1)、tail ["error: unknown model architecture"] → Err 含 "code 1" 與該行 */ }
```

- [ ] **Step 2–4: 確認失敗 → 實作 → 確認通過** — `cargo test -j 4 --test it local_server -- --test-threads=4`
- [ ] **Step 5: 全套檢查後 commit** — `feat(local): launch and supervise llama-server`

---

### Task 10: `Local` hub —— 狀態、指令、子程序管理（`src/local/mod.rs`、`src/local/hub.rs`、`src/local/runtime.rs`）

**Files:**
- Create: `src/local/hub.rs`（下載、引擎、repo 查詢、硬體、快照）、`src/local/runtime.rs`（子程序集合）
- Modify: `src/local/mod.rs`（`install`／`get` 全域入口）
- Test: `tests/it/local_runtime.rs`

**Interfaces:**
- Consumes: Task 2–9 全部
- Produces（`mod.rs`）：
  - `pub fn install(local: Arc<Local>)`、`pub fn get() -> Option<Arc<Local>>`（`OnceLock`）
  - `pub type Notify = Arc<dyn Fn(LocalNotice) + Send + Sync>`；`#[derive(Clone, Debug, PartialEq)] pub enum LocalNotice { Changed, Loading(Option<String>) }`（`Loading(Some(模型顯示名))` 開始載入、`None` 結束）
- Produces（`hub.rs`）：
  - `pub struct Local`；`pub fn Local::new(root: PathBuf, rt: Handle, launcher: Arc<dyn Launcher>, endpoints: Endpoints, notify: Notify) -> Arc<Self>`；`pub struct Endpoints { pub hf: String, pub releases: String }`（正式用 `HF_BASE`／`RELEASES_API`，測試指向 wiremock）
  - `pub fn snapshot(&self) -> Snapshot`
  - `#[derive(Clone, Debug, Default, PartialEq)] pub struct Snapshot { pub engine: EngineState, pub builds: Option<Result<Vec<Build>, String>>, pub hardware: Hardware, pub models: Vec<store::Installed>, pub downloads: BTreeMap<String, DownloadState>, pub repos: BTreeMap<String, RepoState> }`
  - `pub enum EngineState { Missing, Installing { build: String, done: u64, total: u64 }, Ready(EngineInstall), Failed(String) }`（`Default` = `Missing`）
  - `pub enum DownloadState { Running { done: u64, total: u64 }, Paused { done: u64, total: u64 }, Failed(String) }`
  - `pub enum RepoState { Loading, Ready(RepoInfo), Failed(String) }`；`pub struct RepoInfo { pub catalog: Catalog, pub meta: ModelMeta, pub fits: Vec<(u64, Option<Fit>)> /* 與 catalog.choices 對應：required 與等級；VRAM 未量到為 None */, pub mtp_required: Vec<u64>, pub auto: Option<Pick> }`
  - `pub enum Command { RefreshHardware, ListBuilds, InstallEngine { build: Option<String> /* None = 最新 */, variant: Option<Variant> /* None = 自動 */ }, RemoveEngine, QueryRepo(String), Download { repo: String, choice: usize, mtp: Option<usize> }, Resume(String), CancelDownload(String), Delete(String) }`
  - `pub fn command(&self, cmd: Command)` —— 全部丟到 `rt` 背景執行，狀態變了就 `notify(LocalNotice::Changed)`。
    - `Download`：引擎是 `Missing` 時先自動 `InstallEngine{None, None}`，完成並量到 VRAM 後再下載模型（spec「還沒下載引擎時點推薦模型」）。先寫 manifest（`complete: false`）再下載，讓中斷後 `list` 看得到、可「繼續下載」。
    - `Delete`：先 `runtime.stop(id)` 再刪資料夾。
    - `RefreshHardware`：`detect_gpu` + `system_memory`；引擎就緒時跑 `llama-server --list-devices`（`CREATE_NO_WINDOW`，逾時 15 s）取 VRAM；log 偵測結果。
  - `pub fn settings(&self) -> LocalSettings`：最近一次 `apply_config` 收到的設定（`LocalClient::warm_up` 讀 `preload`）。精靈與管理頁都用 `Command::Download`，沒有另一條下載路徑。
- Produces（`runtime.rs`，由 `Local` 持有並轉呼叫）：
  - `pub async fn Local::ensure(&self, model_id: &str) -> Result<String, String>` —— 回 `http://127.0.0.1:{port}`；該模型沒在跑就依 `LocalSettings` 組 `ServerPlan` 啟動（`notify(Loading(Some(name)))` → `wait_ready` → `notify(Loading(None))`）；同一模型的並發 `ensure` 只啟動一次（per-model `tokio::sync::Mutex`）。
  - 啟動失敗規則：(1) MTP 開著 → 關 MTP 重試一次並 log；(2) 這是新啟用、尚未 `confirm` 的引擎 → `engine::rollback` 後重試一次並 log；首次就緒時對新引擎呼叫 `engine::confirm`。仍失敗回 `Err(原文)`。
  - 子程序意外結束：下一次 `ensure` 重啟一次；連續兩次啟動失敗後停在失敗狀態（回同一錯誤），直到 `apply_config` 或使用者重新選模型。
  - `pub fn Local::apply_config(&self, cfg: &Config)` —— 算出所有 `ServiceSlot` 實際用到的本地模型集合：不在集合裡的子程序停掉；`preload` 為真時依序 `ensure` 集合裡還沒跑的（前一個就緒才啟動下一個）；`idle_minutes` 或 MTP 開關變了就重啟受影響的子程序。
  - `pub fn Local::stop(&self, model_id: &str)`、`pub fn Local::shutdown(&self)`（全部 kill）

- [ ] **Step 1: 寫失敗測試**（假 `Launcher` + wiremock 當 `/health`；`Endpoints` 指向 wiremock；tempdir 當 root 並預先寫好 engine `active.json` 與一個 complete 的模型）

```rust
#[tokio::test] async fn ensure_starts_once_for_concurrent_requests() { /* 兩個 ensure 並發 → launcher 只被呼叫一次、兩者回同一 base_url */ }
#[tokio::test] async fn mtp_failure_retries_without_mtp() { /* 假 Process 帶 MTP 的那次立即 exit(1)，第二次正常 → Ok，launcher 收到的第二個 plan.mtp == Mtp::Off */ }
#[tokio::test] async fn unconfirmed_engine_rolls_back_on_failure() { /* active.json current=B previous=A；B 的 exe 啟動即 exit → 改用 A 成功，active.json current=A */ }
#[tokio::test] async fn apply_config_stops_models_no_slot_uses() { /* 先 ensure("m1")；apply_config 只有雲端服務 → 假 Process.kill 被呼叫 */ }
#[tokio::test] async fn delete_stops_the_server_before_removing_files() { /* ensure 後 command(Delete) → kill 先於資料夾消失 */ }
#[tokio::test] async fn loading_notices_wrap_the_start() { /* notify 收到 Loading(Some(_)) 然後 Loading(None) */ }
```

- [ ] **Step 2–4: 確認失敗 → 實作 → 確認通過** — `cargo test -j 4 --test it local_runtime -- --test-threads=4`
- [ ] **Step 5: 全套檢查後 commit** — `feat(local): local model hub and server runtime`

---

### Task 11: 翻譯層接上本地服務與程式生命週期

**Files:**
- Create: `src/translation/clients/local.rs`
- Modify: `src/translation/clients/mod.rs`（`Client::Local(LocalClient)`、`build_client` 的 `"local"` 分支、`list_models`／`chat`／`model`／`warm_up` 分派）
- Modify: `src/main.rs` 或 `src/app_main.rs::build_app`（建立 `Local`、`local::install`、啟動時 `apply_config`）、`apply_settings`（存檔後 `apply_config`）、設定視窗按取消或關閉時也呼叫一次 `apply_config(目前設定)`（收掉測試連線臨時啟動的子程序）、`shutdown`（`local::get().map(|l| l.shutdown())`）
- Modify: `src/ui/events.rs`（`UiEvent::LocalChanged`、`UiEvent::LocalLoading(Option<String>)`）
- Test: `tests/it/clients_local.rs`

**Interfaces:**
- Consumes: `local::{get, Local::ensure}`、`OpenAiCompatClient`
- Produces:
  - `pub struct LocalClient { model_id: String, thinking: bool, inner: Mutex<Option<(String, Arc<OpenAiCompatClient>)>> }`；`LocalClient::new(model_id: &str, thinking: bool)`
  - `chat`：`local::get()` 為 `None` → `TranslateError::Config { status: None, detail: "local runtime unavailable" }`；`ensure` 失敗 → `TranslateError::Config { status: None, detail: 原文 }`；成功時 base_url 與快取的不同就重建 `OpenAiCompatClient::new(&url, "", &model_id, thinking, EFFORT_AUTO, false)`，再轉呼叫 `chat`。prompt cache log 沿用 `OpenAiCompatClient` 既有那行（llama-server 回 `prompt_tokens_details.cached_tokens` 就有數字）。
  - `warm_up`：讀 `LocalSettings.preload`（經 `local::get()` 的設定快取）為真才 `ensure`；否則什麼都不做。
  - `list_models`：回已下載且 complete 的模型 id（`ModelInfo::bare`）。
  - `Notify` 實作：`LocalNotice::Changed` → `ui.post(UiEvent::LocalChanged)`；`Loading(x)` → `ui.post(UiEvent::LocalLoading(x))`。

- [ ] **Step 1: 寫失敗測試**

```rust
#[tokio::test] async fn local_client_forwards_to_the_ensured_server() { /* 以測試 Launcher + wiremock 建 Local 並 install；wiremock 提供 /health 與 /v1/chat/completions 串流 → chat 回譯文 */ }
#[tokio::test] async fn ensure_failure_is_a_config_error_with_the_original_text() { /* 假 Process 立即 exit(1)、tail ["boom"] → Err(Config{detail 含 "boom"}) */ }
#[test] fn build_client_maps_local_provider() { /* spec.provider = "local" → Client::Local */ }
```

（`local::install` 是全域 `OnceLock`：這幾個測試共用一個測試用 `Local`，放在同一個 `#[tokio::test]` 序列或用 `test-support` 的 `local::install_for_test` 可覆寫版本 —— 實作 `install` 時在 `#[cfg(any(test, feature = "test-support"))]` 提供 `pub fn replace_for_test(Arc<Local>)`。）

- [ ] **Step 2–4: 確認失敗 → 實作 → 確認通過** — `cargo test -j 4 --test it clients_local -- --test-threads=4`
- [ ] **Step 5: 全套檢查後 commit** — `feat(local): route local services through llama-server`

---

### Task 12: Overlay「載入模型中」橫幅

**Files:**
- Modify: `src/ui/overlay/banners.rs`（`local_loading: Option<String>`、`pub fn set_local_loading(&mut self, model: Option<String>)`，畫成資訊列：沿用更新橫幅的 `BG_UPDATE`／`FG_UPDATE` 色、無 ✕、不可點）
- Modify: `src/ui/app.rs`（`UiEvent::LocalLoading` → `banners.set_local_loading`；`UiEvent::LocalChanged` → `ctx.request_repaint()`）
- Modify: 三個語系加 `local.banner.loading`（zh-TW：`正在載入本地模型（{model}）…`）
- Test: `src/ui/overlay/banners.rs` 內既有測試模組補案例

- [ ] **Step 1: 寫失敗測試** — `set_local_loading(Some("Qwen3.5-9B"))` 後 `info_text()` 回 `t("local.banner.loading", &[("model", "Qwen3.5-9B")])`；`None` 後回 `None`；錯誤橫幅文字不受影響。
- [ ] **Step 2–4** — `cargo test -j 4 --lib ui::overlay::banners -- --test-threads=4`
- [ ] **Step 5: 全套檢查後 commit** — `feat(local): show a loading banner while a model starts`

---

### Task 13: 設定 › 本地模型頁

**Files:**
- Create: `src/ui/settings/local_pane.rs`
- Modify: `src/ui/settings/window/mod.rs`（`Page::LocalModels` 插在 `Services` 之後、`title_key` `settings.tab.local`、`icon::LOCAL`；`SettingsWindow` 持有 `LocalPane`；`collect` 寫入 `local_models`）
- Modify: `src/ui/settings/theme.rs`（`icon::LOCAL`，從既有圖示字型挑一個）
- Modify: `src/ui/settings/env.rs`（`SettingsEnv` 加 `fn local_snapshot(&self) -> Snapshot;`、`fn local_command(&self, cmd: Command);`；`RealEnv` 轉給 `local::get()`，沒有時回預設快照、指令丟棄並 log）
- Modify: `tests/common/settings_env.rs`、`tests/it/ui_probe.rs`、`tests/settings_host.rs`（假 env 實作新方法：快照可注入、指令記錄下來）
- Modify: 三個語系（`settings.tab.local` 與 `local.*` 本頁字串）
- Test: `tests/it/local_settings.rs`（egui_kittest，比照 `tests/it/glossary_settings.rs`）

**Interfaces:**
- Consumes: `local::{Snapshot, Command, EngineState, DownloadState}`、`config::local::{LocalSettings, settings, write_into, IDLE_CHOICES}`、widgets（`card`、`setting_row`、`toggle`、`combo_box`、`primary_button`、`secondary_button`、`danger_secondary_button`、`callout`）、`dialog::confirm_delete`
- Produces:
  - `pub struct LocalPane { settings: LocalSettings, dialog: Option<DownloadDialog>, confirm_delete: Option<String> }`；`LocalPane::new(cfg: &Config)`、`pub fn ui(&mut self, ui: &mut Ui, env: &dyn SettingsEnv, services: &[Value])`、`pub fn settings(&self) -> &LocalSettings`
  - 版面由上而下（照 spec「目標」與 v6 草圖）：
    1. **llama.cpp 引擎** 卡：狀態徽章；版本下拉（`builds`，第一次畫時送 `Command::ListBuilds`）與變體下拉（自動／Vulkan／CUDA x.y／CPU）；有比目前新的 build 時顯示「有新版 {tag}」與〔更新〕，選了別的版本或變體時按鈕改為〔安裝所選版本〕；〔刪除引擎〕；下方顯示顯示卡名稱、VRAM／RAM 可用量與〔重新偵測〕，加一行「以目前可用量估算；遊戲沒開時量到的會偏高。」；`Installing` 顯示進度條。
    2. **模型載入** 卡：「啟動時預先載入模型」開關（說明文字用 Global Constraints 的定稿）；「閒置卸載」下拉（不卸載／5／15／30 分鐘）。這兩項與 MTP 開關寫進草稿，按儲存才生效。
    3. **已下載模型** 卡：標題右側總佔用空間與〔＋ 下載模型〕（開 Task 14 的對話框）；每列：名稱 · 量化、來源 repo · 大小、MTP 開關（模型有 MTP 檔或內建才顯示）、相容徽章、〔刪除〕；`Running` 顯示進度條＋〔取消〕；`Paused`／中斷顯示〔繼續下載〕＋〔刪除〕；`Failed` 顯示原文＋〔繼續下載〕；沒有模型時顯示空白提示與連結（開對話框到推薦分頁）。
  - 刪除：該模型被 `services` 裡任何本地服務引用 → `confirm_delete`（文案含服務名稱與釋放的大小）；否則直接送 `Command::Delete`。
  - `SettingsWindow::collect` 用 `config::local::write_into` 寫 `local_models`。

- [ ] **Step 1: 寫失敗測試**

```rust
#[test] fn page_order_puts_load_settings_above_installed_models() { /* 以注入的快照畫頁面；依 accesskit 節點順序：引擎標題 < 預先載入開關 < 已下載模型標題 */ }
#[test] fn preload_and_idle_go_into_the_saved_config() { /* 關掉預先載入、選 30 分鐘、按儲存 → Save(cfg) 的 local_models == {"preload": false, "idle_minutes": 30, ...} */ }
#[test] fn disk_actions_apply_immediately_even_if_cancelled() { /* 按刪除（未被引用）→ 假 env 收到 Command::Delete；之後按取消 → 不會再送任何指令 */ }
#[test] fn deleting_a_model_in_use_asks_first() { /* services 有本地服務引用 id → 先出確認框，確認後才收到 Command::Delete */ }
#[test] fn paused_download_offers_continue() { /* 快照 DownloadState::Paused → 找得到 t("local.continue_download") 按鈕；點了收到 Command::Resume(id) */ }
```

- [ ] **Step 2–4** — `cargo test -j 4 --test it local_settings -- --test-threads=4`
- [ ] **Step 5: 全套檢查後 commit** — `feat(local): local models settings page`

---

### Task 14: 「下載模型」對話框（推薦／自訂分頁、量化表）

**Files:**
- Create: `src/ui/settings/local_dialog.rs`（對話框與兩個分頁）、`src/ui/settings/quant_table.rs`（量化表元件）
- Modify: 三個語系（`local.dialog.*`、`local.fit.*`、`local.memory.*`）
- Test: `tests/it/local_settings.rs`（續）

**Interfaces:**
- Consumes: `local::{Snapshot, RepoState, RepoInfo, Command}`、`local::catalog::RECOMMENDED`、`local::fit::{Fit, Pick, MemoryKind, memory_kind}`、`dialog::dialog`
- Produces:
  - `pub enum DialogMode { Download, Select }`（`Select` 給精靈用：按鈕文字變〔選這個〕、不送下載指令而回傳選擇）
  - `pub struct Selection { pub repo: String, pub choice: usize, pub mtp: Option<usize> }`
  - `pub struct DownloadDialog { mode: DialogMode, tab: Tab, custom_input: String, expanded: Option<String>, picks: BTreeMap<String, (usize, Option<usize>)> }`；`DownloadDialog::new(mode: DialogMode, tab: Tab)`；`pub fn ui(&mut self, ui: &Ui, env: &dyn SettingsEnv, snapshot: &Snapshot) -> DialogOutcome`；`pub enum DialogOutcome { Open, Closed, Selected(Selection) }`；`pub enum Tab { Recommended, Custom }`
  - 開啟時對每個 `RECOMMENDED.repo` 送一次 `Command::QueryRepo`（快照已有就不送）。
  - **推薦分頁**每列：模型名稱、`約需 {size} {memory_kind}`（`local.memory.vram`／`local.memory.vram_ram`／`local.memory.ram`）、所選量化 · 檔案大小、相容徽章（`local.fit.smooth`／`slow`／`offload`／`nofit`／`unknown`）、〔選擇版本〕（只有一個量化時停用）、〔下載〕（已下載則停用並標 `local.downloaded`）。**不顯示**定位文字。預設所選量化為 `RepoInfo.auto`。
  - **量化表**（`quant_table::show(ui, info: &RepoInfo, pick: &mut (usize, Option<usize>))`）：依 `bits` 分列（`n-bit`，`None` 歸「其他」），每個量化一個色點 chip（綠／黃／紅；`Offload` 用黃），滑過顯示「需要約 X GB，可用 Y GB」；最後一列 MTP：〔不使用〕與各 MTP chip；選中的加框。
  - **自訂分頁**：輸入框＋〔查詢〕→ `parse_repo` 失敗顯示 `local.dialog.bad_repo`，成功送 `QueryRepo`；`Failed` 顯示原文；`Ready` 顯示 repo 名稱、量化表與〔下載所選〕。
  - 下載後（`Download` 模式）對話框關閉，進度出現在已下載模型卡片。
  - 大於 `NoFit` 以外都直接下載；`NoFit` 先出確認（`local.dialog.nofit_confirm`），確認才下載（spec：放不下也可以下載，但跳警告）。

- [ ] **Step 1: 寫失敗測試**

```rust
#[test] fn recommended_rows_show_memory_kind_not_tier() { /* 快照兩個 repo Ready（一般、MoE）→ 有 t("local.memory.vram", ..) 與 t("local.memory.vram_ram", ..)；畫面上沒有 "入門"／"Entry" 之類（改檢查 locales 裡沒有 local.tier.* 鍵） */ }
#[test] fn single_quant_repo_disables_choose_version() { /* catalog 只有一個 choice → 〔選擇版本〕disabled */ }
#[test] fn custom_tab_rejects_non_hf_input() { /* 輸入 "https://example.com/a/b" 按查詢 → 顯示 local.dialog.bad_repo、沒送指令 */ }
#[test] fn nofit_download_asks_first() { /* auto pick 為 NoFit → 按下載先出確認；確認後收到 Command::Download */ }
#[test] fn select_mode_returns_the_choice() { /* DialogMode::Select 點〔選這個〕→ DialogOutcome::Selected(Selection{..})、沒送 Download */ }
```

- [ ] **Step 2–4** — `cargo test -j 4 --test it local_settings -- --test-threads=4`
- [ ] **Step 5: 全套檢查後 commit** — `feat(local): download dialog with recommended and custom tabs`

---

### Task 15: 本地服務表單與儲存檢查

**Files:**
- Modify: `src/ui/settings/service_form.rs`（`provider == "local"` 時：沒有金鑰、端點、思考深度、取模型清單，保留思考開關；模型欄改成已下載且 complete 的模型下拉，顯示「名稱 · 量化（大小）」；下方提示「清單只列已下載的模型。」與連到本地模型頁的連結；測試連線照舊，旁註「第一次測試要先載入模型，可能要等十幾秒。」）
- Modify: `src/ui/settings/window/mod.rs::service_errors`（加本地檢查）
- Modify: 三個語系（`local.form.*`、`error.local_model_missing`、`error.local_over_capacity`）
- Test: `tests/it/service_form.rs`、`tests/it/settings_window.rs`（既有檔補案例）

**Interfaces:**
- Consumes: `SettingsEnv::local_snapshot`、`local::fit::combined_fit`、`store::Installed.manifest.{required, moe}`
- Produces:
  - `fn local_errors(cfg: &Config, snapshot: &Snapshot) -> Vec<SaveError>`（`window/mod.rs` 內）：(1) 任一用途的本地服務模型不在 `snapshot.models`（或未 complete）→ `error.local_model_missing`（帶服務名稱）；(2) 各用途用到的**不同**本地模型 ≥ 2 個且 `combined_fit(...) == Fit::NoFit` → `error.local_over_capacity`，args 列出各模型所需 GB 與可用量。`collect` 把它併進 `errors`；`SettingsWindow` 每幀取一次快照傳進去。
  - 刪除模型後，引用它的服務在服務清單卡片上顯示 `local.form.model_missing` 徽章。

- [ ] **Step 1: 寫失敗測試**

```rust
#[test] fn local_form_has_no_key_or_endpoint_fields() { /* 本地服務表單找不到 API 金鑰、端點輸入框 */ }
#[test] fn model_dropdown_lists_only_complete_downloads() { /* 快照：一個 complete、一個 Paused → 下拉只有前者 */ }
#[test] fn two_different_local_models_that_do_not_fit_block_saving() { /* 收訊與框選各指一個本地服務、模型不同，所需加總超過可用 → 儲存被擋、callout 有 error.local_over_capacity */ }
#[test] fn same_local_model_in_two_slots_is_fine() { /* 兩個用途同一模型 → 不擋 */ }
```

- [ ] **Step 2–4** — `cargo test -j 4 --test it service_form settings_window -- --test-threads=4`
- [ ] **Step 5: 全套檢查後 commit** — `feat(local): local service form and capacity check`

---

### Task 16: 首次設定精靈的一鍵安裝

**Files:**
- Create: `src/ui/settings/wizard/local_step.rs`
- Modify: `src/ui/settings/wizard/mod.rs`、`steps.rs`（選了 `local` 後顯示 `LocalInstall` 而不是一般表單；`can_advance` 對本地服務改為「所選模型已下載完成」）
- Modify: 三個語系（`local.wizard.*`）
- Test: `tests/it/wizard.rs`（既有檔補案例）

**Interfaces:**
- Consumes: `local::catalog::{RECOMMENDED, wizard_pick}`、`DownloadDialog`（`DialogMode::Select`）、`SettingsEnv::{local_snapshot, local_command}`
- Produces:
  - `pub struct LocalInstall { selection: Option<Selection>, dialog: Option<DownloadDialog>, started: bool }`；`pub fn ui(&mut self, ui: &mut Ui, env: &dyn SettingsEnv) -> InstallState`；`pub enum InstallState { Choosing, Downloading, Done(String /* model_id */), Failed(String) }`
  - 流程：
    1. 進入時送 `RefreshHardware`、各推薦 repo 的 `QueryRepo`；引擎未安裝時先送 `InstallEngine{None, None}`（量 VRAM 需要引擎；只下載引擎、尚未下載模型）。
    2. `wizard_pick` 挑出預設（只從 `wizard: true` 裡挑），確認卡片顯示：模型名稱 · 量化、相容徽章、模型大小、引擎大小（變體名）、合計、預估記憶體（含種類）。
    3. 〔換一個模型或版本〕開 `DownloadDialog::new(DialogMode::Select, Tab::Recommended)`（含自訂分頁）；選完回到確認卡片。
    4. 〔下載〕→ `Command::Download`；畫面換成引擎與模型各一條進度；〔取消下載〕送 `CancelDownload`（保留 `.part`，提示之後可在設定的本地模型頁「繼續下載」）；〔上一步〕可返回改選服務商。
    5. 下載完成 → 服務的 `model` 寫入該 id；〔下一步〕才可按。失敗顯示原文與〔重試〕（送 `Resume`）。
  - 引擎未就緒、量不到 VRAM 時，確認卡片的相容徽章顯示 `local.fit.unknown`，仍可下載。

- [ ] **Step 1: 寫失敗測試**

```rust
#[test] fn picking_local_shows_the_confirmation_card_with_sizes() { /* 快照：六個 repo Ready、硬體夠 → 預設是 Gemma 4 12B QAT（非 26B/27B）；畫面有模型大小、引擎大小、合計 */ }
#[test] fn next_is_disabled_until_the_download_completes() { /* 送出下載後快照 Running → 〔下一步〕disabled；改成 complete 模型 → enabled，Finish(cfg) 的服務 model == 該 id */ }
#[test] fn change_model_opens_the_shared_dialog_in_select_mode() { /* 點〔換一個模型或版本〕→ 出現 t("local.dialog.select") 按鈕 */ }
#[test] fn cancel_download_keeps_progress_for_later() { /* 點〔取消下載〕→ 收到 Command::CancelDownload(id)，畫面回確認卡片並顯示 local.wizard.cancelled_hint */ }
```

- [ ] **Step 2–4** — `cargo test -j 4 --test it wizard -- --test-threads=4`
- [ ] **Step 5: 全套檢查後 commit** — `feat(local): one-click local model install in the setup wizard`

---

### Task 17: 文件與實機驗證

**Files:**
- Modify: `README.md`、`README_ZH-TW.md`、`README_ZH-CN.md`（新增「本地模型」一節：不必另裝軟體、程式會下載 llama.cpp 與模型、相容等級的意思、檔案存放在程式資料夾的 `llama` 與 `models`；依記憶規則不寫設定鍵名、選項原文與快捷鍵）

- [ ] **Step 1: 寫 README 三語版本**（內容一致、各自依語言慣例標點）。
- [ ] **Step 2: 全套檢查**（fmt／clippy／test）。
- [ ] **Step 3: 實機驗證清單**（`cargo run`，請使用者在遊戲登入世界內時操作，回報 log）：
  - 本地模型頁：引擎自動選 Vulkan 並下載、版本清單可選舊 build、`--list-devices` 量到 VRAM。
  - 下載 Qwen3.5-4B；中途取消再「繼續下載」；sha256 驗證通過。
  - 建立本地服務 → 測試連線；收訊翻一句；確認 `app.log` 有 `[translate] prompt cache: provider=openai-compatible … cached=` 且第二句 `cached` > 0。
  - 閒置 5 分鐘後模型卸載、下一句出現「載入模型中」橫幅。
  - 下載有 MTP 的 Gemma 4 26B-A4B（若硬體允許）確認 `--spec-type draft-mtp` 啟動成功；內建 MTP 的行為記下結果。
  - 關閉本程式後工作管理員沒有殘留 `llama-server.exe`。
- [ ] **Step 4: commit** — `docs(readme): describe local models`
