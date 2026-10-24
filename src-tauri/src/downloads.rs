use gtnh_core::downloads::{CancellationToken, DownloadProgress};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::{Emitter, Manager, State};

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum DownloadState {
    Downloading,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DownloadStatus {
    id: String,
    revision: u64,
    url: String,
    state: DownloadState,
    received: u64,
    total: Option<u64>,
    path: Option<String>,
    error: Option<String>,
}

struct Job {
    status: DownloadStatus,
    cancel: Option<CancellationToken>,
}
#[derive(Default)]
struct Inner {
    revision: u64,
    // Keep the latest attempt for every URL, including terminal states for remounts.
    jobs: BTreeMap<String, Job>,
}
#[derive(Default)]
pub(crate) struct DownloadStore(Mutex<Inner>);
impl DownloadStore {
    fn status(&self) -> Result<Vec<DownloadStatus>, String> {
        Ok(self
            .0
            .lock()
            .map_err(|e| e.to_string())?
            .jobs
            .values()
            .map(|job| job.status.clone())
            .collect())
    }
    fn begin(&self, url: String) -> Result<(DownloadStatus, CancellationToken), String> {
        let mut inner = self.0.lock().map_err(|e| e.to_string())?;
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
    fn progress(
        &self,
        id: &str,
        progress: DownloadProgress,
    ) -> Result<Option<DownloadStatus>, String> {
        self.update(id, |job| {
            job.status.received = progress.received;
            job.status.total = progress.total;
        })
    }
    fn finish(
        &self,
        id: &str,
        result: Result<PathBuf, String>,
    ) -> Result<Option<DownloadStatus>, String> {
        self.update(id, |job| {
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
        })
    }
    fn update(
        &self,
        id: &str,
        update: impl FnOnce(&mut Job),
    ) -> Result<Option<DownloadStatus>, String> {
        let mut inner = self.0.lock().map_err(|e| e.to_string())?;
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
    fn cancel(&self, id: &str) -> Result<(), String> {
        let inner = self.0.lock().map_err(|e| e.to_string())?;
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

#[tauri::command]
pub(crate) fn download_status(
    store: State<'_, DownloadStore>,
) -> Result<Vec<DownloadStatus>, String> {
    store.status()
}
#[tauri::command]
pub(crate) fn cancel_download(store: State<'_, DownloadStore>, id: String) -> Result<(), String> {
    store.cancel(&id)
}
#[tauri::command]
pub(crate) fn start_download(
    app: tauri::AppHandle,
    store: State<'_, DownloadStore>,
    config: State<'_, crate::ConfigStore>,
    url: String,
) -> Result<(), String> {
    gtnh_core::downloads::validate_url(&url)?;
    let directory = config.directory.join("downloads");
    let (job, cancel) = store.begin(url.clone())?;
    let _ = app.emit("download-progress", &job);
    tauri::async_runtime::spawn(async move {
        let last_emit = Mutex::new(Instant::now());
        let result = gtnh_core::downloads::download(&url, &directory, cancel, |progress| {
            let store = app.state::<DownloadStore>();
            if let Ok(Some(status)) = store.progress(&job.id, progress) {
                if let Ok(mut last) = last_emit.lock() {
                    if last.elapsed() >= Duration::from_millis(200) {
                        let _ = app.emit("download-progress", status);
                        *last = Instant::now();
                    }
                }
            }
        })
        .await;
        if let Ok(Some(status)) = app.state::<DownloadStore>().finish(&job.id, result) {
            let _ = app.emit("download-progress", status);
        }
    });
    Ok(())
}

#[cfg(test)]
#[path = "downloads_tests.rs"]
mod tests;
