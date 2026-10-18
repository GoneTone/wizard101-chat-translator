---
name: writing-release-notes
description: Use when drafting GitHub release notes or changelog content for this project (Wizard101 Chat Translator). Triggers when the user asks to write release content for a new version, summarize a tag-to-tag diff (e.g. "v0.1.0...v0.2.0"), or produce "更新內容 / 版本說明 / release note". Produces bilingual (en on top, zh-Hant on bottom) Discord-style prose for headline features with a short bullet list for minor items — NOT a corporate-style changelog with one section header per category.
---

# Writing Release Notes (this project)

## 風格摘要

**雙語輸出**：英文在上、繁體中文在下（與 `.github/release-footer.md` 的語言順序一致），內容對齊（同樣的 hero 段落數、同樣的 bullet 數、同樣的 emoji 位置）。兩個 H2 直接相鄰，**不加 `---` 水平線**。

每一語言內部：單一 `## What's New` / `## 此版本重點` 標題開場，前段用 1–3 段**散文**敘述主打功能，最後接一段「On top of that, we also did:」/「除此之外，我們還做了：」+ 條列次要項。整體語氣像 Discord 公告，不是 SaaS changelog。

## 為什麼這樣寫

- **重點功能用散文**：可以一邊介紹一邊埋槽點、把使用情境帶出來，讀者比較記得住
- **次要項用 bullet**：本來就不需要鋪陳，硬寫散文反而拖
- **不切 H2/H3 per feature**：分節會讓 release note 看起來像產品手冊，喪失「聊天感」

## 這份草稿會被貼到哪裡（先讀這段）

`release-windows.yml` 建立 draft release 時，release body 已經自動排成：

```
[這裡是你要寫的版本說明]

---

## What's Changed （GitHub 自動產生）

---

[.github/release-footer.md 的固定文案]
```

所以這個技能產出的是**最上方那段手動版本說明**，維護者編輯 draft 時貼進去。因此：

- **不要**自己補防毒誤判警告、封號風險聲明、回報問題連結——footer 已經有了，重複只是雜訊
- **不要**自己補「完整 diff」連結或 commit 清單——中間那段 What's Changed 是自動產生的
- **不要**在自己這段的結尾加 `---`——workflow 已經接上了

## 結構（必照）

整份輸出順序（兩個 H2 直接相鄰，**不加 `---`**）：

```
## What's New
（英文 hero 散文）
（英文 bullet 區）

## 此版本重點
（繁中 hero 散文）
（繁中 bullet 區）
```

每一語言內部：

1. **ONE H2**：英文用 `## What's New`、繁中用 `## 此版本重點`（不要再切子標題；散文段落自帶轉場）
2. **Hero 散文**（1–3 段，每段一個主題群）
   - **開場直述**：第一句就講這個版本做到什麼、怎麼做到的，不繞圈子。**不要反問句開場**（「有沒有過這種時候？」「還記得以前⋯⋯嗎？」這類鋪陳讀起來刻意），也不要「本次更新包含⋯⋯」這種制式開頭——直述不等於流水帳，講的是行為與差別，不是「有哪些更新」
   - 一段內可以串多個相關功能（例：跨訊息選取 + Ctrl+C 複製 + 自訂彈出選單 → 同段「複製聊天內容」）
   - 第二人稱直接對讀者：英文 "your / press / no more"；繁中「你的 / 按下去 / 終於不用」
   - emoji 偶爾用，每段 1–2 個，挑自然停頓處（句末或破折號前後）；**中英文 emoji 位置要對齊**
3. **過場句 + bullet 區**
   - 英文過場："On top of that, we also did:"
   - 繁中過場：「除此之外，我們還做了：」
   - 每條以**改動類型動詞**開頭（見「寫明改動類型」）：`Optimized …: detail` / `最佳化…：細節`、`Fixed … ` / `修正…的問題：`、`Removed …` / `移除…`；維護類用 `Maintenance:` / `維護：`
   - **中英文 bullet 條數與順序要一一對應**

