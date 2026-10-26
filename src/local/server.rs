//! 啟動並監看 `llama-server.exe` 子程序，等它就緒。

use crate::local::hardware::{Gpu, detect_gpu};
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
        self.output.lock().unwrap().all_on_gpu
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
    /// 這次載入每層都放 GPU；還沒看到 `load_tensors: offloaded` 為 `None`。
    all_on_gpu: Option<bool>,
    /// 模型不在記憶體裡（睡眠中，或醒來重新載入還沒完成）。
    unloaded: bool,
    /// 載入中（啟動到 `listening on`、醒來到重新載入完成）：這段每行都寫進 log。
    loading: bool,
    /// 醒來後還沒看到 `--fit` 的第一行。
    fit_pending: bool,
}

impl Default for Output {
    fn default() -> Self {
        Self {
            all_on_gpu: None,
            unloaded: false,
            loading: true,
            fit_pending: false,
        }
    }
}

impl Output {
    /// 讀一行輸出更新狀態；回傳這行要不要寫進 log。
    fn observe(&mut self, line: &str) -> bool {
        let logged = self.loading || important(line);
        if line.contains("server is entering sleeping state") {
            self.unloaded = true;
            return true;
        }
        if line.contains("server is exiting sleeping state") {
            *self = Self {
                all_on_gpu: None,
                unloaded: true,
                loading: true,
                fit_pending: true,
            };
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
            (self.all_on_gpu, self.fit_pending) = (None, false);
        }
        self.all_on_gpu = track_offload(self.all_on_gpu, line);
        logged
    }
}

/// 載入以外的時間要寫進 log 的行：警告與錯誤（時間戳記後的等級字母 W／E）、每次請求的 `print_timing`，以及認不出格式的。
fn important(line: &str) -> bool {
    static LEVEL: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"^\d+\.\d+\.\d+\.\d+ ([A-Z]) ").unwrap());
    match LEVEL.captures(line) {
        Some(c) => matches!(&c[1], "W" | "E") || line.contains("print_timing"),
        None => !line.is_empty(),
    }
}

/// 依一行輸出更新「每層都在 GPU」：看到 `load_tensors: offloaded n/m layers to GPU` 才有值，任一次 n < m 就是 `false`。
///
/// 主模型與 MTP 草稿模型各印一次。`--fit` 把張量（例如 MoE 專家）移到系統記憶體時 offloaded 仍是 n/n，
/// 但會先印 `cannot meet free memory target`，看到它也是 `false`。
fn track_offload(all_on_gpu: Option<bool>, line: &str) -> Option<bool> {
    if line.contains("cannot meet free memory target") {
        return Some(false);
    }
    static LINE: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"load_tensors: offloaded (\d+)/(\d+) layers to GPU").unwrap()
    });
    let Some(c) = LINE.captures(line) else {
        return all_on_gpu;
    };
    match (c[1].parse::<u32>(), c[2].parse::<u32>()) {
        (Ok(n), Ok(m)) => Some(all_on_gpu.unwrap_or(true) && n >= m),
        _ => all_on_gpu,
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
        assert_eq!((out.all_on_gpu, out.unloaded), (Some(true), false));
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
            (out.all_on_gpu, out.unloaded),
            (None, true),
            "reload pending"
        );
        waking.extend([
            "9.10.100.000 I common_params_fit_impl: cannot meet free memory target of 1024 MiB, need to reduce device memory by 512 MiB",
            main,
            "9.20.000.000 I srv    load_model: initializing, n_slots = 1, n_ctx_slot = 8192, kv_unified = 'false'",
        ]);
        let (out, _) = feed(&waking);
        assert_eq!((out.all_on_gpu, out.unloaded), (Some(false), false));
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
            "ggml_cuda_init: found 1 CUDA devices:",
        ];
        let (_, logged) = feed(&lines);
        assert_eq!(
            logged,
            [
                true, true, false, true, true, true, true, true, true, true, false, true
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
        assert_eq!(fold(&moved), Some(false), "--fit moved tensors off the GPU");
        let kept = ["common_params_fit_impl: no changes needed", main, draft];
        assert_eq!(fold(&kept), Some(true));
    }
}
