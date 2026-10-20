//! WizChatReader：掛入順序、失敗分類與還原、事件過濾、分頁狀態與輸入框（以 FakeMemory 模擬遊戲）。

use crate::common;

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard};
use std::time::Duration;

use common::fake_game::{
    BASE, RING_SIZE, place_addline, place_cave, place_root_window, put_button, put_children,
    put_name, put_slot, slot_addr,
};
use common::log_capture::{capture_logs, log_lines_with};
use common::reader_fixture::VirtualClock;
use common::spy_memory::SpyMemory;
use windows_sys::Win32::UI::WindowsAndMessaging::GetDesktopWindow;
use wizard101_chat_translator::reader::chat_hook::SLOTS;
use wizard101_chat_translator::reader::chat_tabs::{CUR_TAB_OFFSET, TabState};
use wizard101_chat_translator::reader::hook_state;
use wizard101_chat_translator::reader::hooks::{MODULE, STUB_EXIT_WAIT};
use wizard101_chat_translator::reader::markup::parse_line;
use wizard101_chat_translator::reader::mem_reader::{
    GameNotRunning, HOOK_READY_POLL, HOOK_READY_TIMEOUT, INPUT_CONTAINER, INPUT_RECHECK,
    MessageSink, WizChatReader,
};
use wizard101_chat_translator::reader::memory::{FakeMemory, MemError, MemOp, ProcessMemory};
use wizard101_chat_translator::reader::process::{
    client_size, client_to_screen, install_path_of, install_root, window_exists, window_pid,
};
use wizard101_chat_translator::reader::ui_rect::client_rect;
use wizard101_chat_translator::util::{self, Clock};

const CAVE: u64 = 0x1_4000_1000;
const ROOT_JUMP: u64 = 0x1_4080_0000;
const ADDLINE_JUMP: u64 = 0x1_4120_0000;
/// FakeMemory 的第一次配置（root window export）與下一次（環形緩衝）。
const EXPORT: u64 = 0x2000_0000;
const RING: u64 = 0x2001_0000;
const ROOT: u64 = 0x3000_0000;
const BOX: u64 = 0x3040_0000;
const OTHER: u64 = 0x3080_0000;
const EDIT: u64 = 0x30C0_0000;
/// 分頁按鈕從這裡起每個間隔 `NODE_GAP`。
const BUTTONS: u64 = 0x3400_0000;
const NODE_GAP: u64 = 0x40_0000;
const TAB_BUTTONS: [&str; 9] = [
    "MainFilterButton",
    "GroupFilterButton",
    "HouseFilterButton",
    "FriendFilterButton",
    "Channel1FilterButton",
    "Channel2FilterButton",
    "Channel3FilterButton",
    "Channel4FilterButton",
    "TeamUpFilterButton",
];

fn hwnd() -> isize {
    unsafe { GetDesktopWindow() as isize }
}

fn pid() -> u32 {
    window_pid(hwnd())
}

fn logged(slot: u32, needle: &str) -> Vec<String> {
    log_lines_with(&format!("slot={slot}"))
        .into_iter()
        .filter(|l| l.contains(needle))
        .collect()
}

/// messages.log 的替身：照 Python `MessageLog` 的格式記下每一行（不濾除錯行）。
#[derive(Clone, Default)]
struct Lines(Arc<Mutex<Vec<String>>>);

impl MessageSink for Lines {
    fn record(&mut self, seq: u64, tab: u32, raw: &str, verdict: &str) {
        self.0
            .lock()
            .unwrap()
            .push(format!("[seq={seq} tab={tab} {verdict}] {raw}"));
    }

    fn note(&mut self, text: &str) {
        self.0.lock().unwrap().push(text.to_owned());
    }
}

impl Lines {
    fn all(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }

    fn count(&self, needle: &str) -> usize {
        self.all().iter().filter(|l| l.contains(needle)).count()
    }
}

/// 警告過的圖示集合是全程序共用的：連線成功的測試持讀鎖，要斷言它的測試持寫鎖。
static ICON_SET: RwLock<()> = RwLock::new(());

/// 同一執行緒的巢狀 `Game` 共用一把讀鎖：已有寫者在等時再取一次讀鎖會死鎖。
struct IconGuard;

