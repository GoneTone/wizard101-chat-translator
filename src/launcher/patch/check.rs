//! 整份安裝的檢查與（官網版）更新流程。

use std::collections::{BTreeSet, HashMap};
use std::fs::{self, File};
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tokio::time::MissedTickBehavior;
use windows_sys::Win32::Foundation::{ERROR_LOCK_VIOLATION, ERROR_SHARING_VIOLATION};

use super::PatchFailure;
use super::cache::{CacheRecord, PackageRecords, file_stamp, is_fresh, record_for, record_path};
use super::crc::{ki_crc, ki_crc_reader};
use super::fetch::{self, install_file};
use super::list::{FileEntry, parse_file_list, read_local_packages, select};
use super::server::{self, ListLocation};
use super::url::{file_url, is_gzip};
use crate::launcher::install::{Install, InstallSource};
use crate::launcher::running;
use crate::local::download::{self, FREE_SPACE_MARGIN};
use crate::logs::log;
use crate::net;

/// 一份安裝的檢查進度與結果，送給介面顯示。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatchStatus {
    Checking { done: u64, total: u64 },
    Downloading { done: u64, total: u64 },
    UpToDate,
    Updated,
    Failed(PatchFailure),
}

/// 檢查通過的結果：有沒有下載、整個檢查花的秒數（四捨五入）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PatchOutcome {
    pub downloaded: bool,
    pub seconds: u64,
}

/// 檢查流程對外的相依（連線、下載、程序與磁碟），測試時換成假的。
pub trait PatchEnv: Sync {
    fn list_location(
        &self,
        install: &Install,
    ) -> impl Future<Output = Result<ListLocation, PatchFailure>> + Send;
    /// 下載到 `temp`；收到超過 `limit` 位元組就失敗。
    fn fetch(
        &self,
        url: &str,
        temp: &Path,
        done: &AtomicU64,
        limit: Option<u64>,
    ) -> impl Future<Output = Result<(), PatchFailure>> + Send;
    fn install_busy(&self, root: &Path) -> impl Future<Output = bool> + Send;
    fn free_space(&self, dir: &Path) -> Result<u64, PatchFailure>;
    /// 算檔案 CRC 的函式；測試換成會模擬檔案被鎖住的版本。
    fn hasher(&self) -> fn(&Path) -> io::Result<u32> {
        file_crc
    }
}

/// 實際連線、下載與查程序的 `PatchEnv`。
pub struct SystemPatchEnv {
    http: reqwest::Client,
}

impl SystemPatchEnv {
    pub fn new() -> Self {
        Self {
            http: net::download_client("http://versionak.us.wizard101.com"),
        }
    }
}

impl Default for SystemPatchEnv {
    fn default() -> Self {
        Self::new()
    }
}

impl PatchEnv for SystemPatchEnv {
    async fn list_location(&self, install: &Install) -> Result<ListLocation, PatchFailure> {
        let (host, port) = server::patch_server(&install.root)?;
        server::list_location(&host, port).await
    }

    async fn fetch(
        &self,
        url: &str,
        temp: &Path,
        done: &AtomicU64,
        limit: Option<u64>,
    ) -> Result<(), PatchFailure> {
        fetch::fetch_to(&self.http, url, temp, done, limit).await
    }

    async fn install_busy(&self, root: &Path) -> bool {
        let root = root.to_path_buf();
        // 列舉程序是同步呼叫，不能卡住非同步執行緒；查不了就當成開著
        tokio::task::spawn_blocking(move || running::install_busy(&root))
            .await
            .unwrap_or_else(|e| {
                log(format!(
                    "[patch] install busy check failed, assuming busy: {e}"
                ));
                true
            })
    }

    fn free_space(&self, dir: &Path) -> Result<u64, PatchFailure> {
        download::free_space(dir).map_err(|e| PatchFailure::Io(format!("{}: {e}", dir.display())))
    }
}

/// 檢查一份安裝；官網版順便下載變動的檔案，Steam 版只檢查。
///
/// 成功回報 `UpToDate`／`Updated`、失敗回報 `Failed`，各恰好一次；無論成敗都會試著存已驗證的紀錄，存不了只記 log。
pub async fn check_install<E: PatchEnv>(
    install: &Install,
    env: &E,
    report: &(dyn Fn(PatchStatus) + Sync),
) -> Result<PatchOutcome, PatchFailure> {
    let started = Instant::now();
    let root = &install.root;
    log(format!(
        "[patch] check start: source={:?} root={}",
        install.source,
        root.display()
    ));
    let mut records = Records::new(root);
    let mut list_fetched = false;
    let result = run(install, env, report, &mut records, &mut list_fetched).await;
    // 失敗時也要存，已驗證或已下載的檔案下次不必重做
    let saved = blocking(move || records.save_dirty()).await;
    if result.is_ok() {
        let renamed = if list_fetched {
            fs::rename(list_temp(root), list_copy(root)).map_err(io_failure(&list_copy(root)))
        } else {
            Ok(())
        };
        // 紀錄與清單副本只是快取，留不住下次重算就好，不擋開遊戲
        if let Err(e) = saved.and(renamed) {
            log(format!(
                "[patch] cannot keep check cache: root={} error={e:?}",
                root.display()
            ));
        }
    }
    // 成功時已改名，這裡只清掉失敗留下的清單暫存
    let _ = fs::remove_file(list_temp(root));
    let seconds = started.elapsed().as_secs_f64().round() as u64;
    match &result {
        Ok(downloaded) => {
            log(format!(
                "[patch] check done: root={} downloaded={downloaded} elapsed={seconds}s",
                root.display()
            ));
            report(if *downloaded {
                PatchStatus::Updated
            } else {
                PatchStatus::UpToDate
            });
        }
        Err(e) => {
            log(format!(
                "[patch] check failed: root={} error={e:?} elapsed={seconds}s",
                root.display()
            ));
            report(PatchStatus::Failed(e.clone()));
        }
    }
    result.map(|downloaded| PatchOutcome {
        downloaded,
        seconds,
    })
}

