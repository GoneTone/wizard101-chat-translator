//! 收訊與翻譯執行緒送往 UI 執行緒的事件；UI 執行緒依序取出套用到 overlay。

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};

use crate::reader::chat_tabs::TabState;

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
}

/// 任何執行緒都能用的 UI 事件投遞端：送進 channel 後喚醒 UI 重繪。
#[derive(Clone)]
pub struct UiPoster {
    tx: Sender<UiEvent>,
    repaint: Arc<dyn Fn() + Send + Sync>,
}

impl UiPoster {
    /// 建立投遞端與 UI 執行緒的接收端；`repaint` 在每次投遞後呼叫。
    pub fn new(repaint: Arc<dyn Fn() + Send + Sync>) -> (Self, Receiver<UiEvent>) {
        let (tx, rx) = mpsc::channel();
        (Self { tx, repaint }, rx)
    }

    /// 測試用：不重繪，事件留在回傳的接收端。
    pub fn for_test() -> (Self, Receiver<UiEvent>) {
        Self::new(Arc::new(|| {}))
    }

    /// 投遞一則事件；UI 已關閉（接收端不在）時丟棄。
    pub fn post(&self, event: UiEvent) {
        let _ = self.tx.send(event);
        (self.repaint)();
    }
}