## 取得素材

```bash
git log <prev-tag>..HEAD --oneline                          # 看所有 commit
git log <prev-tag>..HEAD --merges --pretty=format:"%h %s"   # 分支標題（高層脈絡）
git diff <prev-tag>..HEAD -- README.md                      # 確認對外講法（使用者看得到的行為）
git diff <prev-tag>..HEAD --stat | tail -30                 # 大致變動規模
ls docs/superpowers/specs                                   # 設計文件：功能的 why 通常寫在這
```

本專案沒有 PR 流程時就看 merge commit 的分支名（`feat/message-selection-copy` 之類）與 `docs/` 底下的 spec 標題，比逐條 commit 快。從中挑 2–3 個 hero，其餘進 bullet。**Dependabot / CI / 測試修正 / 內部整合不進 hero，彙整成 bullet 區最後的「Maintenance:」／「維護：」條目（使用者要求要提到）。**

## 每個行為都要對過原始碼

版本說明是給使用者看的對外文件，**不准憑印象或憑「聽起來合理」寫**。散文比條列更容易夾帶沒查證的細節——順口就寫下去了，但方向、預設值、順序都可能剛好相反。落筆前把每個具體斷言查回原始碼：

| 這種句子 | 查哪裡 |
|---------|--------|
| 「原文在上、譯文在下」之類的版面描述 | `src/ui/overlay/message_list.rs`、`src/ui/overlay/bubble.rs`（字級、顏色、排列順序） |
| 「預設會⋯⋯」「一啟動就⋯⋯」 | `src/config/mod.rs` 的 `default_config()` |
| 「按下熱鍵之後會⋯⋯」 | `src/composer/paste.rs`、`src/ui/input_box.rs` |
| 「翻譯失敗時會⋯⋯」 | `src/translation/pool.rs`、`src/ui/overlay/banners.rs` 的橫幅邏輯 |
| 「掛不進去時會顯示⋯⋯」 | `src/reader/hook_state.rs`、README 的「遊戲改版後掛不進去」 |
| 使用者流程（精靈幾步、點哪裡） | README 的「一般使用者」段 |

拿不準又查不到就**把那句拿掉**，不要留一個「大概是這樣」的描述——使用者會照著它去操作。

## 講給誰聽

讀者是**遊戲玩家**，不是開發者。挑素材與用字都以「他在遊戲裡會感覺到什麼」為準：

| ❌ 開發者視角 | ✅ 玩家視角 |
|------------|-----------|
| 「重構 reader，把 diff state 收進 `_Track`」 | 省略（他感覺不到） |
| 「新增 `translate_system_message()` 路徑與 LRU 快取」 | 「系統訊息現在也會翻，而且重複的句子不再重打一次 API」 |
| 「overlay 的 selection geometry 從 Tk layout 還原」 | 「拖曳就能選取聊天內容，Ctrl+C 直接複製」 |
| 「hook-ready 等待加上上限並回報 version mismatch」 | 「遊戲改版掛不進去時，疊加視窗會直接告訴你，而不是靜靜卡著」 |

## 寫明改動類型（別讓玩家誤會以前的行為）

只描述「現在怎樣」，玩家會看不出改了什麼，甚至誤以為以前本來就這樣、或以前不是這樣。每一項都要讓人一眼看出是哪一種改動：

| 類型 | 繁中 | English | 寫法 |
|------|------|---------|------|
| 新功能 | 新增 | Added／We added | 講新增了什麼、玩家怎麼用 |
| 效能或體驗改善 | 最佳化 | Optimized | 講以前慢／卡在哪，現在差多少 |
| 修 bug | 修正…的問題 | Fixed | **先寫以前的錯誤現象**（標題列是灰的、會蓋住聊天框），再寫現在 |
| 拿掉設定或功能 | 移除 | Removed | 講為什麼不再需要、舊設定值怎麼處理 |

