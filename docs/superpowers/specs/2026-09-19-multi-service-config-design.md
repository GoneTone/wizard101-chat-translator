# Wizard101 聊天翻譯助手：多組翻譯服務設定設計

日期：2026-09-19
狀態：設計完成，待實作。

## 目標

把目前「ChatGPT／Claude／自訂端點各保留一份設定、同時只有一份生效」的模型設定，改成
**可自由新增多組具名翻譯服務**的清單（兩組 ChatGPT、三組 Claude、一組自訂端點都可以），
並讓使用者快速切換預設服務、必要時為個別用途指定不同服務。

1. **服務是清單而非固定三格**：點「新增翻譯服務」→ 選服務商 → 填設定 → 存檔成一筆。
2. **一組預設服務**，聊天收訊、聊天發話、區域翻譯預設全部跟著它走。
3. **每個用途可單獨指定**另一組服務（例如收訊用便宜快的、區域翻譯用強的），預設值是
   「跟隨預設」—— 不想分的人永遠只需要管一組。
4. **舊設定自動遷移**，使用者已填的金鑰與模型一個都不能掉。

## 非目標

- **不做自動備援**：主服務失敗不自動退到下一組。失敗判定、重試策略與「現在其實在用哪
  一組」的顯示都要另外設計，本案不碰。
- **不在疊加視窗的標題列加快速切換入口**：切換一律走設定視窗的「翻譯服務」分頁。
- **不做服務排序拖曳、不做「複製一份」**。
- **不改翻譯提示詞、不改並行度模型**：`max_parallel_translations` 維持全域一份，三格
  共用同一個 `ConcurrencyGate` —— 那是保護本機與帳號額度的總閘，不因分了服務而變三倍。
- **不動 wizwalker 掛入與 reader**：這條資料流與服務設定無關。

## 決策摘要

| 決策 | 選擇 | 理由 |
|---|---|---|
| 資料模型 | `api` 區塊退場，換成 `services` 清單 ＋ `default_service` ＋ `service_slots` | 單一資料模型；保留舊區塊會讓 UI 與查找都得寫兩套特判 |
| 分派的指向 | 服務的 `id`（隨機 8 碼 hex） | 改名、刪除中間一筆、調整順序都不會誤傷分派 |
| 用途插槽 | 收訊／發話／區域三格 | 使用者指定；發話固定翻英文、短句，值得單獨指派便宜模型 |
| 未指定的表示 | `null` ＝跟隨預設 | 與「剛好指到同一筆」區分得開，log 也讀得出差別 |
| 查找入口 | `services.resolve(cfg, slot)` 唯一一處 | 呼叫端不必各自判斷 `null` 怎麼解 |
| 執行期實例 | 三個 `Translator`，不去重 | 三個 client 成本可忽略；去重會帶來別名共享問題，`apply_settings` 反而複雜 |
| 服務商設定何處存 | `services.py`（由 `ui/providers.py` 搬入） | 它本來就不碰 tkinter，放在 `ui/` 下是歷史包袱 |
| 清單 UI | 獨立「翻譯服務」分頁 ＋ 子對話框編輯 | 基本分頁已有介面語言、目標語言、兩把熱鍵，再塞清單會過長；對話框讓精靈與設定共用同一份表單 |
| 用途分派擺法 | 摺疊區塊，預設收合 | 多數人用不到；攤開四個長相一樣的下拉會讓人分不清哪個才是主要的 |
| 名稱去重 | 按下〔確定〕時一律檢查，撞名補 ` (2)` | 不必特判新增或編輯；分派下拉只顯示名稱，同名會直接讓人選錯 |
| 序號括號 | 一律半形，寫死不進語言檔 | 使用者指定 |

## 資料模型

### config.json

```json
"services": [
  {"id": "1a2b3c4d", "name": "ChatGPT（快）", "provider": "openai",
   "model": "gpt-5-mini", "api_key": "…", "thinking": false},
  {"id": "5e6f7a8b", "name": "Claude（強）", "provider": "claude",
   "model": "claude-opus-5", "api_key": "…", "effort": "auto"}
],
"default_service": "1a2b3c4d",
"service_slots": {"incoming": null, "outgoing": null, "region": "5e6f7a8b"}
```

- 每筆服務 ＝ `id` ＋ `name` ＋ `provider` ＋ 該服務商在 `API_PROFILE_FIELDS` 裡的欄位。
  欄位表仍是「服務商有哪些欄位」的單一真實來源，原本 `_prune_profiles` 的清理邏輯改成
  逐筆套用。
