//! 收訊端：掛入一個遊戲客戶端（autobot cave、root window 與 AddLine hook），依序讀出新增的聊天事件與分頁狀態。
//!
//! hook 會寫入遊戲程序記憶體：連線失敗、`close` 與 drop 都經由 cave 解除並還原遊戲，
//! 解除不了就留下 hookstate 檔，下次啟動修復。

use std::collections::HashSet;
use std::error::Error;
use std::fmt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::chat_hook::{
    AddLineHook, HEADER, HookRecord, SLOT_SIZE, SLOTS, group_broadcasts, pending_broadcast,
    read_ring,
};
use super::chat_tabs::{
    CHATBOX_WINDOW, TabState, find_tab_buttons, read_current_tab, read_tab_state,
};
use super::hook_state;
use super::hooks::{AutobotCave, HookError, MODULE, RootWindowHook};
use super::markup::{ChatLine, forget_warned_icons, parse_line};
use super::memory::{MemError, ProcessMemory};
use super::pattern::ScanError;
use super::process;
use super::ui_rect::client_rect;
use super::ui_tree::{FLAG_VISIBLE, Window, find_by_name};
use crate::config::local_state_dir;
use crate::logs::log;
use crate::py::pyrepr::repr_str;
use crate::util::Clock;

/// 螢幕矩形 `(x, y, w, h)`。
type ScreenRect = (i32, i32, i32, i32);

/// 遊戲聊天輸入區容器：開啟輸入時帶 VISIBLE 旗標（實測）。
pub const INPUT_CONTAINER: &str = "chatEditContainer";
/// 輸入框往上找父節點的層數上限（實機只有 4 層）。
const MAX_PARENT_DEPTH: usize = 64;
/// exe 的偏好載入位址：log 把跳轉點換算回靜態位址，才對得上反組譯。
const IMAGE_BASE: u64 = 0x1_4000_0000;
/// 等 root window hook 第一次被執行的上限；逾時視為 hook 與遊戲版本對不上。
pub const HOOK_READY_TIMEOUT: Duration = Duration::from_secs(15);
/// 等 root window hook 時的輪詢間隔。
pub const HOOK_READY_POLL: Duration = Duration::from_millis(300);
/// 外來 ChatBox 位址的重驗間隔（秒）：新的主 ChatBox 可能重用外來的舊位址。
pub const FOREIGN_RECHECK_SECONDS: u64 = 5;

/// 一則新聊天；`tabs` 是它所在的分頁，伺服器廣播收斂後含多個。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatEvent {
    /// 解析後的聊天內容。
    pub line: ChatLine,
    /// 所在的分頁編號。
    pub tabs: Vec<i32>,
}

/// 連不上或掛不進遊戲；三種只在文案上分流，上層的退避重連照舊。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameNotRunning {
    /// 找不到遊戲、程序消失或讀寫失敗。
    Missing(String),
    /// 開不了程序 handle（多半是遊戲以系統管理員身分執行、本程式沒有）。
    AccessDenied(String),
    /// 掛入點與這個遊戲版本對不上（pattern 掃不到、多重命中或 hook 遲遲沒被執行）。
    VersionMismatch(String),
}

impl fmt::Display for GameNotRunning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(msg) | Self::AccessDenied(msg) | Self::VersionMismatch(msg) => {
                f.write_str(msg)
            }
        }
    }
}

impl std::error::Error for GameNotRunning {}

/// messages.log 的寫入端：每筆 hook 記錄的判定與不屬於單筆記錄的事件。
pub trait MessageSink: Send {
    /// 記一筆 hook 記錄：序號、分頁、原始 markup 與判定。
    fn record(&mut self, seq: u64, tab: u32, raw: &str, verdict: &str);
    /// 記一則事件（遺失、截斷等）。
    fn note(&mut self, text: &str);
}

/// 依 PID 開啟遊戲程序記憶體（實機用 `WinProcess::open`，測試注入 `FakeMemory`）。
pub type Opener = Box<dyn Fn(u32) -> Result<Box<dyn ProcessMemory>, MemError> + Send>;

