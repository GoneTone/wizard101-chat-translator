//! 擷取：從凍結影格裁切、WGC 影格換算出 client 區、單色畫面與擷取失敗變成 `CaptureError`（注入假的 grab，不碰真的 WGC）。

use crate::common;

use image::{Rgb, RgbImage};
use wizard101_chat_translator::region::capture::{
    CaptureError, ClientGeometry, FRAME_TIMEOUT, Frame, capture_screen, capture_window,
    capture_window_with, crop_frame, window_region,
};

use common::log_capture::{capture_logs, log_lines_with, logged};

/// 200×100 白底、(50, 30) 起 20×10 的黑色矩形，模擬凍結下來的整個 client 畫面。
fn client_frame() -> Frame {
    let mut image = RgbImage::from_pixel(200, 100, Rgb([255, 255, 255]));
    for y in 30..40 {
        for x in 50..70 {
            image.put_pixel(x, y, Rgb([0, 0, 0]));
        }
    }
    Frame {
        image,
        client_origin: (1000, 500),
        client_size: (200, 100),
    }
}

fn decode(png: &[u8]) -> RgbImage {
    image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .unwrap()
        .to_rgb8()
}

#[test]
fn crop_frame_cuts_the_selected_rect_out_of_the_frame() {
    capture_logs();
    let png = crop_frame(&client_frame(), (1050, 530, 20, 10)).unwrap();
    let decoded = decode(&png);
    assert_eq!(decoded.dimensions(), (20, 10));
    assert!(decoded.pixels().all(|p| *p == Rgb([0, 0, 0])));
    assert!(logged(&format!(
        "[region] region cropped (rect=(1050, 530, 20, 10), region=(50, 30, 20, 10), png_bytes={})",
        png.len()
    )));
}

#[test]
fn crop_frame_clips_to_the_client_area() {
    let frame = client_frame();
    let rect = (1180, 530, 40, 10);
    let (_, _, w, h) = window_region(rect, frame.client_origin, frame.client_size).unwrap();
    let decoded = decode(&crop_frame(&frame, rect).unwrap());
    assert_eq!(decoded.dimensions(), (w as u32, h as u32));
}

#[test]
fn crop_frame_outside_the_game_is_outside_game() {
    let err = crop_frame(&client_frame(), (0, 0, 10, 10)).unwrap_err();
    assert_eq!(
        err,
        CaptureError::OutsideGame(
            "selection outside game window (rect=(0, 0, 10, 10), client=(1000, 500, 200, 100))"
                .into()
        )
    );
    assert_eq!(
        err.to_string(),
        "selection outside game window (rect=(0, 0, 10, 10), client=(1000, 500, 200, 100))"
    );
}

/// client 區在螢幕 (720, 181)、大小 4×3；DWM 外框左上角在 (719, 150)。
const CLIENT: ClientGeometry = ClientGeometry {
    client_origin: (720, 181),
    client_size: (4, 3),
    frame_origin: (719, 150),
};

#[test]
fn capture_offsets_the_client_area_inside_the_dwm_frame() {
    capture_logs();
    let mut grabbed = RgbImage::from_pixel(6, 35, Rgb([255, 255, 255]));
    for y in 31..34 {
        for x in 1..5 {
            grabbed.put_pixel(x, y, Rgb([255, 0, 0]));
        }
    }
    grabbed.put_pixel(1, 31, Rgb([0, 0, 255]));
    let frame = capture_window_with(
        0x9b0fc8,
        |hwnd, timeout| {
            assert_eq!((hwnd, timeout), (0x9b0fc8, FRAME_TIMEOUT));
            Ok(grabbed)
        },
        CLIENT,
    )
    .unwrap();
    assert_eq!(frame.client_origin, (720, 181));
    assert_eq!(frame.client_size, (4, 3));
    assert_eq!(frame.image.dimensions(), (4, 3));
    assert_eq!(*frame.image.get_pixel(0, 0), Rgb([0, 0, 255]));
    assert_eq!(*frame.image.get_pixel(3, 2), Rgb([255, 0, 0]));
    let lines = log_lines_with("[region] frame captured (hwnd=0x9b0fc8, ");
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].contains(
            "[region] frame captured (hwnd=0x9b0fc8, client=(720, 181), size=4x3, frame=6x35, ms="
        ),
        "{lines:?}"
    );
    assert!(
        log_lines_with("does not contain client")
            .iter()
            .all(|l| !l.contains("frame 6x35"))
    );
}

