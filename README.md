# Wizard101 對話翻譯助手

Wizard101 聊天 AI 翻譯：讀取遊戲聊天訊息即時翻成**你設定的目標語言**疊加顯示（保留原文）；
熱鍵輸入（任何語言）自動翻成**英文**，逐字**自動鍵入**遊戲聊天欄（遊戲不支援貼上；**不會自動送出**，自己確認後按 Enter）。
來源語言一律自動判斷。

收訊透過 [wizwalker](https://codeberg.org/LaurenzNotHere/wizwalker) 掛入遊戲、
直接讀聊天顯示控件（`chatLog`）的全文：100% 準確（無 OCR 辨識誤差）、
訊息有序、含他人與自己的發言及發送者身分，不攔封包。

## ⚠ 重要：封號風險

本工具靠 wizwalker **掛入（注入）遊戲程序** —— 為了讀聊天控件，它會
`WriteProcessMemory` 寫入 code cave 並建立執行緒下 hook（**非純讀**）。
這**違反 Wizard101 服務條款**，比單純讀取更具侵入性、**封號風險更高**,
**可能導致帳號被停權，請自行斟酌**。

## 一般使用者

不需要 Python 環境，下載打包好的 exe 即可使用：

1. 到 [Releases](https://github.com/GoneTone/wizard101-chat-translator/releases/latest)
   下載 `Wizard101ChatTranslator.exe`，放到任意資料夾（設定 `config.json`
   與紀錄檔 `app.log` 都會存在 exe 同一個資料夾）
2. 雙擊執行：
   - 第一次執行（找不到 `config.json`）會跳出**首次設定精靈**：第一步選翻譯服務、
     填金鑰並測試連線，第二步選目標語言與熱鍵，存好設定後自動進入主流程
   - 之後每次啟動都直接進主流程，不會再跳精靈
3. 開啟 Wizard101 並**登入進遊戲世界內**，疊加視窗才讀得到聊天訊息
4. 想改設定（服務商、金鑰、目標語言、熱鍵…）時，點疊加視窗標題列的齒輪
   （⚙）開啟設定視窗，存檔即套用，不必重開程式
5. 要結束程式時，點疊加視窗標題列右上角的 ✕（會先解除遊戲掛入再退出）

**防毒軟體誤判**：本工具會讀取遊戲聊天記憶體（掛入遊戲程序）並監聽全域熱鍵，
行為模式與部分惡意程式相似，可能被防毒軟體標記或直接刪除/隔離，這是誤判，
請自行評估風險後把程式加入防毒白名單。

**系統管理員權限**：若 Wizard101 以**系統管理員**身分執行，本工具也必須以
系統管理員身分執行（掛入遊戲程序需要對等或更高權限），否則會一直卡在
「連線遊戲中…」。

## 開發者

以下為原始碼開發、除錯、打包用的流程。

### 安裝

以 [uv](https://docs.astral.sh/uv/) 管理隔離環境（Windows、Python 3.11+）：

1. `uv sync` —— 建立 `.venv` 並依 `pyproject.toml` 裝好所有相依
   （含收訊用的 wizwalker；已於 `[tool.uv.sources]` 指向有跟進最新 client
   pattern 的 [LaurenzNotHere fork](https://codeberg.org/LaurenzNotHere/wizwalker)，
   官方 PyPI 版 pattern 過舊、對不上現行 client）
2. 複製 `config.example.json` 為 `config.json`，填入自架 AI 伺服器的
   `api.base_url` 與 `api.model`(OpenAI 相容 `/v1/chat/completions`)

> 遊戲改版導致掛入報 `PatternFailed` 時，更新 fork 後重跑 `uv sync`（或
> `uv lock --upgrade-package wizwalker`）取得新 pattern。

### 打包（exe）

```
uv run pyinstaller build.spec --noconfirm
```

產物在 `dist/Wizard101ChatTranslator.exe`，單一 windowed exe（無主控台黑窗）。
分發時只需這個 exe;`config.json` 與 `app.log` 會在使用者第一次執行時自動建立
在 exe 同一個資料夾（見上方「一般使用者」）。

### 放版

版本號遵循 [SemVer](https://semver.org/lang/zh-TW/)，唯一真實來源是
`src/__init__.py` 的 `__version__`（`pyproject.toml` 的 `version` 只是中繼資料，
由 `tests/test_version.py` 釘住兩者一致）。0.x 期間 minor 版可含破壞性變更
（例如 `config.json` 欄位改名），patch 版只修 bug。

放一版的步驟：

1. 同步改 `src/__init__.py` 與 `pyproject.toml` 的版本號
2. `uv run pytest` 全綠
3. `git commit -m "chore: release v0.2.0"`
4. `git tag v0.2.0`（tag 一律 `v` 前綴）並推上 GitHub
5. `uv run pyinstaller build.spec --noconfirm` 打包，在 GitHub 上以該 tag
   建立 Release，把 `dist/Wizard101ChatTranslator.exe` 當作 asset 上傳

### 使用（從原始碼執行）

1. 開啟並登入 Wizard101 到遊戲世界內
2. `uv run run.py`（等同 `uv run python -m src.main`）
   - 若狀態一直停在「連線遊戲中…」或顯示「遊戲未就緒／連線中斷」，以**系統管理員**身分開終端再執行（wizwalker 掛入常需要）
   - 若掛入時報 `PatternFailed`：遊戲已改版、pattern 過舊，更新 fork 後重跑 `uv sync`
3. 聊天出現訊息 → 疊加視窗顯示「原文 + 目標語言譯文」（最新在最下，可向上滾動看歷史；預設不自動清除）
   - 視窗固定大小、可互動：**拖曳頂端標題列移動、拖右下角把手縮放**，位置與大小自動存檔
   - 注意：視窗不滑鼠穿透，它蓋住的那塊區域點擊不會傳到遊戲
4. 想發言：先在遊戲裡點開聊天輸入欄 → 按 `Ctrl+Space` → 輸入訊息（任何語言） → Enter →
   工具切回遊戲、把翻好的英文**逐字自動鍵入**聊天欄 → 自己檢查後按 Enter 送出
   - 遊戲漏字/太快 → 調大 `config.json` 的 `type_delay`（每字元間隔秒數）
   - 打字期間游標要停在遊戲聊天欄，別去點別的視窗

## 設定（config.json）

> 這些欄位一般使用者也可透過疊加視窗的設定視窗（齒輪 ⚙）填寫，不必手改此檔。

| 欄位 | 說明 |
|------|------|
| `api.provider` | 翻譯服務商：`openai`（ChatGPT）、`claude`（Anthropic）、`custom`（自訂 OpenAI 相容端點，需自行架設，填 `api.base_url`） |
| `api.base_url` / `api.model` / `api.api_key` | `provider` 為 `custom` 時使用 `base_url`（自架 OpenAI 相容 API，`/v1/chat/completions`）；`openai`／`claude` 則固定用官方端點，只需 `model` 與 `api_key` |
| `target_language` | 收訊翻成的目標語言（人讀名稱，直接帶入提示詞），例如 `繁體中文（台灣）`、`日本語`、`Español`。發話固定翻成英文、來源語言一律自動判斷 |
| `api.thinking` | 模型是否啟用思考/reasoning（預設 `false`：關閉思考）。ChatGPT 官方端點只送它認得的 `reasoning_effort: "none"`；自訂端點併入常見後端的停用參數（`reasoning_effort`/`chat_template_kwargs.enable_thinking`/`think` 等）；Claude 不適用此欄（維持模型預設 adaptive）。設 `true` 則不帶任何思考參數、維持模型預設。不論設定為何，譯文中的 `<think>…</think>` 一律去除。嚴格伺服器若因某參數報錯，回報後可移除 |
| `poll_interval` | 輪詢間隔秒數（預設 0.4）。每輪讀一次聊天記錄全文、與上輪比對取新增行 |
| `max_parallel_translations` | 同時進行的收訊翻譯則數（1–8，預設 4）。1＝逐則排隊；大於 1 同時翻多則，訊息密集時更快跟上 |
| `game_path` | 遊戲根目錄（含 `Bin\`、`Data\` 的那層）；`null`（預設）＝自動偵測執行中的遊戲程序路徑。自動偵測失敗才需手動填（如非標準安裝） |
| `fade_seconds` | overlay 訊息淡出秒數（預設 0：永不淡出，靠滾動看歷史；>0 才會定時清除） |
| `max_messages` | 視窗保留的訊息則數上限（預設 200），超過移除最舊 |
| `hotkey` | 呼出輸入框的熱鍵（預設 `ctrl+space`） |
| `auto_show_input` | 遊戲開啟聊天輸入框時自動呼出翻譯輸入、關閉時自動收回（預設 `true`；熱鍵仍可用） |
| `overlay_alpha` | 視窗不透明度（半透明底板與縮小泡泡，預設 `0.80`，範圍 0.3–1.0；文字恆為不透明），小＝更透明 |
| `overlay` | 疊加視窗 `{x, y, width, height}`;`x/y` 為 `null` 用預設位置。拖曳標題列可移動、拖右下角可縮放，調整後自動存回此處 |

## 已知限制

- 遊戲聊天白名單：非白名單英文詞可能被遊戲過濾，任何翻譯工具都繞不過
- 收訊延遲約 `poll_interval` + 翻譯時間
- 依賴 wizwalker 的記憶體 pattern：遊戲改版後 pattern 可能失效（掛入時報
  `PatternFailed`），需等 [LaurenzNotHere fork](https://codeberg.org/LaurenzNotHere/wizwalker) 跟進更新後重跑 `uv sync`
- Ctrl+C 結束會自動解除 hook。即使被**強制結束**（工作管理員 kill、當機）遺留了
  hook，下次啟動也會**自動修復**（把遺留的原始 bytes 寫回、等同補做 unhook）、免重開
  遊戲——修復狀態存於 `%LOCALAPPDATA%\wizard101-chat-translator\`，依 PID + 模組基址比對，
  只對同一個仍在執行的遊戲程序套用。僅「狀態檔遺失的舊遺留」才需重開遊戲一次。
- 翻玩家發言（**含自己的 `[你]` 發言**與他人發言）；系統訊息（掉寶/經驗/升等）與遊戲除錯行不翻
- wizwalker 掛入與全域熱鍵（keyboard 套件）在部分環境需以**系統管理員**身分執行