- `id` 只用來被 `default_service` 與 `service_slots` 指向，不在介面上露臉。
- `service_slots` 的 `null` ＝跟隨預設。
- 預設值：`services: []`、`default_service: null`、三格全 `null`（等同現在三家都空白的
  初始狀態，`is_configured()` 為 False 時照舊進首次設定精靈）。

### 模組配置

新增 `src/services.py`，並把 `src/ui/providers.py` 整個搬進來（`Provider`、`PROVIDERS`、
`validate_endpoint_fields`），`API_PROFILE_FIELDS` 與 `needs_base_url` 也從 `config.py`
移過來。`src/ui/providers.py` 刪除。原本的 `validate_api_form(api)` 更名為
`validate_service(service)` —— 服務 dict 比攤平的 api dict 只多了 `id` 與 `name`，檢查內容
一模一樣，留兩支只會讓呼叫端猶豫該用哪一支。

搬完後 `services.py` 的職責是「服務是什麼、有哪些、怎麼找」：

| 名稱 | 作用 |
|---|---|
| `PROVIDERS` / `Provider` | 服務商的顯示資料：標籤 key、短名、申請金鑰連結、有哪些欄位 |
| `API_PROFILE_FIELDS` | 各服務商的欄位與預設值 |
| `SLOT_INCOMING` / `SLOT_OUTGOING` / `SLOT_REGION` / `SLOTS` | 用途插槽的常數 |
| `new_service(provider, existing)` | 產生一筆新服務（新 id、自動命名、欄位預設值） |
| `unique_name(name, existing, ignore_id=None)` | 撞名補序號 |
| `resolve(cfg, slot)` | 取該插槽實際生效的服務（攤平副本，含 `provider`） |
| `validate_endpoint_fields(api)` | 連上端點所需的欄位檢查（不含模型），取模型清單用 |
| `validate_service(service)` | 必填欄位檢查（含模型），回傳錯誤文案 key 列表 |
| `normalize(cfg)` | 遷移 ＋ 補值 ＋ 清除過期欄位 ＋ 校驗，回傳是否有變動 |

`config.py` 回歸單純的 JSON 讀寫與其他欄位補值，載入時呼叫一次 `normalize(cfg)`；回傳
有變動就沿用既有機制把整理後的結果重寫回 config.json。

`active_api(cfg)` 由 `resolve(cfg, slot)` 取代，兩者回傳同形（攤平、含 `provider`），
所以 `test_translate(api, target)`、`list_models(api)`、`Translator(**api, …)` 的簽名
都不必動。

### 名稱規則

- `name` 由使用者自訂，是清單、預設下拉與三格分派下拉唯一顯示的字。
- 在編輯對話框選好服務商時，名稱欄自動填該家短名（`ChatGPT`／`Claude`／自訂端點）。
  名稱欄**若使用者還沒手動編輯過**，改服務商時跟著重新生成；動過就不再覆蓋 —— 與精靈裡
  「翻譯目標語言跟隨介面語言、動過就不跟」是同一個既有模式。
- 按下〔確定〕時一律與其他筆比對，撞名就補序號：第一筆無後綴，第二筆起 `ChatGPT (2)`、
  `ChatGPT (3)`。括號一律半形、寫死在 `services.py`，不進語言檔。
- 名稱清空不跳錯誤對話框，退回自動生成的名稱。

### 遷移

`normalize()` 內兩段串接，兩段都留 log：

1. **扁平 `api` → per-provider**：現有的 `_is_legacy_api()` 與 `_migrate_api()` 原樣保留。
2. **per-provider → `services`**：三家中 `model`／`api_key`／`base_url` 任一非空的各轉成
   一筆，名稱帶服務商短名（遷移當下的介面語言；名稱是一次性生成的資料值，日後換語言
   不會、也不該跟著變）。原本 `api.provider` 選中的那家設為 `default_service`，三格全
   `null`。三家都沒填過 → 空清單。

遷移後刪掉舊的 `api` 區塊並立刻重寫 config.json，不讓兩份格式並存。

### 校驗

手改 config.json 的防線，每條都留 log：

- 認不得的 `provider` → 整筆丟棄。
- 缺欄位 → 依 `API_PROFILE_FIELDS` 補預設；多餘欄位 → 刪除。
- 重複或缺少的 `id` → 重新產生。
- `default_service` 指向不存在 → 退回清單第一筆（清單空則 `null`）。
- `service_slots` 某格指向不存在 → 退回 `null`。
- `service_slots` 出現不認得的鍵 → 刪除。

`is_configured(cfg)` 改成「`default_service` 指得到服務，且該筆通過 `validate_service()`」。

## UI

分頁順序：基本 ｜ **翻譯服務** ｜ 進階 ｜ 關於。原本內嵌在「基本」的 `ApiFields` 整塊
搬走，基本分頁剩介面語言、翻譯目標語言、兩把熱鍵。

