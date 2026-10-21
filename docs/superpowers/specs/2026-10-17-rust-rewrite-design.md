# Wizard101 聊天翻譯助手：整套改用 Rust 重寫設計

日期：2026-10-17
狀態：設計完成，待審。

## 為什麼要做

發佈版是 PyInstaller onefile exe（121 MB）。每次啟動都要先把整包 Python runtime 與相依套件解壓到 `%TEMP%\_MEIxxxxx`，啟動要等很久，檔案也很肥。依 `2026-09-16-faster-exe-startup-design.md` 的量測，啟動約 2.8 秒，其中八成多發生在第一行 log 之前（解壓 1.35 秒、Python 初始化與 import 約 1 秒）。打包內容約 73% 是 OCR 相關（OpenCV、onnxruntime、模型），啟動時一行都沒用到，卻因為 onefile 而每次都被整包解壓。

改用 Rust 編成原生單一執行檔，不需要解壓，啟動時間與檔案大小一起解決，執行效能也會跟著變好。使用者也把這次重寫視為對未來的投資。

## 目標

1. **功能與操作和現行 Python 版（v0.6.0）完全一致**：現有每一項功能在 Rust 版都正常運作，操作方式相同。使用者換版後不應察覺差別，只會覺得開得快、檔案小。
2. **使用者資料無縫沿用**：`config.json` 原封不動直接讀（含舊格式的自動遷移）；`%LOCALAPPDATA%\wizard101-chat-translator\` 下的快取與 hookstate 檔沿用同一格式。
3. **發佈形式維持單一 exe**（見記憶「exe 必須單一檔案」，不改 onedir）。

## 非目標

- **外觀不必一致**：視窗外觀可以換成新介面框架的樣子；要一致的是功能、操作與行為。
- **不加新功能**：重寫期間 Python 版凍結、只修嚴重 bug；Rust 版只追平，不順便擴充。
- **PyInstaller 專屬機制不移植**：解壓期間的啟動畫面（含進度條）、`_MEI*` 暫存目錄監看與殺掉兄弟 bootloader 程序，在原生 exe 下都沒有意義。
- **log 不逐字相同**：格式、前綴、欄位維持一致；Python 特有的內容（例外類別名稱、traceback、`repr()` 引號風格）不追求逐字重現。

## 反目標（做到了也算失敗）

- 任何一項現有功能在 Rust 版異常、消失，或操作方式改變。
- 舊版使用者升級後要重新設定（設定遺失、服務商清單不見、快捷鍵失效）。
- 掛入／卸除 hook 讓遊戲當掉，或留下沒還原的程式碼位元組。

## 唯一刻意的行為改變：OCR 偵測正規化

實驗（2026-10-17，4 張遊戲截圖）發現現行版本的兩個錯字（`Items` 被讀成 `It ems`、`Welcome to the` 多出 `i`）出自 RapidOCR 的偵測輸入正規化：它用 mean/std 0.5，PaddleOCR 官方用 ImageNet 值（mean `[0.485, 0.456, 0.406]`、std `[0.229, 0.224, 0.225]`）。改用官方值後兩張都讀對，其餘兩張不變。

使用者決定 Rust 版**套用這個修正**，但正式採用前要**再用一批截圖驗證沒有副作用**。截圖由使用者提供，涵蓋商店標題、NPC 對話框、任務卡、中英混排等場景。驗證方式：同一批截圖分別跑 Python 版（現行參數）與 Rust 版（官方參數），逐張並排；若新參數在任一張讀得比舊參數差，回報使用者決定。

## 已決定的做法

### 分支與交付

- 使用者的決定：從 master 開 `refactor/rust-rewrite` 分支，**全部完成後開一個 PR**，以 merge commit 合回 master。
- 重寫期間 master 上的 Python 嚴重 bug 修正，要在分支的 Rust 版補上同等修正（定期把 master merge 進分支，衝突時以「把該行為移植到 Rust」解決）。
- 分支內 **Python 程式碼保留到最後一步才刪**：產生對照用的 golden 資料要靠它跑。
- 版本號：Rust 版發佈為 **v1.0.0**（我的決定）。

### 分段順序與驗收

照「能在遊戲裡實機驗收」的順序推進，每段由使用者實機驗收通過才往下。實作者（我）寫程式，使用者負責實機驗收。

| 段 | 範圍 | 使用者驗收重點 |
|---|---|---|
| 1 | 基礎（設定讀寫與遷移、log、i18n、單一實例）＋翻譯核心（四家服務商、提示詞、快取、翻譯池）＋掛入遊戲與收訊＋ overlay 全部功能（含泡泡、分頁、選取複製、橫幅） | 收訊翻譯、多開、切分頁、overlay 操作與舊版一致；開關程式多次遊戲不當 |
| 2 | 發話：快捷鍵、輸入框、Ctrl+V 攔截與逐字輸入 | 發話流程、輸入框位置與自動顯示／隱藏、快捷鍵吞鍵不誤觸輸入法 |
| 3 | 框選翻譯：WGC 擷取、選取畫面、框與結果卡、OCR | 框選流程一致；OCR 修正的截圖驗證 |
| 4 | 設定視窗、首次設定精靈、服務商管理、更新檢查、其餘 | 所有設定項可設可存、即時預覽、測試連線、模型清單 |
| 5 | 收尾：刪 Python、搬到最終目錄結構、CI、發佈流程、文件 | 用 CI 產出的 exe 跑一輪完整功能對照清單 |

第 1 段沒有設定視窗，驗收時直接沿用使用者現有的 `config.json`。

### 對照的方法

- **純邏輯一律用 golden 資料對照**：在 Python 還在分支時，寫一次性的匯出腳本，餵一批輸入給 Python 函式，把輸出存成 JSON fixture，Rust 測試讀同一份 fixture 斷言結果一致。至少涵蓋：聊天標記解析與分類（`markup.py`）、廣播收合（`chat_hook.py`）、`merge_lines`、譯文後處理（`postprocess.py`）、系統訊息快取的數字佔位、提示詞與 turns 組裝（**逐 byte 相同**）、範例驗證、`config` 載入／遷移／`normalize`（含各種舊格式與壞檔）、快捷鍵字串解析、SemVer 比較、`error_detail`、i18n 語言比對（系統語言清單 → 選中語言）、ui 幾何計算（`geometry.py`）。匯出腳本與 fixture 進版控，Python 刪除後 fixture 留下。
- **現有 87 個測試檔的行為都要有對應的 Rust 測試**；Tk 專屬的測試（視窗停放、pump）改成驗證新框架下的同等行為，或在功能對照清單中標成實機驗收項。
- **功能對照清單**（本文件後半）是使用者實機驗收的依據，每段驗收就勾該段的項目。

### 技術選擇（我的決定）

| 區塊 | 選擇 | 理由／備註 |
|---|---|---|
| Win32 | `windows` crate（微軟官方） | 掛入、視窗樣式、剪貼簿、鍵盤 hook 全走它 |
| 介面 | `eframe`／`egui`（多 viewport），視窗樣式用 raw HWND 補 | 一套框架管所有視窗；每像素透明可取代 Tk 的「色鍵＋半透明底窗」雙窗技巧；`Galley` 提供文字排版與座標→字元的 hit-test，選取複製有現成基礎；模型清單邊打邊篩在 egui 很直接。`WS_EX_NOACTIVATE`、owner、`WS_EX_APPWINDOW`、置頂等由取得 HWND 後直接設定 |
| 非同步 | `tokio`（網路）＋ `std::thread`（reader、鍵盤 hook） | 串流可隨時取消（drop／abort）；翻譯池改成 tokio 工作＋可動態調整上限的 semaphore |
| HTTP | `reqwest`（rustls）＋ SSE 解析 | Claude、Gemini 都沒有官方 Rust SDK，改直接打 REST 串流 |
| JSON | `serde_json`（開 `preserve_order`） | 設定遷移以 `Value` 操作、保留未知欄位與鍵順序；LRU 依插入順序 |
| OCR | `rusto-rs`（純 Rust 的 RapidOCR 移植）＋同三個模型 | 實驗：結果與 Python 逐字相同、exe 8.8 MB、不需 onnxruntime DLL；偵測 mean/std 改官方值 |
| 擷取 | `windows-capture` crate（Python 版綁的就是它） | 只用 WGC，沒有 PrintWindow 退路（與現行程式碼一致；舊 spec 寫 PrintWindow 已過時） |
| 影像 | `image` crate | PNG 編解碼 |
| exe 資源 | `winresource` 或 `embed-resource` | 版本資訊（單一 en-US StringTable）、圖示、manifest |

**繪圖後端**（我的決定）：eframe 用 wgpu 後端、Windows 上走 DX12；沒有可用的硬體 adapter 時（VM、遠端桌面、老舊內顯）改用 WARP 軟體 adapter（`force_fallback_adapter`），確保 Tk 時代能開的環境 Rust 版也能開。

**介面框架的退場條件**：若第 1 段做 overlay 時，egui 無法做到下列任一項 —— 不搶焦點、工作列按鈕行為、點擊不穿透到遊戲、文字外框與逐字選取、在遠端桌面／無 GPU 環境下以軟體繪圖正常顯示 —— 就把 overlay 改成 raw Win32 ＋ Direct2D／DirectWrite（`IDWriteTextLayout::HitTestPoint` 提供 hit-test），其餘視窗仍用 egui。發生時回報使用者。

**`rusto-rs` 的風險**：早期版本（0.3.1）、單一維護者。若停止維護或有無法修的 bug，把原始碼收進專案自行維護（純 Rust，可行）。

### 模組結構

沿用現有職責劃分，Rust 模組與現有 Python 套件同名：`reader`、`composer`、`input`、`translation`、`config`、`region`、`ui`、`i18n`、`logs`、`runtime`。單一 crate、單一 binary，不預先拆 workspace。

最終目錄是標準 Cargo 結構（`Cargo.toml` 與 `src/` 在 repo 根目錄）。過渡期 Python 佔用 `src/`，Rust crate 先放在子目錄，第 5 段刪 Python 後再搬到根目錄；過渡目錄名稱由實作者決定。

### 單一 exe 與資源

- i18n JSON、圖示、服務商圖示以 `include_bytes!`／`include_str!` 編進 exe。
- OCR 模型（約 31 MB）也編進 exe；若 `rusto-rs` 只吃檔案路徑，第一次框選時解到 `local_state_dir()` 下（以雜湊檢查是否需重寫），**不在啟動時做**。
- 不需要啟動畫面：原生 exe 啟動快，原本的啟動畫面是為了遮解壓時間。若實測從雙擊到 overlay 出現超過 0.5 秒，回報使用者再議。

### 其他已決定的細節

- **DPI**：程式啟動即宣告 per-monitor DPI aware（manifest）。現行版本啟動時是 DPI-unaware，第一次掛入時才被 wizwalker 改成 `PROCESS_PER_MONITOR_DPI_AWARE`，所以掛入前建立的 overlay、泡泡由系統點陣放大（邏輯座標），之後開的輸入框、框選、設定則是實體座標。Rust 版的規則（我的決定）：
  - 介面尺寸與字級以 DIP 計（egui 的 point 即 DIP），在縮放 125%／150% 的螢幕上，實際大小與舊版 overlay 相同，但改成清晰而非點陣放大。
  - `config.json` 的 `overlay`、`bubble_position` 一律以實體像素讀寫。舊值在高 DPI 螢幕上可能是邏輯座標，因此載入後若視窗超出可見螢幕範圍，就拉回螢幕內（只修正位置，不改大小）。
  - 遊戲記憶體、WGC 擷取、`ClientToScreen` 都是實體座標，與 per-monitor aware 一致，不需額外補償。
  - 驗收：使用者若只有 100% 縮放的螢幕，第 1、3 段驗收時暫時把 Windows 縮放切到 125% 跑一輪 overlay 與框選。
- **不讀 `Root.wad`**：wizwalker 建 `Client` 時會順手要求 `<安裝目錄>\Data\GameData\Root.wad` 存在，本專案用不到它。Rust 版不做這項檢查。
- **行程 handle 要關**：wizwalker 從不關 `OpenProcess` 的 handle，Rust 版在 reader 結束時關閉（使用者看不到差別）。
- **log**：發佈版（windows subsystem）寫 exe 旁的 `app.log`；開發版（`cargo run`）寫 stderr。`messages.log` 兩者都寫。格式、session header、UTC 毫秒時戳、CRLF、7 天／5 MB 修剪規則全部照舊。
- **重現 Python `repr()`**（我的決定）：`examples.py:82-85` 對範例集的 `repr()` 取 sha1，這個值同時是系統訊息快取 fingerprint 的 `x<12 碼>`。Rust 端逐字重現該資料結構的 `repr()`（字串引號與跳脫規則），以 golden 對照驗證，舊的系統訊息快取（最多 2000 筆付費翻譯）升級後才能繼續命中。
- **版本號單一來源**改成 `Cargo.toml` 的 `version`（程式內用 `env!("CARGO_PKG_VERSION")`）。

## 留給實作者決定的部分

- 各 crate 的具體版本（選撰寫當下最新的穩定版，鎖在 `Cargo.lock`）。
- 過渡期 Rust 目錄名稱、模組內部檔案切分。
- egui 的佈局、配色、字級（外觀可換；字型依語系選字的規則照舊）。
- 測試的組織方式。

## 功能對照清單

以下是盤點現行程式碼得出的行為，Rust 版全部要對上。行號指的是 v0.6.0 的 Python 程式碼，實作時以程式碼為準，舊 spec 若有出入（例如框選擷取的 PrintWindow）以程式碼為準。

### A. 啟動、關閉與執行期

- **單一實例**：named mutex `wizard101-chat-translator.single-instance`。第二次啟動時找標題等於在地化 `app_name()` 的可見視窗，`SW_RESTORE` ＋強制前景後退出（`runtime/single_instance.py`、`main.py:380-385`）。Python 版與 Rust 版用同一個 mutex 名稱。
- 啟動時記錄是否以系統管理員執行（只記 log）。
- 首次啟動（沒有可用服務）進設定精靈，完成後才進主畫面。
- **更新檢查**：啟動時查一次，之後每 60 分鐘查一次。`GET https://api.github.com/repos/GoneTone/wizard101-chat-translator/releases/latest`，逾時 10 秒，跟隨轉址，UA `wizard101-chat-translator/<版本>`；404 視為沒有發佈；SemVer（含預發布排序）比較；有新版就顯示 overlay 更新橫幅（`runtime/updater.py`、`main.py:101-124`）。
- **關閉順序**：設停止旗標 → 關翻譯池 → 解除鍵盤 hook → 等 reader 結束（8＋2 秒）→ 寫出快取 → 結束程序（`main.py:127-155`）。overlay ✕、泡泡關閉、Alt+F4、工作列關閉都走同一條路。

