//! 多帳號一鍵登入：帳號儲存、遊戲位置偵測、登入連線、Steam 票據、用戶端啟動與一鍵編排。

pub mod accounts;
pub mod install;
pub mod launch;
pub mod running;

pub use accounts::{Account, AccountKind};
