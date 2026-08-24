# Wizard101 聊天翻譯助手：收訊平行翻譯設計

日期：2026-08-24
狀態：已與使用者逐節確認

## 目標

收訊翻譯目前是嚴格序列（`main.py` 的 `while pending:` 一次翻一則、等回來才翻下一則），造成三個問題：

1. **單則卡住會堵住整條佇列**——這是最主要的痛點。一則翻譯逾時，其後所有訊息都在等它。
2. 聊天熱鬧、多人同時說話時譯文落後。
3. 伺服器的批次處理能力閒置。

本設計以工作池取代序列迴圈，並**維持顯示順序與上下文品質不變**。

## 前提實測（2026-08-24，自架端點 `gemma-4-26b-a4b`）

| 情境 | 總時間 | 單則中位數 |
|---|---|---|
| 序列 ×4 | 3.3 秒 | 0.8 秒 |
| 平行 ×4 | 1.9 秒 | 1.9 秒 |
| 序列 ×8 | 6.7 秒 | 0.8 秒 |
| 平行 ×8 | 3.6 秒 | 2.8 秒 |

結論：

- 後端有 continuous batching，平行確實有效，但**加速比上限約 1.8 倍**，並非線性。
- **4 併發已接近飽和**，8 併發幾乎沒有額外收益，只讓單則延遲更差。
- 平行會讓**單則延遲變差**（0.8 → 1.9 秒）。零星訊息（一次一則）時平行度本來就是 1，行為與現在完全相同。

因此本設計的主要價值是**消除單則阻塞**，吞吐提升是次要收穫。開啟 thinking 時單則需 15～28 秒，1.8 倍的加速救不了它——那需要換模型或關閉 thinking，不在本設計範圍。

## 決策摘要

| 決策點 | 結論 |
|---|---|
| 併發機制 | `ThreadPoolExecutor`（標準庫），不自行實作工作池 |
| 顯示順序 | **佔位法**：訊息一讀到就先顯示原文，譯文稍後填入；不需要重排序緩衝 |
| 上下文 | 由 reader 讀取順序推進，不再由「翻譯成功」驅動；平行不損失上下文 |
| 退避 | **全域閘門**（gate），沿用現有 `BACKOFF_STEPS` 5→15→30 秒 |
| 重試 | offline／config error 無限重試；`TranslatorBadOutput` 與其他例外不重試 |
| 平行度 | config 可設，範圍 1–8，預設 4；設 1 完全等同現行序列行為 |

### 上下文不會因平行而流失

一個容易誤判的點：`Translator._history` 存的是**原文**而非譯文（`translator.py:236`），而 reader 讀到新行的當下就已知道它們的完整順序。只要在送出翻譯請求**前**就依序把行推進上下文，每一則都看得到它之前的所有行，即使那些還在翻譯中。因此「平行會讓同批訊息互相看不見」的疑慮不成立。

## 1. 元件與職責

平行化的難點不在開執行緒，而在於現況有三者相黏：`Translator` 自己藏著 history、`reader_loop` 同時扮演讀取者與翻譯者、overlay 只接受已完成的訊息。拆解如下：

```
src/
├── context.py           # 新增：ChatContext，執行緒安全的近期原文行緩衝
├── translation_pool.py  # 新增：TranslationPool，工作池＋退避閘門＋重試
├── translator.py        # 調整：translate_incoming/outgoing 明確吃 context 參數
├── main.py              # 調整：reader_loop 瘦身為「讀取→佔位→派發」
├── config.py            # 調整：新增 max_parallel_translations
├── ui/settings.py       # 調整：進階分頁新增對應欄位
└── reader/overlay.py    # 調整：新增 update_message 佔位填入 API
```

**`ChatContext`（新）**
執行緒安全的近期原文行緩衝，介面為 `push(line)` 與 `snapshot()`，內部持鎖，容量沿用 `CONTEXT_LINES`。獨立成元件的理由：它會同時被 reader 執行緒、多個 worker、以及 `InputBox` 的發話執行緒存取。後者現況就已跨執行緒讀 `_history`（`input_box.py:106` 的 `_worker`），只是沒有鎖——本次順帶修掉這個既有的資料競爭。

**`Translator`（改介面）**
`translate_incoming(text, context)` 與 `translate_outgoing(text, context)` 明確吃 context 參數，不再自行維護狀態。翻譯後端本就無狀態，改完即可安全平行呼叫，也更容易測試。

**`TranslationPool`（新）**
包一層 `ThreadPoolExecutor`，負責派發、退避閘門、per-line 重試，並透過回呼送出結果。獨立成元件是因為錯誤處理邏輯量不小（見第 3 節），留在 `reader_loop` 會讓它再次成為大雜燴，且無法單獨測試。

**`OverlayWindow`（加 API）**
`add_message(...)` 接受呼叫端指定的 `msg_id`，新增 `update_message(msg_id, translated)` 就地填入譯文。找不到該 id（訊息已被 `prune` 或 `max_messages` 擠掉）即安靜忽略。

