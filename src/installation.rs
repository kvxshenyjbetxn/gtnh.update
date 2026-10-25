//! Staged, non-destructive installation of client/server ZIPs and local folders.
use crate::{validate_rule_path, Conflict, Rule};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Transfer {
    pub path: String,
    pub exists: bool,
    pub conflict: Conflict,
    pub directory: bool,
    pub bytes: u64,
}
#[derive(Debug, Clone, Serialize)]
pub struct Preview {
    pub destination: String,
    pub files: Vec<Transfer>,
    pub missing: Vec<String>,
}
impl Transfer {
    fn will_copy(&self, decisions: &HashMap<&str, Conflict>) -> bool {
        !self.directory
            && (!self.exists
                || self.conflict == Conflict::Replace
                || (self.conflict == Conflict::Ask
                    && decisions.get(self.path.as_str()) == Some(&Conflict::Replace)))
    }
}
impl Preview {
    /// Bytes known to be copied; unresolved Ask conflicts are budgeted at confirmation.
    pub fn required_bytes(&self, decisions: &[Decision]) -> Result<u64, String> {
        let decisions: HashMap<_, _> = decisions
            .iter()
            .map(|d| (d.path.as_str(), d.conflict))
            .collect();
        self.files
            .iter()
            .filter(|f| f.will_copy(&decisions))
            .try_fold(0u64, |sum, f| {
                sum.checked_add(f.bytes)
                    .ok_or_else(|| "Transfer size overflow".into())
            })
    }
}
#[derive(Debug, Clone, Deserialize)]
pub struct Decision {
    pub path: String,
    pub conflict: Conflict,
}
#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    pub stage: &'static str,
    pub done: usize,
    pub total: usize,
}
pub struct Prepared {
    _temporary: tempfile::TempDir,
    root: PathBuf,
    source: PathBuf,
    game: PathBuf,
    destination: PathBuf,
    preview: Preview,
}
fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}

