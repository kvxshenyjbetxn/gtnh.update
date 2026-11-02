//! In-place server update following the GTNH wiki: managed folders and launch files are
//! swapped with the new pack, everything else (world, settings, custom files) is untouched.
use crate::installation::{
    check_targets, checked_source, collect_rules, extract, pack_root, publish, stage_folder,
    transfer_files, validate_decisions, Decision, Progress, Transfer,
};
use crate::mods::{self, ModPlan, Sources};
use crate::{validate_rule_path, ModRule, Rule};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
};

const STAGING_PREFIX: &str = ".gtnh-update-";
const JOURNAL: &str = ".gtnh-update-journal.json";
const MANIFEST: &str = "gtnh-backup.json";
const FOLDERS: &[&str] = &["config", "libraries", "mods", "resources", "scripts"];
const LAUNCH_EXTENSIONS: &[&str] = &[".jar", ".sh", ".bat", ".cmd", ".command", ".ps1"];
const SCRIPT_EXTENSIONS: &[&str] = &[".sh", ".bat", ".cmd", ".command", ".ps1"];
const SERVER_ONLY: &[&str] = &[
    "java9args.txt",
    "lwjgl3ify-forgepatches.jar",
    "startserver-java9.bat",
    "startserver-java9.sh",
];
const MAX_SCRIPT_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerItem {
    pub path: String,
    pub directory: bool,
    pub old: bool,
    pub new: bool,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptChange {
    pub path: String,
    pub old: Option<String>,
    pub new: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum JavaChange {
    ToJava17,
    ToJava8,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Recovery {
    RolledBack,
    Completed,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerPreview {
    pub server: String,
    pub backup: String,
    pub previous_backup: bool,
    pub replaced: Vec<ServerItem>,
    pub added: Vec<String>,
    pub journey_map: bool,
    pub scripts: Vec<ScriptChange>,
    pub java_change: Option<JavaChange>,
    pub recovered: Option<Recovery>,
    pub mods: ModPlan,
    pub files: Vec<Transfer>,
    pub missing: Vec<String>,
}

/// Test hook that simulates a failing or killed process.
#[doc(hidden)]
#[derive(Debug, Clone, Copy)]
pub enum Fault {
    FailAfterMoves(usize),
    CrashAfterMoves(usize),
    CrashAfterCommit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Item {
    path: String,
    old: Option<String>,
    new: bool,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Journal {
    version: u32,
    staging: PathBuf,
    payload_root: PathBuf,
    backup: PathBuf,
    final_backup: PathBuf,
    items: Vec<Item>,
    committed: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Manifest<'a> {
    created_at: u64,
    pack: &'a str,
    items: &'a [Item],
}
struct Journey {
    config_old: String,
    config_new: String,
    name: String,
}

pub struct PreparedServerUpdate {
    staging: tempfile::TempDir,
    server: PathBuf,
    payload_root: PathBuf,
    final_backup: PathBuf,
    items: Vec<Item>,
    journey: Option<Journey>,
    pack_name: String,
    preview: ServerPreview,
    additions: Sources,
}

fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}
fn is_real_dir(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.is_dir())
}

struct Entry {
    name: String,
    dir: bool,
    plain: bool,
}
// Entries with non-Unicode names can never be managed, so they are simply ignored.
fn entries(dir: &Path) -> Result<Vec<Entry>, String> {
    let mut result = Vec::new();
    for entry in fs::read_dir(dir).map_err(error)? {
        let entry = entry.map_err(error)?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let metadata = fs::symlink_metadata(entry.path()).map_err(error)?;
        result.push(Entry {
            name,
            dir: metadata.is_dir(),
            plain: !metadata.file_type().is_symlink() && (metadata.is_dir() || metadata.is_file()),
        });
    }
    Ok(result)
}
fn find_ci(dir: &Path, name: &str) -> Result<Option<String>, String> {
    let lower = name.to_lowercase();
    Ok(entries(dir)?
        .into_iter()
        .map(|e| e.name)
        .find(|n| n.to_lowercase() == lower))
}
fn is_launch_file(lower: &str) -> bool {
    lower == "java9args.txt" || LAUNCH_EXTENSIONS.iter().any(|e| lower.ends_with(e))
}
fn is_script(lower: &str) -> bool {
    lower == "java9args.txt" || SCRIPT_EXTENSIONS.iter().any(|e| lower.ends_with(e))
}
fn read_small(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let metadata = fs::metadata(path).map_err(error)?;
    if !metadata.is_file() || metadata.len() > MAX_SCRIPT_BYTES {
        return Ok(None);
    }
    fs::read(path).map(Some).map_err(error)
}
fn move_path(source: &Path, destination: &Path, name: &str) -> Result<(), String> {
    publish(source, destination).map_err(|e| {
        format!(
            "Cannot move {name}: {e}. Make sure the server is stopped and no program is using its files."
        )
    })
}

pub fn prepare(
    archive: &Path,
    server: &Path,
    rules: &[Rule],
    mods: &[ModRule],
    progress: impl Fn(Progress),
) -> Result<PreparedServerUpdate, String> {
    let server = server.canonicalize().map_err(error)?;
    let not_server = "Selected folder is not a GTNH server (expected mods and config folders)";
    if !server.is_dir()
        || !["mods", "config"].iter().all(|n| {
            find_ci(&server, n)
                .ok()
                .flatten()
                .is_some_and(|f| is_real_dir(&server.join(f)))
        })
    {
        return Err(not_server.into());
    }
    let parent = server
        .parent()
        .ok_or("Server folder has no parent")?
        .to_owned();
    let server_name = server
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("Invalid server folder name")?
        .to_owned();
    let pack = archive.canonicalize().map_err(error)?;
    if pack.is_dir() && (server.starts_with(&pack) || pack.starts_with(&server)) {
        return Err("The selected pack and server must be separate, non-nested folders".into());
    }
    let recovered = recover(&server)?;
    let _ = remove_stale_staging(&parent);
    let final_backup = parent.join(format!("{server_name}.gtnh-backup"));
    let previous_backup = exists(&final_backup);
    if previous_backup && !is_real_dir(&final_backup) {
        return Err("The backup location is occupied by something that is not a folder".into());
    }

    let staging = tempfile::Builder::new()
        .prefix(STAGING_PREFIX)
        .tempdir_in(&parent)
        .map_err(error)?;
    let payload = staging.path().join("payload");
    fs::create_dir(&payload).map_err(error)?;
    fs::create_dir(staging.path().join("backup")).map_err(error)?;
    if pack.is_dir() {
        stage_folder(&pack, &payload, &progress)?;
    } else {
        extract(&pack, &payload, &progress)?;
    }
    let payload_root = pack_root(&payload, false)?;
    let (mut plan, additions) = mods::apply_rules(&payload_root, mods)?;
    let (mut files, missing) = collect_rules(&server, rules)?;
    check_targets(&payload_root, &mut files)?;
    let files: Vec<_> = files.into_values().collect();
    plan.duplicates = mods::find_duplicates(&payload_root, &additions, &files, &server);

    let server_entries = entries(&server)?;
    let pack_entries = entries(&payload_root)?;
    let server_by: HashMap<_, _> = server_entries
        .iter()
        .map(|e| (e.name.to_lowercase(), e))
        .collect();
    let pack_by: HashMap<_, _> = pack_entries
        .iter()
        .map(|e| (e.name.to_lowercase(), e))
        .collect();

    // (item, old is dir, new is dir)
    let mut managed: Vec<(Item, Option<bool>, Option<bool>)> = Vec::new();
    let mut added = Vec::new();
    for entry in &pack_entries {
        let lower = entry.name.to_lowercase();
        let old = server_by.get(&lower);
        if FOLDERS.contains(&lower.as_str()) || (!entry.dir && is_launch_file(&lower)) {
            managed.push((
                Item {
                    path: entry.name.clone(),
                    old: old.map(|e| e.name.clone()),
                    new: true,
                },
                old.map(|e| e.dir),
                Some(entry.dir),
            ));
        } else if old.is_none() {
            added.push(entry.name.clone());
        }
    }
    for entry in &server_entries {
        let lower = entry.name.to_lowercase();
        if !pack_by.contains_key(&lower)
            && (FOLDERS.contains(&lower.as_str())
                || (!entry.dir && SERVER_ONLY.contains(&lower.as_str())))
        {
            managed.push((
                Item {
                    path: entry.name.clone(),
                    old: Some(entry.name.clone()),
                    new: false,
                },
                Some(entry.dir),
                None,
            ));
        }
    }
    managed.sort_by(|a, b| a.0.path.cmp(&b.0.path));
    added.sort();
    // Links and special files are never moved implicitly.
    for (item, _, _) in &managed {
        if let Some(old) = &item.old {
            if !server_by[&old.to_lowercase()].plain {
                return Err(format!("Links/special files cannot be updated: {old}"));
            }
        }
        if item.new {
            checked_source(&payload_root, &item.path)?;
        }
    }

    let mut scripts = Vec::new();
    for (item, old_dir, new_dir) in &managed {
        if *old_dir == Some(true) || *new_dir == Some(true) || !is_script(&item.path.to_lowercase())
        {
            continue;
        }
        let old = match &item.old {
            Some(old) => read_small(&server.join(old))?,
            None => None,
        };
        let new = if item.new {
            read_small(&payload_root.join(&item.path))?
        } else {
            None
        };
        let text = |bytes: &Vec<u8>| String::from_utf8_lossy(bytes).into_owned();
        match (&item.old, item.new, old, new) {
            (Some(_), true, Some(o), Some(n)) if o != n => scripts.push(ScriptChange {
                path: item.path.clone(),
                old: Some(text(&o)),
                new: Some(text(&n)),
            }),
            (Some(_), false, Some(o), _) => scripts.push(ScriptChange {
                path: item.path.clone(),
                old: Some(text(&o)),
                new: None,
            }),
            _ => {}
        }
    }
    scripts.sort_by(|a, b| a.path.cmp(&b.path));

    let java_change = match (
        server_by.contains_key("java9args.txt"),
        pack_by.contains_key("java9args.txt"),
    ) {
        (false, true) => Some(JavaChange::ToJava17),
        (true, false) => Some(JavaChange::ToJava8),
        _ => None,
    };

    let mut journey = None;
    if let (Some(config_old), Some(config_new)) = (
        find_ci(&server, "config")?,
        find_ci(&payload_root, "config")?,
    ) {
        if let Some(name) = find_ci(&server.join(&config_old), "JourneyMapServer")? {
            if is_real_dir(&server.join(&config_old).join(&name)) {
                journey = Some(Journey {
                    config_old,
                    config_new,
                    name,
                });
            }
        }
    }

    let mut items: Vec<Item> = managed.iter().map(|(i, _, _)| i.clone()).collect();
    items.extend(added.iter().map(|path| Item {
        path: path.clone(),
        old: None,
        new: true,
    }));
    let preview = ServerPreview {
        server: server.to_string_lossy().into_owned(),
        backup: final_backup.to_string_lossy().into_owned(),
        previous_backup,
        replaced: managed
            .iter()
            .map(|(item, old_dir, new_dir)| ServerItem {
                path: item.path.clone(),
                directory: new_dir.or(*old_dir).unwrap_or(false),
                old: item.old.is_some(),
                new: item.new,
            })
            .collect(),
        added,
        journey_map: journey.is_some(),
        scripts,
        java_change,
        recovered,
        mods: plan,
        files,
        missing,
    };
    Ok(PreparedServerUpdate {
        staging,
        server,
        payload_root,
        final_backup,
        items,
        journey,
        pack_name: pack
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        preview,
        additions,
    })
}

enum Stop {
    Error(String),
    Crash(String),
}
impl From<String> for Stop {
    fn from(e: String) -> Self {
        Stop::Error(e)
    }
}

impl PreparedServerUpdate {
    pub fn preview(&self) -> &ServerPreview {
        &self.preview
    }
    pub fn finish(
        self,
        decisions: &[Decision],
        progress: impl Fn(Progress),
    ) -> Result<PathBuf, String> {
        self.run(decisions, None, progress)
    }
    #[doc(hidden)]
    pub fn finish_with_fault(
        self,
        decisions: &[Decision],
        fault: Fault,
        progress: impl Fn(Progress),
    ) -> Result<PathBuf, String> {
        self.run(decisions, Some(fault), progress)
    }

    fn journal(&self, committed: bool) -> Journal {
        Journal {
            version: 1,
            staging: self.staging.path().to_owned(),
            payload_root: self.payload_root.clone(),
            backup: self.staging.path().join("backup"),
            final_backup: self.final_backup.clone(),
            items: self.items.clone(),
            committed,
        }
    }

    fn apply(&self, fault: Option<Fault>, progress: &impl Fn(Progress)) -> Result<(), Stop> {
        let journal = self.journal(false);
        let mut moves = 0usize;
        let check = |moves: usize| -> Result<(), Stop> {
            match fault {
                Some(Fault::FailAfterMoves(n)) if moves >= n => {
                    Err(Stop::Error("Injected failure".into()))
                }
                Some(Fault::CrashAfterMoves(n)) if moves >= n => {
                    Err(Stop::Crash("Injected crash".into()))
                }
                _ => Ok(()),
            }
        };
        for (index, item) in journal.items.iter().enumerate() {
            if let Some(old) = &item.old {
                check(moves)?;
                move_path(&self.server.join(old), &journal.backup.join(old), old)?;
                moves += 1;
            }
            if item.new {
                check(moves)?;
                move_path(
                    &journal.payload_root.join(&item.path),
                    &self.server.join(&item.path),
                    &item.path,
                )?;
                moves += 1;
            }
            progress(Progress {
                stage: "replacing",
                done: index + 1,
                total: journal.items.len(),
            });
        }
        check(moves)?;
        if let Some(journey) = &self.journey {
            let target_config = self.server.join(&journey.config_new);
            if let Some(existing) = find_ci(&target_config, &journey.name)? {
                let existing = target_config.join(existing);
                if is_real_dir(&existing) {
                    fs::remove_dir_all(&existing).map_err(error)?;
                } else {
                    fs::remove_file(&existing).map_err(error)?;
                }
            }
            copy_dir(
                &journal.backup.join(&journey.config_old).join(&journey.name),
                &target_config.join(&journey.name),
            )?;
        }
        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let manifest = Manifest {
            created_at,
            pack: &self.pack_name,
            items: &self.items,
        };
        let bytes = serde_json::to_vec_pretty(&manifest).map_err(error)?;
        fs::write(journal.backup.join(MANIFEST), bytes).map_err(error)?;
        write_journal(&self.server, &self.journal(true))?;
        if matches!(fault, Some(Fault::CrashAfterCommit)) {
            return Err(Stop::Crash("Injected crash after commit".into()));
        }
        Ok(())
    }

    fn run(
        self,
        decisions: &[Decision],
        fault: Option<Fault>,
        progress: impl Fn(Progress),
    ) -> Result<PathBuf, String> {
        // Everything below happens in staging; a failure leaves the server untouched.
        validate_decisions(&self.preview.files, decisions)?;
        transfer_files(
            &self.server,
            &self.payload_root,
            &self.preview.files,
            decisions,
            &progress,
        )?;
        mods::apply_additions(&self.payload_root, &self.additions)?;
        write_journal(&self.server, &self.journal(false))?;
        match self.apply(fault, &progress) {
            Ok(()) => {}
            Err(Stop::Crash(message)) => {
                let _ = self.staging.keep();
                return Err(message);
            }
            Err(Stop::Error(e)) => {
                let journal = self.journal(false);
                return match rollback(&self.server, &journal) {
                    Ok(()) => {
                        let _ = remove_journal(&self.server);
                        Err(format!("{e}. All changes were rolled back."))
                    }
                    Err(failure) => {
                        let staging = self.staging.keep();
                        Err(format!(
                            "{e}. Rollback failed ({failure}); the update is incomplete. \
                             The previous files are in {}. The next update attempt will retry recovery.",
                            staging.join("backup").display()
                        ))
                    }
                };
            }
        }
        progress(Progress {
            stage: "publishing",
            done: 0,
            total: 1,
        });
        let backup = self.staging.path().join("backup");
        if let Err(e) = finalize_backup(&backup, &self.final_backup) {
            let _ = self.staging.keep();
            return Err(format!(
                "The server was updated, but the backup could not be finalized: {e}. \
                 The next update attempt will complete it."
            ));
        }
        remove_journal(&self.server)?;
        // The server is already updated: a locked leftover must not fail it; it is retried later.
        let _ = remove_dir_retrying(&self.staging.keep());
        Ok(self.server.clone())
    }
}

fn write_journal(server: &Path, journal: &Journal) -> Result<(), String> {
    use std::io::Write;
    let mut temporary = tempfile::NamedTempFile::new_in(server).map_err(error)?;
    temporary
        .write_all(&serde_json::to_vec_pretty(journal).map_err(error)?)
        .map_err(error)?;
    temporary.as_file().sync_all().map_err(error)?;
    temporary
        .persist(server.join(JOURNAL))
        .map_err(|e| e.error.to_string())?;
    Ok(())
}
fn remove_journal(server: &Path) -> Result<(), String> {
    match fs::remove_file(server.join(JOURNAL)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(error(e)),
    }
}
fn copy_dir(source: &Path, destination: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(source).map_err(error)?;
    if metadata.file_type().is_symlink() || !(metadata.is_dir() || metadata.is_file()) {
        return Err(format!(
            "Links/special files cannot be copied: {}",
            source.display()
        ));
    }
    if metadata.is_dir() {
        fs::create_dir_all(destination).map_err(error)?;
        for entry in fs::read_dir(source).map_err(error)? {
            let entry = entry.map_err(error)?;
            copy_dir(&entry.path(), &destination.join(entry.file_name()))?;
        }
    } else {
        fs::copy(source, destination).map_err(error)?;
    }
    Ok(())
}
// Idempotent: derived from the filesystem state plus the journal.
fn rollback(server: &Path, journal: &Journal) -> Result<(), String> {
    for item in journal.items.iter().rev() {
        if item.new {
            let in_server = server.join(&item.path);
            let in_payload = journal.payload_root.join(&item.path);
            if exists(&in_server) && !exists(&in_payload) {
                move_path(&in_server, &in_payload, &item.path)?;
            }
        }
        if let Some(old) = &item.old {
            let in_backup = journal.backup.join(old);
            let in_server = server.join(old);
            if exists(&in_backup) && !exists(&in_server) {
                move_path(&in_backup, &in_server, old)?;
            }
        }
    }
    Ok(())
}
// Tolerates an already moved backup so a half-finished roll-forward can be repeated.
fn finalize_backup(backup: &Path, final_backup: &Path) -> Result<(), String> {
    if !exists(backup) {
        return Ok(());
    }
    if exists(final_backup) {
        if !is_real_dir(final_backup) {
            return Err("The backup location is occupied by something that is not a folder".into());
        }
        fs::remove_dir_all(final_backup).map_err(error)?;
    }
    publish(backup, final_backup).map_err(error)
}

fn validate_journal(server: &Path, journal: &Journal) -> Result<(), String> {
    let invalid = || "Invalid update journal; it was left untouched".to_string();
    let parent = server.parent().ok_or_else(invalid)?;
    let server_name = server
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(invalid)?;
    let staging_ok = journal.staging.parent() == Some(parent)
        && journal
            .staging
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_prefix(STAGING_PREFIX))
            .is_some_and(|s| s.len() == 6 && s.bytes().all(|b| b.is_ascii_alphanumeric()));
    let single = |name: &str| validate_rule_path(name) && !name.contains('/');
    if journal.version != 1
        || !staging_ok
        || journal.final_backup.parent() != Some(parent)
        || journal.final_backup.file_name().and_then(|n| n.to_str())
            != Some(format!("{server_name}.gtnh-backup").as_str())
        || journal.backup != journal.staging.join("backup")
        || !journal
            .payload_root
            .starts_with(journal.staging.join("payload"))
        || journal
            .items
            .iter()
            .any(|i| !single(&i.path) || i.old.as_deref().is_some_and(|o| !single(o)))
    {
        return Err(invalid());
    }
    Ok(())
}

/// Finishes or undoes an update interrupted by a crash; `None` when nothing was pending.
pub fn recover(server: &Path) -> Result<Option<Recovery>, String> {
    let server = server.canonicalize().map_err(error)?;
    let bytes = match fs::read(server.join(JOURNAL)) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(error(e)),
    };
    let journal: Journal =
        serde_json::from_slice(&bytes).map_err(|e| format!("Invalid update journal: {e}"))?;
    validate_journal(&server, &journal)?;
    let result = if journal.committed {
        finalize_backup(&journal.backup, &journal.final_backup)?;
        Recovery::Completed
    } else {
        rollback(&server, &journal)?;
        Recovery::RolledBack
    };
    remove_journal(&server)?;
    if is_real_dir(&journal.staging) {
        remove_dir_retrying(&journal.staging)?;
    }
    Ok(Some(result))
}

/// Removes leftover staging folders of finished updates; does nothing while a journal exists,
/// because an interrupted update must be recovered first (its staging holds the old files).
pub fn cleanup_leftovers(server: &Path) -> Result<usize, String> {
    let server = server.canonicalize().map_err(error)?;
    if exists(&server.join(JOURNAL)) {
        return Ok(0);
    }
    remove_stale_staging(server.parent().ok_or("The server has no parent folder")?)
}

fn remove_stale_staging(parent: &Path) -> Result<usize, String> {
    let mut removed = 0;
    for item in fs::read_dir(parent).map_err(error)?.flatten() {
        let matches = item.file_name().to_str().is_some_and(|name| {
            name.strip_prefix(STAGING_PREFIX)
                .is_some_and(|s| s.len() == 6 && s.bytes().all(|b| b.is_ascii_alphanumeric()))
        });
        if matches && is_real_dir(&item.path()) && remove_dir_retrying(&item.path()).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

// Antivirus or indexers may briefly lock freshly extracted files on Windows.
fn remove_dir_retrying(path: &Path) -> Result<(), String> {
    const ATTEMPTS: u32 = 10;
    let mut attempt = 1;
    loop {
        match fs::remove_dir_all(path) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) if attempt >= ATTEMPTS => return Err(error(e)),
            Err(e) => {
                if e.kind() == io::ErrorKind::PermissionDenied && cfg!(windows) {
                    clear_readonly(path);
                }
            }
        }
        attempt += 1;
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}
// Read-only files cannot be deleted on Windows; never follows links.
fn clear_readonly(path: &Path) {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return;
    };
    if metadata.file_type().is_symlink() {
        return;
    }
    if metadata.is_dir() {
        if let Ok(read_dir) = fs::read_dir(path) {
            for entry in read_dir.flatten() {
                clear_readonly(&entry.path());
            }
        }
    } else if metadata.permissions().readonly() {
        let mut permissions = metadata.permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        let _ = fs::set_permissions(path, permissions);
    }
}
