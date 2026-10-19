//! 翻譯輸入框中需要斷言 log 的測試。

use crate::common;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use common::log_capture::{capture_logs, log_lines_with};
use wizard101_chat_translator::input::dispatch::InputBoxControl;
use wizard101_chat_translator::ui::input_box::InputBox;

fn input_box(foreground: fn() -> isize) -> InputBox {
    InputBox::new(
        Box::new(|_| {}),
        Box::new(|_, _| {}),
        Arc::new(AtomicBool::new(false)),
        foreground,
        || (640, 500),
    )
}

/// 含 `marker` 的 log 行（去掉時間戳）。
fn logged(marker: &str) -> Vec<String> {
    log_lines_with(marker)
        .iter()
        .map(|l| l.split_once(' ').unwrap().1.to_owned())
        .collect()
}

#[test]
fn show_records_the_foreground_as_target_and_logs_the_anchor() {
    capture_logs();
    let mut anchored = input_box(|| 0x5A11);
    anchored.set_anchor((11, 22, 33, 44));
    anchored.show();
    assert_eq!(anchored.target_hwnd(), 0x5A11);
    assert_eq!(
        logged("target_hwnd=0x5a11"),
        ["[input] box opened (target_hwnd=0x5a11, anchor=(11, 22, 33, 44))"]
    );

    input_box(|| 0x5A12).show();
    assert_eq!(
        logged("target_hwnd=0x5a12"),
        ["[input] box opened (target_hwnd=0x5a12, anchor=None)"]
    );
}

#[test]
fn retarget_logs_only_real_changes() {
    capture_logs();
    let mut b = input_box(|| 0x5A13);
    b.show();
    b.retarget(0x5A14);
    b.retarget(0);
    b.retarget(0x5A14);
    assert_eq!(
        logged("[input] retarget"),
        ["[input] retarget 0x5a13 -> 0x5a14"]
    );
}
