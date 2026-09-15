# Wizard101 聊天翻譯助手：框選畫面區域翻譯設計

日期：2026-09-15
狀態：已實作（分支 `feat/region-translation`）。實機驗證後有數項決策改變，本文其餘
部分維持當時的設計原貌，變動一律記在文末的「事後修訂」—— 讀本文任一節之前請先看那節。

## 目標

讓使用者按熱鍵、在遊戲畫面上**拖曳框選任意區域**，把區域內的文字辨識並翻成設定的目標
語言。這是為了 wizwalker hook 讀不到的文字（NPC 對話、任務書、道具說明、商城的 Flash
介面等），聊天本身仍走既有的記憶體讀取，不受影響。

1. **一次性操作**：熱鍵 → 拖矩形 → 辨識並翻譯一次 → 顯示 → 點一下消失。要再翻就再按。
2. **辨識以「模型看圖」為主**：截圖直接交給現有的翻譯後端，一次請求完成辨識加翻譯。
3. **模型不吃圖時自動退回本機 OCR**（Windows 內建 `Windows.Media.Ocr`），再走文字翻譯。
4. **譯文貼在框選區域正下方**，原文保持可見，不進疊加視窗的訊息列表、也不蓋在原文上。

## 非目標

- 不持續監看同一區域、不自動重翻（區域內容變了要再按一次熱鍵）。
- 不做 Google Lens 式逐行覆蓋原文：需要每行座標，模型看圖給的座標官方文件明說是近似值，
  兩條辨識路徑的顯示會不一致，遊戲 3D 背景上字排不齊反而更難讀。
- 不做整個畫面自動辨識、不做聊天的 OCR 備援。
- 不打包 Tesseract／PaddleOCR 一類引擎：本機 OCR 只用 Windows 內建的。
- 不動 reader loop、翻譯池、譯文快取、疊加視窗訊息列表 —— 這條資料流與它們無關。
- 卡片先不做框選複製；之後有需要再接現有的 `Popup`。

## 決策摘要

| 決策 | 選擇 | 理由 |
|---|---|---|
| 辨識對象 | 使用者框選的任意區域 | 使用者指定；最通用 |
| 生命週期 | 一次性 | 使用者指定；不需區域差分與重翻判定 |
| 辨識方式 | 模型看圖優先，退回本機 OCR | 零新引擎、任何語言與遊戲字型都吃；純文字模型也要能用 |
| 圖片 token | 不特別壓縮 | 框一個對話框約 176 token（Claude 以 28×28 px 為一個視覺 token），與翻一則聊天同量級 |
| 「不吃圖」判定 | 圖片 4xx **且同一輪文字翻譯成功**才標記 | 任何 4xx 就標會把金鑰錯、模型不存在誤判成不吃圖；不解析錯誤字串（各家措辭不同） |
| 擷取來源 | `PrintWindow` 對遊戲視窗 | 疊加視窗、輸入框、卡片、其他程式一律不入鏡，也不用「先藏再拍」 |
| 譯文位置 | 卡片貼在矩形正下方、同寬 | 眼睛不用離開原文；沿用輸入框貼齊遊戲輸入框那套定位 |
| 卡片焦點 | 不奪焦點 | 遊戲鍵盤操作不中斷；代價是收不到 Esc，改點卡片關閉 |
| 觸發 | 第二個全域熱鍵 `region_hotkey` | 與現有輸入框熱鍵同一套註冊與前景判定 |

---

## 一、架構與資料流

新增 `src/region/` 套件（與 reader／composer／translation 平級，這條資料流的擁有者），
UI 元件照慣例放 `src/ui/`。

