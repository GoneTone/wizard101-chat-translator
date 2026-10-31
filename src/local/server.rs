//! 啟動並監看 `llama-server.exe` 子程序，等它就緒。

use crate::local::hardware::{self, Gpu, detect_gpu};
use crate::local::usage::{self, Reading};
use crate::logs::log;
use crate::util::{Clock, poll_until};
use std::collections::VecDeque;
use std::io::{self, BufRead, BufReader};
use std::net::TcpListener;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject,
};

/// 正式的就緒等待上限（大模型載入 VRAM 要一陣子）。
pub const READY_TIMEOUT: Duration = Duration::from_secs(180);

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const POLL_INTERVAL: Duration = Duration::from_millis(250);
const TAIL_LINES: usize = 20;
const LIST_DEVICES_TIMEOUT: Duration = Duration::from_secs(15);

/// Multi-token prediction 的使用方式。
#[derive(Clone, Debug, PartialEq)]
pub enum Mtp {
    Off,
    Builtin,
    Draft(PathBuf),
}

/// 啟動一個 llama-server 所需的全部資訊。
#[derive(Clone, Debug, PartialEq)]
pub struct ServerPlan {
    pub model_id: String,
    pub exe: PathBuf,
    pub model: PathBuf,
    pub mtp: Mtp,
    pub idle_minutes: u32,
    /// `--cache-ram`（MiB）；0 為關閉。
    pub prompt_cache_mib: u32,
}

/// 組出 llama-server 的命令列參數。
pub fn args(plan: &ServerPlan, port: u16) -> Vec<String> {
    let mut a: Vec<String> = vec![
        "-m".into(),
        plan.model.to_string_lossy().into_owned(),
        "--host".into(),
        "127.0.0.1".into(),
        "--port".into(),
        port.to_string(),
        "-c".into(),
        "8192".into(),
        "--parallel".into(),
        "1".into(),
        // libllama 的 INFO（含 `load_tensors: offloaded`）要 verbosity 4 才印
        "-lv".into(),
        "4".into(),
        "--cache-ram".into(),
        plan.prompt_cache_mib.to_string(),
    ];
    if plan.idle_minutes > 0 {
        a.push("--sleep-idle-seconds".into());
        a.push((plan.idle_minutes * 60).to_string());
    }
    match &plan.mtp {
        Mtp::Off => {}
        Mtp::Builtin => a.extend(["--spec-type".into(), "draft-mtp".into()]),
        Mtp::Draft(p) => a.extend([
            "--spec-type".into(),
            "draft-mtp".into(),
            "--spec-draft-model".into(),
            p.to_string_lossy().into_owned(),
        ]),
    }
    a
}