- **條列一律以類型動詞開頭**；hero 散文也要讓讀者分得出是新功能還是改進（例：「疊加視窗**新增**和遊戲一樣的聊天室分頁」）。
- **改了做法的功能，要點明舊做法與差別**：「收訊翻譯現在直接從遊戲拿到聊天內容」會讓人以為以前不是從遊戲拿；應寫「以前是比對聊天框前後的內容來找出新訊息，偶爾會重複顯示或漏掉；現在改成 hook 掛入遊戲把訊息加進聊天框的那一步」。
- **原理只在讀者需要時用一句話帶過，不展開**：說到「怎麼做到的」時，一句講清楚機制就好（hook 掛入哪一步、結果是什麼），不寫序號、緩衝區、記憶體這類細節——講太細玩家讀不下去。句子要順，避免「遊戲每把一行加進聊天框」這種拗口說法。
- **修正項不能寫成新行為**：「新增服務的視窗開出來就能直接操作」聽起來像新功能；應寫「修正…開啟後沒有被選取的問題（標題列是灰的、按 Esc 沒反應）」。
- **一個修正套用到多處就都列出來**（例：設定視窗、新增服務的視窗、首次設定精靈），不要只寫被回報的那一處。

## 詞彙對照（中英版本要用同一組講法）

| 繁中 | English |
|------|---------|
| 疊加視窗 | the overlay |
| 收訊翻譯 | incoming translation |
| 發話翻英 | outgoing translation（翻成英文送進遊戲） |
| hook 掛入遊戲（中文要帶出 hook，只寫「掛入」讀者不知道是用什麼掛入） | hook into the game |
| 聊天室分頁（不寫「分頁」或「聊天分頁」） | chat tab |
| 首次設定精靈 | the first-run wizard |
| 橫幅 | banner |
| 熱鍵 | hotkey |
| 系統訊息 | system messages |
| 目標語言 | target language |
| 多開（不寫「雙開」，功能不限兩個） | running several game accounts at once |
| 遊戲客戶端（不寫「客戶端」，會與翻譯服務的 client 混淆） | game client |
| 換你發言的時候（不寫「講回去」） | when it is your turn to talk |

第三方站名／來源用「原文 英譯」雙語並列，中英版本字串相同（例如「原神資訊站 Genshin Impact Info」），不要在英文版只留英譯。

## Gold Standard：v0.4.2 之後的下一版（使用者已校稿）

> 這份是 2026-10-16 使用者校過的定稿（v0.4.2 之後的下一版），取代 v0.4.2 的範本。
> 之後每次放版、使用者改完稿，回來用實際發布的版本覆蓋這一段（中英都要）。