| 模組 | 職責 | 依賴 |
|---|---|---|
| `region/capture.py` | 遊戲視窗 → 依螢幕矩形裁切 → PNG bytes；座標換算為純函式 | pywin32、Pillow（新相依） |
| `region/ocr.py` | 本機 OCR：PNG bytes → 文字；引擎建不出來拋 `OcrUnavailable` | `winrt-*` 套件（新相依） |
| `region/pipeline.py` | 決策核心：先丟圖，端點不吃圖就退回 OCR＋文字翻譯；記住「不吃圖」直到 `reset()` | translator、ocr |
| `translation/translator.py` | 新增區域方向：`translate_region_image(png)`／`translate_region_text(text)` | 既有 |
| `translation/prompts.py` | 新增區域翻譯提示詞（目標語言當參數） | 既有 |
| `ui/region_select.py` | 全螢幕選取層：拖矩形、Esc 取消、回傳螢幕座標 | Tk |
| `ui/region_card.py` | 結果卡片：貼在矩形正下方、同寬；辨識中／譯文／錯誤三態 | `anchored_position`、`RichLabel`、`winstyle` |
| `main.py` | 第二個全域熱鍵、接線、設定套用時重註冊熱鍵並 `reset()` pipeline | 既有 |
| `config.py`／`ui/settings.py`／`ui/fields.py` | 新欄位 `region_hotkey`；設定視窗沿用熱鍵捕捉元件 | 既有 |

資料流（一次框選）：

1. 熱鍵（keyboard 執行緒）→ 遊戲在前景才處理（沿用 `on_hotkey` 的判定）→ 記下前景
   HWND → 排進 `ui_queue`。
2. 主執行緒收掉舊卡片、開選取層蓋住遊戲所在的那顆螢幕；拖出矩形後選取層立刻收掉，
   `force_foreground` 把前景還給遊戲。
3. 主執行緒對遊戲 HWND 擷取矩形；卡片以「辨識中…」佔位出現在矩形下方。
4. 背景執行緒跑 `pipeline`，結果經 `ui_queue` 回填卡片；每次框選 session +1，過期結果
   丟掉（與 `InputBox` 同一招）。
5. 點卡片、再按熱鍵、或再次框選 → 卡片消失。

同一時間最多一個框選在跑；背景執行緒每次框選各開一條，靠 session 淘汰舊結果。

## 二、擷取：`region/capture.py`

- 用 Win32 `PrintWindow(hwnd, hdc, PW_RENDERFULLCONTENT)` 向 DWM 要遊戲視窗自己的合成
  內容（只含這個視窗畫的東西），再依框選矩形裁切。遊戲被別的視窗蓋住時拍到的仍是遊戲
  內容，這正是要的。
- 框選矩形是螢幕座標：減掉遊戲 client 區的螢幕原點（`ClientToScreen`）換成視窗內座標，
  超出視窗的部分裁掉；完全落在視窗外 → 回報「請框選遊戲畫面內的區域」。
- 矩形小於幾個像素（誤點）由選取層先當取消（沿用 `is_click`），不會走到這裡。
- GDI bitmap 經 Pillow 轉成 PNG bytes 給 API 與 OCR（pywin32 拿得到位元組但沒有 PNG
  編碼器）。
- 圖片不預先縮放：框選區域通常不大，縮了字看不清；超過模型上限由 API 自己縮。
- **風險**：`PrintWindow` 對 DirectX 視窗在部分環境回全黑。實作前第一個 spike 就是對
  執行中的遊戲拍一張存檔確認。不過關的退路是「只在自家視窗與矩形重疊時暫時 withdraw、
  拍完還原」，有 DWM 時序與閃爍問題，能不走就不走。

## 三、辨識與翻譯

### `translator.py` 的區域方向

與收訊／發話並列，共用同一份系統提示詞，只有使用者回合的內容不同：

- `translate_region_image(png)`：使用者回合＝圖片＋一句指示。OpenAI 相容端點用
  `image_url` 帶 `data:image/png;base64,…`；Claude 用 `image` 區塊帶 base64。各 client 加
  一個帶圖的請求組裝，狀態碼判定、截斷偵測、`strip_think`、參數被拒重送全部沿用。
- `translate_region_text(text)`：使用者回合＝本機 OCR 出來的文字。**不走**
  `translate_incoming`：那套提示詞與後處理是為單行聊天調的（few-shot、發話者前綴、剔除
  模型自創的英文），任務書一類的多行敘述不適用。
- 長度上限用現有的 2048 那檔：任務書一頁翻成 CJK 可能超過 512。
- 成功路徑沿用 `_chat` 的 log 格式，但 `source` 改記矩形尺寸與字元數，不記整段內容。

