# Wizard101 聊天翻譯助手：疊加視窗訊息的框選與複製設計

日期：2026-09-06
狀態：已實作並合併回 `master`（2026-09-07）。**實機驗證後有數項決策改變，本文其餘
部分維持當時的設計原貌，變動一律記在文末的「事後修訂」——讀本文任一節之前請先看那節。**

## 目標

讓使用者能用滑鼠在疊加視窗的訊息上**框選文字並複製到剪貼簿**。這是
`2026-09-01-system-message-translation-design.md` 結尾預告的「另一份獨立的 spec」。

1. **字元級框選**：按住左鍵拖曳選字，選取範圍即時反白。
2. **選取可跨同一則訊息的原文行與譯文行**，但**不跨越訊息邊界**。（邊界限制**已於事後
   取消**，見文末「事後修訂」）
3. **兩條複製路徑**：`Ctrl+C` 與右鍵選單「複製」。

## 非目標

- **不做跨訊息選取**。使用者已確認一次只選一則。（**已於事後推翻**，見文末「事後修訂」）
- 不做全選、不做選取後的搜尋／匯出、不做泡泡狀態下的選取。
- **不改變訊息的渲染方式**。描邊文字（`_outlined_line`）是為了「透明度調低時文字壓在
  亮色遊戲畫面上仍有對比」而存在的刻意設計，本功能不得以它為代價——這也是不採用
  `tk.Text` 承載訊息的原因（`tk.Text` 畫不出描邊）。
- 不動既有的拖曳移動、四邊縮放、捲動與自動貼底邏輯（除了一處拖曳期間的暫停，見下）。

## 前提探測（2026-09-06，丟棄用的純 Tk 腳本，不進專案）

三個決定性的行為已實測確認：

| 探測項 | 結果 |
|---|---|
| `canvas.index(item, "@x,y")` 在**換行後**的文字上 | 正確對映到視覺行與字元索引（`@3,22` → 第 2 視覺行起點 index 44） |
| 由字元索引反查 x 座標 | `font.measure(該視覺行前綴)` ＋ 原點偏移 2px 與 Tk 完全吻合（38 vs 36+2、73 vs 71+2） |
| 兩個 canvas 同時保有原生選取 | **不可能**。canvas2 一取得選取，canvas1 的 `select_item()` 立刻變 `None` |

前兩項的意義：**換行規則可以向 Tk 問出來，不必自行實作換行器**。第三項的意義：
既然選取要跨原文行與譯文行（兩個獨立 canvas），原生選取用不上，**反白必須自繪**。

## 關鍵約束：`BG` 是 colorkey，透明像素接不到滑鼠

`src/ui/palette.py:9` 已記下這件事：本體以 `BG` 當 `-transparentcolor`，該色像素連
hit-test 都被 Windows 跳過。訊息列的底色正是 `BG`，所以**訊息區只有文字墨跡接得到
滑鼠事件**；字間空隙、行尾、左右邊距的點擊會穿透到下層的 `backdrop`。

框選若只綁在 text canvas 上，使用者就無法從空白處起手——這是本設計採用雙路由
（見「二、事件路由」）的唯一原因。

## 決策摘要

| 決策 | 選擇 | 理由 |
|---|---|---|
| 選取粒度 | 字元級框選 | 使用者指定 |
| 選取邊界 | 同一則內跨原文↔譯文，不跨訊息 | 使用者指定 |
| 反白繪製 | 自繪 `create_rectangle`，壓在文字之下 | 原生選取跨不了 canvas（已實測） |
| 換行資訊來源 | 向 Tk 問（`index("@x,y")`） | 不重寫換行演算法 |
| 行內 x 座標 | `font.measure(視覺行前綴)` | 已實測與 Tk 吻合；純函式、可測 |
| 滑鼠事件 | text canvas ＋ backdrop 雙路由，統一螢幕座標 | colorkey 讓空白處的事件落到 backdrop |
| 「不跨訊息」 | 夾取（clamp）而非阻擋 | 拖出該則就夾到頭／尾，行為自然、無額外判斷 |
| 複製觸發 | `Ctrl+C` ＋ 右鍵選單 | 使用者指定；右鍵是不依賴鍵盤焦點的保底路徑 |

---

## 一、選取模型：`src/ui/selection.py`（新模組）

