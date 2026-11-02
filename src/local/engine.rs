//! llama.cpp 官方 Windows 預建包：列出、安裝、切換與回退。

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use regex::Regex;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::local::download::{DownloadError, Progress, download};
use crate::local::hardware::{Gpu, GpuKind, NVIDIA};
use crate::logs::log;
use crate::net::error_chain;
use crate::util::write_atomic;

pub const RELEASES_API: &str =
    "https://api.github.com/repos/ggml-org/llama.cpp/releases?per_page=30";
/// 最低支援的 llama.cpp build 編號（`b11871`）。
pub const MIN_BUILD: u32 = 11871;

static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^b(\d+)$").unwrap());
static ENGINE_ZIP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^llama-b\d+-bin-win-(vulkan|cpu|cuda-[\d.]+)-x64\.zip$").unwrap()
});

/// 引擎後端變體；`Cuda` 帶 CUDA 版本（如 `13.4`）。
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Variant {
    Vulkan,
    Cpu,
    Cuda(String),
}

impl fmt::Display for Variant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Variant::Vulkan => f.write_str("vulkan"),
            Variant::Cpu => f.write_str("cpu"),
            Variant::Cuda(v) => write!(f, "cuda-{v}"),
        }
    }
}

impl Variant {
    fn parse(s: &str) -> Option<Variant> {
        match s {
            "vulkan" => Some(Variant::Vulkan),
            "cpu" => Some(Variant::Cpu),
            _ => s.strip_prefix("cuda-").map(|v| Variant::Cuda(v.to_owned())),
        }
    }

    /// 穩定的顯示順序：Vulkan、Cpu、Cuda 版本由小到大。
    fn sort_key(&self) -> (u8, Vec<u32>) {
        match self {
            Variant::Vulkan => (0, vec![]),
            Variant::Cpu => (1, vec![]),
            Variant::Cuda(v) => (2, v.split('.').filter_map(|p| p.parse().ok()).collect()),
        }
    }
}

impl Serialize for Variant {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Variant {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Variant::parse(&s).ok_or_else(|| serde::de::Error::custom(format!("unknown variant {s}")))
    }
}

/// NVIDIA 獨顯用 `build` 裡驅動撐得起的最新 CUDA 版，其他 GPU 用 Vulkan，沒有 GPU 用 CPU。
pub fn auto_variant(gpu: Option<&Gpu>, build: Option<&Build>) -> Variant {
    let Some(gpu) = gpu else {
        return Variant::Cpu;
    };
    let cuda = gpu
        .nvidia_driver
        .filter(|_| gpu.vendor == NVIDIA && gpu.kind == GpuKind::Discrete)
        .and_then(|driver| {
            build?
                .variants
                .iter()
                // 少了 cudart 的 CUDA 版在沒裝 CUDA Toolkit 的電腦上跑不起來
                .filter(|(_, assets)| assets.len() > 1)
                .map(|(v, _)| v)
                .filter(|v| cuda_runs(v, gpu, driver))
                .max_by_key(|v| v.sort_key())
        });
    cuda.cloned().unwrap_or(Variant::Vulkan)
}

/// NVIDIA 的 PCI device ID 依世代遞增，Turing 從這裡起。
const TURING_FIRST_DEVICE: u32 = 0x1e00;