### `prompts.py` 的區域提示詞

目標語言當參數帶入（不寫死任何語言）。要求：逐字辨識畫面上所有可讀文字、翻成目標
語言、只輸出譯文、保留換行與條列結構；畫面沒有文字就輸出空白。措辭沿用既有提示詞的
風格（中文撰寫；先前 A/B 實測英文提示詞不划算）。

### `region/pipeline.py` 的決策

```
if not text_only:
    try: return translate_region_image(png)             # 路徑 image
    except TranslatorConfigError as exc: image_error = exc   # 4xx，不計費
    except TranslatorOffline: raise                     # 端點掛了，文字也過不去
text = ocr.recognize(png)                               # OcrUnavailable 往上拋
if not text: return NoText
translated = translate_region_text(text)                # 失敗 → 拋文字這一路的錯誤
if image_error is not None: text_only = True            # 文字過、圖片被拒 → 證據充分
return translated                                       # 路徑 ocr
```

- 「不吃圖」標記只存在記憶體，設定視窗儲存時 `reset()` 清掉（provider／model 可能換了）。
- 文字翻譯也失敗 → 不標記，回報文字那一路的錯誤；下次框選仍先試圖片。
- 真的不吃圖的端點：每次啟動後第一次框選多一趟被拒的請求，之後直接走 OCR。
- `TranslatorBadOutput`（截斷）→ 回報翻譯失敗，不重試（temperature=0 重試結果相同）。

### `region/ocr.py`

- 辨識語言**優先挑遊戲語言**：從 `OcrEngine.available_recognizer_languages` 找主標籤等於
  `OUTGOING_LANGUAGE_TAG`（`prompts.py`，與 `OUTGOING_LANGUAGE` 並列的 BCP-47 主標籤，遊戲
  語言是既有的產品常數、不是對使用者語言的假設）的那個建引擎；沒裝才退回
  `try_create_from_user_profile_languages()`；仍是 `None` 即拋 `OcrUnavailable`。
  2026-09-15 實測：Windows OCR 是逐語言的引擎，繁中引擎辨識英文會把空格全部吃掉
  （`HelloWizardlOlquest`），依使用者設定檔語言建引擎並不可靠。
- PNG 經 `BitmapDecoder` 解成 `SoftwareBitmap` → `recognize_async`，各行以換行合併。
- WinRT 的 async 在 worker 執行緒用 `asyncio.run` 跑。
- `winrt` 模組延遲到第一次呼叫才 import；載入失敗（套件沒打包進去）也只是
  `OcrUnavailable`，不影響程式啟動。
- 首次建引擎成功時 log 一次可用的辨識語言清單。

## 四、UI 與觸發

### 選取層 `ui/region_select.py`

- 無邊框、置頂的 Toplevel，蓋滿**遊戲所在的那顆螢幕**（整個螢幕，不是工作區），淡淡的
  暗色半透明底讓遊戲仍看得見，十字游標；頂端一行提示「拖曳框選要翻譯的區域，Esc 取消」。
- 拖曳時畫矩形外框並標尺寸；放開後位移小於點擊門檻（`is_click`）視為取消。Esc、右鍵取消。
- 需要鍵盤與滑鼠，所以會奪焦點；關閉後立刻 `force_foreground` 還給遊戲。
- 回傳螢幕座標的矩形，其餘交給 `main.py` 接線。

### 結果卡片 `ui/region_card.py`

- 無邊框、置頂、**不奪焦點**（`make_non_activating`，與彈出選單同一招）。關閉方式：點卡片
  任一處、再按熱鍵、或再次框選時自動換掉。不做自動消失計時。
- 位置：`anchored_position` 貼在矩形正下方、左緣對齊、與矩形同寬（下限一個最小寬度，
  上限工作區寬度）；下方不夠就翻到上方。高度隨內容，文字依寬度換行。
- 三態：辨識中（`notice.pending`）→ 譯文 → 錯誤（紅色，文案來自 `banner_for()`）。
  配色、字型、不透明度沿用疊加視窗（`overlay_alpha`）。

### 熱鍵與設定