### 版面

```
[基本] [翻譯服務] [進階] [關於]
──────────────────────────────────
 預設服務：[ ChatGPT（快）      ▾ ]

 ▸ 各用途指定不同服務（進階）
      聊天收訊：[ 跟隨預設      ▾ ]
      聊天發話：[ 跟隨預設      ▾ ]
      區域翻譯：[ Claude（強）  ▾ ]

 我的服務
 ┌──────────────────────────────┐
 │ ● ChatGPT（快）               │
 │   ChatGPT · gpt-5-mini        │
 │                  〔編輯〕〔刪除〕│
 ├──────────────────────────────┤
 │   Claude（強）                │
 │   Claude · claude-opus-5      │
 │                  〔編輯〕〔刪除〕│
 └──────────────────────────────┘
          〔 ＋ 新增翻譯服務 〕
```

### 新模組

**`src/ui/service_form.py` → `ServiceForm(ttk.Frame)`**

單筆服務的編輯表單：名稱欄、服務商下拉、依 `API_PROFILE_FIELDS` 動態生成的欄位、
測試連線。這是現在 `ApiFields` 瘦身後的樣子：

- `_profiles`／`_switch_profile`／`_last_provider` 那套「跨服務商保留值」的機制整組刪掉
  —— 每一筆本來就只屬於一家。
- 服務商由三顆 radio 改成 readonly `Combobox`。
- `ModelField`、測試連線、`set_target_language_fn()` 原樣沿用。

**精靈與對話框共用同一個 `ServiceForm`**：精靈直接內嵌它，不開對話框。

**`src/ui/service_list.py` → `ServicePane(ttk.Frame)`**

分頁內容，外加新增／編輯用的 modal 包裝（`Toplevel` ＋ `wait_window`，回傳服務 dict 或
`None`）—— 它只有這一個呼叫者，獨立成檔太碎。

- **預設服務**：readonly `Combobox`，選項是各服務的 `name`。
- **用途分派**：摺疊區塊內三個 readonly `Combobox`，選項是「跟隨預設」＋各服務名。
- **卡片清單**：`ttk.Frame` ＋ 分隔線，一行粗體名稱、一行灰字 `服務商 · 模型`、右側
  〔編輯〕〔刪除〕。預設那張名稱前綴 `●`，其餘留等寬空位對齊。整份放在既有的
  `ScrollableFrame` 裡。

`src/ui/fields.py` 只留 `HotkeyField`／`LanguageField`／`UiLanguageField`。

**`form.py` 新增 `collapsible(parent, text)`**：Tk 沒有內建摺疊區塊，寫一個放進共用小元件
的既有落腳處，回傳 body frame。三格皆「跟隨預設」時收合；任一格被指定過則開窗即展開
（否則設定會藏在摺疊裡看不見）。

### 互動

- **新增**：〔＋ 新增翻譯服務〕開對話框，服務商預設第一家、名稱自動帶入。確定後加入清單
  尾端；若原本清單是空的，新筆自動成為預設。
- **編輯**：對話框帶入該筆現值。服務商可改，改了欄位跟著換、舊值不保留；`id` 不變，
  所以指向它的預設與分派都不受影響。
- **刪除**：`messagebox.askyesno` 確認。被 slot 指到的格子退回「跟隨預設」，刪到的是預設
  就由清單第一筆接手，兩者都留 log。**最後一筆的〔刪除〕停用** —— 刪光就無從翻譯，擋在
  按鈕比擋在儲存清楚。
- **驗證時機**：對話框按〔確定〕跑 `validate_service()`（必填欄位），不強制測試連線通過
  —— 與設定視窗現行行為一致；精靈那條路仍強制測過或明示略過。
- **未儲存的編輯**：`ServicePane` 自己持有一份 draft 的服務清單，對話框回傳的結果進 draft，
  不直接寫 `cfg`；按〔儲存〕才落地。設定視窗的 `_collect_into_draft()` 要把它一起收進去，
  換介面語言重建視窗時才不會丟掉剛加到一半的服務。

### 精靈

第二步標題改為「建立翻譯服務」，內嵌 `ServiceForm`；完成時寫入單筆服務、設為
`default_service`、三格全 `null`。精靈不做清單管理。

## 執行期接線

`build_translation(cfg, deliver)` 改為回傳 `dict[str, Translator]`（不再由呼叫端先算好
一份攤平的 `api` 傳進來 —— 三格各有各的，攤平一份已無意義）：

```python
translators = {slot: Translator(**resolve(cfg, slot),
                                target_language=cfg["target_language"])
               for slot in SLOTS}
```

接線改動三處：

