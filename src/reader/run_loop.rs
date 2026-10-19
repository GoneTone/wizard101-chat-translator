//! 單一遊戲客戶端的收訊執行緒主迴圈：讀聊天 → 推進上下文、在 overlay 佔位 → 交給翻譯池。
//!
//! 多個客戶端各跑一條，由 supervisor 起／收。本迴圈不做翻譯（單則卡住不延誤後續讀取），
//! 也不直接碰 UI —— 所有更新都包成 `UiEvent` 投遞給 UI 執行緒。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

use super::chat_tabs::TabState;
use super::mem_reader::{ChatEvent, GameNotRunning, WizChatReader};
use super::process;
use super::status::StatusBoard;
use crate::config::{Config, py};
use crate::i18n::t;
use crate::logs::log;
use crate::translation::cache::TranslationCache;
use crate::translation::context::ClientContexts;
use crate::translation::pool::TranslationPool;
use crate::ui::events::{UiEvent, UiPoster};
use crate::util::{Clock, read};

/// 收訊間隔：每輪只讀 hook 環形緩衝的計數器，很便宜。
pub const POLL_INTERVAL: Duration = Duration::from_millis(100);
/// 遊戲聊天輸入框的取樣間隔：只讀一個可見性旗標，可比 `POLL_INTERVAL` 密得多。
pub const INPUT_POLL_INTERVAL: Duration = Duration::from_millis(50);
/// 完整分頁狀態與前景的取樣間隔：一次要讀約 45 處記憶體；切換分頁另由每輪讀目前分頁偵測。
pub const TAB_POLL_INTERVAL: Duration = Duration::from_millis(400);
/// 找不到遊戲時的重試間隔。
pub const GAME_MISSING_INTERVAL: Duration = Duration::from_secs(5);
/// 掛入點與遊戲版本對不上時的重試間隔：重試也不會自己好起來，不必每 5 秒重掃一次。
pub const VERSION_MISMATCH_INTERVAL: Duration = Duration::from_secs(60);
/// 長時間等待切成這麼長的片段，停止時最多延遲一段才醒來。
const WAIT_SLICE: Duration = Duration::from_millis(50);

/// 等 `duration`，`stop` 被設定時提早返回（`CancellationToken` 沒有阻塞式等待，故分段睡）。
pub(crate) fn wait(clock: &Clock, stop: &CancellationToken, duration: Duration) {
    let deadline = (clock.now)() + duration;
    while !stop.is_cancelled() {
        let remaining = deadline.saturating_duration_since((clock.now)());
        if remaining.is_zero() {
            return;
        }
        (clock.sleep)(remaining.min(WAIT_SLICE));
    }
}

/// 跨所有收訊執行緒共用的訊息序號：翻譯池以它回填 overlay 的佔位列，必須全域唯一。
pub struct MessageIds(AtomicU64);

impl Default for MessageIds {
    fn default() -> Self {
        Self(AtomicU64::new(1))
    }
}

impl MessageIds {
    /// 下一個序號（從 1 起）。
    pub fn next(&self) -> u64 {
        self.0.fetch_add(1, Ordering::Relaxed)
    }
}

/// 收訊迴圈需要的 reader 能力；`WizChatReader` 實作，測試注入假的。
pub trait ChatSource: Send {
    /// 自上次呼叫後新增的聊天事件；未連線時先連線。
    fn read_new(&mut self) -> Result<Vec<ChatEvent>, GameNotRunning>;
    /// 是否已掛入遊戲。
    fn anchored(&self) -> bool;
    /// 遊戲聊天框目前的分頁（一次記憶體讀取）。
    fn current_tab(&self) -> Option<i32>;
    /// 完整分頁狀態。
    fn tab_state(&mut self) -> Option<TabState>;
    /// 遊戲聊天輸入框是否開啟。
    fn input_open(&mut self) -> bool;
    /// 遊戲聊天輸入框的螢幕矩形 `(x, y, w, h)`。
    fn input_box_screen_rect(&mut self) -> Option<(i32, i32, i32, i32)>;
    /// 是否輸出系統訊息。
    fn set_emit_system(&mut self, emit: bool);
    /// 解除 hook、關閉連線。
    fn close(&mut self);
}