/// 這張卡與驅動（乘以 100）跑得動這個 CUDA 版；12.x 要驅動 528.33 以上，13.x 要 580 以上且 Turing 以後。
fn cuda_runs(variant: &Variant, gpu: &Gpu, driver: u32) -> bool {
    let Variant::Cuda(version) = variant else {
        return false;
    };
    match version.split('.').next() {
        Some("12") => driver >= 52_833,
        // CUDA 13 拿掉了 Maxwell／Pascal／Volta
        Some("13") => driver >= 58_000 && gpu.device >= TURING_FIRST_DEVICE,
        _ => false,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

/// 一個 release 與它每個變體需要下載的 asset。
#[derive(Clone, Debug, PartialEq)]
pub struct Build {
    pub tag: String,
    pub published: String,
    pub variants: Vec<(Variant, Vec<Asset>)>,
}

fn asset_of(a: &Value) -> Option<Asset> {
    Some(Asset {
        name: a["name"].as_str()?.to_owned(),
        url: a["browser_download_url"].as_str()?.to_owned(),
        size: a["size"].as_u64()?,
    })
}

/// `b1234` 形式的 tag 取出編號。
pub fn build_number(tag: &str) -> Option<u32> {
    TAG.captures(tag).and_then(|c| c[1].parse().ok())
}

/// 解析 GitHub releases JSON，只留 Windows 預建包齊全且不低於 [`MIN_BUILD`] 的 build（新到舊）。
pub fn parse_releases(json: &Value) -> Vec<Build> {
    let Some(releases) = json.as_array() else {
        return vec![];
    };
    let mut out = vec![];
    for r in releases {
        let Some(tag) = r["tag_name"].as_str() else {
            continue;
        };
        if build_number(tag).is_none_or(|n| n < MIN_BUILD) {
            continue;
        }
        let assets: Vec<Asset> = r["assets"]
            .as_array()
            .map(|a| a.iter().filter_map(asset_of).collect())
            .unwrap_or_default();
        let mut variants: Vec<(Variant, Vec<Asset>)> = vec![];
        for a in &assets {
            let Some(v) = ENGINE_ZIP
                .captures(&a.name)
                .and_then(|c| Variant::parse(&c[1]))
            else {
                continue;
            };
            let mut list = vec![a.clone()];
            if let Variant::Cuda(ver) = &v {
                let cudart = format!("cudart-llama-bin-win-cuda-{ver}-x64.zip");
                list.extend(assets.iter().filter(|x| x.name == cudart).cloned());
            }
            variants.push((v, list));
        }
        if variants.is_empty() {
            continue;
        }
        variants.sort_by_key(|(v, _)| v.sort_key());
        out.push(Build {
            tag: tag.to_owned(),
            published: r["published_at"].as_str().unwrap_or_default().to_owned(),
            variants,
        });
    }
    out
}

/// 從 `url`（正式為 [`RELEASES_API`]）取得可安裝的 build 清單。
pub async fn list_builds(http: &reqwest::Client, url: &str) -> Result<Vec<Build>, String> {
    // GitHub API 要求帶 User-Agent，缺了會被拒（403）
    let response = http
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header(
            "User-Agent",
            format!("wizard101-chat-translator/{}", env!("CARGO_PKG_VERSION")),
        )
        .send()
        .await
        .map_err(error_chain)?;
    let status = response.status();
    if !status.is_success() {
        log(format!(
            "[local] list llama.cpp builds failed: HTTP {}",
            status.as_u16()
        ));
        return Err(format!("HTTP {}", status.as_u16()));
    }
    let data: Value = response
        .json()
        .await
        .map_err(|e| format!("unexpected response: {}", error_chain(e)))?;
    let builds = parse_releases(&data);
    log(format!("[local] listed {} llama.cpp builds", builds.len()));
    Ok(builds)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EngineInstall {
    pub build: String,
    pub variant: Variant,
}

/// `<root>/llama/active.json` 的內容：目前用的版本與可回退的上一版。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ActiveFile {
    pub current: Option<EngineInstall>,
    pub previous: Option<EngineInstall>,
}

fn llama_dir(root: &Path) -> PathBuf {
    root.join("llama")
}

fn active_path(root: &Path) -> PathBuf {
    llama_dir(root).join("active.json")
}

pub fn engine_dir(root: &Path, i: &EngineInstall) -> PathBuf {
    llama_dir(root).join(format!("{}-{}", i.build, i.variant))
}

/// 遞迴找 `llama-server.exe`。
pub fn server_exe(dir: &Path) -> Option<PathBuf> {
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = server_exe(&path) {
                return Some(found);
            }
        } else if entry.file_name().eq_ignore_ascii_case("llama-server.exe") {
            return Some(path);
        }
    }
    None
}

