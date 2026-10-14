# Wizard101 聊天翻譯助手：收訊改用 AddLine hook＋翻譯視窗分頁同步設計

日期：2026-10-14
狀態：設計完成，待實作。

## 目標

收訊端目前每輪讀聊天控件 `chatLog` 的全文，再與上一輪差分出新增行。`chatLog` 只是遊戲「目前分頁」緩衝的複本，切分頁、傳送、登出入時整份內容會被換掉，差分只能靠一整串防線（看過集合、暖機、空讀判定基準過期、輸入框關聯放行、暴增吸收、鏡射節點剔除）去**猜**哪些是新訊息。實際使用中，這些防線仍會重複吐出舊訊息或吞掉新訊息，不可靠。

本案改成：

1. **收訊事件驅動**：在遊戲的 `ChatBox::AddLine` 掛 hook。遊戲每把一行加進聊天框，hook 就把該行連同序號複製進環形緩衝；翻譯器依序號讀取。結構上不會重複，也不會漏；就算漏了（緩衝被覆寫），也會留下明確的 log。
2. **翻譯視窗有分頁，並自動跟著遊戲**：每行訊息帶著所屬分頁，翻譯視窗為每個分頁各保留一份訊息清單。分頁列的名稱與未讀數直接取自遊戲按鈕上的文字，目前分頁跟著遊戲切換。
3. **雙開時只顯示前景客戶端**：翻譯視窗顯示目前前景那個遊戲客戶端的分頁與訊息。

兩者寫成同一份設計、一起上線：收訊單獨上線的話，過渡期翻譯視窗只能把各分頁混在一起，或只顯示目前分頁，行為會很怪。

## 非目標

- **不保留舊的差分當退路**：hook 掛不上（遊戲改版）就顯示「遊戲版本不相容」，等本專案更新支援。使用者明確接受改版後暫時失效。
- **翻譯視窗的分頁不能點**：只跟著遊戲切換，不提供在翻譯視窗上獨立切換分頁（使用者選「自動跟著遊戲」）。
- **不回補連上之前的歷史**：hook 只看得到掛上之後新增的行，與現況一致。
- **私訊不特別處理**：私訊在遊戲裡就是寫進主要分頁、帶洋紅色碼，`markup.py` 本來就會取出顏色。
- **不改翻譯提示詞、翻譯池、快取、框選 OCR、發話路徑本身**。

## 反目標（做到了也算失敗）

- 任何情境下同一則訊息在**同一個客戶端**被翻譯兩次或在翻譯視窗出現兩次。雙開時兩個角色都聽到的同一句話，兩個客戶端各翻一次，不算違反（現況就是如此；不做跨客戶端的內容比對去重，那正是下一條禁止的事）。
- 為了「保險」又加回內容比對式的去重或過濾。

## 依據（2026-10-14，遊戲版本 `W.1.610.21`，`r806919.Wizard_1_610`）

### 靜態分析（拆 `Root.wad` 與反組譯 `WizardGraphicalClient.exe`）

| 項目 | 證據 |
|---|---|
| 聊天由伺服器以結構化訊息送來 | `GameMessages.xml` 的 `MSG_RADIALCHAT`／`MSG_DIRECTEDCHAT`／`MSG_CHANNELCHAT`，欄位 `SourceName`、`SourceID`、`Message`、`Filter` |
| 封包加密，不能側錄 | exe 內 `Session::ParseEncryption`、靜態連結 Crypto++ 8.2.0（rijndael、gcm） |
| 遊戲 log 不含聊天 | `WizardClient.log`（19 MB）中 `chat` 出現 0 次 |
| 顯示路徑 | `GameClient::MSG_*Chat` → `DisplayChat` 事件（`ChatInfo`）→ `ChatBox::HandleDisplayChat` 套 `Chat.lang` 樣板 → `ChatBox::AddLine(this, line, tab)`（`0x140746da0`） |
| `AddLine` 行為 | 依分頁編號把行附加到 `this+0x7a8/0x7c8/0x7e8/0x808/0x828`（編號 0–4）或 `[this+0x848]` 陣列（5 以上）的 `std::wstring`；分頁編號等於 `this+0x7a0`（目前分頁）時才把整個緩衝塞給 chatLog |
| 伺服器廣播 | 另一個呼叫點對分頁 0、1、2、3 與每個自訂頻道各呼叫一次 `AddLine`，內容相同 |

### 實機探測（`probe_addline.py`、`probe_tabs.py`，見文末「原型檔案」）

