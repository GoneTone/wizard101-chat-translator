//! 每個本地模型一個 llama-server 子程序：按需啟動、失敗重試，依設定收掉或重啟。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::sync::OwnedMutexGuard;
use tokio_util::sync::CancellationToken;

use super::LocalNotice;
use super::engine::{self, EngineInstall};
use super::hf::hf_client;
use super::hub::{EngineState, Local};
use super::server::{self, Mtp, Process, READY_TIMEOUT, ServerPlan};
use super::store::{self, Installed};
use crate::config::Config;
use crate::config::local::{self, LocalSettings};
use crate::config::services::{LOCAL_PROVIDER, ServiceSlot, resolve};
use crate::logs::log;
use crate::util::lock;

/// 連續失敗（啟動失敗或意外結束）幾次後停在失敗狀態。
const MAX_FAILURES: u32 = 2;

/// 所有模型的子程序，以模型 id 為鍵。
#[derive(Default)]
pub(super) struct Servers(Mutex<BTreeMap<String, Arc<Slot>>>);

#[derive(Default)]
struct Slot {
    /// 讓同一模型的並發啟動只跑一次；持有者即進行中的啟動。
    start: Arc<tokio::sync::Mutex<()>>,
    state: Mutex<SlotState>,
}

/// 刪檔期間占住模型的啟動鎖；排隊中的 `ensure` 等它放掉才會往下跑。
pub(super) struct Hold {
    slot: Arc<Slot>,
    _start: OwnedMutexGuard<()>,
}

impl Drop for Hold {
    fn drop(&mut self) {
        lock(&self.slot.state).closing -= 1;
    }
}

#[derive(Default)]
struct SlotState {
    running: Option<Running>,
    failures: u32,
    /// 這串連續失敗裡有意外結束：之後的啟動成功不清零，啟動就崩的才會停在失敗。
    crashed: bool,
    error: String,
    /// 進行中啟動的取消鈕；`stop` 按下後該次啟動會殺掉子程序並作廢。
    starting: CancellationToken,
    /// 等著占住啟動鎖的 `stop_and_wait` 數量；不為 0 時拿到鎖的 `ensure` 要讓開。
    closing: u32,
}

impl SlotState {
    fn clear_failures(&mut self) {
        self.failures = 0;
        self.crashed = false;
    }

    fn fail(&mut self, id: &str, error: String) {
        self.failures += 1;
        self.error = error;
        if self.failures == MAX_FAILURES {
            log(format!(
                "[local] llama-server[{id}] failed {MAX_FAILURES} times in a row; not retrying until the settings are applied again"
            ));
        }
    }
}

struct Running {
    process: Box<dyn Process>,
    base_url: String,
    /// 啟動用的 llama-server.exe（判斷引擎資料夾還有沒有人在用）。
    exe: PathBuf,
}

/// 載入橫幅：持有期間顯示該模型載入中；放掉時改顯示其他還在載入的，都沒有才收起。
pub struct Loading {
    hub: Arc<Local>,
    name: String,
}

impl Drop for Loading {
    fn drop(&mut self) {
        let next = {
            let mut loading = lock(&self.hub.loading);
            if let Some(i) = loading.iter().position(|n| *n == self.name) {
                loading.remove(i);
            }
            loading.last().cloned()
        };
        (self.hub.notify)(LocalNotice::Loading(next));
    }
}

impl Servers {
    fn slot(&self, id: &str) -> Arc<Slot> {
        lock(&self.0).entry(id.to_owned()).or_default().clone()
    }

    fn get(&self, id: &str) -> Option<Arc<Slot>> {
        lock(&self.0).get(id).cloned()
    }

    fn all(&self) -> Vec<(String, Arc<Slot>)> {
        lock(&self.0)
            .iter()
            .map(|(id, slot)| (id.clone(), slot.clone()))
            .collect()
    }
}

