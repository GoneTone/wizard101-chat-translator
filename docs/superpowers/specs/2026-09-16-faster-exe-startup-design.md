# Wizard101 聊天翻譯助手：exe 啟動加速與啟動畫面設計

日期：2026-09-16
狀態：設計完成，待實作。

## 目標

縮短打包版 exe 從雙擊到可用的時間，並讓**整段等待都有畫面**，不再是「按了沒反應」。

1. 砍掉啟動路徑上不必要的 import 與打包內容。
2. 從雙擊的第一刻就顯示啟動畫面，並隨階段更新狀態文字。
3. 維持**單一 exe** 的發布形式不變。

## 非目標

- **不改成 onedir**。onedir 可省下每次啟動的解壓（約 1.35 秒），但發布形式會從單檔變成
  資料夾，且 PyInstaller 記載 splash 的已知問題多半發生在 onedir 模式。單檔對使用者的
  「下載即用」是刻意保留的取捨。
- **不延後 `Translator` 的建立**。那會讓 Claude 官方 provider 的使用者也省下 import 成本，
  但那是翻譯核心的架構改動，不屬於本次範圍。
- 不動 reader loop、翻譯池、譯文快取、疊加視窗 —— 這次只碰啟動路徑與打包設定。
- 不追求「更快的翻譯」：本文只處理啟動，不碰執行期效能。

## 現況測量

exe 為 onefile，**131 MB 壓縮／284.2 MB 解壓／1136 個檔案**。啟動分三段：

| 階段 | 耗時 | 測量方式 |
|---|---|---|
| bootloader 解壓到 `%TEMP%\_MEIxxxxx` | 1.35 s | `CArchiveReader` 完整解壓並寫盤（warm SSD，不含 Defender） |
| Python 初始化 ＋ `import src.main` | 0.99 s | `python -X importtime`（warm） |
| `[app] version=` → `[app] running` | 0.47 s | 實機 `app.log` 時戳 |

合計約 **2.8 秒**，其中約 83% 發生在第一行 log 之前 —— 程式自身的建構不是瓶頸。

import 那 0.99 秒裡 **`anthropic` 佔 0.68 秒**（實測：載入 0.989 s，以假模組取代後 0.311 s）。

打包體積的大宗（未壓縮）：`cv2` 117.18 MB、`onnxruntime` 37.58 MB、`rapidocr` 模型
31.84 MB、`numpy.libs` 21.16 MB、`PYZ` 16.05 MB、`PIL` 13.41 MB。OCR 相關合計約 208 MB
（73%），但 `region/ocr.py` 早已把 rapidocr 延遲到第一次框選才載入 —— 這 208 MB
**在啟動路徑上一行都沒被 import**，卻因 onefile 而每次啟動都被完整解壓。

## 三項改動

### A. 延遲 import `anthropic`

`anthropic` 只在 `_ClaudeClient` 用到三個名稱（`Anthropic`、`APIConnectionError`、
`APIStatusError`），以及模組級 `_anthropic_detail` 的型別註解。

- 移除 `translation/translator.py` 頂層的 `import anthropic`。
- `_ClaudeClient.__init__` 內 import 並存成實例屬性，`except` 子句改用該屬性。
- `_anthropic_detail(exc)` 的型別註解改成**字串形式**。Python 3.11／3.12 會在函式定義時
  求值註解（PEP 649 的延遲求值是 3.14 才有），而專案 `requires-python >=3.11`，不能只靠
  3.14 的行為。函式本身只存取 `exc.body` 與 `exc.message`，duck typing 即可。

收益不對稱，需在發布說明中誠實描述：

- **custom／OpenAI 相容 provider：完整省下 0.68 秒**，`anthropic` 永不載入。
- **Claude 官方 provider：省 0 秒**，成本只是從 import 階段移到 `build_app()`；差別在於
  那時啟動畫面已顯示「Starting…」，不是空白等待。

### B. 排除打包中的死重

| 項目 | 未壓縮 | 依據 |
|---|---|---|
| `cv2\opencv_videoio_ffmpeg500_64.dll` | 30.88 MB | rapidocr 全套只用影像處理 API（`resize`／`cvtColor`／`findContours` 等），無一處碰 `VideoCapture`／`imshow` |
| `PIL\_avif.pyd` | 7.89 MB | 專案無任何 AVIF 用途 |

ffmpeg DLL 不是 Python 模組，`Analysis(excludes=...)` 管不到，而
`exclude_system_libraries()` 只對 POSIX 有效（過濾 `/lib*`、`/usr/lib*`）。只能過濾
`a.binaries`：

```python
a.binaries = [b for b in a.binaries if "opencv_videoio_ffmpeg" not in b[0]]
```

PIL 的 AVIF 先試 `excludes=["PIL.AvifImagePlugin"]` —— 切斷 import 鏈後 `_avif.pyd`
通常就不會被收；若打包產物顯示它仍在，再補進上面的過濾清單。**以打包產物驗證，不靠猜測。**

預期省下約 39 MB 解壓（約 0.18 秒）與約 17 MB 下載體積。

### C. 啟動畫面（PyInstaller Splash）

