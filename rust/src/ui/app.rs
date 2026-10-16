//! egui 應用程式外殼：overlay 是根 viewport、泡泡是 immediate viewport，每幀先取完 UI 事件再繪製。

use std::sync::mpsc::Receiver;
use std::sync::{Arc, OnceLock};

use eframe::NativeOptions;
use eframe::egui::{self, Color32, ViewportBuilder, ViewportCommand, ViewportId};
use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup, wgpu};

use crate::config::app_name;
use crate::i18n::{current_language, t};
use crate::logs::log;
use crate::ui::events::{UiEvent, UiPoster};
use crate::ui::palette::{BAR, BG, FG_TRANSLATED};
use crate::ui::{fonts_egui, win32};

const BUBBLE_SIZE: f32 = 48.0;

/// UI 執行緒這一端：事件接收端與重繪用的 Context（`App::new` 時才綁上）。
pub struct Inbox {
    rx: Receiver<UiEvent>,
    ctx: Arc<OnceLock<egui::Context>>,
}

/// 建立 UI 事件通道；UI 起來前投遞的事件留在 channel，等第一幀一起處理。
pub fn channel() -> (UiPoster, Inbox) {
    let ctx = Arc::new(OnceLock::<egui::Context>::new());
    let wake = Arc::clone(&ctx);
    let (poster, rx) = UiPoster::new(Arc::new(move || {
        if let Some(ctx) = wake.get() {
            ctx.request_repaint();
        }
    }));
    (poster, Inbox { rx, ctx })
}

/// overlay 與泡泡的狀態；所有方法都在 UI 執行緒呼叫。
pub struct App {
    rx: Receiver<UiEvent>,
    hwnd: isize,
    alpha: f32,
    minimized: bool,
    shown: bool,
    status: Option<&'static str>,
    error: Option<(&'static str, Vec<(&'static str, String)>)>,
    foreground_slot: Option<u32>,
}

impl App {
    /// 綁上 Context 讓投遞端能喚醒重繪；`alpha` 是底板不透明度。
    pub fn new(ctx: &egui::Context, inbox: Inbox, alpha: f32) -> Self {
        let _ = inbox.ctx.set(ctx.clone());
        Self {
            rx: inbox.rx,
            hwnd: 0,
            alpha,
            minimized: false,
            shown: true,
            status: None,
            error: None,
            foreground_slot: None,
        }
    }

    /// 目前狀態列的 state key。
    pub fn status(&self) -> Option<&'static str> {
        self.status
    }

    /// overlay 的 HWND；第一幀之前是 0。
    pub fn hwnd(&self) -> isize {
        self.hwnd
    }

    /// 縮成泡泡（隱藏 overlay）或展開回 overlay。
    pub fn set_minimized(&mut self, minimized: bool) {
        self.minimized = minimized;
    }

    fn apply(&mut self, event: UiEvent) {
        match event {
            UiEvent::SetStatus(key) => self.status = Some(key),
            UiEvent::SetError { key, args } => self.error = Some((key, args)),
            UiEvent::ClearError => self.error = None,
            UiEvent::SetForegroundSlot(slot) => self.foreground_slot = slot,
            // shortcut: messages and tabs are dropped until the overlay model exists (Task 20)
            UiEvent::AddMessage { .. }
            | UiEvent::UpdateMessage { .. }
            | UiEvent::SetTabs { .. } => {}
        }
    }

    fn show_bubble(&self, ctx: &egui::Context) {
        let corner = ctx
            .input(|i| i.viewport().outer_rect)
            .map(|r| r.right_top());
        let mut builder = ViewportBuilder::default()
            .with_title(app_name())
            .with_inner_size([BUBBLE_SIZE, BUBBLE_SIZE])
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_always_on_top();
        if let Some(corner) = corner {
            builder = builder.with_position(corner - egui::vec2(BUBBLE_SIZE, 0.0));
        }
        let fill = BAR.gamma_multiply(self.alpha);
        ctx.show_viewport_immediate(ViewportId::from_hash_of("bubble"), builder, |ui, _| {
            let rect = ui.max_rect();
            ui.painter()
                .circle_filled(rect.center(), rect.width() / 2.0 - 2.0, fill);
        });
    }
}

