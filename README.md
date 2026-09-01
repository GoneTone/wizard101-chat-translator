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
   與紀錄檔 `app.log`、`messages.log` 都會存在 exe 同一個資料夾）
2. 雙擊執行：
   - 第一次執行（找不到 `config.json`）會跳出**首次設定精靈**：第一步選介面語言
     （繁體中文／简体中文／English），第二步選翻譯服務、填金鑰並測試連線，
     第三步選目標語言與熱鍵，存好設定後自動進入主流程
   - 之後每次啟動都直接進主流程，不會再跳精靈；介面語言之後仍可隨時在設定視窗
     （齒輪 ⚙）改，存檔即立即套用，不必重開程式
3. 開啟 Wizard101 並**登入進遊戲世界內**，疊加視窗才讀得到聊天訊息
4. 想改設定（服務商、金鑰、目標語言、熱鍵…）時，點疊加視窗標題列的齒輪
   （⚙）開啟設定視窗，存檔即套用，不必重開程式
5. 要結束程式時，點疊加視窗標題列右上角的 ✕（會先解除遊戲掛入再退出）
6. 每次啟動會自動檢查 GitHub 上有沒有新版本，有的話疊加視窗會多出一條藍色橫幅
   （點橫幅開下載頁，點右側 ✕ 關掉這次提醒）；也可以在設定視窗的「關於」分頁
   手動檢查更新、查看專案連結與紀錄檔所在資料夾

**防毒軟體誤判**：本工具會讀取遊戲聊天記憶體（掛入遊戲程序）並監聽全域熱鍵，
行為模式與部分惡意程式相似，可能被防毒軟體標記或直接刪除/隔離，這是誤判，
請自行評估風險後把程式加入防毒白名單。

**系統管理員權限**：一般情況（遊戲以一般權限啟動）不需要管理員權限。只有當
Wizard101 本身以**系統管理員**身分執行時，本工具也必須以系統管理員身分執行
（掛入遊戲程序需要對等或更高權限）；權限不足時疊加視窗會顯示
「⚠  權限不足」橫幅，關掉本工具、右鍵選「以系統管理員身分執行」再開一次即可。

**遊戲改版後掛不進去**：本工具靠固定的記憶體特徵（pattern）找到聊天控件，遊戲改版
可能讓這組特徵失效。此時疊加視窗會顯示「⚠  遊戲版本不相容」橫幅——先把**遊戲與
本工具**都更新到最新版；若兩者都已是最新仍出現這條橫幅，代表本工具尚未跟上這個遊戲
版本，只能等新版釋出（可在設定視窗的「關於」分頁檢查更新）。這種情況下不會有任何
東西被寫進遊戲，等待期間放著不管也不會有副作用。

## 開發者

以下為原始碼開發、除錯、打包用的流程。

### 安裝