自行以 tkinter 寫的啟動視窗**做不到「立刻顯示」** —— 它最快也要等解壓（1.35 s）加
Python 初始化與 import（0.99 s）之後才畫得出來，只能覆蓋最後 0.47 秒，等於沒做。唯一
能在解壓開始前就顯示的是 PyInstaller 的 Splash，它跑在 bootloader（C 層）。

視覺與文案的決策：

- **品牌啟動畫面**，480×300。
- **狀態文字全英文寫死，不接 i18n**。技術上前半段本來就無法 i18n：解壓期間顯示的
  `text_default` 是打包時寫死的，那時 Python 還沒啟動、讀不到 `ui_language`；`text_font`
  同樣是打包時定死、執行期換不了字族。全英文可統一用 Segoe UI，無缺字風險，也沒有
  「前半英文後半中文」的跳躍感。
- 名稱與版本號**畫在底圖上** —— Splash 只提供一行動態文字（單一 `text_pos`）。

階段與時間軸（**A 與 B 生效後**的預期值）：

```
0.00s  解壓開始      Initializing…          （spec 的 text_default，bootloader 顯示）
1.17s  Python 起     Loading components…    （run.py，在 import src.main 之前）
1.48s  接線前        Starting…              （main()，build_app 之前）
1.95s  overlay 就緒  （關閉）
```

啟動畫面本身幾乎不增加體積：`Splash(binaries=a.binaries, ...)` 的 `binaries` 就是給它
偵測 tkinter 用的 —— 專案本來就打包了 tcl/tk，Splash 會沿用而非再塞一份。

`run.py` 必須**先更新文字再 import**：`import src.main` 要 0.31 秒（A 之後），否則這段
時間畫面還停在 `Initializing…`。

## 元件與檔案

| 檔案 | 改動 |
|---|---|
| `src/splash.py` | 新增。薄包裝，公開 `update(text)` 與 `close()` |
| `run.py` | import `src.main` 之前先更新文字 |
| `src/main.py` | 一個 `update()`、三個 `close()`（見下） |
| `build.spec` | 加 `Splash(...)`、過濾 `a.binaries`、補 `excludes` |
| `tools/splash_image.py` | 新增。Pillow 生成底圖，build 時由 spec 呼叫 |
| `src/translation/translator.py` | A 的改動 |

### `src/splash.py`

`pyi_splash` 只在打包版存在：開發模式會 `ImportError`，bootloader 未帶 splash 會
`RuntimeError`，socket 斷了會 `ConnectionError`。本模組的職責就是**吞掉這一切，讓呼叫端
可以無條件呼叫**，不必到處寫 try。形狀對齊 `src/log.py`：小、無狀態、失敗不影響主流程。

生命週期不做成 context manager —— `close()` 之後程式還要跑好幾小時的 `mainloop`，包不
起來；也不做成注入 `build_app` 的 progress 物件，那是為「未來可能換別的進度 UI」預先
抽象，違反 YAGNI。

### 關閉時機：三個出口

`main()` 有三條路徑會離開啟動階段，每條都要關閉，否則啟動畫面會一直浮在最上層：

1. **已有實例在跑** → `focus_running_instance()` 之前關（否則會蓋住被喚起的視窗）。
2. **首次執行精靈** → `run_wizard()` 之前關。
3. **正常路徑** → `build_app()` 之後、`mainloop()` 之前關。

### `tools/splash_image.py`

讀 `src/assets/icon.ico` 的 128px frame（原生尺寸，不放大）與 `src/__init__.py` 的
`__version__`，輸出 480×300 PNG 到 `build/`。不進版控 —— 它是 build 產物，且版本號每次
發布都會變。版本號從 `__version__` 帶入，維持版本號的單一真實來源。

底圖生成放在獨立模組而非塞進 `build.spec`：spec 已有 60 行版本資訊與打包設定，再加繪圖
邏輯會混兩種職責。

- **字型三段退階**：`segoeuisb.ttf`（Semibold）→ `segoeui.ttf` → Pillow 內建預設。退階時
  印明顯 warning 到 build log，但**不擋 build** —— 字型只影響美觀，不值得讓 release 掛掉。
  發布在 `windows-latest` runner 上跑 `uv run pyinstaller build.spec`，兩種 Segoe 都有，
  退階只是保險。
- 底色**避開 magenta `#ff00ff`** —— Windows 用它做透明色。
- 生成失敗（icon 讀不到、寫檔失敗）則讓 build 直接失敗，不靜默跳過：發出一個沒有啟動
  畫面的版本卻沒人發現，比 build 紅掉糟。

## 錯誤處理與 log

`src/splash.py` **刻意不在模組載入時記 log**：`run.py` 會在 `redirect_output()` 之前就
呼叫 `update()`，而 windowed exe 在輸出導向之前 `stderr` 是 `None`（`log()` 會直接 return），
那時寫出去的行會憑空消失 —— 正好是打包版這個最需要診斷的情境。

可用性改由 `main()` 在 `redirect_output()` 之後記一次，並帶上 `frozen` 以區分兩種
「沒有啟動畫面」：