impl eframe::App for App {
    fn logic(&mut self, _ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(event) = self.rx.try_recv() {
            self.apply(event);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.hwnd == 0 {
            self.hwnd = win32::hwnd_of(frame);
        }
        if self.shown == self.minimized {
            self.shown = !self.minimized;
            ctx.send_viewport_cmd(ViewportCommand::Visible(self.shown));
            // winit 處理 Visible 時會重寫 extended style 洗掉 NOACTIVATE，要保證下一幀補回
            ctx.request_repaint();
        }
        if self.hwnd != 0 {
            win32::make_non_activating(self.hwnd);
        }
        ui.painter()
            .rect_filled(ui.max_rect(), 0.0, BG.gamma_multiply(self.alpha));
        if let Some(key) = self.status {
            ui.colored_label(FG_TRANSLATED, t(&format!("status.{key}"), &[]));
        }
        if self.minimized {
            self.show_bubble(&ctx);
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        Color32::TRANSPARENT.to_normalized_gamma_f32()
    }
}

/// overlay 根視窗的 eframe 設定：DX12 經 DirectComposition 才有每像素透明；沒有硬體 adapter 時 wgpu 自動退到 WARP。
pub fn native_options() -> NativeOptions {
    let mut wgpu_options = WgpuConfiguration::default();
    if let WgpuSetup::CreateNew(setup) = &mut wgpu_options.wgpu_setup {
        setup.instance_descriptor.backends = wgpu::Backends::DX12;
        setup
            .instance_descriptor
            .backend_options
            .dx12
            .presentation_system = wgpu::Dx12SwapchainKind::DxgiFromVisual;
    }
    NativeOptions {
        viewport: ViewportBuilder::default()
            .with_title(app_name())
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top()
            .with_taskbar(true)
            .with_active(false),
        wgpu_options,
        ..Default::default()
    }
}

/// 開啟 UI 並跑到視窗關閉；`build` 在第一幀前以 egui Context 建立 app。
pub fn run<A: eframe::App + 'static>(
    native: NativeOptions,
    build: impl FnOnce(&egui::Context) -> A + 'static,
) -> Result<(), eframe::Error> {
    eframe::run_native(
        &app_name(),
        native,
        Box::new(|cc| {
            if let Some(render) = &cc.wgpu_render_state {
                let info = render.adapter.get_info();
                log(format!(
                    "[ui] renderer: backend={:?} adapter={:?} type={:?}",
                    info.backend, info.name, info.device_type
                ));
            }
            fonts_egui::install(&cc.egui_ctx, &current_language());
            Ok(Box::new(build(&cc.egui_ctx)))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;

    #[test]
    fn app_drains_events_each_frame() {
        let (poster, inbox) = channel();
        poster.post(UiEvent::SetStatus("listening"));
        poster.post(UiEvent::SetForegroundSlot(Some(2)));
        poster.post(UiEvent::SetError {
            key: "error.offline",
            args: vec![("detail", "x".into())],
        });
        let mut harness = Harness::new_eframe(|cc| {
            fonts_egui::install(&cc.egui_ctx, "en-US");
            App::new(&cc.egui_ctx, inbox, 0.8)
        });
        harness.step();
        let app = harness.state();
        assert_eq!(app.status, Some("listening"));
        assert_eq!(app.foreground_slot, Some(2));
        assert_eq!(app.error.as_ref().map(|e| e.0), Some("error.offline"));
    }

    #[test]
    fn posting_after_start_requests_repaint() {
        let (poster, inbox) = channel();
        let ctx = egui::Context::default();
        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = Arc::clone(&hits);
        ctx.set_request_repaint_callback(move |_| {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });
        let _app = App::new(&ctx, inbox, 0.8);
        poster.post(UiEvent::ClearError);
        assert!(hits.load(std::sync::atomic::Ordering::SeqCst) > 0);
    }
}
