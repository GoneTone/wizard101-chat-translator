//! 視窗架構驗證（egui 的退場條件）：開出真的 overlay（含縮成泡泡）、翻譯輸入框與框選的框和結果卡、停在螢幕外，以程式讀回各項行為。
//!
//! 會短暫出現螢幕外視窗與工作列按鈕，所以預設略過；一個程序只能建一次事件迴圈，每個 probe 要各自依名稱手動跑：
//! `cargo test --test it ui_probe::<名稱> -- --ignored --exact --nocapture`
//! （`ui_probe`、`settings_window_has_a_dark_caption_and_a_taskbar_button`）

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

use eframe::egui::{self, ColorImage, ViewportCommand};
use eframe::egui_wgpu::wgpu;
use image::{Rgb, RgbImage};
use tokio_util::sync::CancellationToken;
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
    EnumWindows, GWL_STYLE, GetForegroundWindow, GetWindowLongPtrW, GetWindowRect,
    GetWindowThreadProcessId, HTCLIENT, HTTRANSPARENT, IsWindowVisible, SMTO_ABORTIFHUNG,
    SendMessageTimeoutW, WM_NCHITTEST, WS_CAPTION, WS_EX_APPWINDOW, WS_EX_NOACTIVATE,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT,
};
use winit::platform::windows::EventLoopBuilderExtWindows;
use wizard101_chat_translator::config::services::ServiceSpec;
use wizard101_chat_translator::config::{Config, app_name};
use wizard101_chat_translator::i18n::t;
use wizard101_chat_translator::input::dispatch::GameInputEvents;
use wizard101_chat_translator::logs::log;
use wizard101_chat_translator::region::capture::Frame as CaptureFrame;
use wizard101_chat_translator::region::pipeline::RegionResult;
use wizard101_chat_translator::ui::app::{self, App};
use wizard101_chat_translator::ui::events::{UiEvent, UiPoster};
use wizard101_chat_translator::ui::geometry::Rect;
use wizard101_chat_translator::ui::input_box::{self, InputBox};
use wizard101_chat_translator::ui::input_host::Desktop;
use wizard101_chat_translator::ui::overlay::frame::Frame;
use wizard101_chat_translator::ui::palette::{BAR, BG, GRIP};
use wizard101_chat_translator::ui::region::card::ANCHOR_GAP;
use wizard101_chat_translator::ui::region::flow::{
    CaptureKind, RegionCapturer, RegionDesktop, RegionFlow, RegionRunner, Shot,
};
use wizard101_chat_translator::ui::region::region_box;
use wizard101_chat_translator::ui::region::selector::{self, SelectorAction};
use wizard101_chat_translator::ui::run;
use wizard101_chat_translator::ui::settings::env::SettingsEnv;
use wizard101_chat_translator::ui::settings::theme;
use wizard101_chat_translator::ui::settings_host::{SettingsHost, WindowOps};
use wizard101_chat_translator::ui::win32::{
    ex_style, hwnd_of, owner_of, scale_at, window_rect, window_title,
};

const PARK: f32 = 10000.0;
/// 輸入框的螢幕外游標位置（實體像素）與假的工作區：box 貼在游標下、不被拉回真的螢幕。
const BOX_CURSOR: (i32, i32) = (10000, 10000);
const BOX_AREA: (i32, i32, i32, i32) = (9000, 9000, 4000, 4000);
const ALPHA: f32 = 0.8;
/// 框選用的假遊戲視窗與它螢幕外的 client 區，以及其中的框選矩形（實體像素）。
const REGION_GAME: isize = 0x5EED;
const REGION_CLIENT: Rect = (11000, 11000, 800, 600);
const REGION_RECT: Rect = (11100, 11100, 300, 120);
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
    /// 選取畫面一開就直接以這個矩形完成框選（不經滑鼠，選取畫面也就不會建立）。
    select: Option<Rect>,
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
        if let Some(flow) = self.app.region_flow_mut()
            && flow.selector_mut().is_some()
            && let Some(rect) = s.select.take()
        {
            flow.on_selector(SelectorAction::Selected(rect));
        }
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

/// 框選的桌面替身：不切前景。
struct RegionFake;

