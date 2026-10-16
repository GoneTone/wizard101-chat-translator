//! egui 應用程式外殼：overlay 是根 viewport、泡泡是 immediate viewport，每幀先取完 UI 事件再繪製。

use std::sync::mpsc::Receiver;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use eframe::NativeOptions;
use eframe::egui::{self, Color32, ViewportBuilder, ViewportCommand, ViewportId};
use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup, wgpu};

use crate::config::app_name;
use crate::i18n::current_language;
use crate::logs::log;
use crate::ui::events::{UiEvent, UiPoster};
use crate::ui::overlay::frame::{Clicked, Frame, Win32Shell};
use crate::ui::overlay::model::OverlayModel;
use crate::ui::palette::BAR;
use crate::ui::{fonts_egui, win32};

const BUBBLE_SIZE: f32 = 48.0;
/// 設定接上之前的訊息上限（對應 config 預設值）。
const DEFAULT_MAX_MESSAGES: usize = 200;

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
    frame: Frame,
    minimized: bool,
    shown: bool,
    status: Option<&'static str>,
    error: Option<(&'static str, Vec<(&'static str, String)>)>,
    model: OverlayModel,
}

impl App {
    /// 綁上 Context 讓投遞端能喚醒重繪；`frame` 是 overlay 的外框（含底板不透明度與起始位置）。
    pub fn new(ctx: &egui::Context, inbox: Inbox, frame: Frame) -> Self {
        let _ = inbox.ctx.set(ctx.clone());
        Self {
            rx: inbox.rx,
            hwnd: 0,
            frame,
            minimized: false,
            shown: true,
            status: None,
            error: None,
            model: OverlayModel::new(DEFAULT_MAX_MESSAGES, 0),
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
        self.model.set_minimized(minimized);
    }

    fn apply(&mut self, event: UiEvent) {
        match event {
            UiEvent::SetStatus(key) => self.status = Some(key),
            UiEvent::SetError { key, args } => self.error = Some((key, args)),
            UiEvent::ClearError => self.error = None,
            UiEvent::SetForegroundSlot(slot) => self.model.set_foreground_slot(slot),
            UiEvent::AddMessage {
                slot,
                tabs,
                original,
                translated,
                msg_id,
                pending,
                color,
            } => self.model.add_message(
                slot,
                &tabs,
                &original,
                &translated,
                msg_id,
                pending,
                color.as_deref(),
                Instant::now(),
            ),
            UiEvent::UpdateMessage {
                msg_id,
                translated,
                failed,
            } => self.model.update_message(msg_id, &translated, failed),
            UiEvent::SetTabs { slot, state } => self.model.set_tabs(slot, state),
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
        let fill = BAR.gamma_multiply(self.frame.alpha());
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
        let mut shell = Win32Shell(self.hwnd);
        match self.frame.show(ui, &mut shell, &app_name(), self.status) {
            Some(Clicked::Region) => {
                log("[ui] region button clicked; region capture arrives in phase 3")
            }
            Some(Clicked::Settings) => {
                log("[ui] settings button clicked; settings window arrives in phase 4")
            }
            Some(Clicked::Minimize) => self.set_minimized(true),
            // 與 Alt+F4、工作列關閉同一條路：關掉根 viewport
            Some(Clicked::Close) => ctx.send_viewport_cmd(ViewportCommand::Close),
            None => {}
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
            App::new(&cc.egui_ctx, inbox, Frame::new(0.8, None, |_, _, _, _| {}))
        });
        harness.step();
        let app = harness.state();
        assert_eq!(app.status, Some("listening"));
        assert_eq!(app.model.client(), Some(2));
        assert_eq!(app.error.as_ref().map(|e| e.0), Some("error.offline"));
    }

    #[test]
    fn app_routes_messages_into_the_overlay_model() {
        let (poster, inbox) = channel();
        poster.post(UiEvent::AddMessage {
            slot: 1,
            tabs: vec![0],
            original: "o".into(),
            translated: "...".into(),
            msg_id: Some(7),
            pending: true,
            color: None,
        });
        poster.post(UiEvent::UpdateMessage {
            msg_id: 7,
            translated: "t".into(),
            failed: false,
        });
        let mut harness = Harness::new_eframe(|cc| {
            fonts_egui::install(&cc.egui_ctx, "en-US");
            App::new(&cc.egui_ctx, inbox, Frame::new(0.8, None, |_, _, _, _| {}))
        });
        harness.step();
        let view = harness.state().model.current_view().unwrap();
        assert_eq!(view.messages[0].translated, "t");
        assert!(!view.messages[0].pending);
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
        let _app = App::new(&ctx, inbox, Frame::new(0.8, None, |_, _, _, _| {}));
        poster.post(UiEvent::ClearError);
        assert!(hits.load(std::sync::atomic::Ordering::SeqCst) > 0);
    }
}