/// 一次連線：程序記憶體與掛上的 hook（cave 只在掛入後才有）。
struct Attached {
    mem: Box<dyn ProcessMemory>,
    cave: Option<AutobotCave>,
    export: u64,
    ring: u64,
}

enum AttachError {
    Hook(HookError),
    Timeout,
}

impl From<HookError> for AttachError {
    fn from(err: HookError) -> Self {
        Self::Hook(err)
    }
}

impl fmt::Display for AttachError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Hook(err) => err.fmt(f),
            Self::Timeout => write!(
                f,
                "root window hook did not fire within {}s",
                HOOK_READY_TIMEOUT.as_secs()
            ),
        }
    }
}

impl AttachError {
    /// pattern 掃不到、多重命中（都在寫入遊戲之前）或等待逾時。
    fn is_version_mismatch(&self) -> bool {
        matches!(
            self,
            Self::Timeout
                | Self::Hook(HookError::Scan(
                    ScanError::NotFound(_) | ScanError::Multiple(..)
                ))
        )
    }
}

/// 記下失敗原因；與上次相同回 false（呼叫端就不再寫 log）。
fn remember(last: &mut Option<String>, reason: &str) -> bool {
    if last.as_deref() == Some(reason) {
        return false;
    }
    *last = Some(reason.to_owned());
    true
}

/// 掛入一個遊戲客戶端，回傳每輪新增的聊天事件與遊戲聊天框的分頁狀態。
///
/// hook 只看得到掛上之後新增的行，不回補歷史；序號屬於單一連線，重連從 0 開始。
pub struct WizChatReader {
    /// 是否輸出系統訊息；關閉時序號照推，打開不會倒出舊訊息。
    pub emit_system: bool,
    /// 只用在 log：多開時分辨是哪個客戶端。
    pub slot: u32,
    /// messages.log（None＝不記錄）。
    pub message_log: Option<Box<dyn MessageSink>>,
    /// hookstate 檔所在目錄。
    pub state_dir: PathBuf,
    pub clock: Clock,
    hwnd: isize,
    open: Opener,
    attached: Option<Attached>,
    pid: u32,
    /// 已記過安裝路徑的 PID：重試連線時不重複寫。
    path_logged_for: u32,
    /// 上次記下的掛入失敗原因：同一原因的重試不重複寫 log。
    last_attach_failure: Option<String>,
    edit_node: Option<Window>,
    last_seq: u64,
    chatbox: Option<u64>,
    buttons: Option<Vec<(i32, Window)>>,
    foreign: HashSet<u64>,
    foreign_checked: Option<Instant>,
    tabs_logged: Option<Result<TabState, String>>,
    /// 上一輪暫留的廣播（起始 seq, 筆數）；同一段沒有變長就不再暫留。
    held: (u64, usize),
    refound: bool,
    no_chatbox: Vec<u64>,
}

impl Drop for WizChatReader {
    fn drop(&mut self) {
        // panic 或忘了 close 也要把遊戲還原
        if self.attached.is_some() {
            self.teardown();
        }
    }
}

impl WizChatReader {
    /// 綁定指定的遊戲視窗；第一次 `read_new` 時才連線。
    pub fn new(hwnd: isize, open: Opener) -> Self {
        Self {
            emit_system: false,
            slot: 0,
            message_log: None,
            state_dir: local_state_dir(),
            clock: Clock::default(),
            hwnd,
            open,
            attached: None,
            pid: 0,
            path_logged_for: 0,
            last_attach_failure: None,
            edit_node: None,
            last_seq: 0,
            chatbox: None,
            buttons: None,
            foreign: HashSet::new(),
            foreign_checked: None,
            tabs_logged: None,
            held: (0, 0),
            refound: false,
            no_chatbox: Vec::new(),
        }
    }

    /// 是否已連上並掛入遊戲（未連上時上層顯示「定位中」）。
    pub fn anchored(&self) -> bool {
        self.attached.is_some()
    }

