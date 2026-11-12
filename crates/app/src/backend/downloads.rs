use super::{runtime, Backend};
use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use gtnh_core::downloads::{CancellationToken, DownloadProgress};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime},
};

/// Lifecycle of one download job.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DownloadState {
    Downloading,
    Completed,
    Cancelled,
    // Tombstone telling the UI to drop the job; never stored in the jobs map.
    Deleted,
    Failed,
}

/// Snapshot of one download job. `revision` grows monotonically across the whole store; views keep
/// the highest revision seen per `url` (see `logic::DownloadTracker`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadStatus {
    pub id: String,
    pub revision: u64,
    pub url: String,
    pub state: DownloadState,
    pub received: u64,
    pub total: Option<u64>,
    pub path: Option<String>,
    pub error: Option<String>,
}

struct Job {
    status: DownloadStatus,
    cancel: Option<CancellationToken>,
}
#[derive(Default)]
struct Inner {
    revision: u64,
    // Downloads directory holding index.json; None disables persistence.
    directory: Option<PathBuf>,
    // Keep the latest attempt for every URL, including terminal states for remounts.
    jobs: BTreeMap<String, Job>,
    // True while adopt_downloads runs, so concurrent callers return early.
    adopting: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct IndexEntry {
    url: String,
    file: String,
    bytes: u64,
}
#[derive(Serialize, Deserialize)]
struct Index {
    version: u32,
    entries: Vec<IndexEntry>,
}
const INDEX_FILE: &str = "index.json";

fn valid_file_name(name: &str) -> bool {
    name.starts_with("gtnh-")
        && name.ends_with(".zip")
        && !name.contains("..")
        && !name.contains(['/', '\\', ':'])
}
// A missing or corrupt index is treated as empty.
fn read_index(directory: &Path) -> Vec<IndexEntry> {
    std::fs::read(directory.join(INDEX_FILE))
        .ok()
        .and_then(|data| serde_json::from_slice::<Index>(&data).ok())
        .filter(|index| index.version == 1)
        .map(|index| index.entries)
        .unwrap_or_default()
}
fn write_index(directory: &Path, entries: Vec<IndexEntry>) -> Result<(), String> {
    std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    let data = serde_json::to_vec_pretty(&Index {
        version: 1,
        entries,
    })
    .map_err(|e| e.to_string())?;
    let temp = directory.join(format!("{INDEX_FILE}.tmp"));
    std::fs::write(&temp, data).map_err(|e| e.to_string())?;
    // rename replaces an existing file on Windows, macOS and Linux.
    std::fs::rename(&temp, directory.join(INDEX_FILE)).map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        e.to_string()
    })
}
fn valid_entry(directory: &Path, entry: &IndexEntry) -> bool {
    gtnh_core::downloads::validate_url(&entry.url).is_ok()
        && valid_file_name(&entry.file)
        && std::fs::symlink_metadata(directory.join(&entry.file))
            .is_ok_and(|m| m.file_type().is_file() && m.len() == entry.bytes)
}

#[derive(Debug, Clone, PartialEq)]
struct Orphan {
    file: String,
    bytes: u64,
    modified: SystemTime,
}
// Orphan archives and candidate URLs.
type AdoptionInput = (Vec<Orphan>, Vec<String>);
#[derive(Debug, PartialEq)]
struct AdoptPlan {
    url: String,
    keep: Orphan,
    delete: Vec<String>,
}

