//! 每個本地模型一個 llama-server 子程序：按需啟動、失敗重試，依設定收掉或重啟。

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Instant;

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

/// 連續啟動失敗幾次後停在失敗狀態。
const MAX_FAILURES: u32 = 2;

/// 所有模型的子程序，以模型 id 為鍵。
#[derive(Default)]
pub(super) struct Servers(Mutex<BTreeMap<String, Arc<Slot>>>);

#[derive(Default)]
struct Slot {
    /// 讓同一模型的並發啟動只跑一次。
    start: tokio::sync::Mutex<()>,
    state: Mutex<SlotState>,
}

#[derive(Default)]
struct SlotState {
    running: Option<Running>,
    failures: u32,
    error: String,
    /// `stop` 時遞增，讓進行中的啟動知道結果已作廢。
    epoch: u64,
}

struct Running {
    process: Box<dyn Process>,
    base_url: String,
}

impl Servers {
    fn slot(&self, id: &str) -> Arc<Slot> {
        lock(&self.0).entry(id.to_owned()).or_default().clone()
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
        let epoch = {
            let mut st = lock(&slot.state);
            if let Some(running) = st.running.as_mut() {
                let Some(code) = running.process.exited() else {
                    return Ok(running.base_url.clone());
                };
                log(format!(
                    "[local] llama-server[{model_id}] exited unexpectedly code={code}: {}",
                    running.process.tail().join(" | ")
                ));
                st.running = None;
            }
            if st.failures >= MAX_FAILURES {
                return Err(st.error.clone());
            }
            st.epoch
        };
        let result = self.start(model_id).await;
        let mut st = lock(&slot.state);
        match result {
            Ok(mut running) if st.epoch != epoch => {
                log(format!(
                    "[local] llama-server[{model_id}] was stopped while starting"
                ));
                running.process.kill();
                Err(format!("model {model_id} was unloaded while starting"))
            }
            Ok(running) => {
                st.failures = 0;
                let url = running.base_url.clone();
                st.running = Some(running);
                Ok(url)
            }
            Err(e) => {
                st.failures += 1;
                st.error = e.clone();
                Err(e)
            }
        }
    }

    /// 停掉模型的 llama-server；沒在跑則無動作。
    pub fn stop(&self, model_id: &str) {
        let slot = lock(&self.servers.0).get(model_id).cloned();
        if let Some(slot) = slot {
            stop_slot(model_id, &slot);
        }
    }

    /// 停掉所有 llama-server。
    pub fn shutdown(&self) {
        for (id, slot) in self.servers.all() {
            stop_slot(&id, &slot);
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

    async fn start(&self, id: &str) -> Result<Running, String> {
        let model = store::list(&self.root)
            .into_iter()
            .find(|m| m.id == id && m.complete)
            .ok_or_else(|| format!("model {id} is not downloaded"))?;
        (self.notify)(LocalNotice::Loading(Some(display_name(&model.manifest))));
        let result = self.start_with_fallbacks(&model).await;
        (self.notify)(LocalNotice::Loading(None));
        result
    }

    /// 失敗時先關 MTP 重試，再對尚未確認的新引擎回退重試；新引擎首次就緒即確認。
    async fn start_with_fallbacks(&self, model: &Installed) -> Result<Running, String> {
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
            let err = match self.launch(&plan).await {
                Ok(running) => {
                    if active.previous.is_some()
                        && let Err(e) = engine::confirm(&self.root)
                    {
                        log(format!("[local] confirming engine failed: {e}"));
                    }
                    return Ok(running);
                }
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

    async fn launch(&self, plan: &ServerPlan) -> Result<Running, String> {
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
        server::wait_ready(
            &hf_client(&base_url),
            &base_url,
            process.as_mut(),
            READY_TIMEOUT,
        )
        .await?;
        log(format!(
            "[local] llama-server[{}] ready on port {port} in {:.1}s",
            plan.model_id,
            started.elapsed().as_secs_f32()
        ));
        Ok(Running { process, base_url })
    }
}

fn stop_slot(id: &str, slot: &Slot) {
    let mut st = lock(&slot.state);
    st.epoch += 1;
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

fn mtp_mode(root: &std::path::Path, model: &Installed, on: bool) -> Mtp {
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