impl RegionDesktop for RegionFake {
    fn foreground(&mut self, _hwnd: isize) {}
    fn find_game(&self) -> Option<isize> {
        Some(REGION_GAME)
    }
    fn cursor(&self) -> (i32, i32) {
        BOX_CURSOR
    }
}

/// 框選的擷取替身：當場投遞螢幕外 client 區的漸層圖，不截圖。
struct RegionShots(UiPoster);

impl RegionCapturer for RegionShots {
    fn capture(&self, id: u64, _hwnd: isize, kind: CaptureKind) {
        let (x, y, w, h) = REGION_CLIENT;
        let frame = Arc::new(CaptureFrame {
            image: RgbImage::from_fn(w as u32, h as u32, |x, y| Rgb([x as u8, y as u8, 0])),
            client_origin: (x, y),
            client_size: (w, h),
        });
        let selector = (kind == CaptureKind::Select).then(|| {
            (
                REGION_CLIENT,
                selector::images(&frame, &RgbImage::new(1, 1)),
            )
        });
        self.0.post(UiEvent::RegionCaptured {
            id,
            result: Ok(Shot { frame, selector }),
        });
    }
}

/// 立刻回一筆譯文，卡片才會從辨識中換成譯文。
struct RegionAnswer(UiPoster);

impl RegionRunner for RegionAnswer {
    fn run(&self, _png: Vec<u8>, _rect: Rect, session: u64, _cancel: CancellationToken) {
        self.0.post(UiEvent::RegionDone {
            session,
            result: Ok(RegionResult {
                text: "譯文".into(),
                source: "Talk to Merle".into(),
            }),
        });
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

/// 合成結果與 egui 畫的不同的一點：x、y、合成值、egui 值。
type Mismatch = (u32, u32, [u8; 4], [u8; 4]);

/// 合成結果在四角與四邊中點與 `want` 不同之處；系統的圓角、邊框與陰影都落在這些點上。
///
/// `want` 收 (x, y, 寬, 高) 回應有的像素，回 `None`（例如尺寸對不上）時整個回 `None`。
fn edge_mismatches(
    hwnd: isize,
    want: impl Fn(u32, u32, u32, u32) -> Option<[u8; 4]>,
) -> Option<Vec<Mismatch>> {
    let c = compose_capture(hwnd)?;
    let (w, h) = (c.0, c.1);
    let points = [
        (0, 0),
        (w / 2, 0),
        (w - 1, 0),
        (0, h / 2),
        (w - 1, h / 2),
        (0, h - 1),
        (w / 2, h - 1),
        (w - 1, h - 1),
    ];
    let mut found = Vec::new();
    for (x, y) in points {
        let (got, want) = (pixel_at(&c, x, y), want(x, y, w, h)?);
        if !got.iter().zip(want).all(|(a, b)| a.abs_diff(b) <= 2) {
            found.push((x, y, got, want));
        }
    }
    Some(found)
}

/// egui 自己畫出的像素（截圖與擷取尺寸相同時）。
fn drawn_pixel(img: &ColorImage) -> impl Fn(u32, u32, u32, u32) -> Option<[u8; 4]> + '_ {
    |x, y, w, h| {
        ([w as usize, h as usize] == img.size)
            .then(|| img.pixels[y as usize * w as usize + x as usize].to_array())
    }
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
    let shot = screenshot(ctx);
    match &shot {
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

    // 1c. 邊緣沒有系統畫的圓角、邊框或陰影
    let edges = shot
        .as_deref()
        .and_then(|img| edge_mismatches(h, drawn_pixel(img)));
    report.check(
        edges.as_ref().is_some_and(Vec::is_empty),
        format!("check1c composed edges differing from drawn (x, y, composed, drawn): {edges:?}"),
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

    // 4. 根視窗本身縮成泡泡（不隱藏、不另開視窗），事件照常即時處理
    let overlay = window_rect(h);
    shared.lock().unwrap().minimize = true;
    let shrunk = poll(Duration::from_secs(5), Some(ctx), || {
        let (_, _, w, height) = window_rect(h);
        w < overlay.2 && w == height
    });
    let others = own_visible_windows()
        .into_iter()
        .filter(|w| *w != h)
        .count();
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
    let bex = ex_style(h);
    report.check(
        shrunk
            && visible(h)
            && others == 0
            && drained.is_some_and(|d| d < Duration::from_millis(100))
            && bex & WS_EX_APPWINDOW != 0
            && bex & WS_EX_TOPMOST != 0
            && bex & WS_EX_NOACTIVATE != 0,
        format!(
            "check4 overlay window became the bubble: rect={:?} (overlay {overlay:?}) visible={} \
             other windows={others} ex-style={bex:#x} (APPWINDOW={} TOPMOST={} NOACTIVATE={}) \
             event drained after {drained:?} (want < 100 ms)",
            window_rect(h),
            visible(h),
            bex & WS_EX_APPWINDOW != 0,
            bex & WS_EX_TOPMOST != 0,
            bex & WS_EX_NOACTIVATE != 0
        ),
    );

    // 圖示外的三個角要全透明：不能透出白色的重導向表面，也不能有系統畫的圓角邊框（右下角是圖示本身）
    let corners = compose_capture(h).map(|c| {
        let (w, h) = (c.0 - 1, c.1 - 1);
        [(0, 0), (w, 0), (0, h)].map(|(x, y)| pixel_at(&c, x, y))
    });
    report.check(
        corners.is_some_and(|ps| ps.iter().all(|p| *p == [0; 4])),
        format!("check4b bubble composed corners {corners:?} (top-left, top-right, bottom-left; want all [0, 0, 0, 0])"),
    );

    // 一次重繪代表展開的那一下點擊；之後不再要求重繪，量「展開後沒有 NOACTIVATE」的空窗
    shared.lock().unwrap().minimize = false;
    ctx.request_repaint();
    let shown = poll(Duration::from_secs(1), None, || {
        window_rect(hwnd()) == overlay
    });
    // 展開後取樣 300 ms：根視窗不再隱藏又顯示，NOACTIVATE 應該從頭到尾都在
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
        shown && gap.is_some_and(|g| g < Duration::from_millis(100)) && fg != h,
        format!(
            "expand: overlay rect restored={shown} {:?}, longest stretch without NOACTIVATE {gap:?} (None = never restored) ex-style={:#x} foreground={fg:#x}",
            window_rect(hwnd()),
            ex_style(hwnd())
        ),
    );

    // 系統在每次顯示或改變大小時可能重填重導向表面：展開後再比一次
    std::thread::sleep(Duration::from_millis(200));
    let (ok, composed) = composed_matches(hwnd(), drawn);
    report.check(
        ok,
        format!("expand: composed background {composed:?} == drawn {drawn:?}"),
    );
    // winit 顯示視窗時會重算外框：展開後邊緣也要維持
    let edges = shot
        .as_deref()
        .and_then(|img| edge_mismatches(hwnd(), drawn_pixel(img)));
    report.check(
        edges.as_ref().is_some_and(Vec::is_empty),
        format!("expand: composed edges differing from drawn: {edges:?}"),
    );

    // 6. 無錨點的翻譯輸入框（螢幕外、不搶前景）：四邊沒有 Windows 11 的灰邊與圓角，底色為 BG
    // 擷取本身也會留下本程序的小視窗：只認開框之後才出現的那個
    let existing = own_visible_windows();
    poster.post(UiEvent::ShowInputBox { retarget: None });
    let mut input = 0isize;
    let opened = poll(Duration::from_secs(5), Some(ctx), || {
        input = own_visible_windows()
            .into_iter()
            .find(|w| !existing.contains(w))
            .unwrap_or(0);
        input != 0
    });
    std::thread::sleep(Duration::from_millis(300));
    // 上緣是拖曳條，其餘是 BG
    let (bar, bg) = (BAR.to_array(), input_box::BG.to_array());
    let edges = opened
        .then(|| edge_mismatches(input, |_, y, _, _| Some(if y == 0 { bar } else { bg })))
        .flatten();
    let (ex, owner) = (ex_style(input), owner_of(input));
    let fg = unsafe { GetForegroundWindow() as isize };
    report.check(
        edges.as_ref().is_some_and(Vec::is_empty)
            && ex & WS_EX_APPWINDOW == 0
            && owner == hwnd()
            && fg == fg_before,
        format!(
            "check6 input box={input:#x} rect={:?} composed edges differing (x, y, composed, want) \
             {edges:?} ex-style={ex:#x} APPWINDOW={} owner={owner:#x} (overlay={:#x}) \
             foreground before={fg_before:#x} after={fg:#x}",
            window_rect(input),
            ex & WS_EX_APPWINDOW != 0,
            hwnd()
        ),
    );

    // 7. 框選框與結果卡（螢幕外）：不搶前景、沒有工作列按鈕、四邊沒有系統灰邊，卡片底色的不透明度等於 overlay
    let existing = own_visible_windows();
    shared.lock().unwrap().select = Some(REGION_RECT);
    poster.post(UiEvent::RegionHotkey { hwnd: REGION_GAME });
    let want_box = region_box::window_rect(REGION_RECT);
    let card_at = (REGION_RECT.0, REGION_RECT.1 + REGION_RECT.3 + ANCHOR_GAP);
    let (mut boxed, mut card) = (0isize, 0isize);
    let opened = poll(Duration::from_secs(5), Some(ctx), || {
        for w in own_visible_windows()
            .into_iter()
            .filter(|w| !existing.contains(w))
        {
            let rect = window_rect(w);
            if rect == want_box {
                boxed = w;
            } else if (rect.0, rect.1) == card_at {
                card = w;
            }
        }
        boxed != 0 && card != 0
    });
    std::thread::sleep(Duration::from_millis(300));
    let fg = unsafe { GetForegroundWindow() as isize };
    for (name, w) in [("box", boxed), ("card", card)] {
        let (ex, owner) = (ex_style(w), owner_of(w));
        report.check(
            opened && ex & WS_EX_APPWINDOW == 0 && owner == hwnd() && fg == fg_before,
            format!(
                "check7 region {name}={w:#x} rect={:?} ex-style={ex:#x} APPWINDOW={} \
                 owner={owner:#x} (overlay={:#x}) foreground before={fg_before:#x} after={fg:#x}",
                window_rect(w),
                ex & WS_EX_APPWINDOW != 0,
                hwnd()
            ),
        );
    }
    // 框只畫框線與把手：四邊全透明，但整個視窗都收得到滑鼠
    let edges = opened
        .then(|| edge_mismatches(boxed, |_, _, _, _| Some([0; 4])))
        .flatten();
    let hit = hit_test_center(boxed);
    report.check(
        edges.as_ref().is_some_and(Vec::is_empty)
            && hit.is_some_and(|c| c != HTTRANSPARENT as u32 && c != 0),
        format!(
            "check7b region box composed edges differing (x, y, composed, want) {edges:?} \
             WM_NCHITTEST center={hit:?}"
        ),
    );
    let (border, bg) = (
        GRIP.gamma_multiply(ALPHA).to_array(),
        BG.gamma_multiply(ALPHA).to_array(),
    );
    let edges = opened
        .then(|| edge_mismatches(card, |_, _, _, _| Some(border)))
        .flatten();
    // 左側留白裡、邊框內側的一點：只有底色
    let inside = compose_capture(card).map(|c| pixel_at(&c, 4, c.1 / 2));
    let want_a = (ALPHA * 255.0).round() as u8;
    report.check(
        edges.as_ref().is_some_and(Vec::is_empty)
            && inside.is_some_and(|p| p.iter().zip(bg).all(|(a, b)| a.abs_diff(b) <= 2)),
        format!(
            "check7c region card composed edges differing (x, y, composed, want) {edges:?} \
             background {inside:?} (want {bg:?}, alpha ~{want_a})"
        ),
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
    let region_poster = poster.clone();
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
    let result = run::run(native, move |ctx| {
        let _ = ctx_slot.set(ctx.clone());
        let input = InputBox::new(
            Box::new(|_| {}),
            Box::new(|_, _| {}),
            Arc::new(AtomicBool::new(false)),
            || 0,
            || BOX_CURSOR,
        );
        let events = GameInputEvents::new(Arc::default(), || 0);
        let region = RegionFlow::new(
            Box::new(RegionFake),
            Box::new(RegionShots(region_poster.clone())),
            Box::new(RegionAnswer(region_poster)),
            Arc::new(AtomicBool::new(false)),
        );
        let mut app = App::new(ctx, inbox, Frame::new(ALPHA, None, |_, _, _, _| {}));
        app.set_input(input, events, Arc::new(RwLock::new(Config::new())));
        app.set_region(region);
        let desktop = Desktop {
            force_foreground: |_| false,
            work_area_at: |_, _| BOX_AREA,
            trigger_held: || false,
            typing: || false,
        };
        app.set_input_desktop(desktop);
        app.set_region_desktop(desktop);
        Probe {
            app,
            shared: wrapper_shared,
        }
    });
    let mut report = checker.join().unwrap();
    report.check(result.is_ok(), format!("eframe exit {result:?}"));
    report
}

#[test]
#[ignore = "opens real offscreen windows; needs a desktop session, run alone with --exact"]
fn ui_probe() {
    log::set_logger(&WARNINGS).unwrap();
    log::set_max_level(log::LevelFilter::Warn);

    let hardware = run_probe("hardware", run::native_options());

    // 5. 強制走 WARP：同一套檢查再跑一次（eframe 在同一執行緒重用事件迴圈）
    let warp = run_probe("warp", run::warp_options());

    let failed: Vec<_> = [hardware, warp]
        .iter()
        .flat_map(|r| r.lines.iter().map(move |l| (r.label, l)))
        .filter(|(_, (ok, _))| !ok)
        .map(|(label, (_, line))| format!("{label}: {line}"))
        .collect();
    assert!(failed.is_empty(), "{failed:#?}");
}

/// 設定視窗 probe 用的 env：背景工作一律不做。
struct NoopEnv;

impl SettingsEnv for NoopEnv {
    fn test_connection(&self, _round: u64, _spec: ServiceSpec, _target: String) {}
    fn list_models(&self, _round: u64, _spec: ServiceSpec) {}
    fn check_update(&self, _round: u64) {}
    fn record_hotkey(&self, _round: u64) {}
    fn cancel_recording(&self) {}
    fn open_folder(&self, _path: &Path) {}
    fn has_cache(&self) -> bool {
        false
    }
    fn clear_cache(&self) -> usize {
        0
    }
}

/// 包住 App：檢查執行緒要求時按一次 ⚙，並回報 overlay 的 HWND。
struct SettingsProbe {
    app: App,
    open: Arc<AtomicBool>,
    hwnd: Arc<AtomicIsize>,
}

impl eframe::App for SettingsProbe {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.app.logic(ctx, frame);
        if self.open.swap(false, Ordering::SeqCst) {
            self.app.open_settings();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.app.ui(ui, frame);
        self.hwnd.store(hwnd_of(frame), Ordering::SeqCst);
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        self.app.clear_color(visuals)
    }
}

/// 兩個顏色每個通道都差不到 `tolerance`。
fn close_to(got: [u8; 4], want: [u8; 4], tolerance: u8) -> bool {
    got.iter()
        .zip(want)
        .all(|(a, b)| a.abs_diff(b) <= tolerance)
}

/// 檢查執行緒：等 overlay 出來、按 ⚙、找到設定視窗後讀回樣式與合成後的顏色，結束時關掉 overlay。
fn check_settings_window(
    report: &mut Report,
    ctx: &egui::Context,
    overlay: &AtomicIsize,
    open: &AtomicBool,
) {
    let ready = poll(Duration::from_secs(10), Some(ctx), || {
        let h = overlay.load(Ordering::SeqCst);
        h != 0 && visible(h)
    });
    if !ready {
        report.check(false, "overlay never became visible".into());
        return;
    }
    let existing = own_visible_windows();
    let fg_before = unsafe { GetForegroundWindow() as isize };
    open.store(true, Ordering::SeqCst);
    let title = t("settings.title", &[("app", &app_name())]);
    let mut win = 0isize;
    let opened = poll(Duration::from_secs(5), Some(ctx), || {
        win = own_visible_windows()
            .into_iter()
            .find(|w| !existing.contains(w) && window_title(*w) == title)
            .unwrap_or(0);
        win != 0
    });
    if !opened {
        report.check(
            false,
            format!("settings window {title:?} never became visible"),
        );
        return;
    }
    // 讓 DWM 套上深色標題列、egui 畫完第一幀
    std::thread::sleep(Duration::from_millis(500));
    let style = unsafe { GetWindowLongPtrW(win as HWND, GWL_STYLE) } as u32;
    let (ex, owner) = (ex_style(win), owner_of(win));
    // 搶前景換成了替身，但一般視窗顯示時系統仍可能直接啟用它：前景只記下、不當成檢查項
    let fg = unsafe { GetForegroundWindow() as isize };
    report.check(
        style & WS_CAPTION == WS_CAPTION
            && (ex & WS_EX_APPWINDOW != 0 || owner == 0)
            && ex & WS_EX_TOPMOST == 0,
        format!(
            "settings window={win:#x} rect={:?} style={style:#x} CAPTION={} ex-style={ex:#x} \
             APPWINDOW={} TOPMOST={} owner={owner:#x} foreground before={fg_before:#x} after={fg:#x}",
            window_rect(win),
            style & WS_CAPTION == WS_CAPTION,
            ex & WS_EX_APPWINDOW != 0,
            ex & WS_EX_TOPMOST != 0
        ),
    );

    let (x, y, _, _) = window_rect(win);
    let scale = scale_at(x, y);
    let capture = compose_capture(win);
    let caption_want = BG.to_array();
    let caption = capture
        .as_ref()
        .map(|c| pixel_at(c, c.0 / 2, (10.0 * scale) as u32));
    report.check(
        caption.is_some_and(|p| close_to(p, caption_want, 6)),
        format!("settings caption composed {caption:?} (want ~{caption_want:?})"),
    );
    // 底部按鈕列左側（側欄右邊、儲存被擋提示的位置，沒被擋時只有底色）
    let content_want = theme::WINDOW_BG.to_array();
    let content = capture.as_ref().map(|c| {
        let px = ((theme::SIDEBAR_WIDTH + 16.0) * scale) as u32;
        pixel_at(c, px, c.1 - (8.0 * scale) as u32)
    });
    report.check(
        content.is_some_and(|p| close_to(p, content_want, 2) && p[3] == 255),
        format!("settings content composed {content:?} (want {content_want:?}, opaque)"),
    );
}

#[test]
#[ignore = "opens a real offscreen window; needs a desktop session, run alone with --exact"]
fn settings_window_has_a_dark_caption_and_a_taskbar_button() {
    let mut options = run::native_options();
    options.viewport = options
        .viewport
        .with_position([PARK, PARK])
        .with_inner_size([320.0, 200.0]);
    options.event_loop_builder = Some(Box::new(|b| {
        b.with_any_thread(true);
    }));
    let ctx_slot = Arc::new(OnceLock::<egui::Context>::new());
    let (open, overlay) = (
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicIsize::new(0)),
    );
    let (_poster, inbox) = app::channel();

    let checker = {
        let (ctx_slot, open, overlay) = (
            Arc::clone(&ctx_slot),
            Arc::clone(&open),
            Arc::clone(&overlay),
        );
        std::thread::spawn(move || {
            let mut report = Report {
                label: "settings",
                lines: Vec::new(),
            };
            let deadline = Instant::now() + Duration::from_secs(10);
            while ctx_slot.get().is_none() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
            match ctx_slot.get() {
                Some(ctx) => {
                    check_settings_window(&mut report, ctx, &overlay, &open);
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                    ctx.request_repaint();
                }
                None => report.check(false, "eframe never started".into()),
            }
            report
        })
    };

    let (probe_open, probe_hwnd) = (Arc::clone(&open), Arc::clone(&overlay));
    let result = run::run(options, move |ctx| {
        let _ = ctx_slot.set(ctx.clone());
        let mut host = SettingsHost::new(Box::new(NoopEnv));
        host.set_window_ops(WindowOps {
            force_foreground: |_| false,
            cursor_work_area: || (BOX_AREA, scale_at(BOX_AREA.0, BOX_AREA.1)),
            ..WindowOps::real()
        });
        let mut app = App::new(ctx, inbox, Frame::new(ALPHA, None, |_, _, _, _| {}));
        app.set_settings(
            host,
            Arc::new(RwLock::new(Config::new())),
            Box::new(|_| None),
        );
        SettingsProbe {
            app,
            open: probe_open,
            hwnd: probe_hwnd,
        }
    });
    let mut report = checker.join().unwrap();
    report.check(result.is_ok(), format!("eframe exit {result:?}"));
    let failed: Vec<_> = report
        .lines
        .iter()
        .filter(|(ok, _)| !ok)
        .map(|(_, line)| line.clone())
        .collect();
    assert!(failed.is_empty(), "{failed:#?}");
}
