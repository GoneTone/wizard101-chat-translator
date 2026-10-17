//! 視窗架構驗證（egui 的退場條件）：開出真的 overlay 與泡泡、停在螢幕外，以程式讀回各項行為。
//!
//! 會短暫出現螢幕外視窗與工作列按鈕，所以預設略過；手動跑：
//! `cargo test --manifest-path rust/Cargo.toml --test ui_probe -- --ignored --nocapture`

use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use eframe::egui::{self, ColorImage, ViewportCommand};
use eframe::egui_wgpu::wgpu;
use windows_capture::capture::{Context, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame as WgcFrame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
    MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};
use windows_capture::window::Window;
use windows_sys::Win32::Foundation::{HWND, LPARAM, RECT};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetWindowRect, GetWindowThreadProcessId, HTCLIENT,
    HTTRANSPARENT, IsWindowVisible, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_NCHITTEST,
    WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOPMOST, WS_EX_TRANSPARENT,
};
use winit::platform::windows::EventLoopBuilderExtWindows;
use wizard101_chat_translator::logs::log;
use wizard101_chat_translator::ui::app::{self, App};
use wizard101_chat_translator::ui::events::{UiEvent, UiPoster};
use wizard101_chat_translator::ui::overlay::frame::Frame;
use wizard101_chat_translator::ui::win32::{ex_style, hwnd_of};

const PARK: f32 = 10000.0;
const ALPHA: f32 = 0.8;
/// 取背景像素時離左下角的距離：避開 Windows 11 的圓角。
const BG_INSET: usize = 10;

/// 收集 egui-wgpu 透過 `log` crate 發出的警告（透明 swapchain 不支援時會在這裡出聲）。
struct Warnings(Mutex<Vec<String>>);

impl log::Log for Warnings {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Warn
    }
    fn log(&self, r: &log::Record) {
        if self.enabled(r.metadata()) {
            self.0
                .lock()
                .unwrap()
                .push(format!("{}: {}", r.target(), r.args()));
        }
    }
    fn flush(&self) {}
}

static WARNINGS: Warnings = Warnings(Mutex::new(Vec::new()));

#[derive(Default)]
struct Shared {
    hwnd: isize,
    status: Option<&'static str>,
    minimize: bool,
    adapter: Option<wgpu::AdapterInfo>,
}

/// 包住真正的 App，把它的狀態轉給檢查執行緒、並依檢查執行緒的要求縮成泡泡。
struct Probe {
    app: App,
    shared: Arc<Mutex<Shared>>,
}

impl eframe::App for Probe {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.app.logic(ctx, frame);
        let mut s = self.shared.lock().unwrap();
        s.status = self.app.status();
        self.app.set_minimized(s.minimize);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.app.ui(ui, frame);
        let mut s = self.shared.lock().unwrap();
        s.hwnd = hwnd_of(frame);
        if s.adapter.is_none() {
            s.adapter = frame.wgpu_render_state().map(|r| r.adapter.get_info());
        }
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        self.app.clear_color(visuals)
    }
}

struct Report {
    label: &'static str,
    lines: Vec<(bool, String)>,
}

impl Report {
    fn check(&mut self, ok: bool, line: String) {
        log(format!(
            "[ui] probe {}: {} {line}",
            self.label,
            if ok { "PASS" } else { "FAIL" }
        ));
        self.lines.push((ok, line));
    }
}