impl Local {
    /// 回傳模型的 llama-server base URL；沒在跑就啟動並等它就緒。
    pub async fn ensure(&self, model_id: &str) -> Result<String, String> {
        let slot = self.servers.slot(model_id);
        // 拿到啟動鎖就在狀態鎖內換新取消鈕：之後的 stop 一定取消得到這次啟動
        let (_start, cancel) = loop {
            let guard = slot.start.clone().lock_owned().await;
            let mut st = lock(&slot.state);
            if st.closing == 0 {
                // 在狀態鎖內檢查：shutdown 之前就過了這關的啟動，會被它取消
                if self.closed.load(Ordering::SeqCst) {
                    return Err("local runtime is shut down".into());
                }
                st.starting = CancellationToken::new();
                break (guard, st.starting.clone());
            }
            drop(st);
            // 鎖是先到先得：放掉再排一次，就排到正在等的 stop_and_wait 後面
            drop(guard);
        };
        let _engine_files = self.engine_files.read().await;
        {
            let mut st = lock(&slot.state);
            if cancel.is_cancelled() {
                log(format!(
                    "[local] llama-server[{model_id}] stopped before starting"
                ));
                return Err(stopped_error(model_id));
            }
            if let Some(running) = st.running.as_mut() {
                let Some(code) = running.process.exited() else {
                    return Ok(running.base_url.clone());
                };
                let tail = running.process.tail().join("\n");
                log(format!(
                    "[local] llama-server[{model_id}] exited unexpectedly code={code}"
                ));
                st.running = None;
                st.crashed = true;
                st.fail(
                    model_id,
                    format!("llama-server exited unexpectedly with code {code}:\n{tail}"),
                );
            }
            if st.failures >= MAX_FAILURES {
                return Err(st.error.clone());
            }
        }
        // 前置條件不符（還沒下載完、沒有引擎）不算失敗：下載或安裝完就該能直接啟動
        let model = store::list(&self.root)
            .into_iter()
            .find(|m| m.id == model_id && m.complete)
            .ok_or_else(|| format!("model {model_id} is not downloaded"))?;
        if engine::read_active(&self.root).current.is_none() {
            return Err("llama.cpp engine is not installed".into());
        }
        let result = self.load(&model, &cancel).await;
        let mut st = lock(&slot.state);
        match result {
            Ok(mut running) if cancel.is_cancelled() => {
                running.process.kill();
                Err(stopped_error(model_id))
            }
            Ok(running) => {
                if !st.crashed {
                    st.failures = 0;
                }
                let url = running.base_url.clone();
                st.running = Some(running);
                Ok(url)
            }
            Err(e) if cancel.is_cancelled() => Err(e),
            Err(e) => {
                st.fail(model_id, e.clone());
                Err(e)
            }
        }
    }

    /// 停掉模型的 llama-server（含進行中的啟動）；沒在跑則無動作。
    pub fn stop(&self, model_id: &str) {
        if let Some(slot) = self.servers.get(model_id) {
            stop_slot(model_id, &slot);
        }
    }

    /// 程式關閉：停掉所有 llama-server，之後的 `ensure` 一律回錯誤。
    pub fn shutdown(&self) {
        log("[local] shutting down; no more llama-server starts");
        self.closed.store(true, Ordering::SeqCst);
        self.stop_all();
    }

    /// 停掉所有 llama-server（含進行中的啟動）。
    pub(super) fn stop_all(&self) {
        for (id, slot) in self.servers.all() {
            stop_slot(&id, &slot);
        }
    }

    /// 停掉模型並等進行中的啟動收尾；持有回傳值期間不會有新的啟動，可安全刪檔。
    pub(super) async fn stop_and_wait(&self, model_id: &str) -> Hold {
        let slot = self.servers.slot(model_id);
        close(model_id, &slot);
        hold(model_id, slot).await
    }

    /// 清掉模型的失敗紀錄（例如下載完成後）。
    pub(super) fn clear_failures(&self, model_id: &str) {
        if let Some(slot) = self.servers.get(model_id) {
            lock(&slot.state).clear_failures();
        }
    }

    /// 跑著 llama-server 的模型 id。
    pub(super) fn running_ids(&self) -> BTreeSet<String> {
        self.servers
            .all()
            .into_iter()
            .filter(|(_, slot)| lock(&slot.state).running.is_some())
            .map(|(id, _)| id)
            .collect()
    }

    /// 有跑著的 llama-server 是從這個引擎的資料夾啟動的。
    fn engine_in_use(&self, install: &EngineInstall) -> bool {
        let dir = engine::engine_dir(&self.root, install);
        self.servers.all().iter().any(|(_, slot)| {
            lock(&slot.state)
                .running
                .as_ref()
                .is_some_and(|r| r.exe.starts_with(&dir))
        })
    }