| 觀察 | 結果 |
|---|---|
| 每則訊息呼叫次數 | 13 則訊息恰好 13 次，全部來自格式化函式 |
| 切分頁、傳送、登出再登入 | 零次呼叫，轉場**不重放**歷史 |
| 同字連續訊息 | 兩次一模一樣的掉寶是兩次獨立呼叫 |
| 一般聊天寫入的分頁 | 每則只寫一個分頁：0＝主要（說話、私訊、系統）、1＝隊伍、2＝房間 |
| 不在目前分頁的訊息 | hook 立刻拿到；現行做法要等使用者切過去才讀得到 |
| `ChatBox+0x7a0` | 目前分頁，與點擊同步 |
| `WizardChatBox` 視窗物件 | 就是 ChatBox 本身（`+0x7a0` 隨點擊變化） |
| 分頁名稱 | 各 `*FilterButton` 的 `+0x2e0` wstring，是遊戲畫面上的文字（`主要的`、`队伍`、`房间`），有未讀時帶 `(n)` 尾綴；隱藏中的分頁存的是未解析的 `<string;GUI2_...>` |
| 行內容 | 與 chatLog 每一行的 markup 相同（`<color;..><image;Art/Art_Chat_*>`、`<link;GID:..>`、`[你]`） |

另外：wizwalker 的 `read_wide_string` 以「長度 ≥ 4 字元」判斷字串是否在堆積上，正確應看 capacity ≥ 8，4–7 字元的短字串（例如 `队伍(2)`）會讀壞。本案讀 wstring 一律自己判斷 capacity。

## 決策摘要

| 決策 | 選擇 | 理由／出處 |
|---|---|---|
| 收訊資料源 | hook `ChatBox::AddLine` 入口 | 實測每則恰一次、轉場不重放；改讀分頁緩衝仍是差分（使用者確認走 hook） |
| 舊差分 | 整套刪除，不留退路 | 使用者決定 |
| 分頁呈現 | 標題列下方一排分頁，名稱與未讀數照抄遊戲按鈕，目前分頁高亮 | 使用者在示意圖中選 A |
| 分頁切換 | 自動跟著遊戲，不能在翻譯視窗上點 | 使用者決定 |
| 雙開 | 只顯示前景客戶端的分頁與訊息，跟著前景遊戲視窗切換 | 使用者決定 |
| 訊息前的 ①② 標記 | 移除 | 只顯示一個客戶端，標記沒有資訊量（我的判斷） |
| 未讀數 | 直接顯示遊戲按鈕文字，不自行計算 | 照抄遊戲；自己算會和遊戲不一致（我的判斷） |
| 翻譯上下文 | 每個（客戶端, 分頁）一份 | 隊伍與主要分頁是不同對話，混在一起會誤導翻譯（我的判斷） |
| 伺服器廣播 | 收斂成一則，翻譯一次，放進每個分頁 | 避免同一則翻 4 次以上（我的判斷，判準見元件設計） |
| 環形緩衝 | 256 槽 × 2 KB | 輪詢 0.4 秒內超過 256 行不切實際（我的判斷） |
| hook 機器碼的測試 | 以 unicorn 模擬執行，加為 dev 相依 | 機器碼寫錯會讓遊戲當掉，值得自動化驗證（我的判斷） |

## 元件設計

### 1. `src/reader/chat_hook.py`（新）：AddLine hook 與環形緩衝

- `AddLineHook(SimpleHook)`：沿用 wizwalker 的 `SimpleHook`。跳躍點是 `AddLine` 入口的 `mov [rsp+0x20], rbx`（5 bytes、無 RIP 相對定址，可直接搬到 stub 開頭）。stub 程式碼放 autobot 區（`get_hook_address` 要回傳足夠大的空間，預設 50 bytes 不夠）；環形緩衝用 `exports` 另外 `VirtualAllocEx`。
- pattern（`W.1.610.21` 實測在 exe 內唯一命中 `0x140746da0`）：
  ```
  \x48\x89\x5C\x24\x20\x55\x56\x57\x41\x54\x41\x55\x41\x56\x41\x57\x48\x8D\xAC\x24....
  \x48\x81\xEC....\x48\x8B\x05....\x48\x33\xC4\x48\x89\x85....\x44\x89\x44\x24.
  \x48\x89\x55.\x48\x8B\xF9\x48\x89\x55.\x41\x83\xF8\x05\x7C
  ```
