# Wizard101 聊天翻譯助手：多客戶端（雙開）收訊設計

日期：2026-09-21
狀態：已實作。

## 目標

使用者同時開多個遊戲客戶端（不同帳號）時，**每個客戶端的聊天都要被翻譯**，並在疊加視窗
上標示每則訊息來自哪一個客戶端；發話、Ctrl+V 與自動呼出輸入框都要對準使用者當下面對的
那個客戶端。單開的使用者外觀與行為完全不變。

## 非目標

- **不讀角色名**：實機探測（見下）證實只掛 root_window hook 時，進世界後 UI 樹裡沒有角色
  名，要等使用者開過一次法術書才會出現 `CharacterName` 節點；wizwalker 的 player hook 讀
  得到，但那是再多注入一支 hook，違反「只掛 root_window hook」的專案規則。本案一律用
  掛入順序編號標示來源。
- **不做多個疊加視窗**、不做「只顯示前景客戶端訊息」的過濾模式。
- **不加設定項**：偵測到幾個客戶端就掛幾個，沒有開關、沒有上限設定。
- **不改差分演算法、標記解析、翻譯提示詞**。

## 現況與問題

`WizChatReader._connect()` 呼叫 wizwalker `ClientHandler.get_new_clients()` 後直接拿
`clients[0]`；`EnumWindows` 依 Z 軸由上往下，所以掛到的是啟動當下疊在最上層的遊戲視窗，
之後鎖死在該 PID。結果：

- 另一個客戶端的聊天完全不翻譯，也沒有任何提示。
- 被掛入的客戶端關掉後才斷線重連，此時靜默改掛剩下的那個。
- 自動呼出輸入框、錨點、Ctrl+V 的單行判定都讀被掛入客戶端的 `chatEditContainer`，
  使用者在另一個客戶端貼多行文字會被拆成多段送出。
- 發話熱鍵與框選 OCR 本來就跟前景視窗走，兩個客戶端都能用。
- 開兩份翻譯器不可行：單一實例 mutex 與 `instance_watch` 會砍掉第二份。

## 實機探測結論（2026-09-21）

以只掛 root_window hook 的方式傾印整棵 UI 樹，三個時間點：

| 階段 | 節點數 | `CharacterName` | 其他含角色名的節點 |
|---|---|---|---|
| 選角畫面 | 50 | 無 | `txtName`＝目前選到的角色，會隨選取改變 |
| 進世界、未開過法術書 | 382 | 無 | 無 |
| 進世界、開過法術書 | 1043 | `DeckConfiguration → … → TitleScroll → CharacterName`，值為角色名 | 無 |

附帶發現：選角畫面就能成功掛入（hook 0.3 秒就緒），法術書子樹第一次打開時建立、關閉後
仍留在樹上。

## 決策摘要

| 決策 | 選擇 | 理由 |
|---|---|---|
| 並行模型 | 每個客戶端一條 `reader_loop` 執行緒＋一個發現迴圈 | 掛入最多卡 15 秒，單執行緒輪詢會讓另一個客戶端停收；全非同步改寫沒省到東西 |
| reader 綁定 | `WizChatReader(hwnd)`，`_connect` 直接 `wizwalker.Client(hwnd)` | 不再靠 `get_new_clients()` 的順序 |
| 視窗列舉 | wizwalker `utils.get_all_wizard_handles()`（視窗類別名） | 比 exe 路徑列舉精準，且是同一套 wizwalker 認得的 handle |
| 來源標記 | 掛入順序編號，最小空號，帶圈數字 `①②…` | 零額外讀取；使用者在任一客戶端講一句即可對照 |
| 標記顯示時機 | 「多客戶端模式」旗標：第一次同時兩個編號在線就開，開了不關 | 避免列表上半有標記下半沒有；剛關掉的帳號淡出期內仍看得出是誰的 |
| 翻譯上下文 | 每客戶端一份 `ChatContext` | 不同世界、不同對話，混用會誤導模型 |
| 狀態列／橫幅 | 共用 `StatusBoard` 彙整，各執行緒只回報自己 | overlay 只有一條狀態列 |
| 輸入框事件 | 回呼帶 `hwnd`，只認前景視窗 | 使用者在 B 時 A 的聊天框開關不該影響他 |
| 設定 | 無新增 | YAGNI |

## 架構

