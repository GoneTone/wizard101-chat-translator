//! 收訊端：遊戲程序的記憶體存取、pattern 掃描、hook 掛入與程序／視窗辨識。

pub mod chat_hook;
pub mod chat_tabs;
pub mod hook_state;
pub mod hooks;
pub mod markup;
pub mod mem_reader;
pub mod memory;
pub mod message_log;
pub mod pattern;
pub mod process;
pub mod run_loop;
pub mod status;
pub mod supervisor;
pub mod ui_rect;
pub mod ui_tree;
