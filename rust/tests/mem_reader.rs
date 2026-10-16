//! WizChatReader：掛入順序、失敗分類與還原、事件過濾、分頁狀態與輸入框（以 FakeMemory 模擬遊戲）。

#[allow(dead_code)]
mod common;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use common::fake_game::{place, put_button, put_children, put_name};
use windows_sys::Win32::UI::WindowsAndMessaging::GetDesktopWindow;
use wizard101_chat_translator::logs::{self, Sink};
use wizard101_chat_translator::reader::chat_hook::{self, DATA_OFF, HEADER, SLOT_SIZE, SLOTS};
use wizard101_chat_translator::reader::chat_tabs::{CUR_TAB_OFFSET, TabState};
use wizard101_chat_translator::reader::hook_state;
use wizard101_chat_translator::reader::hooks::{AUTOBOT_PATTERN, MODULE, ROOT_WINDOW_PATTERN};
use wizard101_chat_translator::reader::mem_reader::{
    Clock, GameNotRunning, HOOK_READY_POLL, HOOK_READY_TIMEOUT, INPUT_CONTAINER, MessageSink,
    WizChatReader,
};
use wizard101_chat_translator::reader::memory::{
    FakeMemory, MemError, MemOp, ProcessMemory, Region,
};
use wizard101_chat_translator::reader::process::{
    client_size, client_to_screen, install_path_of, install_root, window_exists, window_pid,
};
use wizard101_chat_translator::reader::ui_rect::client_rect;

const BASE: u64 = 0x1_4000_0000;
const CAVE: u64 = 0x1_4000_1000;
const ROOT_JUMP: u64 = 0x1_4080_0000;
const ADDLINE_JUMP: u64 = 0x1_4120_0000;
/// FakeMemory 的第一次配置（root window export）與下一次（環形緩衝）。
const EXPORT: u64 = 0x2000_0000;
const RING: u64 = 0x2001_0000;
const RING_SIZE: usize = 0x80040;
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

/// 整個測試檔共用一個 log 檔（全域 sink 只有一個）；各測試以自己的 slot 篩選。
fn log_lines_with(marker: &str) -> Vec<String> {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    let path = PATH.get_or_init(|| {
        let path = std::env::temp_dir().join(format!("mem_reader_{}.log", std::process::id()));
        logs::init(Sink::File(std::fs::File::create(&path).unwrap()));
        path
    });
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|l| l.contains(marker))
        .map(str::to_owned)
        .collect()
}

fn logged(slot: u32, needle: &str) -> Vec<String> {
    log_lines_with(&format!("slot={slot}"))
        .into_iter()
        .filter(|l| l.contains(needle))
        .collect()
}

/// 交給 reader 的記憶體：與測試共用同一份 FakeMemory，並可記錄／弄壞 export 讀取與寫入。
struct Shared {
    mem: Arc<FakeMemory>,
    export_reads: Arc<AtomicUsize>,
    failing_export_reads: Arc<AtomicUsize>,
    fail_writes: Arc<AtomicBool>,
}

impl ProcessMemory for Shared {
    fn read(&self, addr: u64, len: usize) -> Result<Vec<u8>, MemError> {
        if addr == EXPORT {
            self.export_reads.fetch_add(1, Ordering::SeqCst);
            let failing = &self.failing_export_reads;
            if failing
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                .is_ok()
            {
                return Err(MemError::Read(addr));
            }
        }
        self.mem.read(addr, len)
    }

    fn write(&self, addr: u64, bytes: &[u8]) -> Result<(), MemError> {
        if self.fail_writes.load(Ordering::SeqCst) {
            return Err(MemError::Write(addr));
        }
        self.mem.write(addr, bytes)
    }

    fn alloc(&self, size: usize) -> Result<u64, MemError> {
        self.mem.alloc(size)
    }

    fn free(&self, addr: u64) {
        self.mem.free(addr)
    }

    fn module_base(&self, name: &str) -> Result<u64, MemError> {
        self.mem.module_base(name)
    }

    fn regions(&self, module: &str) -> Result<Vec<Region>, MemError> {
        self.mem.regions(module)
    }

