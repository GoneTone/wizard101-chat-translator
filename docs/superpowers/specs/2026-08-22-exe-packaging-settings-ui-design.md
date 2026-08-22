# Wizard101 聊天翻譯助手：exe 打包與軟體內設定 UI 設計

日期：2026-08-22
狀態：已與使用者逐節確認

## 目標

1. 將本專案打包成**單一 exe**，讓一般使用者免裝 Python 環境即可使用。
2. 設定改為**軟體內完成**：首次啟動走多步驟設定精靈，之後可從 overlay 開設定視窗調整；不再要求使用者手動編輯 `config.json`。

## 決策摘要

| 決策點 | 結論 |
|---|---|
| 翻譯服務選擇 | ChatGPT（OpenAI）／Claude（Anthropic）／自訂 OpenAI 相容端點 |
| 首次設定 | 多步驟精靈（選服務 → 填 API → 測試連線 → 偏好設定） |
| 一般設定視窗 | 全部選項開放，分「基本／進階」分頁 |
| GUI 技術 | tkinter 原生（與現有 overlay／input_box 同技術棧，零新 GUI 依賴） |
| 發佈形式 | PyInstaller onefile、windowed（無主控台視窗） |
| 設定檔位置 | exe 旁邊（frozen 時以 `sys.executable` 所在目錄為準） |
| 命名 | 一律用全名 **Wizard101**：exe 為 `Wizard101ChatTranslator.exe`、UI 顯示名稱、README；`pyproject.toml` 專案名改為 `wizard101-chat-translator` |
| API 參數調整 | OpenAI 相容路徑 `temperature` 改為 `0`；兩種 client 請求 timeout 改為 **60 秒** |

## 1. 整體架構與模組切分

```
src/
├── main.py            # 調整:啟動時判斷「未完成設定 → 精靈」;掛設定視窗入口
├── config.py          # 調整:config 路徑改為應用程式目錄;api 區塊新增 provider 欄位
├── translator.py      # 調整:拆出 provider client;提示詞建構不動
└── ui/                # 新增:設定相關 UI(reader/composer 不動)
    ├── wizard.py      # 首次設定精靈(多步驟導航)
    ├── settings.py    # 一般設定視窗(分頁:基本/進階)
    └── fields.py      # 兩者共用的欄位元件(服務商選擇、金鑰輸入、測試連線等)
```

- `ui/wizard.py`：只負責首次流程導航（上一步／下一步／完成），表單欄位重用 `fields.py`；完成時寫入 config、回到主流程。
- `ui/settings.py`：載入現有 config → 表單編輯 → 儲存並套用；與 wizard 共用欄位元件，不重複寫表單。
- `fields.py`：欄位元件＋「測試連線」邏輯（實際打一次最小翻譯請求），wizard 與 settings 共用同一份。
- `overlay.py`：標題列加齒輪按鈕開設定視窗，其餘不動。
- `main.py`：config 不完整（API 未設定）→ 先跑精靈；否則直接進現有主流程。移除「請編輯 config.json」的 `sys.exit`。

設定套用方式（**不需重啟程式**）：儲存時重建 `Translator`、重新註冊熱鍵；`poll_interval`／`type_delay` 本來就每輪從 cfg dict 讀，即改即生效；`max_messages`／`fade_seconds` 改動時更新 overlay 屬性。例外：`game_path` 需重掛 hook，儲存後提示「下次啟動生效」。

## 2. 首次設定精靈

觸發條件：config 的 API 未設定完成（`model` 為空或 provider 必填欄位缺）。設定完成過的使用者不會再看到。

單一視窗置中、固定大小，底部「上一步／下一步」＋步驟指示（●●○○），共 4 步：

1. **歡迎＋選擇翻譯服務**：一句話說明工具用途；radio 卡片三選一——ChatGPT（OpenAI）／Claude（Anthropic）／自訂端點（進階，任何 OpenAI 相容伺服器如 LM Studio、Ollama、OpenRouter）。
2. **填 API 資訊**（依上一步動態變化）：
   - ChatGPT／Claude：API 金鑰（遮蔽顯示、可切換明文）＋模型下拉（內建常用清單、預設選好：ChatGPT 預設現行常用款（實作時以 OpenAI 官方文件查證模型 ID，不憑記憶寫死）、Claude 預設 `claude-opus-5`，附提示「較快較省：claude-haiku-4-5」；可手動輸入其他模型 ID）。附「取得金鑰」連結開官方金鑰頁。
   - 自訂端點：base_url、模型、API 金鑰（選填）、thinking 開關。
3. **測試連線**：按鈕實際打一次最小翻譯請求。成功 → 綠色「✓ 連線成功」＋範例譯文；失敗 → 人話錯誤（401 →「金鑰無效或過期」、404／模型錯誤 →「找不到模型」、連不上 →「無法連線到伺服器，請檢查網址」）。測試中按鈕停用。**測試成功才能下一步**；另提供「略過測試」小字連結。
4. **基本偏好＋完成**：翻譯目標語言（下拉常用語言＋可自行輸入）；熱鍵（顯示目前值，點「更改」進入按鍵捕捉模式）。按「完成」→ 寫入 config → 進主流程。

