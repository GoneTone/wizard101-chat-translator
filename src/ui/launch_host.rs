//! 托管啟動遊戲帳號視窗：帳號清單與啟動進度常駐（關窗也保留），開著時才畫視窗；從視窗開的帳號都啟動就依設定自動關窗。

use std::time::Instant;

use eframe::egui;

use crate::launcher::Account;
use crate::launcher::install::InstallSource;
use crate::launcher::orchestrate::LaunchStatus;
use crate::launcher::patch::PatchStatus;
use crate::logs::log;
use crate::ui::settings::launch_list::LaunchList;
use crate::ui::settings::launch_window::{self, LaunchAction};
use crate::ui::settings::patch_card::PatchBoard;
use crate::ui::settings::theme;
use crate::ui::settings_host::{HostedWindow, Spec, WindowOps};

const LAUNCH: Spec = Spec {
    name: "launch window",
    title_key: "launch.title",
    size: theme::LAUNCH_SIZE,
    min_size: theme::LAUNCH_MIN_SIZE,
    topmost_once: true,
};

/// 啟動遊戲帳號視窗與它的狀態；所有方法都在 UI 執行緒呼叫。
pub struct LaunchHost {
    list: LaunchList,
    /// 各安裝的更新狀態；關窗也保留，重開照常顯示。
    patch: PatchBoard,
    /// 從視窗啟動帳號後要不要自動關窗（設定的 `launch_auto_close`）。
    auto_close: bool,
    open: bool,
    win: HostedWindow,
}

impl Default for LaunchHost {
    fn default() -> Self {
        Self {
            list: LaunchList::default(),
            patch: PatchBoard::default(),
            auto_close: true,
            open: false,
            win: HostedWindow::new(LAUNCH),
        }
    }
}

impl LaunchHost {
    /// 換掉視窗操作（測試與 probe 用：不搶前景、假的工作區）。
    pub fn set_window_ops(&mut self, ops: WindowOps) {
        self.win.ops = ops;
    }

    /// 開窗；已開著就叫到前景。
    pub fn open(&mut self) {
        if self.open {
            log("[ui] launch window already open; bringing it to the front");
            self.win.bring_to_front();
            return;
        }
        self.open = true;
        log(format!(
            "[ui] launch window opened: accounts={} launchable={} auto_close={}",
            self.list.accounts().len(),
            self.list.open_count(),
            self.auto_close
        ));
    }

    /// 關窗；不再等這次啟動的結果，重開時不會因為之前的啟動自動關掉。
    pub fn close(&mut self) {
        if std::mem::take(&mut self.open) {
            log("[ui] launch window closed");
            self.list.forget_batch();
            self.win.reset();
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// 視窗的 HWND；沒開或還沒找到是 0。
    pub fn hwnd(&self) -> isize {
        self.win.hwnd
    }

    /// 換上帳號清單與上次的勾選（不存在的 id 丟掉）；執行中名單與進度保留。
    pub fn set_accounts(&mut self, accounts: Vec<Account>, selection: &[String]) {
        self.list.set_accounts(accounts, selection);
    }

    /// 換上執行中的帳號名稱；執行中的帳號停用勾選與〔啟動〕、不計入啟動數。
    pub fn set_running(&mut self, names: Vec<String>) {
        self.list.set_running(names);
    }

    /// 啟動後自動關窗的設定值。
    pub fn auto_close(&self) -> bool {
        self.auto_close
    }

    pub fn set_auto_close(&mut self, on: bool) {
        self.auto_close = on;
    }

    /// 某帳號的啟動進度有變；從視窗開的帳號都有結果、沒有失敗且開著自動關窗時關窗。
    pub fn set_launch_status(&mut self, id: &str, status: LaunchStatus) {
        self.list.set_status(id, status);
        let Some(settled) = self.list.settle() else {
            return;
        };
        if settled.failed > 0 {
            log(format!(
                "[ui] launch window kept open: {} account(s) failed, {} opened",
                settled.failed, settled.opened
            ));
        } else if !self.auto_close {
            log(format!(
                "[ui] launch window kept open: auto-close is off ({} account(s) opened)",
                settled.opened
            ));
        } else {
            log(format!(
                "[ui] launch window auto-closed after {} account(s) opened",
                settled.opened
            ));
            self.close();
        }
    }

    /// 某份安裝的更新狀態有變；`None` 是收起。
    pub fn set_patch_status(&mut self, source: InstallSource, status: Option<PatchStatus>) {
        self.patch.apply(source, status, Instant::now());
    }

    /// 每幀呼叫：開著就畫出視窗；✕ 或 Alt+F4 直接關窗，其餘動作交給 App。
    pub fn show(&mut self, ctx: &egui::Context, known: &[isize]) -> Vec<LaunchAction> {
        if !self.open {
            return Vec::new();
        }
        let Self {
            list,
            patch,
            auto_close,
            win,
            ..
        } = self;
        let mut closing = false;
        let actions = win.show(ctx, known, |ui, close| {
            closing |= close;
            launch_window::show(ui, list, patch, auto_close)
        });
        if closing {
            self.close();
        }
        actions
    }
}