### B. 掛入遊戲與收訊（reader）

- **多開**：每 5 秒列舉 class 為 `Wizard Graphical Client` 的視窗；新的 hwnd 取最小可用 slot、開自己的 reader 執行緒；執行緒結束就釋放 slot；停止時所有 reader 共用 8 秒等待（`reader/supervisor.py`）。
- **reader 迴圈**（`reader/loop.py:150-280`）：`IsWindow(hwnd)` 為假就結束；每 0.1 秒讀環形緩衝；每 0.05 秒讀聊天輸入框是否開啟；每輪讀目前分頁，每 0.4 秒或分頁變動時讀完整分頁狀態；本 hwnd 在前景時每次完整取樣都通知 overlay 切到這個客戶端。
- **掛入失敗**：每 5 秒重試。狀態與橫幅分 `access_denied`、`version_mismatch`、`game_missing`；`PatternFailed`／多重比對／逾時 → 版本不相容，開不了程序 → 權限不足（`loop.py:184-199`）。橫幅彙總：永久性問題優先，任一客戶端已掛上就不顯示 `game_missing`（`status.py:61-82`）。
- **掛入順序**（`mem_reader.py:393-434`）：開程序 → 清掉已死 PID 的 hookstate 檔 → 依 hookstate 修復殘留 hook → 掛 root_window hook → 存 hookstate → 每 0.3 秒輪詢、最多 15 秒等就緒 → 掛 AddLine hook → 存 hookstate。
- **pattern 掃描**：列舉模組（名稱不分大小寫），走 `VirtualQueryEx`；只掃 `MEM_COMMIT` 且保護屬性恰為 `PAGE_EXECUTE_READ`、`PAGE_EXECUTE_READWRITE`、`PAGE_READWRITE`、`PAGE_READONLY` 的區段；整段讀出用 bytes regex（DOTALL）搜尋，比對不跨區段；0 筆 → PatternFailed，多於 1 筆 → 多重比對。pattern 字串原樣沿用（Rust `regex::bytes` 加 `(?s-u)`）。
- **autobot code cave**（wizwalker `memory/handler.py:31-121`）：掃 `AUTOBOT_PATTERN`，存原本 **208 bytes**（pattern 原始字串的長度），寫入 **4100 個 0**；hook stub 依序配在其中（root window 在 +0、50 bytes；AddLine 在 +50、256 bytes）。卸除時逐一寫回跳轉點、等 0.5 秒、再釋放 root window 的 export，最後只在內容不同時還原那 208 bytes。數字照抄；export 改在等待後才釋放是使用者決定的安全改動（wizwalker 在等待前釋放，仍在 stub 裡的遊戲執行緒可能寫進已釋放的頁面）。
- **root_window hook**：pattern `.......\x48\x8B\x01.......\xFF\x50\x70\x84`；比對處 7 bytes 改成 `E9 rel32 90 90`；stub `push rax; mov rax,[r13+D8]; mov [export],rax; pop rax; mov rcx,[r13+D8]; jmp back`；export 用 `VirtualAllocEx` 配 8 bytes（RWX），卸除時釋放。
- **AddLine hook**（`reader/chat_hook.py:18-99`）：stub 位元組原樣沿用；環形緩衝 0x80040 bytes、256 格 × 0x800，**刻意不釋放**；寫入只用 `WriteProcessMemory`，不呼叫 `VirtualProtectEx`。
- **hookstate 檔**：`%LOCALAPPDATA%\wizard101-chat-translator\hookstate-<pid>.json`，格式 `{"base": <int>, "ops": [{"addr": <int>, "bytes": "<小寫 hex>"}]}`（Python `json.dumps` 預設分隔、無結尾換行）；順序為 hook 跳轉（root、AddLine）在前、autobot 208 bytes 在最後；每掛一個 hook 寫一次，正常卸除就刪；重連時模組 base 相同才套用。**Rust 版要能讀 Python 版留下的檔案**。
- **事件過濾**（`mem_reader.py:125-233`）：只收 `this == WizardChatBox` 的事件；未知 `this` 每輪最多重新找一次 ChatBox，外來位址 5 秒後重驗；廣播收合規則（分頁 0,1,2,3 接著遞增 ≥5、文字相同、序號連續、至少 4 筆）與「尾端未完整的廣播保留一輪」；系統訊息關閉時不送出但序號照推進。
- **標記解析**（`reader/markup.py:71-112`）：表情變 `:name:`、去標籤、依序反轉義 `&lt;` `&gt;` `&amp;`、`\x00` 變空白、以 Python `str.split()` 語意壓空白；依圖示白名單、`[sender] ` 前綴、`<link;GID`、伺服器廣播（開頭 `<color;` 且無 `Art/`）分類。
- **分頁狀態**（`reader/chat_tabs.py`）：目前分頁在 ChatBox+0x7A0；標籤是 `*FilterButton+0x2E0` 的 wstring；按鈕與其父層 +0x9C 的 flags bit 1 都成立、且標籤不以 `<string;` 開頭才算可見。
- **UI 樹走訪**：children 在 +0x70（shared_ptr vector、步距 16、上限 1000）；name 是 +0x50 的 `std::string`（+0x10 的 int32 長度 ≥ 16 視為 heap 指標，UTF-8 解碼失敗回空字串）；parent +0x88、flags +0x9C、rect（4×i32）+0xA0；依名稱找視窗是完整 DFS、吞掉讀取錯誤。
- **輸入框錨點**（`mem_reader.py:344-361`、`ui_rect.py`）：`chatEditContainer` 的 rect 加上所有父層位移，乘上 `clientW/rootW`（兩軸都用寬度比例）；大小為 `x2-x1+1`；以 `ClientToScreen` 轉成螢幕座標。
- **訊息分派**（`loop.py:235-259`）：系統訊息命中快取就直接顯示；否則先顯示待翻譯占位，丟進系統訊息池、不帶上下文；玩家訊息帶「該客戶端＋該分頁」的上下文快照，先推入上下文再送出。
- **`messages.log`**：每行 `[slot=N] [seq=S tab=T verdict] raw`，略過符合 `\s*\[[A-Z]{3,5}\]` 的遊戲除錯行（`reader/message_log.py`）。