/// 輪詢 `cond`；`ctx` 給定時每輪都要求重繪，`None` 則只靠 app 自己排的幀。
fn poll(timeout: Duration, ctx: Option<&egui::Context>, mut cond: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if cond() {
            return true;
        }
        if let Some(ctx) = ctx {
            ctx.request_repaint();
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    cond()
}

fn visible(hwnd: isize) -> bool {
    unsafe { IsWindowVisible(hwnd as HWND) != 0 }
}

unsafe extern "system" fn collect_own(hwnd: HWND, lparam: LPARAM) -> i32 {
    let found = unsafe { &mut *(lparam as *mut Vec<isize>) };
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    if pid == std::process::id() && unsafe { IsWindowVisible(hwnd) } != 0 {
        found.push(hwnd as isize);
    }
    1
}

fn own_visible_windows() -> Vec<isize> {
    let mut found = Vec::<isize>::new();
    unsafe { EnumWindows(Some(collect_own), &mut found as *mut _ as LPARAM) };
    found
}

fn hit_test_center(hwnd: isize) -> Option<u32> {
    let mut r = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    unsafe { GetWindowRect(hwnd as HWND, &mut r) };
    let (x, y) = ((r.left + r.right) / 2, (r.top + r.bottom) / 2);
    let lparam = (((y as u16 as u32) << 16) | x as u16 as u32) as LPARAM;
    let mut result = 0usize;
    let ok = unsafe {
        SendMessageTimeoutW(
            hwnd as HWND,
            WM_NCHITTEST,
            0,
            lparam,
            SMTO_ABORTIFHUNG,
            2000,
            &mut result,
        )
    };
    (ok != 0).then_some(result as u32)
}

fn screenshot(ctx: &egui::Context) -> Option<Arc<ColorImage>> {
    let slot = Arc::new(Mutex::new(None));
    let sink = Arc::clone(&slot);
    ctx.send_viewport_cmd(ViewportCommand::Screenshot(egui::ScreenshotCallback::new(
        move |image| *sink.lock().unwrap() = Some(image),
    )));
    poll(Duration::from_secs(3), Some(ctx), || {
        slot.lock().unwrap().is_some()
    });
    slot.lock().unwrap().take()
}

/// 一張 WGC 影格：寬、高、RGBA 像素。
type Capture = (u32, u32, Vec<u8>);

/// 收第一張影格就停的 WGC 擷取。
struct Grab(Arc<Mutex<Option<Capture>>>);

impl GraphicsCaptureApiHandler for Grab {
    type Flags = Arc<Mutex<Option<Capture>>>;
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(Self(ctx.flags))
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut WgcFrame,
        control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        let (w, h) = (frame.width(), frame.height());
        let buffer = frame.buffer()?;
        let pixels = buffer.as_nopadding_buffer(&mut Vec::new()).to_vec();
        *self.0.lock().unwrap() = Some((w, h, pixels));
        control.stop();
        Ok(())
    }
}

/// 用 Windows Graphics Capture 拍視窗經 DWM 合成後的樣子（螢幕外的視窗也拍得到）。
fn compose_capture(hwnd: isize) -> Option<Capture> {
    let slot = Arc::new(Mutex::new(None));
    let settings = Settings::new(
        Window::from_raw_hwnd(hwnd as *mut _),
        CursorCaptureSettings::WithoutCursor,
        DrawBorderSettings::WithoutBorder,
        SecondaryWindowSettings::Default,
        MinimumUpdateIntervalSettings::Default,
        DirtyRegionSettings::Default,
        ColorFormat::Rgba8,
        Arc::clone(&slot),
    );
    let control = Grab::start_free_threaded(settings).ok()?;
    poll(Duration::from_secs(3), None, || {
        slot.lock().unwrap().is_some()
    });
    let _ = control.stop();
    slot.lock().unwrap().take()
}

fn pixel_at((w, _, pixels): &Capture, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * w + x) * 4) as usize;
    [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
}

/// 合成後的左下角背景像素是否與 egui 自己畫出的一致（重導向表面透白時 RGB 會偏亮）。
fn composed_matches(hwnd: isize, drawn: [u8; 4]) -> (bool, Option<[u8; 4]>) {
    let inset = BG_INSET as u32;
    let composed = compose_capture(hwnd).map(|c| pixel_at(&c, inset, c.1 - inset));
    let ok = composed.is_some_and(|c| c.iter().zip(drawn).all(|(a, b)| a.abs_diff(b) <= 2));
    (ok, composed)
}