```
supervisor（1 條執行緒）
  每 5 秒 get_all_wizard_handles()
  ├─ 新 hwnd → 配最小空號 → 起 reader_loop(hwnd, slot) 執行緒
  ├─ 執行緒結束 → 釋出編號
  ├─ 同時 ≥2 編號在線 → overlay 多客戶端模式（一次性）
  └─ 零視窗 → StatusBoard 回報無客戶端

reader_loop(hwnd, slot)（每客戶端 1 條）
  WizChatReader(hwnd) ─ ChatContext ─ _InputWatch ─ MessageLog(slot)
  → 共用：TranslationPool ×2、TranslationCache、ui_queue、overlay、StatusBoard、msg_id 計數器
```

### `WizChatReader`

- 建構子新增必要參數 `hwnd`。
- `_connect`：`self._client = wizwalker.Client(hwnd)`（建構時 pymem 開程序，`CouldNotOpenProcess`
  照舊分類為 `GameAccessDenied`）；不再建 `ClientHandler`。
- `_teardown`：改呼叫 `self._client.close()`；unhook 成功才清 hook 狀態檔，邏輯不變。
- hook 狀態檔已按 PID 分檔，`sweep`／`_repair_leaked_hooks` 不動。
- 其餘（差分、`input_open`、`input_box_screen_rect`、`HOOK_READY_TIMEOUT`）全部不動。

### `src/reader/supervisor.py`（新）

實作時 `StatusBoard` 與橫幅函式獨立成 `src/reader/status.py`（避免 loop ↔ supervisor 循環匯入），
supervisor 只管生命週期。

- `supervise(stop, spawn, enumerate_windows, on_multi_client, board, interval)`：主迴圈。
  `spawn(hwnd, slot)` 由 `main` 提供、閉包帶著 cfg／pool／overlay 等共用物件。
  - `enumerate_windows()` 預設為 wizwalker 的 `get_all_wizard_handles`，測試注入假函式。
  - 維護 `active: dict[hwnd, (slot, thread)]`。新 hwnd → `slot = 最小未用正整數` → `spawn(hwnd, slot)`
    起執行緒。已結束的執行緒（`is_alive()` 為 False）→ 從 `active` 移除、釋號。
  - 第一次觀察到本輪列舉到的遊戲視窗數（`len(windows)`，非 `active` 的執行緒數——殭屍執行緒不算）
    ≥ 2 → 排 `overlay.set_multi_client()` 進 `ui_queue`，之後不再呼叫。
    先呼叫 `on_multi_client()` 成功後才鎖存，回呼拋例外時下一輪重試。
  - `stop` 設定後以**一個共用的** `JOIN_TIMEOUT`（8 秒）預算 join 所有子執行緒（每條各自
    `reader.close()` unhook）；`main.shutdown` 以 `JOIN_TIMEOUT + 2` 秒 join supervisor，確保
    「仍存活」的 log 一定寫得出來。
- `StatusBoard`：加鎖；`report(slot, state, game_issue)`、`drop(slot)`、`refresh()`。
  零客戶端由空的回報表自然算出等待遊戲中＋找不到遊戲，不需專門方法。
  每次呼叫後重算整體狀態與橫幅，只在結果改變時 `ui_queue.put(...)`。
  - 狀態優先序：`translating` > `listening` > `locating` > `access_denied`／`version_mismatch`
    > `waiting_game`（無視窗或全部一般性失敗）。
  - 橫幅 `game_issue`：任一編號為 `notice.access_denied`／`notice.version_mismatch` → 該 key；
    否則沒有任一編號 anchored → `notice.game_missing`；否則 `None`。之後仍交
    `translation_banner(game_issue, pool, system_pool)` 決定最終橫幅。

### `reader_loop`

- 簽名新增 `hwnd`、`slot`、`board`、`msg_ids`（共用計數器）。`_OverlayFeed` 退場，狀態改走 `board.report(slot, ...)`；
  執行緒結束前 `board.drop(slot)`。
- 每輪開頭多一個結束條件：`IsWindow(hwnd)` 為 False → log、`reader.close()`、返回。
  視窗在但掛入失敗（更新器畫面等）照舊每 5 秒重試。
- `overlay.add_message(..., slot=slot)`。
- `ChatContext` 每客戶端一份，由 `main` 的 `spawn` 建立並登錄在 hwnd→context 對照表傳入
  `reader_loop`：發話翻譯要帶的上下文改取「譯文要打回去的那個客戶端」的聊天
  （`InputBox.target_hwnd`），不再有全域一份。