以 [uv](https://docs.astral.sh/uv/) 管理隔離環境（Windows、Python 3.11+）：

1. `uv sync` —— 建立 `.venv` 並依 `pyproject.toml` 裝好所有相依
   （含收訊用的 wizwalker；已於 `[tool.uv.sources]` 指向有跟進最新 client
   pattern 的 [LaurenzNotHere fork](https://codeberg.org/LaurenzNotHere/wizwalker)，
   官方 PyPI 版 pattern 過舊、對不上現行 client）
2. `uv run run.py` —— 第一次執行會跑首次設定精靈（選服務商、填金鑰或自架
   伺服器網址、測試連線），設定寫進專案根目錄的 `config.json`（不進版控）

> 遊戲改版導致掛入報 `PatternFailed` 時，更新 fork 後重跑 `uv sync`（或
> `uv lock --upgrade-package wizwalker`）取得新 pattern。

### 新增介面語言

介面語言清單由 `src/i18n/` 底下的語言檔掃描而來，**新增一個語言不必改任何程式碼**：

1. 複製 `src/i18n/zh-TW.json`（來源語言，key 最齊全）成 `src/i18n/<語言碼>.json`，
   例如 `ja.json`，把每一則文案翻好
2. 檔案開頭這三個欄位是該語言自己的資料，不是給譯者翻的文案：

   | 欄位 | 說明 |
   |------|------|
   | `language.name` | 該語言的自稱（endonym），例如 `日本語`。語言選單在任何介面語言下都顯示它、不翻譯；也是這個語言的使用者首次執行時預設的 `target_language` |
   | `language.font` | 介面字族，例如 `Yu Gothic UI`。沒宣告則退 `Segoe UI` |
   | `language.locales` | 要吃下的 Windows locale 名稱，空白分隔（例如 `zh_TW zh_HK zh_MO`）。**同語言不同字集才需要指名**；`ja_JP` 這種靠語言前綴就對得上語言碼 `ja`，留空即可 |

3. `uv run pytest` —— `tests/test_i18n.py` 會檢查新語言檔的 key 與來源語言一致、
   變數（`{app}` 等）沒被翻壞、metadata 有填

打包時 `build.spec` 以 `src/i18n/*.json` 收錄，新檔會自動被帶進 exe。

### 打包（exe）

```
uv run pyinstaller build.spec --noconfirm
```

產物在 `dist/Wizard101ChatTranslator.exe`，單一 windowed exe（無主控台黑窗）。
分發時只需這個 exe;`config.json`、`app.log` 與 `messages.log` 會在使用者第一次執行時
自動建立在 exe 同一個資料夾（見上方「一般使用者」）。

### 紀錄檔

兩份紀錄都以「每次啟動一段」分段、只保留近 7 天，每行開頭是 UTC＋0 時戳：

- `app.log`：程式診斷輸出（掛入、翻譯請求、設定套用、例外等）。
- `messages.log`：收訊端讀到的**原始**聊天內容，不做任何清理與過濾
  （`RAW` 為含標記的原文、含系統訊息，只排除遊戲自己灌進聊天控件的
  `[WARN]`／`[ERRO]`／`[DBGM]` 除錯行；`OUT` 為實際送去翻譯的行），
  訊息漏翻／重複翻譯之類的問題請一併附上這份。

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

> 更新檢查看的是 GitHub 的 **Release**（`/releases/latest`），只推 tag 不建 Release
> 的話使用者端不會收到更新提醒。

### 使用（從原始碼執行）

1. 開啟並登入 Wizard101 到遊戲世界內
2. `uv run run.py`（等同 `uv run python -m src.main`）
   - 若顯示「權限不足」：遊戲以系統管理員身分執行、掛不進去，改以**系統管理員**身分開終端再執行
   - 若狀態停在「連線遊戲中…」或顯示「遊戲未就緒／連線中斷」，確認遊戲已啟動並登入進世界內
   - 若掛入時報 `PatternFailed`、或橫幅顯示「遊戲版本不相容」：遊戲已改版、pattern 過舊，
     更新 fork 後重跑 `uv sync`
3. 聊天出現訊息 → 疊加視窗顯示「原文 + 目標語言譯文」（最新在最下，可向上滾動看歷史；預設不自動清除）
   - 可互動：**拖曳頂端標題列移動、拖任一邊或任一角縮放**（右下角另有把手；標題列上的
     ⚙／─／✕ 按鈕本身仍是按鈕，不觸發縮放），位置與大小自動存檔
   - 注意：視窗不滑鼠穿透，它蓋住的那塊區域點擊不會傳到遊戲
4. 想發言：先在遊戲裡點開聊天輸入欄 → 按 `Ctrl+Space` → 輸入訊息（任何語言） → Enter →
   工具切回遊戲、把翻好的英文**逐字自動鍵入**聊天欄 → 自己檢查後按 Enter 送出
   - 遊戲漏字/太快 → 調大 `config.json` 的 `type_delay`（每字元間隔秒數）
   - 打字期間游標要停在遊戲聊天欄，別去點別的視窗

## 設定（config.json）

> 這些欄位一般使用者也可透過疊加視窗的設定視窗（齒輪 ⚙）填寫，不必手改此檔。

| 欄位 | 說明 |
|------|------|
| `ui_language` | 介面語言，語言碼對應 `src/i18n/<語言碼>.json`，目前內建 `zh-TW`（繁體中文（台灣））、`zh-CN`（简体中文（中国））、`en`（English）；`null`（預設）＝下次啟動時依 Windows 系統語言自動判定。也可隨時在設定視窗（齒輪 ⚙）改，立即套用 |
| `api.provider` | 目前選用的翻譯服務商：`openai`（ChatGPT）、`claude`（Anthropic）、`custom`（自訂 OpenAI 相容端點，需自行架設） |
| `api.openai` / `api.claude` / `api.custom` | 每家各存一份設定，互不覆蓋——換家再換回來不必重填。生效的是 `provider` 指到的那一份。每家只存自己用得到的欄位，載入時會清掉不屬於該家的欄位並回寫 |
| `api.<服務商>.model` / `.api_key` | 三家共通。`openai`／`claude` 用官方端點（網址寫死在程式裡），填 `model` 與 `api_key` 即可 |
| `api.custom.base_url` | 僅 `custom`：自架 OpenAI 相容 API 的網址（`/v1/chat/completions`），只填到主機與連接埠 |
| `target_language` | 收訊翻成的目標語言（人讀名稱，直接帶入提示詞），例如 `繁體中文（台灣）`、`日本語`、`Español`。發話固定翻成英文、來源語言一律自動判斷 |
| `translate_system_messages` | 是否翻譯並顯示遊戲系統訊息（掉寶、經驗、升等廣播、好友邀請等，預設 `false`）。開啟後系統訊息會與玩家對話依遊戲內順序交錯顯示，並佔用 `max_messages` 的額度 |
| `api.openai.thinking` / `api.custom.thinking` | 模型是否啟用思考/reasoning（預設 `false`：關閉思考）。ChatGPT 官方端點只送它認得的 `reasoning_effort: "none"`；自訂端點併入常見後端的停用參數（`reasoning_effort`/`chat_template_kwargs.enable_thinking`/`think` 等）。設 `true` 則不帶任何思考參數、維持模型預設。不論設定為何，譯文中的 `<think>…</think>` 一律去除。嚴格伺服器若因某參數報錯，回報後可移除 |
| `api.claude.effort` | 僅 `claude`：思考深度。`auto`（預設）＝不帶參數、由模型自行決定（adaptive）；`low`＝送 `output_config.effort="low"` 壓到最低，譯文更快也更省 Token。Claude 沒有「完全不思考」的選項，故不用 `thinking` 開關 |
| `poll_interval` | 輪詢間隔秒數（預設 0.4）。每輪讀一次聊天記錄全文、與上輪比對取新增行 |
| `max_parallel_translations` | 同時進行的收訊翻譯則數（1–8，預設 4）。1＝逐則排隊；大於 1 同時翻多則，訊息密集時更快跟上 |
| `game_path` | 遊戲根目錄（含 `Bin\`、`Data\` 的那層）；`null`（預設）＝自動偵測執行中的遊戲程序路徑。自動偵測失敗才需手動填（如非標準安裝） |
| `fade_seconds` | overlay 訊息淡出秒數（預設 0：永不淡出，靠滾動看歷史；>0 才會定時清除） |
| `max_messages` | 視窗保留的訊息則數上限（預設 200），超過移除最舊 |
| `hotkey` | 呼出輸入框的熱鍵（預設 `ctrl+space`） |
| `auto_show_input` | 遊戲開啟聊天輸入框時自動呼出翻譯輸入、關閉時自動收回（預設 `true`；熱鍵仍可用） |
| `overlay_alpha` | 視窗不透明度（半透明底板與縮小泡泡，預設 `0.80`，範圍 0.3–1.0；文字恆為不透明），小＝更透明。縮小後的泡泡會在此值上再打八折（不低於 0.30），比主視窗更低調 |
| `overlay` | 疊加視窗 `{x, y, width, height}`;`x/y` 為 `null` 用預設位置。拖曳標題列可移動、拖四邊／四角可縮放，調整後自動存回此處 |
| `input_width` | 翻譯輸入框寬度（預設 460）。輸入框可左右拖曳縮放，關閉時存回此處;高度依提示文字行數自動調整、不記憶 |

## 已知限制

- 遊戲聊天白名單：非白名單英文詞可能被遊戲過濾，任何翻譯工具都繞不過
- 收訊延遲約 `poll_interval` + 翻譯時間
- 依賴 wizwalker 的記憶體 pattern：遊戲改版後 pattern 可能失效（掛入時報
  `PatternFailed`，或 hook 掛上了卻不被觸發），需等
  [LaurenzNotHere fork](https://codeberg.org/LaurenzNotHere/wizwalker) 跟進更新後重跑
  `uv sync`。兩種情形疊加視窗都顯示「⚠  遊戲版本不相容」橫幅，與「遊戲沒開」分得開
- Ctrl+C 結束會自動解除 hook。即使被**強制結束**（工作管理員 kill、當機）遺留了
  hook，下次啟動也會**自動修復**（把遺留的原始 bytes 寫回、等同補做 unhook）、免重開
  遊戲——修復狀態存於 `%LOCALAPPDATA%\wizard101-chat-translator\`，依 PID + 模組基址比對，
  只對同一個仍在執行的遊戲程序套用。僅「狀態檔遺失的舊遺留」才需重開遊戲一次。
- 翻玩家發言（**含自己的 `[你]` 發言**與他人發言）；系統訊息預設不翻，可在設定視窗的
  「進階」分頁開啟（開啟後譯文會快取在 `%LOCALAPPDATA%\wizard101-chat-translator\`，
  已翻過的句型再出現就直接用快取、不必再打 API；同一輪內湧入的重複句型因為都還沒翻完，
  仍會各打一次）。全服升等廣播因每則的玩家名不同，快取無法命中。快取跨啟動保留，
  換服務商／模型／目標語言時自動整份作廢；譯得不滿意想重翻，可在設定視窗的
  「關於」分頁按「清除」手動清掉。
  遊戲除錯行一律不翻
- wizwalker 掛入與全域熱鍵（keyboard 套件）只在遊戲以系統管理員身分執行時需要同樣
  提權（完整性等級要對等），一般情況不必；權限不足時橫幅會直接指出解法

## 圖示

程式圖示（`src/assets/icon.ico`）改作自 Wizard101 的遊戲圖示，右上角疊上「文A」翻譯徽章，
exe 與所有視窗共用同一份。底圖著作權屬 KingsIsle Entertainment，本專案與其並無隸屬關係。
