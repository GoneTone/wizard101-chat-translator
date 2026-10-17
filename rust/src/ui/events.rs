//! 收訊與翻譯執行緒送往 UI 執行緒的事件；UI 執行緒依序取出套用到 overlay。

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use crate::reader::chat_tabs::TabState;
use crate::region::pipeline::RegionResult;
use crate::runtime::updater::Release;
use crate::ui::geometry::Rect;

/// 一則 overlay 更新。
#[derive(Debug, Clone, PartialEq)]
pub enum UiEvent {
    /// 新增一則訊息；`pending` 時 `translated` 是佔位文字，之後以 `msg_id` 回填。
    AddMessage {
        slot: u32,
        tabs: Vec<i32>,
        original: String,
        translated: String,
        msg_id: Option<u64>,
        pending: bool,
        color: Option<String>,
    },
    /// 回填佔位列的譯文（`failed` 時是失敗提示）。
    UpdateMessage {
        msg_id: u64,
        translated: String,
        failed: bool,
    },
    /// 某客戶端的分頁狀態；`None`＝分頁不明（遊戲不在）。
    SetTabs { slot: u32, state: Option<TabState> },
    /// 目前在前景的客戶端編號。
    SetForegroundSlot(Option<u32>),
    /// 狀態列（`status.*` 的 state key）。
    SetStatus(&'static str),
    /// 顯示錯誤橫幅（文案 key 與 format 參數）。
    SetError {
        key: &'static str,
        args: Vec<(&'static str, String)>,
    },
    /// 收起錯誤橫幅。
    ClearError,
    /// 自動檢查發現新版；關掉過的版本不會再跳出。
    UpdateFound(Release),
    /// 呼出翻譯輸入框；`retarget` 有值時先改綁到該客戶端。
    ShowInputBox { retarget: Option<isize> },
    /// 遊戲聊天輸入框開了；`anchor` 是它的螢幕矩形。
    GameInputOpened { hwnd: isize, anchor: Option<Rect> },
    /// 遊戲聊天輸入框關了。
    GameInputClosed { hwnd: isize },
    /// 發話翻譯結果；`Err` 是已在地化的錯誤訊息。
    OutgoingDone {
        session: u64,
        result: Result<String, String>,
    },
    /// 框選熱鍵；`hwnd` 為 0 表示取消目前的選取層。
    RegionHotkey { hwnd: isize },
    /// 框選辨識翻譯結果；`Err` 是 `describe_error` 的文案。
    RegionDone {
        session: u64,
        result: Result<RegionResult, String>,
    },
    /// 結束程式。
    Quit,
}

/// 任何執行緒都能用的 UI 事件投遞端：送進 channel 後喚醒 UI 重繪。
#[derive(Clone)]
pub struct UiPoster {
    tx: Sender<UiEvent>,
    repaint: Arc<dyn Fn() + Send + Sync>,
    /// 所有投遞端共用的「最後送出的前景客戶端」（`None`＝還沒送過）。
    foreground: Arc<Mutex<Option<Option<u32>>>>,
}

impl UiPoster {
    /// 建立投遞端與 UI 執行緒的接收端；`repaint` 在每次投遞後呼叫。
    pub fn new(repaint: Arc<dyn Fn() + Send + Sync>) -> (Self, Receiver<UiEvent>) {
        let (tx, rx) = mpsc::channel();
        let foreground = Arc::default();
        (
            Self {
                tx,
                repaint,
                foreground,
            },
            rx,
        )
    }

    /// 測試用：不重繪，事件留在回傳的接收端。
    pub fn for_test() -> (Self, Receiver<UiEvent>) {
        Self::new(Arc::new(|| {}))
    }

    /// 投遞一則事件；UI 已關閉（接收端不在）時丟棄。
    ///
    /// 前景客戶端與上一次（任一投遞端）送出的相同時直接略過：overlay 套用後不會變，免得遊戲在前景時每 400ms 整窗重繪。
    pub fn post(&self, event: UiEvent) {
        if let UiEvent::SetForegroundSlot(slot) = event {
            // 鎖住到送出為止，兩條 reader 執行緒的送出順序才與記下的值一致
            let mut last = self.foreground.lock().unwrap_or_else(|e| e.into_inner());
            if *last == Some(slot) {
                return;
            }
            *last = Some(slot);
            let _ = self.tx.send(event);
        } else {
            let _ = self.tx.send(event);
        }
        (self.repaint)();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn repeated_foreground_slot_neither_queues_nor_repaints() {
        let wakes = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&wakes);
        let (a, rx) = UiPoster::new(Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        }));
        let b = a.clone();
        for (poster, slot) in [(&a, 1), (&a, 1), (&b, 2), (&a, 1), (&a, 1)] {
            poster.post(UiEvent::SetForegroundSlot(Some(slot)));
        }
        b.post(UiEvent::SetForegroundSlot(None));
        a.post(UiEvent::SetStatus("ready"));
        a.post(UiEvent::SetStatus("ready"));
        let got: Vec<UiEvent> = rx.try_iter().collect();
        assert_eq!(
            got,
            [
                UiEvent::SetForegroundSlot(Some(1)),
                UiEvent::SetForegroundSlot(Some(2)),
                UiEvent::SetForegroundSlot(Some(1)),
                UiEvent::SetForegroundSlot(None),
                UiEvent::SetStatus("ready"),
                UiEvent::SetStatus("ready"),
            ]
        );
        assert_eq!(wakes.load(Ordering::SeqCst), got.len());
    }
}