模組分成兩層：**模組層純函式**負責幾何與取字（不碰事件、不碰剪貼簿），**`Selection`
類別**持有選取狀態與反白繪製。兩層都不碰視窗管理、不碰剪貼簿——事件路由與剪貼簿
留在 `overlay.py`。

### 資料來源一律回頭問 canvas

`_outlined_line` 畫了九個 text item（八個描邊 ＋ 一個本色），本色那個掛有 `"fg"` tag，
九個都掛 `"txt"` tag。凡是需要 item 的地方一律用 `"fg"` 定位本色 item：

- 文字內容：`canvas.itemcget("fg", "text")`
- 字型：`tkfont.Font(canvas, font=canvas.itemcget("fg", "font"))`，由 `Selection`
  **對每個 canvas 快取一份**——拖曳時每個 motion 事件都要量測，不能每次重建。
- 排版邊界：`canvas.bbox("fg")`。**不可用 `bbox("all")`**：描邊副本往外多 1px，
  會讓視覺行數的計算多算出半行。

字型與文字都不另外在 overlay 記一份，避免與畫面脫節。

### 位置表示

```python
class Caret(NamedTuple):
    """選取端點。line＝該則訊息內第幾個 canvas（0＝原文行，1＝譯文行）；
    index＝該行文字內的字元索引。"""
    line: int
    index: int
```

一次選取是 `(anchor, focus)` 兩個 `Caret`，繪製與取字前正規化成 `(start, end)`，
`start > end` 時對調——反向拖曳（由下往上、由右往左）必須與正向等價。

### 公開函式

```python
def caret_at(canvas, x, y) -> int
```
包一層 `canvas.index("fg", f"@{x},{y}")`，把本地座標換成字元索引。座標超出文字範圍時
Tk 自己會夾到頭／尾（已實測：`@999,999` → `len(text)`）。

```python
def visual_lines(canvas, font) -> list[tuple[int, int, int]]
```
換行後每個視覺行的 `(起始索引, 結束索引, y_top)`。作法：`bbox("fg")` 高度 ÷
`font.metrics("linespace")` 得視覺行數，逐行以 `canvas.index("fg", "@2,<該行中線>")`
取行首索引，行尾＝下一行行首（最後一行＝文字長度）。y_top＝`2 + n * linespace`
（文字 item 以 `anchor="nw"` 畫在 `(2, 2)`，見 `_outlined_line`）。

```python
def highlight_rects(canvas, font, start: int, end: int) -> list[tuple[int, int, int, int]]
```
對每個與 `[start, end)` 相交的視覺行產生一個矩形。行內 x 由
`2 + font.measure(該視覺行文字[:欲求索引 - 行首索引])` 得到；跨越整行時直接取行的
左右界。回傳的是 canvas 本地座標，繪製交給呼叫端。

```python
def selected_text(texts: list[str], start: Caret, end: Caret) -> str
```
從該則的各行文字取出選取到的內容。跨兩行時以 `\n` 相接。純字串運算、不碰 Tk——
`texts` 由 `Selection` 以 `itemcget("fg", "text")` 讀出後傳入，函式本身保持可純測。
`start == end`（空選取）回傳空字串。

### `Selection` 類別

持有註冊的列、`anchor`／`focus` 兩個 `Caret`、以及作用中的那一則：

```python
register(row, canvases)   # add_message 建列後註冊
forget(row)               # 列被銷毀時解除註冊；若選取在該列則一併清除
begin(x_root, y_root)     # 起手，回傳是否命中某一則
extend(x_root, y_root)    # 拖曳中更新 focus（夾在該則範圍內）並重畫
finish()                  # 放開滑鼠，結束拖曳狀態
clear(reason)             # 清除選取並記 log
redraw()                  # 換行寬度改變後依現有 anchor/focus 重畫
text()                    # 目前選到的字串
active / dragging         # 目前是否有選取／是否正在拖曳
```

### 反白顏色

`src/ui/palette.py` 新增 `SELECT_BG = "#2d4a7a"`。**絕不能用 `BG`**——那是 colorkey，
畫上去等於沒畫。用不透明色另有一個附帶好處：已反白的區域從此接得到滑鼠，拖曳回頭
經過選取區時事件不會突然掉到 backdrop。

