# Wizard101 聊天翻譯助手：版本更新檢查與「關於」分頁設計

日期：2026-08-31
狀態：已與使用者逐項確認

## 目標

1. **啟動時自動檢查 GitHub 上有沒有新版本**，有的話在疊加視窗顯示一條可點擊的更新橫幅，點擊開瀏覽器到 Releases 頁。
2. 設定視窗新增「關於」分頁，提供**手動檢查更新**按鈕，並集中顯示版本、專案連結、開發者、紀錄檔位置。

檢查是純粹的「告知」：本工具不下載、不自我更新，使用者自行到 Releases 頁下載新的 exe 覆蓋舊的。

## 非目標

- 自動下載／自動安裝／自動重啟（風險高、與單一 exe 分發方式不合）。
- 定時輪詢（常駐期間反覆檢查）。每次啟動查一次已足夠。
- 「關閉自動檢查」的設定開關（YAGNI；橫幅本身可關，且不打斷操作）。
- 預發行（pre-release）版本的提醒。
- 更新說明（release notes）的內文顯示——橫幅只給版本號與連結。

## 決策摘要

| 決策點 | 結論 |
|---|---|
| 通知形式 | 疊加視窗**橫幅**（不搶焦點，啟動時使用者可能正在遊戲中） |
| 與既有錯誤橫幅的關係 | **獨立一列並存**，互不覆蓋（兩者生命週期完全不同） |
| 提醒頻率 | **每次啟動都提醒**，不記狀態、config 不新增欄位；使用者點 ✕ 只關掉這一次 |
| 版本來源 | GitHub API `/repos/{repo}/releases/latest`（自動排除 draft 與 pre-release） |
| 尚無 release（HTTP 404） | 視為「已是最新版」，靜默、只留 log。目前 repo 未公開、未發版正是這個狀態 |
| HTTP 客戶端 | 既有相依 `httpx`，timeout 10 秒 |
| 版本比較 | 三段數字 SemVer 比較；任一邊解析失敗就不提醒 |
| 手動檢查的非同步做法 | 沿用 `ui/fields.py` 既有的「背景執行緒 ＋ `poll_queue`」模式，不另造 |

---

## 一、`src/updater.py`（新模組）

單一職責＝**回答「有沒有比目前這版更新的 release」**。不碰 UI、不碰 config。

### 公開介面

| 名稱 | 行為 |
|---|---|
| `Release`（frozen dataclass） | `version: str`（已去掉 `v` 前綴，如 `"0.2.0"`）、`url: str`（該 release 頁網址） |
| `UpdateCheckError` | 檢查失敗（連線不通、配額用盡、回應無法解析）。**「沒有新版」不走例外** |
| `parse_version(text) -> tuple[int, int, int] \| None` | 純函式；解析不出來回 `None` |
| `is_newer(latest, current) -> bool` | 純函式；任一邊解析失敗回 `False` |
| `fetch_latest_release(client=None) -> Release \| None` | 打 API；查無 release（404）回 `None`，其他失敗拋 `UpdateCheckError` |
| `check_for_update(current=__version__, client=None) -> Release \| None` | 上面兩者的組合：回「比目前新的 release」，否則 `None` |

模組常數（連結的單一真實來源，UI 端一律引用、不各自寫死字串）：

```python
GITHUB_REPO  = "GoneTone/wizard101-chat-translator"
PROJECT_URL  = f"https://github.com/{GITHUB_REPO}"
RELEASES_URL = f"{PROJECT_URL}/releases/latest"
AUTHOR_URL   = "https://github.com/GoneTone"
```

### 請求細節

- 端點：`https://api.github.com/repos/{GITHUB_REPO}/releases/latest`
- 標頭：`Accept: application/vnd.github+json`、`User-Agent: wizard101-chat-translator/{__version__}`（GitHub API 要求帶 UA，缺了會被拒）
- timeout：10 秒。啟動路徑不等它，慢一點無妨；但也不能讓手動檢查按鈕卡到天荒地老。
- 不帶任何認證。未認證配額為每 IP 每小時 60 次，本工具一次啟動只打一次，綽綽有餘。
- `client` 參數是為了測試注入假 client（沿用 `translator.py` 同名參數的既有慣例）。

### 回應處理

