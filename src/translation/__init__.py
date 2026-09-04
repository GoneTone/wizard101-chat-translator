"""翻譯端：對外打 API 的 client（translator）、收訊平行翻譯的工作池（pool）與總量閘
（gate）、系統訊息譯文快取（cache），以及收訊與發話共用的近期對話上下文（context）。
不碰遊戲、不碰 UI；reader 讀到的行經 main 交到這裡，結果經 ui_queue 回到 overlay。"""
