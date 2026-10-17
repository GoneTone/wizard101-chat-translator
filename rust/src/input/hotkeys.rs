//! 全域熱鍵：在按鍵送達前景程式之前決定吞不吞，熱鍵生效時遊戲就不會收到那顆鍵
//! （例如空白鍵讓 NPC 對話跳到下一句）。

use std::sync::Arc;

use crate::input::dispatcher::{
    Blocking, HandlerId, KeyEvent, Pressed, SharedDispatcher, is_pressed,
};
use crate::input::keys::{Code, Key, is_modifier, key_codes_or_empty, parse_hotkey};
use crate::logs::log;
use crate::py::pyrepr::repr_str;
use crate::util::lock;

/// 未指派的虛擬鍵，AutoHotkey 的 MenuMaskKey 預設也用它。
pub const MASK_VK: u16 = 0xE8;
/// 熱鍵沒列到的修飾鍵群組按著時不算湊齊組合（按 `ctrl+shift+space` 不會觸發 `ctrl+space`）。
pub const MODIFIER_GROUPS: [&str; 4] = ["ctrl", "shift", "alt", "windows"];

/// 熱鍵組合沒辦法逐鍵攔截的原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CannotSwallow {
    /// 多段組合（如 `ctrl+a, s`）。
    MultiStep,
    /// 只有修飾鍵。
    OnlyModifiers,
}

/// 一組熱鍵的逐次按鍵決策，掛在 `Dispatcher` 上，跑在低階 hook 裡。
///
/// 組合裡的非修飾鍵按下、湊齊組合（修飾鍵須完全吻合）時呼叫 `on_trigger()`，回 true 表示
/// 有接手，就吞掉這顆鍵直到放開；先按下的鍵早已送出，吞不回來。`on_trigger` 必須立刻返回。
pub struct HotkeyInterceptor {
    hotkey: String,
    keys: Vec<Key>,
    trigger_codes: Vec<Code>,
    unwanted_modifiers: Vec<Key>,
    on_trigger: Box<dyn FnMut() -> bool + Send>,
    mask: Box<dyn Fn() + Send>,
    swallow_override: Option<Box<dyn Fn() -> bool + Send>>,
    /// 已接手、吞到放開為止的鍵；按住時 Windows 會連續送 down。
    held: Vec<Code>,
}

impl HotkeyInterceptor {
    /// 外層 `Err` 是鍵名無效；內層 `Err` 是這組熱鍵無法逐鍵攔截。
    pub fn new(
        hotkey: &str,
        on_trigger: Box<dyn FnMut() -> bool + Send>,
        mask: Box<dyn Fn() + Send>,
    ) -> Result<Result<Self, CannotSwallow>, String> {
        Ok(Self::from_steps(
            hotkey,
            &parse_hotkey(hotkey)?,
            on_trigger,
            mask,
        ))
    }

    /// 同 `new`，但吃已解析好的 `parse_hotkey` 結果。
    fn from_steps(
        hotkey: &str,
        steps: &[Vec<Key>],
        on_trigger: Box<dyn FnMut() -> bool + Send>,
        mask: Box<dyn Fn() + Send>,
    ) -> Result<Self, CannotSwallow> {
        let [keys] = steps else {
            return Err(CannotSwallow::MultiStep);
        };
        let keys = keys.clone();
        let trigger_codes: Vec<Code> = keys
            .iter()
            .filter(|key| !key.iter().any(|&c| is_modifier(c)))
            .flatten()
            .copied()
            .collect();
        if trigger_codes.is_empty() {
            return Err(CannotSwallow::OnlyModifiers);
        }
        let unwanted_modifiers = MODIFIER_GROUPS
            .iter()
            .map(|name| key_codes_or_empty(name))
            .filter(|group| !keys.iter().flatten().any(|c| group.contains(c)))
            .collect();
        Ok(Self {
            hotkey: hotkey.to_string(),
            keys,
            trigger_codes,
            unwanted_modifiers,
            on_trigger,
            mask,
            swallow_override: None,
            held: Vec::new(),
        })
    }

    /// `f()` 成立時，觸發鍵一律吞掉、不呼叫 `on_trigger`（不記入按住、不補遮罩鍵）。
    pub fn swallow_override(mut self, f: Box<dyn Fn() -> bool + Send>) -> Self {
        self.swallow_override = Some(f);
        self
    }

    fn should_swallow(&mut self, ev: KeyEvent, pressed: &Pressed) -> bool {
        if self.swallow_override.as_ref().is_some_and(|f| f()) {
            return true;
        }
        if !ev.down || !self.completes_combo(ev.code, pressed) || !(self.on_trigger)() {
            return false;
        }
        self.held.push(ev.code);
        (self.mask)();
        log(format!(
            "[hotkey] {} swallowed; injected mask key vk={MASK_VK:#x}",
            repr_str(&self.hotkey)
        ));
        true
    }