/// 向系統要一個目前空閒的本機埠。
pub fn free_port() -> io::Result<u16> {
    Ok(TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

/// 執行中的 llama-server 子程序。
pub trait Process: Send {
    /// 已結束則回傳結束碼。
    fn exited(&mut self) -> Option<i32>;
    fn kill(&mut self);
    /// 最近輸出的幾行，錯誤訊息用。
    fn tail(&self) -> Vec<String>;
    /// 子程序的 PID，量它實際佔用的記憶體用。
    fn pid(&self) -> Option<u32> {
        None
    }
    /// 每一層都放在 GPU 上；還沒看到 llama.cpp 回報為 `None`。
    fn all_on_gpu(&self) -> Option<bool> {
        None
    }
    /// 這次載入以記憶體對映放在系統記憶體的權重（llama.cpp 的 `CPU_Mapped` buffer，位元組）；不在 private working set 裡。
    fn mapped(&self) -> u64 {
        0
    }
    /// 模型不在記憶體裡（睡眠中，或醒來重新載入還沒完成），這時量到的不是模型的佔用。
    fn unloaded(&self) -> bool {
        false
    }
}

/// 啟動 llama-server 與偵測硬體的方式（測試以假實作替換）。
pub trait Launcher: Send + Sync {
    fn launch(&self, plan: &ServerPlan, port: u16) -> io::Result<Box<dyn Process>>;

    /// 偵測 GPU（見 [`detect_gpu`]）。
    fn detect_gpu(&self) -> Option<Gpu> {
        detect_gpu()
    }

    /// 顯示卡的專用記憶體（見 [`hardware::adapter_memory`]）。
    fn adapter_memory(&self, gpu: &Gpu) -> Option<(u64, u64)> {
        hardware::adapter_memory(gpu)
    }

    /// 執行 `llama-server --list-devices`（有逾時），回傳 stdout 與 stderr 的內容。
    fn list_devices(&self, exe: &Path) -> Result<String, String> {
        run_list_devices(exe)
    }

    /// 這些 llama-server 實際佔用的記憶體（見 [`usage::process_usage`]）。
    fn process_usage(&self, pids: &[u32], integrated: bool) -> Reading {
        usage::process_usage(pids, integrated)
    }
}

/// 真的以 `Command` 啟動子程序。
pub struct RealLauncher;

struct RealProcess {
    child: Child,
    tail: Arc<Mutex<VecDeque<String>>>,
    output: Arc<Mutex<Output>>,
    readers: Vec<JoinHandle<()>>,
}

impl RealProcess {
    /// 程序已結束、管線到 EOF，等讀取執行緒把最後幾行寫進 `tail`。
    fn drain_readers(&mut self) {
        for r in self.readers.drain(..) {
            let _ = r.join();
        }
    }
}

impl Drop for RealProcess {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            self.kill();
        }
    }
}

impl Process for RealProcess {
    fn exited(&mut self) -> Option<i32> {
        let code = self.child.try_wait().ok().flatten()?.code().unwrap_or(-1);
        self.drain_readers();
        Some(code)
    }
    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.drain_readers();
    }
    fn tail(&self) -> Vec<String> {
        self.tail.lock().unwrap().iter().cloned().collect()
    }
    fn pid(&self) -> Option<u32> {
        Some(self.child.id())
    }
    fn all_on_gpu(&self) -> Option<bool> {
        self.output.lock().unwrap().all_on_gpu()
    }
    fn mapped(&self) -> u64 {
        self.output.lock().unwrap().mapped
    }
    fn unloaded(&self) -> bool {
        self.output.lock().unwrap().unloaded
    }
}

impl Launcher for RealLauncher {
    fn launch(&self, plan: &ServerPlan, port: u16) -> io::Result<Box<dyn Process>> {
        let mut child = Command::new(&plan.exe)
            .args(args(plan, port))
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        log(format!(
            "[local] llama-server[{}] started pid={} port={port}",
            plan.model_id,
            child.id()
        ));
        assign_to_job(&child, &plan.model_id);
        let tail = Arc::new(Mutex::new(VecDeque::new()));
        let output = Arc::new(Mutex::new(Output::default()));
        let mut readers = Vec::new();
        if let Some(out) = child.stdout.take() {
            let (id, t, o) = (plan.model_id.clone(), tail.clone(), output.clone());
            readers.push(pump_lines(out, id, t, o));
        }
        if let Some(err) = child.stderr.take() {
            let (id, t, o) = (plan.model_id.clone(), tail.clone(), output.clone());
            readers.push(pump_lines(err, id, t, o));
        }
        Ok(Box::new(RealProcess {
            child,
            tail,
            output,
            readers,
        }))
    }
}

/// 每行交給 `output` 判讀、留最後 `TAIL_LINES` 行，需要的才寫進 log；以位元組讀取，非 UTF-8 輸出不會讓執行緒死掉。
fn pump_lines(
    pipe: impl io::Read + Send + 'static,
    model_id: String,
    tail: Arc<Mutex<VecDeque<String>>>,
    output: Arc<Mutex<Output>>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        for raw in BufReader::new(pipe).split(b'\n') {
            let Ok(raw) = raw else { break };
            let line = String::from_utf8_lossy(&raw).trim_end().to_string();
            if output.lock().unwrap().observe(&line) {
                log(format!("[local] llama-server[{model_id}]: {line}"));
            }
            let mut t = tail.lock().unwrap();
            if t.len() == TAIL_LINES {
                t.pop_front();
            }
            t.push_back(line);
        }
    })
}