```markdown
## What's New

Incoming translation now reads chat in a new way. It used to compare the chat box before and after each check to find new lines, which now and then showed a message twice or missed one. Now it hooks into the step where the game adds messages to the chat box, so the translator gets a copy of every message as the game handles it. Nothing is repeated or lost, and chat in tabs you are not looking at is picked up right away instead of waiting until you switch over. Chat is also checked every 0.1 seconds instead of the old default of 0.4, so translations start sooner 📨 If a game update ever breaks this, the overlay shows the version banner, and you will need a new release of the translator.

We added chat tabs to the overlay that match the game's, with the same names and unread counts, and the overlay switches tabs whenever you do in the game. Each tab keeps its own messages, so flipping back and forth is instant. When you run several game accounts at once, the overlay shows the game client you are playing on 🗂️

On top of that, we also did:
- Optimized resizing the overlay: dragging its edge no longer freezes the window, even with a long chat history; older messages are redrawn in the background
- Fixed the settings window, the windows that open while you add a translation service, and the first-run wizard opening without being selected (grey title bar, Esc did nothing); also fixed clicks on the empty part of a provider card doing nothing, which sometimes took several clicks to pick one
- Fixed the translation input box covering the game's chat box: when it opens above the chat box and its hint or error message gets longer, it now grows upward, and after you drag it somewhere else it grows upward from there
- Removed the setting for how often chat is checked: with the new way of reading chat, checking for new lines costs almost nothing, so it is fixed at every 0.1 seconds and needs no tuning; the old value in your config is cleared automatically on startup
- Maintenance: the Anthropic SDK is updated to 1.11.0, the developer tools are updated, and the code and tests were reorganized

## 此版本重點

收訊翻譯改用新的方式讀取聊天。以前是比對聊天框前後的內容來找出新訊息，偶爾會重複顯示或漏掉；現在改成 hook 掛入遊戲把訊息加進聊天框的那一步，遊戲每處理一則訊息，就同步交一份給翻譯助手。訊息不會再重複或遺漏，沒在看的聊天室分頁也會立刻收到，不用等你切過去。檢查頻率也從預設的 0.4 秒提高到每 0.1 秒，譯文更早開始翻 📨 萬一遊戲改版讓它失效，疊加視窗會顯示版本不相容的橫幅，需要等翻譯助手出新版。

疊加視窗新增和遊戲一樣的聊天室分頁，名稱與未讀數都照遊戲顯示；你在遊戲裡切換聊天室分頁，它也跟著切。每個聊天室分頁各自保留訊息，來回切換一瞬間就好。多開遊戲的時候，疊加視窗會顯示你正在操作的那個遊戲客戶端 🗂️

除此之外，我們還做了：
- 最佳化縮放疊加視窗的效能：拖邊框改大小不會再卡住，聊天紀錄再長也一樣；較舊的訊息會在背景陸續補畫
- 修正設定視窗、新增翻譯服務時跳出的視窗，以及首次設定精靈開啟後沒有被選取的問題（標題列是灰的、按 Esc 沒反應）；也修正點服務商卡片的空白處沒反應、有時要點好幾下才選到的問題
- 修正翻譯輸入框蓋住遊戲聊天框的問題：開在聊天框上方時，提示或錯誤訊息變長會改成往上長；拖到別處之後再變長，也會從你放的位置往上長
- 移除收訊頻率的設定：改用新的讀取方式後，檢查有沒有新訊息幾乎不耗資源，因此固定每 0.1 秒檢查一次，不需要再調整；舊設定檔裡的這個值會在啟動時自動清除
- 維護：Anthropic SDK 更新到 1.11.0、更新開發工具，並重新整理程式碼與測試
```

**注意這份範例做對的事**：

- 中英文 hero 段落數相同（兩段）、bullet 條數相同（五條）、emoji 位置對齊（📨 / 🗂️）
- **每項都寫明改動類型**：hero 用「改用新的方式」「新增」，bullet 以「最佳化／修正…的問題／移除／維護」開頭；修正項先寫以前的錯誤現象（標題列是灰的、按 Esc 沒反應；點空白處沒反應）
- **改了做法要點明舊做法與差別，原理一句帶過**：舊做法只寫「比對聊天框前後的內容、偶爾重複或漏掉」，新做法只寫「hook 掛入遊戲把訊息加進聊天框的那一步」，不展開序號、緩衝區、切分頁時內容被換掉這類細節
- **新做法帶來的限制要講**：遊戲改版可能讓它失效、會顯示橫幅、需要等新版
- **一個修正套用到多處就全列**：設定視窗、新增服務的視窗、首次設定精靈
- **移除設定講為什麼不再需要、舊值怎麼處理**；只寫「收訊頻率的設定」這種泛稱，不寫選項名稱或在哪個分頁
- 數字都對過程式碼：0.4 秒是 v0.4.2 的預設值、0.1 秒是現在的固定值；縮放以前卡幾秒取決於訊息數，所以不寫秒數
- 玩家感覺不到的不寫：只刪掉兩個字串的翻譯更新、README 與技能檔的修改
- 固定按鍵（Esc）可以寫；可自訂的熱鍵組合不寫

