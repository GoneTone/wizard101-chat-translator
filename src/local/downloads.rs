//! 模型下載：依 repo 查詢結果寫清單、下載檔案、暫停／繼續與刪除。

use std::sync::atomic::Ordering;

use tokio_util::sync::CancellationToken;

use super::download::{self, DownloadError, Progress};
use super::fit;
use super::hf::{self, RepoFile};
use super::hub::{DownloadState, Local, PROGRESS_TICK, RepoInfo};
use super::store::{self, Manifest, ManifestFile};
use crate::logs::log;
use crate::util::lock;

impl Local {
    /// 下載 repo 的 `choice`（與 `mtp`）；這次才裝好引擎時改用量到 VRAM 後的自動挑選。
    pub(super) async fn download(&self, repo: &str, choice: usize, mtp: Option<usize>) {
        if self.repo_info(repo).is_none() {
            self.query_repo(repo).await;
        }
        let Some((mut id, mut manifest)) = self
            .repo_info(repo)
            .and_then(|info| manifest_for(repo, &info, choice, mtp))
        else {
            log(format!(
                "[local] download {repo}: no repo info for choice {choice} / mtp {mtp:?}"
            ));
            return;
        };
        let Some(cancel) = self.claim(&id) else {
            log(format!("[local] download {id} already running"));
            return;
        };
        match self.ensure_engine().await {
            Ok(false) => {}
            // 引擎裝好前量不到 VRAM，使用者看到的相容等級是「未知」；裝好時 refresh_hardware 已重算
            Ok(true) => {
                let auto = self.repo_info(repo).and_then(|info| {
                    let pick = info.auto?;
                    manifest_for(repo, &info, pick.choice, pick.mtp)
                });
                if let Some((auto_id, auto_manifest)) = auto {
                    if auto_id != id && !self.rename_claim(&id, &auto_id) {
                        log(format!("[local] download {auto_id} already running"));
                        self.release(&id);
                        return self.update(|s| {
                            s.downloads.remove(&id);
                        });
                    }
                    log(format!(
                        "[local] download {repo}: picked {auto_id} after measuring VRAM (requested {id})"
                    ));
                    (id, manifest) = (auto_id, auto_manifest);
                }
            }
            Err(e) => {
                self.download_failed(&id, e);
                return self.release(&id);
            }
        }
        match store::write_manifest(&self.root, &id, &manifest) {
            Ok(()) => {
                self.update(|s| s.models = store::list(&self.root));
                self.fetch_model(&id, &cancel).await;
            }
            Err(e) => self.download_failed(&id, format!("writing manifest failed: {e}")),
        }
        self.release(&id);
    }

    /// 依磁碟上的清單繼續下載；引擎沒裝的話先裝。
    pub(super) async fn resume(&self, id: &str) {
        let Some(cancel) = self.claim(id) else {
            return;
        };
        match self.ensure_engine().await {
            Ok(_) => self.fetch_model(id, &cancel).await,
            Err(e) => self.download_failed(id, e),
        }
        self.release(id);
    }

    /// 登記 `id` 的下載；已在下載中回 `None`。
    fn claim(&self, id: &str) -> Option<CancellationToken> {
        let token = {
            let mut cancels = lock(&self.cancels);
            if cancels.contains_key(id) {
                return None;
            }
            let token = CancellationToken::new();
            cancels.insert(id.to_owned(), token.clone());
            token
        };
        self.update(|s| {
            s.downloads
                .insert(id.to_owned(), DownloadState::Running { done: 0, total: 0 });
        });
        Some(token)
    }

    /// 把下載登記從 `from` 改到 `to`；`to` 已在下載中回 `false`。
    fn rename_claim(&self, from: &str, to: &str) -> bool {
        {
            let mut cancels = lock(&self.cancels);
            if cancels.contains_key(to) {
                return false;
            }
            if let Some(token) = cancels.remove(from) {
                cancels.insert(to.to_owned(), token);
            }
        }
        self.update(|s| {
            if let Some(state) = s.downloads.remove(from) {
                s.downloads.insert(to.to_owned(), state);
            }
        });
        true
    }

    fn release(&self, id: &str) {
        lock(&self.cancels).remove(id);
    }

