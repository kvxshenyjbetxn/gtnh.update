//! Non-UI backend: a direct port of the Tauri side (`src-tauri/src`), without IPC.
//!
//! Views fetch it with `Backend::get(cx)` (an `Arc<Backend>`; `Backend::init(cx)` must run once at
//! startup). Everything returns `Result<_, String>` with the same error strings as the Tauri
//! commands.
//!
//! # API (methods of [`Backend`], named like the Tauri commands)
//!
//! Settings and config (synchronous, small files, guarded by one mutex):
//! - `load_settings() -> Result<Settings, String>`
//! - `save_settings(Settings) -> Result<Settings, String>`
//! - `config_directory() -> String`
//! - `open_config_directory() -> Result<(), String>` (creates the folder, opens it in the OS)
//! - `open_release_url(String) -> Result<(), String>` (only allowed URLs, opens the browser)
//!
//! Filesystem helpers (async; run on the tokio runtime, awaitable from `cx.spawn`):
//! - `source_directory(path, client) -> Result<String, String>`
//! - `instances_directory(path) -> Result<String, String>`
//! - `relative_selections(root, paths) -> Result<Vec<String>, String>`
//! - `inspect_mod(path) -> Result<ModInfo, String>`
//! - `list_releases() -> Result<Vec<Release>, String>` (network)
//!
//! Downloads (see `downloads.rs`):
//! - `start_download(url)`, `cancel_download(id)`, `delete_download(url)` (synchronous, return at once)
//! - `download_status() -> Result<Vec<DownloadStatus>, String>`
//! - `adopt_downloads(urls) -> impl Future<Result<Vec<DownloadStatus>, String>>` (async, network)
//! - `watch_downloads() -> Result<(UnboundedReceiver<DownloadStatus>, Vec<DownloadStatus>), String>`
//!   subscribes first, then snapshots
//!
//! Installation (see `installation.rs`):
//! - `prepare_installation(InstallRequest)`, `prepare_server_update(ServerUpdateRequest)`,
//!   `finish_installation(Vec<Decision>)`, `cancel_installation()` (all synchronous; the work runs
//!   in the background)
//! - `installation_status(since: Option<u64>) -> Result<Option<Status>, String>`
//!
//! # Download events
//!
//! Tauri's `download-progress` event became `DownloadStore::subscribe()` (also reachable via
//! `Backend::watch_downloads`), which returns a `futures::channel::mpsc::UnboundedReceiver`.
//! Any number of views may subscribe; closed receivers are pruned automatically. A view should
//! feed the snapshot and every received event into a `logic::DownloadTracker`, which keeps the
//! highest revision per URL and hides `deleted` tombstones, and re-render when it returns a list.
//! Typical loop inside `cx.spawn`: `while let Some(status) = receiver.next().await { ... }`.
//! Progress events are throttled to one per 200 ms per download (start and end always arrive).
//!
//! # Installation status
//!
//! There is one process-owned installation. Poll `installation_status(since)` every 500 ms
//! (`cx.background_executor().timer(..)`) while it is active, passing the last accepted revision;
//! `Ok(None)` means nothing changed. `logic::InstallWatcher` implements the "no overlapping
//! polls, reject stale revisions" rules. `phase` goes `preparing -> preview -> copying ->
//! completed`, or `failed` at any point.
//!
//! # Runtime
//!
//! `runtime.rs` owns a lazily created multi-thread tokio runtime. Async methods spawn onto it
//! immediately and return a future that is safe to await on GPUI's executor.
#![allow(dead_code)] // the download / catalog half of the backend is consumed by the release catalog view

pub mod downloads;
pub mod installation;
pub mod runtime;