繪製以 `create_rectangle(..., fill=SELECT_BG, outline="", tags="sel")` 加入，隨後
`tag_lower("sel", "txt")` 壓到描邊與本色之下——描邊與本色的既有繪製邏輯完全不動。
清除選取即 `delete("sel")`。

**連帶要改一行**：`_fit_line_height` 目前用 `bbox("all")` 量高度，反白矩形一畫進來
就會被算進去（矩形底緣以 `linespace` 為準，可能比文字墨跡低一兩個像素），視窗每縮放
一次列高就長高一點。改成 `bbox("txt")`，只認文字。

---

## 二、事件路由：`src/ui/overlay.py`

### 雙路由，統一座標系

所有滑鼠事件一律轉成螢幕座標 `(e.x_root, e.y_root)` 再交給 `Selection`；它不管事件
從哪個 widget 來。

| 事件落點 | 接收者 | 處理 |
|---|---|---|
| 文字墨跡像素 | 該列的 text canvas（新綁 `<ButtonPress-1>`／`<B1-Motion>`／`<ButtonRelease-1>`／`<Button-3>`） | 進選取／彈選單 |
| 字間空隙、行尾、左右邊距（colorkey 穿透） | `backdrop`（已綁 press／drag／release 做邊緣縮放，另新綁 `<Button-3>`） | 左鍵先問 `_edge_under`；**在邊上維持現狀走縮放**，不在邊上才進選取 |
| 標題列、捲軸、右下把手、更新橫幅 | 各自既有 handler | 完全不動 |

**右鍵與左鍵一樣要雙路由**：在選取區以外的空白處按右鍵，事件同樣會穿透到 backdrop。

按下之後 Tk 的 implicit grab 會把後續事件鎖在起手的那個 widget，所以拖曳跨越墨跡與
空隙不會中斷。這是跨 Toplevel（backdrop ↔ 本體）的行為，列入實機驗收。

### 命中哪一行

`Selection` 遍歷已註冊的列，用既有的 `point_in_rect`（`src/ui/geometry.py`，純幾何、已有
測試）配 `winfo_rootx/rooty/width/height` 找出螢幕座標落在哪個 canvas，再減去該
canvas 的 root 座標換成本地座標交給 `caret_at`。**不新寫命中函式。**

### 「不跨訊息」是夾出來的

起手那一刻決定作用中的那一則。拖曳時若座標不落在該則的任一行內，focus 夾到該則的
頭或尾（在該則之上→`Caret(0, 0)`；之下→`Caret(最後一行, 該行長度)`）。往上拖就選到
該則開頭、往下拖就選到結尾，不需要任何「阻擋跨越」的判斷。

### 焦點

選取成立時對本體 `self._win.focus_force()`，`Ctrl+C` 才收得到鍵盤事件——`backdrop`
帶 `WS_EX_NOACTIVATE`，從它起手不會給焦點。本體本來就有工作列按鈕、點標題列時本來
就會奪焦，所以不算新行為；但**這是最需要實機驗證的一點**（遊戲全螢幕時的行為），
`focus_force` 前後各埋一行 log。右鍵選單不依賴焦點，是保底路徑。

### 拖曳期間暫停貼底

`_refresh_scroll` 在選取拖曳進行中跳過 `yview_moveto(1.0)`。否則新訊息進來把畫面
拉走，游標下的字整個換掉，選到的範圍會亂跳。拖曳結束後恢復既有行為（`_follow`
本身的值不動）。

### 選取的失效邊界

以下情況一律清除選取，各埋一行 log 記錄原因：

- **選取所在的列被銷毀**：`prune`、`set_limits` 擠掉、`add_message` 溢位。
- **`update_message` 就地換掉譯文行的文字**：字元索引失效；只有選取落在該列時才清。
- **`minimize()` 縮成泡泡**。
- **在空白處按下且沒選到任何字**：清除既有選取（等同「點一下取消選取」）。

**視窗縮放不清除選取**：`_on_canvas_configure` 改了 `_wrap`、文字重新換行，但字元
索引仍然有效，重呼叫 `highlight_rects` 重畫反白即可（成本近乎零），選取內容不變。

### 針對性小整理：`_drop_row`

`row.destroy()` 目前散在三處（`add_message` 溢位、`set_limits`、`prune`）。抽成一個
`_drop_row(entry)`，讓「銷毀列」與「清掉該列的選取」永遠成對出現——漏掉任一處就會
留下指向已銷毀 widget 的選取。**僅止於此**，不擴大到無關的重構。