### C. 翻譯

- **服務商**（`translation/clients.py`）：
  - `custom`（OpenAI 相容）：串流 `POST {base_url}/v1/chat/completions`，有金鑰才帶 Bearer；`deterministic` 時才送 `temperature:0`；關閉思考時送 `reasoning_effort:"none"`、`chat_template_kwargs.enable_thinking:false`、`think:false`、`enable_thinking:false`。
  - `openai`：同一 client，base URL 固定 `https://api.openai.com`；用 `max_completion_tokens`、不送 `temperature`、加 `stream_options.include_usage`；關閉思考只送 `reasoning_effort:"none"`。
  - `claude`：Messages API 串流；不送 `temperature`；effort 非 auto 才送 `output_config.effort`；一律用結構化輸出（JSON schema `{translation: string}`）並解析回來；系統提示詞結尾帶 `cache_control: ephemeral`。**Python SDK 預設會對 429／5xx 自動重試 2 次（含退避），Rust 版要補上同等重試**。
  - `gemini`：`streamGenerateContent`；`system_instruction`、`max_output_tokens`；effort 非 auto 才送 `thinking_config.thinking_level`；角色 assistant → `model`、其餘 → `user`；不送 temperature；空金鑰送 `" "`；彙整文字時略過思考內容。SDK 若有內建重試，同樣補上（實作時查證）。
