//! 每個本地模型一個 llama-server 子程序：按需啟動、失敗重試，依設定收掉或重啟。

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio_util::sync::CancellationToken;

use super::LocalNotice;
use super::catalog::RECOMMENDED;
use super::engine;
use super::hf::hf_client;
use super::hub::{EngineState, Local};
use super::server::{self, Mtp, Process, READY_TIMEOUT, ServerPlan};
use super::store::{self, Installed, Manifest};
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
    start: tokio::sync::Mutex<()>,
    state: Mutex<SlotState>,
}

#[derive(Default)]
struct SlotState {
    running: Option<Running>,
    failures: u32,
    error: String,
    /// 進行中啟動的取消鈕；`stop` 按下後該次啟動會殺掉子程序並作廢。
    starting: CancellationToken,
}

impl SlotState {
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
        let _start = slot.start.lock().await;
        let cancel = {
            let mut st = lock(&slot.state);
            if let Some(running) = st.running.as_mut() {
                let Some(code) = running.process.exited() else {
                    return Ok(running.base_url.clone());
                };
                let tail = running.process.tail().join("\n");
                log(format!(
                    "[local] llama-server[{model_id}] exited unexpectedly code={code}"
                ));
                st.running = None;
                st.fail(
                    model_id,
                    format!("llama-server exited unexpectedly with code {code}:\n{tail}"),
                );
            }
            if st.failures >= MAX_FAILURES {
                return Err(st.error.clone());
            }
            st.starting = CancellationToken::new();
            st.starting.clone()
        };
        // 前置條件不符（還沒下載完、沒有引擎）不算失敗：下載或安裝完就該能直接啟動
        let model = store::list(&self.root)
            .into_iter()
            .find(|m| m.id == model_id && m.complete)
            .ok_or_else(|| format!("model {model_id} is not downloaded"))?;
        if engine::read_active(&self.root).current.is_none() {
            return Err("llama.cpp engine is not installed".into());
        }
        let result = self.start(&model, &cancel).await;
        let mut st = lock(&slot.state);
        match result {
            Ok(mut running) if cancel.is_cancelled() => {
                running.process.kill();
                Err(stopped_error(model_id))
            }
            Ok(running) => {
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

    /// 停掉所有 llama-server。
    pub fn shutdown(&self) {
        for (id, slot) in self.servers.all() {
            stop_slot(&id, &slot);
        }
    }

    /// `stop` 後等進行中的啟動收尾，回來時子程序一定已結束、檔案已放開。
    pub(super) async fn stop_and_wait(&self, model_id: &str) {
        if let Some(slot) = self.servers.get(model_id) {
            stop_slot(model_id, &slot);
            drop(slot.start.lock().await);
        }
    }

    /// `shutdown` 後等所有進行中的啟動收尾。
    pub(super) async fn shutdown_and_wait(&self) {
        for (id, slot) in self.servers.all() {
            stop_slot(&id, &slot);
            drop(slot.start.lock().await);
        }
    }

    /// 清掉模型的失敗紀錄（例如下載完成後）。
    pub(super) fn clear_failures(&self, model_id: &str) {
        if let Some(slot) = self.servers.get(model_id) {
            lock(&slot.state).failures = 0;
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
                lock(&slot.state).failures = 0;
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
        self.shutdown();
        self.ensure_in_order(running);
    }

    /// 在背景依序啟動：前一個就緒（或失敗）才啟動下一個，讓 `--fit` 看到已佔用的記憶體。
    fn ensure_in_order(&self, ids: Vec<String>) {
        if ids.is_empty() {
            return;
        }
        let me = self.arc();
        self.rt.spawn(async move {
            for id in ids {
                if let Err(e) = me.ensure(&id).await {
                    log(format!("[local] preloading {id} failed: {e}"));
                }
            }
        });
    }

    async fn start(
        &self,
        model: &Installed,
        cancel: &CancellationToken,
    ) -> Result<Running, String> {
        (self.notify)(LocalNotice::Loading(Some(display_name(&model.manifest))));
        let result = self.start_with_fallbacks(model, cancel).await;
        (self.notify)(LocalNotice::Loading(None));
        result
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
            let plan = ServerPlan {
                model_id: model.id.clone(),
                exe,
                model: store::main_file(&self.root, model),
                mtp: mtp.clone(),
                idle_minutes: settings.idle_minutes,
            };
            let err = match self.launch(&plan, cancel).await {
                Ok(running) => {
                    if active.previous.is_some()
                        && let Err(e) = engine::confirm(&self.root)
                    {
                        log(format!("[local] confirming engine failed: {e}"));
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
        Ok(Running { process, base_url })
    }
}

fn stopped_error(id: &str) -> String {
    format!("model {id} was unloaded while starting")
}

fn stop_slot(id: &str, slot: &Slot) {
    let mut st = lock(&slot.state);
    st.starting.cancel();
    st.failures = 0;
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

/// 載入提示用的顯示名：「推薦清單名稱或 repo 名 · 量化」。
fn display_name(m: &Manifest) -> String {
    let name = RECOMMENDED
        .iter()
        .find(|r| r.repo == m.repo)
        .map_or_else(|| m.repo.rsplit('/').next().unwrap_or_default(), |r| r.name);
    format!("{name} · {}", m.quant)
}
