//! Mod rules: identify jars by `modid` (from `mcmod.info`) or, failing that, a filename stem,
//! so "add" and "remove" rules keep working when version numbers in file names change.
use crate::{installation::Transfer, validate_rule_path, ModAction, ModRule};
use serde::Serialize;
use std::{
    collections::{BTreeSet, HashMap},
    fs,
    io::Read,
    path::{Path, PathBuf},
};

const MAX_MCMOD_INFO: u64 = 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModInfo {
    pub name: String,
    pub mod_ids: Vec<String>,
    pub stem: String,
    pub file: String,
}
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModRemoval {
    pub name: String,
    pub files: Vec<String>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModAddition {
    pub name: String,
    pub file: String,
    pub source: String,
    pub replaces: Vec<String>,
    pub missing: bool,
}
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModPlan {
    pub removed: Vec<ModRemoval>,
    pub added: Vec<ModAddition>,
    pub duplicates: Vec<Vec<String>>,
}
/// Existing add sources as (source path, file name inside `mods/`).
pub(crate) type Sources = Vec<(PathBuf, String)>;

fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Lowercase name without the version suffix: `journeymap-1.7.10-5.2.19.jar` -> `journeymap`.
pub fn stem_of(file_name: &str) -> String {
    let lower = file_name.to_lowercase();
    let base = [".jar", ".zip"]
        .iter()
        .find_map(|e| lower.strip_suffix(e))
        .unwrap_or(&lower);
    let is_version = |token: &str| {
        let after = |prefix: &str| {
            token
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
        };
        token.starts_with(|c: char| c.is_ascii_digit()) || after("v") || after("mc")
    };
    let kept: Vec<_> = base
        .split(['-', '_', ' ', '+'])
        .filter(|t| !t.is_empty())
        .take_while(|t| !is_version(t))
        .collect();
    if kept.is_empty() {
        base.to_owned()
    } else {
        kept.join("-")
    }
}

struct Meta {
    name: Option<String>,
    ids: Vec<String>,
}

// Values of `"key" : "value"` pairs; tolerates the broken JSON many old mods ship.
fn scan_values(text: &str, key: &str) -> Vec<String> {
    let needle = format!("\"{key}\"");
    let mut values = Vec::new();
    let mut rest = text;
    while let Some(position) = rest.find(&needle) {
        rest = &rest[position + needle.len()..];
        let Some(after) = rest.trim_start().strip_prefix(':') else {
            continue;
        };
        let Some(after) = after.trim_start().strip_prefix('"') else {
            continue;
        };
        let mut value = String::new();
        let mut chars = after.chars();
        while let Some(c) = chars.next() {
            match c {
                '"' => break,
                '\\' => value.extend(chars.next()),
                _ => value.push(c),
            }
        }
        values.push(value);
    }
    values
}
fn parse_mcmod(text: &str) -> Meta {
    let text = text.trim_start_matches('\u{feff}');
    let (name, ids) = match serde_json::from_str::<serde_json::Value>(text) {
        Ok(json) => {
            let list = match &json {
                serde_json::Value::Array(items) => items.as_slice(),
                serde_json::Value::Object(object) => match object.get("modList") {
                    Some(serde_json::Value::Array(items)) => items.as_slice(),
                    _ => std::slice::from_ref(&json),
                },
                _ => &[],
            };
            let field = |key: &str| -> Vec<String> {
                list.iter()
                    .filter_map(|m| m.get(key).and_then(|v| v.as_str()).map(str::to_owned))
                    .collect()
            };
            (field("name"), field("modid"))
        }
        Err(_) => (scan_values(text, "name"), scan_values(text, "modid")),
    };
    let ids: BTreeSet<_> = ids
        .iter()
        .map(|id| id.trim().to_lowercase())
        .filter(|id| !id.is_empty())
        .collect();
    Meta {
        name: name
            .first()
            .map(|n| n.trim().to_owned())
            .filter(|n| !n.is_empty()),
        ids: ids.into_iter().collect(),
    }
}
fn read_meta(path: &Path) -> Result<Meta, String> {
    let file = fs::File::open(path).map_err(error)?;
    let mut zip =
        zip::ZipArchive::new(file).map_err(|e| format!("Not a mod archive: {path:?}: {e}"))?;
    let entry = zip
        .file_names()
        .find(|n| n.eq_ignore_ascii_case("mcmod.info"))
        .map(str::to_owned);
    let mut meta = Meta {
        name: None,
        ids: Vec::new(),
    };
    if let Some(entry) = entry {
        if let Ok(file) = zip.by_name(&entry) {
            let mut bytes = Vec::new();
            if file
                .take(MAX_MCMOD_INFO + 1)
                .read_to_end(&mut bytes)
                .is_ok()
                && bytes.len() as u64 <= MAX_MCMOD_INFO
            {
                meta = parse_mcmod(&String::from_utf8_lossy(&bytes));
            }
        }
    }
    Ok(meta)
}
fn display_path(path: PathBuf) -> String {
    let text = path.to_string_lossy().into_owned();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => rest.to_owned(),
        _ => text,
    }
}
fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

