# Wizard101 聊天記憶體結構 — 逆向調查筆記

目標:找到「有序、帶身分的聊天訊息容器」的穩定存取路徑,取代目前「純讀渲染文字 + 內容差分」的做法(該做法在重複訊息上本質不可靠)。

執行檔:`Bin/WizardGraphicalClient.exe`(56 MB,x64,未加殼,MSVC)。
ImageBase(靜態)= `0x140000000`。以下位址標 **RVA**(執行時 = 模組基址 + RVA;模組基址因 ASLR 每次不同)。

## 已確認的發現(靜態反組譯)

### 1. 聊天走 KingsIsle PropertyClass 屬性系統
- RTTI 類名確認存在:`ChatHistory`、`ChatInfo`、`ChatSpamHelper`、`BasicChatPlayer`、`GuildChatInfo`、`ChatDiagnostics`、`HelpChatElement`、`WindowBubble`(頭頂聊天氣泡)。
- 這些聊天類別**都沒有 vtable**(非多型),所以無法用 RTTI COL/vtable 當靜態錨點。
- 聊天欄位以**字串名**註冊到屬性系統(通用 get/set,非固定偏移)。

### 2. ChatHistory(訊息容器)相關位址
- `"ChatHistory"` 字串:RVA `0x2c3a9d8`(明文,屬性名)。
- **ChatHistory 屬性名 getter**:函式 RVA `0x14e24f0` —— 建立/回傳名字物件,存到全域:
- **全域名字物件**:RVA `0x3376f90`(存 magic-static 保護於 RVA `0x3376f88`)。
- **ChatHistory 屬性描述子 vtable**:RVA `0x2c3a9f8`(緊接 "ChatHistory" 字串)。方法群位於 RVA `0x14e2xxx–0x14e4xxx`(含 name-compare dispatch `0x14e4970`)。
- 唯一呼叫 name getter 的位置:RVA `0x14e498a`(屬性名比對,非欄位偏移註冊)。

### 3. 聊天行渲染路徑
- 格式字串(UTF-16):`<image;Art/Art_Chat_Say.dds;%d;%d;%X>`,RVA `0x2a84cf0`。
- 唯一引用它的函式:RVA `0x744fb9` 附近 —— 聊天行渲染器。從渲染上下文物件(暫存於暫存器 r13)讀 `+0x418`、`+0x650`、`+0x8bc` 等欄位,sprintf(RVA `0x3dc5e0`)組出 image 前綴。
- **這是繪製管線的暫時物件**,每次重繪建立,不是持久訊息儲存 —— 對應我們在記憶體看到的「多份渲染副本 + 雙緩衝乒乓 + 尾端活躍重排」。

## 為什麼純靜態到此為止

從「ChatHistory 屬性名」追到「執行時那個裝著有序訊息的 vector」,中間隔著整個 PropertyClass 物件圖:
1. 哪個 PropertyClass **擁有** ChatHistory 欄位、偏移多少;
2. 該擁有者類別的**執行時單例**在哪(從全域/模組可達的指標鏈);
3. 容器內元素(ChatInfo?)的 sender / text / timestamp 欄位偏移。

其中(2)的執行時指標與 ASLR 後位址、以及(1)(3)的實際偏移,**都需要動態除錯器驗證**(下斷點、觀察暫存器/記憶體),這在無 GUI 除錯環境無法完成。此外每次遊戲改版偏移可能失效要重做。

## 下一步(需在本機用 Cheat Engine / x64dbg)

以上靜態位址可當動態逆向的起點:
1. 附加 x64dbg 到 `WizardGraphicalClient.exe`,在 name getter(模組基址 + `0x14e498a`)或渲染器(+`0x744fb9`)下斷點,觀察 caller 傳入的物件指標。
2. 對「渲染上下文」或「屬性擁有者」做 Cheat Engine pointer scan(從已知聊天字串位址往回掃指標),找跨重啟穩定的 `模組基址 + 偏移 → …` 路徑。
3. 找到容器後,確認元素佈局(sender/text 偏移),即可讀有序訊息,達成重複、即時、與遊戲完全一致。

## 動態逆向(純讀 pointer scan,不需除錯器) — 2026-08-21

