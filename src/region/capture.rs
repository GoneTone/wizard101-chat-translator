//! 遊戲畫面擷取：`capture_window` 用 WGC 把遊戲 client 區拍成凍結的 `Frame`，`crop_frame` 再依框選矩形裁成 PNG；`capture_screen` 拍整顆螢幕給選取層當暗化背景。
//!
//! 選取層顯示的畫面與送去辨識的畫面必須是同一幀，所以先凍結、放開滑鼠後才裁；WGC 只拍遊戲自己畫的東西，疊加視窗不會入鏡。

use std::io::Cursor;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use image::{ImageFormat, Rgb, RgbImage, RgbaImage, imageops};
use windows_capture::capture::{Context, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame as WgcFrame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
    MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};
use windows_capture::window::Window;
use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleBitmap,
    CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC,
    SRCCOPY, SelectObject,
};

use crate::logs::log;
use crate::reader::process::{client_size, client_to_screen, is_minimized};
use crate::ui::geometry::Rect;

/// 等 WGC 第一張影格的上限。
pub const FRAME_TIMEOUT: Duration = Duration::from_secs(2);

/// 一次 `capture_window` 拍到的完整遊戲 client 區畫面，連同 client 區的螢幕位置與大小。
pub struct Frame {
    pub image: RgbImage,
    pub client_origin: (i32, i32),
    pub client_size: (i32, i32),
}

/// 擷取失敗：框選矩形完全落在遊戲 client 區外，或等不到影格、拍到整張單色等其他失敗；附上原因。
#[derive(Debug, Clone, PartialEq)]
pub enum CaptureError {
    OutsideGame(String),
    Failed(String),
}

impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutsideGame(reason) | Self::Failed(reason) => f.write_str(reason),
        }
    }
}

/// 遊戲視窗的 client 區螢幕位置與大小，以及 DWM 看得見的外框左上角（WGC 影格的原點）。
pub struct ClientGeometry {
    pub client_origin: (i32, i32),
    pub client_size: (i32, i32),
    pub frame_origin: (i32, i32),
}

/// 螢幕矩形 (x, y, w, h) → 遊戲 client 內的矩形；超出 client 的部分裁掉，完全落在外面回 `None`。
pub fn window_region(
    (x, y, w, h): Rect,
    (ox, oy): (i32, i32),
    (cw, ch): (i32, i32),
) -> Option<Rect> {
    let (left, top) = ((x - ox).max(0), (y - oy).max(0));
    let (right, bottom) = ((x - ox + w).min(cw), (y - oy + h).min(ch));
    (right > left && bottom > top).then_some((left, top, right - left, bottom - top))
}

/// 從凍結的 `Frame` 裁出框選矩形（螢幕座標），編碼成 PNG bytes。
pub fn crop_frame(frame: &Frame, screen_rect: Rect) -> Result<Vec<u8>, CaptureError> {
    let (ox, oy) = frame.client_origin;
    let (cw, ch) = frame.client_size;
    let region =
        window_region(screen_rect, frame.client_origin, frame.client_size).ok_or_else(|| {
            CaptureError::OutsideGame(format!(
                "selection outside game window (rect={screen_rect:?}, client={:?})",
                (ox, oy, cw, ch)
            ))
        })?;
    let (rx, ry, rw, rh) = region;
    let cropped = imageops::crop_imm(&frame.image, rx as u32, ry as u32, rw as u32, rh as u32);
    let mut png = Cursor::new(Vec::new());
    cropped
        .to_image()
        .write_to(&mut png, ImageFormat::Png)
        .map_err(|e| CaptureError::Failed(format!("png encode failed: {e}")))?;
    let png = png.into_inner();
    log(format!(
        "[region] region cropped (rect={screen_rect:?}, region={region:?}, png_bytes={})",
        png.len()
    ));
    Ok(png)
}

/// 把遊戲視窗 hwnd 的整個 client 區用 WGC 拍成一張 `Frame`；任一層失敗都變成 `CaptureError`。
pub fn capture_window(hwnd: isize) -> Result<Frame, CaptureError> {
    // 最小化或視窗已消失都在啟動 WGC 之前失敗。
    reject_minimized(hwnd, is_minimized(hwnd))?;
    let client = client_geometry(hwnd)?;
    capture_window_with(hwnd, grab_window, client)
}

