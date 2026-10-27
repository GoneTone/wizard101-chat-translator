//! 已下載模型的磁碟存放：`<root>/models/<id>/` 內一份 `manifest.json` 加上 GGUF 檔。

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::catalog::RECOMMENDED;
use super::gguf::ModelMeta;
use super::hf::Choice;
use crate::logs::log;
use crate::util::write_atomic;

const MANIFEST_NAME: &str = "manifest.json";
const HEADERS_DIR: &str = "gguf_headers";

/// 清單中的一個檔案；`path` 為 repo 內路徑，本地檔名取 basename。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ManifestFile {
    pub path: String,
    pub size: u64,
    pub sha256: Option<String>,
}

impl ManifestFile {
    /// 本地檔名（repo 路徑的 basename）。
    pub fn local_name(&self) -> Option<&str> {
        Path::new(&self.path).file_name()?.to_str()
    }
}

/// 一個已下載（或下載中）模型的清單。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub repo: String,
    pub quant: String,
    pub files: Vec<ManifestFile>,
    pub mtp: Option<ManifestFile>,
    pub moe: bool,
    pub has_builtin_mtp: bool,
    /// 預估所需位元組（沒載入過時判斷相容用）。
    pub required: u64,
    /// 載入後量到的實際佔用；沒載入過為 `None`。
    // 舊鍵 `measured` 的 RAM 含已搬上 GPU 的對映分頁、偏高，換鍵名讓它作廢
    #[serde(default, rename = "measurement")]
    pub measured: Option<Measured>,
    /// 主模型的層數（GGUF 的 `block_count`）；較早的清單沒有，為 0。
    #[serde(default)]
    pub layers: u32,
}

/// 一次載入實際量到的佔用（位元組），以及那次的載入方式。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Measured {
    pub vram: u64,
    pub ram: u64,
    /// 每一層都放在 GPU 上（llama.cpp 的 `load_tensors: offloaded n/n`，且 `--fit` 沒把張量移走）。
    pub all_on_gpu: bool,
    /// 那次有開 MTP。
    pub mtp: bool,
    /// 那次用的引擎（`{build}-{variant}`）；較早的紀錄沒有，為空字串。
    #[serde(default)]
    pub engine: String,
}

impl Measured {
    /// 與上一筆合併：同一個引擎、同樣的 MTP，且兩次都全放 GPU 時各取最大值（第一次請求後還會長）；否則整筆換成這次的。
    ///
    /// 部分卸載的那幾次 `--fit` 擺放可能不同，VRAM 與 RAM 不能各取各的最大值。
    pub fn merged(self, old: Option<&Measured>) -> Self {
        match old {
            Some(o)
                if self.all_on_gpu
                    && o.all_on_gpu
                    && o.mtp == self.mtp
                    && o.engine == self.engine =>
            {
                Self {
                    vram: self.vram.max(o.vram),
                    ram: self.ram.max(o.ram),
                    ..self
                }
            }
            _ => self,
        }
    }
}

impl Manifest {
    /// 有 MTP 可開（另附的 MTP 檔或模型內建）。
    pub fn has_mtp(&self) -> bool {
        self.mtp.is_some() || self.has_builtin_mtp
    }

    /// 一層大約多少位元組（主模型大小 ÷ 層數）；層數不明為 0。
    pub fn layer_bytes(&self) -> u64 {
        let main: u64 = self.files.iter().map(|f| f.size).sum();
        main.checked_div(u64::from(self.layers)).unwrap_or(0)
    }

    /// 主模型分檔加上（`mtp` 時）MTP 檔的大小。
    pub fn weights(&self, mtp: bool) -> u64 {
        let extra = self.mtp.as_ref().filter(|_| mtp).map_or(0, |f| f.size);
        self.files.iter().map(|f| f.size).sum::<u64>() + extra
    }

    /// 顯示名：推薦清單內的 repo 用推薦名稱，其餘用 repo 名；後接量化。
    pub fn display_name(&self) -> String {
        let name = RECOMMENDED
            .iter()
            .find(|r| r.repo == self.repo)
            .map_or_else(
                || self.repo.rsplit('/').next().unwrap_or_default(),
                |r| r.name,
            );
        format!("{name} · {}", self.quant)
    }
}

/// 磁碟上的一個模型；`complete` 表示所有檔都已下載完成。
#[derive(Clone, Debug, PartialEq)]
pub struct Installed {
    pub id: String,
    pub manifest: Manifest,
    pub complete: bool,
}

/// 由 repo 與量化標籤組出可當資料夾名稱的 id。
pub fn model_id(repo: &str, quant_label: &str) -> String {
    format!("{}__{}", repo.replace('/', "__"), quant_label)
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect()
}

