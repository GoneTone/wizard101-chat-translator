//! 按鍵事件分派：依序問過每個攔截器決定吞不吞，並維護目前按著的鍵與被動熱鍵。
//!
//! 對應 Python `keyboard` 的 `_KeyboardListener`；`process` 跑在低階鍵盤 hook 裡，handler 必須立刻返回。

use std::collections::BTreeSet;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::input::keys::{Code, Key};
use crate::logs::log;

/// hook 收到的原始 vk 與 extended 旗標，給需要算按鍵名稱的錄製器用。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawKey {
    pub vk: u32,
    pub extended: bool,
}

/// 一次按下或放開；`raw` 在測試或合成事件裡可能沒有。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    pub down: bool,
    pub code: Code,
    pub raw: Option<RawKey>,
}

/// 目前按著（且未被吞掉）的鍵。
pub type Pressed = BTreeSet<Code>;

/// 逐鍵決定吞不吞的攔截器。
pub trait Blocking: Send {
    /// 回 true 放行、false 吞掉；`pressed` 不含這次事件。
    fn handle(&mut self, ev: KeyEvent, pressed: &Pressed) -> bool;
}

/// `add_blocking`／`add_passive` 回傳的代號，給 `remove` 用。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HandlerId(u64);

/// 與 hook 執行緒共用的分派器。
pub type SharedDispatcher = Arc<Mutex<Dispatcher>>;

/// `keyboard.add_hotkey` 的 `timeout`：多段熱鍵兩段之間最多隔多久。
const STEP_TIMEOUT: Duration = Duration::from_secs(1);

struct Passive {
    /// 每段所有 code 組合（`itertools.product` 後各自排序）。
    combos: Vec<Vec<Vec<Code>>>,
    allowed: Vec<BTreeSet<Code>>,
    on_trigger: Box<dyn FnMut() + Send>,
    index: usize,
    last_update: Instant,
}

/// 依註冊順序呼叫攔截器，全部放行時更新按鍵狀態並比對被動熱鍵。
#[derive(Default)]
pub struct Dispatcher {
    next_id: u64,
    blocking: Vec<(HandlerId, Box<dyn Blocking>)>,
    passive: Vec<(HandlerId, Passive)>,
    pressed: Pressed,
    processed: u64,
}

impl Dispatcher {
    pub fn new() -> Self {
        Self::default()
    }

    /// 掛上攔截器，排在既有的之後。
    pub fn add_blocking(&mut self, handler: Box<dyn Blocking>) -> HandlerId {
        let id = self.next_id();
        self.blocking.push((id, handler));
        id
    }

    /// `keyboard.add_hotkey(suppress=False)`：湊齊組合時呼叫 `on_trigger`，按鍵照樣放行。
    pub fn add_passive(
        &mut self,
        steps: Vec<Vec<Key>>,
        on_trigger: Box<dyn FnMut() + Send>,
    ) -> HandlerId {
        let id = self.next_id();
        let allowed = steps
            .iter()
            .map(|step| step.iter().flatten().copied().collect())
            .collect();
        let combos = steps.iter().map(|step| combinations(step)).collect();
        self.passive.push((
            id,
            Passive {
                combos,
                allowed,
                on_trigger,
                index: 0,
                last_update: Instant::now(),
            },
        ));
        id
    }

    /// 解除 `add_blocking`／`add_passive` 掛上的 handler。
    pub fn remove(&mut self, id: HandlerId) {
        self.blocking.retain(|(h, _)| *h != id);
        self.passive.retain(|(h, _)| *h != id);
    }

    /// 目前掛著的攔截器與被動熱鍵總數。
    pub fn handler_count(&self) -> usize {
        self.blocking.len() + self.passive.len()
    }

    /// 處理一次按鍵事件，回 true 放行、false 吞掉。
    pub fn process(&mut self, ev: KeyEvent) -> bool {
        self.process_at(ev, Instant::now())
    }

    /// 目前按著（且未被吞掉）的鍵。
    pub fn pressed(&self) -> &Pressed {
        &self.pressed
    }

    /// 到目前為止處理過的按鍵事件數（診斷 hook 是否還活著用）。
    pub fn processed(&self) -> u64 {
        self.processed
    }

    fn next_id(&mut self) -> HandlerId {
        self.next_id += 1;
        HandlerId(self.next_id)
    }

