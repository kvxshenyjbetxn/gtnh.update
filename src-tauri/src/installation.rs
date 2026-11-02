use gtnh_core::{
    installation::{Decision, Prepared, Preview, Progress},
    server_update::{PreparedServerUpdate, ServerPreview},
    ModRule, RenameRule, Rule,
};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Mutex};
use tauri::{Manager, State};

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Status {
    revision: u64,
    phase: String,
    url: String,
    client: bool,
    progress: Option<Progress>,
    preview: Option<Preview>,
    server_preview: Option<ServerPreview>,
    destination: Option<String>,
    error: Option<String>,
}
enum PreparedJob {
    Client(Prepared),
    Server(PreparedServerUpdate),
}
#[derive(Default)]
struct Job {
    status: Status,
    prepared: Option<PreparedJob>,
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
    // Runs `act` unless an installation in progress uses this archive.
    pub(crate) fn while_unused<T>(
        &self,
        url: &str,
        act: impl FnOnce() -> Result<T, String>,
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
        act()
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
pub(crate) fn installation_status(
    store: State<'_, InstallStore>,
    since: Option<u64>,
) -> Result<Option<Status>, String> {
    let job = store.0.lock().map_err(|e| e.to_string())?;
    Ok(changed_status(&job.status, since))
}
// Polling passes the last revision it accepted so a large unchanged preview is never serialized again.
fn changed_status(status: &Status, since: Option<u64>) -> Option<Status> {
    (since != Some(status.revision)).then(|| status.clone())
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
    #[serde(default)]
    mods: Vec<ModRule>,
    #[serde(default)]
    renames: Vec<RenameRule>,
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
    fn resolve(&self, downloads: &crate::downloads::DownloadStore) -> Result<PathBuf, String> {
        self.key()?;
        if let Some(url) = &self.url {
            downloads.completed_path(url)
        } else {
            gtnh_core::pack_selection::validate(
                self.local_path.as_deref().ok_or("Missing local pack")?,
            )
        }
    }
}
#[tauri::command]
pub(crate) fn prepare_installation(
    app: tauri::AppHandle,
    store: State<'_, InstallStore>,
    downloads: State<'_, crate::downloads::DownloadStore>,
    request: InstallRequest,
) -> Result<(), String> {
    if !request.client {
        return Err("Server updates use prepare_server_update".into());
    }
    if !gtnh_core::validate_rule_path(&request.name) || request.name.contains('/') {
        return Err("Invalid destination folder name".into());
    }
    let key = request.key()?;
    store.begin(key, request.client)?;
    let pack = match request.resolve(&downloads) {
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
            &request.mods,
            &request.renames,
            |p| app.state::<InstallStore>().progress(p),
        );
        match result {
            Ok(prepared) => {
                if let Ok(mut job) = app.state::<InstallStore>().0.lock() {
                    job.status.phase = "preview".into();
                    job.status.preview = Some(prepared.preview().clone());
                    job.status.revision += 1;
                    job.prepared = Some(PreparedJob::Client(prepared));
                }
            }
            Err(error) => app.state::<InstallStore>().fail(error),
        }
    });
    Ok(())
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ServerUpdateRequest {
    url: Option<String>,
    local_path: Option<PathBuf>,
    server: String,
    #[serde(default)]
    rules: Vec<Rule>,
    #[serde(default)]
    mods: Vec<ModRule>,
}
impl ServerUpdateRequest {
    fn key(&self) -> Result<String, String> {
        match (&self.url, &self.local_path) {
            (Some(url), None) => {
                let official = gtnh_core::downloads::validate_url(url)?;
                if !official.path().starts_with("/ServerPacks/") {
                    return Err("Archive type does not match server update".into());
                }
                Ok(url.clone())
            }
            (None, Some(path)) => Ok(path.to_string_lossy().into_owned()),
            _ => Err("Choose exactly one downloaded archive or local pack".into()),
        }
    }
    fn resolve(&self, downloads: &crate::downloads::DownloadStore) -> Result<PathBuf, String> {
        self.key()?;
        if let Some(url) = &self.url {
            downloads.completed_path(url)
        } else {
            gtnh_core::pack_selection::validate(
                self.local_path.as_deref().ok_or("Missing local pack")?,
            )
        }
    }
}
#[tauri::command]
pub(crate) fn prepare_server_update(
    app: tauri::AppHandle,
    store: State<'_, InstallStore>,
    downloads: State<'_, crate::downloads::DownloadStore>,
    request: ServerUpdateRequest,
) -> Result<(), String> {
    let key = request.key()?;
    store.begin(key, false)?;
    let pack = match request.resolve(&downloads) {
        Ok(resolved) => resolved,
        Err(error) => {
            store.fail(error.clone());
            return Err(error);
        }
    };
    tauri::async_runtime::spawn_blocking(move || {
        let result = gtnh_core::server_update::prepare(
            &pack,
            std::path::Path::new(&request.server),
            &request.rules,
            &request.mods,
            |p| app.state::<InstallStore>().progress(p),
        );
        match result {
            Ok(prepared) => {
                if let Ok(mut job) = app.state::<InstallStore>().0.lock() {
                    job.status.phase = "preview".into();
                    job.status.server_preview = Some(prepared.preview().clone());
                    job.status.revision += 1;
                    job.prepared = Some(PreparedJob::Server(prepared));
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
    let prepared = {
        let mut job = store.0.lock().map_err(|e| e.to_string())?;
        if job.status.phase != "preview" {
            return Err("No installation is ready to confirm".into());
        }
        let prepared = job.prepared.take().ok_or("Missing prepared installation")?;
        job.status.phase = "copying".into();
        job.status.revision += 1;
        prepared
    };
    tauri::async_runtime::spawn_blocking(move || {
        let progress = |p| app.state::<InstallStore>().progress(p);
        let result = match prepared {
            PreparedJob::Client(prepared) => prepared.finish(&decisions, progress),
            PreparedJob::Server(prepared) => prepared.finish(&decisions, progress),
        };
        match result {
            Ok(destination) => {
                if let Ok(mut job) = app.state::<InstallStore>().0.lock() {
                    job.status.phase = "completed".into();
                    job.status.destination = Some(destination.to_string_lossy().into_owned());
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
    fn unchanged_revision_skips_status_payload() {
        let status = Status {
            revision: 7,
            ..Default::default()
        };
        assert!(changed_status(&status, Some(7)).is_none());
        assert_eq!(changed_status(&status, Some(6)).unwrap().revision, 7);
        assert_eq!(changed_status(&status, None).unwrap().revision, 7);
    }
    #[test]
    fn local_inputs_are_left_untouched_and_ambiguous_inputs_fail() {
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
            mods: vec![],
            renames: vec![],
        };
        assert!(request.key().is_ok());
        let path = request
            .resolve(&crate::downloads::DownloadStore::default())
            .unwrap();
        assert_eq!(path, local.canonicalize().unwrap());
        assert_eq!(std::fs::read_to_string(&local).unwrap(), "user archive");
        request.url = Some("https://downloads.gtnewhorizons.com/ServerPacks/pack.zip".into());
        assert!(request.key().is_err());
        std::fs::remove_file(&local).unwrap();
    }
    #[test]
    fn server_request_requires_server_pack_and_one_source() {
        let mut request = ServerUpdateRequest {
            url: Some("https://downloads.gtnewhorizons.com/ServerPacks/pack.zip".into()),
            local_path: None,
            server: String::new(),
            rules: vec![],
            mods: vec![],
        };
        assert!(request.key().is_ok());
        request.url =
            Some("https://downloads.gtnewhorizons.com/Multi_mc_downloads/pack.zip".into());
        assert!(request.key().is_err());
        request.local_path = Some(PathBuf::from("pack.zip"));
        assert!(request.key().is_err());
        request.url = None;
        assert!(request.key().is_ok());
        request.local_path = None;
        assert!(request.key().is_err());
    }
    #[test]
    fn active_install_pins_only_its_archive_and_blocks_a_second_install() {
        let store = InstallStore::default();
        store.begin("one".into(), true).unwrap();
        assert!(store.while_unused("one", || Ok(())).is_err());
        assert!(store.while_unused("two", || Ok(())).is_ok());
        assert!(store.begin("two".into(), false).is_err());
        store.0.lock().unwrap().status.phase = "preview".into();
        assert!(store.while_unused("one", || Ok(())).is_err());
        assert!(store.begin("two".into(), false).is_err());
        store.fail("failed".into());
        assert!(store.while_unused("one", || Ok(())).is_ok());
        store.begin("two".into(), false).unwrap();
    }
}