用純讀 `ReadProcessMemory` + numpy 寫了多層反向 pointer scanner(`scripts/ptrscan.py`),
從一則獨特訊息字串反向追指標鏈到模組固定區。**方法成功,秒級找到穩定入口:**

- 打獨特訊息 `QZX99WIZ`,depth 1 就找到 4 條「.data 全域 → 訊息」路徑:
  - `模組基址+0x333cfa8 / +0x333db38 / +0x33631d8` → **同一個 .data 靜態物件**(RVA `0x33b0fc0`)+0x1c8
  - `模組基址+0x3363198` → 一則渲染訊息 wstring(+0x2)
- RVA `0x33b0fc0` 的靜態物件經 RTTI/字串確認是 **ControlRichEdit**(遊戲聊天顯示控件單例)。
  其 +0x1c8 是顯示文字,但**是排版用的碎片 run**(以 `\x00` 分隔、部分重疊,如 "building"/"lding"/"ilding"),不是乾淨的一則一項。
- ControlRichEdit 沒有乾淨的 `vector<std::wstring>` 行陣列;訊息 wstring(如 `<center>QZX99WIZ`、`<color;…>[sender] text</color>`)散在 heap。

**決定性結論:** 遊戲**顯示**的聊天在記憶體就是**渲染標記文字**(`<color>`/`<center>`/`<link>`/`[sender]` 全混在 wstring 裡),
**沒有**分離 sender/text、帶順序/時間戳的乾淨結構化容器。ChatInfo/ChatHistory 類別存在但不以乾淨欄位形式持有顯示訊息(可能僅用於網路/序列化)。
這從根本解釋了為何「重複的相同訊息無法用內容區分」——記憶體裡本來就沒存訊息身分。

**pointer scan 的價值**:證明能純讀找到 .data 穩定入口(ControlRichEdit @RVA `0x33b0fc0`),
可用來取代「全記憶體掃描找聊天」→ 解決定位卡住/乒乓/殭屍/掃描慢;但內容仍是渲染碎片,訊息身分問題是記憶體本質限制,非演算法可解。

## 網路層調查 — 2026-08-21

找到聊天 handler 名字串:`HandleChatFail`、`HandleChatCommandReply`、`HandleChatCommandTell`,
及聊天訊息欄位 `senderId`、`ChannelID`。但這些字串**沒有任何 lea 或指標引用** ——
它們透過 KingsIsle 的訊息 dispatch 框架**間接**呼叫(和屬性系統一樣,靠 hash/索引 dispatch,
非直接引用),追下去又是框架黑洞。

且動態掃描揭示關鍵事實:sender 資訊以 `<link;GID:191965934…,發送者,2>` 形式**內嵌在渲染標記文字裡**
(GID = 發送者的全域唯一 ID),連同 text、`<color>`、`<center>` 都在同一個 wstring。

## 三管齊下的最終結論

exe 靜態反組譯 + 純讀 pointer scan + 網路 handler 追蹤,共同證實:
**Wizard101 的聊天訊息在持久記憶體裡只以「渲染標記文字」存在**(含 `<link;GID;發送者>` + text + 樣式標記),
**沒有**帶訊息身分 / 順序 / 時間戳的結構化容器可供純讀取得。這是記憶體佈局的本質,
不是演算法或工具能繞過的 —— 「快速連續發送的完全相同訊息無法區分」是資訊層面的硬限制。

可行的最佳成果是 pointer scan 找到的**固定入口**(ControlRichEdit @RVA `0x33b0fc0`),
能提升定位穩定性(不必全記憶體掃描),但內容仍是渲染文字。

## ControlRichEdit 富文字模型調查 — 2026-08-21(選項 A)

嘗試從固定入口 ControlRichEdit(RVA `0x33b0fc0`)解析出乾淨的可視窗行列表。結果:
- 它的欄位是多個 vector,元素是 **PropertyClass window 物件**(嵌 `GetClassName`/`HasParent`/`Parent` 反射方法名)—— 是**整棵 UI window 子樹**,不是訊息行。
- `+0xa0` 的 12 元素不是聊天行,是各種 UI 控件(`BrightnessControlWindow`/`ControlFreeChat`/`BadgeFilter`/`Pip Conversion`…)。
- 訊息文字被拆散成 `+0x1c8` 的**排版 run 碎片** + 字元級 glyph/run 物件,沒有「一行 → 一則訊息文字」的乾淨映射。
- 要重組需逐 run/glyph 讀字元碼、依行/順序拼接、還原 sender/markup —— 極高成本,且成果仍是渲染文字(重複訊息身分問題不變)。