---

## 三、複製

### 內容組裝

跨兩行時原文與譯文之間以 `\n` 相接（`selected_text` 負責）。

### 寫入剪貼簿

`clipboard_clear()` → `clipboard_append(text)` → `update()`。Windows 下 Tk 要 flush
過才會真的落進系統剪貼簿，關掉程式後內容仍在。

### 兩條觸發路徑

- **`Ctrl+C`**：綁在本體 `self._win` 上，`<Control-c>` 與 `<Control-C>` 都要綁
  （Caps Lock 開著時 Tk 送的是後者）。**不可用 `bind_all`**——那會連設定視窗的
  輸入框一起攔截。沒有選取時不做任何事，也不吃掉事件。
- **右鍵選單**：`tk.Menu(tearoff=0)`，單一項目「複製」，以
  `tk_popup(x_root, y_root)` 彈出、`finally` 裡 `grab_release()`（標準慣例，否則點
  別處關掉選單後滑鼠會被 grab 卡住）。**只在有選取時才彈**，沒選取就不彈，不做灰掉
  的空選單。字型走 `ui_font()`，語言切換時由既有的 `refresh_labels()` 重建。

空選取（按下即放開、`anchor == focus`）不寫剪貼簿，避免把使用者原本的剪貼簿內容
清成空字串。

---

## 四、介面文字與 log

### i18n

新增一個 key `menu.copy`，三份語言檔（`zh-TW` 為來源、`en`、`zh-CN`）都要補；
`tests/test_i18n.py` 會擋漏。

`selection.py` 是純幾何、不含任何介面文字，不必加進
`tests/test_no_hardcoded_ui_text.py` 的 `SCAN_TARGETS`；選單文字寫在已被掃描的
`overlay.py` 裡。

### log（`[ui]` 前綴，內容一律英文）

- 選取起手：第幾列、第幾行、字元索引。
- 選取結束：範圍與長度。
- 複製：**字元數**。
- `focus_force` 的結果。
- 每一種清除選取的原因（prune／update／minimize／點空白）。

**只記長度不記內容**——log 不該把使用者的對話再抄一份。

---

## 五、測試策略

單元測試都用既有的 `root` fixture（`tests/conftest.py`：真實 Tk，視窗停到螢幕外但
保持 mapped，量得到真實排版），**不需要開遊戲**。

### `selection.py` 幾何層

- `visual_lines` 對中英混排的換行文字回報的行首索引，與 `canvas.index("@x,y")`
  round-trip 一致。
- `highlight_rects` 算出的 x 與 Tk 自己的 `index("@x,y")` 反查一致。**這是守住
  「`font.measure` 與 Tk 換行對齊」這個核心假設的哨兵測試**——哪天 Tk 的換行行為變了
  會當場轉紅，而不是等使用者回報「反白偏掉了」。
- `selected_text`：單行、跨兩行、反向拖曳、空選取。
- 夾取：座標落在該則之上／之下時分別夾到頭／尾。

### `OverlayWindow` 層

- 選取後 `prune` 掉該列 → 選取清除且無殘留的 `sel` 矩形。
- `update_message` 換掉譯文 → 選取清除。
- `_on_canvas_configure` 改變 wrap → 反白重畫，選取內容不變。
- 複製後 `root.clipboard_get()` 拿到正確字串。

### 實機驗收（開遊戲並登入進世界內）

1. 在文字墨跡上按下拖曳可選字，反白即時跟隨。
2. **從字間空隙／行尾起手也能選**（backdrop 路由生效）。
3. **從空隙起手、拖過墨跡、再回到空隙，選取不中斷**（跨 Toplevel 的 implicit grab）。
4. 由原文行拖到譯文行可連續選取。
5. 拖出該則的上下界會夾住，不會選到別則。
6. `Ctrl+C` 有效（焦點那一點）。
7. 右鍵選單「複製」有效，且在空白處按右鍵也彈得出來。
8. 選取期間新訊息進來不打斷選取、畫面不被拉走。
9. 縮放視窗後反白仍貼齊文字。
10. 四邊縮放、標題列拖曳、捲動全部維持原狀。

---

## 不在本設計範圍

