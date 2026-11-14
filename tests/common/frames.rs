//! 離屏 viewport 測試用的連續幀：上一幀登記過的視窗，下一幀當成已建好。
#![allow(dead_code)]

use std::time::Duration;

use eframe::egui::{Context, ViewportCommand, ViewportId};
use wizard101_chat_translator::ui::fonts_egui;
use wizard101_chat_translator::ui::viewport::testing::{
    close_next, input_with_windows, native_like_context,
};

pub struct Frames {
    pub ctx: Context,
    pub windows: Vec<ViewportId>,
    /// 最近一幀根 viewport 要求的重繪延遲。
    pub root_repaint: Duration,
}

impl Frames {
    pub fn new() -> Self {
        let ctx = native_like_context();
        fonts_egui::install(&ctx, "en-US");
        Self {
            ctx,
            windows: Vec::new(),
            root_repaint: Duration::MAX,
        }
    }

    /// 跑一幀（`close` 為真時模擬按 ✕）；`show` 在幀內畫 host，回傳這一幀所有 viewport 收到的指令。
    pub fn run(&mut self, close: bool, mut show: impl FnMut(&Context)) -> Vec<ViewportCommand> {
        close_next(close);
        let mut output = self
            .ctx
            .run_ui(input_with_windows(&self.windows), |ui| show(ui.ctx()));
        close_next(false);
        output.textures_delta.clear();
        self.root_repaint = output.viewport_output[&ViewportId::ROOT].repaint_delay;
        self.windows = output
            .viewport_output
            .keys()
            .copied()
            .filter(|id| *id != ViewportId::ROOT)
            .collect();
        output
            .viewport_output
            .into_values()
            .flat_map(|v| v.commands)
            .collect()
    }
}
