//! 熱鍵錄製：移植 `keyboard.read_hotkey(suppress=False)`，第一次有鍵放開時回報目前按著的組合。

use std::collections::BTreeMap;

use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyState;

use crate::input::dispatcher::{Blocking, HandlerId, KeyEvent, Pressed, RawKey, SharedDispatcher};
use crate::input::keys::{Code, event_name, hotkey_name};
use crate::logs::log;
use crate::util::lock;

const SHIFT_VKS: [u32; 3] = [0x10, 0xA0, 0xA1];
const ALT_GR_SCAN: Code = 541;
const VK_LCONTROL: u32 = 0xA2;
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
        lock(&self.dispatcher).remove(self.id);
        log("[hotkey] recording stopped");
    }
}

/// 開始錄製：第一次有鍵放開時以 hook 執行緒呼叫一次 `done(熱鍵字串)`，按鍵照樣放行。
///
/// `done` 只能做投遞，不可做 I/O，也不可在裡面 drop 回傳的 `Recording`。
pub fn record(d: &SharedDispatcher, done: Box<dyn FnOnce(String) + Send>) -> Recording {
    let id = lock(d).add_blocking(Box::new(Recorder {
        pressed: BTreeMap::new(),
        shift: false,
        alt_gr: false,
        done: Some(done),
    }));
    log("[hotkey] recording started");
    Recording {
        dispatcher: d.clone(),
        id,
    }
}

struct Recorder {
    /// 按著的鍵 → 按下當下算出的名稱。
    pressed: BTreeMap<Code, String>,
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
        if ev.code == ALT_GR_SCAN && raw.vk == VK_LCONTROL {
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
        self.track_modifiers(ev, raw);
        if ev.down {
            if let Some(name) = name {
                self.pressed.insert(ev.code, name);
            }
            return true;
        }
        self.pressed.remove(&ev.code);
        let names: Vec<String> = self.pressed.values().cloned().chain(name).collect();
        let hotkey = hotkey_name(&names);
        log(format!("[hotkey] recorded {hotkey:?}"));
        if let Some(done) = self.done.take() {
            done(hotkey);
        }
        true
    }
}