/// 讀 `active.json`；不存在或損毀都當作尚未安裝。
pub fn read_active(root: &Path) -> ActiveFile {
    match fs::read_to_string(active_path(root)) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
            log(format!("[local] active.json is corrupt, ignoring: {e}"));
            ActiveFile::default()
        }),
        Err(e) => {
            if e.kind() != io::ErrorKind::NotFound {
                log(format!("[local] cannot read active.json: {e}"));
            }
            ActiveFile::default()
        }
    }
}

fn write_active(root: &Path, active: &ActiveFile) -> io::Result<()> {
    let text = serde_json::to_string_pretty(active).map_err(io::Error::other)?;
    write_atomic(&active_path(root), &text)
}

fn remove_dir_if_exists(dir: &Path) -> io::Result<()> {
    match fs::remove_dir_all(dir) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

fn extract_zip(zip_path: &Path, dest: &Path) -> Result<(), String> {
    let file = fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    // `extract` 會擋掉跳出 `dest` 的條目路徑（zip-slip）
    archive.extract(dest).map_err(|e| e.to_string())
}

/// 把單一 zip 的進度換算成整體的間隔。
const PART_TICK: Duration = Duration::from_millis(100);

/// 下載並解壓 `build` 的 `variant` 到引擎資料夾；失敗或取消不留半成品，也不動 `active.json`。
pub async fn install(
    http: &reqwest::Client,
    root: &Path,
    build: &Build,
    variant: &Variant,
    progress: &Progress,
    cancel: &CancellationToken,
) -> Result<EngineInstall, String> {
    if build_number(&build.tag).is_none_or(|n| n < MIN_BUILD) {
        return Err(format!(
            "build {} is not supported (minimum b{MIN_BUILD})",
            build.tag
        ));
    }
    let assets = build
        .variants
        .iter()
        .find(|(v, _)| v == variant)
        .map(|(_, a)| a)
        .ok_or_else(|| format!("build {} has no {variant} variant", build.tag))?;
    let installed = EngineInstall {
        build: build.tag.clone(),
        variant: variant.clone(),
    };
    let dir = engine_dir(root, &installed);
    if server_exe(&dir).is_some() {
        log(format!(
            "[local] engine {}-{variant} already installed",
            build.tag
        ));
        return Ok(installed);
    }

    let size: u64 = assets.iter().map(|a| a.size).sum();
    log(format!(
        "[local] installing llama.cpp {} {variant} ({size} bytes)",
        build.tag
    ));
    let started = Instant::now();
    let downloads = llama_dir(root).join(".downloads");
    let mut tmp_name = dir.as_os_str().to_owned();
    tmp_name.push(".tmp");
    let tmp = PathBuf::from(tmp_name);
    remove_dir_if_exists(&tmp).map_err(|e| e.to_string())?;

    let result = async {
        fs::create_dir_all(&downloads).map_err(|e| e.to_string())?;
        progress.total.store(size, Ordering::Relaxed);
        progress.done.store(0, Ordering::Relaxed);
        let mut base = 0;
        for a in assets {
            let zip_path = downloads.join(&a.name);
            // 每個 zip 各自計進度，換算成整體寫回 `progress`，CUDA 版第二個 zip 開始時才不會歸零
            let part = Progress::default();
            let fetch = download(http, &a.url, &zip_path, a.size, None, &part, cancel);
            tokio::pin!(fetch);
            let mut tick = tokio::time::interval(PART_TICK);
            let fetched = loop {
                tokio::select! {
                    r = &mut fetch => break r,
                    _ = tick.tick() => progress
                        .done
                        .store(base + part.done.load(Ordering::Relaxed), Ordering::Relaxed),
                }
            };
            fetched.map_err(|e| e.to_string())?;
            base += a.size;
            progress.done.store(base, Ordering::Relaxed);
            if cancel.is_cancelled() {
                return Err(DownloadError::Cancelled.to_string());
            }
            progress.extracting.store(true, Ordering::Relaxed);
            // 在別的執行緒解壓：這個 task 不能停住，進度才能把「解壓中」回報出去
            let (zip, dest) = (zip_path.clone(), tmp.clone());
            let extracted = tokio::task::spawn_blocking(move || extract_zip(&zip, &dest))
                .await
                .map_err(|e| format!("extracting {} failed: {e}", a.name))
                .and_then(|r| r);
            progress.extracting.store(false, Ordering::Relaxed);
            if let Err(e) = fs::remove_file(&zip_path) {
                log(format!("[local] cannot remove {}: {e}", a.name));
            }
            extracted?;
            if cancel.is_cancelled() {
                return Err(DownloadError::Cancelled.to_string());
            }
        }
        remove_dir_if_exists(&dir).map_err(|e| e.to_string())?;
        fs::rename(&tmp, &dir).map_err(|e| e.to_string())
    }
    .await;

    if let Err(e) = result {
        let _ = fs::remove_dir_all(&tmp);
        log(format!(
            "[local] install llama.cpp {} {variant} failed: {e}",
            build.tag
        ));
        return Err(e);
    }
    log(format!(
        "[local] installed llama.cpp {} {variant} in {:.1}s",
        build.tag,
        started.elapsed().as_secs_f32()
    ));
    Ok(installed)
}

/// 切到 `new`：原本的 current 變成可回退的 previous；
/// 已有 previous（current 還沒確認可用）時保留 previous，刪掉沒確認過的 current。
pub fn activate(root: &Path, new: EngineInstall) -> io::Result<()> {
    let mut active = read_active(root);
    log(format!(
        "[local] activating engine {}-{}",
        new.build, new.variant
    ));
    match (&active.previous, active.current.take()) {
        (None, current) => active.previous = current,
        (Some(prev), Some(unconfirmed)) if unconfirmed != *prev && unconfirmed != new => {
            log(format!(
                "[local] removing unconfirmed engine {}-{}",
                unconfirmed.build, unconfirmed.variant
            ));
            // 正在載入模型時 Windows 鎖著檔案刪不掉；只記下來，不擋這次安裝
            if let Err(e) = remove_dir_if_exists(&engine_dir(root, &unconfirmed)) {
                log(format!("[local] removing unconfirmed engine failed: {e}"));
            }
        }
        _ => {}
    }
    active.current = Some(new);
    write_active(root, &active)
}

/// 新版首次就緒後呼叫：刪掉 previous 的資料夾並清掉紀錄。
pub fn confirm(root: &Path) -> io::Result<()> {
    let mut active = read_active(root);
    let Some(prev) = active.previous.take() else {
        return Ok(());
    };
    log(format!(
        "[local] confirming engine, removing previous {}-{}",
        prev.build, prev.variant
    ));
    // 同一版重裝時 previous 與 current 指向同一資料夾，不能刪
    if active.current.as_ref() != Some(&prev) {
        remove_dir_if_exists(&engine_dir(root, &prev))?;
    }
    write_active(root, &active)
}

/// current 啟動失敗時回退：有不同的 previous 才刪 current 並改回它；否則什麼都不動，回傳 `None`。
pub fn rollback(root: &Path) -> io::Result<Option<EngineInstall>> {
    let mut active = read_active(root);
    let (Some(cur), Some(prev)) = (&active.current, &active.previous) else {
        return Ok(None);
    };
    if cur == prev {
        return Ok(None);
    }
    log(format!(
        "[local] rolling back engine {}-{} to {}-{}",
        cur.build, cur.variant, prev.build, prev.variant
    ));
    remove_dir_if_exists(&engine_dir(root, cur))?;
    active.current = active.previous.take();
    write_active(root, &active)?;
    Ok(active.current)
}

/// 刪除所有已安裝的引擎。
pub fn remove_all(root: &Path) -> io::Result<()> {
    log("[local] removing all llama.cpp engines");
    remove_dir_if_exists(&llama_dir(root))
}