use gtnh_core::Settings;
use std::{
    future::Future,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

pub use downloads::{DownloadState, DownloadStatus, DownloadStore};
pub use installation::{InstallRequest, InstallStore, ServerUpdateRequest, Status};

/// Shared backend state; the counterpart of Tauri's managed `DownloadStore`, `InstallStore` and
/// `ConfigStore`.
pub struct Backend {
    pub config_dir: PathBuf,
    pub downloads: DownloadStore,
    pub install: InstallStore,
    settings_lock: Mutex<()>,
}

/// GPUI global holding the backend.
pub struct BackendGlobal(pub Arc<Backend>);
impl gpui::Global for BackendGlobal {}

impl Backend {
    /// Computes the config dir (`<config>/app.gtnh.updater`), runs the startup work and sets the
    /// [`BackendGlobal`].
    pub fn init(cx: &mut gpui::App) {
        let directory = dirs::config_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("app.gtnh.updater");
        cx.set_global(BackendGlobal(Arc::new(Backend::new(directory))));
    }
    pub fn get(cx: &gpui::App) -> Arc<Backend> {
        cx.global::<BackendGlobal>().0.clone()
    }

    /// Builds the backend and performs the startup work of Tauri's `.setup()`.
    pub fn new(config_dir: PathBuf) -> Backend {
        let backend = Backend {
            downloads: DownloadStore::default(),
            install: InstallStore::default(),
            settings_lock: Mutex::new(()),
            config_dir,
        };
        // A damaged downloads folder must not block startup.
        let _ = backend.downloads.restore(backend.config_dir.join("downloads"));
        // Staging folders of a killed installation can be several GB; best effort.
        let settings = gtnh_core::load_settings(&backend.config_dir).ok();
        if let Some(instances) = settings
            .as_ref()
            .and_then(|s| s.prism_instances_directory.as_ref())
        {
            let _ = gtnh_core::installation::remove_stale_staging(Path::new(instances));
        }
        // Staging folders of finished server updates that were locked at the time.
        if let Some(server) = settings.as_ref().and_then(|s| s.server_source.as_ref()) {
            let _ = gtnh_core::server_update::cleanup_leftovers(Path::new(server));
        }
        backend
    }

    pub fn load_settings(&self) -> Result<Settings, String> {
        let _guard = self.settings_lock.lock().map_err(|e| e.to_string())?;
        gtnh_core::load_settings(&self.config_dir).map_err(|e| e.to_string())
    }
    pub fn save_settings(&self, settings: Settings) -> Result<Settings, String> {
        let _guard = self.settings_lock.lock().map_err(|e| e.to_string())?;
        gtnh_core::save_settings(&self.config_dir, &settings).map_err(|e| e.to_string())?;
        Ok(settings)
    }
    pub fn config_directory(&self) -> String {
        self.config_dir.to_string_lossy().into_owned()
    }
    pub fn open_config_directory(&self) -> Result<(), String> {
        std::fs::create_dir_all(&self.config_dir).map_err(|e| e.to_string())?;
        open::that_detached(&self.config_dir).map_err(|e| e.to_string())
    }
    pub fn open_release_url(&self, url: String) -> Result<(), String> {
        if !gtnh_core::releases::allowed_external_url(&url) {
            return Err("Unsupported URL".into());
        }
        open::that_detached(url).map_err(|e| e.to_string())
    }

    pub fn source_directory(
        &self,
        path: String,
        client: bool,
    ) -> impl Future<Output = Result<String, String>> {
        runtime::run_blocking(move || {
            gtnh_core::selection::source_directory(Path::new(&path), client)
                .map_err(|e| e.to_string())
        })
    }
    pub fn instances_directory(&self, path: String) -> impl Future<Output = Result<String, String>> {
        runtime::run_blocking(move || {
            gtnh_core::selection::instances_directory(Path::new(&path)).map_err(|e| e.to_string())
        })
    }
    pub fn relative_selections(
        &self,
        root: String,
        paths: Vec<String>,
    ) -> impl Future<Output = Result<Vec<String>, String>> {
        runtime::run_blocking(move || {
            gtnh_core::selection::relative_selections(Path::new(&root), &paths)
                .map_err(|e| e.to_string())
        })
    }
    pub fn inspect_mod(
        &self,
        path: String,
    ) -> impl Future<Output = Result<gtnh_core::mods::ModInfo, String>> {
        runtime::run_blocking(move || gtnh_core::mods::inspect(Path::new(&path)))
    }
    pub fn list_releases(
        &self,
    ) -> impl Future<Output = Result<Vec<gtnh_core::releases::Release>, String>> {
        runtime::run(gtnh_core::releases::fetch_official())
    }
}