    /// 依清單下載缺少的檔案；已完整的檔略過。
    async fn fetch_model(&self, id: &str, cancel: &CancellationToken) {
        let Some(model) = store::list(&self.root).into_iter().find(|m| m.id == id) else {
            return self.download_failed(id, format!("model {id} has no manifest"));
        };
        let m = &model.manifest;
        let files: Vec<&ManifestFile> = m.files.iter().chain(&m.mtp).collect();
        let total: u64 = files.iter().map(|f| f.size).sum();
        let dir = store::models_dir(&self.root).join(&model.id);
        log(format!(
            "[local] model download start: id={id} repo={} files={} bytes={total}",
            m.repo,
            files.len()
        ));
        let mut done = 0;
        for f in files {
            let Some(name) = f.local_name() else {
                return self.download_failed(id, format!("invalid file path {}", f.path));
            };
            let dest = dir.join(name);
            if std::fs::metadata(&dest).is_ok_and(|md| md.len() == f.size) {
                done += f.size;
                continue;
            }
            let url = hf::resolve_url(&self.endpoints.hf, &m.repo, &f.path);
            let progress = Progress::default();
            let fetch = download::download(
                &self.hf_http,
                &url,
                &dest,
                f.size,
                f.sha256.as_deref(),
                &progress,
                cancel,
            );
            let result = self
                .track(&progress, fetch, |s, d, _| {
                    s.downloads.insert(
                        id.to_owned(),
                        DownloadState::Running {
                            done: done + d,
                            total,
                        },
                    );
                })
                .await;
            match result {
                Ok(()) => done += f.size,
                Err(DownloadError::Cancelled) => {
                    let done = done + progress.done.load(Ordering::Relaxed);
                    log(format!(
                        "[local] model download paused: id={id} done={done}"
                    ));
                    return self.update(|s| {
                        s.downloads
                            .insert(id.to_owned(), DownloadState::Paused { done, total });
                    });
                }
                Err(e) => return self.download_failed(id, e.to_string()),
            }
        }
        log(format!("[local] model download finished: id={id}"));
        self.clear_failures(id);
        self.update(|s| {
            s.downloads.remove(id);
            s.models = store::list(&self.root);
        });
    }

    fn download_failed(&self, id: &str, error: String) {
        log(format!("[local] model download failed: id={id}: {error}"));
        self.update(|s| {
            s.downloads
                .insert(id.to_owned(), DownloadState::Failed(error));
        });
    }

    /// 先取消進行中的下載並等它放開 `.part`、停掉子程序並等它結束，再刪資料夾。
    pub(super) async fn delete(&self, id: &str) {
        if let Some(cancel) = lock(&self.cancels).get(id) {
            cancel.cancel();
        }
        while lock(&self.cancels).contains_key(id) {
            tokio::time::sleep(PROGRESS_TICK).await;
        }
        self.stop_and_wait(id).await;
        let result = store::delete(&self.root, id);
        self.update(|s| {
            match result {
                Ok(()) => {
                    log(format!("[local] deleted model {id}"));
                    s.downloads.remove(id);
                }
                Err(e) => {
                    log(format!("[local] deleting model {id} failed: {e}"));
                    s.downloads
                        .insert(id.to_owned(), DownloadState::Failed(e.to_string()));
                }
            }
            s.models = store::list(&self.root);
        });
    }
}

/// 依 repo 查詢結果組出（模型 id, 清單）；索引超出範圍回 `None`。
fn manifest_for(
    repo: &str,
    info: &RepoInfo,
    choice: usize,
    mtp: Option<usize>,
) -> Option<(String, Manifest)> {
    let main = info.catalog.choices.get(choice)?;
    let mtp_file = match mtp {
        Some(i) => Some(&info.catalog.mtp.get(i)?.file),
        None => None,
    };
    let entry = |f: &RepoFile| ManifestFile {
        path: f.path.clone(),
        size: f.size,
        sha256: f.sha256.clone(),
    };
    let manifest = Manifest {
        repo: repo.to_owned(),
        quant: main.quant.label.clone(),
        files: main.files.iter().map(entry).collect(),
        mtp: mtp_file.map(entry),
        moe: info.meta.expert_count > 0,
        has_builtin_mtp: info.meta.has_mtp,
        required: fit::required(
            &info.meta,
            fit::total_size(main),
            mtp_file.map_or(0, |f| f.size),
        ),
    };
    Some((store::model_id(repo, &main.quant.label), manifest))
}