// Regular app-named archives that neither the index nor a job refers to.
fn find_orphans(
    directory: &Path,
    indexed: &HashSet<String>,
    job_paths: &HashSet<PathBuf>,
) -> Vec<Orphan> {
    let Ok(read) = std::fs::read_dir(directory) else {
        return vec![];
    };
    read.flatten()
        .filter_map(|item| {
            let file = item.file_name().into_string().ok()?;
            if !valid_file_name(&file)
                || indexed.contains(&file)
                || job_paths.contains(&directory.join(&file))
            {
                return None;
            }
            let meta = std::fs::symlink_metadata(item.path()).ok()?;
            meta.file_type().is_file().then(|| Orphan {
                file,
                bytes: meta.len(),
                modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            })
        })
        .collect()
}
// URLs whose size is shared with another candidate are ambiguous and skipped.
fn plan_adoption(orphans: &[Orphan], sizes: &BTreeMap<String, u64>) -> Vec<AdoptPlan> {
    sizes
        .iter()
        .filter(|(url, size)| sizes.iter().all(|(u, s)| u == *url || s != *size))
        .filter_map(|(url, size)| {
            let mut matching: Vec<_> = orphans.iter().filter(|o| o.bytes == *size).collect();
            // Newest first; name breaks ties deterministically.
            matching.sort_by(|a, b| b.modified.cmp(&a.modified).then(a.file.cmp(&b.file)));
            let (keep, rest) = matching.split_first()?;
            Some(AdoptPlan {
                url: url.clone(),
                keep: (*keep).clone(),
                delete: rest.iter().map(|o| o.file.clone()).collect(),
            })
        })
        .collect()
}

