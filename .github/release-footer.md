## Notice 注意

**_This software injects into the game process to read in-game conversations. KingsIsle does not sanction this, and it may be treated as a breach of the [Wizard101 Terms of Use](https://www.wizard101.com/game/termsofuse); under those terms KingsIsle may suspend an account for any reason, or for no reason. Use at your own risk._**

**_本軟體會掛入（注入）遊戲程序讀取對話內容，此行為未經 KingsIsle 認可，可能被認定違反 [Wizard101 服務條款](https://www.wizard101.com/game/termsofuse)；依該條款，KingsIsle 可基於任何理由（或無需理由）停權，請自行斟酌。_**

**_Because it reads game memory and listens for a global hotkey, antivirus software may flag or quarantine it. This is a false positive; add it to your allowlist if you decide to use it. This software is guaranteed to be virus-free._**

**_本軟體會讀取遊戲記憶體並監聽全域熱鍵，行為模式與部分惡意程式相似，可能被防毒軟體誤判並刪除／隔離；請自行評估後加入白名單，本軟體保證無毒。_**

## Updating 更新

If you are updating from an earlier version, quit the software first (click the ✕ on the overlay's title bar) — while it is running the exe is locked by Windows and still hooked into the game. Then overwrite the old `Wizard101ChatTranslator.exe` in the same folder; your existing `config.json` is kept, so nothing has to be set up again.

若是從舊版更新，請先關閉本軟體（點疊加視窗標題列右上角的 ✕） —— 程式執行中，exe 會被 Windows 鎖住、也還掛在遊戲上。關掉後覆蓋同一個資料夾裡的舊 `Wizard101ChatTranslator.exe` 即可，原本的 `config.json` 會保留，設定不必重填。

## Reporting Issues 回報問題

If you run into any problem, please report it and attach the `app.log` next to the exe (and `messages.log` for missing or duplicated translations): <https://github.com/GoneTone/wizard101-chat-translator/issues>

如果發現任何問題，請附上 exe 旁的 `app.log`（訊息漏翻／重複翻譯的問題請一併附上 `messages.log`）回報：<https://github.com/GoneTone/wizard101-chat-translator/issues>