/// 一個量化版本的模型 id；主檔不在 repo 根目錄時加上所在資料夾，不同資料夾的同名量化才不會撞號。
pub fn choice_id(repo: &str, choice: &Choice) -> String {
    let dir = choice
        .files
        .first()
        .and_then(|f| f.path.rsplit_once('/'))
        .map(|(dir, _)| dir);
    match dir {
        Some(dir) => model_id(repo, &format!("{}__{dir}", choice.quant.label)),
        None => model_id(repo, &choice.quant.label),
    }
}

/// 所有模型的根資料夾。
pub fn models_dir(root: &Path) -> PathBuf {
    root.join("models")
}

/// 讀過的 GGUF 檔頭（以檔案的 sha256 為鍵）：同一個檔案的形狀不會變，重開程式不必再下載檔頭。
pub fn cached_header(root: &Path, sha256: &str) -> Option<ModelMeta> {
    let text = fs::read_to_string(header_path(root, sha256)?).ok()?;
    serde_json::from_str(&text).ok()
}

/// 記下讀到的 GGUF 檔頭；寫不進去只記 log，下次重新下載。
pub fn cache_header(root: &Path, sha256: &str, meta: &ModelMeta) {
    let Some(path) = header_path(root, sha256) else {
        return;
    };
    let written = serde_json::to_string(meta)
        .map_err(io::Error::other)
        .and_then(|text| write_atomic(&path, &text));
    if let Err(e) = written {
        log(format!(
            "[local] caching the GGUF header {sha256} failed: {e}"
        ));
    }
}

/// 檔頭快取的路徑；sha256 不是 64 位 hex（來自網路）時為 `None`。
fn header_path(root: &Path, sha256: &str) -> Option<PathBuf> {
    (sha256.len() == 64 && sha256.bytes().all(|b| b.is_ascii_hexdigit())).then(|| {
        models_dir(root)
            .join(HEADERS_DIR)
            .join(format!("{sha256}.json"))
    })
}

/// id 必須恰為單一路徑元件，避免 config 手改後跳出 `models/`。
fn is_valid_id(id: &str) -> bool {
    let mut parts = Path::new(id).components();
    matches!(
        (parts.next(), parts.next()),
        (Some(Component::Normal(n)), None) if n == id
    )
}

fn model_dir(root: &Path, id: &str) -> io::Result<PathBuf> {
    if !is_valid_id(id) {
        log(format!("[local] rejected invalid model id={id:?}"));
        return Err(io::ErrorKind::InvalidInput.into());
    }
    Ok(models_dir(root).join(id))
}

/// 寫入（覆蓋）模型的 `manifest.json`，必要時建立資料夾。
pub fn write_manifest(root: &Path, id: &str, manifest: &Manifest) -> io::Result<()> {
    let text = serde_json::to_string_pretty(manifest).map_err(io::Error::other)?;
    write_atomic(&model_dir(root, id)?.join(MANIFEST_NAME), &text)
}

fn file_complete(dir: &Path, f: &ManifestFile) -> bool {
    let Some(name) = f.local_name() else {
        return false;
    };
    fs::metadata(dir.join(name)).is_ok_and(|m| m.len() == f.size)
        && !dir.join(format!("{name}.part")).exists()
}

