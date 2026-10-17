//! reader_loop 行為：read_new() → 推進上下文 → overlay 佔位 → 提交翻譯池；以假 reader 與虛擬時鐘驗證。

#[allow(dead_code)]
mod common;

use std::panic::AssertUnwindSafe;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use common::log_capture::{capture_logs, log_lines_with};
use common::reader_fixture::{
    EXPECTED_PANIC, VirtualClock, banners, drain, echo, failing_pool, pool, quiet_panics, runtime,
    statuses, submitted,
};
use serde_json::json;
use tokio::runtime::Runtime;
use tokio_util::sync::CancellationToken;
use wizard101_chat_translator::config::Config;
use wizard101_chat_translator::i18n::t;
use wizard101_chat_translator::reader::chat_tabs::TabState;
use wizard101_chat_translator::reader::markup::ChatLine;
use wizard101_chat_translator::reader::mem_reader::{ChatEvent, GameNotRunning, WizChatReader};
use wizard101_chat_translator::reader::run_loop::{
    ChatSource, GAME_MISSING_INTERVAL, INPUT_POLL_INTERVAL, LoopEnv, MessageIds, POLL_INTERVAL,
    ReaderShared, reader_loop_with,
};
use wizard101_chat_translator::reader::status::StatusBoard;
use wizard101_chat_translator::translation::cache::TranslationCache;
use wizard101_chat_translator::translation::context::ClientContexts;
use wizard101_chat_translator::translation::errors::TranslateError;
use wizard101_chat_translator::translation::pool::TranslationPool;
use wizard101_chat_translator::ui::events::{UiEvent, UiPoster};

type Results = Receiver<(u64, String, bool)>;

enum Read {
    Lines(Vec<ChatEvent>),
    Fail(GameNotRunning),
    Panic,
}

fn ev(text: &str) -> ChatEvent {
    ChatEvent {
        line: ChatLine {
            text: text.to_owned(),
            color: None,
            own: false,
            system: false,
        },
        tabs: vec![0],
        seq: 0,
        raw: String::new(),
    }
}

fn on_tab(tab: i32, text: &str) -> ChatEvent {
    ChatEvent {
        tabs: vec![tab],
        ..ev(text)
    }
}

/// 一行系統訊息（帶遊戲顯示色）。
fn drop_line(text: &str) -> ChatEvent {
    let mut event = ev(text);
    event.line.color = Some("#00ff00".into());
    event.line.system = true;
    event
}

fn lines(texts: &[&str]) -> Read {
    Read::Lines(texts.iter().map(|t| ev(t)).collect())
}

fn empty(n: usize) -> Vec<Read> {
    (0..n).map(|_| lines(&[])).collect()
}

fn tab_state(current: i32) -> TabState {
    TabState {
        tabs: vec![(0, "General".into()), (1, "Team".into())],
        current,
    }
}

/// 假 reader 留下的紀錄。
#[derive(Default)]
struct Probe {
    closed: bool,
    emit_system: Vec<bool>,
    read_at: Vec<Duration>,
    input_at: Vec<Duration>,
    tab_reads: usize,
}

/// 依序回傳 `reads[i]`；讀完最後一項就設 stop。
struct FakeReader {
    reads: Vec<Read>,
    states: Vec<Option<TabState>>,
    inputs: Vec<bool>,
    input_rect: Option<(i32, i32, i32, i32)>,
    anchored: bool,
    emit_system: bool,
    on_read: Option<Box<dyn FnMut(usize) + Send>>,
    n: usize,
    stop: CancellationToken,
    clock: VirtualClock,
    probe: Arc<Mutex<Probe>>,
}

impl FakeReader {
    fn state(&self) -> Option<TabState> {
        let last = self.states.len().checked_sub(1)?;
        self.states[self.n.saturating_sub(1).min(last)].clone()
    }
}