// Track every component, not just full filenames: Mods/a and mods/b collide on Windows.
fn register(
    names: &mut HashMap<String, (String, bool)>,
    path: &str,
    directory: bool,
) -> Result<(), String> {
    let parts: Vec<_> = path.split('/').collect();
    for end in 1..=parts.len() {
        let name = parts[..end].join("/");
        let is_dir = end < parts.len() || directory;
        if let Some((previous, was_dir)) = names.get(&name.to_lowercase()) {
            if *previous != name || *was_dir != is_dir || (!is_dir && end == parts.len()) {
                return Err(format!("Conflicting archive/transfer paths: {name}"));
            }
        } else {
            names.insert(name.to_lowercase(), (name, is_dir));
        }
    }
    Ok(())
}
// zip 2.x coalesces identical names internally. Count raw central-directory
// records at its validated offset so hidden duplicate entries cannot bypass
// our path checks. ZIP64 uses the same fixed header/variable-length fields.
fn reject_coalesced_entries(
    path: &Path,
    directory_start: u64,
    visible: usize,
) -> Result<(), String> {
    let mut reader = fs::File::open(path).map_err(error)?;
    reader
        .seek(SeekFrom::Start(directory_start))
        .map_err(error)?;
    let mut count = 0usize;
    loop {
        let mut header = [0u8; 46];
        reader.read_exact(&mut header[..4]).map_err(error)?;
        if &header[..4] != b"PK\x01\x02" {
            break;
        }
        reader.read_exact(&mut header[4..]).map_err(error)?;
        count += 1;
        if count > visible || count > 200_000 {
            return Err("Duplicate ZIP entries or excessive entry count".into());
        }
        let variable_bytes: i64 = [28, 30, 32]
            .iter()
            .map(|&offset| u16::from_le_bytes([header[offset], header[offset + 1]]) as i64)
            .sum();
        reader
            .seek(SeekFrom::Current(variable_bytes))
            .map_err(error)?;
    }
    if count != visible {
        return Err("Inconsistent ZIP central directory".into());
    }
    Ok(())
}
fn extract(archive: &Path, into: &Path, progress: &impl Fn(Progress)) -> Result<(), String> {
    let mut zip = zip::ZipArchive::new(fs::File::open(archive).map_err(error)?).map_err(error)?;
    if zip.len() > 200_000 {
        return Err("Too many archive entries".into());
    }
    reject_coalesced_entries(archive, zip.central_directory_start(), zip.len())?;
    let mut names = HashMap::new();
    let mut size = 0u64;
    // Validate the complete directory before writing anything.
    for index in 0..zip.len() {
        let file = zip.by_index(index).map_err(error)?;
        let name = file.name().strip_suffix('/').unwrap_or(file.name());
        if !validate_rule_path(name) {
            return Err(format!("Unsafe archive path: {}", file.name()));
        }
        if let Some(mode) = file.unix_mode() {
            let kind = mode & 0o170000;
            if kind != 0 && kind != 0o100000 && kind != 0o040000 {
                return Err(format!(
                    "Archive links/special files are not supported: {name}"
                ));
            }
        }
        register(&mut names, name, file.is_dir())?;
        size = size.checked_add(file.size()).ok_or("Archive too large")?;
        if size > 32 * 1024 * 1024 * 1024 {
            return Err("Archive exceeds 32 GiB extraction limit".into());
        }
    }
    if fs2::available_space(into).map_err(error)? < size {
        return Err("Not enough free space to extract the archive".into());
    }
    for index in 0..zip.len() {
        let mut file = zip.by_index(index).map_err(error)?;
        let path = into.join(file.name());
        if file.is_dir() {
            fs::create_dir_all(&path).map_err(error)?;
        } else {
            fs::create_dir_all(path.parent().ok_or("Invalid archive path")?).map_err(error)?;
            let mut output = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(error)?;
            let expected = file.size();
            let copied = io::copy(
                &mut (&mut file).take(expected.saturating_add(1)),
                &mut output,
            )
            .map_err(error)?;
            if copied != expected {
                return Err("Archive entry size mismatch".into());
            }
            output.sync_all().map_err(error)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                // Preserve executable bits only; never restore setuid/setgid or global write access.
                let executable = file.unix_mode().unwrap_or(0) & 0o111;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o644 | executable))
                    .map_err(error)?;
            }
        }
        drop(file);
        progress(Progress {
            stage: "extracting",
            done: index + 1,
            total: zip.len(),
        });
    }
    Ok(())
}
fn pack_root(extracted: &Path, client: bool) -> Result<PathBuf, String> {
    let valid = |path: &Path| {
        if client {
            path.join("instance.cfg").is_file() && path.join("mmc-pack.json").is_file()
        } else {
            path.join("mods").is_dir() && path.join("config").is_dir()
        }
    };
    if valid(extracted) {
        return Ok(extracted.to_owned());
    }
    let mut candidates = Vec::new();
    for entry in fs::read_dir(extracted).map_err(error)? {
        let path = entry.map_err(error)?.path();
        if path.is_dir() && valid(&path) {
            candidates.push(path);
        }
    }
    if candidates.len() == 1 {
        Ok(candidates.remove(0))
    } else {
        Err(
            "Unsupported pack layout: expected a Prism instance or server with mods and config"
                .into(),
        )
    }
}
fn checked_source(root: &Path, relative: &str) -> Result<Option<fs::Metadata>, String> {
    let mut current = root.to_owned();
    let mut last = None;
    for component in relative.split('/') {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || (!metadata.is_file() && !metadata.is_dir())
                {
                    return Err(format!(
                        "Links/special files cannot be transferred: {relative}"
                    ));
                }
                last = Some(metadata);
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(error(e)),
        }
    }
    if !current.canonicalize().map_err(error)?.starts_with(root) {
        return Err(format!("Transfer outside source: {relative}"));
    }
    Ok(last)
}
fn collect(
    root: &Path,
    path: &str,
    conflict: Conflict,
    files: &mut BTreeMap<String, Transfer>,
) -> Result<(), String> {
    if !validate_rule_path(path) {
        return Err(format!("Unsafe transfer path: {path}"));
    }
    let metadata =
        checked_source(root, path)?.ok_or_else(|| format!("Source disappeared: {path}"))?;
    files.insert(
        path.into(),
        Transfer {
            path: path.into(),
            exists: false,
            conflict,
            directory: metadata.is_dir(),
            bytes: metadata.len(),
        },
    );
    if metadata.is_dir() {
        for entry in fs::read_dir(root.join(path)).map_err(error)? {
            let name = entry
                .map_err(error)?
                .file_name()
                .into_string()
                .map_err(|_| "Non-Unicode source path")?;
            collect(root, &format!("{path}/{name}"), conflict, files)?;
        }
    }
    Ok(())
}

