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
   - 每條：`Topic: detail (with a cheeky aside if it fits)` / `主題：細節（可加調皮註解）`
   - **中英文 bullet 條數與順序要一一對應**

## 取得素材

```bash
git log <prev-tag>..HEAD --oneline                          # 看所有 commit
git log <prev-tag>..HEAD --merges --pretty=format:"%h %s"   # 分支標題（高層脈絡）
git diff <prev-tag>..HEAD -- README.md                      # 確認對外講法（使用者看得到的行為）
git diff <prev-tag>..HEAD --stat | tail -30                 # 大致變動規模
ls docs/superpowers/specs                                   # 設計文件：功能的 why 通常寫在這
```

本專案沒有 PR 流程時就看 merge commit 的分支名（`feat/message-selection-copy` 之類）與 `docs/` 底下的 spec 標題，比逐條 commit 快。從中挑 2–3 個 hero，其餘進 bullet。**Dependabot / CI / 測試重構 / 機械 refactor 永遠進 bullet 區或省略。**

## 每個行為都要對過原始碼

版本說明是給使用者看的對外文件，**不准憑印象或憑「聽起來合理」寫**。散文比條列更容易夾帶沒查證的細節——順口就寫下去了，但方向、預設值、順序都可能剛好相反。落筆前把每個具體斷言查回原始碼：

| 這種句子 | 查哪裡 |
|---------|--------|
| 「原文在上、譯文在下」之類的版面描述 | `src/ui/overlay.py` 的 `add_message()`（字級、顏色、pack 順序） |
| 「預設會⋯⋯」「一啟動就⋯⋯」 | `src/config.py` 的預設值字典 |
| 「按下熱鍵之後會⋯⋯」 | `src/composer/paste.py`、`src/ui/input_box.py` |
| 「翻譯失敗時會⋯⋯」 | `src/translation/pool.py`、`src/ui/overlay.py` 的橫幅邏輯 |
| 「掛不進去時會顯示⋯⋯」 | `src/reader/hook_state.py`、README 的「遊戲改版後掛不進去」 |
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

## 詞彙對照（中英版本要用同一組講法）

| 繁中 | English |
|------|---------|
| 疊加視窗 | the overlay |
| 收訊翻譯 | incoming translation |
| 發話翻英 | outgoing translation（翻成英文送進遊戲） |
| 掛入遊戲 | hook into the game |
| 首次設定精靈 | the first-run wizard |
| 橫幅 | banner |
| 熱鍵 | hotkey |
| 系統訊息 | system messages |
| 目標語言 | target language |
| 多開（不寫「雙開」，功能不限兩個） | running several game accounts at once |
| 遊戲客戶端（不寫「客戶端」，會與翻譯服務的 client 混淆） | game client |
| 換你發言的時候（不寫「講回去」） | when it is your turn to talk |

第三方站名／來源用「原文 英譯」雙語並列，中英版本字串相同（例如「原神資訊站 Genshin Impact Info」），不要在英文版只留英譯。

## Gold Standard：v0.4.0（使用者已校稿）

> 這份是 2026-09-21 使用者逐句校過的定稿，覆蓋了原本依 commit 歷史草擬的 v0.1.0 暫定範本。
> 之後每次放版、使用者改完稿，回來用實際發布的版本覆蓋這一段（中英都要）。

```markdown
## What's New

You can now keep as many translation services as you like — two OpenAI keys, a Claude, an OpenAI-compatible server of your own — each with its own name, and switch the default between them whenever you want. If you would rather not run everything through one model, incoming chat, your outgoing lines and the on-screen area translation can each point at a different service; leave them alone and they simply follow the default 🔀

The other big one: running several game accounts at once finally works the way you would expect. Every open game client gets its own hook, every message in the overlay is tagged with the game client it came from (①, ②, …) the moment a second one shows up, and the tags stay put even after one of them closes. When it is your turn to talk, same rule — the translation input box, the typed-in translation and Ctrl+V all go to the game client you are actually looking at, not the one that happened to start first 🎮

On top of that, we also did:
- First-run wizard: step two now creates your first translation service instead of just filling in a key
- Adding a service starts from provider cards with each one's logo, so you pick where it goes before you type anything
- Old settings migrate on their own — every key you had is kept, and a service this version does not recognise is set aside rather than thrown away
- The "game path" setting is gone from the advanced tab; the tool never needed it, and with several game clients it could not have been right anyway

## 此版本重點

翻譯服務現在想建幾組就建幾組 —— 兩把 OpenAI 金鑰、一組 Claude、一台自己架的 OpenAI 相容伺服器 —— 各取一個名字，預設要用哪一組隨時切。不想全部走同一個模型的話，聊天收訊、你自己的發話、畫面框選翻譯可以各指定一組；不動它們就跟著預設走 🔀

另一件大事：多開好幾個帳號終於是你預期的樣子。每個開著的遊戲客戶端都各自掛入，第二個遊戲客戶端一出現，疊加視窗裡每則訊息前面就標出它來自哪一個（①、②⋯⋯），其中一個關掉之後標記也不會消失。換你發言的時候也是同一套 —— 翻譯輸入框、打進去的譯文、Ctrl+V 貼上，全都對準你當下正在看的那個遊戲客戶端，而不是先啟動的那個 🎮

除此之外，我們還做了：
- 首次設定精靈：第二步改成直接建立你的第一組翻譯服務，不再只是填一把金鑰
- 新增服務先從帶各家標誌的服務商卡片選起，先決定要去哪裡再開始填
- 舊設定會自己遷移 —— 你原本的金鑰一把都不會掉，這一版認不得的服務會先收在一旁而不是刪掉
- 進階頁的「遊戲路徑」設定拿掉了；工具從來不需要它，多開時它也不可能填得對
```

**注意這份範例做對的事**：

- 中英文 hero 段落數相同（兩段）、bullet 條數相同（四條）、emoji 位置對齊（🔀 / 🎮）
- 開場直述：第一句就講這版做到什麼（想建幾組服務就建幾組），沒有反問句、沒有「本次更新包含」
- 把同一主題的子功能壓在同一段：多組服務、預設服務、三個用途各指定一組合成一段；多開的掛入、標記、發話對準前景合成另一段
- 每個行為都對得上程式碼／文件：三用途各指定（README 功能清單、multi-service spec）、標記在第二個遊戲客戶端出現才顯示且不消失（multi-client spec）、遊戲路徑設定移除（commit）
- 用語照使用者校稿：「多開」不寫「雙開」、「遊戲客戶端」不寫「客戶端」、發話段用「換你發言的時候」
- 沒有 `## 收訊 / ## 發話 / ## 底層` 之類分節
- 沒有補防毒警告、封號風險、回報連結（footer 已有）
- 兩個 H2 直接相鄰，**沒有** `---` 分隔線

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
| 把 Dependabot / CI / 測試重構拉到 hero | 永遠進 bullet 區或直接省略 |
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
- [ ] 主打功能是不是散文？沒被我寫成 bullet？
- [ ] 有沒有第二人稱直接對讀者？
- [ ] 是不是不小心又灑了「✨ ⚡ 🎨」三段式分節？
- [ ] 有沒有拿 `_Track`／`reader_loop` 之類內部名詞當賣點？
- [ ] Dependabot / CI / 機械 refactor 有沒有乖乖待在 bullet 區？
- [ ] 兩邊讀起來都是自然的母語文字，不是互相直譯？