/// `capture_window` 的本體：影格由 `grab` 提供（測試注入假的），再從中切出 client 區。
pub fn capture_window_with(
    hwnd: isize,
    grab: impl FnOnce(isize, Duration) -> Result<RgbImage, CaptureError>,
    client: ClientGeometry,
) -> Result<Frame, CaptureError> {
    let started = Instant::now();
    let image = grab(hwnd, FRAME_TIMEOUT)?;
    let elapsed_ms = (started.elapsed().as_secs_f64() * 1000.0).round() as u64;
    let (cw, ch) = client.client_size;
    let (dx, dy) = (
        client.client_origin.0 - client.frame_origin.0,
        client.client_origin.1 - client.frame_origin.1,
    );
    let (fw, fh) = image.dimensions();
    if dx < 0 || dy < 0 || dx + cw > fw as i32 || dy + ch > fh as i32 {
        log(format!(
            "[region] frame {fw}x{fh} does not contain client {cw}x{ch} at offset ({dx}, {dy}); crop may be off (DPI scaling?)"
        ));
    }
    // 與 PIL 的 crop 相同：超出影格的部分補黑，client 區大小不變。
    let mut client_image = RgbImage::new(cw.max(0) as u32, ch.max(0) as u32);
    imageops::replace(&mut client_image, &image, -i64::from(dx), -i64::from(dy));
    let first = client_image
        .pixels()
        .next()
        .copied()
        .unwrap_or(Rgb([0, 0, 0]));
    // 單色畫面（全白、全黑）代表擷取沒拿到遊戲內容，送去 OCR 只會辨識不到字。
    if client_image.pixels().all(|p| *p == first) {
        let Rgb([r, g, b]) = first;
        return Err(CaptureError::Failed(format!(
            "blank capture (hwnd={hwnd:#x}, colour={:?})",
            (r, g, b)
        )));
    }
    log(format!(
        "[region] frame captured (hwnd={hwnd:#x}, client={:?}, size={cw}x{ch}, frame={fw}x{fh}, ms={elapsed_ms})",
        client.client_origin
    ));
    Ok(Frame {
        image: client_image,
        client_origin: client.client_origin,
        client_size: client.client_size,
    })
}

/// 把 monitor（螢幕矩形）整顆拍成圖片，只給選取層當暗化背景；失敗退回全黑，框選本身不受影響。
pub fn capture_screen(monitor: Rect) -> RgbImage {
    grab_screen(monitor).unwrap_or_else(|reason| {
        log(format!(
            "[region] screen grab failed (monitor={monitor:?}): {reason}"
        ));
        RgbImage::new(monitor.2.max(0) as u32, monitor.3.max(0) as u32)
    })
}

/// 最小化的視窗 WGC 不會送影格，直接回錯誤，不空等 `FRAME_TIMEOUT`。
fn reject_minimized(hwnd: isize, minimized: bool) -> Result<(), CaptureError> {
    if minimized {
        return Err(CaptureError::Failed(format!(
            "capture failed (hwnd={hwnd:#x}): game window is minimized"
        )));
    }
    Ok(())
}

fn client_geometry(hwnd: isize) -> Result<ClientGeometry, CaptureError> {
    let failed =
        |api: &str| CaptureError::Failed(format!("capture failed (hwnd={hwnd:#x}): {api} failed"));
    Ok(ClientGeometry {
        client_origin: client_to_screen(hwnd, 0, 0).ok_or_else(|| failed("ClientToScreen"))?,
        client_size: client_size(hwnd).ok_or_else(|| failed("GetClientRect"))?,
        frame_origin: frame_origin(hwnd)?,
    })
}

/// DWM 看得見的視窗外框左上角（螢幕座標）；不含 GetWindowRect 算進去的隱形拖曳邊框。
fn frame_origin(hwnd: isize) -> Result<(i32, i32), CaptureError> {
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    let hresult = unsafe {
        DwmGetWindowAttribute(
            hwnd as HWND,
            DWMWA_EXTENDED_FRAME_BOUNDS as u32,
            (&mut rect as *mut RECT).cast(),
            size_of::<RECT>() as u32,
        )
    };
    if hresult != 0 {
        return Err(CaptureError::Failed(format!(
            "DwmGetWindowAttribute failed (hwnd={hwnd:#x}, hresult={:#010x})",
            hresult as u32
        )));
    }
    Ok((rect.left, rect.top))
}

