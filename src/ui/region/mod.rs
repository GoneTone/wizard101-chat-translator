//! 框選翻譯的介面：全螢幕選取畫面、框選後的可調整框與結果卡。

pub mod card;
#[cfg(any(test, feature = "test-support"))]
pub mod fakes;
pub mod flow;
pub mod region_box;
pub mod selector;