fn stage_folder(pack: &Path, into: &Path, progress: &impl Fn(Progress)) -> Result<(), String> {
    let mut entries = BTreeMap::new();
    for entry in fs::read_dir(pack).map_err(error)? {
        let name = entry
            .map_err(error)?
            .file_name()
            .into_string()
            .map_err(|_| "Non-Unicode pack path")?;
        collect(pack, &name, Conflict::Replace, &mut entries)?;
    }
    if entries.len() > 200_000 {
        return Err("Too many pack entries".into());
    }
    let mut aliases = HashMap::new();
    let mut bytes = 0u64;
    for entry in entries.values() {
        register(&mut aliases, &entry.path, entry.directory)?;
        if !entry.directory {
            bytes = bytes.checked_add(entry.bytes).ok_or("Pack size overflow")?;
        }
    }
    if bytes > 32 * 1024 * 1024 * 1024 {
        return Err("Pack exceeds 32 GiB staging limit".into());
    }
    if fs2::available_space(into).map_err(error)? < bytes {
        return Err("Not enough free space to copy the pack".into());
    }
    for (index, entry) in entries.values().enumerate() {
        let metadata = checked_source(pack, &entry.path)?.ok_or("Pack file disappeared")?;
        if metadata.is_dir() != entry.directory {
            return Err("Pack changed while preparing installation".into());
        }
        let target = into.join(&entry.path);
        if entry.directory {
            fs::create_dir_all(&target).map_err(error)?;
        } else {
            fs::create_dir_all(target.parent().ok_or("Invalid pack path")?).map_err(error)?;
            fs::copy(pack.join(&entry.path), &target).map_err(error)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(
                    &target,
                    fs::Permissions::from_mode(0o644 | (metadata.permissions().mode() & 0o111)),
                )
                .map_err(error)?;
            }
            #[cfg(windows)]
            {
                let mut permissions = fs::metadata(&target).map_err(error)?.permissions();
                permissions.set_readonly(false);
                fs::set_permissions(&target, permissions).map_err(error)?;
            }
        }
        progress(Progress {
            stage: "preparingPack",
            done: index + 1,
            total: entries.len(),
        });
    }
    Ok(())
}