- **參數學習**（僅 OpenAI 相容）：400 回應點名某參數時（`clients.py:129-139` 的 regex），拿掉該參數或互換 `max_tokens`／`max_completion_tokens` 後重送；學到的結果留在該 client 實例。
- **串流取消**：另一執行緒可中途關閉回應，讓伺服器停止生成。
- **token 上限與逾時**：`MAX_TOKENS` 512、`MAX_TOKENS_THINKING` 4096、`MAX_TOKENS_REGION` 4096、逾時 60 秒；因長度上限停止（`length`／`max_tokens`／`MAX_TOKENS`）→ 視為輸出異常並記下樣本。
- **prompt cache log**：Claude 記 read／write／uncached；OpenAI、Gemini 記 cached tokens；沿用 `[translate] prompt cache:` 格式。
- **提示詞**（`translation/prompts.py`）：四種（收訊、發話（固定翻成英文）、系統訊息（可帶嚴格重試附註）、框選），共用規則區塊，目標是英文時規則不同；turn 順序：few-shot → 上下文 user turn（說明＋最多 8 行）→ assistant 確認 → 待翻譯文字；發話固定附 `FEWSHOT_OUTGOING`；系統訊息與框選不帶上下文；`PROMPT_REVISION` = 6。**輸出要與 Python 逐 byte 相同**（golden 對照）。
- **產生的 few-shot 範例**（`translation/examples.py`）：由設定的模型把 6 句示範改寫成目標語言；分收訊、系統、框選三條路徑驗證；最多 6 次嘗試並合併；背景產生。存在 `translation-examples.json`：`{"entries":{fingerprint:{incoming,system,region:[src,out]|null}}}`，最多 32 筆 LRU、原子寫入；fingerprint `provider|base_url|model|target|e4`；目標是英文時改用內建範例。
- **後處理**（`translation/postprocess.py`）：去 `<think>…</think>`；去掉外語或重複前方名稱的括號註解；還原原本的 `[Sender]`；偵測譯文退回英文 → 只有系統訊息會嚴格重試一次。框選逐行送、去 code fence 行；行數對得上就逐行（參考前後一行）整理，否則整塊顯示；空結果視為輸出異常。注意 Python `\w`、`casefold`、`splitlines` 的 Unicode 語意與 Rust 預設不同，靠 golden 對照抓差異。
- **系統訊息快取**（`translation/cache.py`）：`system-message-cache.json` ＝ `{"fingerprint":…, "entries":{template:translation}}`；查詢前把數字換成 `{0}`、`{1}`…，取出後依序還原；LRU 2000 筆，每 20 筆新資料與關閉時寫出，原子寫入；fingerprint `provider|base_url|model|target|p6|x<examples sha1 前 12 碼>`；佔位對不上、翻譯途中 fingerprint 變了、或譯文退回英文時不寫入，前兩者改直接翻原文。
- **翻譯池**：玩家訊息與系統訊息兩個池，共用一個可在執行期調整上限的併發閘門（設定 1–8，預設 4）。重試：離線類（連線錯誤、逾時、429、5xx）無限重試、全域退避 5／15／30 秒；設定類（4xx）每 15 秒重試；輸出異常或非預期錯誤 → 放棄該行並顯示翻譯失敗；任一次成功重置退避。發話與框選不重試。
- **錯誤訊息**：`error_detail()` 依序取 `error.message`、字串 `error`、頂層 `message`，否則去 HTML 標籤，最後壓成一行；overlay 橫幅三種（有訊息、只有狀態碼、都沒有），設定測試與結果卡用 `friendly_error`。
- **模型清單**：OpenAI 相容 `GET /v1/models` 取 `data[].id` 排序；Claude 自動翻頁；Gemini 只留支援 `generateContent` 的並去掉 `models/` 前綴；404／405 或沒有 `data` → 改為手動輸入。

