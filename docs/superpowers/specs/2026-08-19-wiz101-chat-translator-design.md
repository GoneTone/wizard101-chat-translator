# Wiz101 聊天翻譯助手 — 設計文件

日期:2026-08-19
狀態:已由使用者核准設計方向,待實作

## 目標

讓玩家在 Wizard101 裡與外國玩家順暢聊天:

1. **收訊**:遊戲聊天框裡的英文訊息即時翻成繁體中文,疊加顯示在遊戲畫面上(原文 + 譯文並列)。
2. **發訊**:玩家以繁體中文輸入,自動翻成英文並貼進遊戲聊天欄(不自動送出,由玩家確認後按 Enter)。

翻譯引擎為使用者自架的 OpenAI 相容 API(`/v1/chat/completions`)。

## 明確的範圍限制(YAGNI)

- 只支援聊天框區域的 OCR;頭頂聊天泡泡不追蹤(泡泡訊息同時出現在聊天框時即可翻到)。
- 只支援英文 → 繁中(收訊)與繁中 → 英文(發訊),不做多語言設定。
- 不讀取遊戲記憶體、不攔截封包、不注入 DLL — 純螢幕擷取與輸入模擬,零封號風險。
- 使用情境為視窗化 / 無邊框模式(使用者確認之遊玩模式)。

## 架構

單一 Python 應用,三個模組:

```
wiz101-chat-translator/
├── src/
│   ├── main.py            # 進入點:載入設定、啟動 reader 執行緒與 hotkey 監聽、tkinter 主迴圈
│   ├── config.py          # config.json 讀寫與預設值
│   ├── translator.py      # OpenAI 相容 API client(共用)
│   ├── reader/
│   │   ├── capture.py     # mss 截取聊天框區域
│   │   ├── ocr.py         # Windows OCR(winocr)逐行辨識
│   │   ├── dedup.py       # 行級去重(模糊比對)
│   │   └── overlay.py     # 疊加視窗(tkinter,置頂、半透明、滑鼠穿透)
│   ├── composer/
│   │   ├── input_box.py   # 熱鍵呼出的繁中輸入框(tkinter Toplevel)
│   │   └── paste.py       # 剪貼簿 + 焦點切換 + Ctrl+V 模擬
│   └── region_picker.py   # 首次啟動時滑鼠框選聊天框區域
├── tests/
│   ├── test_dedup.py
│   ├── test_translator.py # API 以 mock 測試
│   └── test_config.py
├── config.example.json
├── pyproject.toml
└── README.md
```

## 模組規格

### translator.py(共用翻譯 client)

- `httpx` POST 到 `{base_url}/v1/chat/completions`,參數來自 config:`base_url`、`model`、`api_key`(可空)。
- 兩個函式:`to_zh(text: str) -> str`、`to_en(text: str) -> str`。
- System prompt 要求:
  - 收訊(→繁中):口語化翻譯,保留遊戲術語原文(如 spell 名、地名可附原文)。
  - 發訊(→英文):口語、簡短、貼近遊戲聊天用語;避免生僻詞(降低被遊戲白名單過濾器擋掉的機率)。
- 逾時 10 秒;失敗丟出例外由呼叫端處理(overlay 顯示錯誤、input box 顯示錯誤訊息)。

### reader(收訊端)

- **capture.py**:`mss` 每 1.5 秒(config 可調 `poll_interval`)截取 config 中儲存的聊天框矩形。
- **ocr.py**:`winocr` 以英文語言包辨識,輸出逐行文字(依 y 座標排序)。
- **dedup.py**:維護最近 N 行(預設 200)的已見集合;新截圖的每一行與已見行做相似度比對(`difflib.SequenceMatcher`,ratio ≥ 0.9 視為同一行),只有全新行才送翻譯。純函式設計,可單元測試。
- **overlay.py**:tkinter 無邊框視窗,置頂 + 半透明;以 pywin32 設定 `WS_EX_TRANSPARENT | WS_EX_LAYERED` 讓滑鼠點擊穿透。每則訊息兩行:灰色小字原文、白色繁中譯文;最多同時顯示 8 則,超過 3 分鐘(config `fade_seconds`)自動移除。位置預設在聊天框上方,config 可調。
- 翻譯 API 連線失敗時,overlay 顯示紅色「⚠ 翻譯伺服器離線」列並以退避重試(5s → 15s → 30s 上限)。

### composer(發訊端)

- 全域熱鍵(預設 `ctrl+space`,config 可改;`keyboard` 套件監聽)呼出置頂輸入框,呼出時記住目前前景視窗 handle。
- 玩家打繁中按 Enter → 呼叫 `to_en()` → 英文寫入剪貼簿 → 還原前景視窗焦點 → 模擬 `Ctrl+V` 貼上 → **停住,不送出**。
- Esc 取消關閉輸入框。
- 翻譯失敗:輸入框內顯示錯誤,原文保留,可重按 Enter 重試。
- 前提約定(寫進 README):玩家先在遊戲中開啟聊天輸入欄,再按熱鍵。若貼上時機不對,英文仍在剪貼簿,可手動 Ctrl+V。

### region_picker.py

- 首次啟動(config 無 `chat_region`)時,全螢幕半透明遮罩讓使用者拖曳框選聊天框,座標存入 config。
- 之後可用系統匣選單或重新執行 `--pick-region` 重選。

### config.json

```json
{
  "api": { "base_url": "http://…", "model": "…", "api_key": "" },
  "chat_region": { "left": 0, "top": 0, "width": 0, "height": 0 },
  "poll_interval": 1.5,
  "fade_seconds": 180,
  "hotkey": "ctrl+space",
  "overlay_position": { "x": null, "y": null }
}
```

`config.json` 不進 git(含伺服器位址);提供 `config.example.json`。

## 錯誤處理總覽

| 情況 | 行為 |
|------|------|
| AI 伺服器連不上(收訊) | overlay 顯示離線提示,退避重試,恢復後繼續 |
| AI 伺服器連不上(發訊) | 輸入框顯示錯誤,保留原文可重試 |
| OCR 回傳空白 | 略過該輪 |
| 遊戲視窗不存在/最小化 | 暫停截圖輪詢,偵測到視窗恢復後繼續 |

## 測試策略

- **單元測試**(pytest):`dedup.py`(新行判定、模糊比對閾值、集合上限)、`translator.py`(mock httpx,驗證 request 組裝與錯誤路徑)、`config.py`(預設值、缺欄位容錯)。
- **手動驗證**:OCR 對遊戲字體的辨識率、overlay 顯示與滑鼠穿透、熱鍵→翻譯→貼上全流程,進遊戲實測。

## 已知限制

- Wizard101 聊天白名單過濾器:非白名單英文詞可能被遊戲擋掉,任何翻譯方案都無法繞過(也不應繞過)。
- OCR 為輪詢制,訊息顯示延遲約 1.5–3 秒(輪詢間隔 + 翻譯延遲)。
- 全螢幕獨佔模式下 overlay 不可見(使用者以視窗化/無邊框遊玩,不受影響)。