| 情況 | 行為 |
|---|---|
| 200 | 取 `tag_name` 與 `html_url` 組 `Release`；`tag_name` 去掉 `v` 前綴 |
| 404 | 回 `None`（尚無 release，或 repo 尚未公開）；log 一行，不當錯誤 |
| 其他狀態碼（403 配額、5xx…） | 拋 `UpdateCheckError`，訊息帶狀態碼 |
| 連線失敗（`httpx.HTTPError`） | 拋 `UpdateCheckError`，訊息帶例外內容 |
| JSON 壞掉或缺 `tag_name` | 拋 `UpdateCheckError` |

### 版本比較

`parse_version` 以 `^v?(\d+)\.(\d+)\.(\d+)` 抓三段整數，後綴（`-beta.1`、`+build`）一律忽略；抓不到回 `None`。`is_newer` 只在兩邊都解析成功、且 latest 的 tuple 嚴格大於 current 時回 `True`——**寧可漏提醒也不要誤報**，畢竟橫幅會把使用者導去下載頁。

### log

```
[update] checking latest release
[update] latest=0.2.0 current=0.1.0 newer=True
[update] no release published yet (404)
[update] check failed: status=403 ...
[update] check failed: ...（連線層例外訊息）
```

---

## 二、啟動時自動檢查（`src/main.py`）

reader 執行緒啟動之後，多開一條 daemon 執行緒：

```python
def update_check_worker() -> None:
    try:
        release = check_for_update()
    except UpdateCheckError as exc:
        print(f"[update] check failed: {exc}", file=sys.stderr)
        return
    if release is not None:
        ui_queue.put(lambda: overlay.set_update(release))
```

- 走既有的 `ui_queue`，不從背景執行緒碰 tkinter（與 reader 執行緒同一套規則）。
- 例外全數吞在這條執行緒裡：更新檢查失敗**絕不能影響啟動或收訊**。
- daemon：關閉程式時不等它，卡在連線中也不會拖住結束流程。

## 三、更新橫幅（`src/reader/overlay.py`）

新增與 `set_error`／`clear_error` 對稱的一組方法：

| 方法 | 行為 |
|---|---|
| `set_update(release)` | 顯示更新橫幅（存下 `release` 供換語言時重繪） |
| `clear_update()` | 移除橫幅並清掉狀態 |
| `update_text()` | 測試／除錯輔助，回目前橫幅文字或 `None`（對齊既有的 `error_text()`） |

版面：自己一列 `tk.Frame`，`pack(side="bottom", fill="x", before=self._scroll_area)`，與錯誤橫幅同樣排在 `expand=True` 的捲動區之前（視窗縮小時不會被擠掉）；兩條橫幅各佔一列、同時存在時上下並排。

外觀與互動：藍字（連結色，沿用 `fields.py` 的 `#4a7ddc`）、`cursor="hand2"`，文字為 `t("update.available", version=...)`，點擊 → `webbrowser.open(release.url)`；右側一個 `✕` 標籤，點了呼叫 `clear_update()`（只關掉這一次，不寫任何狀態）。

`refresh_labels()` 在 `_error_key` 之後一併處理：`self._update_release` 不是 `None` 就重新 `set_update(...)`，讓換語言後的橫幅跟著翻譯。

## 四、設定視窗「關於」分頁（`src/ui/settings.py`）

在「基本」「進階」之後新增第三個分頁（同樣包在 `ScrollableFrame` 裡），由上而下四塊：

| 區塊 | 內容 |
|---|---|
| 版本 | `v{__version__}` ＋「檢查更新」按鈕 ＋ 按鈕右側的結果文字 |
| 專案 | GitHub 專案頁連結（`PROJECT_URL`） |
| 開發者 | `GoneTone`，可點，連 `AUTHOR_URL` |
| 紀錄檔 | `app_dir()` 路徑文字 ＋「開啟資料夾」按鈕（`os.startfile(app_dir())`） |

底部按鈕列既有的 `v{__version__}` 小字**維持不動**：那是任何分頁下都看得到的版本標示，關於分頁的版本列則是檢查更新的入口，兩者用途不同。

### 手動檢查的四種狀態

沿用 `ApiFields` 測試連線的節奏（按鈕 disable ＋ 換文字，worker 丟 queue，`poll_queue` 在主執行緒收）：

