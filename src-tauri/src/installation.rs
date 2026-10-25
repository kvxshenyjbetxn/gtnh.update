use gtnh_core::{
    installation::{Decision, Prepared, Preview, Progress},
    Rule,
};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Mutex};
use tauri::{Emitter, Manager, State};

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Status {
    revision: u64,
    phase: String,
    url: String,
    client: bool,
    progress: Option<Progress>,
    preview: Option<Preview>,
    destination: Option<String>,
    error: Option<String>,
    warning: Option<String>,
}
#[derive(Default)]
struct Job {
    status: Status,
    prepared: Option<Prepared>,
    archive: Option<PathBuf>,
}
#[derive(Default)]
pub(crate) struct InstallStore(Mutex<Job>);
impl InstallStore {
    fn begin(&self, url: String, client: bool) -> Result<(), String> {
        let mut job = self.0.lock().map_err(|e| e.to_string())?;
        if matches!(
            job.status.phase.as_str(),
            "preparing" | "preview" | "copying"
        ) {
            return Err("Finish or cancel the current installation first".into());
        }
        *job = Job {
            status: Status {
                revision: job.status.revision + 1,
                phase: "preparing".into(),
                url,
                client,
                ..Default::default()
            },
            ..Default::default()
        };
        Ok(())
    }
    pub(crate) fn start_download<T>(
        &self,
        url: &str,
        start: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let job = self.0.lock().map_err(|e| e.to_string())?;
        if job.status.url == url
            && matches!(
                job.status.phase.as_str(),
                "preparing" | "preview" | "copying"
            )
        {
            return Err("This archive is being installed".into());
        }
        start()
    }
    fn progress(&self, progress: Progress) {
        if let Ok(mut job) = self.0.lock() {
            job.status.progress = Some(progress);
            job.status.revision += 1;
        }
    }
    fn fail(&self, error: String) {
        if let Ok(mut job) = self.0.lock() {
            job.prepared = None;
            job.status.phase = "failed".into();
            job.status.error = Some(error);
            job.status.revision += 1;
        }
    }
}
#[tauri::command]
pub(crate) fn installation_status(store: State<'_, InstallStore>) -> Result<Status, String> {
    Ok(store.0.lock().map_err(|e| e.to_string())?.status.clone())
}
#[tauri::command]
pub(crate) fn cancel_installation(store: State<'_, InstallStore>) -> Result<(), String> {
    let mut job = store.0.lock().map_err(|e| e.to_string())?;
    if matches!(job.status.phase.as_str(), "preparing" | "copying") {
        return Err("Wait for the current filesystem operation to finish".into());
    }
    let revision = job.status.revision + 1;
    *job = Job {
        status: Status {
            revision,
            ..Default::default()
        },
        ..Default::default()
    };
    Ok(())
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct InstallRequest {
    url: Option<String>,
    local_path: Option<PathBuf>,
    source: String,
    parent: String,
    name: String,
    client: bool,
    rules: Vec<Rule>,
}
impl InstallRequest {
    fn key(&self) -> Result<String, String> {
        match (&self.url, &self.local_path) {
            (Some(url), None) => {
                let official = gtnh_core::downloads::validate_url(url)?;
                if self.client != official.path().starts_with("/Multi_mc_downloads/") {
                    return Err("Archive type does not match installation mode".into());
                }
                Ok(url.clone())
            }
            (None, Some(path)) => Ok(path.to_string_lossy().into_owned()),
            _ => Err("Choose exactly one downloaded archive or local pack".into()),
        }
    }
    fn resolve(
        &self,
        downloads: &crate::downloads::DownloadStore,
    ) -> Result<(PathBuf, Option<PathBuf>), String> {
        self.key()?;
        if let Some(url) = &self.url {
            let path = downloads.completed_path(url)?;
            Ok((path.clone(), Some(path)))
        } else {
            Ok((
                gtnh_core::pack_selection::validate(
                    self.local_path.as_deref().ok_or("Missing local pack")?,
                )?,
                None,
            ))
        }
    }
}
fn cleanup_archive(owned: Option<&std::path::Path>) -> Option<String> {
    owned.and_then(|path| {
        std::fs::remove_file(path).err().map(|e| {
            format!(
                "Installed, but could not remove archive {}: {e}",
                path.display()
            )
        })
    })
}
#[tauri::command]
pub(crate) fn prepare_installation(
    app: tauri::AppHandle,
    store: State<'_, InstallStore>,
    downloads: State<'_, crate::downloads::DownloadStore>,
    request: InstallRequest,
) -> Result<(), String> {
    if !gtnh_core::validate_rule_path(&request.name) || request.name.contains('/') {
        return Err("Invalid destination folder name".into());
    }
    let key = request.key()?;
    store.begin(key, request.client)?;
    // Only a managed download is owned: user-selected ZIPs/folders must never be removed.
    let (pack, owned_archive) = match request.resolve(&downloads) {
        Ok(resolved) => resolved,
        Err(error) => {
            store.fail(error.clone());
            return Err(error);
        }
    };
    let destination = PathBuf::from(request.parent).join(request.name);
    tauri::async_runtime::spawn_blocking(move || {
        let result = gtnh_core::installation::prepare(
            &pack,
            std::path::Path::new(&request.source),
            &destination,
            request.client,
            &request.rules,
            |p| app.state::<InstallStore>().progress(p),
        );
        match result {
            Ok(prepared) => {
                if let Ok(mut job) = app.state::<InstallStore>().0.lock() {
                    job.status.phase = "preview".into();
                    job.status.preview = Some(prepared.preview().clone());
                    job.status.revision += 1;
                    job.prepared = Some(prepared);
                    job.archive = owned_archive;
                }
            }
            Err(error) => app.state::<InstallStore>().fail(error),
        }
    });
    Ok(())
}
#[tauri::command]
pub(crate) fn finish_installation(
    app: tauri::AppHandle,
    store: State<'_, InstallStore>,
    decisions: Vec<Decision>,
) -> Result<(), String> {
    let (prepared, archive, url) = {
        let mut job = store.0.lock().map_err(|e| e.to_string())?;
        if job.status.phase != "preview" {
            return Err("No installation is ready to confirm".into());
        }
        let prepared = job.prepared.take().ok_or("Missing prepared installation")?;
        let archive = job.archive.clone();
        job.status.phase = "copying".into();
        job.status.revision += 1;
        (prepared, archive, job.status.url.clone())
    };
    tauri::async_runtime::spawn_blocking(move || {
        let result = prepared.finish(&decisions, |p| app.state::<InstallStore>().progress(p));
        match result {
            Ok(destination) => {
                let warning = cleanup_archive(archive.as_deref());
                if archive.is_some() {
                    if let Ok(Some(status)) = app
                        .state::<crate::downloads::DownloadStore>()
                        .mark_installed(&url, &destination)
                    {
                        let _ = app.emit("download-progress", status);
                    }
                }
                if let Ok(mut job) = app.state::<InstallStore>().0.lock() {
                    job.status.phase = "completed".into();
                    job.status.destination = Some(destination.to_string_lossy().into_owned());
                    job.status.warning = warning;
                    job.status.revision += 1;
                }
            }
            Err(error) => app.state::<InstallStore>().fail(error),
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_inputs_are_never_owned_or_deleted_and_ambiguous_inputs_fail() {
        let directory =
            gtnh_core::pack_selection::validate(std::env::temp_dir().as_path()).unwrap();
        let local = directory.join(format!("gtnh-local-{}.zip", std::process::id()));
        std::fs::write(&local, "user archive").unwrap();
        let mut request = InstallRequest {
            url: None,
            local_path: Some(local.clone()),
            source: String::new(),
            parent: String::new(),
            name: "new".into(),
            client: false,
            rules: vec![],
        };
        assert!(request.key().is_ok());
        let (path, owned) = request
            .resolve(&crate::downloads::DownloadStore::default())
            .unwrap();
        assert_eq!(path, local.canonicalize().unwrap());
        assert!(owned.is_none());
        assert!(cleanup_archive(owned.as_deref()).is_none());
        assert_eq!(std::fs::read_to_string(&local).unwrap(), "user archive");
        request.url = Some("https://downloads.gtnewhorizons.com/ServerPacks/pack.zip".into());
        assert!(request.key().is_err());
        std::fs::remove_file(&local).unwrap();
    }
    #[test]
    fn active_install_pins_only_its_archive_and_blocks_a_second_install() {
        let store = InstallStore::default();
        store.begin("one".into(), true).unwrap();
        assert!(store.start_download("one", || Ok(())).is_err());
        assert!(store.start_download("two", || Ok(())).is_ok());
        assert!(store.begin("two".into(), false).is_err());
        store.0.lock().unwrap().status.phase = "preview".into();
        assert!(store.start_download("one", || Ok(())).is_err());
        assert!(store.begin("two".into(), false).is_err());
        store.fail("failed".into());
        assert!(store.start_download("one", || Ok(())).is_ok());
        store.begin("two".into(), false).unwrap();
    }
}