    /// 睡著的模型被請求喚醒時呼叫：橫幅顯示它載入中，直到回傳值放掉。
    pub fn waking(&self, model_id: &str) -> Loading {
        log(format!(
            "[local] llama-server[{model_id}] is sleeping; the request reloads it"
        ));
        let name = lock(&self.state)
            .models
            .iter()
            .find(|m| m.id == model_id)
            .map_or_else(|| model_id.to_owned(), |m| m.manifest.display_name());
        self.loading_banner(name)
    }

    fn loading_banner(&self, name: String) -> Loading {
        lock(&self.loading).push(name.clone());
        (self.notify)(LocalNotice::Loading(Some(name.clone())));
        Loading {
            hub: self.arc(),
            name,
        }
    }

    /// 套用設定：收掉沒有用途引用的模型、重啟設定變動的，`preload` 時依序預先載入。
    pub fn apply_config(&self, cfg: &Config) {
        let new = local::settings(cfg);
        let wanted: BTreeSet<String> = ServiceSlot::ALL
            .into_iter()
            .map(|slot| resolve(cfg, slot))
            .filter(|s| s.provider == LOCAL_PROVIDER && !s.model.is_empty())
            .map(|s| s.model)
            .collect();
        let old = std::mem::replace(&mut *lock(&self.settings), new.clone());
        log(format!(
            "[local] apply config: models={wanted:?} preload={} idle_minutes={}",
            new.preload, new.idle_minutes
        ));
        let mut restart = Vec::new();
        for (id, slot) in self.servers.all() {
            let changed =
                old.idle_minutes != new.idle_minutes || mtp_on(&old, &id) != mtp_on(&new, &id);
            if wanted.contains(&id) && !changed {
                lock(&slot.state).clear_failures();
                continue;
            }
            if wanted.contains(&id) && lock(&slot.state).running.is_some() {
                log(format!(
                    "[local] restarting llama-server[{id}] for new settings"
                ));
                restart.push(id.clone());
            }
            stop_slot(&id, &slot);
        }
        self.ensure_in_order(if new.preload {
            wanted.into_iter().collect()
        } else {
            restart
        });
    }

    /// 停掉跑著的 llama-server 再依序重啟（換引擎後用新版）。
    pub(super) fn restart_servers(&self) {
        let running: Vec<String> = self
            .servers
            .all()
            .into_iter()
            .filter(|(_, slot)| lock(&slot.state).running.is_some())
            .map(|(id, _)| id)
            .collect();
        self.stop_all();
        self.ensure_in_order(running);
    }

    /// 在背景依序啟動：前一個就緒（或失敗）才啟動下一個，讓 `--fit` 看到已佔用的記憶體。
    fn ensure_in_order(&self, ids: Vec<String>) {
        if ids.is_empty() {
            return;
        }
        let me = self.arc();
        self.rt.spawn(async move {
            me.measured_once().await;
            for id in ids {
                if me.closed.load(Ordering::SeqCst) {
                    return;
                }
                if let Err(e) = me.ensure(&id).await {
                    log(format!("[local] preloading {id} failed: {e}"));
                }
            }
        });
    }

    async fn load(&self, model: &Installed, cancel: &CancellationToken) -> Result<Running, String> {
        let _banner = self.loading_banner(model.manifest.display_name());
        self.start_with_fallbacks(model, cancel).await
    }