| 狀態 | 呈現 |
|---|---|
| 檢查中 | 按鈕 disabled、文字換成 `button.checking`；結果文字清空 |
| 已是最新版 | 綠色 `✓ ` ＋ `update.latest`（`check_for_update` 回 `None`，含尚無 release 的情況） |
| 有新版 | 藍色 `update.available`（帶版本號），可點 → 開 Releases 頁 |
| 檢查失敗 | 紅色 `✗ ` ＋ `update.failed`（帶例外訊息，讓使用者看得出是斷網還是配額） |

顏色沿用既有值（成功 `#2e8b57`、失敗 `#cc3333`、連結 `#4a7ddc`）。視窗在等待期間被關掉時 `poll_queue` 自己會停、結果丟棄，不需額外處理。

換語言重建視窗（`_rebuild`）時，關於分頁不保留檢查結果——與 API 測試結果同理，重建即清空；分頁索引本來就由 `_restore_tab` 記住，使用者仍會回到關於分頁。

## 五、連結標籤共用元件

`fields.py` 目前把「取得金鑰」連結寫成 inline 的三行（藍字 ＋ `hand2` ＋ `webbrowser.open`）。關於分頁還要用兩次（專案、開發者），因此抽成共用元件，放在 `src/ui/fields.py`（精靈與設定視窗共用元件的既有所在）：

```python
def link_label(parent, text: str, url: str) -> tk.Widget:
    """藍字可點的連結標籤：點擊以系統瀏覽器開啟 url。"""
```

並把 `fields.py` 既有的「取得金鑰」連結改用它，避免同一段樣式寫第三遍。

## 六、i18n

三份語言檔（`zh-TW` 為來源語言）各補以下 key，`tests/test_i18n.py` 會擋掉漏翻與變數打壞：

| key | 用途 |
|---|---|
| `settings.tab.about` | 分頁標題「關於」 |
| `about.version` | 版本列標籤 |
| `about.project` | 專案列標籤 |
| `about.author` | 開發者列標籤 |
| `about.logs` | 紀錄檔列標籤 |
| `about.logs_hint` | 紀錄檔用途一句話說明（回報問題時附這兩份檔） |
| `button.check_update` | 「檢查更新」 |
| `button.checking` | 「檢查中…」 |
| `button.open_folder` | 「開啟資料夾」 |
| `update.latest` | 「已是最新版本」 |
| `update.available` | 「有新版本 v{version}，點此下載」（帶 `{version}`；橫幅與設定視窗**共用同一則**，兩處文案相同，不重複定義） |
| `update.failed` | 「檢查更新失敗：{error}」 |

開發者名稱 `GoneTone` 與各網址是識別碼而非文案，**不進語言檔**（與 `provider.openai`／`provider.claude` 是品牌名不進語言檔同理）。

## 七、測試

| 檔案 | 內容 |
|---|---|
| `tests/test_updater.py`（新） | `parse_version` 邊界（`v` 前綴、後綴、非法字串、位數不足）；`is_newer` 的大於／相等／小於／解析失敗；`fetch_latest_release` 以假 client 測 200／404／403／`httpx.HTTPError`／JSON 缺欄位；`check_for_update` 的組合行為 |
| `tests/test_overlay.py` | 補 `set_update`／`clear_update`／兩條橫幅並存／換語言後橫幅重繪 |
| `tests/test_settings.py` | 補關於分頁存在、版本文字、檢查更新按鈕與四種結果的呈現（注入假 checker） |
| `tests/test_no_hardcoded_ui_text.py` | 既有掃描自動涵蓋新增的 UI 文字 |

所有測試離線可跑：網路一律以假 client／假 checker 注入，不打真的 GitHub API。

## 八、文件

- README「一般使用者」段補一句：啟動時會自動檢查更新，有新版時疊加視窗會顯示可點的橫幅；也可在設定視窗的「關於」分頁手動檢查。
- README「放版」段補一句：**要在 GitHub 上建立 Release**（不只推 tag）新版才會被偵測到。

## 驗收條件

1. `uv run pytest` 全綠。
2. 目前狀態（GitHub 上尚無任何 release）啟動：**不出現任何橫幅**，`app.log` 有 `[update] no release published yet` 之類的一行。
3. 關於分頁手動檢查：目前狀態顯示「已是最新版本」；把 `check_for_update` 暫時指向一個確實有 release 的公開 repo 或注入假回應時，顯示可點的新版提示，點擊會開瀏覽器。
4. 斷網啟動：不影響啟動與收訊，`app.log` 留下 `[update] check failed: ...`。
5. 更新橫幅與「遊戲未就緒」橫幅可同時出現、各自可見。