#[test]
fn capture_logs_when_the_frame_does_not_contain_the_client() {
    capture_logs();
    let mut grabbed = RgbImage::from_pixel(3, 32, Rgb([255, 255, 255]));
    grabbed.put_pixel(1, 31, Rgb([0, 0, 255]));
    let frame = capture_window_with(0x51, |_, _| Ok(grabbed), CLIENT).unwrap();
    assert!(logged(
        "[region] frame 3x32 does not contain client 4x3 at offset (1, 31); crop may be off (DPI scaling?)"
    ));
    // 超出影格的部分補黑，Frame 仍是 client 區大小。
    assert_eq!(frame.image.dimensions(), (4, 3));
    assert_eq!(*frame.image.get_pixel(0, 0), Rgb([0, 0, 255]));
    assert_eq!(*frame.image.get_pixel(1, 0), Rgb([255, 255, 255]));
    assert_eq!(*frame.image.get_pixel(2, 0), Rgb([0, 0, 0]));
    assert_eq!(*frame.image.get_pixel(0, 1), Rgb([0, 0, 0]));
}

#[test]
fn blank_capture_is_an_error() {
    for (colour, text) in [
        ([255, 255, 255], "(255, 255, 255)"),
        ([0, 0, 0], "(0, 0, 0)"),
    ] {
        let grabbed = RgbImage::from_pixel(6, 35, Rgb(colour));
        let err = capture_window_with(0x1, |_, _| Ok(grabbed), CLIENT).err();
        assert_eq!(
            err,
            Some(CaptureError::Failed(format!(
                "blank capture (hwnd=0x1, colour={text})"
            )))
        );
    }
}

#[test]
fn capture_failures_are_wrapped_as_failed() {
    capture_logs();
    let err = capture_window_with(
        0x77,
        |_, _| {
            Err(CaptureError::Failed(
                "capture failed (hwnd=0x77): gone".into(),
            ))
        },
        CLIENT,
    )
    .err();
    assert_eq!(
        err,
        Some(CaptureError::Failed(
            "capture failed (hwnd=0x77): gone".into()
        ))
    );
    assert_eq!(err.unwrap().to_string(), "capture failed (hwnd=0x77): gone");
    assert!(log_lines_with("frame captured (hwnd=0x77,").is_empty());
}

#[test]
fn capture_window_of_a_missing_window_fails_before_wgc() {
    // ClientToScreen(NULL) 只是一個必然失敗的查詢，不會拍到任何東西。
    assert_eq!(
        capture_window(0).err(),
        Some(CaptureError::Failed(
            "capture failed (hwnd=0x0): ClientToScreen failed".into()
        ))
    );
}

#[test]
fn capture_screen_falls_back_to_black_when_grab_fails() {
    capture_logs();
    // 寬度 0 在 GetDC 之前就失敗，不會碰到螢幕。
    let image = capture_screen((100, 200, 0, 150));
    assert_eq!(image.dimensions(), (0, 150));
    assert!(logged(
        "[region] screen grab failed (monitor=(100, 200, 0, 150)): empty monitor 0x150"
    ));
}

#[test]
fn capture_screen_returns_the_grabbed_image_on_success() {
    capture_logs();
    let image = capture_screen((0, 0, 4, 3));
    assert_eq!(image.dimensions(), (4, 3));
}
