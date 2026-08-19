# Wiz101 聊天翻譯助手

Wizard101 聊天 AI 翻譯:聊天框英文訊息即時翻成繁體中文疊加顯示(保留原文);
熱鍵輸入繁中自動翻成英文貼進遊戲聊天欄(**不會自動送出**,自己確認後按 Enter)。

純螢幕 OCR + 輸入模擬 — 不讀記憶體、不攔封包、不注入,無封號疑慮。

## 安裝

1. Python 3.11+(Windows)
2. `pip install mss winocr pillow httpx keyboard pywin32`
3. 複製 `config.example.json` 為 `config.json`,填入自架 AI 伺服器的
   `api.base_url` 與 `api.model`(OpenAI 相容 `/v1/chat/completions`)

## 使用

1. 以「視窗化 / 無邊框」模式開啟 Wizard101
2. `python -m src.main` — 第一次會要你拖曳框選聊天框範圍(之後可用
   `python -m src.main --pick-region` 重選)
3. 聊天框出現英文訊息 → 畫面上疊加「原文 + 繁中」;訊息 3 分鐘後淡出
4. 想發言:先在遊戲裡點開聊天輸入欄 → 按 `Ctrl+Space` → 打繁中 → Enter →
   英文自動貼進聊天欄 → 自己檢查後按 Enter 送出
   (若貼上時機不對,英文已在剪貼簿,手動 Ctrl+V 即可)

## 已知限制

- 遊戲聊天白名單:非白名單英文詞可能被遊戲過濾,任何翻譯工具都繞不過
- 訊息顯示延遲約 1.5–3 秒(輪詢 + 翻譯)
- 只翻聊天框內容;頭頂泡泡不追蹤
- 全域熱鍵(keyboard 套件)在部分環境需以系統管理員身分執行