- 跨訊息選取、全選、選取後的搜尋／匯出。（跨訊息選取**已於事後實作**，見下節）
- **拖曳到視窗上下緣時自動捲動**。選取限於單一則訊息，該則超出視野的機會不高；有需要
  再加。（**已於事後實作**，見下節）
- 泡泡（縮小狀態）下的選取。
- 訊息渲染方式的改變（描邊維持不變）。
- 設定視窗的新增選項——本功能無需任何 config 欄位。

---

## 事後修訂（2026-09-07）

以下五項在實機驗證與後續需求中改變了原設計。上文未回頭改寫，保留當時的判斷與理由；
兩者衝突時**以本節為準**。

### 一、跨訊息選取（原列為非目標）

使用者實測後要求能跨多則訊息複製，訊息之間以空行分隔。`Caret` 從 `(line, index)`
擴成 `(message, line, index)`，欄位順序即閱讀順序，`NamedTuple` 的字典序比較仍可直接
拿來排先後，`span()` 的正規化未動。`selected_text` 的輸入改成二維 `list[list[str]]`，
同一則內以換行相接、訊息之間以空行相接。

**連帶的新責任**：訊息改以序號定位之後，`prune` 砍掉最舊的一則會讓序號整體位移。
`Selection.forget()` 因此必須維護序號——被移除的列落在選取範圍內就清除選取，落在選取
之前則把兩個 caret 的 message 各減一。少了這段，舊訊息一過期，選取就會無聲地滑到別的
行上。原設計以「作用中的那一列」的物件參照定位，沒有這個問題。

### 二、拖曳時自動捲動（原列為不在範圍）

原本的理由是「選取限於單一則，該則超出視野的機會不高」；跨訊息選取讓這個前提消失，
使用者也直接要求。作法是純函式 `overlay.autoscroll_pixels(y, top, bottom)` 決定每輪
捲動的像素（離邊界越遠越快、有上限），由 40ms 的 `after` 迴圈驅動。**每捲一次都要用
最後記下的游標座標重新 `extend` 一次**——內容在游標底下移動了，壓著的字換了人。

### 三、右鍵選單改為自繪視窗，不用 `tk.Menu`

「三、複製」一節描述的是 `tk.Menu` ＋ `tk_popup`。實測（截圖存證）發現 Tk 在 Windows
會替 menu 畫一圈系統的淺色外框，`borderwidth`／`relief`／`activeborderwidth` 都拔不掉，
與深色疊加視窗格格不入。改為 `src/ui/popup.py` 的 `Popup`：自繪的 `overrideredirect`
小視窗，配色沿用 `palette` 既有值，圖示用 `tk.Canvas` 畫（底色能跟著 hover 換，也避開
`PhotoImage` 被別的執行緒 GC 時對 Tk 呼叫的那個坑）。

**必須掛 `WS_EX_NOACTIVATE`**：`overrideredirect` 視窗一 `deiconify` 就會被 Windows
啟用，鍵盤事件從此落到選單身上——選單開著時本體的 `Ctrl+C` 會失效，而且彈選單會把
焦點從遊戲搶走。backdrop 早為同一理由掛了這個 style，選單一開始漏掉，實機才發現。

### 四、關閉選單只靠點擊，沒有 Esc

Esc 曾實作又移除。使用者的判斷是點空白處才是直覺動作，而鍵盤路徑只在疊加視窗持有
焦點時有效，多一條時靈時不靈的關閉方式反而更糟。連帶補掉一個漏洞：關閉原本只掛在
框選的按下路徑上，點**標題列、捲軸、右下把手**都關不掉選單——而那些正是使用者會直覺
點的地方。改為綁在兩個 shell 視窗的 `<ButtonPress-1>` 上，Tk 的事件會沿 bindtags 傳到
所屬 toplevel，一條綁定涵蓋裡面所有控件。

### 五、`_fit_line_height` 的理由更正

「一、選取模型」末尾稱反白矩形會被 `bbox("all")` 算進列高、導致每次縮放都長高一點。
**這個推論是錯的**：描邊副本畫在 `(3, 3)`，底緣永遠比反白矩形低 1px，兩者的 bbox 恆等。
改用 `bbox("txt")` 仍然保留（只量文字更精確、零成本），但它是防禦性的，不修任何實際
症狀；當初為它寫的那條測試因為驗不到東西已經刪除。