/// 從 llama-server 輸出讀到的載入狀態。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Output {
    /// 這次載入每層都放 GPU（`load_tensors: offloaded n/n`）；還沒看到為 `None`。
    layers_on_gpu: Option<bool>,
    /// 這次載入以記憶體對映留在系統記憶體的權重（`CPU_Mapped` buffer 加總，位元組）；不在 working set 的私有部分裡。
    mapped: u64,
    /// `--fit` 把層或張量（例如 MoE 專家）移到系統記憶體；在同一個程序裡跨重新載入保留。
    ///
    /// 醒來重新載入沿用第一次 fit 寫回的參數，第二次 fit 只會報 `already set by user` 失敗、不再印
    /// `cannot meet`，擺放卻照舊；所以只有之後的 fit 明說不必調整時才清掉。
    fit_moved: bool,
    /// 這一次 fit 有裝置要調整；多顯示卡時其他裝置說不必調整也不清 `fit_moved`。
    fit_run_moved: bool,
    /// 模型不在記憶體裡（睡眠中，或醒來重新載入還沒完成）。
    unloaded: bool,
    /// 載入中（啟動到 `listening on`、醒來到重新載入完成）：這段每行都寫進 log。
    loading: bool,
    /// 醒來後還沒看到 `--fit` 的第一行。
    fit_pending: bool,
    /// 上一行有沒有寫進 log；沒有時間戳記的續行跟著它。
    last_logged: bool,
}

impl Default for Output {
    fn default() -> Self {
        Self {
            layers_on_gpu: None,
            mapped: 0,
            fit_moved: false,
            fit_run_moved: false,
            unloaded: false,
            loading: true,
            fit_pending: false,
            last_logged: true,
        }
    }
}

impl Output {
    /// 每層都放 GPU，且 `--fit` 沒把東西移到系統記憶體；還不知道為 `None`。
    fn all_on_gpu(&self) -> Option<bool> {
        self.layers_on_gpu.map(|on| on && !self.fit_moved)
    }

    /// 讀一行輸出更新狀態；回傳這行要不要寫進 log。
    fn observe(&mut self, line: &str) -> bool {
        let logged = self.update(line);
        self.last_logged = logged;
        logged
    }

    fn update(&mut self, line: &str) -> bool {
        let logged = self.loading || self.important(line);
        if line.contains("server is entering sleeping state") {
            self.unloaded = true;
            return true;
        }
        if line.contains("server is exiting sleeping state") {
            (self.layers_on_gpu, self.mapped) = (None, 0);
            (self.unloaded, self.fit_run_moved) = (true, false);
            (self.loading, self.fit_pending) = (true, true);
            return true;
        }
        if line.contains("failed to reload model after sleeping") {
            (self.loading, self.fit_pending) = (false, false);
            return true;
        }
        if self.loading && !self.unloaded && line.contains("listening on") {
            self.loading = false;
        }
        // 醒來重新載入時 `load_model` 最後印 n_slots；沒看到就以第一個請求的速度統計為準
        if self.unloaded && (line.contains("n_slots =") || line.contains("print_timing")) {
            (self.loading, self.unloaded) = (false, false);
        }
        if self.fit_pending && line.contains("common_params_fit_impl:") {
            (self.layers_on_gpu, self.mapped, self.fit_pending) = (None, 0, false);
        }
        if line.contains("getting device memory data for initial parameters") {
            self.fit_run_moved = false;
        }
        if line.contains("cannot meet free memory target") {
            (self.fit_moved, self.fit_run_moved) = (true, true);
        } else if !self.fit_run_moved
            && (line.contains("can be met on all devices")
                || line.contains("of free device memory, no changes needed"))
        {
            self.fit_moved = false;
        }
        self.layers_on_gpu = track_offload(self.layers_on_gpu, line);
        self.mapped += mapped_bytes(line);
        logged
    }