impl ChatSource for WizChatReader {
    fn read_new(&mut self) -> Result<Vec<ChatEvent>, GameNotRunning> {
        WizChatReader::read_new(self)
    }
    fn anchored(&self) -> bool {
        WizChatReader::anchored(self)
    }
    fn current_tab(&self) -> Option<i32> {
        WizChatReader::current_tab(self)
    }
    fn tab_state(&mut self) -> Option<TabState> {
        WizChatReader::tab_state(self)
    }
    fn input_open(&mut self) -> bool {
        WizChatReader::input_open(self)
    }
    fn input_box_screen_rect(&mut self) -> Option<(i32, i32, i32, i32)> {
        WizChatReader::input_box_screen_rect(self)
    }
    fn set_emit_system(&mut self, emit: bool) {
        self.emit_system = emit;
    }
    fn close(&mut self) {
        WizChatReader::close(self);
    }
}

/// 所有客戶端的收訊執行緒共用的物件；狀態一律經 `board` 回報，由它彙整多個客戶端。
pub struct ReaderShared {
    /// 執行中的設定（設定視窗可能隨時改）。
    pub settings: Arc<RwLock<Config>>,
    /// 送往 overlay 的事件投遞端。
    pub ui: UiPoster,
    /// 程式關閉的訊號。
    pub stop: CancellationToken,
    /// 玩家對話的翻譯池。
    pub pool: Arc<TranslationPool>,
    /// 系統訊息的翻譯池。
    pub system_pool: Arc<TranslationPool>,
    /// 各客戶端狀態彙整（狀態列與橫幅）。
    pub board: Arc<StatusBoard>,
    /// 跨收訊執行緒共用的訊息序號（翻譯池以它回填佔位列）。
    pub msg_ids: Arc<MessageIds>,
    /// 系統訊息譯文快取。
    pub cache: Arc<TranslationCache>,
}

/// 迴圈對外界的依賴：時鐘、視窗是否存在、前景視窗；測試換成假的。
pub struct LoopEnv {
    /// 等待與計時用的時鐘。
    pub clock: Clock,
    /// 視窗是否仍存在。
    pub window_exists: Box<dyn FnMut(isize) -> bool + Send>,
    /// 目前前景視窗。
    pub foreground_window: Box<dyn FnMut() -> isize + Send>,
}

impl Default for LoopEnv {
    fn default() -> Self {
        Self {
            clock: Clock::default(),
            window_exists: Box::new(process::window_exists),
            foreground_window: Box::new(process::foreground_window),
        }
    }
}

/// 一個遊戲客戶端（`hwnd`）的收訊執行緒進入點；stop 被設定或該視窗消失後解除 hook 再返回。
///
/// `slot`＝這個客戶端的顯示編號；`reader` 由呼叫端建好（含 messages.log），外界依賴由 `env` 提供。
/// 結束時（含 panic）一定收回編號並 `close` reader。
pub fn reader_loop<R: ChatSource>(
    env: LoopEnv,
    shared: Arc<ReaderShared>,
    hwnd: isize,
    slot: u32,
    reader: R,
    contexts: Arc<ClientContexts>,
) {
    ClientLoop {
        env,
        s: shared,
        hwnd,
        slot,
        reader,
        contexts,
        input_open: false,
        game_issue: None,
        reported_tabs: None,
        tabs_polled_at: None,
        sampled_tab: None,
    }
    .run();
}

/// reader_loop 的本體：一個客戶端的讀取、分頁同步與訊息分派。
struct ClientLoop<R: ChatSource> {
    env: LoopEnv,
    s: Arc<ReaderShared>,
    hwnd: isize,
    slot: u32,
    reader: R,
    contexts: Arc<ClientContexts>,
    input_open: bool,
    /// 遊戲端問題的橫幅文案 key（`None`＝遊戲正常）。
    game_issue: Option<&'static str>,
    reported_tabs: Option<TabState>,
    tabs_polled_at: Option<Instant>,
    /// 上次完整取樣時的目前分頁。
    sampled_tab: Option<i32>,
}