    fn process_at(&mut self, ev: KeyEvent, now: Instant) -> bool {
        self.processed += 1;
        let Self {
            blocking, pressed, ..
        } = self;
        for (_, handler) in blocking.iter_mut() {
            if !guarded(|| handler.handle(ev, pressed)).unwrap_or(true) {
                return false;
            }
        }
        if ev.down {
            self.pressed.insert(ev.code);
        } else {
            self.pressed.remove(&ev.code);
        }
        for (_, passive) in &mut self.passive {
            passive.process(ev, &self.pressed, now);
        }
        true
    }
}

impl Passive {
    /// `add_hotkey` 的多段狀態機（簡化版）：逾時或在下一段按了別的鍵就從頭來。
    fn process(&mut self, ev: KeyEvent, pressed: &Pressed, now: Instant) {
        let stray = ev.down && !self.allowed[self.index].contains(&ev.code);
        if self.index > 0 && (stray || now - self.last_update >= STEP_TIMEOUT) {
            self.index = 0;
        }
        if !ev.down || !self.combos[self.index].iter().any(|c| pressed.iter().eq(c)) {
            return;
        }
        if self.index + 1 == self.combos.len() {
            self.index = 0;
            guarded(&mut self.on_trigger);
        } else {
            self.index += 1;
            self.last_update = now;
        }
    }
}

/// handler 跑在 hook 回呼裡，panic 不可往外傳到 Win32；吞下來記 log，回 `None`。
fn guarded<T>(f: impl FnOnce() -> T) -> Option<T> {
    catch_unwind(AssertUnwindSafe(f))
        .inspect_err(|_| log("[hotkey] handler panicked"))
        .ok()
}

/// `codes` 裡任一個按著。
pub fn is_pressed(pressed: &Pressed, codes: &[Code]) -> bool {
    codes.iter().any(|c| pressed.contains(c))
}