- 兩條翻譯池（玩家對話、系統訊息）吃 `translators[SLOT_INCOMING]`。
- `InputBox` 吃 `translators[SLOT_OUTGOING]`。
- `RegionPipeline` 吃 `translators[SLOT_REGION]`。

預設情況下三格指向同一筆服務，仍各建一個 client —— 刻意的取捨，換來 `apply_settings`
只是一個迴圈、沒有「這兩格其實是同一個物件」的別名問題：

```python
for slot, tr in translators.items():
    tr.reconfigure(**resolve(cfg, slot), target_language=cfg["target_language"])
```

`reconfigure()` 本來就會收掉舊 client，關閉路徑（`shutdown` 最後 `os._exit`）不必動。

**快取**：`TranslationCache` 服務的是系統訊息，屬收訊，指紋固定綁
`resolve(cfg, SLOT_INCOMING)` 的 `provider`／`model` ＋目標語言。`apply_settings` 裡的
`cache.rebind(...)` 跟著改讀收訊那格 —— 只改了區域翻譯用哪家時，收訊的譯文快取不該被
作廢。

**啟動摘要**：`config_summary()` 改成印三格的實效服務，方便使用者匯出 `app.log` 後一眼
看出誰在跑哪家。**金鑰絕不入 log**，沿用既有的 `has_key=` 慣例：

```
[app] startup; …, services=2, default=ChatGPT(openai/gpt-5-mini,has_key=true),
      incoming=default, outgoing=default, region=Claude(claude/claude-opus-5,has_key=true), …
```

`slot=default` 明確表示「跟隨預設」，與「剛好指到同一筆」區分得開。

## i18n

新文案約 12–15 個 key：分頁名、預設服務、摺疊區標題、三個插槽標籤、跟隨預設、我的服務、
新增／編輯／刪除、刪除確認、自訂端點短名。

- 寫進 `zh-TW.json`（來源語言）、`zh-CN.json`、`en-US.json` 三份；其餘 28 種語言交給
  Crowdin，缺字串依既有的 fallback 鏈退到英文。
- `ChatGPT`／`Claude` 是品牌名，短名寫死在 `PROVIDERS`，不進語言檔；只有「自訂端點」
  需要一個新的短名 key（現有的 `provider.custom` 是「自訂端點（OpenAI API 相容）」，
  當服務名稱太長）。

## 測試

### 新增 `tests/test_services.py`（純資料層，不開 Tk、不需遊戲）

- **遷移三段串接**：直接餵最舊的扁平格式，斷言一路走到新 schema 且金鑰沒掉。
- **遷移的取捨**：三家都空 → 空清單；只填過一家 → 單筆且為預設；填過兩家但選中的是另一家
  → 兩筆、預設是原本選中那家。
- **校驗防線**逐條：認不得的 provider 丟棄、重複 id 重生、`default_service` 失效退第一筆、
  slot 失效退 `null`。
- **`resolve()`**：`null` 取到預設、指定則取該筆、回傳是副本（改它不回頭污染 cfg）。
- **名稱去重**：`ChatGPT` → `ChatGPT (2)` → `ChatGPT (3)`；清空名稱退回自動值。

### 測試輔助

`tests/` 裡約 40 處 `cfg["api"]["custom"].update(base_url=…, model=…)` 只是為了「弄一份
設定完整的 cfg」。在既有的 `tests/conftest.py` 加 `configured_cfg(provider="custom", **fields)`
並全部換過去 —— 之後再改 schema 只要動一處。

### 既有測試的調整

| 檔案 | 調整 |
|---|---|
| `tests/test_fields.py` | `ApiFields` 那批搬成 `tests/test_service_form.py` |
| `tests/test_service_list.py`（新增） | 新增／編輯後 draft 內容、刪除預設由第一筆接手、刪除被指派的服務讓該格退回 `null`、最後一筆刪除鈕停用、三格皆預設時摺疊區收合 |
| `tests/test_config.py` | api 段改寫為 services 段 |
| `tests/test_main.py` | `config_summary` 的斷言改成三格格式 |
| `tests/test_wizard.py` | 配合精靈第二步的新表單 |
| `tests/test_no_hardcoded_ui_text.py` | `SCAN_TARGETS` 移除 `src/ui/providers.py`，補上 `src/services.py`、`src/ui/service_form.py`、`src/ui/service_list.py` |

### 實機驗證

本案沒有動到 wizwalker 掛入，純解析與 UI 測試覆蓋主體；但依提交前檢查，最後仍要開著遊戲
（登入進世界內）`uv run run.py` 跑一次，確認收訊與區域翻譯確實各自走到指定的服務 ——
用兩組明顯不同的模型最容易看出來。
