//! 介面相關模組。

pub mod app;
pub mod clipboard;
pub mod copy_menu;
pub mod events;
pub mod fonts;
pub mod fonts_egui;
pub mod geometry;
pub mod glass;
pub mod icon;
pub mod input_box;
pub mod input_host;
pub mod overlay;
pub mod palette;
pub mod region;
pub mod region_host;
pub mod richtext;
pub mod run;
pub mod settings;
pub mod settings_host;
pub mod text_select;
pub mod update_host;
pub mod viewport;
pub mod win32;

/// 要求這一幀重排（`request_discard`），並另外排一幀：每幀重排次數有限（`max_passes`），被略過的重排靠下一幀補上，畫面才不會停在沒排好的樣子。
pub fn relayout(ctx: &eframe::egui::Context, reason: &'static str) {
    ctx.request_discard(reason);
    ctx.request_repaint();
}