    fn completes_combo(&self, code: Code, pressed: &Pressed) -> bool {
        let others_held = self
            .keys
            .iter()
            .filter(|key| !key.contains(&code))
            .all(|key| is_pressed(pressed, key));
        others_held
            && !self
                .unwanted_modifiers
                .iter()
                .any(|group| is_pressed(pressed, group))
    }
}

impl Blocking for HotkeyInterceptor {
    fn handle(&mut self, ev: KeyEvent, pressed: &Pressed) -> bool {
        if let Some(i) = self.held.iter().position(|&c| c == ev.code) {
            if !ev.down {
                self.held.swap_remove(i);
            }
            return false;
        }
        if !self.trigger_codes.contains(&ev.code) {
            return true;
        }
        !self.should_swallow(ev, pressed)
    }
}

/// `register_hotkey` 掛上的 handler；`unregister` 解除。
pub struct Registration {
    dispatcher: SharedDispatcher,
    id: HandlerId,
}

impl Registration {
    pub fn unregister(self) {
        lock(&self.dispatcher).remove(self.id);
    }
}

/// 註冊全域熱鍵，回傳（註冊，實際生效的熱鍵）。`callback()` 回 true 表示有接手。
///
/// 能逐鍵攔截就掛攔截器，否則退回不吞鍵的被動熱鍵；鍵名不認得時退回 `fallback`。
pub fn register_hotkey(
    d: &SharedDispatcher,
    hotkey: &str,
    callback: Arc<dyn Fn() -> bool + Send + Sync>,
    fallback: &str,
    mask: Arc<dyn Fn() + Send + Sync>,
) -> (Registration, String) {
    match register(d, hotkey, &callback, &mask) {
        Ok(registration) => (registration, hotkey.to_string()),
        Err(err) => {
            log(format!(
                "[hotkey] {} is not a valid key combination ({err}); using {}",
                repr_str(hotkey),
                repr_str(fallback)
            ));
            let registration = register(d, fallback, &callback, &mask)
                .unwrap_or_else(|err| panic!("fallback hotkey {fallback:?} is invalid: {err}"));
            (registration, fallback.to_string())
        }
    }
}