#[derive(Default)]
pub struct DownloadStore {
    inner: Mutex<Inner>,
    // Replaces Tauri's "download-progress" event; closed receivers are dropped on the next emit.
    subscribers: Mutex<Vec<UnboundedSender<DownloadStatus>>>,
}
impl DownloadStore {
    /// Receives every status change published from now on (the "download-progress" event).
    /// Subscribe before reading `status()` so no event is lost; dedupe with per-URL revisions.
    pub fn subscribe(&self) -> UnboundedReceiver<DownloadStatus> {
        let (sender, receiver) = unbounded();
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.push(sender);
        }
        receiver
    }
    pub(super) fn emit(&self, status: &DownloadStatus) {
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.retain(|s| s.unbounded_send(status.clone()).is_ok());
        }
    }
    pub(super) fn set_adopting(&self, value: bool) -> bool {
        let Ok(mut inner) = self.inner.lock() else {
            return false;
        };
        if value && inner.adopting {
            return false;
        }
        inner.adopting = value;
        true
    }
    /// Downloads directory, orphan archives in it, and candidates without a finished or running job.
    fn adoption_input(&self, urls: Vec<String>) -> Result<Option<AdoptionInput>, String> {
        let inner = self.inner.lock().map_err(|e| e.to_string())?;
        let Some(directory) = inner.directory.as_deref() else {
            return Ok(None);
        };
        let indexed = read_index(directory).into_iter().map(|e| e.file).collect();
        let job_paths = inner
            .jobs
            .values()
            .filter_map(|job| job.status.path.as_ref().map(PathBuf::from))
            .collect();
        let orphans = find_orphans(directory, &indexed, &job_paths);
        if orphans.is_empty() {
            return Ok(None);
        }
        let mut candidates = Vec::new();
        for url in urls {
            if candidates.len() < 64
                && !candidates.contains(&url)
                && gtnh_core::downloads::validate_url(&url).is_ok()
                && !inner.jobs.get(&url).is_some_and(|job| {
                    matches!(
                        job.status.state,
                        DownloadState::Downloading | DownloadState::Completed
                    )
                })
            {
                candidates.push(url);
            }
        }
        Ok(Some((orphans, candidates)))
    }
    /// Registers planned orphans as completed downloads, deleting same-URL duplicates.
    fn apply_adoption(&self, plans: Vec<AdoptPlan>) -> Result<Vec<DownloadStatus>, String> {
        let mut inner = self.inner.lock().map_err(|e| e.to_string())?;
        let Some(directory) = inner.directory.clone() else {
            return Ok(vec![]);
        };
        let mut entries = read_index(&directory);
        let mut adopted = Vec::new();
        for plan in plans {
            let path = directory.join(&plan.keep.file);
            let busy = inner.jobs.get(&plan.url).is_some_and(|job| {
                matches!(
                    job.status.state,
                    DownloadState::Downloading | DownloadState::Completed
                )
            });
            let claimed = inner
                .jobs
                .values()
                .any(|job| job.status.path.as_deref() == path.to_str())
                || entries.iter().any(|e| e.file == plan.keep.file);
            let intact = std::fs::symlink_metadata(&path)
                .is_ok_and(|m| m.file_type().is_file() && m.len() == plan.keep.bytes);
            if busy || claimed || !intact || !valid_file_name(&plan.keep.file) {
                continue;
            }
            for file in plan.delete.iter().filter(|f| valid_file_name(f)) {
                if !entries.iter().any(|e| &e.file == file) {
                    let _ = std::fs::remove_file(directory.join(file));
                }
            }
            entries.retain(|e| e.url != plan.url);
            entries.push(IndexEntry {
                url: plan.url.clone(),
                file: plan.keep.file.clone(),
                bytes: plan.keep.bytes,
            });
            inner.revision += 1;
            let status = DownloadStatus {
                id: inner.revision.to_string(),
                revision: inner.revision,
                url: plan.url.clone(),
                state: DownloadState::Completed,
                received: plan.keep.bytes,
                total: Some(plan.keep.bytes),
                path: Some(path.to_string_lossy().into_owned()),
                error: None,
            };
            inner.jobs.insert(
                plan.url,
                Job {
                    status: status.clone(),
                    cancel: None,
                },
            );
            adopted.push(status);
        }
        if !adopted.is_empty() {
            let _ = write_index(&directory, entries);
        }
        Ok(adopted)
    }
    /// Registers previously completed downloads from the on-disk index.
    pub fn restore(&self, directory: PathBuf) -> Result<(), String> {
        let mut inner = self.inner.lock().map_err(|e| e.to_string())?;
        let entries = read_index(&directory);
        let total = entries.len();
        let valid: Vec<_> = entries
            .into_iter()
            .filter(|entry| valid_entry(&directory, entry))
            .collect();
        if valid.len() != total {
            let _ = write_index(&directory, valid.clone());
        }
        for entry in valid {
            inner.revision += 1;
            let status = DownloadStatus {
                id: inner.revision.to_string(),
                revision: inner.revision,
                url: entry.url.clone(),
                state: DownloadState::Completed,
                received: entry.bytes,
                total: Some(entry.bytes),
                path: Some(directory.join(&entry.file).to_string_lossy().into_owned()),
                error: None,
            };
            inner.jobs.entry(entry.url).or_insert(Job {
                status,
                cancel: None,
            });
        }
        inner.directory = Some(directory);
        Ok(())
    }
    // Best effort: a failed index write only loses persistence, not the download.
    fn record(&self, url: &str, path: &Path) {
        let Ok(inner) = self.inner.lock() else { return };
        let Some(directory) = inner.directory.as_deref() else {
            return;
        };
        let (Some(file), Ok(meta)) = (
            path.file_name().and_then(|n| n.to_str()),
            std::fs::metadata(path),
        ) else {
            return;
        };
        if !valid_file_name(file) {
            return;
        }
        let mut entries = read_index(directory);
        for old in entries.iter().filter(|e| e.url == url && e.file != file) {
            if valid_file_name(&old.file) {
                let _ = std::fs::remove_file(directory.join(&old.file));
            }
        }
        entries.retain(|e| e.url != url);
        entries.push(IndexEntry {
            url: url.into(),
            file: file.into(),
            bytes: meta.len(),
        });
        let _ = write_index(directory, entries);
    }
    pub fn completed_path(&self, url: &str) -> Result<PathBuf, String> {
        let inner = self.inner.lock().map_err(|e| e.to_string())?;
        inner
            .jobs
            .get(url)
            .filter(|job| job.status.state == DownloadState::Completed)
            .and_then(|job| job.status.path.as_ref())
            .map(PathBuf::from)
            .ok_or_else(|| "Download this archive before installing it".into())
    }
    /// Deletes a completed archive and its index entry; returns a tombstone status for the UI.
    pub(super) fn delete(&self, url: &str) -> Result<DownloadStatus, String> {
        let mut inner = self.inner.lock().map_err(|e| e.to_string())?;
        let job = inner
            .jobs
            .get(url)
            .filter(|job| job.status.state == DownloadState::Completed)
            .ok_or("Only a downloaded archive can be deleted")?;
        let path = job.status.path.as_deref().map(PathBuf::from);
        if let Some(directory) = inner.directory.clone() {
            let file = path
                .as_deref()
                .filter(|p| p.parent() == Some(directory.as_path()))
                .and_then(|p| p.file_name()?.to_str())
                .filter(|name| valid_file_name(name))
                .map(str::to_owned);
            if let Some(file) = file {
                match std::fs::remove_file(directory.join(&file)) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                        return Err(format!("Could not delete the archive: {e}"));
                    }
                    _ => {}
                }
            }
            let mut entries = read_index(&directory);
            let before = entries.len();
            entries.retain(|e| e.url != url);
            if entries.len() != before {
                let _ = write_index(&directory, entries);
            }
        }
        let mut job = inner.jobs.remove(url).ok_or("Download not found")?;
        inner.revision += 1;
        job.status.state = DownloadState::Deleted;
        job.status.revision = inner.revision;
        job.status.path = None;
        Ok(job.status)
    }
    pub fn status(&self) -> Result<Vec<DownloadStatus>, String> {
        Ok(self
            .inner
            .lock()
            .map_err(|e| e.to_string())?
            .jobs
            .values()
            .map(|job| job.status.clone())
            .collect())
    }
    pub(super) fn begin(&self, url: String) -> Result<(DownloadStatus, CancellationToken), String> {
        let mut inner = self.inner.lock().map_err(|e| e.to_string())?;
        if inner
            .jobs
            .get(&url)
            .is_some_and(|job| job.status.state == DownloadState::Downloading)
        {
            return Err("This file is already downloading".into());
        }
        inner.revision += 1;
        let status = DownloadStatus {
            id: inner.revision.to_string(),
            revision: inner.revision,
            url: url.clone(),
            state: DownloadState::Downloading,
            received: 0,
            total: None,
            path: None,
            error: None,
        };
        let cancel = CancellationToken::new();
        inner.jobs.insert(
            url,
            Job {
                status: status.clone(),
                cancel: Some(cancel.clone()),
            },
        );
        Ok((status, cancel))
    }
    pub(super) fn progress(
        &self,
        id: &str,
        progress: DownloadProgress,
    ) -> Result<Option<DownloadStatus>, String> {
        self.update(id, |job| {
            job.status.received = progress.received;
            job.status.total = progress.total;
        })
    }
    pub(super) fn finish(
        &self,
        id: &str,
        result: Result<PathBuf, String>,
    ) -> Result<Option<DownloadStatus>, String> {
        let completed = result.as_ref().ok().cloned();
        let status = self.update(id, |job| {
            let cancelled = job.cancel.take().is_some_and(|c| c.is_cancelled());
            match result {
                Ok(path) => {
                    job.status.state = DownloadState::Completed;
                    job.status.path = Some(path.to_string_lossy().into_owned());
                }
                Err(_) if cancelled => {
                    job.status.state = DownloadState::Cancelled;
                }
                Err(error) => {
                    job.status.state = DownloadState::Failed;
                    job.status.error = Some(error);
                }
            }
        })?;
        if let (Some(status), Some(path)) = (&status, completed) {
            self.record(&status.url, &path);
        }
        Ok(status)
    }
    fn update(
        &self,
        id: &str,
        update: impl FnOnce(&mut Job),
    ) -> Result<Option<DownloadStatus>, String> {
        let mut inner = self.inner.lock().map_err(|e| e.to_string())?;
        let Some(url) = inner
            .jobs
            .iter()
            .find(|(_, job)| job.status.id == id && job.status.state == DownloadState::Downloading)
            .map(|(url, _)| url.clone())
        else {
            return Ok(None);
        };
        inner.revision += 1;
        let revision = inner.revision;
        let job = inner.jobs.get_mut(&url).unwrap();
        update(job);
        job.status.revision = revision;
        Ok(Some(job.status.clone()))
    }
    pub(super) fn cancel(&self, id: &str) -> Result<(), String> {
        let inner = self.inner.lock().map_err(|e| e.to_string())?;
        if let Some(token) = inner
            .jobs
            .values()
            .find(|job| job.status.id == id)
            .and_then(|job| job.cancel.as_ref())
        {
            token.cancel();
        }
        Ok(())
    }
}

