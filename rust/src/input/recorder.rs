//! 熱鍵錄製：移植 `keyboard.read_hotkey(suppress=False)`，第一次有鍵放開時回報目前按著的組合。

use std::collections::BTreeMap;

use windows_sys::Win32::System::Threading::{GetCurrentProcessId, GetCurrentThreadId};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyState;
use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

use crate::input::dispatcher::{Blocking, HandlerId, KeyEvent, Pressed, RawKey, SharedDispatcher};
use crate::input::hook::{self, ALT_GR_SCAN, VK_LCONTROL};
use crate::input::keys::{Code, event_name, hotkey_name};
use crate::logs::log;
use crate::util::lock;

const SHIFT_VKS: [u32; 3] = [0x10, 0xA0, 0xA1];
const LOCK_KEYS: [(i32, &str); 3] = [
    (0x90, "num lock"),
    (0x14, "caps lock"),
    (0x91, "scroll lock"),
];

/// 進行中的錄製；drop 時從分派器移除 handler，之後的按鍵不再被記錄。
///
/// drop 會鎖分派器，所以不可在 `done` 或任何 handler 裡 drop（hook 執行緒握著同一把鎖，會死鎖）；
/// 請等 `done` 的結果送到 UI 執行緒後再 drop。
pub struct Recording {
    dispatcher: SharedDispatcher,
    id: HandlerId,
}

impl Drop for Recording {
    fn drop(&mut self) {
        hook::trace_events(false);
        let mut d = lock(&self.dispatcher);
        d.remove(self.id);
        log(format!(
            "[hotkey] recording stopped (events processed so far={}, hook calls so far={})",
            d.processed(),
            hook::hook_calls()
        ));
    }
}

/// 開始錄製：錄製開始後按下的鍵第一次放開時，以 hook 執行緒呼叫一次 `done(熱鍵字串)`，按鍵照樣放行。
///
/// `done` 只能做投遞，不可做 I/O，也不可在裡面 drop 回傳的 `Recording`。
pub fn record(d: &SharedDispatcher, done: Box<dyn FnOnce(String) + Send>) -> Recording {
    let mut dispatcher = lock(d);
    let id = dispatcher.add_blocking(Box::new(Recorder {
        pressed: BTreeMap::new(),
        shift: false,
        alt_gr: false,
        done: Some(done),
    }));
    // 開始與結束時的計數相同，表示錄製期間 hook 一個按鍵都沒送進來
    log(format!(
        "[hotkey] recording started (events processed so far={}, hook calls so far={}; {})",
        dispatcher.processed(),
        hook::hook_calls(),
        thread_context()
    ));
    drop(dispatcher);
    hook::trace_events(true);
    Recording {
        dispatcher: d.clone(),
        id,
    }
}

/// 前景視窗與相關執行緒，給錄製開始的診斷 log：鍵盤事件沒進來時看得出當下誰在前景。
fn thread_context() -> String {
    // SAFETY：純查詢的 Win32 呼叫，pid 是本地變數。
    let (foreground, thread, pid) = unsafe {
        let hwnd = GetForegroundWindow();
        let mut pid = 0;
        let thread = GetWindowThreadProcessId(hwnd, &mut pid);
        (hwnd as isize, thread, pid)
    };
    // SAFETY：同上。
    let (ui_thread, own_pid) = unsafe { (GetCurrentThreadId(), GetCurrentProcessId()) };
    format!(
        "foreground hwnd={foreground:#x} thread={thread} pid={pid}, ui thread={ui_thread}, hook thread={}, this pid={own_pid}",
        hook::hook_thread_id()
    )
}

struct Recorder {
    /// 錄製開始後按下、還沒放開的鍵 → 按下當下算出的名稱（算不出名稱的鍵也要記，放開時才認得）。
    pressed: BTreeMap<Code, Option<String>>,
    shift: bool,
    alt_gr: bool,
    done: Option<Box<dyn FnOnce(String) + Send>>,
}

impl Recorder {
    /// 照 `_winkeyboard.process_key`：名稱用事件發生前的修飾鍵狀態算。
    fn name_of(&self, ev: KeyEvent, raw: RawKey) -> Option<String> {
        let mut mods = Vec::new();
        if self.shift {
            mods.push("shift");
        }
        if self.alt_gr {
            mods.push("alt gr");
        }
        mods.extend(
            LOCK_KEYS
                .iter()
                // SAFETY：GetKeyState 只讀目前執行緒的按鍵狀態表。
                .filter(|(vk, _)| unsafe { GetKeyState(*vk) } & 1 != 0)
                .map(|(_, name)| *name),
        );
        event_name(ev.code.max(0) as u32, raw.vk, raw.extended, &mods)
    }

    fn track_modifiers(&mut self, ev: KeyEvent, raw: RawKey) {
        if SHIFT_VKS.contains(&raw.vk) {
            self.shift = ev.down;
        }
        if ev.code == ALT_GR_SCAN as Code && raw.vk == VK_LCONTROL {
            self.alt_gr = ev.down;
        }
    }
}

impl Blocking for Recorder {
    fn handle(&mut self, ev: KeyEvent, _: &Pressed) -> bool {
        let Some(raw) = ev.raw else { return true };
        if self.done.is_none() {
            return true;
        }
        let name = self.name_of(ev, raw);
        // 錄製只到第一次放開為止，記下的按鍵不會洩漏打字內容
        log(format!(
            "[hotkey] recorder saw code={} vk={:#x} down={} name={name:?}",
            ev.code, raw.vk, ev.down
        ));
        self.track_modifiers(ev, raw);
        if ev.down {
            self.pressed.insert(ev.code, name);
            return true;
        }
        // 用鍵盤按〔更改〕時，觸發的 Enter／Space 在錄製開始前就按下了，它的放開不算數。
        if self.pressed.remove(&ev.code).is_none() {
            return true;
        }
        let names: Vec<String> = self
            .pressed
            .values()
            .flatten()
            .cloned()
            .chain(name)
            .collect();
        let hotkey = hotkey_name(&names);
        log(format!("[hotkey] recorded {hotkey:?}"));
        if let Some(done) = self.done.take() {
            done(hotkey);
        }
        true
    }
}