**沿用 v0.4.2 範本的原則**：用玩家遇過的實例講問題（「想問能不能補血，送出去變成請隊友幫你補」）；翻譯品質類的改動講「調整提示詞」與成果，不細講提示詞規則或結構化輸出這類實作；只影響部分服務的改動要寫明範圍（只影響 Claude、其他服務不受影響）。

## 重點群組範例

當有多個小功能時，看怎麼把它們合進同一段：

| 主題群 | 包含的功能 |
|--------|-----------|
| 看得懂別人說什麼 | 收訊翻譯、保留原文、系統訊息翻譯 |
| 講得回去 | 熱鍵輸入、翻英、逐字鍵入、不自動送出 |
| 聊天內容帶得走 | 跨訊息選取、Ctrl+C 複製、自訂彈出選單 |
| 出事的時候不會靜靜卡著 | 版本不相容橫幅、權限不足橫幅、更新提醒橫幅 |
| 設定與上手 | 首次設定精靈、設定即時套用、介面語言切換 |

## Common Mistakes

| ❌ 不要 | ✅ 應該 |
|--------|--------|
| 只輸出單一語言 | 永遠中英雙語，**英文在上、繁中在下**，兩個 H2 直接相鄰（不加 `---`） |
| 在中英之間加 `---` 水平線 | 兩個 H2 直接相鄰即可 |
| 中英文段落數 / bullet 條數不一致 | 一一對應，emoji 位置也對齊 |
| 補上防毒誤判／封號風險／回報問題連結 | 那些在 `.github/release-footer.md`，workflow 會自動接 |
| 補「完整 diff」連結或 commit 清單 | 中間那段 What's Changed 是 GitHub 自動產生的 |
| 憑印象描述行為（版面上下、預設開關、操作順序） | 每個具體斷言都回原始碼／README 查過，查不到就刪掉那句 |
| 整篇都是 bullet | 主打用散文，次要才 bullet |
| `### 訊息選取複製` / `## 主打功能` 分節 | 散文段落自帶轉場，不切標題 |
| 反問句開場（「有沒有過這種時候？」「還記得以前⋯⋯嗎？」） | 第一句直接講這個版本做到什麼、怎麼做到的 |
| 「本次更新包含⋯」「主要新增功能：」制式開場 | 直述也要有內容：講行為與差別，不是報告「有哪些更新」 |
| 列每一條 commit | 只挑玩家會在意的（見「講給誰聽」） |
| 用 `_Track` / `chatLog` / `reader_loop` 這種內部名詞講功能 | 用玩家看得到的行為講；identifier 只在指設定檔／紀錄檔路徑時出現 |
| Emoji 灑滿每行 | 每段 1–2 個，挑自然停頓處 |
| ✨ 主打 / ⚡ 底層 / 🎨 體驗 三段式 | 一段散文走完所有重點 |
| 第三人稱客套（「使用者可以⋯」） | 第二人稱直接對讀者（「你⋯」） |
| 只寫現在的行為，看不出是新增、最佳化還是修正 | 條列以「新增／最佳化／修正…的問題／移除」開頭（見「寫明改動類型」） |
| 「現在直接從遊戲拿到⋯⋯」這種會讓人以為以前不是的說法 | 點明舊做法與差別：「以前是⋯⋯，偶爾會⋯⋯；現在⋯⋯」 |
| 把修正寫成新行為（「視窗開出來就能直接操作」） | 「修正⋯⋯的問題」，並先寫以前的錯誤現象 |
| 把 Dependabot / CI / 測試重構拉到 hero，或乾脆省略不提 | 放 bullet 區最後，以「Maintenance:」／「維護：」開頭彙整成一兩條 |
| 細講譯文格式規則（「專有名詞會附原文」「介面標籤不加括號」之類的提示詞內容） | 翻譯品質類改動講「最佳化了給 AI 的提示詞」與示範句怎麼來，再講玩家感受得到的成果 |
| 中文版逐字直譯英文稿（或反過來） | 兩邊各自寫成自然的母語文字，只對齊結構與節奏 |
| 解釋「否則會怎樣」的反面情境（「翻到一半的語言不會變成半英文介面」） | 只講這個版本做到什麼；反面情境是設計理由，留在 commit／spec 裡 |
| 寫維護者才在意的好處（「不必再維護一張對照表」「新增語言不用改程式碼」） | 只寫玩家在操作時感覺得到的差別 |
| 把授權檔、README 連結、文件更新當成一條版本重點 | 不影響軟體行為的就省略，連 bullet 都不必進 |