### D. 設定（`config/`）

- 位置：`config.json` 在 exe 所在資料夾（開發時在 repo 根目錄）；快取在 `%LOCALAPPDATA%\wizard101-chat-translator\`。
- 欄位與預設、範圍：`config_version` 2、`services` []、`default_service` null、`service_slots` {incoming, outgoing, region}、`unsupported_services` []、`ui_language` null、`target_language`（預設依 zh-TW 的語言名稱）、`fade_seconds` 0（0–3600）、`max_messages` 200（10–1000）、`max_parallel_translations` 4（1–8）、`translate_system_messages` false、`hotkey` "ctrl+space"、`region_hotkey` "ctrl+shift+space"、`auto_show_input` true、`paste_hotkey` true、`type_delay` 0.02（0–0.5）、`overlay_alpha` 0.8（0.3–1.0）、`overlay` {x,y,width 640,height 420}、`bubble_position` {x,y}。
- 服務項目 `{id（8 位 hex）, name, provider, …}`，各家欄位：openai（model、api_key、thinking=false）、claude（model、api_key、effort="auto"）、gemini（model、api_key、effort="low"）、custom（base_url、model、api_key、thinking=false）。
- 載入：預設值深度合併在檔案值之下；進階數值夾在範圍內、非數字回預設；壞檔 → 用預設且**不覆寫**原檔；丟掉 `poll_interval`；有變動才以 UTF-8、縮排 2、保留非 ASCII 重寫。
- `normalize()`（`config/services.py:411-421`）：容器型別修正；`unsupported_services` 中已支援的服務商搬回 `services`；未知服務商原封不動搬進 `unsupported_services`；舊版 `api` 區塊（平面與分服務商兩種）轉成服務；欄位補齊與型別檢查（bool 不算 int）、去掉過時欄位、id 與名稱去重（「名稱 (2)」）；`default_service` 與 slots 只能指向存在的服務。
- 驗證：`model` 必填；非 custom 必填 `api_key`；custom 必填 `base_url`；`region_hotkey` 不可等於 `hotkey`。
- 首次啟動依系統語言決定介面語言（`config/__init__.py:143-157`）。
- overlay 位置大小與泡泡位置在拖曳／縮放放開時立即寫回。

### E. 快捷鍵與發話（`input/`、`composer/`）

- 預設 `ctrl+space` 開輸入框、`ctrl+shift+space` 開框選；兩者相同時框選停用；名稱無效時退回預設；**既有 config 裡的快捷鍵字串要解析成同樣的按鍵**（含 `keyboard` 套件接受的名稱寫法）。
- 低階鍵盤 hook（`WH_KEYBOARD_LL`）：組合鍵完全符合（沒有多餘修飾鍵）且處理器接受時才吞掉觸發鍵，吞到放開為止；吞掉後送 VK 0xE8 down/up，避免單獨的修飾鍵切換輸入法或開選單。多段或只有修飾鍵的組合退回不吞鍵的模式。被吞的鍵不算「按下」。
- 只有前景程序是 `WizardGraphicalClient.exe` 時快捷鍵才動作。
- **輸入框**：Enter 壓空白後在背景翻譯，空白 Enter 或 Esc 關閉；譯文超過 80 字顯示錯誤並保留原文；遊戲關閉聊天框時輸入框隱藏但保留草稿；關閉時把前景還給遊戲並取消進行中的請求。翻譯帶目標客戶端目前分頁的上下文。
- **送回遊戲**（`composer/paste.py:80-91`）：CR/LF 變空白並修剪；`AttachThreadInput` ＋ `SetForegroundWindow` 後等 0.15 秒；以 `type_delay` 逐字送 Unicode `SendInput`；前景一變就中止；不送 Enter。送字前先放開目前認為按著的鍵。
- **Ctrl+V 攔截**（`paste.py:94-167`，遊戲在前景且設定開啟時）：讀 `CF_UNICODETEXT`、CRLF 正規化成 LF；只有該客戶端聊天輸入框開著時才走單行模式（LF 變空白）；最多等 3 秒讓 Ctrl 與 V 放開再送字；`\n` 送 Enter；上一次還在送時忽略新的貼上；送字期間吞掉所有 V 鍵事件。
- 輸入框的自動顯示／隱藏只跟著**前景**客戶端（`input/dispatch.py:114-148`）。

### F. Overlay（`ui/overlay/`）

- 半透明底（透明度 = 設定值，色 `#101018`）、文字與按鈕不透明；不搶焦點；永遠在最上層；**點擊不穿透到遊戲**；有工作列按鈕且出現在 Alt+Tab；首次啟動置中，之後記住位置大小。overlay **不跟著遊戲視窗移動**，多開時只切換內容到前景客戶端。
- 標題列（20px）：圖示、名稱、依狀態上色的狀態字、按鈕 ⛶（框選）、⚙、─（縮成泡泡）、✕；名稱太長時被截的是名稱而不是按鈕；拖曳標題列移動；頂端邊緣帶改為縮放。
- 縮放：四邊與四角，邊緣帶 6px、角落帶 14px，最小 200×90；右下角有抓取紋。
- 分頁列：鏡像遊戲分頁文字（不翻譯），目前分頁亮字＋2px 底線，只顯示不可點，沒有分頁時隱藏。
- 訊息清單：每個（客戶端、分頁）一份；每則兩行 —— 原文 9pt、遊戲顏色調暗（×0.71），譯文 11pt、遊戲顏色；待翻譯灰色、失敗紅色；譯文就地填入、下方訊息往下推；最多訊息數與 N 秒後淡出移除；沒有訊息時把狀態大字置中顯示。
- 黏底：在底部（≥0.999）時新訊息自動捲到底；只有使用者捲動時重新判定；不黏底時以「頂端訊息＋位移」錨定閱讀位置，移除或插入訊息時畫面不跳；拖曳選取中暫停自動捲動。
- 改變寬度時重新換行不卡頓（現行做法：可視範圍附近同步重排、其餘分批回填；Rust 版只要達到同樣的不卡頓即可）。
- 文字外框：8 個方向 1px 的 `#0a0a10` 外框加上本色文字。
- 自繪細捲軸：8px、圓角、外框、最小 20px、hover 變色、不需捲動時隱藏；滑鼠滾輪在清單、捲軸、底窗上都有效。
- 選取複製：字元級、可跨訊息；拖出可視範圍自動捲動（每 40ms 2–24px）；選取開始時取得鍵盤焦點讓 Ctrl+C 生效（Caps Lock 開著也要有效）；複製格式為訊息內行以 `\n` 連接、訊息間空一行；有選取時右鍵出現只有「複製」一項的選單（不搶焦點、不超出螢幕、點 overlay 其他地方就關）。
- 橫幅：錯誤橫幅（網址可點，不會被清單擠掉）；更新橫幅（整列可點開發佈頁，✕ 關閉後本次執行不再顯示該版本）。
- 泡泡（48px）：顯示 app 圖示、圓形；透明度 max(0.30, alpha×0.8)；左下角未讀數（上限 99+）；點擊（移動 <5px）展開、拖曳移動並記住位置，預設在 overlay 右上角；有自己的工作列按鈕與相同關閉路徑；每 300ms 檢查，若泡泡**變成**前景且游標不在它上面就自動展開（從工作列點按鈕的情境）。
- 提示框：停留 500ms 後在游標 +12px 處出現、不超出螢幕、不搶焦點，每次顯示重新讀文字。
- 透明度與介面語言變更即時套用。