- 緩衝配置：開頭 `0x40` bytes 是標頭（`u64` 累計呼叫數），之後 256 槽 × `0x800` bytes。每槽：`+0x00` 序號（最後寫入）、`+0x08` 回傳位址、`+0x10` `this`、`+0x18` 分頁 `r8d`、`+0x1c` 複製的字元數、`+0x20` 原始長度、`+0x30` 起 UTF-16 內容（最多 1000 字元，超過截斷）。
- stub 行為：執行原指令 → 保存用到的暫存器與旗標 → 累計數 +1 當序號 → 槽位先把序號寫 0 → 複製欄位與字串（依 capacity 判斷 SSO／堆積）→ 最後寫入序號 → 還原 → 跳回。`probe_addline.py` 是已實機跑過的版本，可直接沿用，**但它是 64 槽版本**：算槽位用的 `and eax, imm8`（`83 E0 ib`）會把立即值符號延伸，256 槽時 `0xFF` 變成 `and eax, 0xFFFFFFFF`、完全沒取模，寫入會越過緩衝讓遊戲當掉。改用 `and eax, imm32`（`25 id`）或 `movzx eax, al`。
- 讀取端（純函式，記憶體讀取以參數注入，可不靠遊戲測試）：依「上次讀到的序號」往後讀到累計數為止。
  - 槽位序號為 0：寫入中，下一輪再讀。
  - 槽位序號不等於預期：已被覆寫，記 log（含遺失筆數）並跳過。
  - 讀完內容再讀一次序號，與預期不同即視為讀取期間被覆寫（seqlock）。
  - 累計數與上次差距超過 256：記 log 遺失筆數，從最舊的仍有效槽位開始讀。
- `hook_state`：新 hook 的跳躍點原始 bytes 要一併存進修復狀態檔。修復時**先還原各 hook 的跳躍點，再還原 autobot**，否則還原 autobot 的瞬間，仍指向 autobot 的跳躍會跑進被改回的原始程式碼。現行 `_save_hook_state` 的 ops 順序是 autobot 在前，要調整。

### 2. `src/reader/mem_reader.py`（重寫）

- 連線：照舊開程序、修復殘留 hook、掛 root_window hook（輸入框偵測與貼齊還要用），再掛 AddLine hook，存修復狀態。pattern 掃不到（`PatternFailed`／`PatternMultipleResults`）照現行分類為 `GameVersionMismatch`，整條 reader 照現行流程 teardown 後重試；只有 AddLine 失敗、root_window 成功時，也不保留輸入框相關功能（我的判斷：改版失效本來就預期要等更新，不為降級模式多做一種狀態）。
- ChatBox 位址：透過 root window 找名為 `WizardChatBox` 的視窗，取其 base address。找不到就視為尚未進入世界，下一輪重找。
- `read_new()` 改為回傳本輪新增的事件清單，每個事件帶分頁編號與解析後的 `ChatLine`：
  - 只收 `this` 等於主 ChatBox 的事件。遇到不認得的 `this`，先重找一次 `WizardChatBox`（登出入、換角色後 ChatBox 可能是新建的物件，實機尚未確認）；重找後相符就更新主 ChatBox 位址並照收，仍不相符才判定為其他聊天框（例如組隊浮動視窗，實機尚未確認是否存在），記一次 log 後忽略。不能因為位址過期而默默丟掉之後所有訊息。
  - 每行原文交給 `markup.py` 的單行解析，解析不出來（除錯行、未知圖示）就丟棄，照舊記 log。
  - 伺服器廣播收斂：連續序號、原文相同、分頁編號從 0 起嚴格遞增的一組事件，收斂成一個「所有分頁」事件。真實聊天不可能在同一瞬間以相同文字依序出現在 0、1、2、3 號分頁。
  - `emit_system` 為 False 時照常推進序號，只是不回傳系統行。
- 分頁狀態：每輪讀 `ChatBox+0x7a0`，以及每個 `*FilterButton` 的 `+0x2e0` 文字。按鈕與分頁編號的對應：

  | 按鈕 | 分頁編號 | 依據 |
  |---|---|---|
  | MainFilterButton | 0 | 實測 |
  | GroupFilterButton | 1 | 實測 |
  | HouseFilterButton | 2 | 實測 |
  | FriendFilterButton | 3 | 靜態推論，待實機確認 |
  | TeamUpFilterButton | 4 | 靜態推論（`AddLine` 對 4 號更新 `GUI2_00002068` 的未讀數），待實機確認 |
  | Channel1–4FilterButton | 5–8 | 靜態推論，待實機確認 |

  文字以 `<string;` 開頭代表該分頁目前隱藏，不列入。按鈕節點快取，失效時重找（沿用 `_edit_container` 的作法）。
