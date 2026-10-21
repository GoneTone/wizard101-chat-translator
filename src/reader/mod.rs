//! 收訊端：掛入遊戲讀聊天、解析聊天行與分頁，並管理各客戶端的收訊執行緒與狀態。

pub mod chat_tabs;
pub mod hook;
pub mod markup;
pub mod mem_reader;
pub mod message_log;
pub mod run_loop;
pub mod status;
pub mod supervisor;
pub mod ui_rect;
pub mod ui_tree;
