//! 啟動 eframe：overlay 根視窗的設定、WARP 軟體算圖的退路與事件迴圈入口。

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use eframe::NativeOptions;
use eframe::egui::{self, ViewportBuilder};
use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup, wgpu};

use crate::config::app_name;
use crate::i18n::current_language;
use crate::logs::log;
use crate::ui::fonts_egui;
use crate::ui::icon::app_icon;
use crate::ui::win32;

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
            .with_active(false)
            // 不給的話 eframe 會裝上 egui 標誌
            .with_icon(app_icon()),
        wgpu_options,
        ..Default::default()
    }
}

/// 強制走 WARP 軟體 adapter 的 eframe 設定（硬體 adapter 建立裝置或 surface 失敗時的退路）。
pub fn warp_options() -> NativeOptions {
    let mut native = native_options();
    if let WgpuSetup::CreateNew(setup) = &mut native.wgpu_options.wgpu_setup {
        setup.native_adapter_selector = Some(Arc::new(|adapters, _surface| {
            adapters
                .iter()
                .find(|a| a.get_info().device_type == wgpu::DeviceType::Cpu)
                .cloned()
                .ok_or_else(|| "no WARP adapter".to_owned())
        }));
    }
    native
}

/// `visible` 為 false 時根視窗建立時就藏著（首次設定精靈：overlay 不會先閃一下）。
fn with_root_visibility(mut native: NativeOptions, visible: bool) -> NativeOptions {
    if !visible {
        native.viewport = native.viewport.with_visible(false);
    }
    native
}

/// 同 [`run`]，但 app 建立前就失敗（硬體 adapter 列得出來卻開不了）時改用 WARP 再試一次；`visible` 見 [`with_root_visibility`]。
pub fn run_with_warp_fallback<A: eframe::App + 'static>(
    visible: bool,
    build: impl FnOnce(&egui::Context) -> A + 'static,
) -> Result<(), eframe::Error> {
    let build = Rc::new(RefCell::new(Some(build)));
    let attempt = |native| {
        let build = Rc::clone(&build);
        run(with_root_visibility(native, visible), move |ctx| {
            let build = build.borrow_mut().take().expect("app is built once");
            build(ctx)
        })
    };
    match attempt(native_options()) {
        Err(e) if build.borrow().is_some() => {
            log(format!(
                "[ui] hardware renderer failed before the overlay opened: {e}; retrying with WARP"
            ));
            attempt(warp_options())
        }
        other => other,
    }
}

/// 開啟 UI 並跑到視窗關閉；`build` 在第一幀前以 egui Context 建立 app。
pub fn run<A: eframe::App + 'static>(
    native: NativeOptions,
    build: impl FnOnce(&egui::Context) -> A + 'static,
) -> Result<(), eframe::Error> {
    win32::disable_redirection_bitmap();
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
            // 事件迴圈此時已建立（winit 建立時註冊了 raw input），每次啟動（含 WARP 重試、精靈）都經過這裡
            win32::stop_raw_keyboard_input();
            fonts_egui::install(&cc.egui_ctx, &current_language());
            Ok(Box::new(build(&cc.egui_ctx)))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_viewport_carries_the_app_icon() {
        // 沒給 icon 時 eframe 會換上 egui 標誌
        for native in [native_options(), warp_options()] {
            let icon = native.viewport.icon.expect("root viewport has an icon");
            assert_eq!((icon.width, icon.height), (48, 48));
            assert_ne!(*icon, egui::IconData::default());
        }
    }

    #[test]
    fn root_viewport_can_start_hidden() {
        for native in [native_options(), warp_options()] {
            let hidden = with_root_visibility(native, false);
            assert_eq!(hidden.viewport.visible, Some(false));
        }
        assert_eq!(
            with_root_visibility(native_options(), true)
                .viewport
                .visible,
            None
        );
    }
}