    /// 載入以外的時間要寫進 log 的行：警告與錯誤（時間戳記後的等級字母 W／E）與每次請求的 `print_timing`；
    /// 沒有時間戳記的是上一筆的續行，跟著上一行；但像當機訊息的一律寫（`GGML_ASSERT` 等直接寫 stderr）。
    fn important(&self, line: &str) -> bool {
        static LEVEL: LazyLock<regex::Regex> =
            LazyLock::new(|| regex::Regex::new(r"^\d+\.\d+\.\d+\.\d+ ([A-Z]) ").unwrap());
        static CRASH: LazyLock<regex::Regex> =
            LazyLock::new(|| regex::Regex::new(r"(?i)error|assert|abort|exception").unwrap());
        match LEVEL.captures(line) {
            Some(c) => matches!(&c[1], "W" | "E") || line.contains("print_timing"),
            None => !line.is_empty() && (self.last_logged || CRASH.is_match(line)),
        }
    }
}

/// `load_tensors:   CPU_Mapped model buffer size = X MiB` 的大小；其他行為 0。
///
/// 只算對映檔案的 buffer：其他 buffer（CUDA_Host、CPU_REPACK 等）是私有記憶體，已在 private working set 裡。
fn mapped_bytes(line: &str) -> u64 {
    static LINE: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"load_tensors:\s+CPU_Mapped model buffer size =\s+([\d.]+) MiB").unwrap()
    });
    LINE.captures(line)
        .and_then(|c| c[1].parse::<f64>().ok())
        .map_or(0, |mib| (mib * (1u64 << 20) as f64) as u64)
}

/// 依一行輸出更新「每層都在 GPU」：看到 `load_tensors: offloaded n/m layers to GPU` 才有值，任一次 n < m 就是 `false`。
///
/// 主模型與 MTP 草稿模型各印一次。
fn track_offload(layers_on_gpu: Option<bool>, line: &str) -> Option<bool> {
    static LINE: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"load_tensors: offloaded (\d+)/(\d+) layers to GPU").unwrap()
    });
    let Some(c) = LINE.captures(line) else {
        return layers_on_gpu;
    };
    match (c[1].parse::<u32>(), c[2].parse::<u32>()) {
        (Ok(n), Ok(m)) => Some(layers_on_gpu.unwrap_or(true) && n >= m),
        _ => layers_on_gpu,
    }
}

/// 全程序共用的 Job Object；handle 刻意不關，程式結束時系統關閉它，順帶殺掉所有子程序。
fn job() -> Option<isize> {
    static JOB: OnceLock<Option<isize>> = OnceLock::new();
    *JOB.get_or_init(|| unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            log(format!(
                "[local] CreateJobObjectW failed: {}",
                io::Error::last_os_error()
            ));
            return None;
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        if ok == 0 {
            log(format!(
                "[local] SetInformationJobObject failed: {}",
                io::Error::last_os_error()
            ));
            return None;
        }
        Some(job as isize)
    })
}

fn assign_to_job(child: &Child, model_id: &str) {
    let Some(job) = job() else { return };
    let ok = unsafe { AssignProcessToJobObject(job as _, child.as_raw_handle() as _) };
    if ok == 0 {
        log(format!(
            "[local] llama-server[{model_id}] job assign failed: {}",
            io::Error::last_os_error()
        ));
    }
}