    /// 失敗時先關 MTP 重試，再對尚未確認的新引擎回退重試；新引擎首次就緒即確認。
    async fn start_with_fallbacks(
        &self,
        model: &Installed,
        cancel: &CancellationToken,
    ) -> Result<Running, String> {
        let settings = self.settings();
        let mut mtp = mtp_mode(&self.root, model, mtp_on(&settings, &model.id));
        loop {
            let active = engine::read_active(&self.root);
            let exe = active
                .current
                .as_ref()
                .and_then(|i| engine::server_exe(&engine::engine_dir(&self.root, i)))
                .ok_or("llama.cpp engine is not installed")?;
            let model_file = store::main_file(&self.root, model)
                .map_err(|e| format!("model {} has no usable main file: {e}", model.id))?;
            let plan = ServerPlan {
                model_id: model.id.clone(),
                exe,
                model: model_file,
                mtp: mtp.clone(),
                idle_minutes: settings.idle_minutes,
            };
            let err = match self.launch(&plan, cancel).await {
                Ok(running) => {
                    match &active.previous {
                        Some(p) if active.current.as_ref() != Some(p) && self.engine_in_use(p) => {
                            log(format!(
                                "[local] previous engine {}-{} is still in use; confirming later",
                                p.build, p.variant
                            ))
                        }
                        Some(_) => {
                            if let Err(e) = engine::confirm(&self.root) {
                                log(format!("[local] confirming engine failed: {e}"));
                            }
                        }
                        None => {}
                    }
                    return Ok(running);
                }
                Err(_) if cancel.is_cancelled() => return Err(stopped_error(&model.id)),
                Err(e) => e,
            };
            if mtp != Mtp::Off {
                log(format!(
                    "[local] llama-server[{}] failed with MTP, retrying without MTP",
                    model.id
                ));
                mtp = Mtp::Off;
                continue;
            }
            if active.previous.is_none() {
                return Err(err);
            }
            if let Some(current) = &active.current
                && self.engine_in_use(current)
            {
                log(format!(
                    "[local] llama-server[{}] failed on {}-{}, not rolling back: other models run on it",
                    model.id, current.build, current.variant
                ));
                return Err(err);
            }
            match engine::rollback(&self.root) {
                Ok(Some(previous)) => {
                    log(format!(
                        "[local] llama-server[{}] failed on the new engine, retrying with {}-{}",
                        model.id, previous.build, previous.variant
                    ));
                    self.update(|s| s.engine = EngineState::Ready(previous));
                }
                Ok(None) => return Err(err),
                Err(e) => {
                    log(format!("[local] engine rollback failed: {e}"));
                    return Err(err);
                }
            }
        }
    }

    async fn launch(
        &self,
        plan: &ServerPlan,
        cancel: &CancellationToken,
    ) -> Result<Running, String> {
        let port = server::free_port().map_err(|e| e.to_string())?;
        let base_url = format!("http://127.0.0.1:{port}");
        log(format!(
            "[local] starting llama-server[{}]: {} {}",
            plan.model_id,
            plan.exe.display(),
            server::args(plan, port).join(" ")
        ));
        let mut process = self.launcher.launch(plan, port).map_err(|e| {
            log(format!(
                "[local] llama-server[{}] failed to launch: {e}",
                plan.model_id
            ));
            e.to_string()
        })?;
        let started = Instant::now();
        let http = hf_client(&base_url);
        let ready = tokio::select! {
            r = server::wait_ready(&http, &base_url, process.as_mut(), READY_TIMEOUT) => r,
            () = cancel.cancelled() => Err(stopped_error(&plan.model_id)),
        };
        if let Err(e) = ready {
            if cancel.is_cancelled() {
                log(format!(
                    "[local] llama-server[{}] stopped while starting",
                    plan.model_id
                ));
                process.kill();
            }
            return Err(e);
        }
        log(format!(
            "[local] llama-server[{}] ready on port {port} in {:.1}s",
            plan.model_id,
            started.elapsed().as_secs_f32()
        ));
        Ok(Running {
            process,
            base_url,
            exe: plan.exe.clone(),
        })
    }
}

fn stopped_error(id: &str) -> String {
    format!("model {id} was unloaded while starting")
}

/// 標記要占住啟動鎖並先停掉；之後必須接著 `hold`，`Hold` 放掉時才會取消標記。
fn close(id: &str, slot: &Slot) {
    lock(&slot.state).closing += 1;
    stop_slot(id, slot);
}

async fn hold(id: &str, slot: Arc<Slot>) -> Hold {
    let start = slot.start.clone().lock_owned().await;
    // 排在前面的啟動可能已經又跑起來，拿到鎖後再停一次
    stop_slot(id, &slot);
    Hold {
        slot,
        _start: start,
    }
}

fn stop_slot(id: &str, slot: &Slot) {
    let mut st = lock(&slot.state);
    st.starting.cancel();
    st.clear_failures();
    if let Some(mut running) = st.running.take() {
        log(format!("[local] stopping llama-server[{id}]"));
        running.process.kill();
    }
}

/// 沒設定過的模型預設開啟 MTP。
fn mtp_on(settings: &LocalSettings, id: &str) -> bool {
    settings.mtp.get(id).copied().unwrap_or(true)
}

fn mtp_mode(root: &Path, model: &Installed, on: bool) -> Mtp {
    if !on {
        Mtp::Off
    } else if let Some(file) = store::mtp_file(root, model) {
        Mtp::Draft(file)
    } else if model.manifest.has_builtin_mtp {
        Mtp::Builtin
    } else {
        Mtp::Off
    }
}