const REPORT_INTERVAL: Duration = Duration::from_millis(100);

fn io_failure(path: &Path) -> impl Fn(io::Error) -> PatchFailure + '_ {
    move |e| PatchFailure::Io(format!("{}: {e}", path.display()))
}

fn list_copy(root: &Path) -> PathBuf {
    root.join("PatchInfo").join("LatestFileList.bin")
}

fn list_temp(root: &Path) -> PathBuf {
    fetch::temp_path(&list_copy(root))
}

/// 在 blocking 執行緒跑讀檔、算 CRC 等同步工作。
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, PatchFailure> + Send + 'static,
) -> Result<T, PatchFailure> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| PatchFailure::Io(e.to_string()))?
}

/// 回 `Ok(true)` 代表有下載檔案；紀錄的存檔與清單副本改名交給呼叫端。
async fn run<E: PatchEnv>(
    install: &Install,
    env: &E,
    report: &(dyn Fn(PatchStatus) + Sync),
    records: &mut Records,
    list_fetched: &mut bool,
) -> Result<bool, PatchFailure> {
    let loc = env.list_location(install).await?;
    let entries = load_list(&install.root, &loc, env, list_fetched).await?;
    let local_packages = read_local_packages(&install.root);
    let mut selected: Vec<FileEntry> = select(&entries, install.source, &local_packages)
        .into_iter()
        .cloned()
        .collect();
    // Steam 版從不下載，不會有暫存，也不碰它的遊戲檔
    if install.source == InstallSource::Official {
        let root = install.root.clone();
        selected = blocking(move || {
            for entry in &selected {
                fetch::remove_temps(&root.join(entry.local_path()));
            }
            Ok(selected)
        })
        .await?;
    }
    let Scanned { stale, locked } =
        scan(&install.root, selected, records, report, env.hasher()).await?;
    if let Some((_, first)) = locked.first() {
        // 只有這份安裝的程式開著時，被鎖住才是正在用；否則照一般讀檔錯誤處理
        if !env.install_busy(&install.root).await {
            return Err(first.clone());
        }
        log(format!(
            "[patch] {} file(s) in use by a running client, skipped: first={:?}",
            locked.len(),
            locked
                .iter()
                .take(5)
                .map(|(src, _)| src)
                .collect::<Vec<_>>()
        ));
    }
    if stale.is_empty() {
        return Ok(false);
    }
    if install.source == InstallSource::Steam {
        log(format!(
            "[patch] steam install is outdated: {} file(s) differ, first={:?}",
            stale.len(),
            stale
                .iter()
                .take(5)
                .map(|e| e.src.as_str())
                .collect::<Vec<_>>()
        ));
        return Err(PatchFailure::SteamOutdated);
    }
    if env.install_busy(&install.root).await {
        log(format!(
            "[patch] {} file(s) need download but the game or launcher is running",
            stale.len()
        ));
        return Err(PatchFailure::GameOpen);
    }
    let bytes: u64 = stale.iter().map(|e| u64::from(e.size)).sum();
    let need = bytes.saturating_add(FREE_SPACE_MARGIN);
    let free = env.free_space(&install.root)?;
    if free < need {
        log(format!(
            "[patch] not enough disk space: need={need} free={free}"
        ));
        return Err(PatchFailure::NoSpace { need, free });
    }
    download(&install.root, &loc, env, &stale, bytes, records, report).await?;
    Ok(true)
}

/// 本機副本的 CRC 吻合就用它，否則下載到暫存並驗大小與 CRC。
async fn load_list<E: PatchEnv>(
    root: &Path,
    loc: &ListLocation,
    env: &E,
    fetched: &mut bool,
) -> Result<Vec<FileEntry>, PatchFailure> {
    let (copy, crc) = (list_copy(root), loc.crc);
    let cached = blocking(move || Ok(fs::read(copy).ok().filter(|b| ki_crc(b) == crc))).await?;
    let bytes = match cached {
        Some(bytes) => {
            log(format!(
                "[patch] reusing local file list: {} bytes",
                bytes.len()
            ));
            bytes
        }
        None => {
            let temp = list_temp(root);
            let dir = root.join("PatchInfo");
            fs::create_dir_all(&dir).map_err(io_failure(&dir))?;
            *fetched = true;
            let limit = Some(u64::from(loc.size));
            env.fetch(&loc.url, &temp, &AtomicU64::new(0), limit)
                .await?;
            let (size, crc) = (loc.size, loc.crc);
            blocking(move || {
                let bytes = fs::read(&temp).map_err(io_failure(&temp))?;
                let actual = ki_crc(&bytes);
                if bytes.len() as u64 != u64::from(size) || actual != crc {
                    log(format!(
                        "[patch] file list mismatch: expected_crc={crc:08x} actual_crc={actual:08x} expected_size={size} actual_size={}",
                        bytes.len()
                    ));
                    return Err(PatchFailure::Protocol(format!(
                        "file list CRC mismatch: expected {crc:08x}/{size} bytes, got {actual:08x}/{} bytes",
                        bytes.len()
                    )));
                }
                log(format!("[patch] downloaded file list: {} bytes", bytes.len()));
                Ok(bytes)
            })
            .await?
        }
    };
    blocking(move || parse_file_list(&bytes).map_err(PatchFailure::Protocol)).await
}