```
[splash] startup screen not present (frozen=False)   ← 開發模式，預期如此
[splash] startup screen not present (frozen=True)    ← 打包版漏掉 splash，真的有問題
```

`update()`／`close()` 失敗時各記一行。整個生命週期只呼叫兩次 `update` 與一次 `close`，
不會洗版。`close()` 之後的 `update()` 直接忽略（不轉給 `pyi_splash`）—— 首次執行精靈那條
路徑就會這樣走，真的送出去只會換來一行誤導人的失敗 log。

log 訊息一律英文，前綴 `[splash]`。

## 測試

| 測試 | 內容 |
|---|---|
| `tests/test_splash.py` | 無 `pyi_splash` 時 `update()`／`close()` 不拋例外；注入假模組時確實轉呼叫到 |
| `tests/test_splash_image.py` | 生成不拋例外、輸出為 480×300 PNG |
| `tests/test_translator.py`（既有） | A 的改動不得破壞既有注入 client 的測試 |
| 新增：釘住 A | subprocess 起乾淨直譯器，斷言 `import src.main` 後 `anthropic not in sys.modules` |

最後一項的理由：A 的價值全靠「啟動路徑不碰 `anthropic`」，而這種性質會被任何一次無心的
import 悄悄破壞，且不會有任何既有測試變紅。同一個 process 內別的測試早已載入
`anthropic`，所以必須用 subprocess 起乾淨直譯器才驗得準，成本約 0.5 秒。

## 實機驗證（打包後）

前四項是已知風險的直接對照，第五項驗證 B 的排除無害：

1. **焦點** —— splash 關閉後 overlay 是否被丟到背景。
2. **圖示** —— 工作列與視窗圖示是否正常（`apply_window_icon()` 是否被 splash 佔住
   primary window 影響）。
3. **首次執行精靈** —— 將 `config.json` 改名模擬，splash 須在精靈出現前關閉。
4. **第二實例** —— 再開一次 exe，splash 須在既有視窗被喚起前關閉。
5. **框選翻譯** —— 仍能正常辨識，確認 ffmpeg DLL 的排除無害。

## 驗收條件

- `uv run ruff check src tests tools` 零錯誤。
- `uv run pytest` 全綠。
- 上述五項實機驗證通過。
- 啟動時間前後對照：預期 warm 從約 2.8 秒降到約 2.0 秒，且**全程有畫面**。

## 已知風險

PyInstaller 官方文件記載了兩個問題，且都正中本專案要害：

> 焦點問題：splash 關閉時，應用程式視窗或對話框會被丟到背景。
> 使用 tkinter 的程式可能無法透過 `Tk.iconphoto()` 正確設定 app-wide 視窗圖示，因為
> splash 被當成 primary window 初始化。

第一個對「疊在遊戲上的 overlay」是真風險 —— 專案已為了搶前景寫了一整套
`force_foreground`／`AttachThreadInput`。第二個直接打在 `apply_window_icon()` 上；它走
Win32 `SetClassLongPtrW` 而非 `iconphoto()`，但仍依賴「開一個隱藏 Toplevel 當跳板取
hwnd」，splash 佔住 primary window 後會不會歪掉只能實測。

文件把這兩項標在 **onedir** 模式下，onefile 記載的問題較少，但不能假設不會發生。

**實作順序上，這兩項排在最前面驗**，不等全部寫完才發現。若焦點問題無解，應回頭調整
甚至放棄 C —— 一個會把 overlay 踢到遊戲背後的啟動畫面，代價比它解決的問題更大；此時
A 與 B 仍可獨立保留，啟動時間一樣會從 2.8 秒降到約 2.0 秒。

圖示問題也先定好因應方式，同樣不等實測炸開才臨場想辦法：`apply_window_icon(root)`
（`src/main.py`）在正常路徑上全程跑在 splash 還佔著 primary window 的期間，若實測發現
圖示跑掉，最低成本的修法是在正常路徑 `splash.close()` 之後**立刻重呼一次
`apply_window_icon(root)`**——splash 交還 primary window 後再設一次類別圖示即可，不必
回頭放棄整個 C。這段補呼叫**先不寫進程式碼**：圖示問題目前只是文件記載的已知風險、
本專案尚未實測到，搶先加一個未觀察到的 bug 的 workaround 是預先設計，違反 YAGNI；只有
實機驗證真的看到圖示跑掉時才動手補這一行。把因應方式先寫進 spec，是讓那次驗證結束在
「照著做」，而不是臨時展開一輪除錯。

## 後續追蹤

- **發布說明要揭露 A 的收益不對稱**：本文件已在「A. 延遲 import `anthropic`」一節記錄
  Claude 官方 provider 的使用者實際上省不到這 0.68 秒（成本只是從 import 階段挪到
  `build_app()`），但目前的實作計畫沒有任何任務涵蓋「把這件事寫進發布說明」，本專案
  也還沒有 `CHANGELOG`。下一次發布本次改動時，release note 需誠實描述這個差異，
  不能讓 Claude 官方 provider 的使用者誤以為自己也省下了 0.68 秒；記在這裡以免被忘記。
