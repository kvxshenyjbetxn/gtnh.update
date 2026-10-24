use gtnh_core::Settings;
use std::{path::PathBuf, sync::Mutex};
use tauri::{Manager, State};
use tauri_plugin_opener::OpenerExt;

struct ConfigStore {
    directory: PathBuf,
    lock: Mutex<()>,
}

#[tauri::command]
fn load_settings(store: State<'_, ConfigStore>) -> Result<Settings, String> {
    let _guard = store.lock.lock().map_err(|e| e.to_string())?;
    gtnh_core::load_settings(&store.directory).map_err(|e| e.to_string())
}
#[tauri::command]
fn save_settings(settings: Settings, store: State<'_, ConfigStore>) -> Result<Settings, String> {
    let _guard = store.lock.lock().map_err(|e| e.to_string())?;
    gtnh_core::save_settings(&store.directory, &settings).map_err(|e| e.to_string())?;
    Ok(settings)
}
#[tauri::command]
fn config_directory(store: State<'_, ConfigStore>) -> String {
    store.directory.to_string_lossy().into_owned()
}
#[tauri::command]
fn open_config_directory(
    app: tauri::AppHandle,
    store: State<'_, ConfigStore>,
) -> Result<(), String> {
    std::fs::create_dir_all(&store.directory).map_err(|e| e.to_string())?;
    app.opener()
        .open_path(store.directory.to_string_lossy().into_owned(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn source_directory(path: String, client: bool) -> Result<String, String> {
    gtnh_core::selection::source_directory(std::path::Path::new(&path), client)
        .map_err(|e| e.to_string())
}
#[tauri::command]
fn relative_selections(root: String, paths: Vec<String>) -> Result<Vec<String>, String> {
    gtnh_core::selection::relative_selections(std::path::Path::new(&root), &paths)
        .map_err(|e| e.to_string())
}
#[tauri::command]
async fn list_releases() -> Result<Vec<gtnh_core::releases::Release>, String> {
    gtnh_core::releases::fetch_official().await
}
#[tauri::command]
fn open_release_url(app: tauri::AppHandle, url: String) -> Result<(), String> {
    if !gtnh_core::releases::allowed_external_url(&url) {
        return Err("Unsupported URL".into());
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            app.manage(ConfigStore {
                directory: app.path().app_config_dir()?,
                lock: Mutex::new(()),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            load_settings,
            save_settings,
            config_directory,
            open_config_directory,
            source_directory,
            relative_selections,
            list_releases,
            open_release_url
        ])
        .run(tauri::generate_context!())
        .expect("Failed to run GTNH Updater");
}