/// 在 blocking 執行緒跑整個比對；進度經 channel 轉給 `report`，紀錄不論成敗都放回 `records`。
async fn scan(
    root: &Path,
    selected: Vec<FileEntry>,
    records: &mut Records,
    report: &(dyn Fn(PatchStatus) + Sync),
    hash: fn(&Path) -> io::Result<u32>,
) -> Result<Scanned, PatchFailure> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut taken = std::mem::replace(records, Records::new(root));
    let root = root.to_path_buf();
    let task = tokio::task::spawn_blocking(move || {
        let send = |status| {
            let _ = tx.send(status);
        };
        let stale = scan_files(&root, &selected, &mut taken, &send, hash);
        (taken, stale)
    });
    while let Some(status) = rx.recv().await {
        report(status);
    }
    let (taken, stale) = task.await.map_err(|e| PatchFailure::Io(e.to_string()))?;
    *records = taken;
    stale
}

/// 比對結果：仍不符或缺少的檔案，與被鎖住讀不了的檔案（檔名與讀檔錯誤）。
struct Scanned {
    stale: Vec<FileEntry>,
    locked: Vec<(String, PatchFailure)>,
}

/// 逐檔比對紀錄、必要時重算 CRC；被鎖住的檔案不改紀錄、留給呼叫端判斷。
fn scan_files(
    root: &Path,
    selected: &[FileEntry],
    records: &mut Records,
    report: &dyn Fn(PatchStatus),
    hash: fn(&Path) -> io::Result<u32>,
) -> Result<Scanned, PatchFailure> {
    let started = Instant::now();
    let total = selected.len() as u64;
    let mut last_report = Instant::now();
    report(PatchStatus::Checking { done: 0, total });
    let (mut hashed, mut stale, mut locked) = (0usize, Vec::new(), Vec::new());
    for (i, entry) in selected.iter().enumerate() {
        let dest = root.join(entry.local_path());
        let stamp = file_stamp(&dest);
        if !is_fresh(records.get(entry)?, entry, stamp) {
            match stamp {
                Some(stamp) if stamp.0 == u64::from(entry.size) => {
                    hashed += 1;
                    match hash(&dest) {
                        Ok(crc) if crc == entry.crc => {
                            records.set(entry, record_for(entry, stamp))?;
                        }
                        Ok(_) => stale.push(entry.clone()),
                        Err(e) if is_locked(&e) => {
                            locked.push((entry.src.clone(), io_failure(&dest)(e)));
                        }
                        Err(e) => {
                            log(format!("[patch] cannot read {}: {e}", dest.display()));
                            return Err(io_failure(&dest)(e));
                        }
                    }
                }
                _ => stale.push(entry.clone()),
            }
        }
        let done = i as u64 + 1;
        if done == total || last_report.elapsed() >= REPORT_INTERVAL {
            last_report = Instant::now();
            report(PatchStatus::Checking { done, total });
        }
    }
    log(format!(
        "[patch] checked {total} file(s): hashed={hashed} stale={} locked={} elapsed={}ms",
        stale.len(),
        locked.len(),
        started.elapsed().as_millis()
    ));
    Ok(Scanned { stale, locked })
}

/// 檔案被別的程式以不共用方式開著或鎖住區段。
fn is_locked(e: &io::Error) -> bool {
    matches!(
        e.raw_os_error().map(|code| code as u32),
        Some(ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION)
    )
}

fn file_crc(path: &Path) -> io::Result<u32> {
    File::open(path).and_then(ki_crc_reader)
}

/// 逐檔下載、驗證、換上並立刻存該套件的紀錄。
async fn download<E: PatchEnv>(
    root: &Path,
    loc: &ListLocation,
    env: &E,
    stale: &[FileEntry],
    total: u64,
    records: &mut Records,
    report: &(dyn Fn(PatchStatus) + Sync),
) -> Result<(), PatchFailure> {
    let started = Instant::now();
    log(format!(
        "[patch] downloading {} file(s), {total} bytes",
        stale.len()
    ));
    let done = AtomicU64::new(0);
    let mut ticker = tokio::time::interval(REPORT_INTERVAL);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut finished = 0u64;
    for entry in stale {
        let dest = root.join(entry.local_path());
        let temp = fetch::temp_path(&dest);
        if let Some(dir) = dest.parent() {
            fs::create_dir_all(dir).map_err(io_failure(dir))?;
        }
        let url = file_url(loc, entry);
        let fetched = {
            let size = u64::from(entry.size);
            // gzip 檔的 size 是解壓後大小；難壓縮的資料壓完會比原檔略大
            let limit = Some(if is_gzip(entry) {
                size + size / 100 + 1024
            } else {
                size
            });
            let fetch = env.fetch(&url, &temp, &done, limit);
            tokio::pin!(fetch);
            loop {
                tokio::select! {
                    r = &mut fetch => break r,
                    _ = ticker.tick() => report(PatchStatus::Downloading {
                        done: done.load(Ordering::Relaxed),
                        total,
                    }),
                }
            }
        };
        let installed = match fetched {
            // 下載途中才開的遊戲或啟動器會讀到換到一半的檔案
            Ok(()) if env.install_busy(root).await => Err(PatchFailure::GameOpen),
            Ok(()) => {
                let (e, t, d) = (entry.clone(), temp.clone(), dest.clone());
                blocking(move || install_file(&e, &t, &d)).await
            }
            Err(e) => Err(e),
        };
        let stamp = installed.inspect_err(|e| {
            let _ = fs::remove_file(&temp);
            log(format!(
                "[patch] download failed: file={} error={e:?}",
                entry.src
            ));
        })?;
        records.set(entry, record_for(entry, stamp))?;
        // 存不了就留在 dirty，結束時再試；紀錄只是快取，不中斷更新
        let _ = records.save(&entry.package);
        // gzip 檔收到的是壓縮後位元組，每檔完成時校正成解壓後大小的累計
        finished += u64::from(entry.size);
        done.store(finished, Ordering::Relaxed);
    }
    report(PatchStatus::Downloading {
        done: done.load(Ordering::Relaxed),
        total,
    });
    log(format!(
        "[patch] downloaded {} file(s), {finished} bytes, elapsed={}s",
        stale.len(),
        started.elapsed().as_secs_f64().round() as u64
    ));
    Ok(())
}