impl ChatSource for FakeReader {
    fn read_new(&mut self) -> Result<Vec<ChatEvent>, GameNotRunning> {
        if let Some(hook) = self.on_read.as_mut() {
            hook(self.n);
        }
        {
            let mut probe = self.probe.lock().unwrap();
            probe.emit_system.push(self.emit_system);
            probe.read_at.push(self.clock.elapsed());
        }
        let i = self.n;
        self.n += 1;
        if self.n >= self.reads.len() {
            self.stop.cancel();
        }
        match &self.reads[i] {
            Read::Lines(events) => Ok(events.clone()),
            Read::Fail(err) => {
                // 比照真實 reader：回報 GameNotRunning 前一律先 teardown
                self.anchored = false;
                Err(err.clone())
            }
            Read::Panic => panic!("{EXPECTED_PANIC} read_new blew up"),
        }
    }

    fn anchored(&self) -> bool {
        self.anchored
    }

    fn current_tab(&self) -> Option<i32> {
        self.state().map(|s| s.current)
    }

    fn tab_state(&mut self) -> Option<TabState> {
        self.probe.lock().unwrap().tab_reads += 1;
        self.state()
    }

    fn input_open(&mut self) -> bool {
        self.probe
            .lock()
            .unwrap()
            .input_at
            .push(self.clock.elapsed());
        let last = self.inputs.len().saturating_sub(1);
        self.inputs
            .get(self.n.saturating_sub(1).min(last))
            .copied()
            .unwrap_or(false)
    }

    fn input_box_screen_rect(&mut self) -> Option<(i32, i32, i32, i32)> {
        self.input_rect
    }

    fn set_emit_system(&mut self, emit: bool) {
        self.emit_system = emit;
    }

    fn close(&mut self) {
        self.probe.lock().unwrap().closed = true;
    }
}

/// 一個客戶端的收訊環境：真的翻譯池（譯文回傳原文與上下文）、虛擬時鐘、假視窗。
struct Setup {
    rt: Runtime,
    stop: CancellationToken,
    settings: Arc<RwLock<Config>>,
    pool: Arc<TranslationPool>,
    pool_rx: Results,
    system_pool: Arc<TranslationPool>,
    system_rx: Results,
    cache: Arc<TranslationCache>,
    contexts: Arc<ClientContexts>,
    clock: VirtualClock,
    probe: Arc<Mutex<Probe>>,
    ui: UiPoster,
    ui_rx: Receiver<UiEvent>,
    window_exists: Box<dyn FnMut(isize) -> bool + Send>,
    foreground: Box<dyn FnMut() -> isize + Send>,
    hwnd: isize,
    slot: u32,
}

impl Setup {
    fn new() -> Self {
        capture_logs();
        let rt = runtime();
        let (player_pool, pool_rx) = pool(&rt, echo());
        let (system_pool, system_rx) = pool(&rt, echo());
        let (ui, ui_rx) = UiPoster::for_test();
        let path = std::env::temp_dir().join("reader_loop_cache_never_written.json");
        Self {
            rt,
            stop: CancellationToken::new(),
            settings: Arc::new(RwLock::new(Config::new())),
            pool: player_pool,
            pool_rx,
            system_pool,
            system_rx,
            cache: Arc::new(TranslationCache::new(path, "fp".into())),
            contexts: Arc::new(ClientContexts::new()),
            clock: VirtualClock::new(),
            probe: Arc::default(),
            ui,
            ui_rx,
            window_exists: Box::new(|_| true),
            foreground: Box::new(|| 0),
            hwnd: 0x1,
            slot: 1,
        }
    }

    fn system_messages(self, on: bool) -> Self {
        let cfg = json!({ "translate_system_messages": on });
        *self.settings.write().unwrap() = cfg.as_object().unwrap().clone();
        self
    }

    fn reader(&self, reads: Vec<Read>) -> FakeReader {
        FakeReader {
            reads,
            states: Vec::new(),
            inputs: Vec::new(),
            input_rect: Some((749, 893, 732, 44)),
            anchored: true,
            emit_system: false,
            on_read: None,
            n: 0,
            stop: self.stop.clone(),
            clock: self.clock.clone(),
            probe: Arc::clone(&self.probe),
        }
    }