pub fn inspect(path: &Path) -> Result<ModInfo, String> {
    let metadata = fs::symlink_metadata(path).map_err(error)?;
    if !metadata.is_file() {
        return Err("Not a mod archive: choose a regular file".into());
    }
    let meta = read_meta(path)?;
    let canonical = path.canonicalize().map_err(error)?;
    let name = file_name(&canonical);
    let fallback = name
        .strip_suffix(".jar")
        .or_else(|| name.strip_suffix(".JAR"))
        .unwrap_or(&name)
        .to_owned();
    Ok(ModInfo {
        name: meta.name.unwrap_or(fallback),
        mod_ids: meta.ids,
        stem: stem_of(&name),
        file: display_path(canonical),
    })
}

fn same_mod(a_ids: &[String], a_stem: &str, b_ids: &[String], b_stem: &str) -> bool {
    if a_ids.is_empty() || b_ids.is_empty() {
        a_stem == b_stem
    } else {
        a_ids.iter().any(|id| b_ids.contains(id))
    }
}

struct Jar {
    relative: String,
    ids: Vec<String>,
    stem: String,
}
// Broken jars count as having no ids (stem only); they never fail the operation.
fn jar_ids(path: &Path) -> Vec<String> {
    read_meta(path).map(|m| m.ids).unwrap_or_default()
}
fn scan(game: &Path) -> Vec<Jar> {
    let mut jars = Vec::new();
    for directory in ["mods", "mods/1.7.10"] {
        let Ok(entries) = fs::read_dir(game.join(directory)) else {
            continue;
        };
        for entry in entries.flatten() {
            let (Ok(kind), Ok(name)) = (entry.file_type(), entry.file_name().into_string()) else {
                continue;
            };
            if kind.is_file() && name.to_lowercase().ends_with(".jar") {
                jars.push(Jar {
                    relative: format!("{directory}/{name}"),
                    ids: jar_ids(&entry.path()),
                    stem: stem_of(&name),
                });
            }
        }
    }
    jars.sort_by(|a, b| a.relative.cmp(&b.relative));
    jars
}
fn normalized(ids: &[String]) -> Vec<String> {
    let ids: BTreeSet<_> = ids
        .iter()
        .map(|id| id.trim().to_lowercase())
        .filter(|id| !id.is_empty())
        .collect();
    ids.into_iter().collect()
}

/// Applies enabled rules to the staged game folder (removals and replaced pack mods only).
/// Returns the plan so far and the add sources that exist and must be copied at the end.
pub(crate) fn apply_rules(game: &Path, rules: &[ModRule]) -> Result<(ModPlan, Sources), String> {
    let mut jars = scan(game);
    let mut plan = ModPlan::default();
    let mut sources = Sources::new();
    for rule in rules.iter().filter(|r| r.enabled) {
        let ids = normalized(&rule.mod_ids);
        let stem = rule.stem.to_lowercase();
        let mut matched = Vec::new();
        jars.retain(|jar| {
            let hit = same_mod(&ids, &stem, &jar.ids, &jar.stem);
            if hit {
                matched.push(jar.relative.clone());
            }
            !hit
        });
        for relative in &matched {
            fs::remove_file(game.join(relative)).map_err(error)?;
        }
        matched.sort();
        match rule.action {
            ModAction::Remove => plan.removed.push(ModRemoval {
                name: rule.name.clone(),
                files: matched,
            }),
            ModAction::Add => {
                let source = rule
                    .file
                    .as_deref()
                    .filter(|f| !f.is_empty())
                    .ok_or("Add rule without a mod file")?;
                let path = PathBuf::from(source);
                let name = file_name(&path);
                if !validate_rule_path(&name) {
                    return Err(format!("Invalid mod file name: {source}"));
                }
                let missing = !fs::symlink_metadata(&path).is_ok_and(|m| m.is_file());
                if !missing {
                    sources.push((path, name.clone()));
                }
                plan.added.push(ModAddition {
                    name: rule.name.clone(),
                    file: format!("mods/{name}"),
                    source: source.to_owned(),
                    replaces: matched,
                    missing,
                });
            }
        }
    }
    Ok((plan, sources))
}

pub(crate) fn apply_additions(game: &Path, sources: &Sources) -> Result<(), String> {
    for (source, name) in sources {
        if !fs::symlink_metadata(source).is_ok_and(|m| m.is_file()) {
            return Err(format!("Mod file disappeared: {}", source.display()));
        }
        let target = game.join("mods").join(name);
        fs::create_dir_all(target.parent().ok_or("Invalid mod target")?).map_err(error)?;
        fs::copy(source, &target).map_err(error)?;
    }
    Ok(())
}