- 新欄位 `region_hotkey`，預設 `ctrl+shift+space`；註冊與無效組合退回預設沿用
  `register_hotkey`。設定視窗基本分頁多一列，沿用現有的「按下按鍵」捕捉元件；表單驗證
  加一條：兩個熱鍵不可相同。
- 按下時：選取層開著 → 取消它（同鍵切換）；卡片開著 → 收掉並開選取層；遊戲在前景 →
  開選取層；其他視窗在前景 → 忽略並 log。
- 設定儲存時重新註冊兩個熱鍵、呼叫 pipeline 的 `reset()`。

### i18n

新增的介面文字進 `en-US`、`zh-TW`、`zh-CN` 三份語言檔，其餘語系照既有機制退回英文、
等 Crowdin 補；`test_no_hardcoded_ui_text` 擋住漏掉的字串。

## 五、錯誤處理與 log

log 一律 `[region]` 前綴、英文、帶 context；譯文本身不進 log（可能很長）也不進
`messages.log`。

| 狀況 | 卡片 | log |
|---|---|---|
| 熱鍵按下但遊戲不在前景 | 無 | `hotkey ignored: foreground is not the game window (exe=…)` |
| 矩形完全落在遊戲視窗外 | 請框選遊戲畫面內的區域 | `selection outside game window (rect=…, client=…)` |
| `PrintWindow` 失敗或回空白 | 擷取失敗 | `capture failed (hwnd=…, rect=…, err=…)` |
| 圖片 4xx、文字成功 | 譯文 | `image input rejected (status=…, detail=…); falling back to local OCR`、`endpoint marked text-only` |
| 圖片 4xx、文字也失敗 | 文字那一路的錯誤 | 兩次失敗各一行 |
| `TranslatorOffline` | 離線／HTTP 錯誤（`banner_for` 文案） | `translate failed (status=…, detail=…)` |
| `TranslatorBadOutput` | 翻譯失敗 | translator 既有的截斷診斷 |
| `OcrUnavailable` | 本機辨識不可用，請安裝 Windows OCR 語言包或改用支援看圖的模型 | `local OCR unavailable: …` |
| OCR 空字串 | 沒有辨識到文字 | `local OCR found no text (rect=…)` |
| 成功 | 譯文 | `done in N.Ns via image/ocr (model=…, rect=…, chars=…)` |

背景執行緒任何未預期例外都接住、log traceback、卡片顯示「發生錯誤」。

## 六、測試與驗證

不需遊戲、`pytest` 可跑：

- `capture.py`：螢幕矩形 → 視窗內矩形、裁切、完全在外的純函式。`PrintWindow` 那層不測。
- `pipeline.py`：注入假 translator／假 OCR，涵蓋圖片成功、4xx＋文字成功→標記、4xx＋
  文字失敗→不標記、已標記直接走 OCR、Offline 不退回、OcrUnavailable、空字串、`reset`。
- `translator.py`：兩個 client 帶圖請求的 body 形狀（注入假 http／anthropic client，沿用
  既有手法）。
- `region_select.py`／`region_card.py`：拖曳→矩形、點擊→取消、Esc→取消；卡片三態文字、
  貼齊接線。沿用 conftest 的螢幕外停放。
- `config.py`／`settings.py`：新欄位補值、兩個熱鍵相同的驗證錯誤。
- `test_no_hardcoded_ui_text`、`test_i18n` 自動涵蓋新字串。

實機驗證（要開遊戲、登入進世界內）：

1. **第一個 spike**：`PrintWindow` 對遊戲視窗拍圖存檔，確認不是黑的、疊加視窗沒入鏡。
2. 打包後的 exe：`winrt-*` 套件被 PyInstaller 正確收進去（可能要加 `hiddenimports`）、
   Pillow 正常；在沒裝 OCR 語言包的帳號上確認錯誤提示。
3. 三家 provider 各框一次 NPC 對話；自架純文字模型確認退回 OCR 的流程與 log。

## 七、相依與文件