    fn run_reader(&mut self, reader: FakeReader) -> Vec<UiEvent> {
        let board = Arc::new(StatusBoard::new(
            self.ui.clone(),
            Arc::clone(&self.pool),
            Arc::clone(&self.system_pool),
        ));
        let shared = Arc::new(ReaderShared {
            settings: Arc::clone(&self.settings),
            ui: self.ui.clone(),
            stop: self.stop.clone(),
            pool: Arc::clone(&self.pool),
            system_pool: Arc::clone(&self.system_pool),
            board,
            msg_ids: Arc::new(MessageIds::default()),
            cache: Arc::clone(&self.cache),
        });
        let env = LoopEnv {
            clock: self.clock.clock(Duration::ZERO),
            window_exists: std::mem::replace(&mut self.window_exists, Box::new(|_| true)),
            foreground_window: std::mem::replace(&mut self.foreground, Box::new(|| 0)),
        };
        let contexts = Arc::clone(&self.contexts);
        reader_loop_with(env, shared, self.hwnd, self.slot, reader, contexts);
        drain(&self.ui_rx)
    }

    fn run(&mut self, reads: Vec<Read>) -> Vec<UiEvent> {
        let reader = self.reader(reads);
        self.run_reader(reader)
    }
}

#[derive(Debug, PartialEq)]
struct Added {
    original: String,
    translated: String,
    pending: bool,
    color: Option<String>,
    msg_id: Option<u64>,
    slot: u32,
    tabs: Vec<i32>,
}

fn added(events: &[UiEvent]) -> Vec<Added> {
    events
        .iter()
        .filter_map(|e| match e {
            UiEvent::AddMessage {
                slot,
                tabs,
                original,
                translated,
                msg_id,
                pending,
                color,
            } => Some(Added {
                original: original.clone(),
                translated: translated.clone(),
                pending: *pending,
                color: color.clone(),
                msg_id: *msg_id,
                slot: *slot,
                tabs: tabs.clone(),
            }),
            _ => None,
        })
        .collect()
}

fn texts(events: &[UiEvent]) -> Vec<(String, String)> {
    added(events)
        .into_iter()
        .map(|a| (a.original, a.translated))
        .collect()
}

fn tab_events(events: &[UiEvent]) -> Vec<(u32, Option<TabState>)> {
    events
        .iter()
        .filter_map(|e| match e {
            UiEvent::SetTabs { slot, state } => Some((*slot, state.clone())),
            _ => None,
        })
        .collect()
}

fn foreground_events(events: &[UiEvent]) -> Vec<Option<u32>> {
    events
        .iter()
        .filter_map(|e| match e {
            UiEvent::SetForegroundSlot(slot) => Some(*slot),
            _ => None,
        })
        .collect()
}

fn pending() -> String {
    t("notice.pending", &[])
}