取消行為：中途關閉視窗＝退出程式，不留半套設定；已填欄位不保留。

## 3. 一般設定視窗

入口：overlay 齒輪按鈕。`ttk.Notebook` 兩分頁；視窗單例（重複開啟只帶到前景）。

- **基本**：翻譯服務 radio（切換時欄位跟著變，重用 `fields.py`）、API 欄位、「測試連線」、翻譯目標語言、熱鍵。
- **進階**（每項附一行小字說明，文案沿用 `config.py` 現有註解）：輪詢間隔、訊息淡出秒數、訊息保留上限、鍵入延遲、遊戲路徑（留空＝自動偵測，附「瀏覽…」按鈕）。

數值欄位用 `ttk.Spinbox` 限定範圍（如 poll_interval 0.1–5.0）。`api.thinking` 只在「自訂端點」顯示。

底部「儲存／取消」。儲存流程：驗證必填（不過 → 標紅提示、不關窗）→ 寫入 config.json → 立即套用（同第 1 節）。overlay 位置尺寸、輸入框位置維持拖曳即存，不進設定視窗。

## 4. translator 的 provider 架構

config 的 `api` 區塊：

```json
"api": {
  "provider": "openai" | "claude" | "custom",
  "api_key": "...",
  "model": "...",
  "base_url": "...",   // 僅 custom 使用；openai/claude 用官方端點
  "thinking": false    // 僅 custom 使用
}
```

舊 config 相容：載入時無 `provider` 欄位 → 視為 `custom`，原設定原封不動繼續用。

translator.py 內部拆兩個 client，對外介面（`translate_incoming`／`translate_outgoing`）與提示詞建構完全不動，client 介面為 `(system, text) -> 譯文`：

- `_OpenAICompatClient`：現有 httpx 打 `/v1/chat/completions`；`temperature=0`。ChatGPT＝此 client＋固定官方 base_url；custom＝此 client＋使用者 base_url。
- `_ClaudeClient`：官方 `anthropic` SDK（新增依賴），`client.messages.create(model, max_tokens, system, messages=[{user}])`。Claude 5 系不帶 `temperature`（會 400）；thinking 用預設（adaptive），不另設參數。回應同樣過 `strip_think`。

兩種 client 請求 timeout 均為 60 秒。

錯誤分類統一（main.py 重試邏輯不認識兩套例外）：

- `TranslatorOffline`（可重試）：連線失敗、逾時、429、5xx。main.py 原 `except httpx.HTTPError` 改抓此例外，退避重試行為不變。
- `TranslatorConfigError`（不可重試）：401／403（金鑰無效）、404（模型不存在）。overlay 顯示「⚠ API 設定有誤，請開設定檢查」，reader 暫停翻譯（訊息留在 pending），設定視窗儲存成功後自動恢復。

測試連線：以表單當下值建暫時 `Translator`，背景執行緒翻一句固定測試文字；與正式翻譯同一條路徑。

依賴變動：`pyproject.toml` 加 `anthropic`。

## 5. 打包、設定檔路徑、log 與建置流程

- **應用程式目錄**：`config.py` 新增判斷——frozen（`sys.frozen`）時為 `sys.executable` 所在目錄，開發時為專案根目錄；`config.json` 與 log 都放這裡。
- **無主控台＋log 檔**：windowed 建置；frozen 模式下 stdout／stderr 導向 exe 旁 `app.log`（每次啟動覆寫），現有 `print(..., file=sys.stderr)` 診斷自然落入 log。開發模式行為不變。
- **PyInstaller**：dev 依賴加 `pyinstaller`；`build.spec` 進版控（onefile、windowed、名稱 `Wizard101ChatTranslator.exe`）；建置指令 `uv run pyinstaller build.spec` 寫進 README。不做 CI 自動建置（YAGNI）。不加管理員權限 manifest；README 註明遊戲若以管理員執行，工具也需以管理員執行。
- **打包冒煙測試（實作前 spike）**：先打包**現況程式**實跑，驗證 wizwalker（git fork）、tkinter、keyboard、pymem 在 frozen 環境可用；需補 hidden imports／datas 則記錄於 build.spec。此為全案最大技術風險，先確認可行再投入 UI 實作。
- **README**：加一般使用者章節（下載 exe → 首次設定精靈 → 開始用）與防毒誤判說明。

## 測試策略

- pytest（純邏輯）：config 遷移（無 provider → custom）、provider 對應 client 選擇、錯誤映射（401 → `TranslatorConfigError`、timeout → `TranslatorOffline`，mock client）、欄位驗證函式。
- 實機驗證：精靈／設定視窗互動、打包產物（開遊戲實跑）。

## 已知風險與限制

1. **wizwalker 打包相容性**：git fork 依賴，可能需要 hidden imports／資料檔——以 spike 前置驗證。
2. **防毒誤判**：onefile＋全域鍵盤 hook＋讀遊戲記憶體，被 Defender／防毒標記機率不低；無憑證簽章下靠 README 說明與回報誤判。
3. **金鑰明文儲存**：config.json 存 exe 旁（與現況相同），UI 中註明。

## 範圍外

- CI 自動建置、安裝程式、自動更新
- 系統匣常駐
- overlay／輸入框本體的視覺改版