fn register(
    d: &SharedDispatcher,
    hotkey: &str,
    callback: &Arc<dyn Fn() -> bool + Send + Sync>,
    mask: &Arc<dyn Fn() + Send + Sync>,
) -> Result<Registration, String> {
    let steps = parse_hotkey(hotkey)?;
    let (cb, m) = (callback.clone(), mask.clone());
    let id = match HotkeyInterceptor::from_steps(
        hotkey,
        &steps,
        Box::new(move || cb()),
        Box::new(move || m()),
    ) {
        Ok(interceptor) => {
            log(format!(
                "[hotkey] {} registered; its trigger key is swallowed when handled",
                repr_str(hotkey)
            ));
            lock(d).add_blocking(Box::new(interceptor))
        }
        Err(reason) => {
            let why = match reason {
                CannotSwallow::MultiStep => "is a multi-step hotkey",
                CannotSwallow::OnlyModifiers => "has only modifier keys",
            };
            log(format!(
                "[hotkey] {} {why}; registered without swallowing, its keys still reach the game",
                repr_str(hotkey)
            ));
            let cb = callback.clone();
            lock(d).add_passive(
                steps,
                Box::new(move || {
                    cb();
                }),
            )
        }
    };
    Ok(Registration {
        dispatcher: d.clone(),
        id,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::input::keys::key_to_scan_codes;

    fn code(name: &str) -> Code {
        key_to_scan_codes(name).unwrap()[0]
    }

    fn down(code: Code) -> KeyEvent {
        KeyEvent { down: true, code }
    }

    fn up(code: Code) -> KeyEvent {
        KeyEvent { down: false, code }
    }

    struct Probe {
        interceptor: HotkeyInterceptor,
        triggers: Arc<Mutex<u32>>,
        masks: Arc<Mutex<u32>>,
    }

    fn interceptor(hotkey: &str, handled: bool) -> Probe {
        let triggers = Arc::new(Mutex::new(0));
        let masks = Arc::new(Mutex::new(0));
        let (t, m) = (triggers.clone(), masks.clone());
        let interceptor = HotkeyInterceptor::new(
            hotkey,
            Box::new(move || {
                *t.lock().unwrap() += 1;
                handled
            }),
            Box::new(move || *m.lock().unwrap() += 1),
        )
        .unwrap()
        .unwrap();
        Probe {
            interceptor,
            triggers,
            masks,
        }
    }

    impl Probe {
        fn handle(&mut self, ev: KeyEvent, pressed: &[Code]) -> bool {
            self.interceptor
                .handle(ev, &pressed.iter().copied().collect())
        }

        fn triggers(&self) -> u32 {
            *self.triggers.lock().unwrap()
        }

        fn masks(&self) -> u32 {
            *self.masks.lock().unwrap()
        }
    }

    #[test]
    fn swallows_the_trigger_key_when_the_combo_is_handled() {
        let (ctrl, space) = (code("ctrl"), code("space"));
        let mut p = interceptor("ctrl+space", true);
        assert!(!p.handle(down(space), &[ctrl]));
        assert!(!p.handle(up(space), &[ctrl]));
        assert_eq!(p.triggers(), 1);
    }

    #[test]
    fn passes_the_key_through_when_the_callback_does_not_handle_it() {
        let (ctrl, space) = (code("ctrl"), code("space"));
        let mut p = interceptor("ctrl+space", false);
        assert!(p.handle(down(space), &[ctrl]));
        assert!(p.handle(up(space), &[ctrl]));
        assert_eq!(p.triggers(), 1);
    }

    #[test]
    fn masks_modifiers_only_when_swallowing() {
        let (ctrl, shift, space) = (code("ctrl"), code("shift"), code("space"));
        let mut p = interceptor("ctrl+shift+space", true);
        p.handle(down(space), &[ctrl, shift]);
        p.handle(down(space), &[ctrl, shift]);
        p.handle(up(space), &[ctrl, shift]);
        assert_eq!(p.masks(), 1);
        let mut passthrough = interceptor("ctrl+shift+space", false);
        passthrough.handle(down(space), &[ctrl, shift]);
        assert_eq!(passthrough.masks(), 0);
    }

    #[test]
    fn passes_the_bare_trigger_key_through() {
        let mut p = interceptor("ctrl+space", true);
        assert!(p.handle(down(code("space")), &[]));
        assert_eq!(p.triggers(), 0);
    }

    #[test]
    fn extra_modifier_does_not_match() {
        let mut p = interceptor("ctrl+space", true);
        assert!(p.handle(down(code("space")), &[code("ctrl"), code("shift")]));
        assert_eq!(p.triggers(), 0);
    }

    #[test]
    fn extra_non_modifier_key_still_matches() {
        let mut p = interceptor("ctrl+space", true);
        assert!(!p.handle(down(code("space")), &[code("ctrl"), code("w")]));
        assert_eq!(p.triggers(), 1);
    }

    #[test]
    fn unrelated_keys_pass_without_calling_back() {
        let ctrl = code("ctrl");
        let mut p = interceptor("ctrl+space", true);
        assert!(p.handle(down(code("a")), &[ctrl]));
        assert!(p.handle(down(ctrl), &[ctrl]));
        assert_eq!(p.triggers(), 0);
    }

    #[test]
    fn key_repeat_is_swallowed_until_release_even_after_the_modifier_is_up() {
        let (ctrl, space) = (code("ctrl"), code("space"));
        let mut p = interceptor("ctrl+space", true);
        assert!(!p.handle(down(space), &[ctrl]));
        assert!(!p.handle(down(space), &[]));
        assert!(!p.handle(up(space), &[]));
        assert_eq!(p.triggers(), 1);
        assert!(p.handle(down(space), &[]));
    }

    #[test]
    fn combo_without_modifiers_swallows_whichever_key_completes_it() {
        let (a, space) = (code("a"), code("space"));
        let mut p = interceptor("a+space", true);
        assert!(!p.handle(down(space), &[a]));
        p.handle(up(space), &[a]);
        assert!(!p.handle(down(a), &[space]));
        assert_eq!(p.triggers(), 2);
    }

    #[test]
    fn rejects_combos_it_cannot_swallow() {
        for (hotkey, reason) in [
            ("ctrl", CannotSwallow::OnlyModifiers),
            ("ctrl+shift", CannotSwallow::OnlyModifiers),
            ("ctrl+a, s", CannotSwallow::MultiStep),
        ] {
            let result = HotkeyInterceptor::new(hotkey, Box::new(|| true), Box::new(|| {}));
            assert_eq!(result.unwrap().err(), Some(reason), "{hotkey}");
        }
    }

    #[test]
    fn rejects_unknown_key_names() {
        let result = HotkeyInterceptor::new("ctrl+nope", Box::new(|| true), Box::new(|| {}));
        assert_eq!(
            result.err(),
            Some("Key 'nope' is not mapped to any known key.".to_string())
        );
    }

    #[test]
    fn swallow_override_swallows_the_trigger_key_without_calling_back() {
        let (ctrl, v) = (code("ctrl"), code("v"));
        let mut p = interceptor("ctrl+v", true);
        p.interceptor = p.interceptor.swallow_override(Box::new(|| true));
        assert!(!p.handle(down(v), &[]));
        assert!(p.handle(down(ctrl), &[]));
        assert_eq!((p.triggers(), p.masks()), (0, 0));
    }
}