/// WGC 執行緒送回來的事件：第一張影格（或其轉換錯誤），或 session 先關掉了。
enum Arrival {
    Frame(Result<RgbImage, String>),
    Closed,
}

/// 收第一張影格就停的 WGC 擷取。
struct Grab {
    arrivals: Sender<Arrival>,
    sent: bool,
}

impl GraphicsCaptureApiHandler for Grab {
    type Flags = Sender<Arrival>;
    type Error = String;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(Self {
            arrivals: ctx.flags,
            sent: false,
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut WgcFrame,
        control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        if !self.sent {
            self.sent = true;
            let (w, h) = (frame.width(), frame.height());
            // 影格緩衝只在回呼期間有效，先複製出來。
            let image = frame
                .buffer()
                .map_err(|e| e.to_string())
                .and_then(|buffer| {
                    rgba_to_rgb(w, h, buffer.as_nopadding_buffer(&mut Vec::new()).to_vec())
                });
            let _ = self.arrivals.send(Arrival::Frame(image));
        }
        control.stop();
        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        let _ = self.arrivals.send(Arrival::Closed);
        Ok(())
    }
}

/// WGC 的 RGBA 影格緩衝 → RGB 圖；緩衝大小與寬高對不上時回錯誤。
fn rgba_to_rgb(w: u32, h: u32, rgba: Vec<u8>) -> Result<RgbImage, String> {
    RgbaImage::from_raw(w, h, rgba)
        .map(|rgba| image::DynamicImage::ImageRgba8(rgba).to_rgb8())
        .ok_or_else(|| format!("buffer does not match {w}x{h}"))
}

/// 用 WGC 拍 hwnd 的一張影格（範圍是 DWM 看得見的外框）。
///
/// 擷取在 windows-capture 自己的執行緒跑（free-threaded）：呼叫端執行緒的 COM 狀態不影響 WinRT 初始化。
fn grab_window(hwnd: isize, timeout: Duration) -> Result<RgbImage, CaptureError> {
    let (arrivals, received) = mpsc::channel();
    let settings = Settings::new(
        Window::from_raw_hwnd(hwnd as *mut _),
        CursorCaptureSettings::WithoutCursor,
        DrawBorderSettings::WithoutBorder,
        SecondaryWindowSettings::Default,
        MinimumUpdateIntervalSettings::Default,
        DirtyRegionSettings::Default,
        ColorFormat::Rgba8,
        arrivals,
    );
    let control = Grab::start_free_threaded(settings)
        .map_err(|e| CaptureError::Failed(format!("capture failed (hwnd={hwnd:#x}): {e}")))?;
    let image = wait_for_frame(&received, hwnd, timeout);
    if let Err(e) = control.stop() {
        log(format!(
            "[region] capture stop failed (hwnd={hwnd:#x}): {e}"
        ));
    }
    image
}

/// 等 WGC 執行緒送回第一張影格，最多等 `timeout`。
fn wait_for_frame(
    received: &Receiver<Arrival>,
    hwnd: isize,
    timeout: Duration,
) -> Result<RgbImage, CaptureError> {
    match received.recv_timeout(timeout) {
        Ok(Arrival::Frame(Ok(image))) => Ok(image),
        Ok(Arrival::Frame(Err(e))) => Err(CaptureError::Failed(format!(
            "frame conversion failed (hwnd={hwnd:#x}): {e}"
        ))),
        Ok(Arrival::Closed) | Err(RecvTimeoutError::Disconnected) => Err(CaptureError::Failed(
            format!("capture session closed before a frame arrived (hwnd={hwnd:#x})"),
        )),
        Err(RecvTimeoutError::Timeout) => Err(CaptureError::Failed(format!(
            "no frame within {:?}s (hwnd={hwnd:#x})",
            timeout.as_secs_f64()
        ))),
    }
}

/// 用 GDI 把螢幕矩形拍下來（含分層視窗，同 PIL 的 `ImageGrab.grab`）。
fn grab_screen((x, y, w, h): Rect) -> Result<RgbImage, String> {
    if w <= 0 || h <= 0 {
        return Err(format!("empty monitor {w}x{h}"));
    }
    let mut bgra = vec![0u8; w as usize * h as usize * 4];
    let result = unsafe {
        let screen = GetDC(std::ptr::null_mut());
        if screen.is_null() {
            return Err("GetDC failed".into());
        }
        let memory = CreateCompatibleDC(screen);
        let bitmap = CreateCompatibleBitmap(screen, w, h);
        let result = if memory.is_null() {
            Err("CreateCompatibleDC failed")
        } else if bitmap.is_null() {
            Err("CreateCompatibleBitmap failed")
        } else {
            let previous = SelectObject(memory, bitmap);
            let blitted = BitBlt(memory, 0, 0, w, h, screen, x, y, SRCCOPY | CAPTUREBLT) != 0;
            // GetDIBits 要求點陣圖不能還選在 DC 裡。
            SelectObject(memory, previous);
            let mut info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    ..Default::default()
                },
                ..Default::default()
            };
            if !blitted {
                Err("BitBlt failed")
            } else if GetDIBits(
                memory,
                bitmap,
                0,
                h as u32,
                bgra.as_mut_ptr().cast(),
                &mut info,
                DIB_RGB_COLORS,
            ) != h
            {
                Err("GetDIBits failed")
            } else {
                Ok(())
            }
        };
        DeleteObject(bitmap);
        DeleteDC(memory);
        ReleaseDC(std::ptr::null_mut(), screen);
        result
    };
    result.map_err(str::to_owned)?;
    Ok(RgbImage::from_fn(w as u32, h as u32, |px, py| {
        let i = (py as usize * w as usize + px as usize) * 4;
        Rgb([bgra[i + 2], bgra[i + 1], bgra[i]])
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grab_window_converts_the_first_frame_from_rgba() {
        let mut rgba = vec![0u8; 3 * 2 * 4];
        rgba[..4].copy_from_slice(&[0, 0, 255, 255]);
        let image = rgba_to_rgb(3, 2, rgba).unwrap();
        assert_eq!(image.dimensions(), (3, 2));
        assert_eq!(image.get_pixel(0, 0), &Rgb([0, 0, 255]));
        assert_eq!(
            rgba_to_rgb(3, 2, vec![0; 5]),
            Err("buffer does not match 3x2".to_string())
        );
    }

    #[test]
    fn a_minimized_game_fails_immediately() {
        assert_eq!(reject_minimized(0x1, false), Ok(()));
        assert_eq!(
            reject_minimized(0x1, true),
            Err(CaptureError::Failed(
                "capture failed (hwnd=0x1): game window is minimized".into()
            ))
        );
    }

    #[test]
    fn grab_times_out_after_two_seconds() {
        assert_eq!(FRAME_TIMEOUT, Duration::from_secs(2));
        let (_arrivals, received) = mpsc::channel::<Arrival>();
        let timeout = Duration::from_millis(50);
        let started = Instant::now();
        let result = wait_for_frame(&received, 0x1, timeout);
        assert!(started.elapsed() >= timeout);
        assert_eq!(
            result.err(),
            Some(CaptureError::Failed(
                "no frame within 0.05s (hwnd=0x1)".into()
            ))
        );
        assert_eq!(format!("{:?}s", FRAME_TIMEOUT.as_secs_f64()), "2.0s");
    }

    #[test]
    fn first_frame_is_returned() {
        let (arrivals, received) = mpsc::channel();
        let image = RgbImage::from_pixel(3, 2, Rgb([0, 0, 255]));
        arrivals.send(Arrival::Frame(Ok(image.clone()))).unwrap();
        assert_eq!(wait_for_frame(&received, 0x1, FRAME_TIMEOUT), Ok(image));
    }

    #[test]
    fn session_closed_before_a_frame_is_an_error() {
        let (arrivals, received) = mpsc::channel();
        arrivals.send(Arrival::Closed).unwrap();
        assert_eq!(
            wait_for_frame(&received, 0x1, FRAME_TIMEOUT).err(),
            Some(CaptureError::Failed(
                "capture session closed before a frame arrived (hwnd=0x1)".into()
            ))
        );
        drop(arrivals);
        assert!(matches!(
            wait_for_frame(&received, 0x1, FRAME_TIMEOUT),
            Err(CaptureError::Failed(m)) if m.contains("closed")
        ));
    }

    #[test]
    fn conversion_failure_is_an_error() {
        let (arrivals, received) = mpsc::channel();
        arrivals.send(Arrival::Frame(Err("bad".into()))).unwrap();
        assert_eq!(
            wait_for_frame(&received, 0x1, FRAME_TIMEOUT).err(),
            Some(CaptureError::Failed(
                "frame conversion failed (hwnd=0x1): bad".into()
            ))
        );
    }
}