    fn conn(&self) -> &Attached {
        self.attached.as_ref().expect("reader is attached")
    }

    fn root(&self) -> Result<Window, MemError> {
        let conn = self.conn();
        Ok(Window {
            addr: conn.mem.read_u64(conn.export)?,
        })
    }

    /// 自上次呼叫後新增的聊天事件（依序、含重複）；未連線時先連線。
    pub fn read_new(&mut self) -> Result<Vec<ChatEvent>, GameNotRunning> {
        if self.attached.is_none() {
            self.connect()?;
        }
        let conn = self.conn();
        let got = match read_ring(conn.mem.as_ref(), conn.ring, self.last_seq) {
            Ok(got) => got,
            Err(err) => {
                self.teardown();
                return Err(GameNotRunning::Missing(format!(
                    "chat ring read failed (likely left the game): {err}"
                )));
            }
        };
        let first = self.last_seq + 1;
        self.last_seq = got.last_seq;
        if got.lost > 0 {
            self.note(
                &format!("lost {} chat lines (ring overwritten)", got.lost),
                &format!("window={first}-{}", got.last_seq),
            );
        }
        let mut records = got.records;
        let held = pending_broadcast(&records);
        if held > 0 {
            let head = records[records.len() - held].seq;
            if records.last().map(|r| r.seq) == Some(got.last_seq)
                && (head, held) != self.held
                && (head != self.held.0 || held > self.held.1)
            {
                // 廣播可能被這一輪切開：游標停在開頭之前，下一輪連同後段一起收斂
                self.held = (head, held);
                self.last_seq = head - 1;
                records.truncate(records.len() - held);
            }
        }
        self.refound = false;
        self.no_chatbox.clear();
        let mut events = Vec::new();
        for (tabs, rec) in group_broadcasts(&records) {
            if let Some(line) = self.judge(&tabs, &rec) {
                events.push(ChatEvent {
                    line,
                    tabs: tabs.iter().map(|&t| t as i32).collect(),
                });
            }
        }
        if let (Some(first), Some(last)) = (self.no_chatbox.first(), self.no_chatbox.last()) {
            log(format!(
                "[reader] AddLine dropped, main chat box not found (slot={}, pid={}, \
                 dropped={}, seq={first}-{last})",
                self.slot,
                self.pid,
                self.no_chatbox.len()
            ));
        }
        Ok(events)
    }

    /// 判定一組記錄（廣播收斂後的代表記錄）並寫進 messages.log；要回傳的回 ChatLine。
    fn judge(&mut self, tabs: &[u32], rec: &HookRecord) -> Option<ChatLine> {
        if rec.copied < rec.length {
            self.note(
                "chat line truncated",
                &format!(
                    "seq={}, tab={}, length={}, kept={}",
                    rec.seq, rec.tab, rec.length, rec.copied
                ),
            );
        }
        let mut line = None;
        let verdict = match self.chatbox_verdict(rec) {
            Some(rejected) => rejected,
            None => match parse_line(&rec.text) {
                None => "dropped",
                Some(parsed) if parsed.system && !self.emit_system => "system-off",
                Some(parsed) => {
                    let verdict = if tabs.len() > 1 {
                        "broadcast"
                    } else if parsed.system {
                        "system"
                    } else {
                        "player"
                    };
                    line = Some(parsed);
                    verdict
                }
            },
        };
        self.record(rec.seq, rec.tab, &rec.text, verdict);
        if tabs.len() > 1 {
            let shown: Vec<String> = tabs.iter().map(u32::to_string).collect();
            log(format!(
                "[reader] server broadcast collapsed across {} tabs (slot={}, pid={}, seq={}, \
                 tabs=({}))",
                tabs.len(),
                self.slot,
                self.pid,
                rec.seq,
                shown.join(", ")
            ));
            // 廣播是連號寫入，其餘各筆的序號與分頁可由代表記錄還原
            for (i, &tab) in tabs.iter().enumerate().skip(1) {
                self.record(rec.seq + i as u64, tab, &rec.text, "broadcast-dup");
            }
        }
        line
    }