### G. 框選翻譯（`region/`、`ui/region/`）

- 快捷鍵或 ⛶ 觸發 → 凍結一張遊戲畫面 → 在遊戲所在螢幕開全螢幕選取畫面：凍結畫面 45% 亮度疊在全螢幕截圖上，拖曳時顯示框線、W×H、框內原亮度；提示在另一個不搶焦點的小窗；Esc、右鍵或單純點一下取消；選取畫面強制取得前景。
- 擷取：只用 WGC，在自己的執行緒抓一張（2 秒逾時）；以 DWM 外框座標為原點、用 `ClientToScreen` ＋ `GetClientRect` 切出客戶區；尺寸對不上只記 log（可能是 DPI）；整張單色 → 擷取錯誤；選取超出遊戲範圍 → 對應錯誤。
- 框：選取後留在畫面上（框線＋8 個把手），可拖曳移動或縮放（最小 20），放開時重新擷取（即時畫面）並重新翻譯、取消前一個請求；拖曳不到 5px 還原。不搶焦點。
- 結果卡：狀態有翻譯中／結果（原文 9pt 調暗、空一行、譯文）／錯誤；位置先放框下方，不夠就上方，再不夠放較寬的一側，側邊位置會「黏住」避免來回跳；可選取文字、拖曳選取時暫借焦點讓 Ctrl+C 生效；右鍵「複製」（有選取複製選取，否則全部）；點一下或 ✕ 關閉；使用 overlay 透明度。
- OCR：PP-OCRv6 det small、ch_ppocr_mobile_v2.0 cls、PP-OCRv6 rec small；參數照 RapidOCR 3.9.2 預設（偵測 mean/std 改官方值，見上）；`merge_lines` 規則照舊；沒有文字 → 空結果；引擎延遲到第一次框選才建立。結果以 session 編號標記，過期結果丟棄。