## 語言與標點

**英文區**

- 半形標點 . , ! ? ; : ( ) " "
- 散文內可用 em dash ` — ` 作節奏停頓（用空格包圍）
- 自然口語英文，不要把中文逐字直譯
- 雙引號用直引號 `"`，不要用花引號 `“` `”`（GitHub markdown 顯示一致性）

**繁中區**

- 繁體中文 (台灣)
- 全形標點 ，。！？；：（）「」『』、
- 散文內可用破折號 `——` 作節奏停頓
- 程式碼 / identifier 維持半形

## 草稿流程

1. `git log <prev-tag>..HEAD --merges` 抓分支標題、`git log --oneline` 看細部、`ls docs/superpowers/specs` 補 why
2. 挑 2–3 個 hero（**合併相關的**），其餘進 bullet；開發者視角的項目直接刷掉
3. **查證**：把要寫進去的每個行為斷言回原始碼／README 對過（見「每個行為都要對過原始碼」）
4. **先寫英文版**：第一句直述這版做到什麼，再把其餘用散文寫清楚，最後雕 emoji 和段落節奏
5. **再寫繁中版**：對齊段落結構、bullet 條數、emoji 位置；用自然中文重寫，不逐字直譯
6. 兩個 H2 直接相鄰，**不要加 `---`**，也不要補 footer 已有的內容
7. 給使用者過稿
8. 若使用者改稿後風格有調整 → **回來更新這個技能的 Gold Standard 段落**（中英都要更新），下次才不會走回頭路

## 紅旗自檢

寫完前自問：

- [ ] 中英文都寫了？**英文在上、繁中在下**？兩個 H2 直接相鄰（**沒有** `---`）？
- [ ] 中英文段落數、bullet 條數一致？emoji 位置對齊？
- [ ] 有沒有不小心重寫了 footer 已有的防毒／封號／回報問題內容？
- [ ] 各語言內是不是只有一個 H2（`## What's New` / `## 此版本重點`）？沒有別的標題？
- [ ] 每個講到行為的句子都查過原始碼／README 了？沒有一句是「應該是這樣」寫出來的？
- [ ] 開場是不是直述？沒有反問句鋪陳、也沒有「本次更新包含⋯⋯」制式開頭？
- [ ] 每一項都看得出是新增、最佳化、修正還是移除？條列都以類型動詞開頭？
- [ ] 有沒有哪句會讓人誤以為以前的行為不一樣？改了做法的地方有點明舊做法嗎？
- [ ] 修正項有先寫以前的錯誤現象嗎？同一個修正套用到的地方都列了嗎？
- [ ] 講原理的地方是不是一句就帶過？有沒有展開序號、緩衝區這類玩家讀不下去的細節？
- [ ] 主打功能是不是散文？沒被我寫成 bullet？
- [ ] 有沒有第二人稱直接對讀者？
- [ ] 是不是不小心又灑了「✨ ⚡ 🎨」三段式分節？
- [ ] 有沒有拿 `_Track`／`reader_loop` 之類內部名詞當賣點？
- [ ] Dependabot / CI / 測試修正有沒有彙整成 bullet 區最後的「維護：」條目（不進 hero、也不省略）？
- [ ] 兩邊讀起來都是自然的母語文字，不是互相直譯？
