# 框選擷取移到背景執行緒 —— 設計

## 為什麼

目前按下框選熱鍵（或 ⛶）、以及調整完框選框放開滑鼠時，擷取都在 UI 執行緒上同步跑完。這段期間 overlay、輸入框、框與卡片全部停止重繪。實機 `app.log`（2026-10-19，遊戲 client 1920x1080、螢幕 3440x1440）的耗時：

| 步驟 | 耗時 |
|---|---|
| WGC 拍遊戲畫面（`capture_window`，含等第一張影格與單色檢查） | 88～169 ms |
| GDI 拍整顆螢幕當暗化背景（`capture_screen`）＋兩張圖轉 `ColorImage`（`Selector::new`） | 50～90 ms |
| 合計（每次開框） | 約 0.15～0.25 秒 |

調整框只重拍 WGC，約 0.1～0.17 秒，這段時間連框都頓住。

## 目標

- 開框與調整框時，UI 執行緒不做任何擷取：WGC、GDI 截螢幕、RGB → `ColorImage` 轉換都移到背景。
- 調整框放開的當下就換成「辨識中」卡片，不必等重拍完。
- 行為與現在一致：
  - 選取畫面顯示的那一幀，和框選後送辨識的是同一幀（`Selector::frame()` 裁切，不重拍）；
  - 選取畫面等觸發鍵放開（`defer_for_release`，上限 1 秒）才顯示、搶前景；
  - 擷取失敗的錯誤卡片、最小化檢查、`OutsideGame` 都照舊。

## 非目標

- 讓選取畫面更早出現：它必須顯示拍到的那一幀，所以一定要等擷取完成。背景化只讓等待期間其他畫面不卡。
- 撤銷進行中的 WGC 擷取：拍一張最多 2 秒（`FRAME_TIMEOUT`），過期的結果直接丟掉即可。
- 貼圖上傳（`load_texture`）：需要 egui Context，留在選取畫面第一次繪製那一幀。
- 裁切與 PNG 編碼：框選範圍小，耗時可忽略，留在 UI 執行緒，`OutsideGame` 的處理不動。

## 資料流

比照 `RegionRunner` 的模式：流程把工作交給一個 trait 物件，正式版在背景執行緒做完後，以 `UiEvent` 送回 UI 執行緒，流程依編號判斷要用還是丟掉。

1. **新 trait `RegionCapturer`**（`src/ui/region/flow.rs`）：`fn capture(&self, job: CaptureJob)`。
   - `CaptureJob` 帶擷取編號、遊戲 hwnd，以及用途：
     - 開框：拍遊戲畫面＋所在螢幕＋背景；
     - 重拍：只拍遊戲畫面。
   - 完成後投遞 `UiEvent::RegionCaptured { id, result }`。
2. **正式版**（`src/app_main.rs`，取代 `WinRegionDesktop` 的三個擷取方法）：
   - 每個 job 開一條 `std::thread`；
   - 依序呼叫 `capture::capture_window`、`monitor_rect_at`（開框時），以及 `capture::capture_screen` 加兩張圖的 `ColorImage` 轉換；
   - panic 用 `catch_unwind` 轉成 `CaptureError::Failed`，跟 `PipelineRunner` 一樣不讓流程卡在等待中。
3. **`RegionDesktop` 拿掉 `capture_window`／`capture_screen`／`monitor_at`**，只留前景、找遊戲、游標、裁切這些立即完成的操作。
4. **`Selector`** 改由現成的 `ColorImage` 建立：轉換在背景做完，結果直接帶進來。現有的 `Selector::new(monitor, frame, backdrop)` 若測試還要用，可以留著當包裝。
5. **`UiEvent::RegionCaptured`** 在 `src/ui/app.rs` 分派給 `RegionHost`，再交給 `RegionFlow::on_captured(id, result)`。
   - `UiEvent` 要求 `Debug + Clone + PartialEq`，結果型別要能滿足：`Arc<Frame>` 可以手寫比較指標，或包一層只比編號，由實作決定。

## 流程狀態

`RegionFlow` 多一個「等擷取中」的狀態：`Option<PendingCapture { id, 用途 }>`。擷取編號單調遞增，每發一個 job 就 +1，跟辨識翻譯用的 `session` 分開。