### H. 設定介面（`ui/settings/`）

- 設定視窗：分頁「基本／服務／進階／關於」，每頁可捲動；只有第一次顯示時置頂；新開的視窗要主動取得前景（Windows 預設不會啟用）。
  - 基本：介面語言（即時預覽、重建視窗時保留位置、目前分頁與未存的輸入）、譯者名單、Crowdin 連結、目標語言（可自行輸入的下拉）、快捷鍵、各勾選項。
  - 進階：數值欄位、透明度滑桿（即時預覽）。
  - 關於：檢查更新、連結、開啟 log 資料夾、清除快取。
  - 儲存時驗證並以警告對話框提示；取消時還原透明度與語言預覽。
- 首次設定精靈：三步（●○ 指示）、上一步／下一步；第二步要測試成功或選擇「略過測試」才能下一步；換語言時重建精靈。
- 服務管理：預設服務下拉、可收合的分工指派（收訊／發話／框選各用哪個服務）、服務卡片（服務商圖示、標記、編輯／刪除，最後一個不能刪）、新增時選服務商；對話框為 modal。
- 服務表單：依服務商顯示欄位、金鑰欄可切換顯示、測試連線（背景執行，過期結果丟棄）。
- 模型欄位：「重新整理」抓模型清單，輸入時即時篩選；抓不到清單時改手動輸入。
- 快捷鍵欄位：用低階 hook 錄製，輸入法會吃掉的組合也錄得到。
- 鍵盤操作：只能用滑鼠點的元件也要能 Tab 到並以 Enter／Space 觸發，且有可見的焦點框。
- 文字中的 `[文字](網址)` 與裸網址可點（只允許 http／https）。