impl<R: ChatSource> Drop for ClientLoop<R> {
    // 迴圈本體 panic 也要收回編號、解除 hook，不留殭屍編號與未 unhook 的客戶端
    fn drop(&mut self) {
        self.s.board.remove(self.slot);
        self.reader.close();
    }
}

impl<R: ChatSource> ClientLoop<R> {
    fn run(mut self) {
        while !self.s.stop.is_cancelled() {
            if !(self.env.window_exists)(self.hwnd) {
                log(format!(
                    "[reader] game window gone (slot={}, hwnd={:#x}); stopping this reader",
                    self.slot, self.hwnd
                ));
                self.force_input_closed();
                self.report_tabs(None);
                break;
            }
            let Some(events) = self.read() else {
                continue;
            };
            self.sync_tabs();
            for event in events {
                self.dispatch(event);
            }
            self.report_activity();
            self.poll_input();
            self.wait_watching_input(POLL_INTERVAL);
        }
    }

    fn listening_status(&self) -> &'static str {
        if self.reader.anchored() {
            "listening"
        } else {
            "locating"
        }
    }

    /// 讀這輪的新聊天事件；遊戲不在時已等過重試間隔，回 None 讓迴圈重來。
    fn read(&mut self) -> Option<Vec<ChatEvent>> {
        let slot = self.slot;
        // 每輪重讀設定：設定視窗可能在執行中切換系統訊息開關
        let emit = py::truthy(read(&self.s.settings).get("translate_system_messages"));
        self.reader.set_emit_system(emit);
        self.report_activity();
        match self.reader.read_new() {
            Ok(events) => {
                if self.game_issue.take().is_some() {
                    log(format!("[reader] game back, resuming (slot={slot})"));
                }
                Some(events)
            }
            Err(err) => {
                let (status, issue, retry_in) = match err {
                    GameNotRunning::AccessDenied(_) => (
                        "access_denied",
                        "notice.access_denied",
                        GAME_MISSING_INTERVAL,
                    ),
                    GameNotRunning::VersionMismatch(_) => (
                        "version_mismatch",
                        "notice.version_mismatch",
                        VERSION_MISMATCH_INTERVAL,
                    ),
                    GameNotRunning::Missing(_) => {
                        ("waiting_game", "notice.game_missing", GAME_MISSING_INTERVAL)
                    }
                };
                // 只在原因改變時記錄，否則每輪重試都灌一行
                if self.game_issue != Some(issue) {
                    log(format!(
                        "[reader] game not ready (slot={slot}, hwnd={:#x}): {err}",
                        self.hwnd
                    ));
                }
                self.game_issue = Some(issue);
                self.s.board.report(slot, status, self.game_issue);
                self.force_input_closed();
                self.report_tabs(None);
                wait(&self.env.clock, &self.s.stop, retry_in);
                None
            }
        }
    }

    fn sync_tabs(&mut self) {
        // 目前分頁只要一次讀取、每輪都看：遊戲裡切分頁不必等 TAB_POLL_INTERVAL 才同步
        let current = self.reader.current_tab();
        let now = (self.env.clock.now)();
        if let Some(at) = self.tabs_polled_at
            && now.saturating_duration_since(at) < TAB_POLL_INTERVAL
            && (current.is_none() || current == self.sampled_tab)
        {
            return;
        }
        self.tabs_polled_at = Some(now);
        let state = self.reader.tab_state();
        // 完整取樣失敗時記下這輪讀到的分頁，免得每輪都重試完整取樣
        self.sampled_tab = state.as_ref().map(|s| s.current).or(current);
        self.report_tabs(state);
        // 每次取樣都送（overlay 對同一客戶端是 no-op）：只送邊緣時，兩條執行緒錯開會漏掉快速 A→B→A
        if (self.env.foreground_window)() == self.hwnd {
            self.s.ui.post(UiEvent::SetForegroundSlot(Some(self.slot)));
        }
    }

    fn report_tabs(&mut self, state: Option<TabState>) {
        if state == self.reported_tabs {
            return;
        }
        // 遊戲不在時分頁不明：發話上下文退回主分頁
        self.contexts
            .set_current_tab(state.as_ref().map_or(0, |s| s.current));
        self.reported_tabs.clone_from(&state);
        self.s.ui.post(UiEvent::SetTabs {
            slot: self.slot,
            state,
        });
    }

    /// 一則新訊息：在 overlay 佔位並交給對應的翻譯池（系統訊息快取命中則直接顯示完成態）。
    fn dispatch(&self, event: ChatEvent) {
        let s = &self.s;
        let msg_id = s.msg_ids.next();
        let ChatEvent { line, tabs, .. } = event;
        let add = |translated: String, msg_id: Option<u64>| UiEvent::AddMessage {
            slot: self.slot,
            tabs: tabs.clone(),
            original: line.text.clone(),
            translated,
            pending: msg_id.is_some(),
            msg_id,
            color: line.color.clone(),
        };
        if line.system {
            // 系統訊息不進上下文；快取命中就直接以完成態顯示，不佔位也不進 pool
            if let Some(cached) = s.cache.get(&line.text) {
                s.ui.post(add(cached, None));
                return;
            }
            s.ui.post(add(t("notice.pending", &[]), Some(msg_id)));
            s.system_pool.submit(line.text, Vec::new(), msg_id);
            return;
        }
        let context = self.contexts.for_tab(tabs.first().copied().unwrap_or(0));
        // 該行之前的行；提交後即固定，重試不漂移
        let snapshot = context.snapshot();
        context.push(line.text.clone());
        s.ui.post(add(t("notice.pending", &[]), Some(msg_id)));
        s.pool.submit(line.text, snapshot, msg_id);
    }

    fn report_activity(&self) {
        let s = &self.s;
        let state = if s.pool.in_flight() > 0 || s.system_pool.in_flight() > 0 {
            "translating"
        } else {
            self.listening_status()
        };
        s.board.report(self.slot, state, self.game_issue);
    }

    /// 取樣一次遊戲聊天輸入框，狀態翻轉才記錄並投遞開關事件。
    fn poll_input(&mut self) {
        let now_open = self.reader.input_open();
        if now_open == self.input_open {
            return;
        }
        self.input_open = now_open;
        if now_open {
            let rect = self.reader.input_box_screen_rect();
            let anchor = match rect {
                Some((x, y, w, h)) => format!("({x}, {y}, {w}, {h})"),
                None => "unavailable".to_owned(),
            };
            log(format!(
                "[reader] game chat input opened (slot={}, hwnd={:#x}, anchor={anchor})",
                self.slot, self.hwnd
            ));
            self.s.ui.post(UiEvent::GameInputOpened {
                hwnd: self.hwnd,
                anchor: rect,
            });
        } else {
            self.log_input_closed();
        }
    }

    /// 遊戲斷線＝輸入框已不存在：開著就同步收回。
    fn force_input_closed(&mut self) {
        if self.input_open {
            self.input_open = false;
            self.log_input_closed();
        }
    }

    fn log_input_closed(&self) {
        log(format!(
            "[reader] game chat input closed (slot={}, hwnd={:#x})",
            self.slot, self.hwnd
        ));
        self.s.ui.post(UiEvent::GameInputClosed { hwnd: self.hwnd });
    }

    /// 等待下一輪讀取，期間以 `INPUT_POLL_INTERVAL` 持續取樣輸入框狀態。
    ///
    /// 不另開執行緒：reader 不支援跨執行緒併發呼叫。
    fn wait_watching_input(&mut self, duration: Duration) {
        let deadline = (self.env.clock.now)() + duration;
        while !self.s.stop.is_cancelled() {
            let remaining = deadline.saturating_duration_since((self.env.clock.now)());
            if remaining.is_zero() {
                return;
            }
            (self.env.clock.sleep)(remaining.min(INPUT_POLL_INTERVAL));
            self.poll_input();
        }
    }
}