/// 檢查執行緒：依序驗證各項，結束時關掉視窗。
fn run_checks(
    report: &mut Report,
    ctx: &egui::Context,
    shared: &Mutex<Shared>,
    poster: &UiPoster,
    fg_before: isize,
) {
    let hwnd = || shared.lock().unwrap().hwnd;
    // 不額外要求重繪：winit 顯示視窗時會洗掉 NOACTIVATE，app 必須自己補回
    let ready = poll(Duration::from_secs(10), None, || {
        let h = hwnd();
        h != 0 && visible(h) && ex_style(h) & WS_EX_NOACTIVATE != 0
    });
    let h = hwnd();
    if !ready {
        report.check(false, format!("overlay never became visible (hwnd={h:#x})"));
        return;
    }
    let adapter = shared.lock().unwrap().adapter.clone();
    report.check(
        adapter.is_some(),
        format!(
            "renderer {:?}",
            adapter.map(|a| (a.backend, a.name, a.device_type))
        ),
    );

    // 1. 每像素透明：背景像素半透明、文字像素不透明，且 swapchain 沒有退成不透明
    let warned: Vec<String> = WARNINGS
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|w| w.contains("CompositeAlphaMode"))
        .cloned()
        .collect();
    let mut drawn = None;
    match screenshot(ctx) {
        Some(img) => {
            let [w, height] = img.size;
            // 左下角：右下有抓取紋、上方是不透明的標題列
            drawn = Some(img.pixels[(height - BG_INSET) * w + BG_INSET].to_array());
            let corner = img.pixels[(height - 2) * w + 2].a();
            let max_a = img.pixels.iter().map(|p| p.a()).max().unwrap_or(0);
            let want = (ALPHA * 255.0).round() as i32;
            report.check(
                (corner as i32 - want).abs() <= 2 && max_a == 255 && warned.is_empty(),
                format!(
                    "check1 per-pixel alpha: background a={corner} (want ~{want}), max a={max_a}, \
                     alpha-mode warnings={warned:?}"
                ),
            );
        }
        None => report.check(false, "check1 per-pixel alpha: no screenshot".into()),
    }

    // 1b. DWM 合成結果與 egui 畫的相同：底下沒有透白的重導向表面
    let drawn = drawn.unwrap_or_default();
    let (ok, composed) = composed_matches(h, drawn);
    report.check(
        ok,
        format!(
            "check1b composed background {composed:?} == drawn {drawn:?} (ex-style={:#x})",
            ex_style(h)
        ),
    );

    // 2. 不奪焦點、有工作列按鈕、置頂
    let ex = ex_style(h);
    let fg = unsafe { GetForegroundWindow() as isize };
    report.check(
        ex & WS_EX_NOACTIVATE != 0
            && ex & WS_EX_APPWINDOW != 0
            && ex & WS_EX_TOPMOST != 0
            && fg != h,
        format!(
            "check2 ex-style={ex:#x} NOACTIVATE={} APPWINDOW={} TOPMOST={} foreground before={fg_before:#x} after={fg:#x} overlay={h:#x}",
            ex & WS_EX_NOACTIVATE != 0,
            ex & WS_EX_APPWINDOW != 0,
            ex & WS_EX_TOPMOST != 0
        ),
    );

    // 3. 點擊不穿透
    let hit = hit_test_center(h);
    report.check(
        hit.is_some_and(|c| c != HTTRANSPARENT as u32 && c != 0) && ex & WS_EX_TRANSPARENT == 0,
        format!(
            "check3 WM_NCHITTEST center={hit:?} (HTCLIENT={HTCLIENT}) WS_EX_TRANSPARENT={}",
            ex & WS_EX_TRANSPARENT != 0
        ),
    );

    // 4. overlay 隱藏、只剩泡泡時事件仍被處理
    shared.lock().unwrap().minimize = true;
    let mut bubble = 0isize;
    let swapped = poll(Duration::from_secs(5), Some(ctx), || {
        bubble = own_visible_windows()
            .into_iter()
            .find(|w| *w != h)
            .unwrap_or(0);
        !visible(h) && bubble != 0
    });
    // 不再主動要求重繪：只靠投遞端的喚醒
    std::thread::sleep(Duration::from_millis(300));
    let posted = Instant::now();
    poster.post(UiEvent::SetStatus("waiting_game"));
    let mut drained = None;
    while posted.elapsed() < Duration::from_secs(1) {
        if shared.lock().unwrap().status == Some("waiting_game") {
            drained = Some(posted.elapsed());
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let bex = ex_style(bubble);
    report.check(
        swapped && drained.is_some() && bex & WS_EX_APPWINDOW != 0 && bex & WS_EX_TOPMOST != 0,
        format!(
            "check4 overlay hidden={} bubble={bubble:#x} bubble ex-style={bex:#x} \
             (APPWINDOW={} TOPMOST={}) event drained after {drained:?}",
            !visible(h),
            bex & WS_EX_APPWINDOW != 0,
            bex & WS_EX_TOPMOST != 0
        ),
    );

    // 泡泡圖示外的角落要全透明（RGB 也是 0），不能透出白色；再往外是系統畫的圓角邊框
    let corner = compose_capture(bubble).map(|c| pixel_at(&c, 6, 6));
    report.check(
        corner.is_some_and(|p| p.iter().all(|&v| v <= 2)),
        format!("check4b bubble composed corner {corner:?} (want ~[0, 0, 0, 0])"),
    );

    // 一次重繪代表展開的那一下點擊；之後不再要求重繪，量「已顯示但還沒有 NOACTIVATE」的空窗
    shared.lock().unwrap().minimize = false;
    ctx.request_repaint();
    let shown = poll(Duration::from_secs(1), None, || visible(hwnd()));
    // 顯示後取樣 300 ms：winit 在顯示之後才重寫 extended style，最長的空窗才是點擊會搶焦點的時段
    let sampled = Instant::now();
    let (mut gap_start, mut longest) = (None::<Instant>, Duration::ZERO);
    while sampled.elapsed() < Duration::from_millis(300) {
        let missing = ex_style(hwnd()) & WS_EX_NOACTIVATE == 0;
        match (missing, gap_start) {
            (true, None) => gap_start = Some(Instant::now()),
            (false, Some(at)) => {
                longest = longest.max(at.elapsed());
                gap_start = None;
            }
            _ => {}
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let gap = match gap_start {
        Some(_) => None,
        None => Some(longest),
    };
    let fg = unsafe { GetForegroundWindow() as isize };
    report.check(
        shown && gap.is_some_and(|g| g < Duration::from_millis(100)) && fg != h && fg != bubble,
        format!(
            "expand: shown={shown}, longest stretch without NOACTIVATE {gap:?} (None = never restored) ex-style={:#x} foreground={fg:#x}",
            ex_style(hwnd())
        ),
    );

    // 系統在每次顯示時重填重導向表面：展開後再比一次
    std::thread::sleep(Duration::from_millis(200));
    let (ok, composed) = composed_matches(hwnd(), drawn);
    report.check(
        ok,
        format!("expand: composed background {composed:?} == drawn {drawn:?}"),
    );
}

fn run_probe(label: &'static str, mut native: eframe::NativeOptions) -> Report {
    native.viewport = native
        .viewport
        .with_position([PARK, PARK])
        .with_inner_size([320.0, 200.0]);
    native.event_loop_builder = Some(Box::new(|b| {
        b.with_any_thread(true);
    }));
    let shared = Arc::new(Mutex::new(Shared::default()));
    let ctx_slot = Arc::new(OnceLock::<egui::Context>::new());
    let (poster, inbox) = app::channel();
    poster.post(UiEvent::SetStatus("listening"));
    let fg_before = unsafe { GetForegroundWindow() as isize };

    let checker = {
        let (shared, ctx_slot) = (Arc::clone(&shared), Arc::clone(&ctx_slot));
        std::thread::spawn(move || {
            let mut report = Report {
                label,
                lines: Vec::new(),
            };
            let deadline = Instant::now() + Duration::from_secs(10);
            while ctx_slot.get().is_none() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
            match ctx_slot.get() {
                Some(ctx) => {
                    run_checks(&mut report, ctx, &shared, &poster, fg_before);
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                    ctx.request_repaint();
                }
                None => report.check(false, "eframe never started".into()),
            }
            report
        })
    };

    let wrapper_shared = Arc::clone(&shared);
    let result = app::run(native, move |ctx| {
        let _ = ctx_slot.set(ctx.clone());
        Probe {
            app: App::new(ctx, inbox, Frame::new(ALPHA, None, |_, _, _, _| {})),
            shared: wrapper_shared,
        }
    });
    let mut report = checker.join().unwrap();
    report.check(result.is_ok(), format!("eframe exit {result:?}"));
    report
}

#[test]
#[ignore = "opens real (offscreen) windows; run manually"]
fn ui_probe() {
    log::set_logger(&WARNINGS).unwrap();
    log::set_max_level(log::LevelFilter::Warn);

    let hardware = run_probe("hardware", app::native_options());

    // 5. 強制走 WARP：同一套檢查再跑一次（eframe 在同一執行緒重用事件迴圈）
    let warp = run_probe("warp", app::warp_options());

    let failed: Vec<_> = [hardware, warp]
        .iter()
        .flat_map(|r| r.lines.iter().map(move |l| (r.label, l)))
        .filter(|(_, (ok, _))| !ok)
        .map(|(label, (_, line))| format!("{label}: {line}"))
        .collect();
    assert!(failed.is_empty(), "{failed:#?}");
}