/// 每 250 ms 輪詢 `/health` 直到 200；程序先結束或逾時（會先 kill）都回含輸出尾段的錯誤。
pub async fn wait_ready(
    http: &reqwest::Client,
    base_url: &str,
    process: &mut dyn Process,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let url = format!("{base_url}/health");
    loop {
        if let Some(code) = process.exited() {
            let tail = process.tail().join("\n");
            log(format!("[local] llama-server exited early code={code}"));
            return Err(format!("llama-server exited with code {code}:\n{tail}"));
        }
        if let Ok(resp) = http.get(&url).send().await
            && resp.status().is_success()
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            process.kill();
            let tail = process.tail().join("\n");
            log(format!(
                "[local] llama-server not ready after {}s, killed",
                timeout.as_secs()
            ));
            return Err(format!(
                "llama-server did not become ready within {}s:\n{tail}",
                timeout.as_secs()
            ));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

fn run_list_devices(exe: &Path) -> Result<String, String> {
    let mut child = std::process::Command::new(exe)
        .arg("--list-devices")
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let pipes: [Option<Box<dyn io::Read + Send>>; 2] = [
        child.stdout.take().map(|p| Box::new(p) as _),
        child.stderr.take().map(|p| Box::new(p) as _),
    ];
    let readers: Vec<_> = pipes
        .into_iter()
        .flatten()
        .map(|mut pipe| {
            std::thread::spawn(move || {
                let mut buf = Vec::new();
                let _ = pipe.read_to_end(&mut buf);
                String::from_utf8_lossy(&buf).into_owned()
            })
        })
        .collect();
    let deadline = Instant::now() + LIST_DEVICES_TIMEOUT;
    let status = poll_until(
        &Clock::default(),
        deadline,
        Duration::from_millis(100),
        || child.try_wait().ok().flatten(),
    );
    if status.is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let output: Vec<String> = readers.into_iter().filter_map(|r| r.join().ok()).collect();
    let output = output.join("\n");
    match status {
        None => Err(format!(
            "did not finish within {}s",
            LIST_DEVICES_TIMEOUT.as_secs()
        )),
        Some(s) if !s.success() => Err(format!("exited with {s}:\n{output}")),
        Some(_) => Ok(output),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fold(lines: &[&str]) -> Option<bool> {
        lines
            .iter()
            .fold(None, |all, line| track_offload(all, line))
    }

    /// 依序餵 `lines`，回傳最後的狀態與每行有沒有寫進 log。
    fn feed(lines: &[&str]) -> (Output, Vec<bool>) {
        let mut out = Output::default();
        let logged = lines.iter().map(|l| out.observe(l)).collect();
        (out, logged)
    }

    /// 醒來重新載入：重新判斷是否全放 GPU，載入完成前算卸下；睡眠中也算卸下。
    #[test]
    fn sleeping_and_waking_reset_the_load_state() {
        let main = "0.39.819.281 I load_tensors: offloaded 43/43 layers to GPU";
        let (out, _) = feed(&[
            main,
            "1.00.000.000 I srv  server_on_li: listening on http://127.0.0.1:1",
        ]);
        assert_eq!((out.all_on_gpu(), out.unloaded), (Some(true), false));
        let asleep = [
            main,
            "1.00.000.000 I srv  server_on_li: listening on http://127.0.0.1:1",
            "9.00.000.000 I srv  handle_sleep: server is entering sleeping state",
        ];
        assert!(feed(&asleep).0.unloaded);
        let mut waking = asleep.to_vec();
        waking.push("9.10.000.000 I srv  handle_sleep: server is exiting sleeping state");
        let (out, _) = feed(&waking);
        assert_eq!(
            (out.all_on_gpu(), out.unloaded),
            (None, true),
            "reload pending"
        );
        waking.extend([
            "9.10.100.000 I common_params_fit_impl: cannot meet free memory target of 1024 MiB, need to reduce device memory by 512 MiB",
            main,
            "9.20.000.000 I srv    load_model: initializing, n_slots = 1, n_ctx_slot = 8192, kv_unified = 'false'",
        ]);
        let (out, _) = feed(&waking);
        assert_eq!((out.all_on_gpu(), out.unloaded), (Some(false), false));
        let mut failed = asleep.to_vec();
        failed.extend([
            "9.10.000.000 I srv  handle_sleep: server is exiting sleeping state",
            "9.10.100.000 E srv  handle_sleep: failed to reload model after sleeping",
        ]);
        assert!(feed(&failed).0.unloaded);
    }

    /// 載入期間每行都寫；其餘時間只寫 W／E、`print_timing` 與睡眠切換。
    #[test]
    fn only_loading_warnings_and_timings_reach_the_log() {
        let lines = [
            "0.00.100.000 I srv    load_model: loading model 'm.gguf'",
            "0.39.341.105 I srv  server_on_li: listening on http://127.0.0.1:1",
            "0.40.000.000 I slot params_from_: id  0 | task 0 | sampler params: temp 0.8",
            "0.40.000.001 W srv  some warning",
            "0.40.000.002 E srv  some error",
            "0.40.843.564 I slot print_timing: id  0 | task 0 | prompt eval time = 1.0 ms",
            "9.00.000.000 I srv  handle_sleep: server is entering sleeping state",
            "9.10.000.000 I srv  handle_sleep: server is exiting sleeping state",
            "9.10.000.001 I load_tensors: offloaded 43/43 layers to GPU",
            "9.20.000.000 I srv    load_model: initializing, n_slots = 1",
            "9.30.000.000 I slot params_from_: id  0 | task 1 | sampler params: temp 0.8",
        ];
        let (_, logged) = feed(&lines);
        assert_eq!(
            logged,
            [
                true, true, false, true, true, true, true, true, true, true, false
            ]
        );
    }

    #[test]
    fn offloaded_lines_decide_all_on_gpu() {
        let main = "0.39.819.281 I load_tensors: offloaded 43/43 layers to GPU";
        let draft = "0.41.002.100 I load_tensors: offloaded 5/5 layers to GPU";
        assert_eq!(fold(&[main, draft]), Some(true));
        let partial = "0.39.819.281 I load_tensors: offloaded 30/43 layers to GPU";
        assert_eq!(fold(&[partial, draft]), Some(false));
        assert_eq!(fold(&[draft, partial, main]), Some(false), "stays false");
        let dry_run = [
            "common_memory_breakdown_print: | memory breakdown [MiB] | total   free    self   model   context   compute    unaccounted |",
            "common_params_fit_impl: projected to use 3467 MiB of device memory vs. 15210 MiB of free device memory",
        ];
        assert_eq!(fold(&dry_run), None);
        let moved = [
            "common_params_fit_impl: cannot meet free memory target of 1024 MiB, need to reduce device memory by 2048 MiB",
            "common_params_fit_impl: getting device memory data with all MoE tensors moved to system memory",
            main,
        ];
        assert_eq!(
            feed(&moved).0.all_on_gpu(),
            Some(false),
            "--fit moved tensors off the GPU"
        );
        let kept = [
            "common_params_fit_impl: targets for free memory can be met on all devices, no changes needed",
            main,
            draft,
        ];
        assert_eq!(feed(&kept).0.all_on_gpu(), Some(true));
    }

    /// 醒來重新載入時第二次 fit 只會報參數已設定而失敗，擺放沿用第一次：仍算沒全放 GPU，直到之後的 fit 明說不必調整。
    #[test]
    fn a_fit_move_survives_reloads_until_a_fit_needs_no_change() {
        let main = "0.39.819.281 I load_tensors: offloaded 43/43 layers to GPU";
        let mut lines = vec![
            "0.01.000.000 I common_params_fit_impl: cannot meet free memory target of 1024 MiB, need to reduce device memory by 2048 MiB",
            main,
            "0.39.341.105 I srv  server_on_li: listening on http://127.0.0.1:1",
            "9.00.000.000 I srv  handle_sleep: server is entering sleeping state",
            "9.10.000.000 I srv  handle_sleep: server is exiting sleeping state",
            "9.10.000.100 W common_init_result: failed to fit params to free device memory: model_params::tensor_split already set by user, abort",
            main,
            "9.20.000.000 I srv    load_model: initializing, n_slots = 1",
        ];
        assert_eq!(feed(&lines).0.all_on_gpu(), Some(false));
        lines.extend([
            "19.00.000.000 I srv  handle_sleep: server is entering sleeping state",
            "19.10.000.000 I srv  handle_sleep: server is exiting sleeping state",
            "19.10.000.100 I common_params_fit_impl: targets for free memory can be met on all devices, no changes needed",
            main,
            "19.20.000.000 I srv    load_model: initializing, n_slots = 1",
        ]);
        assert_eq!(feed(&lines).0.all_on_gpu(), Some(true));
    }

    /// 沒有時間戳記的續行跟著上一行寫或不寫。
    #[test]
    fn continuation_lines_follow_the_previous_line() {
        let lines = [
            "0.39.341.105 I srv  server_on_li: listening on http://127.0.0.1:1",
            "0.40.000.000 I slot params_from_: sampler chain:",
            "  logits -> top-k -> dist",
            "0.40.000.001 W srv  something odd:",
            "  detail of the warning",
            "",
        ];
        let (_, logged) = feed(&lines);
        assert_eq!(logged, [true, false, false, true, true, false]);
    }

    /// 當機訊息直接寫 stderr、沒有時間戳記：就算上一行沒寫進 log 也要寫。
    #[test]
    fn crash_messages_without_a_timestamp_are_logged() {
        let lines = [
            "0.39.341.105 I srv  server_on_li: listening on http://127.0.0.1:1",
            "0.40.000.000 I slot params_from_: sampler chain:",
            "D:/a/llama.cpp/ggml/src/ggml-cuda.cu:88: GGML_ASSERT(ptr) failed",
            "CUDA error: out of memory",
        ];
        let (_, logged) = feed(&lines);
        assert_eq!(logged, [true, false, true, true]);
    }

    /// 以記憶體對映留在系統記憶體的權重：主模型與草稿模型的 `CPU_Mapped` 行加總，其他 buffer 不算；醒來重新載入重算。
    #[test]
    fn mapped_buffers_add_up_per_load() {
        const MIB: u64 = 1 << 20;
        let load = [
            "0.29.791.778 I load_tensors: offloaded 43/43 layers to GPU",
            "0.29.791.782 I load_tensors:   CPU_Mapped model buffer size =  1848.00 MiB",
            "0.29.791.784 I load_tensors:   CPU_Mapped model buffer size =   440.00 MiB",
            "0.29.791.784 I load_tensors:        CUDA0 model buffer size =  3025.74 MiB",
            "0.34.667.604 I load_tensors:   CPU_Mapped model buffer size =    68.50 MiB",
            "0.36.341.800 I sched_reserve:  CUDA_Host compute buffer size =    50.03 MiB",
            "0.39.341.105 I srv  server_on_li: listening on http://127.0.0.1:1",
        ];
        let mut lines = load.to_vec();
        assert_eq!(
            feed(&lines).0.mapped,
            (1848 + 440) * MIB + 68 * MIB + MIB / 2
        );
        lines.extend([
            "9.00.000.000 I srv  handle_sleep: server is entering sleeping state",
            "9.10.000.000 I srv  handle_sleep: server is exiting sleeping state",
        ]);
        assert_eq!(feed(&lines).0.mapped, 0, "reload pending");
        lines.extend(&load[..3]);
        assert_eq!(feed(&lines).0.mapped, (1848 + 440) * MIB);
    }

    /// 同一次 fit 裡只要有裝置要調整，另一個裝置說不必調整也不算沒移動（多顯示卡）。
    #[test]
    fn one_device_needing_no_change_does_not_clear_a_move_in_the_same_fit() {
        let lines = [
            "0.01.000.000 I common_params_fit_impl: getting device memory data for initial parameters:",
            "0.01.100.000 I common_params_fit_impl: cannot meet free memory target of 1024 MiB, need to reduce device memory by 2048 MiB",
            "0.01.200.000 I common_params_fit_impl: will leave 9000 >= 1024 MiB of free device memory, no changes needed",
            "0.39.819.281 I load_tensors: offloaded 43/43 layers to GPU",
        ];
        assert_eq!(feed(&lines).0.all_on_gpu(), Some(false));
    }
}