    /// `this` 是主 ChatBox 回 None（照收）；否則回 `foreign` 或 `no-chatbox`。
    ///
    /// 不認得的 `this` 先重找 ChatBox（登出入後 ChatBox 可能是新物件），每輪 `read_new` 至多一次。
    fn chatbox_verdict(&mut self, rec: &HookRecord) -> Option<&'static str> {
        if Some(rec.this) == self.chatbox {
            return None;
        }
        let now = (self.clock.now)();
        let recheck = Duration::from_secs(FOREIGN_RECHECK_SECONDS);
        if self.foreign.contains(&rec.this)
            && self
                .foreign_checked
                .is_some_and(|at| now.duration_since(at) < recheck)
        {
            return Some("foreign");
        }
        if !self.refound {
            self.refound = true;
            self.foreign_checked = Some(now);
            if let Err(err) = self.refind_chatbox() {
                self.chatbox = None;
                log(format!(
                    "[reader] chat box lookup failed (slot={}, pid={}): {err}",
                    self.slot, self.pid
                ));
            }
            if Some(rec.this) == self.chatbox {
                return None;
            }
        }
        // 找不到 ChatBox（轉場中）時不記為外來，否則主 ChatBox 會被永久擋掉
        let Some(main) = self.chatbox else {
            self.no_chatbox.push(rec.seq);
            return Some("no-chatbox");
        };
        if self.foreign.insert(rec.this) {
            log(format!(
                "[reader] ignoring AddLine from another chat box (slot={}, pid={}, this={:#x}, \
                 tab={}, main={main:#x})",
                self.slot, self.pid, rec.this, rec.tab
            ));
        }
        Some("foreign")
    }

    /// 重找主 ChatBox；位址變了就清掉按鈕快取與外來集合（新物件可能重用外來的舊位址）。
    fn refind_chatbox(&mut self) -> Result<(), MemError> {
        let found = find_by_name(self.conn().mem.as_ref(), self.root()?, CHATBOX_WINDOW)?
            .first()
            .map(|w| w.addr);
        if let Some(addr) = found
            && found != self.chatbox
        {
            self.buttons = None;
            self.foreign.clear();
            log(format!(
                "[reader] main chat box at {addr:#x} (slot={}, pid={})",
                self.slot, self.pid
            ));
        }
        self.chatbox = found;
        Ok(())
    }

    fn record(&mut self, seq: u64, tab: u32, raw: &str, verdict: &str) {
        if let Some(sink) = self.message_log.as_mut() {
            sink.record(seq, tab, raw, verdict);
        }
    }

    /// 同一則事件寫進 app.log 與 messages.log。
    fn note(&mut self, what: &str, detail: &str) {
        log(format!(
            "[reader] {what} (slot={}, pid={}, {detail})",
            self.slot, self.pid
        ));
        if let Some(sink) = self.message_log.as_mut() {
            sink.note(&format!("{what} ({detail})"));
        }
    }

    /// 遊戲聊天框的目前分頁與可見分頁；未連上、找不到 ChatBox 或讀取失敗回 None。
    pub fn tab_state(&mut self) -> Option<TabState> {
        self.attached.as_ref()?;
        let outcome = match self.read_tabs() {
            Ok(state) => state.map(Ok),
            Err(err) => {
                // 控件可能被遊戲重建：下一輪重找 ChatBox 與按鈕
                self.buttons = None;
                Some(Err(format!("read failed: {err}")))
            }
        };
        if outcome != self.tabs_logged {
            let shown = match &outcome {
                Some(Ok(state)) => {
                    let visible: Vec<String> = state
                        .tabs
                        .iter()
                        .map(|(tab, label)| format!("({tab}, {})", repr_str(label)))
                        .collect();
                    format!(
                        "current={}, visible=[{}]",
                        state.current,
                        visible.join(", ")
                    )
                }
                Some(Err(failed)) => failed.clone(),
                None => "unavailable (chat box not found)".to_owned(),
            };
            log(format!(
                "[reader] chat tabs {shown} (slot={}, pid={})",
                self.slot, self.pid
            ));
            self.tabs_logged = outcome.clone();
        }
        outcome.and_then(Result::ok)
    }

    fn read_tabs(&mut self) -> Result<Option<TabState>, Box<dyn Error>> {
        if self.chatbox.is_none() || self.buttons.is_none() {
            self.refind_chatbox()?;
        }
        let Some(chatbox) = self.chatbox else {
            return Ok(None);
        };
        if self.buttons.is_none() {
            let root = self.root()?;
            let found = find_tab_buttons(self.conn().mem.as_ref(), root)?;
            self.buttons = Some(found);
        }
        let buttons = self.buttons.as_deref().unwrap_or_default();
        Ok(Some(read_tab_state(
            self.conn().mem.as_ref(),
            chatbox,
            buttons,
        )?))
    }

    /// 只讀已知 ChatBox 的目前分頁（一次記憶體讀取）；沒有、讀不到或超出範圍回 None。
    pub fn current_tab(&self) -> Option<i32> {
        read_current_tab(self.attached.as_ref()?.mem.as_ref(), self.chatbox?)
    }

    /// 遊戲聊天輸入框目前是否開啟；未連上或讀取失敗一律視為關閉。
    ///
    /// 首次全樹搜尋一次，之後只讀一個旗標；節點失效自動重找。
    pub fn input_open(&mut self) -> bool {
        if self.attached.is_none() {
            return false;
        }
        let visible = self.edit_container().and_then(|node| match node {
            Some(node) => node
                .flags(self.conn().mem.as_ref())
                .map(|flags| flags & FLAG_VISIBLE != 0),
            None => Ok(false),
        });
        visible.unwrap_or_else(|_| {
            self.edit_node = None;
            false
        })
    }

    /// 快取的 chatEditContainer 節點；input_open 與定位共用同一份快取。
    fn edit_container(&mut self) -> Result<Option<Window>, MemError> {
        if self.edit_node.is_none() {
            let found = find_by_name(self.conn().mem.as_ref(), self.root()?, INPUT_CONTAINER)?;
            self.edit_node = found.first().copied();
        }
        Ok(self.edit_node)
    }

    /// 遊戲聊天輸入框在螢幕上的矩形 `(x, y, w, h)`；未連上、節點失效或量不到回 None。
    pub fn input_box_screen_rect(&mut self) -> Option<ScreenRect> {
        self.attached.as_ref()?;
        self.input_rect().unwrap_or_else(|err| {
            self.edit_node = None;
            log(format!(
                "[reader] game chat input rect unavailable (slot={}): {err}",
                self.slot
            ));
            None
        })
    }

    fn input_rect(&mut self) -> Result<Option<ScreenRect>, Box<dyn Error>> {
        let Some(node) = self.edit_container()? else {
            return Ok(None);
        };
        let mem = self.conn().mem.as_ref();
        let rect = node.rect(mem)?;
        // x2／y2 是含端點的邊：遊戲畫出的邊框比 x2−x1 多 1 邏輯 px（實機截圖逐像素比對）
        let size = (rect.x2 - rect.x1 + 1, rect.y2 - rect.y1 + 1);
        let mut offsets = vec![(rect.x1, rect.y1)];
        let mut current = node;
        while let Some(parent) = current.parent(mem)? {
            // 讀到撕裂的父指標可能成環；沒有上限的話 reader 執行緒會卡死、關閉時 join 不回來
            if offsets.len() > MAX_PARENT_DEPTH {
                return Err(format!("window parent chain deeper than {MAX_PARENT_DEPTH}").into());
            }
            let rect = parent.rect(mem)?;
            offsets.push((rect.x1, rect.y1));
            current = parent;
        }
        let root = self.root()?.rect(mem)?;
        let root_size = (root.x2 - root.x1, root.y2 - root.y1);
        let client = process::client_size(self.hwnd).ok_or("GetClientRect failed")?;
        let (x, y, w, h) = client_rect(&offsets, size, root_size, client).ok_or_else(|| {
            format!(
                "degenerate root window size ({}, {})",
                root_size.0, root_size.1
            )
        })?;
        let (left, top) =
            process::client_to_screen(self.hwnd, 0, 0).ok_or("ClientToScreen failed")?;
        Ok(Some((left + x, top + y, w, h)))
    }

    /// 開程序 → 清掉已結束程序的 hookstate → 修復殘留 hook → 掛 cave 與 root window hook
    /// → 存 hookstate → 等 hook 就緒 → 掛 AddLine hook → 存 hookstate。
    fn connect(&mut self) -> Result<(), GameNotRunning> {
        let pid = process::window_pid(self.hwnd);
        let mem = match (self.open)(pid) {
            Ok(mem) => mem,
            Err(err) => {
                self.teardown();
                return Err(match err {
                    // 程序在、handle 開不了＝完整性等級對不上（medium 開不了 high）
                    MemError::AccessDenied(_) => GameNotRunning::AccessDenied(format!(
                        "cannot open game process handle ({err}); the game is likely running \
                         elevated while this program is not"
                    )),
                    _ => GameNotRunning::Missing(format!("failed to open game process: {err}")),
                });
            }
        };
        if self.pid != pid {
            self.last_attach_failure = None;
        }
        self.pid = pid;
        if self.path_logged_for != pid {
            self.path_logged_for = pid;
            let path = process::install_path_of(pid)
                .map_or_else(|| "unknown".to_owned(), |p| repr_str(&p));
            log(format!(
                "[reader] game path: {path} (slot={}, pid={pid})",
                self.slot
            ));
        }
        hook_state::sweep(&self.state_dir, process::pid_alive);
        hook_state::repair(&self.state_dir, mem.as_ref(), pid, self.slot as usize);
        self.attached = Some(Attached {
            mem,
            cave: None,
            export: 0,
            ring: 0,
        });
        let waited = match self.attach() {
            Ok(waited) => waited,
            Err(err) => {
                self.teardown();
                return Err(if err.is_version_mismatch() {
                    GameNotRunning::VersionMismatch(format!(
                        "hook does not match this game build: {err}"
                    ))
                } else {
                    GameNotRunning::Missing(format!("failed to attach to game: {err}"))
                });
            }
        };
        self.last_attach_failure = None;
        // 連線成功才清：遊戲可能已改版；失敗重試也清的話每輪都會重洗警告
        forget_warned_icons();
        log(format!(
            "[reader] attached to game (slot={}, pid={pid}, hwnd={:#x}, hook_ready_in={:.1}s)",
            self.slot,
            self.hwnd,
            waited.as_secs_f64()
        ));
        Ok(())
    }

    /// 每個 hook 寫入後立刻存還原狀態，等待途中被硬砍才修得回來；AddLine 等 root 就緒才裝，逾時重試不會每次漏一塊 ring。
    fn attach(&mut self) -> Result<Duration, AttachError> {
        let Attached {
            mem, cave, export, ..
        } = self.attached.as_mut().expect("process is open");
        let cave = cave.insert(AutobotCave::install(mem.as_ref())?);
        *export = RootWindowHook::install(mem.as_ref(), cave)?.export_addr();
        self.save_hook_state();
        let waited = self.wait_root_window_ready()?;
        let ring = self.install_chat_hook()?;
        self.attached.as_mut().expect("process is open").ring = ring;
        self.save_hook_state();
        Ok(waited)
    }

    /// 等 hook 把 root window 位址寫回來（非 0＝hook 真的被執行過），回傳等待時間。
    ///
    /// hook 剛寫入時位址尚未有效，讀取失敗一律當成還沒好；真正的失敗只有逾時。
    fn wait_root_window_ready(&mut self) -> Result<Duration, AttachError> {
        let conn = self.attached.as_ref().expect("reader is attached");
        let started = (self.clock.now)();
        let deadline = started + HOOK_READY_TIMEOUT;
        loop {
            if conn.mem.read_u64(conn.export).is_ok_and(|addr| addr != 0) {
                return Ok((self.clock.now)() - started);
            }
            if (self.clock.now)() >= deadline {
                if remember(&mut self.last_attach_failure, "root window hook timeout") {
                    log(format!(
                        "[reader] root window hook never fired within {}s (slot={}, pid={}); the \
                     hook pattern likely does not match this game build",
                        HOOK_READY_TIMEOUT.as_secs(),
                        self.slot,
                        self.pid
                    ));
                }
                return Err(AttachError::Timeout);
            }
            (self.clock.sleep)(HOOK_READY_POLL);
        }
    }

    /// 掛上 AddLine hook（登記在 cave，由 cave 一併解除），回傳環形緩衝位址。
    fn install_chat_hook(&mut self) -> Result<u64, HookError> {
        let (slot, pid) = (self.slot, self.pid);
        let Attached { mem, cave, .. } = self.attached.as_mut().expect("process is open");
        let cave = cave.as_mut().expect("cave is installed");
        let hook = match AddLineHook::install(mem.as_ref(), cave) {
            Ok(hook) => hook,
            Err(err) => {
                if remember(&mut self.last_attach_failure, &format!("addline: {err}")) {
                    log(format!(
                        "[reader] AddLine hook install failed (slot={slot}, pid={pid}): {err}; \
                         a pattern failure here needs chat_hook::PATTERN updated in a new release"
                    ));
                }
                return Err(err);
            }
        };
        let base = mem.module_base(MODULE)?;
        log(format!(
            "[reader] AddLine hook installed (slot={slot}, pid={pid}, jump={:#x}, ring={:#x}, \
             ring_size={:#x})",
            hook.jump_addr().wrapping_sub(base).wrapping_add(IMAGE_BASE),
            hook.ring_addr(),
            HEADER + SLOTS * SLOT_SIZE
        ));
        Ok(hook.ring_addr())
    }

    /// 存 unhook 所需狀態；存不了就修不回，得讓 app.log 看得出原因。
    fn save_hook_state(&self) {
        let conn = self.conn();
        let Some(cave) = &conn.cave else {
            return;
        };
        let ops = cave.restore_ops();
        let saved = conn
            .mem
            .module_base(MODULE)
            .map_err(|err| err.to_string())
            .and_then(|base| {
                hook_state::save_state(&self.state_dir, self.pid, base, &ops)
                    .map_err(|err| err.to_string())
            });
        if let Err(err) = saved {
            log(format!(
                "[reader] could not save hook state (slot={}, pid={}, ops={}): {err}",
                self.slot,
                self.pid,
                ops.len()
            ));
        }
    }

    /// 解除 hook、關閉程序 handle，回到未連線狀態（下次 `read_new` 會重連）。
    fn teardown(&mut self) {
        if let Some(Attached { mem, cave, .. }) = self.attached.take() {
            let unhooked = match cave {
                Some(cave) => cave.uninstall_with(mem.as_ref(), |d| (self.clock.sleep)(d)),
                None => Ok(()),
            };
            match unhooked {
                Ok(()) if self.pid != 0 => hook_state::clear_state(&self.state_dir, self.pid),
                Ok(()) => {}
                // 狀態檔留著，下次啟動修復；不能靜默吞，否則下次才冒出「repaired hooks leaked」而查不出原因
                Err(err) => log(format!(
                    "[reader] unhook failed, leaving repair state for next launch (slot={}): {err}",
                    self.slot
                )),
            }
        }
        // 序號與 ChatBox 位址都屬於單一連線：重連後是新的 hook 與緩衝
        self.edit_node = None;
        self.last_seq = 0;
        self.chatbox = None;
        self.buttons = None;
        self.foreign.clear();
        self.tabs_logged = None;
        self.held = (0, 0);
    }

    /// 停止時呼叫：解除 hook、清掉 hookstate、關閉連線。
    pub fn close(&mut self) {
        self.teardown();
    }
}