| 時機 | 現在 | 改成 |
|---|---|---|
| 熱鍵／⛶ 開框 | 收掉舊框與卡片、撤銷請求，同步擷取後開選取畫面 | 收掉舊框與卡片、撤銷請求，發開框 job、進入等擷取中；`selecting` 設為 true |
| 開框 job 回來（編號相符） | — | 成功：建選取畫面（之後的放開等待、搶前景照舊）。失敗：在游標處顯示錯誤卡片、`selecting` 設為 false，跟現在同步失敗時一樣 |
| 等擷取中又按熱鍵或 ⛶ | — | 視同取消：清掉等待，記 log（`[region] selection cancelled while capturing`）。遊戲還在前景，所以不還前景 |
| 調整框放開 | 同步重拍，再裁切、開辨識中卡片、送出 | 立刻換成辨識中卡片、撤銷舊請求、session +1，發重拍 job |
| 重拍 job 回來（編號相符） | — | 成功：裁切並交給 runner，沿用放開時的那張卡片與 session，不重建卡片。失敗：換成錯誤卡片、保留框，跟現在一樣 |
| 等重拍時又調整框 | — | 發新的 job，舊的編號作廢 |
| 等重拍時關卡片或按熱鍵開新框 | — | 清掉等待，舊結果回來就丟掉 |
| 編號不符的結果 | — | 丟掉，記 log（`[region] stale capture dropped (id=…, current=…)`） |

**`selecting` 的語意改成「選取畫面開著或正在為它擷取」。** 熱鍵執行緒（`on_region_hotkey`）看到 true 就送取消，所以等擷取時連按兩下會取消，不會開出第二個選取畫面。要一併更新 `flow.rs`、`dispatch.rs` 裡相關的文件註解。

## Log

既有的 `frame captured (… ms=…)`、`selection started`、`selector placed`、`box adjusted, recapturing` 都保留，前綴一律用 `[region]`。另外新增：

- 發出 job：`[region] capture started (id=…, hwnd=…, purpose=select|recapture)`；
- 丟掉過期結果，以及等擷取時被取消（見上表）；
- 背景執行緒 panic：`[region] capture thread panicked (id=…): …`。

## 測試

- `fakes.rs` 新增 `FakeCapturer`：只記下收到的 job，不拍任何東西。
  - 測試自己呼叫 `flow.on_captured(id, result)` 送回結果，這樣就能測各種先後順序。
  - 現有的 `FakeDesktop.frames` 佇列改成餵給這個流程。
- `flow.rs` 原有的測試改成新流程，並補上下列情境：
  - 等擷取時再按熱鍵是取消；
  - 過期結果被丟掉；
  - 調整框放開時立刻出現辨識中卡片；
  - 等重拍時關卡片，結果回來不會重開卡片；
  - 重拍失敗時保留框、顯示錯誤。
- `tests/it/region_flow.rs`、`tests/it/ui_probe.rs` 與 `src/ui/app.rs` 的框選測試跟著改：fake 在下一次事件循環送回結果。
- `tests/it/capture.rs` 測的是 `capture.rs` 的純函式，不受影響。

## 實機驗收

由使用者驗收：

1. 按熱鍵開框、框選、出現結果，跟以前一樣。按住熱鍵不放時，選取畫面要等放開才出現。
2. 快速連按兩下熱鍵：不會開出選取畫面，遊戲維持在前景。
3. 調整框放開：卡片立刻變成辨識中，框不會頓住。
4. 遊戲最小化時按 ⛶：出現錯誤卡片。
5. 對照 `app.log`：`capture started` 到 `frame captured` 這段時間內，overlay 照常更新（例如收到的訊息照常出現）。

## 交給實作者決定

- `CaptureJob`／結果的具體型別與命名，以及如何滿足 `UiEvent` 的 derive。
- `Selector` 改成接收 `ColorImage` 的方式（新增建構函式或改原本的）。
- `RegionHost` 是否需要配合調整：選取畫面的建立時機延後，但 `release_wait` 仍從建立那一幀開始算，邏輯應該不必改。