fn args(pairs: &[(&'static str, &str)]) -> Vec<(&'static str, String)> {
    pairs.iter().map(|(k, v)| (*k, (*v).to_owned())).collect()
}

#[test]
fn wiz_chat_reader_is_send() {
    fn _send<T: Send>() {}
    _send::<WizChatReader>();
}

#[test]
fn system_lines_skip_the_chat_context() {
    // 系統訊息不得進入上下文：8 行的窗會被掉寶洗光，玩家對話就失去語境
    let mut s = Setup::new().system_messages(true);
    s.run(vec![
        Read::Lines(vec![drop_line("你获得了 39 金币！"), ev("[Lars] hi")]),
        lines(&[]),
    ]);
    assert_eq!(s.contexts.current().snapshot(), ["[Lars] hi"]);
    assert_eq!(
        submitted(&s.pool_rx, 1),
        [("[Lars] hi".to_owned(), vec![], 2)]
    );
    // 系統訊息不吃上下文，提交時 context 必為空
    assert_eq!(
        submitted(&s.system_rx, 1),
        [("你获得了 39 金币！".to_owned(), vec![], 1)]
    );
}

#[test]
fn system_cache_hit_shown_without_pool() {
    let mut s = Setup::new().system_messages(true);
    assert!(
        s.cache
            .put("你获得了 39 金币！", "你獲得了 {0} 金幣！", "fp")
    );
    let events = s.run(vec![
        Read::Lines(vec![drop_line("你获得了 39 金币！")]),
        lines(&[]),
    ]);
    // 命中快取＝完成態，不經過佔位，遊戲顯示色照樣帶到 overlay
    assert_eq!(
        added(&events),
        [Added {
            original: "你获得了 39 金币！".into(),
            translated: "你獲得了 39 金幣！".into(),
            pending: false,
            color: Some("#00ff00".into()),
            msg_id: None,
            slot: 1,
            tabs: vec![0],
        }]
    );
    assert_eq!(submitted(&s.system_rx, 0), []);
}

#[test]
fn uncached_system_line_goes_to_the_system_pool() {
    let mut s = Setup::new().system_messages(true);
    let events = s.run(vec![
        Read::Lines(vec![drop_line("你获得了 39 金币！")]),
        lines(&[]),
    ]);
    assert_eq!(
        texts(&events),
        [("你获得了 39 金币！".to_owned(), pending())]
    );
    assert!(added(&events)[0].pending);
    assert_eq!(added(&events)[0].msg_id, Some(1));
    assert_eq!(
        submitted(&s.system_rx, 1),
        [("你获得了 39 金币！".to_owned(), vec![], 1)]
    );
    // 不得混進玩家對話那條佇列
    assert_eq!(submitted(&s.pool_rx, 0), []);
}

#[test]
fn banner_shows_a_system_pool_error_when_the_player_pool_is_clean() {
    let mut s = Setup::new().system_messages(true);
    s.system_pool = failing_pool(&s.rt, TranslateError::offline(""));
    let events = s.run(empty(2));
    // 執行緒結束時 board.remove(slot) 把唯一的編號收回，聚合結果變回「找不到遊戲」
    assert_eq!(
        banners(&events),
        [("notice.offline", vec![]), ("notice.game_missing", vec![])]
    );
}

#[test]
fn system_lines_are_ignored_when_the_setting_is_off() {
    let mut s = Setup::new().system_messages(false);
    s.run(vec![lines(&["[Lars] hi"]), lines(&[])]);
    assert_eq!(s.probe.lock().unwrap().emit_system, [false, false]);
    assert_eq!(submitted(&s.system_rx, 0), []);
}

#[test]
fn reader_emit_system_is_resynced_every_loop_iteration() {
    // 設定可能在執行中從設定視窗切換：第一輪讀取後切成 True，第二輪開頭就要跟上
    let mut s = Setup::new().system_messages(false);
    let mut reader = s.reader(empty(3));
    let settings = Arc::clone(&s.settings);
    reader.on_read = Some(Box::new(move |n| {
        if n == 0 {
            settings
                .write()
                .unwrap()
                .insert("translate_system_messages".into(), json!(true));
        }
    }));
    s.run_reader(reader);
    assert_eq!(s.probe.lock().unwrap().emit_system, [false, true, true]);
}

#[test]
fn new_lines_are_placeheld_and_submitted_in_order() {
    let mut s = Setup::new();
    let events = s.run(vec![
        lines(&[]),
        lines(&["[A] a"]),
        lines(&["[B] hi", "[B] hi"]),
        lines(&[]),
    ]);
    // overlay 依讀取順序先佔位，順序不由翻譯完成先後決定
    assert_eq!(
        texts(&events),
        [
            ("[A] a".to_owned(), pending()),
            ("[B] hi".to_owned(), pending()),
            ("[B] hi".to_owned(), pending()),
        ]
    );
    let got = submitted(&s.pool_rx, 3);
    let lines: Vec<_> = got.iter().map(|(l, _, id)| (l.as_str(), *id)).collect();
    assert_eq!(lines, [("[A] a", 1), ("[B] hi", 2), ("[B] hi", 3)]);
    let added = added(&events);
    assert!(added.iter().all(|a| a.pending && a.slot == 1));
    let ids: Vec<_> = added.iter().map(|a| a.msg_id).collect();
    assert_eq!(ids, [Some(1), Some(2), Some(3)]);
}

#[test]
fn game_color_flows_to_overlay_text_to_context_and_pool() {
    let mut s = Setup::new();
    let mut event = ev("[A] a");
    event.line.color = Some("#80ff00".into());
    let events = s.run(vec![Read::Lines(vec![event]), lines(&[])]);
    assert_eq!(added(&events)[0].color.as_deref(), Some("#80ff00"));
    assert_eq!(submitted(&s.pool_rx, 1), [("[A] a".to_owned(), vec![], 1)]);
    assert_eq!(s.contexts.current().snapshot(), ["[A] a"]);
}

#[test]
fn player_line_context_snapshot_before_push() {
    // 每則帶到的 context 是它「之前」的行 —— 平行翻譯時同批訊息仍看得到彼此
    let mut s = Setup::new();
    s.run(vec![
        lines(&["[A] one", "[B] two", "[C] three"]),
        lines(&[]),
    ]);
    let contexts: Vec<_> = submitted(&s.pool_rx, 3)
        .into_iter()
        .map(|(_, c, _)| c)
        .collect();
    assert_eq!(
        contexts,
        [
            vec![],
            vec!["[A] one".to_owned()],
            vec!["[A] one".to_owned(), "[B] two".to_owned()]
        ]
    );
    assert_eq!(
        s.contexts.current().snapshot(),
        ["[A] one", "[B] two", "[C] three"]
    );
}

#[test]
fn player_lines_use_their_own_tab_context() {
    let mut s = Setup::new();
    s.run(vec![
        Read::Lines(vec![
            on_tab(0, "[A] zero"),
            on_tab(1, "[B] one"),
            on_tab(1, "[B] two"),
        ]),
        lines(&[]),
    ]);
    let contexts: Vec<_> = submitted(&s.pool_rx, 3)
        .into_iter()
        .map(|(_, c, _)| c)
        .collect();
    assert_eq!(contexts, [vec![], vec![], vec!["[B] one".to_owned()]]);
}

#[test]
fn events_carry_their_tabs_to_the_overlay() {
    let mut s = Setup::new();
    let events = s.run(vec![Read::Lines(vec![on_tab(1, "[A] hi")]), lines(&[])]);
    assert_eq!(added(&events)[0].tabs, [1]);
}

#[test]
fn banner_follows_pool_error_state() {
    let mut s = Setup::new();
    s.pool = failing_pool(&s.rt, TranslateError::offline(""));
    let events = s.run(empty(2));
    assert_eq!(
        banners(&events),
        [("notice.offline", vec![]), ("notice.game_missing", vec![])]
    );
}

#[test]
fn loop_passes_the_pool_error_detail_to_the_banner() {
    let mut s = Setup::new();
    s.pool = failing_pool(
        &s.rt,
        TranslateError::Config {
            status: Some(401),
            detail: "Incorrect API key".into(),
        },
    );
    let events = s.run(empty(2));
    assert_eq!(
        banners(&events),
        [
            (
                "notice.config_error_detail",
                args(&[("status", "401"), ("message", "Incorrect API key")])
            ),
            ("notice.game_missing", vec![]),
        ]
    );
}

#[test]
fn status_shows_translating_while_pool_busy() {
    let mut s = Setup::new();
    let (busy, _) = pool(&s.rt, Arc::new(|_, _| Box::pin(std::future::pending())));
    s.pool = busy;
    let events = s.run(vec![lines(&["[A] a"]), lines(&[])]);
    assert!(statuses(&events).contains(&"translating"));
}

#[test]
fn status_transitions() {
    // 監聽 →（有新訊息、池忙碌）翻譯中 → 監聽：完整三段都要出現
    let mut s = Setup::new();
    let release = Arc::new(tokio::sync::Notify::new());
    let gate = Arc::clone(&release);
    let (held, _) = pool(
        &s.rt,
        Arc::new(move |line, _| {
            let gate = Arc::clone(&gate);
            Box::pin(async move {
                gate.notified().await;
                Ok(line)
            })
        }),
    );
    s.pool = Arc::clone(&held);
    let mut reader = s.reader(vec![lines(&[]), lines(&["[A] a"]), lines(&[])]);
    reader.on_read = Some(Box::new(move |n| {
        if n == 2 {
            release.notify_one();
            while held.in_flight() > 0 {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }));
    let events = s.run_reader(reader);
    // 執行緒結束時 board.remove(slot) 把唯一的編號收回，聚合結果變回「等待遊戲」
    assert_eq!(
        statuses(&events),
        ["listening", "translating", "listening", "waiting_game"]
    );
}

#[test]
fn status_locating_when_not_anchored() {
    let mut s = Setup::new();
    let mut reader = s.reader(empty(2));
    reader.anchored = false;
    let events = s.run_reader(reader);
    // 狀態未變不重複發；末尾是 board.remove(slot) 把唯一編號收回
    assert_eq!(statuses(&events), ["locating", "waiting_game"]);
}

#[test]
fn game_input_edges_are_logged_once_per_change() {
    // 只在「關→開」與「開→關」的邊緣各記一次，持續開著不重複
    let mut s = Setup::new();
    s.hwnd = 0xABC;
    s.slot = 17;
    let mut reader = s.reader(empty(5));
    reader.inputs = vec![false, true, true, false, false];
    s.run_reader(reader);
    assert_eq!(
        log_lines_with(
            "[reader] game chat input opened (slot=17, hwnd=0xabc, anchor=(749, 893, 732, 44))"
        )
        .len(),
        1
    );
    assert_eq!(
        log_lines_with("[reader] game chat input closed (slot=17, hwnd=0xabc)").len(),
        1
    );
}

fn input_events(events: &[UiEvent]) -> Vec<&UiEvent> {
    events
        .iter()
        .filter(|e| {
            matches!(
                e,
                UiEvent::GameInputOpened { .. } | UiEvent::GameInputClosed { .. }
            )
        })
        .collect()
}

#[test]
fn input_open_and_close_are_posted_with_the_anchor() {
    let mut s = Setup::new();
    s.hwnd = 0xABC;
    let mut reader = s.reader(empty(4));
    reader.inputs = vec![false, true, true, false];
    let events = s.run_reader(reader);
    assert_eq!(
        input_events(&events),
        [
            &UiEvent::GameInputOpened {
                hwnd: 0xABC,
                anchor: Some((749, 893, 732, 44))
            },
            &UiEvent::GameInputClosed { hwnd: 0xABC },
        ]
    );
}

#[test]
fn input_open_without_a_readable_rect_posts_no_anchor() {
    let mut s = Setup::new();
    let mut reader = s.reader(empty(2));
    reader.inputs = vec![true];
    reader.input_rect = None;
    let events = s.run_reader(reader);
    assert_eq!(
        input_events(&events)[0],
        &UiEvent::GameInputOpened {
            hwnd: 0x1,
            anchor: None
        }
    );
}

#[test]
fn game_window_gone_posts_input_closed_when_it_was_open() {
    let mut s = Setup::new();
    let mut reader = s.reader(empty(5));
    reader.inputs = vec![true];
    let calls = Arc::new(Mutex::new(0));
    s.window_exists = Box::new(move |_| {
        let mut n = calls.lock().unwrap();
        *n += 1;
        *n <= 1
    });
    let events = s.run_reader(reader);
    let posted = input_events(&events);
    assert!(matches!(posted[0], UiEvent::GameInputOpened { .. }));
    assert_eq!(posted[1], &UiEvent::GameInputClosed { hwnd: 0x1 });
    assert_eq!(posted.len(), 2);
}

#[test]
fn input_events_are_edge_triggered() {
    let mut s = Setup::new();
    let mut reader = s.reader(empty(4));
    reader.inputs = vec![false, false, false, false];
    let events = s.run_reader(reader);
    assert!(input_events(&events).is_empty());
}

#[test]
fn input_box_is_sampled_during_the_wait_between_polls() {
    // 聊天欄在兩輪讀取之間被打開：不該等到下一輪才發現
    let mut s = Setup::new();
    s.run(empty(2));
    let probe = s.probe.lock().unwrap();
    assert_eq!(probe.read_at, [Duration::ZERO, POLL_INTERVAL]);
    assert_eq!(
        probe.input_at,
        [
            Duration::ZERO,
            INPUT_POLL_INTERVAL,
            POLL_INTERVAL,
            POLL_INTERVAL
        ]
    );
}

#[test]
fn game_missing_retries_every_5s_and_logs_once_per_issue() {
    let mut s = Setup::new();
    s.slot = 19;
    let events = s.run(vec![
        Read::Fail(GameNotRunning::Missing("no game".into())),
        Read::Fail(GameNotRunning::Missing("no game".into())),
        Read::Fail(GameNotRunning::AccessDenied("denied".into())),
        lines(&[]),
    ]);
    let read_at = s.probe.lock().unwrap().read_at.clone();
    assert_eq!(
        read_at,
        [
            Duration::ZERO,
            GAME_MISSING_INTERVAL,
            GAME_MISSING_INTERVAL * 2,
            GAME_MISSING_INTERVAL * 3
        ]
    );
    assert_eq!(
        log_lines_with("[reader] game not ready (slot=19, hwnd=0x1): no game").len(),
        1
    );
    assert_eq!(
        log_lines_with("[reader] game not ready (slot=19, hwnd=0x1): denied").len(),
        1
    );
    assert_eq!(
        log_lines_with("[reader] game back, resuming (slot=19)").len(),
        1
    );
    assert_eq!(
        banners(&events),
        [
            ("notice.game_missing", vec![]),
            ("notice.access_denied", vec![]),
            ("<clear>", vec![]),
            ("notice.game_missing", vec![]),
        ]
    );
    let statuses = statuses(&events);
    assert!(statuses.contains(&"waiting_game") && statuses.contains(&"access_denied"));
    assert!(texts(&events).is_empty());
}

#[test]
fn version_mismatch_shows_its_own_banner_and_status() {
    let mut s = Setup::new();
    let mismatch = || Read::Fail(GameNotRunning::VersionMismatch("pattern failed".into()));
    let events = s.run(vec![mismatch(), mismatch()]);
    assert_eq!(
        banners(&events),
        [
            ("notice.version_mismatch", vec![]),
            ("notice.game_missing", vec![])
        ]
    );
    assert!(statuses(&events).contains(&"version_mismatch"));
}

#[test]
fn window_gone_stops_reader() {
    // 客戶端關掉：這條執行緒要 unhook 並結束，編號交還 supervisor
    let mut s = Setup::new();
    s.slot = 23;
    let mut alive = [true, false].into_iter();
    s.window_exists = Box::new(move |_| alive.next().unwrap_or(false));
    let mut reader = s.reader(empty(3));
    reader.states = vec![Some(tab_state(0))];
    let events = s.run_reader(reader);
    assert!(s.probe.lock().unwrap().closed);
    let tabs = tab_events(&events);
    assert_eq!(tabs.last(), Some(&(23, None)), "關掉的客戶端不該留下分頁列");
    assert!(
        !s.stop.is_cancelled(),
        "視窗消失只結束這一條，不該停掉整個程式"
    );
    assert_eq!(statuses(&events).last(), Some(&"waiting_game"));
    assert_eq!(
        log_lines_with("[reader] game window gone (slot=23, hwnd=0x1); stopping this reader").len(),
        1
    );
}

#[test]
fn reader_panic_still_drops_slot_and_closes() {
    quiet_panics();
    let mut s = Setup::new();
    let reader = s.reader(vec![Read::Panic]);
    let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| s.run_reader(reader)));
    assert!(outcome.is_err());
    assert!(s.probe.lock().unwrap().closed);
    let events = drain(&s.ui_rx);
    // 回報過一次「監聽中」，drop 後聚合變回「等待遊戲」
    assert_eq!(statuses(&events), ["listening", "waiting_game"]);
    assert_eq!(banners(&events), [("notice.game_missing", vec![])]);
}

#[test]
fn message_ids_are_unique_across_threads() {
    let ids = Arc::new(MessageIds::default());
    let threads: Vec<_> = (0..4)
        .map(|_| {
            let ids = Arc::clone(&ids);
            std::thread::spawn(move || (0..500).map(|_| ids.next()).collect::<Vec<_>>())
        })
        .collect();
    let mut got: Vec<u64> = threads
        .into_iter()
        .flat_map(|t| t.join().unwrap())
        .collect();
    got.sort_unstable();
    got.dedup();
    assert_eq!(got.len(), 2000);
    assert_eq!(got[0], 1);
}

#[test]
fn tab_state_is_sent_only_when_it_changes() {
    let mut s = Setup::new();
    let mut reader = s.reader(empty(3));
    reader.states = vec![Some(tab_state(1))];
    let events = s.run_reader(reader);
    assert_eq!(tab_events(&events), [(1, Some(tab_state(1)))]);
    assert_eq!(s.contexts.current_tab(), 1);
}

#[test]
fn tab_state_is_cleared_when_the_game_goes_away() {
    let mut s = Setup::new();
    let mut reader = s.reader(vec![
        lines(&[]),
        Read::Fail(GameNotRunning::Missing("gone".into())),
    ]);
    reader.states = vec![Some(tab_state(0))];
    let events = s.run_reader(reader);
    assert_eq!(tab_events(&events), [(1, Some(tab_state(0))), (1, None)]);
    assert_eq!(s.contexts.current_tab(), 0);
}

#[test]
fn foreground_slot_posted_on_each_full_sample() {
    // 每次完整取樣都送（不只在變成前景的邊緣）：兩條執行緒錯開時快速 A→B→A 才不會停在 B
    let mut s = Setup::new();
    // 另一條 reader 在每次取樣之間把前景送成客戶端 2
    let other = s.ui.clone();
    s.foreground = Box::new(move || {
        other.post(UiEvent::SetForegroundSlot(Some(2)));
        0x1
    });
    // 每輪 POLL_INTERVAL，完整取樣每 TAB_POLL_INTERVAL 一次：第 0、4、8 輪
    let events = s.run(empty(9));
    assert_eq!(s.probe.lock().unwrap().tab_reads, 3);
    let ours: Vec<_> = foreground_events(&events)
        .into_iter()
        .filter(|slot| *slot == Some(1))
        .collect();
    assert_eq!(ours, [Some(1), Some(1), Some(1)]);
    // 取樣當下不在前景就不送
    let mut s = Setup::new();
    let other = s.ui.clone();
    let mut seq = [0x1, 0x9, 0x1].into_iter();
    s.foreground = Box::new(move || {
        other.post(UiEvent::SetForegroundSlot(Some(2)));
        seq.next().unwrap_or(0)
    });
    let events = s.run(empty(9));
    let ours: Vec<_> = foreground_events(&events)
        .into_iter()
        .filter(|slot| *slot == Some(1))
        .collect();
    assert_eq!(ours, [Some(1), Some(1)]);
}

#[test]
fn tab_state_and_foreground_are_sampled_slower_than_chat() {
    // 環形緩衝每輪都讀；分頁狀態與前景只按 TAB_POLL_INTERVAL 取樣（4 輪剛好在一個間隔內）
    let mut s = Setup::new();
    s.foreground = Box::new(|| 0x1);
    let events = s.run((0..4).map(|i| lines(&[&format!("[A] {i}")])).collect());
    assert_eq!(submitted(&s.pool_rx, 4).len(), 4);
    assert_eq!(s.probe.lock().unwrap().tab_reads, 1);
    assert_eq!(foreground_events(&events), [Some(1)]);
}

#[test]
fn a_tab_switch_is_reported_without_waiting_for_the_slow_sample() {
    // 目前分頁每輪都看：遊戲裡切分頁，下一輪就送完整分頁狀態
    let mut s = Setup::new();
    let mut reader = s.reader(empty(4));
    let (main, team) = (tab_state(0), tab_state(1));
    reader.states = vec![
        Some(main.clone()),
        Some(main.clone()),
        Some(team.clone()),
        Some(team.clone()),
    ];
    let events = s.run_reader(reader);
    assert_eq!(tab_events(&events), [(1, Some(main)), (1, Some(team))]);
}
