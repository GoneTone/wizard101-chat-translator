//! 啟動並監看 `llama-server.exe` 子程序，等它就緒。

use crate::local::hardware::{Gpu, detect_gpu};
use crate::logs::log;
use crate::util::{Clock, poll_until};
use std::collections::VecDeque;
use std::io::{self, BufRead, BufReader};
use std::net::TcpListener;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
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
}

/// 啟動 llama-server 與偵測硬體的方式（測試以假實作替換）。
pub trait Launcher: Send + Sync {
    fn launch(&self, plan: &ServerPlan, port: u16) -> io::Result<Box<dyn Process>>;

    /// 偵測 GPU（見 [`detect_gpu`]）。
    fn detect_gpu(&self) -> Option<Gpu> {
        detect_gpu()
    }

    /// 執行 `llama-server --list-devices`（逾時 15 s），回傳 stdout 與 stderr 的內容。
    fn list_devices(&self, exe: &Path) -> Result<String, String> {
        run_list_devices(exe)
    }
}

/// 真的以 `Command` 啟動子程序。
pub struct RealLauncher;

struct RealProcess {
    child: Child,
    tail: Arc<Mutex<VecDeque<String>>>,
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
        let mut readers = Vec::new();
        if let Some(out) = child.stdout.take() {
            readers.push(pump_lines(out, plan.model_id.clone(), tail.clone()));
        }
        if let Some(err) = child.stderr.take() {
            readers.push(pump_lines(err, plan.model_id.clone(), tail.clone()));
        }
        Ok(Box::new(RealProcess {
            child,
            tail,
            readers,
        }))
    }
}

/// 每行寫進 log 並留最後 `TAIL_LINES` 行；以位元組讀取，非 UTF-8 輸出不會讓執行緒死掉。
fn pump_lines(
    pipe: impl io::Read + Send + 'static,
    model_id: String,
    tail: Arc<Mutex<VecDeque<String>>>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        for raw in BufReader::new(pipe).split(b'\n') {
            let Ok(raw) = raw else { break };
            let line = String::from_utf8_lossy(&raw).trim_end().to_string();
            log(format!("[local] llama-server[{model_id}]: {line}"));
            let mut t = tail.lock().unwrap();
            if t.len() == TAIL_LINES {
                t.pop_front();
            }
            t.push_back(line);
        }
    })
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