- 新增執行期相依（2026-09-15 於拋棄式環境實測，Python 3.14 皆有 wheel）：`pillow`、
  `winrt-runtime`、`winrt-Windows.Media.Ocr`、`winrt-Windows.Graphics.Imaging`、
  `winrt-Windows.Storage.Streams`、`winrt-Windows.Foundation`、
  `winrt-Windows.Foundation.Collections`、`winrt-Windows.Globalization`。後三個是 WinRT
  回傳集合與語言物件時**執行期動態載入**的投影，程式碼裡沒有 import，PyInstaller 追不到，
  `build.spec` 要列進 `hiddenimports`。
- README 三份各加一段「框選畫面翻譯」（設定只用泛稱，不寫鍵名與介面文字，兩者都會過期）。`docs/releasing.md` 不動。

## 事後修訂（2026-09-15，實機驗證後）

- **本機 OCR 改用 RapidOCR，推翻「不打包 OCR 引擎」的非目標。** `Windows.Media.Ocr` 對遊戲的
  花體字型讀不出來：拿商城標題「Items Recommended For Your Wizard」離線比了兩個引擎 × 四種倍率
  × 兩種插值 × 四種前處理共 56 種組合，沒有一種讀對；使用者用 Win+Shift+T（Windows 11 剪取
  工具的文字擷取，用的是剪取工具自帶的新引擎，Python 呼叫不到）能讀對。RapidOCR（PP-OCR
  ONNX 模型）同一張圖英文與簡體中文全對、免語言包，代價是 exe 多約 110 MB。§三的引擎語言
  挑選、§七的 winrt 相依與 hiddenimports 全部作廢；改為 `rapidocr` ＋ `onnxruntime`，打包時
  `collect_data_files("rapidocr")` 收模型。
- **5xx 也退回 OCR，但不標記「不吃圖」。** 實測 OpenAI 對純文字模型（gpt-3.5-turbo）收到圖片回
  500 而非 400；§三原本「Offline 不退回」會讓這類模型永遠用不了。改成 4xx 退回並依證據標記、
  5xx／429 退回但不標記（可能只是暫時故障），文字那一路也失敗才回報錯誤。
- **選取時整顆螢幕定格（像剪取工具）。** 按熱鍵先 `PrintWindow` 拍遊戲整窗、再 `ImageGrab` 拍整顆
  螢幕；選取層改成不透明，底下是壓暗的螢幕截圖、遊戲區疊壓暗的遊戲定格影像（不含自家視窗），
  拖框內恢復原亮度；放開後從定格的遊戲影像裁切，不再拍第二次。§二的「放開後再擷取」作廢。
- **看圖路徑先逐字抄寫再翻譯。** 模型會把截圖沒有的句子補進去；提示詞改成輸出「逐字抄寫」＋
  分隔線＋「譯文」（`REGION_SEPARATOR`），並要求逐行對應、不得遺漏；卡片在譯文上方以暗色小字
  顯示抄寫的原文供對照。區域路徑的輸出上限提高到 4096，OpenAI 相容端點的圖片帶 `detail: high`。
- **卡片可反白複製。** 原文與譯文放在同一個文字欄（`RichLabel.set_blocks`），拖曳反白可跨兩段，
  `Ctrl+C` 或右鍵「複製」；點一下（沒拖動）才關閉，另有 ✕ 與「點一下即可關閉」提示。§四「點卡片
  任一處關閉」放寬為「點一下關閉、拖曳選字」。
- **提示文字獨立成不透明小視窗**：選取層原本 35% 透明，畫在同一層的提示跟著變淡看不清。
- **疊加視窗標題列多一顆框選按鈕（⛶）**：不靠前景判斷，改列舉視窗找遊戲 HWND；標題列四顆按鈕與
  卡片 ✕ 都有滑鼠停留提示，⛶ 的提示帶目前熱鍵。
- **取消框選要把前景還給遊戲**（最終審查抓到）：否則下一次熱鍵被「遊戲不在前景」擋掉。
- **啟動時擋兩把熱鍵相同**（升級舊設定檔可能撞名）；擷取層所有 Win32／PIL 例外一律包成
  `CaptureError`。
- **測試基礎設施**：開發者邊玩遊戲邊跑測試 —— `force_foreground` 在測試中一律 no-op、
  `real_position` 測試改成視窗全透明照跑（不跳過）。