- 讀 wstring 一律用本專案自己的 helper（依 capacity 判斷 SSO），不用 wizwalker 的 `read_wide_string`／`maybe_text`。
- 斷線、teardown、`close()` 照現行流程；hook 的序號狀態屬於單一連線，重連後從新 hook 的 0 開始。

### 3. `src/reader/markup.py`

- 新增單行解析：輸入一行原始 markup，回傳 `ChatLine` 或 None。分類規則（系統訊息、伺服器公告、玩家頻道圖示白名單、`_VALID`、自己講的話）與現行 `lines_from_chatlog` 相同，`lines_from_chatlog` 改為逐行呼叫它，或直接刪除，看還有沒有人用。
- 刪除 `lines_from_nodes`、`_mirrors`、`node_sizes`（鏡射節點是差分時代的問題）。

### 4. `src/reader/message_log.py`

`messages.log` 改為逐筆記錄 hook 事件：序號、分頁、原始 markup，以及解析結果（玩家行／系統行／丟棄）。刪除快照與 `new_raw_lines` 差分。遺失、截斷、廣播收斂、非主 ChatBox 事件也各記一筆。內容沒變的輪不留痕跡的原則不變（沒有事件就不寫）。

### 5. `src/reader/loop.py`

- 每個事件帶著（客戶端 slot, 分頁）送進 overlay；廣播事件送「所有分頁」。
- 翻譯上下文改為每個（客戶端, 分頁）一份。`main.py` 的 `contexts` 改成能依分頁取用；發話時用「要打回去的那個客戶端」的**目前分頁**上下文。
- 每輪把該客戶端的分頁狀態（可見分頁的編號與按鈕文字、目前分頁）送給 overlay，只在有變化時送。
- 系統訊息的快取命中與 `system_pool` 流程不變。

### 6. 翻譯視窗（`src/ui/overlay.py`、`src/ui/message_list.py`）

- 標題列下方加一排分頁：名稱照抄遊戲按鈕文字（含未讀 `(n)`），目前分頁高亮，不可點擊。示意圖見 brainstorm 的選項 A。
- 訊息依（客戶端, 分頁）各存一份清單，`max_messages` 與淡出規則套在每份清單上。廣播訊息加到該客戶端每份清單。
- 前景客戶端：沿用輪詢前景視窗的方式，前景是某個遊戲視窗時切到該客戶端；前景不是任何遊戲（翻譯視窗本身、其他程式）時維持上一個。程式剛啟動、還沒有前景遊戲時，顯示第一個掛上的客戶端。
- 切換顯示的（客戶端, 分頁）時，整個清單換成該清單的內容；捲動位置回到最底。
- 移除 `set_multi_client` 與 `slot_marker` 的訊息標記。
- 尚未取得分頁狀態（剛連上、遊戲未進世界）時不顯示分頁列。
- 縮小成泡泡時的未讀數：只計**前景客戶端**在縮小期間收到的訊息，所有分頁都算（縮小時哪個分頁都看不到），伺服器廣播算一則；展開歸零。現行是所有客戶端都計，雙開時要改。被系統訊息開關擋掉的行本來就不會送進 overlay，不計。
- 分頁列文字是遊戲的文字，不經 i18n；不新增 UI 文案（若需要，照現行 i18n 流程加）。

### 7. 刪除清單

- `src/reader/diff.py`（整檔）
- `mem_reader.py` 的差分狀態機：`Track`、看過集合、暖機、`STALE_BASELINE_EMPTY_POLLS`、`INPUT_RELEASE_POLLS` 與輸入框關聯放行、`MAX_NEW_LINES_PER_POLL`／`BULK_APPEND_FILTER_MIN`／`DUPLICATE_BURST_SEEN_RATIO` 等門檻、節點數追蹤、鏡射節點
- `markup.py` 的 `lines_from_nodes`、`_mirrors`、`node_sizes`
- `message_log.py` 的快照差分
- 對應測試：`tests/test_chatlog_diff.py`、`tests/test_system_track.py`，以及 `test_chatlog_parse.py`、`test_message_log.py`、`test_reader_connect.py`、`test_reader_loop.py` 中針對差分的部分
- 已失效的 memory 結論與程式碼註解（改依賴時 grep 舊註解）

輸入框開關偵測（`input_open`、`input_box_screen_rect`、`_InputWatch`）**保留**，它只依賴 root_window hook。

## 錯誤處理與 log

前綴 `[reader]`，訊息用英文，帶 slot、PID：