- `_InputWatch` 回呼改為 `on_input_open(hwnd, anchor)`／`on_input_close(hwnd)`。
- `msg_ids`：`itertools.count(1)` 包一把鎖的 `next()`，全域唯一。

### overlay／`MessageList`

- `slot_marker(slot) -> str`（`ui/message_list.py`，純函式）：1–20 對應 U+2460 起的帶圈數字，
  超過退回 `[n]`。只有 UI 需要它，reader 端不依賴。
- `add_message(..., slot: int | None = None)`；`_Message` 新增 `slot`。
- `set_multi_client()`：旗標打開；對既有每列以 `itemconfigure("txt", ...)` 把原文行改成
  `f"{slot_marker(slot)} {original}"`（沿用 `update_message` 的作法）。旗標開著時新列直接
  帶標記。`_Message.original` 永遠存乾淨原文（`visible_messages()` 等程式面使用）；滑鼠選取
  複製為所見即所得，拖到標記就會一併複製 —— `Selection` 讀的是畫面上的字，剝除標記得改游標
  偏移計算，不值得。
- 系統訊息同規則。旗標預設關 → 單開外觀不變。

### `main.py`

- 起 supervisor 執行緒取代直接起 `reader_loop`；`App.reader_thread` 指向 supervisor。
- `game_chat_open: threading.Event` → `open_chat_windows: set[int]` 加鎖。
- `on_game_input_open(hwnd, anchor)`：加入集合；僅當 `GetForegroundWindow() == hwnd` 才設錨點、
  （依 `auto_show_input`）呼出，並記 `shown_for = hwnd`。
- `on_game_input_close(hwnd)`：自集合移除；僅當 `hwnd == shown_for` 才清錨點、收起。
- `on_paste_hotkey`：`single_line = hwnd in open_chat_windows`（hwnd 為當下前景視窗）。
- `find_game_window()`、`on_hotkey`、`InputBox._target_hwnd` 不動（已是前景／最上層語意）。
- 發話翻譯的上下文：`context_for(input_box.target_hwnd)` 取譯文要打回去的那個客戶端的
  `ChatContext`；hwnd→context 對照表由 `spawn` 填入，不清理（每筆是有界 deque）。

## 錯誤處理

- 每條執行緒沿用既有 `try/except`：掛入失敗、讀取例外只影響自己的編號。
- 視窗消失時 unhook 失敗（程序已死）→ hook 狀態檔留給 `sweep`，與現況一致。
- hwnd 被系統回收重用：舊執行緒退出前新視窗會被視為同一個而暫不起新執行緒，下一次掃描補上。
- supervisor 迴圈本體包 `try/except` 記 log 後繼續，列舉失敗不可讓整個收訊端死掉。

## log 與診斷

- `[reader]` 既有訊息一律補 `slot=N`，例如 `attached to game (slot=1, pid=…, hook_ready_in=…)`。
- supervisor 新增：`client window appeared hwnd=0x… slot=N`、`reader thread exited hwnd=0x… slot=N freed`、
  `multi-client mode on (slots=…)`。
- `MessageLog` 每編號一個實例、共用同一串流，每行前綴 `[slot=N]`。
- 全部英文；不記任何金鑰。

## 測試

- `tests/test_supervisor.py`：最小空號配置與釋出；假列舉＋假 spawn 驗證起／收；`≥2` 才觸發
  多客戶端模式且只觸發一次；`stop` 後 join 子執行緒；`StatusBoard` 優先序與橫幅規則。
- `tests/test_reader_loop.py`：狀態回報到 board；`IsWindow` 為 False 即結束並 `close()`；
  輸入框回呼帶 hwnd；`add_message` 帶 slot。
- `tests/test_reader_connect.py`：patch 對象由 `ClientHandler` 改為 `Client`。
- `tests/test_overlay.py`：預設無標記；`set_multi_client()` 後既有列補標記、新列帶標記；
  `_Message.original` 為乾淨原文；`slot_marker` 含超過 20 的退路。
- `tests/test_main.py`（或 `test_paste.py`）：單行判定改查集合；自動呼出只認前景 hwnd。
- 實機驗收：雙開兩個帳號，確認兩邊聊天都出現且標記正確、切窗發話打進正確客戶端、
  在未掛入順序較後的客戶端 Ctrl+V 多行仍為單行、關掉一個客戶端後標記不消失。

## 文件

- README 中英兩份各加一句：同時開多個遊戲客戶端時，每個客戶端的聊天都會翻譯並標示來源。
  不寫鍵名、不寫介面文字。
