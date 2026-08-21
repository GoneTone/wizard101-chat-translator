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

## 現行實作(未走上述路線)

`src/reader/mem_reader.py`:純讀 `ReadProcessMemory` 掃描聊天標記文字、定錨「活文件」、翻譯尾端新增行。對不同內容的訊息可靠、閒置不冒舊;弱點是「快速連續發送完全相同的短語」可能漏(渲染層無訊息身分,內容無法區分)。