pub fn prepare(
    archive: &Path,
    source: &Path,
    destination: &Path,
    client: bool,
    rules: &[Rule],
    progress: impl Fn(Progress),
) -> Result<Prepared, String> {
    let source = source.canonicalize().map_err(error)?;
    if !source.is_dir() {
        return Err("Source must be a directory".into());
    }
    let name = destination
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("Choose a new folder name")?;
    if !validate_rule_path(name) || name.contains('/') {
        return Err("Invalid destination folder name".into());
    }
    let parent = destination
        .parent()
        .ok_or("Choose a destination parent")?
        .canonicalize()
        .map_err(error)?;
    let destination = parent.join(name);
    let pack = archive.canonicalize().map_err(error)?;
    if pack.is_dir() && (destination.starts_with(&pack) || pack.starts_with(&destination)) {
        return Err(
            "The selected pack and destination must be separate, non-nested folders".into(),
        );
    }
    if destination.starts_with(&source) || source.starts_with(&destination) {
        return Err("Source and destination must be separate, non-nested folders".into());
    }
    if destination.try_exists().map_err(error)? || fs::symlink_metadata(&destination).is_ok() {
        return Err("Destination already exists; choose a new folder name".into());
    }
    let temporary = tempfile::Builder::new()
        .prefix(".gtnh-install-")
        .tempdir_in(parent)
        .map_err(error)?;
    // Even rootless client ZIPs must not expose instance.cfg directly in Prism's
    // watched instances directory before publication.
    let payload = temporary.path().join("payload");
    fs::create_dir(&payload).map_err(error)?;
    if pack.is_dir() {
        stage_folder(&pack, &payload, &progress)?;
    } else {
        extract(&pack, &payload, &progress)?;
    }
    let root = pack_root(&payload, client)?;
    if client {
        crate::instance_metadata::update_client_metadata(&source, &root, name)?;
    }
    let game = if client {
        let dot = root.join(".minecraft");
        let plain = root.join("minecraft");
        if dot.is_dir() && plain.is_dir() {
            return Err("Ambiguous Minecraft directory".into());
        }
        if plain.is_dir() {
            plain
        } else {
            fs::create_dir_all(&dot).map_err(error)?;
            dot
        }
    } else {
        root.clone()
    };
    let mut files = BTreeMap::new();
    let mut missing = Vec::new();
    let mut enabled: Vec<_> = rules.iter().filter(|r| r.enabled).collect();
    enabled.sort_by_key(|r| r.path.len()); // More-specific rules override parent rules.
    for rule in enabled {
        if !validate_rule_path(&rule.path) {
            return Err(format!("Unsafe rule: {}", rule.path));
        }
        if checked_source(&source, &rule.path)?.is_none() {
            missing.push(rule.path.clone());
            continue;
        }
        collect(&source, &rule.path, rule.conflict, &mut files)?;
    }
    let mut aliases = HashMap::new();
    // Include new pack paths in the case-alias/type check.
    fn scan(
        path: &Path,
        base: &Path,
        names: &mut HashMap<String, (String, bool)>,
    ) -> Result<(), String> {
        for entry in fs::read_dir(path).map_err(error)? {
            let entry = entry.map_err(error)?;
            let p = entry.path();
            let relative = p
                .strip_prefix(base)
                .map_err(error)?
                .to_str()
                .ok_or("Non-Unicode archive path")?
                .replace('\\', "/");
            register(names, &relative, p.is_dir())?;
            if p.is_dir() {
                scan(&p, base, names)?;
            }
        }
        Ok(())
    }
    scan(&game, &game, &mut aliases)?;
    for file in files.values_mut() {
        let target = game.join(&file.path);
        file.exists = target.exists();
        // Matching regular files are expected conflicts, not duplicate archive entries.
        if let Some((name, directory)) = aliases.get(&file.path.to_lowercase()) {
            if name != &file.path || *directory != file.directory {
                return Err(format!(
                    "Incompatible file/directory or letter case: {}",
                    file.path
                ));
            }
        } else {
            register(&mut aliases, &file.path, file.directory)?;
        }
    }
    let preview = Preview {
        destination: destination.to_string_lossy().into_owned(),
        files: files.into_values().collect(),
        missing,
    };
    if fs2::available_space(&game).map_err(error)? < preview.required_bytes(&[])? {
        return Err("Not enough free space for selected files".into());
    }
    Ok(Prepared {
        _temporary: temporary,
        root,
        game,
        source,
        destination,
        preview,
    })
}
impl Prepared {
    pub fn preview(&self) -> &Preview {
        &self.preview
    }
    pub fn finish(
        self,
        decisions: &[Decision],
        progress: impl Fn(Progress),
    ) -> Result<PathBuf, String> {
        let required_bytes = self.preview.required_bytes(decisions)?;
        let decisions: HashMap<_, _> = decisions
            .iter()
            .map(|d| (d.path.as_str(), d.conflict))
            .collect();
        // Resolve all questions before copying even the first file.
        for file in &self.preview.files {
            if file.exists
                && !file.directory
                && file.conflict == Conflict::Ask
                && !matches!(
                    decisions.get(file.path.as_str()),
                    Some(Conflict::Keep | Conflict::Replace)
                )
            {
                return Err(format!("Choose replace or keep for {}", file.path));
            }
        }
        if fs2::available_space(&self.game).map_err(error)? < required_bytes {
            return Err("Not enough free space for confirmed transfer".into());
        }
        for (index, file) in self.preview.files.iter().enumerate() {
            let metadata = checked_source(&self.source, &file.path)?
                .ok_or_else(|| format!("Source disappeared: {}", file.path))?;
            if metadata.is_dir() != file.directory {
                return Err(format!("Source changed: {}", file.path));
            }
            let target = self.game.join(&file.path);
            if file.directory {
                fs::create_dir_all(&target).map_err(error)?;
            } else if file.will_copy(&decisions) {
                fs::create_dir_all(target.parent().ok_or("Invalid transfer target")?)
                    .map_err(error)?;
                fs::copy(self.source.join(&file.path), &target).map_err(error)?;
            }
            progress(Progress {
                stage: "copying",
                done: index + 1,
                total: self.preview.files.len(),
            });
        }
        progress(Progress {
            stage: "publishing",
            done: 0,
            total: 1,
        });
        publish(&self.root, &self.destination).map_err(error)?;
        Ok(self.destination.clone())
    }
}
// Atomic no-replace publication, including empty directories created after preview.
#[cfg(unix)]
fn publish(source: &Path, destination: &Path) -> io::Result<()> {
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        source,
        rustix::fs::CWD,
        destination,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(Into::into)
}
#[cfg(windows)]
fn publish(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}
