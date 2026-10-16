//! 收訊與發話共用的近期聊天原文行緩衝（執行緒安全）。

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

/// 帶進提示詞的近期對話行數：短窗涵蓋眼前的對話線，避免遠處舊話題污染判斷。
pub const CONTEXT_LINES: usize = 8;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// 近期聊天原文行的環狀緩衝。
pub struct ChatContext {
    lines: Mutex<VecDeque<String>>,
    max_lines: usize,
}

impl ChatContext {
    pub fn new() -> Self {
        Self::with_capacity(CONTEXT_LINES)
    }

    pub fn with_capacity(max_lines: usize) -> Self {
        Self {
            lines: Mutex::new(VecDeque::new()),
            max_lines,
        }
    }

    /// 加入一行原文，超出容量時擠掉最舊的。
    pub fn push(&self, line: impl Into<String>) {
        let mut lines = lock(&self.lines);
        if self.max_lines == 0 {
            return;
        }
        if lines.len() == self.max_lines {
            lines.pop_front();
        }
        lines.push_back(line.into());
    }

    /// 目前上下文的複本，後續 `push` 不影響已取出的快照。
    pub fn snapshot(&self) -> Vec<String> {
        lock(&self.lines).iter().cloned().collect()
    }
}

impl Default for ChatContext {
    fn default() -> Self {
        Self::new()
    }
}

/// 單一遊戲客戶端的各分頁上下文，`current_tab` 指向遊戲目前選中的分頁。
#[derive(Default)]
pub struct ClientContexts {
    tabs: Mutex<HashMap<i32, Arc<ChatContext>>>,
    current_tab: Mutex<i32>,
}

impl ClientContexts {
    pub fn new() -> Self {
        Self::default()
    }

    /// 該分頁的上下文，不存在就建立。
    pub fn for_tab(&self, tab: i32) -> Arc<ChatContext> {
        Arc::clone(lock(&self.tabs).entry(tab).or_default())
    }

    /// 遊戲目前選中分頁的上下文（發話用）。
    pub fn current(&self) -> Arc<ChatContext> {
        self.for_tab(self.current_tab())
    }

    pub fn current_tab(&self) -> i32 {
        *lock(&self.current_tab)
    }

    pub fn set_current_tab(&self, tab: i32) {
        *lock(&self.current_tab) = tab;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_returns_pushed_lines_in_order() {
        let ctx = ChatContext::new();
        ctx.push("[A] one");
        ctx.push("[B] two");
        assert_eq!(ctx.snapshot(), ["[A] one", "[B] two"]);
    }

    #[test]
    fn snapshot_is_a_copy() {
        let ctx = ChatContext::new();
        ctx.push("[A] one");
        let snap = ctx.snapshot();
        ctx.push("[B] two");
        assert_eq!(snap, ["[A] one"]);
    }

    #[test]
    fn context_keeps_last_8_lines_per_tab() {
        let contexts = ClientContexts::new();
        for i in 0..CONTEXT_LINES + 3 {
            contexts.for_tab(0).push(format!("[A] m{i}"));
        }
        contexts.for_tab(1).push("other");
        let lines = contexts.for_tab(0).snapshot();
        assert_eq!(lines.len(), 8);
        assert_eq!(lines[0], "[A] m3");
        assert_eq!(lines[7], "[A] m10");
        assert_eq!(contexts.for_tab(1).snapshot(), ["other"]);
    }

    #[test]
    fn concurrent_pushes_do_not_lose_lines() {
        let ctx = Arc::new(ChatContext::with_capacity(1000));
        let threads: Vec<_> = "ABCD"
            .chars()
            .map(|tag| {
                let ctx = Arc::clone(&ctx);
                std::thread::spawn(move || {
                    for i in 0..100 {
                        ctx.push(format!("[{tag}] {i}"));
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(ctx.snapshot().len(), 400);
    }

    #[test]
    fn client_contexts_keep_tabs_apart() {
        let contexts = ClientContexts::new();
        contexts.for_tab(0).push("a");
        assert!(contexts.for_tab(1).snapshot().is_empty());
        assert_eq!(contexts.for_tab(0).snapshot(), ["a"]);
    }

    #[test]
    fn client_contexts_current_follows_current_tab() {
        let contexts = ClientContexts::new();
        assert!(Arc::ptr_eq(&contexts.current(), &contexts.for_tab(0)));
        contexts.set_current_tab(1);
        assert!(Arc::ptr_eq(&contexts.current(), &contexts.for_tab(1)));
    }
}