/// 列出磁碟上的模型（依 id 排序）；損壞的清單略過並記 log。
pub fn list(root: &Path) -> Vec<Installed> {
    let Ok(entries) = fs::read_dir(models_dir(root)) else {
        return Vec::new();
    };
    let mut out: Vec<Installed> = entries
        .flatten()
        .filter_map(|e| {
            let dir = e.path();
            let id = e.file_name().to_string_lossy().into_owned();
            let text = match fs::read_to_string(dir.join(MANIFEST_NAME)) {
                Ok(t) => t,
                Err(err) => {
                    if err.kind() != io::ErrorKind::NotFound {
                        log(format!("[local] skip unreadable manifest id={id}: {err}"));
                    }
                    return None;
                }
            };
            let manifest: Manifest = match serde_json::from_str(&text) {
                Ok(m) => m,
                Err(err) => {
                    log(format!("[local] skip corrupt manifest id={id}: {err}"));
                    return None;
                }
            };
            let complete = !manifest.files.is_empty()
                && manifest
                    .files
                    .iter()
                    .chain(&manifest.mtp)
                    .all(|f| file_complete(&dir, f));
            Some(Installed {
                id,
                manifest,
                complete,
            })
        })
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// 模型的主 GGUF（第一個分檔）；id 不合法或清單沒有可用的檔名時回錯誤。
pub fn main_file(root: &Path, m: &Installed) -> io::Result<PathBuf> {
    let name = m.manifest.files.first().and_then(|f| f.local_name());
    let name = name.ok_or(io::ErrorKind::InvalidData)?;
    Ok(model_dir(root, &m.id)?.join(name))
}

/// 模型的 MTP 檔；沒有 MTP 或 id 不合法時為 `None`。
pub fn mtp_file(root: &Path, m: &Installed) -> Option<PathBuf> {
    let name = m.manifest.mtp.as_ref()?.local_name()?;
    Some(model_dir(root, &m.id).ok()?.join(name))
}

/// 刪除模型資料夾；不存在視為成功。
pub fn delete(root: &Path, id: &str) -> io::Result<()> {
    match fs::remove_dir_all(model_dir(root, id)?) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// 清單內所有模型的清單大小（含 MTP）總和。
pub fn total_size(list: &[Installed]) -> u64 {
    list.iter()
        .flat_map(|m| m.manifest.files.iter().chain(&m.manifest.mtp))
        .map(|f| f.size)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 檔頭快取存得回來；sha256 不是 64 位 hex（例如帶路徑字元）時不讀也不寫。
    #[test]
    fn header_cache_round_trips_and_rejects_odd_keys() {
        let dir = tempfile::tempdir().unwrap();
        let meta = ModelMeta {
            arch: "x".into(),
            block_count: 3,
            ..ModelMeta::default()
        };
        let sha = "0f".repeat(32);
        cache_header(dir.path(), &sha, &meta);
        assert_eq!(cached_header(dir.path(), &sha), Some(meta.clone()));
        for odd in [
            "../escape",
            &"0f".repeat(31),
            &format!("{}..", "0f".repeat(31)),
        ] {
            cache_header(dir.path(), odd, &meta);
            assert_eq!(cached_header(dir.path(), odd), None);
        }
        let saved = fs::read_dir(models_dir(dir.path()).join(HEADERS_DIR)).unwrap();
        assert_eq!(saved.count(), 1);
    }

    fn file(path: &str, size: u64) -> ManifestFile {
        ManifestFile {
            path: path.into(),
            size,
            sha256: None,
        }
    }

    fn manifest() -> Manifest {
        Manifest {
            repo: "a/b".into(),
            quant: "Q4".into(),
            files: vec![file("sub/m-1.gguf", 3), file("m-2.gguf", 2)],
            mtp: Some(file("mtp.gguf", 1)),
            moe: false,
            has_builtin_mtp: false,
            required: 6,
            measured: None,
            layers: 0,
        }
    }

    #[test]
    fn model_id_is_a_safe_folder_name() {
        assert_eq!(
            model_id("unsloth/Qwen3.5-4B-GGUF", "Q4_K_M"),
            "unsloth__Qwen3.5-4B-GGUF__Q4_K_M"
        );
        assert_eq!(model_id("a/b", "x:y"), "a__b__x_y");
    }

    #[test]
    fn partial_files_mark_the_model_incomplete() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = model_dir(tmp.path(), "m").unwrap();
        write_manifest(tmp.path(), "m", &manifest()).unwrap();
        fs::write(dir.join("m-1.gguf"), b"abc").unwrap();
        fs::write(dir.join("m-2.gguf.part"), b"x").unwrap();
        fs::write(dir.join("mtp.gguf"), b"z").unwrap();
        assert!(!list(tmp.path())[0].complete);

        fs::remove_file(dir.join("m-2.gguf.part")).unwrap();
        fs::write(dir.join("m-2.gguf"), b"xy").unwrap();
        let l = list(tmp.path());
        assert!(l[0].complete);
        assert_eq!(total_size(&l), 6);
        assert_eq!(main_file(tmp.path(), &l[0]).unwrap(), dir.join("m-1.gguf"));
        assert_eq!(mtp_file(tmp.path(), &l[0]), Some(dir.join("mtp.gguf")));
    }

    /// id 不合法時主檔回錯誤、MTP 檔為 `None`，不組出 `models/` 之外的路徑。
    #[test]
    fn file_paths_reject_invalid_ids() {
        let tmp = tempfile::tempdir().unwrap();
        let bad = Installed {
            id: "../x".into(),
            manifest: manifest(),
            complete: true,
        };
        let err = main_file(tmp.path(), &bad).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(mtp_file(tmp.path(), &bad), None);
    }

    /// 主檔在子資料夾時 id 帶上資料夾，根目錄的維持原本的 id。
    #[test]
    fn choice_id_adds_the_folder_only_outside_the_repo_root() {
        use super::super::hf::{Quant, RepoFile};
        let choice = |path: &str| Choice {
            quant: Quant {
                label: "Q4_K_M".into(),
                bits: Some(4),
            },
            files: vec![RepoFile {
                path: path.into(),
                size: 1,
                sha256: None,
            }],
        };
        assert_eq!(
            choice_id("a/b", &choice("m-Q4_K_M.gguf")),
            model_id("a/b", "Q4_K_M")
        );
        let (x, y) = (
            choice_id("a/b", &choice("x/m-Q4_K_M.gguf")),
            choice_id("a/b", &choice("y/z/m-Q4_K_M.gguf")),
        );
        assert_eq!(x, "a__b__Q4_K_M__x");
        assert_eq!(y, "a__b__Q4_K_M__y_z");
        assert!(is_valid_id(&y));
    }

    /// 只有同引擎、同 MTP、兩次都全放 GPU 才各取最大值；其餘整筆換掉。
    #[test]
    fn measurements_merge_only_for_the_same_all_on_gpu_mode() {
        let m = |vram, ram, all_on_gpu, mtp, engine: &str| Measured {
            vram,
            ram,
            all_on_gpu,
            mtp,
            engine: engine.into(),
        };
        let old = m(3, 1, true, true, "b1-cuda");
        assert_eq!(
            m(2, 2, true, true, "b1-cuda").merged(Some(&old)),
            m(3, 2, true, true, "b1-cuda")
        );
        for new in [
            m(2, 2, false, true, "b1-cuda"),
            m(2, 2, true, false, "b1-cuda"),
            m(2, 2, true, true, "b2-cuda"),
        ] {
            assert_eq!(new.clone().merged(Some(&old)), new);
        }
        let partial = m(5, 9, false, true, "b1-cuda");
        let again = m(6, 4, false, true, "b1-cuda");
        assert_eq!(again.clone().merged(Some(&partial)), again, "partial runs");
        assert_eq!(old.clone().merged(None), old);
    }

    /// 推薦清單內的 repo 用推薦名稱，其餘用 repo 名。
    #[test]
    fn display_name_prefers_the_recommended_name() {
        let mut m = manifest();
        assert_eq!(m.display_name(), "b · Q4");
        m.repo = RECOMMENDED[0].repo.into();
        assert_eq!(m.display_name(), format!("{} · Q4", RECOMMENDED[0].name));
    }

    #[test]
    fn wrong_size_is_incomplete() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = model_dir(tmp.path(), "m").unwrap();
        write_manifest(tmp.path(), "m", &manifest()).unwrap();
        for (n, c) in [("m-1.gguf", "abc"), ("m-2.gguf", "x"), ("mtp.gguf", "z")] {
            fs::write(dir.join(n), c).unwrap();
        }
        assert!(!list(tmp.path())[0].complete);
    }

    #[test]
    fn list_skips_corrupt_and_manifestless_folders_and_sorts() {
        let tmp = tempfile::tempdir().unwrap();
        write_manifest(tmp.path(), "b", &manifest()).unwrap();
        write_manifest(tmp.path(), "a", &manifest()).unwrap();
        let bad = model_dir(tmp.path(), "bad").unwrap();
        fs::create_dir_all(&bad).unwrap();
        fs::write(bad.join(MANIFEST_NAME), "{nope").unwrap();
        fs::create_dir_all(model_dir(tmp.path(), "empty").unwrap()).unwrap();
        let ids: Vec<_> = list(tmp.path()).into_iter().map(|m| m.id).collect();
        assert_eq!(ids, ["a", "b"]);
    }

    #[test]
    fn invalid_ids_are_rejected_and_touch_nothing_outside() {
        let tmp = tempfile::tempdir().unwrap();
        let sibling = tmp.path().join("config.json");
        fs::write(&sibling, "{}").unwrap();
        write_manifest(tmp.path(), "keep", &manifest()).unwrap();
        for id in ["..", ".", "", "a/b", r"a\b", r"C:\x", "../x"] {
            let e = delete(tmp.path(), id).unwrap_err();
            assert_eq!(e.kind(), io::ErrorKind::InvalidInput, "{id:?}");
            let e = write_manifest(tmp.path(), id, &manifest()).unwrap_err();
            assert_eq!(e.kind(), io::ErrorKind::InvalidInput, "{id:?}");
        }
        assert!(sibling.exists());
        assert_eq!(list(tmp.path()).len(), 1);
    }

    #[test]
    fn hostile_file_paths_and_empty_files_are_incomplete() {
        let tmp = tempfile::tempdir().unwrap();
        let mut m = manifest();
        m.files = vec![];
        m.mtp = None;
        write_manifest(tmp.path(), "e", &m).unwrap();
        m.files = vec![file("..", 0)];
        write_manifest(tmp.path(), "h", &m).unwrap();
        assert!(list(tmp.path()).iter().all(|i| !i.complete));
    }

    #[test]
    fn delete_removes_the_folder() {
        let tmp = tempfile::tempdir().unwrap();
        write_manifest(tmp.path(), "m", &manifest()).unwrap();
        delete(tmp.path(), "m").unwrap();
        assert!(list(tmp.path()).is_empty());
        delete(tmp.path(), "m").unwrap();
    }
}
