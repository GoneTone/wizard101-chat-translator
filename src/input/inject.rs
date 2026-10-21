//! 按鍵注入：以 `SendInput` 送出按下、放開、Unicode 字元與遮罩鍵，對應 `_winkeyboard` 的同名函式。

use windows_sys::Win32::Foundation::GetLastError;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, SendInput,
    VK_CONTROL, VK_MENU,
};

use crate::input::hotkeys::MASK_VK;
use crate::input::keys::{ALT_GR, Code, scan_code_to_vk};
use crate::logs::log;

/// 送出按鍵的介面；測試換成假的，不可真的送鍵。
pub trait Injector: Send + Sync {
    /// 按下 `code`（`keyboard.press`）。
    fn press(&self, code: Code);
    /// 放開 `code`（`keyboard.release`）。
    fn release(&self, code: Code);
    /// 直接打出一個字元，不經鍵盤配置（`type_unicode`）。
    fn unicode(&self, ch: char);
    /// 送一次遮罩鍵的按下與放開。
    fn mask(&self);
}

/// 以 `SendInput` 實際送鍵。
pub struct SendInputInjector;

impl SendInputInjector {
    /// `_winkeyboard._send_event`：alt gr 拆成 ctrl+alt；負數 code 是沒有 scan code 的 vk。
    fn send_event(code: Code, flags: u32) {
        let events: Vec<(u16, Code)> = if code == ALT_GR {
            vec![(VK_CONTROL, code), (VK_MENU, code)]
        } else if code > 0 {
            vec![(scan_code_to_vk(code), code)]
        } else {
            vec![(scan_code_to_vk(code), 0)]
        };
        // keybd_event 的 bScan 是 BYTE，只取低 8 位
        send(
            events
                .into_iter()
                .map(|(vk, scan)| key_input(vk, scan as u8 as u16, flags)),
        );
    }
}

impl Injector for SendInputInjector {
    fn press(&self, code: Code) {
        Self::send_event(code, 0);
    }

    fn release(&self, code: Code) {
        Self::send_event(code, KEYEVENTF_KEYUP);
    }

    fn unicode(&self, ch: char) {
        send(encode_units(ch).into_iter().map(|(unit, up)| {
            let flags = KEYEVENTF_UNICODE | if up { KEYEVENTF_KEYUP } else { 0 };
            key_input(0, unit, flags)
        }));
    }

    fn mask(&self) {
        send([
            key_input(MASK_VK, 0, 0),
            key_input(MASK_VK, 0, KEYEVENTF_KEYUP),
        ]);
    }
}

fn key_input(vk: u16, scan: u16, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn send(inputs: impl IntoIterator<Item = INPUT>) {
    let inputs: Vec<INPUT> = inputs.into_iter().collect();
    // SAFETY: inputs 是連續的 INPUT 陣列，長度與大小如實傳入。
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            size_of::<INPUT>() as i32,
        )
    };
    if sent as usize != inputs.len() {
        // SAFETY: 無參數的查詢呼叫，緊接在失敗的 SendInput 之後讀。
        let error = unsafe { GetLastError() };
        log(short_send_message(sent, inputs.len(), error));
    }
}

/// `SendInput` 沒送完（被 UIPI 擋下、輸入桌面被鎖等）時的 log 內容。
fn short_send_message(sent: u32, total: usize, error: u32) -> String {
    format!("[input] SendInput injected {sent} of {total} events: error={error}")
}

/// 字元的 UTF-16 單元，依送出順序：先全部按下，再全部放開（`bool` 為是否放開）。
pub fn encode_units(ch: char) -> Vec<(u16, bool)> {
    let mut buf = [0u16; 2];
    let units = ch.encode_utf16(&mut buf);
    let downs = units.iter().map(|&u| (u, false));
    let ups = units.iter().map(|&u| (u, true));
    downs.chain(ups).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_send_reports_counts_and_the_win32_error() {
        assert_eq!(
            short_send_message(0, 2, 5),
            "[input] SendInput injected 0 of 2 events: error=5"
        );
    }

    #[test]
    fn units_are_all_downs_then_all_ups() {
        assert_eq!(encode_units('a'), [(0x61, false), (0x61, true)]);
        assert_eq!(
            encode_units('😀'),
            [
                (0xD83D, false),
                (0xDE00, false),
                (0xD83D, true),
                (0xDE00, true)
            ]
        );
    }
}
