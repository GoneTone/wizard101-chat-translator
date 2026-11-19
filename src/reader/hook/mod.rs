//! 改寫遊戲機器碼的 hook：模組附近的 stub 區與 root window hook、AddLine hook，以及當機後修復殘留 hook 的還原狀態。

pub mod add_line;
pub mod cave;
pub mod root_window;
pub mod state;