thread_local! {
    static ICON_READ: RefCell<(usize, Option<RwLockReadGuard<'static, ()>>)> =
        const { RefCell::new((0, None)) };
}

impl IconGuard {
    fn acquire() -> Self {
        ICON_READ.with_borrow_mut(|(depth, guard)| {
            if *depth == 0 {
                *guard = Some(util::read(&ICON_SET));
            }
            *depth += 1;
        });
        Self
    }
}

impl Drop for IconGuard {
    fn drop(&mut self) {
        ICON_READ.with_borrow_mut(|(depth, guard)| {
            *depth -= 1;
            if *depth == 0 {
                *guard = None;
            }
        });
    }
}

struct Game {
    mem: Arc<FakeMemory>,
    dir: tempfile::TempDir,
    /// 假時鐘；sleep 只推進它。
    clock: VirtualClock,
    /// 等待中 root window hook 是否會「被執行」（把 root 位址寫進 export）。
    fires: Arc<AtomicBool>,
    /// 交給 reader 的記憶體：與 `mem` 同一份，並記錄／弄壞 export 讀取與寫入。
    spy: SpyMemory,
    /// 第一次等待時 hookstate 檔的內容。
    state_at_wait: Arc<Mutex<Option<hook_state::RestoreOps>>>,
    opened: Arc<Mutex<Vec<u32>>>,
    root_bytes: Vec<u8>,
    addline_bytes: Vec<u8>,
    cave_bytes: Vec<u8>,
    /// 存活期間擋住「清空警告過的圖示」的測試（連線成功會清）。
    _icons: Option<IconGuard>,
}

impl Game {
    fn new() -> Self {
        Self::with(true, true)
    }

    fn with(root_pattern: bool, addline_pattern: bool) -> Self {
        Self {
            _icons: Some(IconGuard::acquire()),
            ..Self::bare(root_pattern, addline_pattern)
        }
    }

    /// 不持有 `ICON_SET` 讀鎖的版本：要自己握寫鎖的測試用。
    fn bare(root_pattern: bool, addline_pattern: bool) -> Self {
        capture_logs();
        let mem = FakeMemory::new();
        mem.add_module(MODULE, BASE);
        let cave_bytes = place_cave(&mem, CAVE);
        let root_bytes = if root_pattern {
            place_root_window(&mem, ROOT_JUMP)
        } else {
            Vec::new()
        };
        let addline_bytes = if addline_pattern {
            place_addline(&mem, ADDLINE_JUMP)
        } else {
            Vec::new()
        };
        put_name(&mem, ROOT, "WorldView");
        put_children(&mem, ROOT, &[BOX]);
        put_name(&mem, BOX, "WizardChatBox");
        put_children(&mem, BOX, &[]);
        let mem = Arc::new(mem);
        Self {
            spy: SpyMemory::over(mem.clone()),
            mem,
            dir: tempfile::tempdir().unwrap(),
            clock: VirtualClock::new(),
            fires: Arc::new(AtomicBool::new(true)),
            state_at_wait: Arc::default(),
            opened: Arc::default(),
            root_bytes,
            addline_bytes,
            cave_bytes,
            _icons: None,
        }
    }

    fn reader(&self, slot: u32) -> WizChatReader {
        let (spy, opened) = (self.spy.clone(), self.opened.clone());
        let mut r = WizChatReader::new(
            hwnd(),
            Box::new(move |pid| {
                opened.lock().unwrap().push(pid);
                Ok(Box::new(spy.clone()) as Box<dyn ProcessMemory>)
            }),
        );
        r.slot = slot;
        r.state_dir = self.dir.path().to_owned();
        let clock = self.clock.clone();
        r.clock = Clock {
            now: Box::new(move || clock.now()),
            sleep: self.on_sleep(),
        };
        r
    }

    /// 假 sleep：推進假時鐘；root window hook 會被執行時把 root 位址寫進目前的 export。
    fn on_sleep(&self) -> Box<dyn Fn(Duration) + Send + Sync> {
        let (mem, clock, fires, seen, dir) = (
            self.mem.clone(),
            self.clock.clone(),
            self.fires.clone(),
            self.state_at_wait.clone(),
            self.dir.path().to_owned(),
        );
        Box::new(move |d| {
            clock.advance(d);
            let export = mem.ops().iter().rev().find_map(|op| match op {
                MemOp::Alloc { addr, size: 8 } => Some(*addr),
                _ => None,
            });
            let Some(export) = export else { return };
            if matches!(mem.read_u64(export), Ok(0)) {
                seen.lock()
                    .unwrap()
                    .get_or_insert_with(|| hook_state::load_state(&dir, pid()).1);
                if fires.load(Ordering::SeqCst) {
                    mem.set_bytes(export, &ROOT.to_le_bytes());
                }
            }
        })
    }

    fn connected(&self, slot: u32) -> WizChatReader {
        let mut r = self.reader(slot);
        assert_eq!(r.read_new().unwrap(), vec![]);
        assert!(r.anchored());
        r
    }

    /// 最近一次配置的環形緩衝位址。
    fn ring(&self) -> u64 {
        self.mem
            .ops()
            .iter()
            .rev()
            .find_map(|op| match op {
                MemOp::Alloc { addr, size } if *size == RING_SIZE => Some(*addr),
                _ => None,
            })
            .expect("ring allocated")
    }

    /// cave 解除時依序的寫回與釋放；`addline` 為 false 表示 AddLine 沒掛上。
    fn restore_ops(&self, addline: bool) -> Vec<MemOp> {
        let mut ops = vec![MemOp::Write {
            addr: ROOT_JUMP,
            bytes: self.root_bytes[..7].to_vec(),
        }];
        if addline {
            ops.push(MemOp::Write {
                addr: ADDLINE_JUMP,
                bytes: self.addline_bytes[..5].to_vec(),
            });
        }
        ops.push(MemOp::Free(EXPORT));
        ops.push(MemOp::Write {
            addr: CAVE,
            bytes: self.cave_bytes[..208].to_vec(),
        });
        ops
    }

    fn state_file_exists(&self) -> bool {
        self.dir
            .path()
            .join(format!("hookstate-{}.json", pid()))
            .exists()
    }

    fn put(&self, seq: u64, text: &str, tab: u32, this: u64) {
        put_record(&self.mem, self.ring(), seq, text, tab, this);
    }

    fn set_chatbox(&self, chatbox: Option<u64>) {
        let kids: Vec<u64> = chatbox.into_iter().collect();
        put_children(&self.mem, ROOT, &kids);
        if let Some(addr) = chatbox {
            put_name(&self.mem, addr, "WizardChatBox");
            put_children(&self.mem, addr, &[]);
        }
    }
}

/// 寫一筆文字記錄並把累計數推到至少 `seq`。
fn put_record(mem: &FakeMemory, ring: u64, seq: u64, text: &str, tab: u32, this: u64) {
    let units: Vec<u16> = text.encode_utf16().collect();
    put_slot(mem, ring, seq, &units, tab, this, None);
}

fn say(name: &str, text: &str) -> String {
    format!(
        "<color;FFFFFF><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> \
         <link;GID:1,{name},2>[{name}]</link> {text} </color>"
    )
}

fn system(text: &str) -> String {
    format!("<color;00FF00><image;Art/Art_Chat_System.dds;24;24;FFFFFFFF> {text}</color>")
}

fn own(text: &str) -> String {
    format!("<color;FFFFFF><image;Art/Art_Chat_Say.dds;24;24;FFFFFFFF> [你] {text} </color>")
}

fn broadcast(text: &str) -> String {
    format!("<color;D9ABF8>{text}</color>")
}

fn texts(r: &mut WizChatReader) -> Vec<(Vec<i32>, String)> {
    r.read_new()
        .unwrap()
        .into_iter()
        .map(|e| (e.tabs, e.line.text))
        .collect()
}

fn ops_kinds(ops: &[MemOp]) -> Vec<(&'static str, u64)> {
    ops.iter()
        .map(|op| match op {
            MemOp::Write { addr, .. } => ("write", *addr),
            MemOp::Alloc { addr, .. } => ("alloc", *addr),
            MemOp::Free(addr) => ("free", *addr),
        })
        .collect()
}

// --- 掛入 ---

#[test]
fn connect_order_matches_python() {
    let g = Game::new();
    let dead = u32::MAX - 3;
    hook_state::save_state(g.dir.path(), dead, BASE, &[(0x1234, vec![1])]).unwrap();
    let leaked = vec![(ROOT_JUMP, g.root_bytes[..7].to_vec())];
    hook_state::save_state(g.dir.path(), pid(), BASE, &leaked).unwrap();

    let _r = g.connected(101);

    assert_eq!(*g.opened.lock().unwrap(), [pid()]);
    let stub = |at: u64| ("write", at);
    assert_eq!(
        ops_kinds(&g.mem.ops()),
        [
            ("write", ROOT_JUMP), // 修復上次髒退出遺留的 hook
            ("write", CAVE),
            ("alloc", EXPORT),
            stub(CAVE),
            ("write", ROOT_JUMP),
            ("alloc", RING),
            stub(CAVE + 50),
            ("write", ADDLINE_JUMP),
        ]
    );
    assert_eq!(
        g.state_at_wait.lock().unwrap().clone(),
        Some(vec![
            (ROOT_JUMP, g.root_bytes[..7].to_vec()),
            (CAVE, g.cave_bytes[..208].to_vec()),
        ]),
        "hookstate must be saved before waiting for the root hook"
    );
    assert_eq!(
        hook_state::load_state(g.dir.path(), pid()),
        (
            Some(BASE),
            vec![
                (ROOT_JUMP, g.root_bytes[..7].to_vec()),
                (ADDLINE_JUMP, g.addline_bytes[..5].to_vec()),
                (CAVE, g.cave_bytes[..208].to_vec()),
            ]
        )
    );
    assert_eq!(hook_state::load_state(g.dir.path(), dead), (None, vec![]));
    let p = pid();
    for line in [
        format!("[reader] game path: unknown (slot=101, pid={p})"),
        format!(
            "[reader] AddLine hook installed (slot=101, pid={p}, jump=0x141200000, \
             ring=0x20010000, ring_size=0x80040)"
        ),
        format!(
            "[reader] attached to game (slot=101, pid={p}, hwnd={:#x}, hook_ready_in=0.3s)",
            hwnd()
        ),
    ] {
        assert_eq!(logged(101, &line).len(), 1, "{line}");
    }
}

#[test]
fn hook_state_is_saved_as_soon_as_the_cave_is_cleared() {
    let g = Game::new();
    let (dir, snapshot) = (g.dir.path().to_owned(), Arc::new(Mutex::new(None)));
    let seen = snapshot.clone();
    *g.spy.after_read.lock().unwrap() = Some(Box::new(move |_, _, mem| {
        let cleared = mem
            .ops()
            .iter()
            .any(|op| matches!(op, MemOp::Write { addr: CAVE, .. }));
        if cleared {
            seen.lock()
                .unwrap()
                .get_or_insert_with(|| hook_state::load_state(&dir, pid()).1);
        }
    }));
    let _r = g.connected(113);
    let saved = snapshot.lock().unwrap().clone();
    assert_eq!(
        saved,
        Some(vec![(CAVE, g.cave_bytes[..208].to_vec())]),
        "hookstate must exist before the root pattern scan"
    );
}

#[test]
fn connect_waits_through_transient_read_errors() {
    let g = Game::new();
    g.spy.fail_reads_at(EXPORT, 3);
    let _r = g.connected(102);
    assert!(g.clock.elapsed() >= HOOK_READY_POLL);
}

#[test]
fn open_failure_is_access_denied() {
    capture_logs();
    let mut r = WizChatReader::new(hwnd(), Box::new(|_| Err(MemError::AccessDenied(4321))));
    let err = r.read_new().unwrap_err();
    assert_eq!(
        err,
        GameNotRunning::AccessDenied(
            "cannot open game process handle (Could not open process: 4321); the game is \
             likely running elevated while this program is not"
                .into()
        )
    );
    assert!(!r.anchored());

    let mut r = WizChatReader::new(hwnd(), Box::new(|_| Err(MemError::Closed)));
    assert_eq!(
        r.read_new().unwrap_err(),
        GameNotRunning::Missing(
            "failed to open game process: Client must be running to preform this action.".into()
        )
    );
}

#[test]
fn pattern_failure_is_version_mismatch_and_leaves_no_partial_hook() {
    let g = Game::with(true, false);
    let mut r = g.reader(103);
    let err = r.read_new().unwrap_err();
    let GameNotRunning::VersionMismatch(msg) = &err else {
        panic!("{err:?}");
    };
    assert!(
        msg.starts_with("hook does not match this game build: Pattern b'\\\\x48\\\\x89"),
        "{msg}"
    );
    assert!(!r.anchored());
    let ops = g.mem.ops();
    assert!(
        !ops.iter()
            .any(|op| matches!(op, MemOp::Alloc { size, .. } if *size == RING_SIZE))
    );
    assert_eq!(ops[ops.len() - 3..], g.restore_ops(false)[..]);
    assert!(!g.state_file_exists());
    let failed = logged(103, "[reader] AddLine hook install failed (slot=103, ");
    assert_eq!(failed.len(), 1);
    assert!(
        failed[0].contains("): Pattern b'")
            && failed[0].ends_with(
                "; a pattern failure here needs chat_hook::PATTERN updated in a new release"
            ),
        "{failed:?}"
    );

    let g = Game::new();
    place_root_window(&g.mem, ROOT_JUMP + 0x1000);
    let mut r = g.reader(103);
    let err = r.read_new().unwrap_err();
    assert!(
        matches!(&err, GameNotRunning::VersionMismatch(msg) if msg.contains(": Got 2 results for b'")),
        "{err:?}"
    );

    let g = Game::with(false, true);
    let mut r = g.reader(103);
    assert!(matches!(
        r.read_new(),
        Err(GameNotRunning::VersionMismatch(_))
    ));
    let ops = g.mem.ops();
    assert_eq!(
        ops.last(),
        Some(&MemOp::Write {
            addr: CAVE,
            bytes: g.cave_bytes[..208].to_vec()
        })
    );
    assert!(!g.state_file_exists());
}

#[test]
fn unknown_icon_warnings_survive_failed_connects_but_not_successful_ones() {
    let warns = |icon: &str| {
        let line = format!("<color;FFFFFF><image;Art/{icon}.dds;24;24;FFFFFFFF> [Zed] hi </color>");
        assert!(parse_line(&line).is_none());
        log_lines_with(&format!("unrecognized chat icon 'Art/{icon}'")).len()
    };
    let _exclusive = util::write(&ICON_SET);
    let game = Game::bare(true, false);
    let mut failing = game.reader(150);
    assert_eq!(warns("Retry_Probe_Icon"), 1);
    assert!(failing.read_new().is_err());
    assert_eq!(
        warns("Retry_Probe_Icon"),
        1,
        "a failed connect cleared the warned-icon set"
    );

    let icon = "Reset_Probe_Icon";
    assert_eq!(warns(icon), 1);
    let connected = Game::bare(true, true);
    let _reader = connected.connected(151);
    assert_eq!(warns(icon), 2);
}

#[test]
fn repeated_attach_failures_log_the_same_reason_once() {
    let g = Game::with(true, false);
    let mut r = g.reader(152);
    for _ in 0..3 {
        assert!(matches!(
            r.read_new(),
            Err(GameNotRunning::VersionMismatch(_))
        ));
    }
    assert_eq!(logged(152, "[reader] game path:").len(), 1);
    assert_eq!(logged(152, "[reader] AddLine hook install failed").len(), 1);

    let g = Game::new();
    g.fires.store(false, Ordering::SeqCst);
    let mut r = g.reader(153);
    for _ in 0..3 {
        assert!(r.read_new().is_err());
    }
    assert_eq!(logged(153, "[reader] game path:").len(), 1);
    assert_eq!(
        logged(153, "[reader] root window hook never fired").len(),
        1
    );
}

#[test]
fn readiness_timeout_is_a_hook_not_firing_mismatch() {
    let g = Game::new();
    g.fires.store(false, Ordering::SeqCst);
    let mut r = g.reader(104);
    assert_eq!(
        r.read_new().unwrap_err(),
        GameNotRunning::HookNotFiring(
            "hook does not match this game build: root window hook did not fire within 15s".into()
        )
    );
    let waited = g.clock.elapsed();
    assert!(waited >= HOOK_READY_TIMEOUT, "{waited:?}");
    assert!(waited <= HOOK_READY_TIMEOUT + HOOK_READY_POLL + Duration::from_millis(500));
    let ops = g.mem.ops();
    assert!(
        !ops.iter()
            .any(|op| matches!(op, MemOp::Alloc { size, .. } if *size == RING_SIZE))
    );
    assert_eq!(ops[ops.len() - 3..], g.restore_ops(false)[..]);
    assert!(!g.state_file_exists());
    assert_eq!(
        logged(
            104,
            &format!(
                "[reader] root window hook never fired within 15s (slot=104, pid={}); the hook \
                 pattern likely does not match this game build",
                pid()
            )
        )
        .len(),
        1
    );
}

#[test]
fn stop_or_game_exit_ends_the_readiness_wait_early() {
    for stopping in [true, false] {
        let g = Game::new();
        g.fires.store(false, Ordering::SeqCst);
        let mut r = g.reader(112);
        let (inner, stop, mem) = (g.on_sleep(), r.stop.clone(), g.mem.clone());
        r.clock.sleep = Box::new(move |d| {
            inner(d);
            if stopping {
                stop.cancel();
            } else {
                mem.kill();
            }
        });
        let err = r.read_new().unwrap_err();
        assert!(
            matches!(&err, GameNotRunning::Missing(msg) if msg.starts_with("failed to attach")),
            "{err:?}"
        );
        let waited = g.clock.elapsed();
        assert!(waited <= HOOK_READY_POLL + STUB_EXIT_WAIT, "{waited:?}");
        if stopping {
            let ops = g.mem.ops();
            assert_eq!(ops[ops.len() - 3..], g.restore_ops(false)[..]);
        }
        assert!(!g.state_file_exists());
    }
}

#[test]
fn stop_before_connecting_writes_nothing() {
    let g = Game::new();
    let mut r = g.reader(114);
    r.stop.cancel();
    let err = r.read_new().unwrap_err();
    assert!(
        matches!(&err, GameNotRunning::Missing(msg) if msg.starts_with("failed to attach")),
        "{err:?}"
    );
    assert!(g.mem.ops().is_empty());
    assert!(!g.state_file_exists());
}

#[test]
fn other_attach_failure_stays_game_not_running() {
    capture_logs();
    let mut r = WizChatReader::new(
        hwnd(),
        Box::new(|_| Ok(Box::new(FakeMemory::new()) as Box<dyn ProcessMemory>)),
    );
    let dir = tempfile::tempdir().unwrap();
    r.state_dir = dir.path().to_owned();
    assert_eq!(
        r.read_new().unwrap_err(),
        GameNotRunning::Missing(
            "failed to attach to game: WizardGraphicalClient.exe module not found.".into()
        )
    );
}

#[test]
fn close_unhooks_and_clears_hook_state() {
    let g = Game::new();
    let mut r = g.connected(105);
    let before = g.mem.ops().len();
    r.close();
    assert!(!r.anchored());
    assert_eq!(g.mem.ops()[before..], g.restore_ops(true)[..]);
    assert!(!g.state_file_exists());
    r.close();
    assert_eq!(g.mem.ops().len(), before + 4);
}

#[test]
fn dropping_the_reader_unhooks_like_close() {
    let g = Game::new();
    let r = g.connected(106);
    let before = g.mem.ops().len();
    drop(r);
    assert_eq!(g.mem.ops()[before..], g.restore_ops(true)[..]);
    assert!(!g.state_file_exists());
}

#[test]
fn unhook_failure_keeps_hook_state_for_repair() {
    let g = Game::new();
    let mut r = g.connected(107);
    g.spy.fail_writes.store(true, Ordering::SeqCst);
    r.close();
    assert!(g.state_file_exists());
    assert_eq!(
        logged(
            107,
            &format!(
                "[reader] unhook failed, leaving repair state for next launch (slot=107): \
                 Unable to write memory at address {ROOT_JUMP}."
            )
        )
        .len(),
        1
    );
}

#[test]
fn closing_after_the_game_exited_writes_nothing_and_clears_state() {
    let g = Game::new();
    let mut r = g.connected(108);
    let before = g.mem.ops().len();
    g.mem.kill();
    r.close();
    assert_eq!(g.mem.ops().len(), before);
    assert!(!g.state_file_exists());
}

#[test]
fn hook_state_save_failure_is_logged() {
    let g = Game::new();
    let mut r = g.reader(109);
    let blocker = g.dir.path().join("not-a-dir");
    std::fs::write(&blocker, "").unwrap();
    r.state_dir = blocker;
    assert_eq!(r.read_new().unwrap(), vec![]);
    let p = pid();
    let lines = logged(109, "[reader] could not save hook state");
    assert_eq!(lines.len(), 3, "{lines:?}");
    for (line, ops) in lines.iter().zip(1..) {
        assert!(line.contains(&format!("(slot=109, pid={p}, ops={ops}): ")));
    }
}

#[test]
fn ring_read_failure_disconnects_as_game_not_running() {
    let g = Game::new();
    let mut r = g.connected(110);
    g.mem.kill();
    assert_eq!(
        r.read_new().unwrap_err(),
        GameNotRunning::Missing(
            "chat ring read failed (likely left the game): Client must be running to preform \
             this action."
                .into()
        )
    );
    assert!(!r.anchored());
    assert!(!g.state_file_exists());
}

#[test]
fn teardown_resets_the_ring_cursor() {
    let g = Game::new();
    let mut r = g.connected(111);
    for seq in 1..=5 {
        g.put(seq, &say("Amy", &format!("m{seq}")), 0, BOX);
    }
    assert_eq!(r.read_new().unwrap().len(), 5);
    r.close();
    assert_eq!(r.read_new().unwrap(), vec![]);
    assert_ne!(g.ring(), RING);
    g.put(1, &say("Amy", "after"), 0, BOX);
    assert_eq!(texts(&mut r), [(vec![0], "[Amy] after".to_owned())]);
}

#[test]
fn install_path_is_the_parent_of_bin() {
    assert_eq!(
        install_root(r"C:\Games\W101\Bin\WizardGraphicalClient.exe").as_deref(),
        Some(r"C:\Games\W101")
    );
    assert_eq!(
        install_root(r"C:\Program Files\Mozilla Firefox\firefox.exe"),
        None
    );
    assert_eq!(install_path_of(std::process::id()), None);
    assert_eq!(install_path_of(0), None);
    assert!(window_exists(hwnd()));
}

// --- 事件 ---

#[test]
fn records_become_events_in_order() {
    let g = Game::new();
    let mut r = g.connected(201);
    g.put(1, &say("Amy", "hi"), 0, BOX);
    g.put(2, &own("yo"), 1, BOX);
    let events = r.read_new().unwrap();
    let got: Vec<_> = events
        .iter()
        .map(|e| (e.tabs.clone(), e.line.text.as_str()))
        .collect();
    assert_eq!(got, [(vec![0], "[Amy] hi"), (vec![1], "[你] yo")]);
}

#[test]
fn identical_consecutive_lines_both_emit() {
    let g = Game::new();
    let mut r = g.connected(202);
    r.emit_system = true;
    let drop = system("你获得了 39 金币！");
    g.put(1, &drop, 0, BOX);
    g.put(2, &drop, 0, BOX);
    let events = r.read_new().unwrap();
    assert_eq!(events.len(), 2);
    assert!(
        events
            .iter()
            .all(|e| e.line.system && e.line.text == "你获得了 39 金币！")
    );
}

#[test]
fn system_lines_suppressed_but_seq_advances() {
    let g = Game::new();
    let mut r = g.connected(203);
    let lines = Lines::default();
    r.message_log = Some(Box::new(lines.clone()));
    g.put(1, &system("掉寶"), 0, BOX);
    g.put(2, &say("Amy", "hi"), 0, BOX);
    assert_eq!(texts(&mut r), [(vec![0], "[Amy] hi".to_owned())]);
    r.emit_system = true;
    assert_eq!(r.read_new().unwrap(), vec![]);
    assert_eq!(lines.count("[seq=1 tab=0 system-off]"), 1);
    assert_eq!(lines.count("[seq=2 tab=0 player]"), 1);
}

#[test]
fn broadcast_becomes_one_event_for_all_tabs() {
    let g = Game::new();
    let mut r = g.connected(204);
    r.emit_system = true;
    let lines = Lines::default();
    r.message_log = Some(Box::new(lines.clone()));
    let notice = broadcast("[Server Message] maintenance");
    for i in 0..4 {
        g.put(i + 1, &notice, i as u32, BOX);
    }
    assert_eq!(
        r.read_new().unwrap(),
        vec![],
        "a 4-tab run is held for channel tabs"
    );
    assert_eq!(
        texts(&mut r),
        [(vec![0, 1, 2, 3], "[Server Message] maintenance".to_owned())]
    );
    assert_eq!(lines.count("[seq=1 tab=0 broadcast]"), 1);
    assert_eq!(lines.count("broadcast-dup"), 3);
    assert_eq!(lines.count("[seq=4 tab=3 broadcast-dup]"), 1);
    assert_eq!(
        logged(
            204,
            &format!(
                "[reader] server broadcast collapsed across 4 tabs (slot=204, pid={}, seq=1, \
                 tabs=(0, 1, 2, 3))",
                pid()
            )
        )
        .len(),
        1
    );
}

#[test]
fn partial_broadcast_held_one_round() {
    let g = Game::new();
    let mut r = g.connected(205);
    r.emit_system = true;
    let lines = Lines::default();
    r.message_log = Some(Box::new(lines.clone()));
    let notice = broadcast("[Server Message] maintenance");
    g.put(1, &say("Amy", "hi"), 0, BOX);
    g.put(2, &notice, 0, BOX);
    g.put(3, &notice, 1, BOX);
    assert_eq!(texts(&mut r), [(vec![0], "[Amy] hi".to_owned())]);
    g.put(4, &notice, 2, BOX);
    g.put(5, &notice, 3, BOX);
    assert_eq!(r.read_new().unwrap(), vec![], "grew again, held once more");
    assert_eq!(
        texts(&mut r),
        [(vec![0, 1, 2, 3], "[Server Message] maintenance".to_owned())]
    );
    assert_eq!(lines.count("[seq=2 "), 1, "held records are logged once");
}

#[test]
fn broadcast_split_after_four_tabs_is_one_event() {
    let g = Game::new();
    let mut r = g.connected(208);
    r.emit_system = true;
    let notice = broadcast("[Server Message] maintenance");
    for (seq, tab) in [(1, 0), (2, 1), (3, 2), (4, 3)] {
        g.put(seq, &notice, tab, BOX);
    }
    let first = texts(&mut r);
    g.put(5, &notice, 5, BOX);
    g.put(6, &notice, 6, BOX);
    assert_eq!(r.read_new().unwrap(), vec![], "grew again, held once more");
    let mut all = first;
    all.extend(texts(&mut r));
    assert_eq!(
        all,
        [(
            vec![0, 1, 2, 3, 5, 6],
            "[Server Message] maintenance".to_owned()
        )]
    );
}

#[test]
fn broadcast_cut_twice_is_still_one_event() {
    let g = Game::new();
    let mut r = g.connected(210);
    r.emit_system = true;
    let notice = broadcast("[Server Message] maintenance");
    g.put(1, &notice, 0, BOX);
    g.put(2, &notice, 1, BOX);
    assert_eq!(r.read_new().unwrap(), vec![]);
    g.put(3, &notice, 2, BOX);
    g.put(4, &notice, 3, BOX);
    assert_eq!(r.read_new().unwrap(), vec![]);
    g.put(5, &notice, 5, BOX);
    g.put(6, &notice, 6, BOX);
    assert_eq!(r.read_new().unwrap(), vec![], "grew again, held once more");
    assert_eq!(
        texts(&mut r),
        [(
            vec![0, 1, 2, 3, 5, 6],
            "[Server Message] maintenance".to_owned()
        )]
    );
}

#[test]
fn a_held_four_tab_run_is_released_when_no_channel_tabs_follow() {
    let g = Game::new();
    let mut r = g.connected(209);
    r.emit_system = true;
    let notice = broadcast("[Server Message] maintenance");
    for (seq, tab) in [(1, 0), (2, 1), (3, 2), (4, 3)] {
        g.put(seq, &notice, tab, BOX);
    }
    assert_eq!(r.read_new().unwrap(), vec![]);
    assert_eq!(texts(&mut r).len(), 1);
    assert_eq!(r.read_new().unwrap(), vec![]);
}

#[test]
fn a_held_tail_is_released_when_nothing_follows() {
    let g = Game::new();
    let mut r = g.connected(206);
    let line = say("Amy", "hi");
    g.put(1, &line, 0, BOX);
    g.put(2, &line, 1, BOX);
    assert_eq!(r.read_new().unwrap(), vec![]);
    assert_eq!(
        texts(&mut r),
        [
            (vec![0], "[Amy] hi".to_owned()),
            (vec![1], "[Amy] hi".to_owned())
        ]
    );
    assert_eq!(r.read_new().unwrap(), vec![]);
}

#[test]
fn a_lone_main_tab_line_is_not_held() {
    let g = Game::new();
    let mut r = g.connected(207);
    g.put(1, &say("Amy", "hi"), 0, BOX);
    assert_eq!(texts(&mut r), [(vec![0], "[Amy] hi".to_owned())]);
}

#[test]
fn unparseable_lines_are_dropped_and_logged() {
    let g = Game::new();
    let mut r = g.connected(208);
    let lines = Lines::default();
    r.message_log = Some(Box::new(lines.clone()));
    let unknown =
        "<color;FFFFFF><image;Art/Art_Unknown_Channel.dds;24;24;FFFFFFFF> [Lars] hi </color>";
    g.put(1, "[DBGM] some debug noise", 0, BOX);
    g.put(2, unknown, 0, BOX);
    assert_eq!(r.read_new().unwrap(), vec![]);
    assert_eq!(
        lines.all(),
        [
            "[seq=1 tab=0 dropped] [DBGM] some debug noise".to_owned(),
            format!("[seq=2 tab=0 dropped] {unknown}")
        ]
    );
}

#[test]
fn only_wizardchatbox_this_is_accepted() {
    let g = Game::new();
    let mut r = g.connected(209);
    let lines = Lines::default();
    r.message_log = Some(Box::new(lines.clone()));
    g.put(1, &say("Amy", "hi"), 0, BOX);
    g.put(2, &say("Bob", "elsewhere"), 0, OTHER);
    g.put(3, &say("Cat", "yo"), 0, BOX);
    assert_eq!(
        texts(&mut r),
        [
            (vec![0], "[Amy] hi".to_owned()),
            (vec![0], "[Cat] yo".to_owned())
        ]
    );
    assert_eq!(lines.count("[seq=2 tab=0 foreign]"), 1);
    assert_eq!(
        logged(
            209,
            &format!(
                "[reader] ignoring AddLine from another chat box (slot=209, pid={}, \
                 this={OTHER:#x}, tab=0, main={BOX:#x})",
                pid()
            )
        )
        .len(),
        1
    );
}

#[test]
fn unknown_this_relooks_chatbox_once_per_round() {
    let g = Game::new();
    let mut r = g.connected(210);
    let lines = Lines::default();
    r.message_log = Some(Box::new(lines.clone()));
    let line = say("Amy", "hi");
    let finds = || g.spy.read_count(EXPORT);
    let start = finds();
    g.put(1, &line, 0, OTHER);
    assert_eq!(r.read_new().unwrap(), vec![]);
    g.set_chatbox(Some(OTHER));
    g.put(2, &line, 0, OTHER);
    g.put(3, &line, 0, OTHER);
    assert_eq!(r.read_new().unwrap(), vec![]);
    assert_eq!(finds() - start, 1, "a known foreign this is not re-looked");
    assert_eq!(lines.count("foreign"), 3);
    assert_eq!(logged(210, "another chat box").len(), 1);

    let g = Game::new();
    let mut r = g.connected(210);
    g.set_chatbox(Some(OTHER));
    g.put(1, &line, 0, OTHER);
    assert_eq!(texts(&mut r), [(vec![0], "[Amy] hi".to_owned())]);
    assert_eq!(
        logged(
            210,
            &format!("[reader] main chat box at {OTHER:#x} (slot=210, ")
        )
        .len(),
        1
    );
}

#[test]
fn missing_chatbox_does_not_mark_the_record_foreign() {
    let g = Game::new();
    g.set_chatbox(None);
    let mut r = g.connected(211);
    let lines = Lines::default();
    r.message_log = Some(Box::new(lines.clone()));
    let line = say("Amy", "hi");
    g.put(1, &line, 0, BOX);
    assert_eq!(r.read_new().unwrap(), vec![]);
    g.set_chatbox(Some(BOX));
    g.put(2, &line, 0, BOX);
    assert_eq!(texts(&mut r), [(vec![0], "[Amy] hi".to_owned())]);
    assert!(lines.all()[0].starts_with("[seq=1 tab=0 no-chatbox]"));
}

#[test]
fn missing_chatbox_is_refound_once_per_poll() {
    let g = Game::new();
    g.set_chatbox(None);
    let mut r = g.connected(212);
    let line = say("Amy", "hi");
    for seq in 1..=3 {
        g.put(seq, &line, 0, BOX);
    }
    let start = g.spy.read_count(EXPORT);
    assert_eq!(r.read_new().unwrap(), vec![]);
    assert_eq!(g.spy.read_count(EXPORT) - start, 1);
    assert_eq!(
        logged(
            212,
            &format!(
                "[reader] AddLine dropped, main chat box not found (slot=212, pid={}, \
                 dropped=3, seq=1-3)",
                pid()
            )
        )
        .len(),
        1
    );
}

#[test]
fn chatbox_lookup_failure_is_logged_and_drops_the_record() {
    let g = Game::new();
    let mut r = g.connected(213);
    put_children(&g.mem, ROOT, &[OTHER]);
    g.put(1, &say("Amy", "hi"), 0, OTHER);
    assert_eq!(r.read_new().unwrap(), vec![]);
    assert_eq!(
        logged(
            213,
            &format!(
                "[reader] chat box lookup failed (slot=213, pid={}): Unable to \
                 read memory at address {}.",
                pid(),
                OTHER + 0x50 + 16
            )
        )
        .len(),
        1
    );
    assert_eq!(logged(213, "main chat box not found").len(), 1);
}

#[test]
fn chatbox_moving_clears_foreign_button_and_input_caches() {
    let g = Game::new();
    put_buttons(&g, &[("MainFilterButton", "Main")], &[]);
    put_edit(&g, BOX, EDIT, false);
    let mut r = g.connected(214);
    g.mem.set_bytes(BOX + CUR_TAB_OFFSET, &0i32.to_le_bytes());
    assert!(r.tab_state().is_some());
    assert!(!r.input_open());
    g.put(1, &say("Amy", "hi"), 0, OTHER);
    assert_eq!(r.read_new().unwrap(), vec![]);
    // 主 ChatBox 重建在先前判定為外來的位址上，按鈕與輸入框也換了一批
    put_children(&g.mem, ROOT, &[OTHER]);
    put_name(&g.mem, OTHER, "WizardChatBox");
    put_edit(&g, OTHER, EDIT + NODE_GAP, true);
    put_buttons_under(&g, OTHER, &[("GroupFilterButton", "Party")]);
    g.mem.set_bytes(OTHER + CUR_TAB_OFFSET, &1i32.to_le_bytes());
    g.clock.advance(Duration::from_secs(6));
    g.put(2, &say("Amy", "again"), 0, OTHER);
    assert_eq!(texts(&mut r), [(vec![0], "[Amy] again".to_owned())]);
    assert_eq!(
        r.tab_state(),
        Some(TabState {
            tabs: vec![(1, "Party".to_owned())],
            current: 1
        })
    );
    assert!(r.input_open(), "the new chat box's input container is read");
}

/// 在 `chatbox` 底下放一個 chatEditContainer。
fn put_edit(g: &Game, chatbox: u64, edit: u64, visible: bool) {
    put_children(&g.mem, chatbox, &[edit]);
    put_name(&g.mem, edit, INPUT_CONTAINER);
    put_children(&g.mem, edit, &[]);
    g.mem
        .set_bytes(edit + 0x9C, &u32::from(visible).to_le_bytes());
}

#[test]
fn foreign_address_is_rechecked_after_the_window() {
    let g = Game::new();
    let mut r = g.connected(215);
    let line = say("Amy", "hi");
    let finds = || g.spy.read_count(EXPORT);
    let start = finds();
    g.put(1, &line, 0, OTHER);
    assert_eq!(r.read_new().unwrap(), vec![]);
    g.set_chatbox(Some(OTHER));
    g.clock.advance(Duration::from_millis(4900));
    g.put(2, &line, 0, OTHER);
    assert_eq!(r.read_new().unwrap(), vec![]);
    assert_eq!(finds() - start, 1, "no re-look inside the window");
    g.clock.advance(Duration::from_millis(200));
    g.put(3, &line, 0, OTHER);
    assert_eq!(texts(&mut r), [(vec![0], "[Amy] hi".to_owned())]);
    assert_eq!(finds() - start, 2);
}

#[test]
fn refinding_the_same_chatbox_is_a_no_op() {
    let g = Game::new();
    put_buttons(&g, &[("MainFilterButton", "主要的")], &[]);
    let mut r = g.connected(216);
    g.mem
        .set_bytes(BOX + CUR_TAB_OFFSET, &0x4141_4141i32.to_le_bytes());
    assert_eq!(r.tab_state(), None);
    g.mem.set_bytes(BOX + CUR_TAB_OFFSET, &0i32.to_le_bytes());
    assert_eq!(
        r.tab_state(),
        Some(TabState {
            tabs: vec![(0, "主要的".to_owned())],
            current: 0
        })
    );
    assert_eq!(logged(216, "main chat box at").len(), 1);
}

#[test]
fn lost_records_are_logged_with_the_count() {
    let g = Game::new();
    let mut r = g.connected(217);
    let lines = Lines::default();
    r.message_log = Some(Box::new(lines.clone()));
    let ring = g.ring();
    for seq in 1..=4 {
        put_record(&g.mem, ring, seq + SLOTS, "overwritten", 0, BOX);
    }
    put_record(&g.mem, ring, 5, &say("Amy", "hi"), 0, BOX);
    g.mem.set_bytes(ring, &5u64.to_le_bytes());
    assert_eq!(r.read_new().unwrap().len(), 1);
    assert_eq!(
        logged(
            217,
            &format!(
                "[reader] lost 4 chat lines (ring overwritten) (slot=217, pid={}, window=1-5)",
                pid()
            )
        )
        .len(),
        1
    );
    assert_eq!(
        lines.all()[0],
        "lost 4 chat lines (ring overwritten) (window=1-5)"
    );
}

#[test]
fn truncated_lines_are_logged_with_the_original_length() {
    let g = Game::new();
    let mut r = g.connected(218);
    let line = say("Amy", "hi");
    g.put(1, &line, 0, BOX);
    g.mem
        .set_bytes(slot_addr(g.ring(), 1) + 0x20, &1500u32.to_le_bytes());
    assert_eq!(r.read_new().unwrap().len(), 1);
    let kept = line.encode_utf16().count();
    assert_eq!(
        logged(
            218,
            &format!(
                "[reader] chat line truncated (slot=218, pid={}, seq=1, tab=0, length=1500, \
                 kept={kept})",
                pid()
            )
        )
        .len(),
        1
    );
}

#[test]
fn non_bmp_text_is_not_reported_as_truncated() {
    let g = Game::new();
    let mut r = g.connected(219);
    g.put(1, &say("Amy", "nice 😂👀"), 0, BOX);
    assert_eq!(texts(&mut r), [(vec![0], "[Amy] nice 😂👀".to_owned())]);
    assert!(logged(219, "truncated").is_empty());
}

// --- 分頁 ---

/// 在 root 底下放 ChatBox 與九個分頁按鈕；未列出文字的按鈕是從未顯示過的分頁。
fn put_buttons(g: &Game, labels: &[(&str, &str)], hidden: &[(&str, bool)]) {
    let mut kids = vec![BOX];
    for (i, name) in TAB_BUTTONS.iter().enumerate() {
        let win = BUTTONS + i as u64 * NODE_GAP;
        let label = labels
            .iter()
            .find(|(n, _)| n == name)
            .map_or("", |(_, l)| l);
        let (button_hidden, container_hidden) = hidden
            .iter()
            .find(|(n, _)| n == name)
            .map_or((false, false), |&(_, container)| (!container, container));
        put_button(
            &g.mem,
            win,
            name,
            label,
            u32::from(!button_hidden),
            u32::from(!container_hidden),
        );
        kids.push(win);
    }
    put_children(&g.mem, ROOT, &kids);
}

/// 把 ChatBox 換到 `chatbox`，並換一批分頁按鈕。
fn put_buttons_under(g: &Game, chatbox: u64, labels: &[(&str, &str)]) {
    let mut kids = vec![chatbox];
    for (i, (name, label)) in labels.iter().enumerate() {
        let win = BUTTONS + 0x1000_0000 + i as u64 * NODE_GAP;
        put_button(&g.mem, win, name, label, 1, 1);
        kids.push(win);
    }
    put_children(&g.mem, ROOT, &kids);
}

fn tab_game(labels: &[(&str, &str)], current: i32) -> Game {
    let g = Game::new();
    put_buttons(&g, labels, &[]);
    g.mem
        .set_bytes(BOX + CUR_TAB_OFFSET, &current.to_le_bytes());
    g
}

#[test]
fn tab_state_reads_current_tab_and_visible_labels() {
    let g = tab_game(
        &[
            ("MainFilterButton", "主要的"),
            ("GroupFilterButton", "队伍(2)"),
        ],
        1,
    );
    let mut r = g.connected(301);
    let expected = Some(TabState {
        tabs: vec![(0, "主要的".to_owned()), (1, "队伍(2)".to_owned())],
        current: 1,
    });
    assert_eq!(r.tab_state(), expected);
    assert_eq!(r.tab_state(), expected);
    let lines = logged(301, "[reader] chat tabs ");
    assert_eq!(
        lines.len(),
        1,
        "an unchanged tab state is logged once: {lines:?}"
    );
    assert!(lines[0].ends_with(&format!(
        "[reader] chat tabs current=1, visible=[(0, '主要的'), (1, '队伍(2)')] (slot=301, pid={})",
        pid()
    )));
}

#[test]
fn tab_hidden_after_being_shown_is_dropped() {
    let labels = [
        ("MainFilterButton", "主要的"),
        ("GroupFilterButton", "队伍(3)"),
    ];
    for container in [false, true] {
        let g = Game::new();
        put_buttons(&g, &labels, &[("GroupFilterButton", container)]);
        g.mem.set_bytes(BOX + CUR_TAB_OFFSET, &0i32.to_le_bytes());
        let mut r = g.connected(302);
        assert_eq!(
            r.tab_state(),
            Some(TabState {
                tabs: vec![(0, "主要的".to_owned())],
                current: 0
            })
        );
    }
}

#[test]
fn tab_state_is_none_without_a_chatbox() {
    let g = Game::new();
    g.set_chatbox(None);
    let mut r = g.connected(303);
    assert_eq!(r.tab_state(), None);
    assert!(logged(303, "chat tabs").is_empty());
}

#[test]
fn repeated_chatbox_lookup_failures_log_once() {
    let g = Game::new();
    let mut r = g.connected(304);
    put_children(&g.mem, ROOT, &[OTHER]);
    assert_eq!(
        [r.tab_state(), r.tab_state(), r.tab_state()],
        [None, None, None]
    );
    let lines = logged(304, "[reader] chat tabs read failed: Unable to read memory");
    assert_eq!(lines.len(), 1, "{lines:?}");
}

#[test]
fn tab_buttons_are_refound_after_the_chatbox_moves() {
    let g = tab_game(&[("MainFilterButton", "Main")], 0);
    let mut r = g.connected(305);
    assert!(r.tab_state().is_some());
    put_name(&g.mem, OTHER, "WizardChatBox");
    put_children(&g.mem, OTHER, &[]);
    put_buttons_under(&g, OTHER, &[("MainFilterButton", "主要的")]);
    g.mem.set_bytes(OTHER + CUR_TAB_OFFSET, &0i32.to_le_bytes());
    g.put(1, &say("Amy", "hi"), 0, OTHER);
    assert_eq!(texts(&mut r), [(vec![0], "[Amy] hi".to_owned())]);
    assert_eq!(
        r.tab_state(),
        Some(TabState {
            tabs: vec![(0, "主要的".to_owned())],
            current: 0
        })
    );
}

#[test]
fn no_visible_tab_buttons_are_not_rescanned_every_poll() {
    let g = Game::new();
    g.mem.set_bytes(BOX + CUR_TAB_OFFSET, &0i32.to_le_bytes());
    let mut r = g.connected(306);
    let empty = Some(TabState {
        tabs: vec![],
        current: 0,
    });
    assert_eq!(r.tab_state(), empty);
    let scans = g.spy.read_count(EXPORT);
    assert_eq!(r.tab_state(), empty);
    assert_eq!(g.spy.read_count(EXPORT), scans);
}

#[test]
fn out_of_range_current_tab_is_a_read_failure() {
    let g = tab_game(&[("MainFilterButton", "主要的")], 0x4141_4141);
    let mut r = g.connected(307);
    assert_eq!(r.tab_state(), None);
    let failed = logged(
        307,
        &format!(
            "[reader] chat tabs read failed: current tab 1094795585 out of range \
             (stale chat box {BOX:#x}?) (slot=307, pid={})",
            pid()
        ),
    );
    assert_eq!(failed.len(), 1);
    // 按鈕快取已丟掉：下一輪重找 ChatBox 與按鈕
    let scans = g.spy.read_count(EXPORT);
    assert_eq!(r.tab_state(), None);
    assert!(g.spy.read_count(EXPORT) > scans);
}

#[test]
fn tab_queries_are_empty_when_not_connected() {
    let g = Game::new();
    let mut r = g.reader(308);
    assert_eq!(r.tab_state(), None);
    assert_eq!(r.current_tab(), None);
    assert!(!r.input_open());
    assert_eq!(r.input_box_screen_rect(), None);
    assert!(!r.anchored());
    assert!(g.mem.ops().is_empty());
}

#[test]
fn current_tab_reads_only_the_known_chatbox() {
    let g = tab_game(&[("MainFilterButton", "主要的")], 1);
    let mut r = g.connected(309);
    assert_eq!(r.current_tab(), None, "no chat box known yet");
    assert!(r.tab_state().is_some());
    assert_eq!(r.current_tab(), Some(1));
    g.mem
        .set_bytes(BOX + CUR_TAB_OFFSET, &0x4141_4141i32.to_le_bytes());
    assert_eq!(r.current_tab(), None);
}

// --- 輸入框 ---

fn put_rect(g: &Game, win: u64, rect: [i32; 4]) {
    let bytes: Vec<u8> = rect.iter().flat_map(|v| v.to_le_bytes()).collect();
    g.mem.set_bytes(win + 0xA0, &bytes);
}

/// 實機探測的容器鏈：chatEditContainer → chatContainer → WizardChatBox → WorldView → Root。
fn input_game(root_rect: [i32; 4]) -> Game {
    let g = Game::new();
    put_children(&g.mem, ROOT, &[BOX, EDIT]);
    put_name(&g.mem, EDIT, INPUT_CONTAINER);
    put_children(&g.mem, EDIT, &[]);
    g.mem.set_bytes(EDIT + 0x9C, &1u32.to_le_bytes());
    put_rect(&g, EDIT, [0, 190, 523, 222]);
    put_rect(&g, ROOT, root_rect);
    let parents = [
        [0, 0, 523, 222],
        [23, 300, 562, 522],
        [0, 0, 1370, 770],
        [0, 0, 1370, 770],
    ];
    let mut child = EDIT;
    for (i, rect) in parents.iter().enumerate() {
        let parent = 0x3800_0000 + i as u64 * NODE_GAP;
        g.mem.set_bytes(child + 0x88, &parent.to_le_bytes());
        put_rect(&g, parent, *rect);
        child = parent;
    }
    g.mem.set_bytes(child + 0x88, &0u64.to_le_bytes());
    g
}

#[test]
fn input_open_reads_the_container_visibility() {
    let g = input_game([0, 0, 1370, 770]);
    let mut r = g.connected(401);
    assert!(r.input_open());
    let scans = g.spy.read_count(EXPORT);
    g.mem.set_bytes(EDIT + 0x9C, &0u32.to_le_bytes());
    assert!(!r.input_open());
    assert_eq!(g.spy.read_count(EXPORT), scans, "node is cached");
}

#[test]
fn missing_input_container_is_rechecked_after_the_window() {
    let g = Game::new();
    let mut r = g.connected(406);
    assert!(!r.input_open());
    let scans = g.spy.read_count(EXPORT);
    put_edit(&g, BOX, EDIT, true);
    assert!(!r.input_open());
    assert_eq!(r.input_box_screen_rect(), None);
    assert_eq!(
        g.spy.read_count(EXPORT),
        scans,
        "no re-search inside the window"
    );
    g.clock.advance(INPUT_RECHECK);
    assert!(r.input_open());
    assert_eq!(g.spy.read_count(EXPORT), scans + 1);
}

#[test]
fn failing_input_container_searches_are_throttled_too() {
    let g = Game::new();
    let mut r = g.connected(407);
    put_children(&g.mem, ROOT, &[OTHER]);
    assert!(!r.input_open());
    let scans = g.spy.read_count(EXPORT);
    assert!(!r.input_open());
    assert_eq!(
        g.spy.read_count(EXPORT),
        scans,
        "no re-search inside the window"
    );
    g.clock.advance(INPUT_RECHECK);
    assert!(!r.input_open());
    assert_eq!(g.spy.read_count(EXPORT), scans + 1);
}

#[test]
fn input_box_screen_rect_walks_parents_and_offsets_by_window_origin() {
    let g = input_game([0, 0, 1370, 770]);
    let mut r = g.connected(402);
    let client = client_size(hwnd()).unwrap();
    let origin = client_to_screen(hwnd(), 0, 0).unwrap();
    let offsets = [(0, 190), (0, 0), (23, 300), (0, 0), (0, 0)];
    let (x, y, w, h) = client_rect(&offsets, (524, 33), (1370, 770), client).unwrap();
    let expected = Some((origin.0 + x, origin.1 + y, w, h));
    assert_eq!(r.input_box_screen_rect(), expected);
    let scans = g.spy.read_count(EXPORT);
    assert_eq!(r.input_box_screen_rect(), expected);
    assert!(r.input_open());
    // 每次讀 root 矩形各一次，節點本身沿用快取不重找
    assert_eq!(g.spy.read_count(EXPORT), scans + 1);
}

#[test]
fn input_box_screen_rect_none_when_node_read_fails() {
    let g = input_game([0, 0, 1370, 770]);
    let mut r = g.connected(403);
    assert!(r.input_box_screen_rect().is_some());
    g.mem.fail_reads_at(EDIT + 0xA0);
    assert_eq!(r.input_box_screen_rect(), None);
    assert_eq!(
        logged(
            403,
            &format!(
                "[reader] game chat input rect unavailable (slot=403): Unable \
                 to read memory at address {}.",
                EDIT + 0xA0
            )
        )
        .len(),
        1
    );
    // 失效節點丟掉，下一輪重找
    let scans = g.spy.read_count(EXPORT);
    assert!(r.input_open());
    assert!(g.spy.read_count(EXPORT) > scans);
}

#[test]
fn input_box_screen_rect_none_when_the_parent_chain_cycles() {
    let g = input_game([0, 0, 1370, 770]);
    g.mem.set_bytes(EDIT + 0x88, &EDIT.to_le_bytes());
    let mut r = g.connected(405);
    assert_eq!(r.input_box_screen_rect(), None);
    assert_eq!(
        logged(
            405,
            "[reader] game chat input rect unavailable (slot=405): window parent chain \
             deeper than 64"
        )
        .len(),
        1
    );
}

#[test]
fn input_box_screen_rect_none_for_a_degenerate_root() {
    let g = input_game([0, 0, 0, 770]);
    let mut r = g.connected(404);
    assert_eq!(r.input_box_screen_rect(), None);
    assert_eq!(
        logged(
            404,
            "[reader] game chat input rect unavailable (slot=404): degenerate root \
             window size (0, 770)"
        )
        .len(),
        1
    );
    let scans = g.spy.read_count(EXPORT);
    r.input_open();
    assert!(g.spy.read_count(EXPORT) > scans, "node re-searched");
}