/// Groups of final `mods/...` files that share an identity (simulated before anything is copied).
pub(crate) fn find_duplicates(
    game: &Path,
    sources: &Sources,
    transfers: &[Transfer],
    transfer_root: &Path,
) -> Vec<Vec<String>> {
    let mut finals: Vec<(String, Vec<String>, String)> = scan(game)
        .into_iter()
        .map(|j| (j.relative, j.ids, j.stem))
        .collect();
    for (source, name) in sources {
        finals.push((format!("mods/{name}"), jar_ids(source), stem_of(name)));
    }
    for t in transfers {
        let lower = t.path.to_lowercase();
        let wanted = !t.directory
            && lower.ends_with(".jar")
            && lower.starts_with("mods/")
            && !(t.exists && t.conflict == crate::Conflict::Keep);
        if wanted {
            let name = t.path.rsplit('/').next().unwrap_or_default();
            finals.push((
                t.path.clone(),
                jar_ids(&transfer_root.join(&t.path)),
                stem_of(name),
            ));
        }
    }
    let mut groups: HashMap<String, BTreeSet<String>> = HashMap::new();
    for (path, ids, stem) in finals {
        let keys: Vec<_> = if ids.is_empty() {
            vec![format!("stem:{stem}")]
        } else {
            ids.iter().map(|id| format!("id:{id}")).collect()
        };
        for key in keys {
            groups.entry(key).or_default().insert(path.clone());
        }
    }
    let unique: BTreeSet<Vec<String>> = groups
        .into_values()
        .filter(|g| g.len() >= 2)
        .map(|g| g.into_iter().collect())
        .collect();
    unique.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn jar(path: &Path, entries: &[(&str, &str)]) {
        let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
        for (name, text) in entries {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(text.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }
    fn inspect_with(info: &[(&str, &str)]) -> ModInfo {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Example-1.2.jar");
        jar(&path, info);
        inspect(&path).unwrap()
    }

    #[test]
    fn stems_drop_version_suffixes() {
        for (name, stem) in [
            ("journeymap-1.7.10-5.2.19-unlimited.jar", "journeymap"),
            ("InfoHUD-0.0.20.jar", "infohud"),
            ("passivemobs-1.3.0.jar", "passivemobs"),
            ("NotEnoughItems-2.7.29-GTNH.jar", "notenoughitems"),
            ("gregtech-5.09.51.jar", "gregtech"),
            ("ae2stuff-0.9.1-GTNH.jar", "ae2stuff"),
            ("Baubles-1.7.10-1.0.1.10.jar", "baubles"),
            (
                "CodeChickenLib-1.7.10-1.1.3.140-universal.jar",
                "codechickenlib",
            ),
            ("lwjgl3ify-2.1.5-forgePatches.jar", "lwjgl3ify"),
            ("mod_v2.jar", "mod"),
            ("1.7.10-thing.jar", "1.7.10-thing"),
        ] {
            assert_eq!(stem_of(name), stem, "{name}");
        }
    }
    #[test]
    fn array_mcmod_info() {
        let info = inspect_with(&[(
            "mcmod.info",
            r#"[{"modid":"JourneyMap","name":"Journey Map"},{"modid":"extra"},{"modid":"extra"}]"#,
        )]);
        assert_eq!(info.mod_ids, ["extra", "journeymap"]);
        assert_eq!(info.name, "Journey Map");
        assert_eq!(info.stem, "example");
        assert!(Path::new(&info.file).is_absolute());
    }
    #[test]
    fn mod_list_v2_and_case_insensitive_entry() {
        let info = inspect_with(&[(
            "McMod.Info",
            r#"{"modListVersion":2,"modList":[{"modid":"abc","name":"ABC"}]}"#,
        )]);
        assert_eq!(info.mod_ids, ["abc"]);
        assert_eq!(info.name, "ABC");
    }
    #[test]
    fn malformed_mcmod_info_uses_fallback_scan() {
        let info = inspect_with(&[(
            "mcmod.info",
            "[{\n\"modid\" : \"broken\",\n\"name\": \"Broken\n Mod\",\n}, ]",
        )]);
        assert_eq!(info.mod_ids, ["broken"]);
        assert!(info.name.starts_with("Broken"));
    }
    #[test]
    fn missing_mcmod_info_has_no_ids() {
        let info = inspect_with(&[("a.class", "x")]);
        assert!(info.mod_ids.is_empty());
        assert_eq!(info.name, "Example-1.2");
        assert_eq!(info.stem, "example");
    }
    #[test]
    fn non_zip_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.jar");
        fs::write(&path, "not a zip").unwrap();
        assert!(inspect(&path).unwrap_err().starts_with("Not a mod archive"));
        assert!(inspect(dir.path()).is_err());
    }
    #[test]
    fn identity_prefers_ids_over_stems() {
        let ids = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(same_mod(&ids(&["a"]), "x", &ids(&["a", "b"]), "y"));
        assert!(!same_mod(&ids(&["a"]), "x", &ids(&["b"]), "x"));
        assert!(same_mod(&[], "x", &ids(&["b"]), "x"));
        assert!(!same_mod(&[], "x", &[], "y"));
    }
}