### I. 介面語言（`i18n/`）

- `<crowdin 語系>.json` 平面 key/value，約 173 個 key；`language.name`、`language.translators` 為中繼資料；來源語言 zh-TW、預設 en-US。
- 查找順序：目前語言 → en-US → zh-TW → key 本身；格式化錯誤也往下退；載入時拒絕重複 key。
- 語言選單只列出宣告了名稱且覆蓋率 ≥60% 的語言，一律顯示該語言自己的名稱。
- 系統語言取自 `GetUserPreferredUILanguages`，以 langcodes 的 `closest_match` 規則比對（Rust 版靠 golden 對照表確保選中同一語言）。
- 字型依語系文字選：繁中 Microsoft JhengHei、簡中 Microsoft YaHei、日文 Yu Gothic UI、韓文 Malgun Gothic、其餘 Segoe UI。
- Crowdin 設定（`crowdin.yml`）與檔案路徑在第 5 段搬目錄時同步更新。

### J. 打包、CI、發佈與文件（第 5 段）

- exe 版本資訊：單一 en-US StringTable，欄位與現行 `build.spec` 相同；四位數檔案版本去掉預發布後綴。
- CI（`ci.yml`）：windows-latest，`cargo fmt --check`、`cargo clippy -- -D warnings`、`cargo test`。
- 發佈（`release-windows.yml`、`docs/releasing.md`）：流程不變（tag 驗證 → 改版本號 → lint／測試 → 提交 → 建置 → draft release ＋ footer ＋ SHA-256 → 手動發佈），工具換成 cargo。
- **防毒誤判**（使用者的決定）：不做程式碼簽章，沿用 README 與 release footer 現有的誤判提醒。原生 exe 的 import table 會直接露出 `OpenProcess`／`VirtualAllocEx`／`WriteProcessMemory`／`WH_KEYBOARD_LL`／`SendInput`，被標記的機率可能比 PyInstaller 版高，因此 `docs/releasing.md` 在「手動發佈」前加一步：用 Defender 掃描 draft release 的 exe，被誤判就到微軟的誤判提交頁送審，通過後再發佈。實際發佈後若誤判嚴重，再另議簽章。
- CLAUDE.md 與 AGENTS.md（兩份逐字同步）的開發規則改成 Rust 對應：uv → cargo、ruff → clippy／fmt、版本號來源、log 呼叫、wizwalker 相關說明改指向自寫的掛入模組。開發指令散在 9 處，要一起改（見記憶「開發指令散在 9 處」）。
- 三份 README 的開發段落；README 提到但程式碼不存在的「遊戲路徑」設定順便移除。