- AddLine hook 掛上：跳躍點位址（換算回靜態位址）、緩衝位址與大小。
- pattern 失敗：沿用版本不相容橫幅與 log。
- 遺失：`lost N chat lines (ring overwritten)`，帶序號範圍。
- 截斷：原始長度。
- 非主 ChatBox 事件：第一次出現時記一次，帶 `this` 與分頁。
- 廣播收斂：收斂的分頁數。
- 分頁狀態變化：目前分頁與可見分頁清單，只在變化時記。
- 修復殘留 hook：沿用現行訊息，新增的 hook 也算在寫回筆數內。

## 測試

不需遊戲的單元測試：

- 環形緩衝讀取：以假記憶體模擬正常、寫入中（序號 0）、覆寫、超過 256 的遺失、seqlock 失敗。
- hook 機器碼：用 unicorn 模擬執行 stub，驗證 SSO 字串、堆積字串、截斷、序號繞回（序號要超過槽數，才抓得到取模錯誤），以及暫存器和旗標（含 DF）還原。`test_stub.py` 是可沿用的原型。unicorn 加到 dev 相依；動 `uv.lock` 要用 uv 0.12.x。
- 單行解析：沿用 `test_chatlog_parse.py` 的樣本。
- 廣播收斂、非主 ChatBox 過濾、`emit_system` 開關。
- 分頁按鈕文字解析：隱藏分頁（`<string;...>`）、帶未讀數、按鈕與編號對應。
- overlay：分頁列顯示、依（客戶端, 分頁）切換清單、前景切換、廣播進每個分頁、`max_messages` 套在單一清單。測試不可干擾執行中的遊戲（搶焦點的動作照舊抽成可 patch 的方法）。

## 實機驗收

開著遊戲（登入進世界內）跑 `uv run run.py`：

1. 主要、隊伍、房間各收發訊息，每則在對應分頁出現一次，翻譯一次。
2. 停在主要分頁時，有人在隊伍說話：隊伍分頁的翻譯立刻出現，切過去就看得到，分頁列顯示的未讀數與遊戲一致。
3. 切分頁、傳送、登出再登入、換角色：沒有任何舊訊息被重新翻譯或重新出現。
4. 連續兩則相同訊息（例如連打兩次 `lol`、兩次相同掉寶）都出現。
5. 確認分頁 3（好友）、4（Team Up）、5 以上（自訂頻道）的按鈕對應；有誤就修正對應表。
6. 開組隊浮動視窗時發話，確認沒有重複（是否存在第二個 ChatBox）；登出入、換角色後確認主 ChatBox 位址是否改變，且之後的訊息照常收到。
7. 雙開：切換前景遊戲視窗，翻譯視窗跟著換成該客戶端的分頁與訊息。
8. 遇到伺服器廣播時（不一定測得到），確認只翻一次、每個分頁都有。
9. 正常關閉後遊戲照常運作；強制結束後重開翻譯器，殘留 hook 被修復，不必重開遊戲。

## 規則與文件同步

- `CLAUDE.md` 與 `AGENTS.md`（兩份要逐字相同）的「wizwalker 掛入須知」：改寫成「收訊靠自訂 AddLine hook 讀聊天事件，另掛 root_window hook 供輸入框偵測；不啟 player/duel/quest 等其他 hook」。
- 三份 README：多開段落改成「翻譯視窗跟著前景的遊戲客戶端」，移除「訊息前面標示來自哪一個」；功能清單加上分頁同步。設定只用泛稱，不寫鍵名。
- memory：`addline-hook-spike`、`chat-tab-ui-has-no-readable-state` 隨實作結果更新。

## 留給實作者決定

- 模組切分的細節（例如分頁狀態讀取放在 `mem_reader.py` 或另開小模組），只要維持 reader／composer／translator 各司其職。
- 事件型別的命名與欄位。
- 分頁列的字型、間距、高亮樣式，以既有 palette 為準。
- 前景偵測要重用 overlay 現有輪詢，還是放在 main 的既有前景追蹤（`ChatInputTracker`、`target_hwnd` 一帶）。

## 原型檔案

探測原型與實機原始記錄存在本機 `.superpowers/sdd/chat-hook-prototypes/`（git 忽略、不進版控）：
`probe_addline.py`（AddLine hook 與環形緩衝，已實機跑過）、`test_stub.py`（unicorn 模擬驗證 stub）、
`probe_tabs.py`／`probe_chatbox.py`（分頁名稱與 ChatBox 欄位探測）、`probe_addline.jsonl`／`probe_tabs.jsonl`（實機記錄）。