/// 依套件懶載入的檢查紀錄，記住哪些套件改過還沒存。
struct Records {
    root: PathBuf,
    loaded: HashMap<String, PackageRecords>,
    dirty: BTreeSet<String>,
}

impl Records {
    fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            loaded: HashMap::new(),
            dirty: BTreeSet::new(),
        }
    }

    fn package(&mut self, package: &str) -> Result<&mut PackageRecords, PatchFailure> {
        if !self.loaded.contains_key(package) {
            let recs = PackageRecords::load(&self.root, package).map_err(|e| {
                log(format!(
                    "[patch] cannot read check records: package={package} error={e}"
                ));
                io_failure(&record_path(&self.root, package))(e)
            })?;
            self.loaded.insert(package.to_string(), recs);
        }
        Ok(self.loaded.get_mut(package).expect("just inserted"))
    }

    fn get(&mut self, entry: &FileEntry) -> Result<Option<&CacheRecord>, PatchFailure> {
        Ok(self.package(&entry.package)?.get(entry))
    }

    fn set(&mut self, entry: &FileEntry, record: CacheRecord) -> Result<(), PatchFailure> {
        self.package(&entry.package)?.set(record);
        self.dirty.insert(entry.package.clone());
        Ok(())
    }

    fn save(&mut self, package: &str) -> Result<(), PatchFailure> {
        if let Some(recs) = self.loaded.get(package) {
            recs.save(&self.root, package).map_err(|e| {
                log(format!(
                    "[patch] cannot save check records: package={package} error={e}"
                ));
                io_failure(&record_path(&self.root, package))(e)
            })?;
        }
        self.dirty.remove(package);
        Ok(())
    }

    /// 存所有改過的套件；有失敗也繼續存其他的，回第一個錯誤。
    fn save_dirty(&mut self) -> Result<(), PatchFailure> {
        let mut first = Ok(());
        for package in std::mem::take(&mut self.dirty) {
            if let Err(e) = self.save(&package) {
                first = first.and(Err(e));
            }
        }
        first
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launcher::patch::fetch::tests::gzip;
    use crate::launcher::patch::list::tests::{PKG_FIELDS, row, schema};
    use std::sync::Mutex;
    use std::sync::atomic::AtomicBool;

    const LIST_URL: &str = "http://list/LatestFileList.bin";
    const PREFIX: &str = "http://files";

    /// 依 (套件, 檔名, 內容) 組一份清單；同套件的列要相鄰。
    fn list_bytes(files: &[(&str, &str, &[u8])]) -> Vec<u8> {
        let mut out = 1u32.to_le_bytes().to_vec();
        let mut current = "";
        for (package, src, body) in files {
            if *package != current {
                out.extend(schema(package, &PKG_FIELDS));
                current = package;
            }
            out.extend(row(src, "", 1, body.len() as u32, ki_crc(body)));
        }
        out
    }

    struct FakeEnv {
        loc: ListLocation,
        bodies: HashMap<String, Vec<u8>>,
        fail: Option<String>,
        busy: AtomicBool,
        /// 下載這個網址後遊戲被開起來。
        busy_after: Option<String>,
        free: u64,
        hasher: fn(&Path) -> io::Result<u32>,
        fetched: Mutex<Vec<String>>,
        limits: Mutex<HashMap<String, Option<u64>>>,
    }

    impl FakeEnv {
        /// 伺服器上的清單與檔案內容；檔案網址為 `PREFIX/src`。
        fn new(files: &[(&str, &str, &[u8])]) -> Self {
            let bodies = files
                .iter()
                .map(|(_, src, body)| (format!("{PREFIX}/{src}"), body.to_vec()))
                .collect();
            let loc = ListLocation {
                version: 1,
                size: 0,
                crc: 0,
                url: LIST_URL.into(),
                prefix: PREFIX.into(),
                suffix: String::new(),
            };
            let mut env = Self {
                loc,
                bodies,
                fail: None,
                busy: AtomicBool::new(false),
                busy_after: None,
                free: u64::MAX,
                hasher: file_crc,
                fetched: Mutex::new(Vec::new()),
                limits: Mutex::default(),
            };
            env.set_list(list_bytes(files));
            env
        }

        fn set_list(&mut self, list: Vec<u8>) {
            self.loc.size = list.len() as u32;
            self.loc.crc = ki_crc(&list);
            self.bodies.insert(LIST_URL.into(), list);
        }

        fn list(&self) -> &[u8] {
            &self.bodies[LIST_URL]
        }

        fn entries(&self) -> Vec<FileEntry> {
            parse_file_list(self.list()).unwrap()
        }

        fn fetched(&self) -> Vec<String> {
            self.fetched.lock().unwrap().clone()
        }
    }

    impl PatchEnv for FakeEnv {
        async fn list_location(&self, _install: &Install) -> Result<ListLocation, PatchFailure> {
            Ok(self.loc.clone())
        }

        async fn fetch(
            &self,
            url: &str,
            temp: &Path,
            done: &AtomicU64,
            limit: Option<u64>,
        ) -> Result<(), PatchFailure> {
            self.fetched.lock().unwrap().push(url.into());
            self.limits.lock().unwrap().insert(url.into(), limit);
            if self.busy_after.as_deref() == Some(url) {
                self.busy.store(true, Ordering::SeqCst);
            }
            if self.fail.as_deref() == Some(url) {
                fs::write(temp, b"partial").unwrap();
                return Err(PatchFailure::Download("connection reset".into()));
            }
            let body = self
                .bodies
                .get(url)
                .ok_or_else(|| PatchFailure::Download("HTTP 404".into()))?;
            if limit.is_some_and(|l| body.len() as u64 > l) {
                return Err(PatchFailure::Download(
                    "response larger than expected".into(),
                ));
            }
            fs::write(temp, body).map_err(|e| PatchFailure::Io(e.to_string()))?;
            done.fetch_add(body.len() as u64, Ordering::Relaxed);
            Ok(())
        }

        async fn install_busy(&self, _root: &Path) -> bool {
            self.busy.load(Ordering::SeqCst)
        }

        fn free_space(&self, _dir: &Path) -> Result<u64, PatchFailure> {
            Ok(self.free)
        }

        fn hasher(&self) -> fn(&Path) -> io::Result<u32> {
            self.hasher
        }
    }

    fn install(dir: &tempfile::TempDir, source: InstallSource) -> Install {
        Install {
            source,
            root: dir.path().to_path_buf(),
        }
    }

    fn put(root: &Path, rel: &str, body: &[u8]) -> PathBuf {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, body).unwrap();
        path
    }

    /// 為本機檔案寫一筆吻合的紀錄，讓它被視為最新。
    fn mark_fresh(root: &Path, entry: &FileEntry) {
        let mut recs = PackageRecords::load(root, &entry.package).unwrap();
        let stamp = file_stamp(&root.join(entry.local_path())).unwrap();
        recs.set(record_for(entry, stamp));
        recs.save(root, &entry.package).unwrap();
    }

    fn record(root: &Path, entry: &FileEntry) -> Option<CacheRecord> {
        PackageRecords::load(root, &entry.package)
            .unwrap()
            .get(entry)
            .copied()
    }

    async fn run(
        install: &Install,
        env: &FakeEnv,
    ) -> (Result<PatchOutcome, PatchFailure>, Vec<PatchStatus>) {
        let statuses = Mutex::new(Vec::new());
        let result = check_install(install, env, &|s| statuses.lock().unwrap().push(s)).await;
        (result, statuses.into_inner().unwrap())
    }

    /// 結束狀態恰好一個且在最後。
    fn assert_terminal(statuses: &[PatchStatus], expected: PatchStatus) {
        let terminal = statuses
            .iter()
            .filter(|s| {
                !matches!(
                    s,
                    PatchStatus::Checking { .. } | PatchStatus::Downloading { .. }
                )
            })
            .count();
        assert_eq!(terminal, 1, "{statuses:?}");
        assert_eq!(statuses.last(), Some(&expected));
    }

    fn assert_no_list_copy(root: &Path) {
        assert!(!list_copy(root).exists());
        assert!(!list_temp(root).exists());
    }

    fn assert_send<T: Send>(_: T) {}

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn fresh_install_reports_up_to_date_without_fetching_files() {
        let dir = tempfile::tempdir().unwrap();
        let inst = install(&dir, InstallSource::Official);
        let env = FakeEnv::new(&[("Base", "Bin/a.dll", b"aaa")]);
        assert_send(check_install(&inst, &env, &|_| {}));
        put(dir.path(), "Bin/a.dll", b"aaa");
        mark_fresh(dir.path(), &env.entries()[0]);

        let (result, statuses) = run(&inst, &env).await;
        assert!(!result.unwrap().downloaded);
        assert_eq!(env.fetched(), [LIST_URL]);
        assert!(statuses.contains(&PatchStatus::Checking { done: 1, total: 1 }));
        assert_terminal(&statuses, PatchStatus::UpToDate);
        assert_eq!(fs::read(list_copy(dir.path())).unwrap(), env.list());
        assert!(!list_temp(dir.path()).exists());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn official_downloads_changed_file_and_updates_record() {
        let dir = tempfile::tempdir().unwrap();
        let inst = install(&dir, InstallSource::Official);
        let env = FakeEnv::new(&[
            ("Base", "Bin/a.dll", b"new!"),
            ("Base", "Data/GameData/b.wad", b"brand new"),
        ]);
        put(dir.path(), "Bin/a.dll", b"old");

        let (result, statuses) = run(&inst, &env).await;
        assert!(result.unwrap().downloaded);
        assert_eq!(fs::read(dir.path().join("Bin/a.dll")).unwrap(), b"new!");
        assert_eq!(
            fs::read(dir.path().join("Data/GameData/b.wad")).unwrap(),
            b"brand new"
        );
        for e in env.entries() {
            let rec = record(dir.path(), &e).unwrap();
            assert_eq!((rec.crc, rec.size), (e.crc, e.size));
            assert_eq!(
                Some(rec.mtime_ms),
                file_stamp(&dir.path().join(e.local_path())).map(|s| s.1)
            );
        }
        assert!(statuses.contains(&PatchStatus::Downloading {
            done: 13,
            total: 13
        }));
        assert_terminal(&statuses, PatchStatus::Updated);
        assert_eq!(fs::read(list_copy(dir.path())).unwrap(), env.list());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn steam_mismatch_is_outdated_and_writes_nothing_but_records() {
        let dir = tempfile::tempdir().unwrap();
        let inst = install(&dir, InstallSource::Steam);
        let env = FakeEnv::new(&[
            ("Base", "Bin/a.dll", b"same"),
            ("Base", "Bin/b.dll", b"newer"),
            ("Other", "Data/c.wad", b"missing"),
        ]);
        put(dir.path(), "Bin/a.dll", b"same");
        put(dir.path(), "Bin/b.dll", b"older");
        let entries = env.entries();

        let (result, statuses) = run(&inst, &env).await;
        assert_eq!(result, Err(PatchFailure::SteamOutdated));
        assert_eq!(env.fetched(), [LIST_URL]);
        assert_eq!(fs::read(dir.path().join("Bin/b.dll")).unwrap(), b"older");
        assert!(!dir.path().join("Data/c.wad").exists());
        assert!(record(dir.path(), &entries[0]).is_some());
        assert!(record(dir.path(), &entries[1]).is_none());
        assert_no_list_copy(dir.path());
        assert_terminal(&statuses, PatchStatus::Failed(PatchFailure::SteamOutdated));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn steam_recompute_match_writes_record() {
        let dir = tempfile::tempdir().unwrap();
        let inst = install(&dir, InstallSource::Steam);
        let env = FakeEnv::new(&[("Base", "Bin/a.dll", b"same"), ("Other", "x.dat", b"x")]);
        put(dir.path(), "Bin/a.dll", b"same");
        put(dir.path(), "x.dat", b"x");

        let (result, statuses) = run(&inst, &env).await;
        assert!(!result.unwrap().downloaded);
        for e in env.entries() {
            let rec = record(dir.path(), &e).unwrap();
            assert_eq!((rec.crc, rec.size), (e.crc, e.size));
        }
        assert_terminal(&statuses, PatchStatus::UpToDate);
        assert!(list_copy(dir.path()).is_file());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn busy_install_blocks_only_when_download_needed() {
        let dir = tempfile::tempdir().unwrap();
        let inst = install(&dir, InstallSource::Official);
        let mut env = FakeEnv::new(&[("Base", "Bin/a.dll", b"new!")]);
        env.busy = AtomicBool::new(true);
        put(dir.path(), "Bin/a.dll", b"old");

        let (result, statuses) = run(&inst, &env).await;
        assert_eq!(result, Err(PatchFailure::GameOpen));
        assert_eq!(env.fetched(), [LIST_URL]);
        assert_eq!(fs::read(dir.path().join("Bin/a.dll")).unwrap(), b"old");
        assert_terminal(&statuses, PatchStatus::Failed(PatchFailure::GameOpen));

        put(dir.path(), "Bin/a.dll", b"new!");
        let (result, statuses) = run(&inst, &env).await;
        assert!(!result.unwrap().downloaded);
        assert_terminal(&statuses, PatchStatus::UpToDate);
    }

    /// `b.dll` 被別的程式鎖住（ERROR_SHARING_VIOLATION）。
    fn b_locked(path: &Path) -> io::Result<u32> {
        if path.ends_with("b.dll") {
            return Err(io::Error::from_raw_os_error(32));
        }
        file_crc(path)
    }

    /// `b.dll` 讀不了，但不是被鎖住（存取被拒）。
    fn b_denied(path: &Path) -> io::Result<u32> {
        if path.ends_with("b.dll") {
            return Err(io::Error::from_raw_os_error(5));
        }
        file_crc(path)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_file_locked_by_a_running_client_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let inst = install(&dir, InstallSource::Official);
        let mut env = FakeEnv::new(&[("Base", "Bin/a.dll", b"aaa"), ("Base", "Bin/b.dll", b"bbb")]);
        env.hasher = b_locked;
        env.busy = AtomicBool::new(true);
        put(dir.path(), "Bin/a.dll", b"aaa");
        put(dir.path(), "Bin/b.dll", b"bbb");
        let entries = env.entries();

        let (result, statuses) = run(&inst, &env).await;
        assert!(!result.unwrap().downloaded);
        assert_eq!(env.fetched(), [LIST_URL]);
        assert!(record(dir.path(), &entries[0]).is_some());
        assert!(record(dir.path(), &entries[1]).is_none());
        assert_terminal(&statuses, PatchStatus::UpToDate);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unreadable_files_fail_unless_locked_by_a_running_client() {
        for (hasher, busy) in [(b_locked as fn(&Path) -> _, false), (b_denied, true)] {
            let dir = tempfile::tempdir().unwrap();
            let inst = install(&dir, InstallSource::Official);
            let mut env = FakeEnv::new(&[("Base", "Bin/b.dll", b"bbb")]);
            env.hasher = hasher;
            env.busy = AtomicBool::new(busy);
            put(dir.path(), "Bin/b.dll", b"bbb");

            let (result, statuses) = run(&inst, &env).await;
            let err = result.unwrap_err();
            assert!(
                matches!(&err, PatchFailure::Io(m) if m.contains("b.dll")),
                "{err:?}"
            );
            assert_terminal(&statuses, PatchStatus::Failed(err));
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn download_failure_keeps_original_and_record() {
        let dir = tempfile::tempdir().unwrap();
        let inst = install(&dir, InstallSource::Official);
        let mut env = FakeEnv::new(&[
            ("Base", "Bin/a.dll", b"new a"),
            ("Base", "Bin/b.dll", b"new b"),
        ]);
        env.fail = Some(format!("{PREFIX}/Bin/b.dll"));
        put(dir.path(), "Bin/a.dll", b"old a");
        put(dir.path(), "Bin/b.dll", b"old b");
        let entries = env.entries();
        let old_b = CacheRecord {
            name_crc: ki_crc(b"Bin/b.dll"),
            size: 5,
            crc: 0x1234,
            mtime_ms: 1,
        };
        let mut recs = PackageRecords::default();
        recs.set(old_b);
        recs.save(dir.path(), "Base").unwrap();

        let (result, statuses) = run(&inst, &env).await;
        let err = result.unwrap_err();
        assert!(matches!(err, PatchFailure::Download(_)));
        assert_eq!(fs::read(dir.path().join("Bin/a.dll")).unwrap(), b"new a");
        assert_eq!(record(dir.path(), &entries[0]).unwrap().crc, entries[0].crc);
        assert_eq!(fs::read(dir.path().join("Bin/b.dll")).unwrap(), b"old b");
        assert_eq!(record(dir.path(), &entries[1]), Some(old_b));
        assert!(!dir.path().join("Bin/b.dll.w101tmp").exists());
        assert_no_list_copy(dir.path());
        assert_terminal(&statuses, PatchStatus::Failed(err));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn corrupt_download_keeps_original_and_record() {
        let dir = tempfile::tempdir().unwrap();
        let inst = install(&dir, InstallSource::Official);
        let mut env = FakeEnv::new(&[("Base", "Bin/a.dll", b"new a")]);
        env.bodies
            .insert(format!("{PREFIX}/Bin/a.dll"), b"bad a".to_vec());
        put(dir.path(), "Bin/a.dll", b"old a");
        let entry = &env.entries()[0];
        let old = CacheRecord {
            name_crc: ki_crc(b"Bin/a.dll"),
            size: 5,
            crc: 0x1234,
            mtime_ms: 1,
        };
        let mut recs = PackageRecords::default();
        recs.set(old);
        recs.save(dir.path(), "Base").unwrap();

        let (result, statuses) = run(&inst, &env).await;
        let err = result.unwrap_err();
        assert!(matches!(&err, PatchFailure::Download(m) if m.contains("CRC mismatch")));
        assert_eq!(fs::read(dir.path().join("Bin/a.dll")).unwrap(), b"old a");
        assert_eq!(record(dir.path(), entry), Some(old));
        assert!(!dir.path().join("Bin/a.dll.w101tmp").exists());
        assert_no_list_copy(dir.path());
        assert_terminal(&statuses, PatchStatus::Failed(err));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn list_crc_mismatch_is_protocol_error() {
        let dir = tempfile::tempdir().unwrap();
        let inst = install(&dir, InstallSource::Official);
        let mut env = FakeEnv::new(&[("Base", "Bin/a.dll", b"new!")]);
        env.loc.crc ^= 1;
        put(dir.path(), "Bin/a.dll", b"old");

        let (result, statuses) = run(&inst, &env).await;
        let err = result.unwrap_err();
        assert!(matches!(err, PatchFailure::Protocol(_)), "{err:?}");
        assert_eq!(env.fetched(), [LIST_URL]);
        assert_eq!(fs::read(dir.path().join("Bin/a.dll")).unwrap(), b"old");
        assert_no_list_copy(dir.path());
        assert_terminal(&statuses, PatchStatus::Failed(err));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn no_space_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let inst = install(&dir, InstallSource::Official);
        let mut env = FakeEnv::new(&[("Base", "Bin/a.dll", b"new!")]);
        env.free = 10;
        put(dir.path(), "Bin/a.dll", b"old");

        let (result, statuses) = run(&inst, &env).await;
        let expected = PatchFailure::NoSpace {
            need: 4 + FREE_SPACE_MARGIN,
            free: 10,
        };
        assert_eq!(result, Err(expected.clone()));
        assert_eq!(env.fetched(), [LIST_URL]);
        assert_eq!(fs::read(dir.path().join("Bin/a.dll")).unwrap(), b"old");
        assert_terminal(&statuses, PatchStatus::Failed(expected));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn fetches_are_capped_at_the_listed_size() {
        let dir = tempfile::tempdir().unwrap();
        let inst = install(&dir, InstallSource::Official);
        let mut env = FakeEnv::new(&[("Base", "Bin/a.dll", b"plain")]);
        let mut list = 1u32.to_le_bytes().to_vec();
        list.extend(schema("Base", &PKG_FIELDS));
        list.extend(row("Bin/a.dll", "", 1, 5, ki_crc(b"plain")));
        list.extend(row("Data/z.wad", "", 4, 3, ki_crc(b"zzz")));
        env.set_list(list);
        env.bodies
            .insert(format!("{PREFIX}/Data/z.wad.gz"), gzip(b"zzz"));

        let (result, _) = run(&inst, &env).await;
        assert!(result.unwrap().downloaded);
        assert_eq!(fs::read(dir.path().join("Data/z.wad")).unwrap(), b"zzz");
        let limits = env.limits.lock().unwrap();
        assert_eq!(limits[LIST_URL], Some(u64::from(env.loc.size)));
        assert_eq!(limits[&format!("{PREFIX}/Bin/a.dll")], Some(5));
        assert_eq!(limits[&format!("{PREFIX}/Data/z.wad.gz")], Some(3 + 1024));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_oversized_gzip_body_fails_without_leaving_a_temp() {
        let dir = tempfile::tempdir().unwrap();
        let inst = install(&dir, InstallSource::Official);
        let mut env = FakeEnv::new(&[]);
        let mut list = 1u32.to_le_bytes().to_vec();
        list.extend(schema("Base", &PKG_FIELDS));
        list.extend(row("Data/z.wad", "", 4, 3, ki_crc(b"zzz")));
        env.set_list(list);
        // 合法的 gzip 後面接填充：沒有上限就會解出正確內容
        let mut body = gzip(b"zzz");
        body.resize(3 + 1024 + 1, 0);
        env.bodies.insert(format!("{PREFIX}/Data/z.wad.gz"), body);

        let (result, _) = run(&inst, &env).await;
        assert!(
            matches!(result, Err(PatchFailure::Download(_))),
            "{result:?}"
        );
        assert!(!dir.path().join("Data/z.wad").exists());
        assert!(!dir.path().join("Data/z.wad.w101tmp").exists());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_game_started_mid_download_stops_before_the_next_swap() {
        let dir = tempfile::tempdir().unwrap();
        let inst = install(&dir, InstallSource::Official);
        let mut env = FakeEnv::new(&[
            ("Base", "Bin/a.dll", b"new a"),
            ("Base", "Bin/b.dll", b"new b"),
        ]);
        env.busy_after = Some(format!("{PREFIX}/Bin/b.dll"));
        put(dir.path(), "Bin/a.dll", b"old a");
        put(dir.path(), "Bin/b.dll", b"old b");
        let entries = env.entries();

        let (result, statuses) = run(&inst, &env).await;
        assert_eq!(result, Err(PatchFailure::GameOpen));
        assert_eq!(fs::read(dir.path().join("Bin/a.dll")).unwrap(), b"new a");
        assert_eq!(record(dir.path(), &entries[0]).unwrap().crc, entries[0].crc);
        assert_eq!(fs::read(dir.path().join("Bin/b.dll")).unwrap(), b"old b");
        assert!(record(dir.path(), &entries[1]).is_none());
        assert!(!dir.path().join("Bin/b.dll.w101tmp").exists());
        assert_terminal(&statuses, PatchStatus::Failed(PatchFailure::GameOpen));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unsaveable_cache_does_not_fail_a_passed_check() {
        // 同名資料夾擋住紀錄暫存檔的寫入或清單副本的改名
        let blockers: [fn(&Path) -> PathBuf; 2] = [
            |root| record_path(root, "Base").with_extension("dat.w101tmp"),
            list_copy,
        ];
        for blocker in blockers {
            let dir = tempfile::tempdir().unwrap();
            let inst = install(&dir, InstallSource::Official);
            let env = FakeEnv::new(&[("Base", "Bin/a.dll", b"new!")]);
            put(dir.path(), "Bin/a.dll", b"old");
            fs::create_dir_all(blocker(dir.path()).join("x")).unwrap();

            let (result, statuses) = run(&inst, &env).await;
            assert!(result.unwrap().downloaded);
            assert_eq!(fs::read(dir.path().join("Bin/a.dll")).unwrap(), b"new!");
            assert!(!list_temp(dir.path()).exists());
            assert_terminal(&statuses, PatchStatus::Updated);
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stale_temps_are_removed_on_official_installs_only() {
        for (source, removed) in [
            (InstallSource::Official, true),
            (InstallSource::Steam, false),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let inst = install(&dir, source);
            let env = FakeEnv::new(&[("Base", "Bin/a.dll", b"aaa")]);
            put(dir.path(), "Bin/a.dll", b"aaa");
            let temps = [
                put(dir.path(), "Bin/a.dll.w101tmp", b"x"),
                put(dir.path(), "Bin/a.dll.w101tmp2", b"x"),
            ];

            let (result, _) = run(&inst, &env).await;
            assert!(!result.unwrap().downloaded);
            for temp in temps {
                assert_eq!(temp.exists(), !removed, "{source:?} {}", temp.display());
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn local_list_copy_is_reused_when_crc_matches() {
        let dir = tempfile::tempdir().unwrap();
        let inst = install(&dir, InstallSource::Official);
        let env = FakeEnv::new(&[("Base", "Bin/a.dll", b"aaa")]);
        put(dir.path(), r"PatchInfo\LatestFileList.bin", env.list());
        put(dir.path(), "Bin/a.dll", b"aaa");

        let (result, statuses) = run(&inst, &env).await;
        assert!(!result.unwrap().downloaded);
        assert!(env.fetched().is_empty());
        assert_terminal(&statuses, PatchStatus::UpToDate);
    }
}