**結論:富文字模型重組的成本與收益不成比例**,相對現有活文件版(讀渲染 markup 文字)沒有實質優勢。停在此。

## 全部逆向路線總結(2026-08-21)

| 路線 | 結果 |
|------|------|
| exe 靜態反組譯(RTTI/屬性系統) | ChatHistory 等類別無 vtable;走 PropertyClass 框架,無固定偏移直達容器 |
| 純讀 pointer scan(自寫工具,不需除錯器) | 秒找 .data 固定入口 ControlRichEdit,但其內容是渲染碎片,非乾淨容器 |
| 網路層 handler 追蹤 | handler 名/欄位無直接引用,走 dispatch 框架;sender 以 `<link;GID>` 內嵌渲染文字 |
| ControlRichEdit 富文字模型 | 多層 window 樹 + run 碎片,重組成本不成比例 |

**最終定論:Wizard101 聊天在持久記憶體只以「渲染標記文字」存在,無帶身分/順序/時間戳的乾淨結構化容器。這是記憶體本質,四條獨立路線一致證實。務實最佳解 = 現行純讀 markup 文字方案(commit 64b874e)。**

## 網路協議定義(可讀!)+ Frida hook 路線 — 2026-08-22

**重大發現:聊天封包的協議定義是可讀的 XML,在 `Data/GameData/Root.wad`**(KIWAD v1, 172812 檔)。
解開 `GameMessages.xml`,聊天訊息(Server→Client)欄位分開、乾淨:
- `MSG_CHANNELCHAT`(頻道/世界聊天): `SourceName`(STR 發送者)、`SourceID`(GID 唯一ID)、`Message`(WSTR 內容)、TargetID、Filter、Flags
- `MSG_DIRECTEDCHAT`(私聊): SourceName、SourceID、Message(WSTR)、TargetID、Filter
- `MSG_RADIALCHAT`(範圍): SourceName、SourceID、Message(STR)、Filter
即封包層 sender/text 是分開的;客戶端 handler 收到後才格式化成 markup 存記憶體顯示。
解 WAD:`scripts/dump_chat_protocol.py`(KIWAD header 5+4+4,檔案表 off/usz/csz/comp/crc/nlen,zlib)。

**Frida hook 可行但找不到聊天框正確入口**(`scripts/frida_*.py`):
- Frida attach 成功,反作弊沒擋,能逐則即時攔截。
- pointer scan 落到的 `ControlRichEdit`(RVA 0x33b0fc0)是**通用文字控件**(物品數量/植物資訊/自己訊息預覽 `<center>text</center>`),**不是聊天框**。
- hook 該類全部 90 個 vtable method(vtable RVA 0x29dd4f8)+ 過濾 Art_Chat → **別人的聊天完全不經過此類任何 method**。
- 底層 `memmove`/`memcpy` hook 會**拖垮/崩潰遊戲**(高頻),不可用。
- 純讀找別人聊天行(含 Art_Chat + `<link;GID>[名]`)的持有物件 → 又是散落渲染副本,無乾淨控件容器。
- DML handler 走數字 order dispatch,handler 名字串(`MSG_ChannelChat`)零引用,dispatch 核心一直定位不到。

**六路線最終定論**(exe 反組譯 / pointer scan / 網路協議 / Frida vtable / memcpy trace / 控件反查):
聊天顯示層在 KI 自訂 window 框架裡**沒有單一可攔截的乾淨入口**;持久記憶體只有散落渲染文字。
唯一乾淨資料在**網路封包層**(MSG_CHANNELCHAT),要取需 hook DML dispatch 核心(未定位)或解密封包 —— 投報比極低。務實最佳解仍為現行純讀 markup 方案(commit 64b874e)。

## 現行實作(未走上述路線)

`src/reader/mem_reader.py`:純讀 `ReadProcessMemory` 掃描聊天標記文字、定錨「活文件」、翻譯尾端新增行。對不同內容的訊息可靠、閒置不冒舊;弱點是「快速連續發送完全相同的短語」可能漏(渲染層無訊息身分,內容無法區分)。