**`reader_loop`（瘦身）**
退回只做：讀新行 → 依序推進 context → overlay 佔位 → 提交給 pool → 維護狀態指示與錯誤橫幅。不再持有 `pending` 佇列，也不再直接呼叫 translator。

## 2. 資料流

### 收訊路徑

reader 執行緒每輪讀到新行後，對每一行依序執行：

```
ctx    = context.snapshot()          # 該行「之前」的行，不含自己
context.push(line)                   # 推進，供後續行使用
msg_id = next(counter)               # reader 執行緒自行發號
ui_queue.put(lambda: overlay.add_message(line, PENDING_NOTICE, msg_id=msg_id))
pool.submit(line, ctx, msg_id)
```

其中 `PENDING_NOTICE` 為佔位期間顯示於譯文位置的文字「翻譯中…」，與既有的
`TRANSLATE_FAILED_NOTICE`（「⚠  這則訊息翻譯不出來」）同置於 `main.py`。

worker 翻完（或放棄）後只做一件事：

```
on_result(msg_id, 譯文或失敗提示)   # 由 main.py 接成 ui_queue.put(...)
```

三個關鍵點：

**msg_id 由 reader 執行緒發號**，而非 overlay 回傳——`ui_queue` 是單向投遞、拿不到回傳值。順序仍然安全：`add_message` 必定比對應的 `update_message` 更早進佇列（後者要等翻譯完成），而 `ui_queue` 是 FIFO 且都在主執行緒依序執行。

**上下文在提交當下即固定。** 每則帶著自己的 snapshot 進 pool，重試時沿用同一份，不因期間湧入新訊息而漂移。這同時取代了現行「成功才寫 history」規則的用途——該規則原是防止重試的行重複出現在自己的上下文裡（`translator.py:236` 註解），新模型下每行只 push 一次，問題自然消失。

**顯示順序由佔位決定**，不需要重排序緩衝。overlay 中的位置在 `add_message` 當下就定案，譯文晚到只是填空。這是「單則卡住不擋後續」與「顯示順序正確」能同時成立的原因。

### 發話路徑

`InputBox._worker` 改為呼叫 `translate_outgoing(text, context.snapshot())`。發話內容一如現況不寫入上下文（送出後遊戲會回顯成聊天行，由收訊路徑記錄）。

### 狀態指示

`pool.in_flight > 0` 顯示「翻譯中…」，否則依 `reader.anchored` 顯示「監聽中」或「連線遊戲中…」。

### 邊界情況

佔位的訊息若在譯文回來前被 `max_messages` 擠掉或被 `prune` 清除，`update_message` 找不到 id 即安靜忽略。預設 `fade_seconds` 為 0（永不依時間清除），故實務上只有訊息量超過 `max_messages`（預設 200）才會遇到。

## 3. 錯誤處理與重試

現行的 `backoff_index` 與 `error_state` 皆為全域單執行緒狀態，平行後不能直接沿用。

### 全域退避閘門

`TranslationPool` 內部維護「下次可送出時間」。worker 動手翻譯前先檢查閘門，未到則等待；任何一次 `TranslatorOffline` 推進閘門（沿用現有 `BACKOFF_STEPS` 5→15→30 秒），任何一次成功則重置閘門與退避級數。

這使兩種情況都正確：伺服器真的離線時，各 worker 各撞一次牆後全部進入等待，不會以併發倍數瘋狂重打；單則偶發失敗時，其他 worker 的成功會立即清除閘門。退避維持全域，因為「伺服器離線」本就是全域事實。

### 三類失敗的處置

| 失敗 | 處置 |
|---|---|
| `TranslatorOffline` | 推進閘門，該則無限重試（伺服器恢復後補上），由閘門節流 |
| `TranslatorConfigError` | 推進閘門至 `CONFIG_ERROR_INTERVAL`（15 秒），無限重試，使用者修正設定後自動恢復 |
| `TranslatorBadOutput` 與其他例外 | **不重試**，直接填入 `TRANSLATE_FAILED_NOTICE` |

第三類沿用既有判斷：`temperature=0` 下重試必得同一結果（2026-08-24 修正 repetition loop 時確立）。

### 職責邊界

pool 不認識 overlay，也不碰 `ui_queue`；它建構時吃一個 `on_result(msg_id, text)` 回呼，由 `main.py` 接成 `ui_queue.put(...)`。橫幅方向相反：pool 只暴露 `in_flight` 與 `error_state`（`None`／`"offline"`／`"config"`），由 `reader_loop` 每輪輪詢並比對變化來掛橫幅、切狀態指示。UI 更新集中於 `reader_loop` 一處，pool 保持純粹、可獨立測試。

### log

進入 offline／config 狀態、放棄某行時寫入 `app.log`，帶 msg_id、重試次數、原文與例外訊息。與現行一致，**只在狀態轉換時印**，避免退避期間洗版。