    fn is_alive(&self) -> bool {
        self.mem.is_alive()
    }
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

struct Game {
    mem: Arc<FakeMemory>,
    dir: tempfile::TempDir,
    /// 假時鐘從建立起經過的時間；sleep 只推進它。
    elapsed: Arc<Mutex<Duration>>,
    /// 等待中 root window hook 是否會「被執行」（把 root 位址寫進 export）。
    fires: Arc<AtomicBool>,
    export_reads: Arc<AtomicUsize>,
    failing_export_reads: Arc<AtomicUsize>,
    fail_writes: Arc<AtomicBool>,
    /// 第一次等待時 hookstate 檔的內容。
    state_at_wait: Arc<Mutex<Option<hook_state::RestoreOps>>>,
    opened: Arc<Mutex<Vec<u32>>>,
    root_bytes: Vec<u8>,
    addline_bytes: Vec<u8>,
    cave_bytes: Vec<u8>,
}

impl Game {
    fn new() -> Self {
        Self::with(true, true)
    }

    fn with(root_pattern: bool, addline_pattern: bool) -> Self {
        log_lines_with("");
        let mem = FakeMemory::new();
        mem.add_module(MODULE, BASE);
        let cave_bytes = place(&mem, CAVE, AUTOBOT_PATTERN, 0x1100);
        let root_bytes = if root_pattern {
            place(&mem, ROOT_JUMP, ROOT_WINDOW_PATTERN, 0x40)
        } else {
            Vec::new()
        };
        let addline_bytes = if addline_pattern {
            place(&mem, ADDLINE_JUMP, chat_hook::PATTERN, 0x80)
        } else {
            Vec::new()
        };
        put_name(&mem, ROOT, "WorldView");
        put_children(&mem, ROOT, &[BOX]);
        put_name(&mem, BOX, "WizardChatBox");
        put_children(&mem, BOX, &[]);
        Self {
            mem: Arc::new(mem),
            dir: tempfile::tempdir().unwrap(),
            elapsed: Arc::default(),
            fires: Arc::new(AtomicBool::new(true)),
            export_reads: Arc::default(),
            failing_export_reads: Arc::default(),
            fail_writes: Arc::default(),
            state_at_wait: Arc::default(),
            opened: Arc::default(),
            root_bytes,
            addline_bytes,
            cave_bytes,
        }
    }

    fn reader(&self, slot: u32) -> WizChatReader {
        let (mem, export_reads, failing_export_reads, fail_writes, opened) = (
            self.mem.clone(),
            self.export_reads.clone(),
            self.failing_export_reads.clone(),
            self.fail_writes.clone(),
            self.opened.clone(),
        );
        let mut r = WizChatReader::new(
            hwnd(),
            Box::new(move |pid| {
                opened.lock().unwrap().push(pid);
                Ok(Box::new(Shared {
                    mem: mem.clone(),
                    export_reads: export_reads.clone(),
                    failing_export_reads: failing_export_reads.clone(),
                    fail_writes: fail_writes.clone(),
                }) as Box<dyn ProcessMemory>)
            }),
        );
        r.slot = slot;
        r.state_dir = self.dir.path().to_owned();
        let start = Instant::now();
        let elapsed = self.elapsed.clone();
        r.clock = Clock {
            now: Box::new(move || start + *elapsed.lock().unwrap()),
            sleep: self.on_sleep(),
        };
        r
    }

