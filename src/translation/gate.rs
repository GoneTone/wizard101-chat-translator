//! 翻譯請求的總量閘：限制同時進行的請求數，與哪一個翻譯池送出的無關。
//!
//! 兩個 [`TranslationPool`](super::pool::TranslationPool)（玩家對話、系統訊息）共用一個閘，
//! 同時進行的收訊翻譯則數因此維持設定字面的語意，而不是每條通道各一份。
//! 不用 `tokio::sync::Semaphore`：上限可在設定視窗即時調小，而號誌收不回已發出的額度。

use std::sync::{Mutex, MutexGuard};

use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::util::lock;

struct Slots {
    limit: usize,
    in_use: usize,
}

/// 可調上限的併發計數閘；[`acquire`](Self::acquire) 取得的額度在 [`GatePermit`] 丟棄時歸還。
pub struct ConcurrencyGate {
    slots: Mutex<Slots>,
    changed: Notify,
}

/// 一個已取得的額度，丟棄時釋放。
pub struct GatePermit<'a> {
    gate: &'a ConcurrencyGate,
}

impl ConcurrencyGate {
    /// 建立上限為 `limit`（至少 1）的閘。
    pub fn new(limit: usize) -> Self {
        Self {
            slots: Mutex::new(Slots {
                limit: limit.max(1),
                in_use: 0,
            }),
            changed: Notify::new(),
        }
    }

    fn slots(&self) -> MutexGuard<'_, Slots> {
        lock(&self.slots)
    }

    /// 目前已發出的額度數。
    pub fn in_use(&self) -> usize {
        self.slots().in_use
    }

    /// 目前的上限。
    pub fn limit(&self) -> usize {
        self.slots().limit
    }

    /// 取得一個額度；額度滿時等待。`stop` 取消即放棄並回傳 `None`（關閉中）。
    pub async fn acquire(&self, stop: &CancellationToken) -> Option<GatePermit<'_>> {
        loop {
            // 先登記等待再檢查額度：檢查後、等待前的釋放才不會漏接
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if stop.is_cancelled() {
                return None;
            }
            {
                let mut slots = self.slots();
                if slots.in_use < slots.limit {
                    slots.in_use += 1;
                    return Some(GatePermit { gate: self });
                }
            }
            tokio::select! {
                biased;
                _ = stop.cancelled() => return None,
                _ = changed => {}
            }
        }
    }

    /// 變更上限。調大時喚醒等待者；調小不收回已發出的額度 —— 在飛行中的請求跑完自然收斂。
    pub fn set_limit(&self, limit: usize) {
        let limit = limit.max(1);
        let mut slots = self.slots();
        if slots.limit == limit {
            return;
        }
        slots.limit = limit;
        drop(slots);
        self.changed.notify_waiters();
    }
}

impl Drop for GatePermit<'_> {
    fn drop(&mut self) {
        self.gate.slots().in_use -= 1;
        // 全部喚醒：被叫醒的那一個若正好因關閉而放棄，空出的額度才不會沒人接手
        self.gate.changed.notify_waiters();
    }
}