### 已知取捨

伺服器長時間離線時，工作累積於 pool 佇列，worker 持續重試最舊的數則，新訊息排在其後。此行為與現況一致（現況卡在 `pending[0]`）。這代表「卡住不擋後續」對**單則卡住**成立，對**全伺服器離線**不成立——後者本就無可翻譯之物。

## 4. 設定

沿用既有進階設定的 pattern，不另造機制。

**`src/config.py`**

- `DEFAULT_CONFIG` 新增 `"max_parallel_translations": 4`
- `ADVANCED_LIMITS` 新增 `"max_parallel_translations": (1, 8)`

`clamp_advanced()` 遍歷 `ADVANCED_LIMITS` 做夾限，新增即自動涵蓋；手動編輯 config.json 填入超界值會被拉回。

預設 4 的依據見前提實測：第 5 條併發之後幾乎無收益，只讓單則延遲更差。**設 1 完全等同現行序列行為**，為保底退路。

**`src/ui/settings.py`**

進階分頁新增一列 `_spin`：

```
同時翻譯則數  [ 4 ]  1＝逐則排隊；大於 1 時卡住的訊息不會擋住後續（範圍 1–8，預設 4）
```

`parse_advanced_values()` 多收一個 Tk 變數，其餘照舊。

**套用時機**

`ThreadPoolExecutor` 的 `max_workers` 無法就地變更，故 `apply_settings()` 中若平行度有變動即重建 pool；舊 pool 以 `shutdown(wait=False)` 放生，其手上的工作完成後仍會經 `on_result` 正常填回 overlay（msg_id 不受 pool 重建影響）。此處理層級與現行「改 hotkey 需重新註冊」「改 API 需 `translator.reconfigure()`」相同，不需重啟程式。

**`config.example.json`** 同步補上該欄位。

## 5. 測試策略

全部不需要遊戲、不需要真實 API。

**`ChatContext`**——push／snapshot 的順序與容量上限；多執行緒併發 push 不掉行。

**`Translator`**——介面變更後，既有驗證 `_history` 自動累積的測試（`test_incoming_history_feeds_next_translation`、`test_failed_translation_not_recorded_to_history`、`test_history_caps_at_context_lines`、`test_outgoing_gets_context_but_does_not_record`）改為驗證「傳入的 context 如何組進 turns」。provider 選擇、錯誤映射、`max_tokens`、截斷偵測等測試不動。

**`TranslationPool`**——fake translator 可控制每則的延遲與失敗方式，`BACKOFF_STEPS` 以 monkeypatch 換成極小值（`test_reader_loop.py` 已有此 pattern）。涵蓋：

- **單則卡住不擋後續**：讓 fake 對某則阻塞，驗證其餘各則仍照常完成。這是本需求的直接驗證，最重要的一條
- offline → 閘門推進 → 重試 → 成功後閘門與退避級數重置
- config error → 走 15 秒間隔、無限重試
- `TranslatorBadOutput` → 不重試，直接回失敗提示
- `error_state` 的轉換與 `in_flight` 計數

併發測試以 `threading.Event` 建立同步點，不以 `sleep` 等待時間——後者是 flaky 測試的主要來源。

**`OverlayWindow`**——`update_message` 就地填入譯文；對已被 `prune` 或 `max_messages` 擠掉的 id 安靜忽略。

**順序保證（整合測試）**——讓 fake 故意使後面的則先翻完，驗證 overlay 中的順序仍為讀取順序。此條直接證明「佔位法不需重排序緩衝」成立。

**`reader_loop`**——`test_reader_loop.py` 需大幅改寫（不再直接翻譯）。改用 fake pool 記錄提交，驗證讀到的行依序推進 context、依序佔位、依序提交，以及狀態指示與橫幅隨 `pool.error_state` 變化。`test_mem_reader.py` 中的 burst guard 測試不受影響。

**config／settings**——夾限範圍、`parse_advanced_values` 新增欄位。

**實機驗證**（`CLAUDE.md` 要求，pytest 全綠不算數）：開遊戲登入進世界內跑一輪，確認三件事——訊息一進來即顯示原文、譯文填回正確位置（順序未亂）、故意讓某則卡住時後續照常翻出。

## 不在本設計範圍

- **thinking 模式過慢**（單則 15～28 秒）：平行的 1.8 倍加速無法解決，需換模型或關閉 thinking。
- **批次翻譯**（一次請求翻多則）：請求數大減且上下文天然完整，但需模型回傳可對齊的多行輸出。以本專案實測的小模型脫稿紀錄（回吐 header、陷入 repetition loop）判斷可靠度不足，且單則出錯會波及整批。已評估並排除。
- **收訊佇列背壓**：訊息湧入快於翻譯速度時 pool 佇列會增長。現階段依 YAGNI 不處理；`max_messages` 已限制 overlay 的顯示量。