    /// 假 sleep：推進假時鐘；root window hook 會被執行時把 root 位址寫進目前的 export。
    fn on_sleep(&self) -> Box<dyn Fn(Duration) + Send> {
        let (mem, elapsed, fires, seen, dir) = (
            self.mem.clone(),
            self.elapsed.clone(),
            self.fires.clone(),
            self.state_at_wait.clone(),
            self.dir.path().to_owned(),
        );
        Box::new(move |d| {
            *elapsed.lock().unwrap() += d;
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

    fn advance(&self, d: Duration) {
        *self.elapsed.lock().unwrap() += d;
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
        let mut ops = vec![
            MemOp::Write {
                addr: ROOT_JUMP,
                bytes: self.root_bytes[..7].to_vec(),
            },
            MemOp::Free(EXPORT),
        ];
        if addline {
            ops.push(MemOp::Write {
                addr: ADDLINE_JUMP,
                bytes: self.addline_bytes[..5].to_vec(),
            });
        }
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

fn slot_addr(ring: u64, seq: u64) -> u64 {
    ring + HEADER + ((seq - 1) % SLOTS) * SLOT_SIZE
}

/// 照 stub 的格式寫一筆記錄並把累計數推到至少 `seq`。
fn put_record(mem: &FakeMemory, ring: u64, seq: u64, text: &str, tab: u32, this: u64) {
    let units: Vec<u16> = text.encode_utf16().collect();
    let mut head = Vec::new();
    head.extend(seq.to_le_bytes());
    head.extend(0x1407443B1u64.to_le_bytes());
    head.extend(this.to_le_bytes());
    head.extend(tab.to_le_bytes());
    head.extend((units.len() as u32).to_le_bytes());
    head.extend((units.len() as u32).to_le_bytes());
    head.resize(DATA_OFF as usize, 0);
    let at = slot_addr(ring, seq);
    mem.set_bytes(at, &head);
    let data: Vec<u8> = units.iter().flat_map(|u| u.to_le_bytes()).collect();
    mem.set_bytes(at + DATA_OFF, &data);
    let count = mem.read_u64(ring).unwrap().max(seq);
    mem.set_bytes(ring, &count.to_le_bytes());
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
        format!("[reader] game path: None (slot=101, pid={p})"),
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
fn connect_waits_through_transient_read_errors() {
    let g = Game::new();
    g.failing_export_reads.store(3, Ordering::SeqCst);
    let _r = g.connected(102);
    assert!(*g.elapsed.lock().unwrap() >= HOOK_READY_POLL);
}

#[test]
fn open_failure_is_access_denied() {
    log_lines_with("");
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
        failed[0].contains("): PatternFailed: Pattern b'")
            && failed[0].ends_with(
                "; a pattern failure here needs chat_hook.PATTERN updated, not the wizwalker fork"
            ),
        "{failed:?}"
    );

    let g = Game::new();
    place(&g.mem, ROOT_JUMP + 0x1000, ROOT_WINDOW_PATTERN, 0x40);
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
fn readiness_timeout_is_version_mismatch() {
    let g = Game::new();
    g.fires.store(false, Ordering::SeqCst);
    let mut r = g.reader(104);
    assert_eq!(
        r.read_new().unwrap_err(),
        GameNotRunning::VersionMismatch(
            "hook does not match this game build: root window hook did not fire within 15s".into()
        )
    );
    let waited = *g.elapsed.lock().unwrap();
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
fn other_attach_failure_stays_game_not_running() {
    log_lines_with("");
    let mut r = WizChatReader::new(
        hwnd(),
        Box::new(|_| Ok(Box::new(FakeMemory::new()) as Box<dyn ProcessMemory>)),
    );
    r.state_dir = tempfile::tempdir().unwrap().path().to_owned();
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
    g.fail_writes.store(true, Ordering::SeqCst);
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
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].contains(&format!("(slot=109, pid={p}, ops=2): OSError: ")));
    assert!(lines[1].contains(&format!("(slot=109, pid={p}, ops=3): OSError: ")));
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
    let (hi, yo) = (say("Amy", "hi"), own("yo"));
    g.put(1, &hi, 0, BOX);
    g.put(2, &yo, 1, BOX);
    let events = r.read_new().unwrap();
    let got: Vec<_> = events
        .iter()
        .map(|e| (e.tabs.clone(), e.line.text.as_str(), e.seq, e.raw.as_str()))
        .collect();
    assert_eq!(
        got,
        [
            (vec![0], "[Amy] hi", 1, hi.as_str()),
            (vec![1], "[你] yo", 2, yo.as_str())
        ]
    );
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
    assert_eq!(
        texts(&mut r),
        [(vec![0, 1, 2, 3], "[Server Message] maintenance".to_owned())]
    );
    assert_eq!(lines.count("[seq=2 "), 1, "held records are logged once");
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
    let finds = || g.export_reads.load(Ordering::SeqCst);
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
    let start = g.export_reads.load(Ordering::SeqCst);
    assert_eq!(r.read_new().unwrap(), vec![]);
    assert_eq!(g.export_reads.load(Ordering::SeqCst) - start, 1);
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
                "[reader] chat box lookup failed (slot=213, pid={}): MemoryReadError: Unable to \
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
fn chatbox_moving_clears_foreign_and_button_caches() {
    let g = Game::new();
    put_buttons(&g, &[("MainFilterButton", "Main")], &[]);
    let mut r = g.connected(214);
    g.mem.set_bytes(BOX + CUR_TAB_OFFSET, &0i32.to_le_bytes());
    assert!(r.tab_state().is_some());
    g.put(1, &say("Amy", "hi"), 0, OTHER);
    assert_eq!(r.read_new().unwrap(), vec![]);
    // 主 ChatBox 重建在先前判定為外來的位址上，按鈕也換了一批
    put_children(&g.mem, ROOT, &[OTHER]);
    put_name(&g.mem, OTHER, "WizardChatBox");
    put_children(&g.mem, OTHER, &[]);
    put_buttons_under(&g, OTHER, &[("GroupFilterButton", "Party")]);
    g.mem.set_bytes(OTHER + CUR_TAB_OFFSET, &1i32.to_le_bytes());
    g.advance(Duration::from_secs(6));
    g.put(2, &say("Amy", "again"), 0, OTHER);
    assert_eq!(texts(&mut r), [(vec![0], "[Amy] again".to_owned())]);
    assert_eq!(
        r.tab_state(),
        Some(TabState {
            tabs: vec![(1, "Party".to_owned())],
            current: 1
        })
    );
}

#[test]
fn foreign_address_is_rechecked_after_the_window() {
    let g = Game::new();
    let mut r = g.connected(215);
    let line = say("Amy", "hi");
    let finds = || g.export_reads.load(Ordering::SeqCst);
    let start = finds();
    g.put(1, &line, 0, OTHER);
    assert_eq!(r.read_new().unwrap(), vec![]);
    g.set_chatbox(Some(OTHER));
    g.advance(Duration::from_millis(4900));
    g.put(2, &line, 0, OTHER);
    assert_eq!(r.read_new().unwrap(), vec![]);
    assert_eq!(finds() - start, 1, "no re-look inside the window");
    g.advance(Duration::from_millis(200));
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
    let lines = logged(304, "[reader] chat tabs read failed: MemoryReadError: ");
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
    let scans = g.export_reads.load(Ordering::SeqCst);
    assert_eq!(r.tab_state(), empty);
    assert_eq!(g.export_reads.load(Ordering::SeqCst), scans);
}

#[test]
fn out_of_range_current_tab_is_a_read_failure() {
    let g = tab_game(&[("MainFilterButton", "主要的")], 0x4141_4141);
    let mut r = g.connected(307);
    assert_eq!(r.tab_state(), None);
    let failed = logged(
        307,
        &format!(
            "[reader] chat tabs read failed: ValueError: current tab 1094795585 out of range \
             (stale chat box {BOX:#x}?) (slot=307, pid={})",
            pid()
        ),
    );
    assert_eq!(failed.len(), 1);
    // 按鈕快取已丟掉：下一輪重找 ChatBox 與按鈕
    let scans = g.export_reads.load(Ordering::SeqCst);
    assert_eq!(r.tab_state(), None);
    assert!(g.export_reads.load(Ordering::SeqCst) > scans);
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
    let scans = g.export_reads.load(Ordering::SeqCst);
    g.mem.set_bytes(EDIT + 0x9C, &0u32.to_le_bytes());
    assert!(!r.input_open());
    assert_eq!(
        g.export_reads.load(Ordering::SeqCst),
        scans,
        "node is cached"
    );
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
    let scans = g.export_reads.load(Ordering::SeqCst);
    assert_eq!(r.input_box_screen_rect(), expected);
    assert!(r.input_open());
    // 每次讀 root 矩形各一次，節點本身沿用快取不重找
    assert_eq!(g.export_reads.load(Ordering::SeqCst), scans + 1);
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
                "[reader] game chat input rect unavailable (slot=403): MemoryReadError: Unable \
                 to read memory at address {}.",
                EDIT + 0xA0
            )
        )
        .len(),
        1
    );
    // 失效節點丟掉，下一輪重找
    let scans = g.export_reads.load(Ordering::SeqCst);
    assert!(r.input_open());
    assert!(g.export_reads.load(Ordering::SeqCst) > scans);
}

#[test]
fn input_box_screen_rect_none_for_a_degenerate_root() {
    let g = input_game([0, 0, 0, 770]);
    let mut r = g.connected(404);
    assert_eq!(r.input_box_screen_rect(), None);
    assert_eq!(
        logged(
            404,
            "[reader] game chat input rect unavailable (slot=404): ValueError: degenerate root \
             window size (0, 770)"
        )
        .len(),
        1
    );
    let scans = g.export_reads.load(Ordering::SeqCst);
    r.input_open();
    assert!(
        g.export_reads.load(Ordering::SeqCst) > scans,
        "node re-searched"
    );
}
