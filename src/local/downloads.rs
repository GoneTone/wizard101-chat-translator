//! 模型下載：依 repo 查詢結果寫清單、下載檔案、暫停／繼續與刪除。

use std::sync::atomic::Ordering;

use tokio_util::sync::CancellationToken;

use super::download::{self, DownloadError, Progress};
use super::fit;
use super::hf::{self, RepoFile};
use super::hub::{DownloadState, Local, PROGRESS_TICK, RepoInfo, rerate};
use super::store::{self, Manifest, ManifestFile};
use crate::i18n::t;
use crate::logs::log;
use crate::util::lock;

enum Rename {
    Done,
    Cancelled,
    Busy,
}

/// 已登記的下載：模型 id、清單與取消鈕。
pub(super) type Claimed = (String, Manifest, CancellationToken);

impl Local {
    /// 依 repo 查詢結果組出 `choice`（與 `mtp`）的清單並登記下載；沒有查詢結果或已在下載中回 `None`。
    pub(super) fn claim_download(
        &self,
        repo: &str,
        choice: usize,
        mtp: Option<usize>,
    ) -> Option<Claimed> {
        let Some((id, manifest)) = self
            .repo_info(repo)
            .and_then(|info| manifest_for(repo, &info, choice, mtp))
        else {
            log(format!(
                "[local] download {repo}: no repo info for choice {choice} / mtp {mtp:?}"
            ));
            return None;
        };
        let Some(cancel) = self.claim(&id) else {
            log(format!("[local] download {id} already running"));
            return None;
        };
        Some((id, manifest, cancel))
    }

    /// 下載已登記的模型；`auto` 且這次才裝好引擎、量到 VRAM 時改用量完後的自動挑選。
    pub(super) async fn download(&self, repo: &str, auto: bool, claimed: Claimed) {
        let (mut id, mut manifest, cancel) = claimed;
        // 先寫清單：引擎安裝失敗或途中被取消時，「繼續下載」仍有清單可依
        let existed = store::list(&self.root).iter().any(|m| m.id == id);
        if let Err(e) = self.write_manifest(&id, &manifest) {
            self.download_failed(&id, e);
            return self.release(&id);
        }
        match self.ensure_engine().await {
            Ok(false) => {}
            Ok(true) if !auto => log(format!(
                "[local] download {id}: keeping the manually picked quant"
            )),
            Ok(true) if !lock(&self.state).hardware.measured => log(format!(
                "[local] download {id}: VRAM measurement failed, keeping the requested choice"
            )),
            // 引擎裝好前量不到 VRAM，使用者看到的相容等級是「未知」；裝好時 refresh_hardware 已重算
            Ok(true) => {
                let auto = self.repo_info(repo).and_then(|info| {
                    let pick = info.auto?;
                    manifest_for(repo, &info, pick.choice, pick.mtp)
                });
                if let Some((auto_id, auto_manifest)) = auto.filter(|(a, _)| *a != id) {
                    match self.rename_claim(&id, &auto_id) {
                        Rename::Done => {}
                        Rename::Cancelled => return self.pause_unstarted(&id, &manifest),
                        Rename::Busy => {
                            log(format!("[local] download {auto_id} already running"));
                            self.drop_provisional(&id, existed);
                            self.release(&id);
                            return self.update(|s| {
                                s.downloads.remove(&id);
                            });
                        }
                    }
                    log(format!(
                        "[local] download {repo}: picked {auto_id} after measuring VRAM (requested {id})"
                    ));
                    self.drop_provisional(&id, existed);
                    (id, manifest) = (auto_id, auto_manifest);
                    if let Err(e) = self.write_manifest(&id, &manifest) {
                        self.download_failed(&id, e);
                        return self.release(&id);
                    }
                }
            }
            Err(e) => {
                self.download_failed(&id, e);
                return self.release(&id);
            }
        }
        if cancel.is_cancelled() {
            return self.pause_unstarted(&id, &manifest);
        }
        self.fetch_model(&id, &cancel).await;
        self.release(&id);
    }

    fn write_manifest(&self, id: &str, manifest: &Manifest) -> Result<(), String> {
        let _manifests = lock(&self.manifests);
        store::write_manifest(&self.root, id, manifest)
            .map_err(|e| format!("writing manifest failed: {e}"))?;
        // 重新下載同一個 id 會清掉它的實測
        self.update(|s| {
            s.models = store::list(&self.root);
            rerate(s);
        });
        Ok(())
    }

    /// 改挑別的量化後，刪掉為原本選擇先寫的清單（原本就有的模型不動）。
    fn drop_provisional(&self, id: &str, existed: bool) {
        if existed {
            return;
        }
        if let Err(e) = store::delete(&self.root, id) {
            log(format!(
                "[local] removing provisional manifest {id} failed: {e}"
            ));
        }
        self.update(|s| s.models = store::list(&self.root));
    }

    /// 還沒開始抓檔就被取消（例如等引擎時）：記為 Paused 並放掉登記，刪除可接著進行。
    fn pause_unstarted(&self, id: &str, manifest: &Manifest) {
        let total = manifest
            .files
            .iter()
            .chain(&manifest.mtp)
            .map(|f| f.size)
            .sum();
        log(format!(
            "[local] download {id} cancelled before fetching files"
        ));
        self.update(|s| {
            s.downloads
                .insert(id.to_owned(), DownloadState::Paused { done: 0, total });
        });
        self.release(id);
    }

    /// 依磁碟上的清單繼續已登記的下載；引擎沒裝的話先裝。
    pub(super) async fn resume(&self, id: &str, cancel: CancellationToken) {
        match self.ensure_engine().await {
            Ok(_) => self.fetch_model(id, &cancel).await,
            Err(e) => self.download_failed(id, e),
        }
        self.release(id);
    }

    /// 登記 `id` 的下載（快照記為下載中）；已在下載中回 `None`。
    pub(super) fn claim(&self, id: &str) -> Option<CancellationToken> {
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

    /// 把下載登記從 `from` 改到 `to`；`from` 已被取消或 `to` 已在下載中則不改。
    fn rename_claim(&self, from: &str, to: &str) -> Rename {
        {
            let mut cancels = lock(&self.cancels);
            // 與 CancelDownload／Delete 取消同一把鎖，兩者不會交錯
            if cancels.get(from).is_some_and(|t| t.is_cancelled()) {
                return Rename::Cancelled;
            }
            if cancels.contains_key(to) {
                return Rename::Busy;
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
        Rename::Done
    }

    fn release(&self, id: &str) {
        lock(&self.cancels).remove(id);
    }

    /// 依清單下載缺少的檔案；已完整的檔略過，磁碟上沒有清單則記為失敗。
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
        let shown = t("local.error.download", &[("error", &error)]);
        self.update(|s| {
            s.downloads
                .insert(id.to_owned(), DownloadState::Failed(shown));
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
        let _hold = self.stop_and_wait(id).await;
        let result = store::delete(&self.root, id);
        self.update(|s| {
            match result {
                Ok(()) => {
                    log(format!("[local] deleted model {id}"));
                    s.downloads.remove(id);
                }
                Err(e) => {
                    log(format!("[local] deleting model {id} failed: {e}"));
                    s.downloads.insert(
                        id.to_owned(),
                        DownloadState::Failed(t(
                            "local.error.delete",
                            &[("error", &e.to_string())],
                        )),
                    );
                }
            }
            s.models = store::list(&self.root);
            // 刪掉的可能是同 repo 其他量化推算用的實測
            rerate(s);
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
        measured: None,
        layers: info.meta.block_count,
    };
    Some((store::choice_id(repo, main), manifest))
}
