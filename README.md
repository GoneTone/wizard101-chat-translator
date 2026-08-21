# Wiz101 聊天翻譯助手

Wizard101 聊天 AI 翻譯:讀取遊戲聊天訊息即時翻成繁體中文疊加顯示(保留原文);
熱鍵輸入繁中自動翻成英文,逐字**自動鍵入**遊戲聊天欄(遊戲不支援貼上;**不會自動送出**,自己確認後按 Enter)。

收訊透過 [wizwalker](https://codeberg.org/LaurenzNotHere/wizwalker) 掛入遊戲、
直接讀聊天顯示控件(`chatLog`)的全文:100% 準確(無 OCR 辨識誤差)、
訊息有序、含他人與自己的發言及發送者身分,不攔封包。

## ⚠ 重要:封號風險

本工具靠 wizwalker **掛入(注入)遊戲程序** —— 為了讀聊天控件,它會
`WriteProcessMemory` 寫入 code cave 並建立執行緒下 hook(**非純讀**)。
這**違反 Wizard101 服務條款**,比單純讀取更具侵入性、**封號風險更高**,
**可能導致帳號被停權,請自行斟酌**。

## 安裝

以 [uv](https://docs.astral.sh/uv/) 管理隔離環境(Windows,Python 3.11+):

1. `uv sync` —— 建立 `.venv` 並依 `pyproject.toml` 裝好所有相依
   (含收訊用的 wizwalker;已於 `[tool.uv.sources]` 指向有跟進最新 client
   pattern 的 [LaurenzNotHere fork](https://codeberg.org/LaurenzNotHere/wizwalker),
   官方 PyPI 版 pattern 過舊、對不上現行 client)
2. 複製 `config.example.json` 為 `config.json`,填入自架 AI 伺服器的
   `api.base_url` 與 `api.model`(OpenAI 相容 `/v1/chat/completions`)

> 遊戲改版導致掛入報 `PatternFailed` 時,更新 fork 後重跑 `uv sync`(或
> `uv lock --upgrade-package wizwalker`)取得新 pattern。

## 使用

1. 開啟並登入 Wizard101 到遊戲世界內
2. `uv run run.py`(等同 `uv run python -m src.main`)
   - 若狀態一直停在「連線遊戲中…」或顯示「遊戲未就緒／連線中斷」,以**系統管理員**身分開終端再執行(wizwalker 掛入常需要)
   - 若掛入時報 `PatternFailed`:遊戲已改版、pattern 過舊,更新 fork 後重跑 `uv sync`
3. 聊天出現訊息 → 疊加視窗顯示「原文 + 繁中」(最新在最下,可向上滾動看歷史;預設不自動清除)
   - 視窗固定大小、可互動:**拖曳頂端標題列移動、拖右下角把手縮放**,位置與大小自動存檔
   - 注意:視窗不再滑鼠穿透,它蓋住的那塊區域點擊不會傳到遊戲
4. 想發言:先在遊戲裡點開聊天輸入欄 → 按 `Ctrl+Space` → 打繁中 → Enter →
   工具切回遊戲、把英文**逐字自動鍵入**聊天欄 → 自己檢查後按 Enter 送出
   - 遊戲漏字/太快 → 調大 `config.json` 的 `type_delay`(每字元間隔秒數)
   - 打字期間游標要停在遊戲聊天欄,別去點別的視窗

## 設定(config.json)

| 欄位 | 說明 |
|------|------|
| `api.base_url` / `api.model` / `api.api_key` | 自架 OpenAI 相容 API |
| `poll_interval` | 輪詢間隔秒數(預設 0.4)。每輪讀一次聊天記錄全文、與上輪比對取新增行 |
| `game_path` | 遊戲根目錄(含 `Bin\`、`Data\` 的那層);`null`(預設)= 自動偵測執行中的遊戲程序路徑。自動偵測失敗才需手動填(如非標準安裝) |
| `fade_seconds` | overlay 訊息淡出秒數(預設 0:永不淡出,靠滾動看歷史;>0 才會定時清除) |
| `max_messages` | 視窗保留的訊息則數上限(預設 200),超過移除最舊 |
| `hotkey` | 呼出輸入框的熱鍵(預設 `ctrl+space`) |
| `overlay` | 疊加視窗 `{x, y, width, height}`;`x/y` 為 `null` 用預設位置。拖曳標題列可移動、拖右下角可縮放,調整後自動存回此處 |

## 已知限制

- 遊戲聊天白名單:非白名單英文詞可能被遊戲過濾,任何翻譯工具都繞不過
- 收訊延遲約 `poll_interval` + 翻譯時間
- 依賴 wizwalker 的記憶體 pattern:遊戲改版後 pattern 可能失效(掛入時報
  `PatternFailed`),需等 [LaurenzNotHere fork](https://codeberg.org/LaurenzNotHere/wizwalker) 跟進更新後重跑 `uv sync`
- 正常用 **Ctrl+C 結束會自動解除 hook**,可重複執行。但若程式被**強制結束**
  (工作管理員 kill、當機)或同一 client 反覆掛入/卸載多次,遊戲內殘留的 hook 可能
  無法完整還原,下次掛入會報 `PatternFailed` —— 此時**重開遊戲客戶端**即可
- 翻玩家發言(**含自己的 `[你]` 發言**與他人發言);系統訊息(掉寶/經驗/升等)與遊戲除錯行不翻
- wizwalker 掛入與全域熱鍵(keyboard 套件)在部分環境需以**系統管理員**身分執行