/// `parse_hotkey_combinations` 的一段：每顆鍵各挑一個 code 的所有組合，各自排序。
fn combinations(step: &[Key]) -> Vec<Vec<Code>> {
    let mut combos = vec![Vec::new()];
    for key in step {
        combos = combos
            .iter()
            .flat_map(|prefix| {
                key.iter().map(move |&code| {
                    let mut combo = prefix.clone();
                    combo.push(code);
                    combo
                })
            })
            .collect();
    }
    for combo in &mut combos {
        combo.sort_unstable();
    }
    combos
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Code = 30;
    const B: Code = 48;
    const C: Code = 46;
    const CTRL: Code = 29;
    const SPACE: Code = 57;

    fn down(code: Code) -> KeyEvent {
        KeyEvent {
            down: true,
            code,
            raw: None,
        }
    }

    fn up(code: Code) -> KeyEvent {
        KeyEvent {
            down: false,
            code,
            raw: None,
        }
    }

    /// 吞掉 `swallow` 這顆鍵、記下每次呼叫的假攔截器。
    struct Fake {
        name: &'static str,
        swallow: Code,
        calls: Arc<Mutex<Vec<&'static str>>>,
    }

    impl Blocking for Fake {
        fn handle(&mut self, ev: KeyEvent, _: &Pressed) -> bool {
            self.calls.lock().unwrap().push(self.name);
            ev.code != self.swallow
        }
    }

    fn counter() -> (Arc<Mutex<u32>>, Box<dyn FnMut() + Send>) {
        let count = Arc::new(Mutex::new(0));
        let c = count.clone();
        (count, Box::new(move || *c.lock().unwrap() += 1))
    }

    #[test]
    fn swallowed_keys_do_not_count_as_pressed() {
        let mut d = Dispatcher::new();
        let calls = Arc::new(Mutex::new(Vec::new()));
        d.add_blocking(Box::new(Fake {
            name: "x",
            swallow: SPACE,
            calls,
        }));
        assert!(d.process(down(CTRL)));
        assert!(!d.process(down(SPACE)));
        assert_eq!(d.pressed(), &Pressed::from([CTRL]));
        assert!(!d.process(up(SPACE)));
        assert!(d.process(up(CTRL)));
        assert!(d.pressed().is_empty());
    }

    #[test]
    fn first_swallowing_handler_short_circuits_later_ones() {
        let mut d = Dispatcher::new();
        let calls = Arc::new(Mutex::new(Vec::new()));
        for (name, swallow) in [("first", A), ("second", SPACE), ("third", 0)] {
            let calls = calls.clone();
            d.add_blocking(Box::new(Fake {
                name,
                swallow,
                calls,
            }));
        }
        assert!(!d.process(down(SPACE)));
        assert_eq!(*calls.lock().unwrap(), ["first", "second"]);
        calls.lock().unwrap().clear();
        assert!(d.process(down(B)));
        assert_eq!(*calls.lock().unwrap(), ["first", "second", "third"]);
    }

    #[test]
    fn passive_single_step_fires_on_key_down_when_pressed_equals_the_combo() {
        let mut d = Dispatcher::new();
        let (count, on_trigger) = counter();
        d.add_passive(vec![vec![vec![CTRL], vec![SPACE]]], on_trigger);
        assert!(d.process(down(CTRL)));
        assert!(d.process(down(SPACE)));
        assert!(d.process(down(SPACE)));
        assert_eq!(*count.lock().unwrap(), 2);
        assert!(d.process(up(SPACE)));
        assert_eq!(*count.lock().unwrap(), 2);
        d.process(down(A));
        d.process(down(SPACE));
        assert_eq!(*count.lock().unwrap(), 2);
    }

    fn tap(d: &mut Dispatcher, code: Code, at: Instant) {
        d.process_at(down(code), at);
        d.process_at(up(code), at);
    }

    #[test]
    fn passive_multi_step_fires_after_each_step_in_order() {
        let mut d = Dispatcher::new();
        let (count, on_trigger) = counter();
        d.add_passive(vec![vec![vec![CTRL], vec![A]], vec![vec![B]]], on_trigger);
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        tap(&mut d, B, ms(0));
        d.process_at(down(CTRL), ms(0));
        tap(&mut d, A, ms(10));
        d.process_at(up(CTRL), ms(20));
        tap(&mut d, B, ms(30));
        assert_eq!(*count.lock().unwrap(), 1);
        tap(&mut d, B, ms(40));
        assert_eq!(*count.lock().unwrap(), 1);
        d.process_at(down(CTRL), ms(50));
        tap(&mut d, A, ms(60));
        d.process_at(up(CTRL), ms(70));
        tap(&mut d, C, ms(80));
        tap(&mut d, B, ms(90));
        assert_eq!(*count.lock().unwrap(), 1);
    }

    #[test]
    fn passive_multi_step_resets_after_one_second() {
        let mut d = Dispatcher::new();
        let (count, on_trigger) = counter();
        d.add_passive(vec![vec![vec![A]], vec![vec![B]]], on_trigger);
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        d.process_at(down(A), ms(0));
        d.process_at(up(A), ms(0));
        d.process_at(down(B), ms(1000));
        assert_eq!(*count.lock().unwrap(), 0);
        d.process_at(up(B), ms(1000));
        d.process_at(down(A), ms(2000));
        d.process_at(up(A), ms(2000));
        d.process_at(down(B), ms(2999));
        assert_eq!(*count.lock().unwrap(), 1);
    }

    #[test]
    fn remove_detaches_both_kinds_of_handler() {
        let mut d = Dispatcher::new();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let blocking = d.add_blocking(Box::new(Fake {
            name: "x",
            swallow: SPACE,
            calls,
        }));
        let (count, on_trigger) = counter();
        let passive = d.add_passive(vec![vec![vec![A]]], on_trigger);
        d.remove(blocking);
        d.remove(passive);
        assert!(d.process(down(SPACE)));
        d.process(down(A));
        assert_eq!(*count.lock().unwrap(), 0);
    }

    #[test]
    fn processed_counts_every_event_including_swallowed_ones() {
        let mut d = Dispatcher::new();
        d.add_blocking(Box::new(Fake {
            name: "x",
            swallow: SPACE,
            calls: Arc::new(Mutex::new(Vec::new())),
        }));
        assert_eq!(d.processed(), 0);
        d.process(down(A));
        d.process(down(SPACE));
        assert_eq!(d.processed(), 2);
    }

    #[test]
    fn is_pressed_matches_any_alternative_code() {
        let pressed = Pressed::from([CTRL]);
        assert!(is_pressed(&pressed, &[-163, CTRL]));
        assert!(!is_pressed(&pressed, &[SPACE]));
        assert!(!is_pressed(&pressed, &[]));
    }
}