/// Matches orphan archives (no index entry) to catalog URLs by remote Content-Length.
async fn adopt(backend: Arc<Backend>, urls: Vec<String>) -> Result<Vec<DownloadStatus>, String> {
    let store = &backend.downloads;
    let Some((orphans, candidates)) = store.adoption_input(urls)? else {
        return Ok(vec![]);
    };
    let handles: Vec<_> = candidates
        .into_iter()
        .map(|url| {
            runtime::spawn(async move {
                let size = gtnh_core::downloads::remote_size(&url).await;
                (url, size)
            })
        })
        .collect();
    let mut sizes = BTreeMap::new();
    for handle in handles {
        if let Ok((url, Ok(Some(size)))) = handle.await {
            sizes.insert(url, size);
        }
    }
    let adopted = store.apply_adoption(plan_adoption(&orphans, &sizes))?;
    for status in &adopted {
        store.emit(status);
    }
    Ok(adopted)
}

impl Backend {
    /// Matches orphan archives (no index entry) to catalog URLs by remote size and registers them
    /// as completed downloads. Returns the adopted statuses (also published to subscribers).
    /// Concurrent calls return an empty list.
    pub fn adopt_downloads(
        self: &Arc<Self>,
        urls: Vec<String>,
    ) -> impl std::future::Future<Output = Result<Vec<DownloadStatus>, String>> {
        let backend = self.clone();
        // Runs to completion on the runtime even if the caller drops the future, so the
        // adopting guard is always released.
        runtime::run(async move {
            if !backend.downloads.set_adopting(true) {
                return Ok(vec![]);
            }
            let result = adopt(backend.clone(), urls).await;
            backend.downloads.set_adopting(false);
            result
        })
    }
    /// Subscribes to download events and then returns the current snapshot (subscribe before
    /// query, as the Svelte UI did). Feed both into a `logic::DownloadTracker`.
    pub fn watch_downloads(
        &self,
    ) -> Result<(UnboundedReceiver<DownloadStatus>, Vec<DownloadStatus>), String> {
        let receiver = self.downloads.subscribe();
        Ok((receiver, self.downloads.status()?))
    }
    /// Deletes a completed archive; fails while an installation uses it.
    pub fn delete_download(&self, url: String) -> Result<(), String> {
        let status = self
            .install
            .while_unused(&url, || self.downloads.delete(&url))?;
        self.downloads.emit(&status);
        Ok(())
    }
    /// Requests cancellation of the job with this id (no-op for unknown or finished ids).
    pub fn cancel_download(&self, id: String) -> Result<(), String> {
        self.downloads.cancel(&id)
    }
    /// Starts downloading an official archive in the background; progress arrives through
    /// `DownloadStore::subscribe` (throttled to one event per 200 ms, plus start and end).
    pub fn start_download(self: &Arc<Self>, url: String) -> Result<(), String> {
        gtnh_core::downloads::validate_url(&url)?;
        let directory = self.config_dir.join("downloads");
        let (job, cancel) = self
            .install
            .while_unused(&url, || self.downloads.begin(url.clone()))?;
        self.downloads.emit(&job);
        let backend = self.clone();
        runtime::spawn(async move {
            let last_emit = Mutex::new(Instant::now());
            let result = gtnh_core::downloads::download(&url, &directory, cancel, |progress| {
                if let Ok(Some(status)) = backend.downloads.progress(&job.id, progress) {
                    if let Ok(mut last) = last_emit.lock() {
                        if last.elapsed() >= Duration::from_millis(200) {
                            backend.downloads.emit(&status);
                            *last = Instant::now();
                        }
                    }
                }
            })
            .await;
            if let Ok(Some(status)) = backend.downloads.finish(&job.id, result) {
                backend.downloads.emit(&status);
            }
        });
        Ok(())
    }
}

#[cfg(test)]
#[path = "downloads_tests.rs"]
mod tests;
