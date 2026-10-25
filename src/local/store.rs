//! 已下載模型的磁碟存放：`<root>/models/<id>/` 內一份 `manifest.json` 加上 GGUF 檔。

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::logs::log;
use crate::util::write_atomic;

const MANIFEST_NAME: &str = "manifest.json";

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
    pub required: u64,
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

/// 所有模型的根資料夾。
pub fn models_dir(root: &Path) -> PathBuf {
    root.join("models")
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

/// 模型的主 GGUF（第一個分檔）。
pub fn main_file(root: &Path, m: &Installed) -> PathBuf {
    debug_assert!(is_valid_id(&m.id));
    let name = m.manifest.files.first().and_then(|f| f.local_name());
    models_dir(root).join(&m.id).join(name.unwrap_or_default())
}

/// 模型的 MTP 檔（若有）。
pub fn mtp_file(root: &Path, m: &Installed) -> Option<PathBuf> {
    debug_assert!(is_valid_id(&m.id));
    let name = m.manifest.mtp.as_ref()?.local_name()?;
    Some(models_dir(root).join(&m.id).join(name))
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
        assert_eq!(main_file(tmp.path(), &l[0]), dir.join("m-1.gguf"));
        assert_eq!(mtp_file(tmp.path(), &l[0]), Some(dir.join("mtp.gguf")));
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
