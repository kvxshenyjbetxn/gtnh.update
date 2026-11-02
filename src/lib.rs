//! Platform-independent settings and migration rules. No GUI dependency.
pub mod downloads;
pub mod installation;
mod instance_metadata;
pub mod mods;
pub mod pack_selection;
pub mod releases;
pub mod selection;
pub mod server_update;
use serde::{Deserialize, Serialize};
use std::{fs, io::Write, path::Path};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Configuration I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Invalid configuration JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Invalid configuration: {0}")]
    Validation(String),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Uk,
    En,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Amoled,
    Light,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Conflict {
    Ask,
    Replace,
    Keep,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub path: String,
    pub enabled: bool,
    pub conflict: Conflict,
    pub custom: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ModAction {
    Add,
    Remove,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModRule {
    pub action: ModAction,
    pub enabled: bool,
    pub name: String,
    pub mod_ids: Vec<String>,
    pub stem: String,
    #[serde(default)]
    pub file: Option<String>,
}

/// Renames a file of the new pack inside its own folder (`path` is relative to the game folder).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RenameRule {
    pub path: String,
    pub to: String,
    pub enabled: bool,
}

const CLIENT_PATHS: &[&str] = &[
    "config/vendingmachine/favourites",
    "config/shaders.properties",
    "ESM",
    "journeymap",
    "resourcepacks",
    "saves",
    "schematics",
    "screenshots",
    "shaderpacks",
    "TCNodeTracker",
    "visualprospecting",
    "BotaniaVars.dat",
    "localconfig.cfg",
    "options.txt",
    "optionsnf.txt",
    "servers.dat",
    "backups",
];

fn defaults(paths: &[&str]) -> Vec<Rule> {
    paths
        .iter()
        .map(|path| Rule {
            path: (*path).into(),
            enabled: *path != "backups",
            conflict: Conflict::Replace,
            custom: false,
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Settings {
    pub schema_version: u32,
    pub language: Language,
    pub theme: Theme,
    pub include_prereleases: bool,
    #[serde(default)]
    pub client_source: Option<String>,
    #[serde(default)]
    pub server_source: Option<String>,
    #[serde(default)]
    pub prism_instances_directory: Option<String>,
    pub client_rules: Vec<Rule>,
    #[serde(default)]
    pub server_rules: Vec<Rule>,
    #[serde(default)]
    pub client_mods: Vec<ModRule>,
    #[serde(default)]
    pub server_mods: Vec<ModRule>,
    #[serde(default)]
    pub client_renames: Vec<RenameRule>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: 1,
            language: Language::Uk,
            theme: Theme::Amoled,
            include_prereleases: false,
            client_source: None,
            server_source: None,
            prism_instances_directory: None,
            client_rules: defaults(CLIENT_PATHS),
            server_rules: Vec::new(),
            client_mods: Vec::new(),
            server_mods: Vec::new(),
            client_renames: Vec::new(),
        }
    }
}

/// Portable relative paths: use `/` as the separator on every host OS.
/// Filesystem containment/symlink checks must also happen at migration time.
pub fn validate_rule_path(path: &str) -> bool {
    !path.is_empty()
        && path.trim() == path
        && !path.contains(['\\', ':', '\0'])
        && !path.chars().any(char::is_control)
        && path.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && !part.ends_with(['.', ' '])
                && !part.contains(['<', '>', '"', '|', '?', '*'])
                && !is_windows_device(part)
        })
}
fn is_windows_device(part: &str) -> bool {
    let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
            })
        })
}
impl Settings {
    pub fn validate(&self) -> Result<(), Error> {
        if self.schema_version != 1 {
            return Err(Error::Validation("Unsupported schema version".into()));
        }
        let (rules, required) = (&self.client_rules, CLIENT_PATHS);
        let mut seen = std::collections::HashSet::new();
        for rule in rules {
            if !validate_rule_path(&rule.path) || !seen.insert(rule.path.to_lowercase()) {
                return Err(Error::Validation(format!(
                    "Invalid or duplicate relative path: {}",
                    rule.path
                )));
            }
            if !rule.custom
                && (!required.contains(&rule.path.as_str()) || rule.conflict != Conflict::Replace)
            {
                return Err(Error::Validation(
                    "Standard rule identity or conflict policy was changed".into(),
                ));
            }
        }
        if required
            .iter()
            .any(|path| !rules.iter().any(|r| !r.custom && r.path == *path))
        {
            return Err(Error::Validation(
                "Standard rules must be retained (disable instead of removing)".into(),
            ));
        }
        let mut seen = std::collections::HashSet::new();
        for rule in &self.server_rules {
            if !rule.custom
                || !validate_rule_path(&rule.path)
                || !seen.insert(rule.path.to_lowercase())
            {
                return Err(Error::Validation(format!(
                    "Invalid or duplicate server transfer path: {}",
                    rule.path
                )));
            }
        }
        validate_mods(&self.client_mods)?;
        validate_mods(&self.server_mods)?;
        let mut seen = std::collections::HashSet::new();
        for rule in &self.client_renames {
            if !valid_rename(rule) || !seen.insert(rule.path.to_lowercase()) {
                return Err(Error::Validation(format!(
                    "Invalid or duplicate rename rule: {}",
                    rule.path
                )));
            }
        }
        Ok(())
    }
}

/// A rename keeps the file in its folder: `to` is one valid component that differs from the old name.
pub fn valid_rename(rule: &RenameRule) -> bool {
    let name = rule.path.rsplit('/').next().unwrap_or_default();
    validate_rule_path(&rule.path)
        && validate_rule_path(&rule.to)
        && !rule.to.contains('/')
        && rule.to.to_lowercase() != name.to_lowercase()
}

fn validate_mods(rules: &[ModRule]) -> Result<(), Error> {
    let invalid = |message: &str| Err(Error::Validation(format!("Invalid mod rule: {message}")));
    let mut files = std::collections::HashSet::new();
    let mut removals = std::collections::HashSet::new();
    for rule in rules {
        if rule.name.is_empty()
            || rule.name.chars().count() > 200
            || rule.stem.is_empty()
            || rule.mod_ids.iter().any(String::is_empty)
        {
            return invalid("name, stem and mod ids must not be empty");
        }
        match (rule.action, &rule.file) {
            (ModAction::Add, Some(file)) if !file.is_empty() => {
                if !files.insert(file.to_lowercase()) {
                    return invalid("duplicate mod file");
                }
            }
            (ModAction::Add, _) => return invalid("an add rule needs a file"),
            (ModAction::Remove, None) => {
                let mut ids = rule.mod_ids.clone();
                ids.sort();
                if !removals.insert((ids, rule.stem.clone())) {
                    return invalid("duplicate removal");
                }
            }
            (ModAction::Remove, Some(_)) => return invalid("a remove rule has no file"),
        }
    }
    Ok(())
}

pub fn load_settings(directory: &Path) -> Result<Settings, Error> {
    match fs::read(directory.join("settings.json")) {
        Ok(bytes) => {
            let mut json: serde_json::Value = serde_json::from_slice(&bytes)?;
            // Compatibility only: discard the removed setting from older config files.
            if let Some(object) = json.as_object_mut() {
                object.remove("includeNightlies");
                // Old files stored a standard (non-custom) server rule; only custom ones remain.
                if let Some(serde_json::Value::Array(rules)) = object.get_mut("serverRules") {
                    rules.retain(|r| r.get("custom").and_then(|c| c.as_bool()) == Some(true));
                }
            }
            let settings: Settings = serde_json::from_value(json)?;
            settings.validate()?;
            Ok(settings)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
        Err(error) => Err(error.into()),
    }
}

pub fn save_settings(directory: &Path, settings: &Settings) -> Result<(), Error> {
    settings.validate()?;
    fs::create_dir_all(directory)?;
    // Refuse to silently replace a damaged or newer-schema configuration.
    load_settings(directory)?;
    let bytes = serde_json::to_vec_pretty(settings)?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    temporary.write_all(&bytes)?;
    temporary.write_all(b"\n")?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(directory.join("settings.json"))
        .map_err(|error| Error::Io(error.error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_are_valid_and_separate() {
        let s = Settings::default();
        s.validate().unwrap();
        assert!(!s.include_prereleases);
        assert!(
            !s.client_rules
                .iter()
                .find(|r| r.path == "backups")
                .unwrap()
                .enabled
        );
    }
    #[test]
    fn older_settings_gain_optional_fields() {
        let mut json = serde_json::to_value(Settings::default()).unwrap();
        let object = json.as_object_mut().unwrap();
        for key in ["clientSource", "serverSource", "prismInstancesDirectory"] {
            object.remove(key);
        }
        let settings: Settings = serde_json::from_value(json).unwrap();
        settings.validate().unwrap();
        assert!(settings.client_source.is_none());
        assert!(settings.server_source.is_none());
        assert!(settings.prism_instances_directory.is_none());
    }
    #[test]
    fn rename_rules_are_validated_and_optional_in_old_settings() {
        let rename = |path: &str, to: &str| RenameRule {
            path: path.into(),
            to: to.into(),
            enabled: true,
        };
        let with = |renames: Vec<RenameRule>| Settings {
            client_renames: renames,
            ..Settings::default()
        };
        with(vec![rename("config/a/ru_RU.lang", "en_US.lang")])
            .validate()
            .unwrap();
        assert!(with(vec![rename("config/a/ru_RU.lang", "b/en_US.lang")])
            .validate()
            .is_err());
        assert!(with(vec![rename("config/a/ru_RU.lang", "")])
            .validate()
            .is_err());
        assert!(with(vec![rename("config/a/ru_RU.lang", "RU_ru.LANG")])
            .validate()
            .is_err());
        assert!(with(vec![rename("../a.lang", "b.lang")])
            .validate()
            .is_err());
        assert!(with(vec![
            rename("config/a.lang", "b.lang"),
            rename("Config/A.lang", "c.lang")
        ])
        .validate()
        .is_err());
        let mut json = serde_json::to_value(Settings::default()).unwrap();
        json.as_object_mut().unwrap().remove("clientRenames");
        let settings: Settings = serde_json::from_value(json).unwrap();
        settings.validate().unwrap();
        assert!(settings.client_renames.is_empty());
    }
    #[test]
    fn prism_directory_roundtrips_and_can_be_cleared() {
        let dir = tempfile::tempdir().unwrap();
        let instances = dir.path().join("Prism instances");
        fs::create_dir(&instances).unwrap();
        let mut settings = Settings {
            prism_instances_directory: Some(instances.to_string_lossy().into_owned()),
            ..Settings::default()
        };
        save_settings(dir.path(), &settings).unwrap();
        assert_eq!(load_settings(dir.path()).unwrap(), settings);
        // A moved directory must not corrupt or silently erase preferences.
        fs::remove_dir(&instances).unwrap();
        assert_eq!(load_settings(dir.path()).unwrap(), settings);
        settings.prism_instances_directory = None;
        save_settings(dir.path(), &settings).unwrap();
        assert!(load_settings(dir.path())
            .unwrap()
            .prism_instances_directory
            .is_none());
    }
    #[test]
    fn removed_setting_is_read_but_not_written() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        let mut json = serde_json::to_value(Settings::default()).unwrap();
        json["includeNightlies"] = true.into();
        std::fs::write(&file, serde_json::to_vec(&json).unwrap()).unwrap();
        let settings = load_settings(dir.path()).unwrap();
        assert_eq!(settings, Settings::default());
        save_settings(dir.path(), &settings).unwrap();
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
        assert!(saved.get("includeNightlies").is_none());
    }
    #[test]
    fn standard_server_rule_is_dropped_and_custom_ones_kept() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        let mut json = serde_json::to_value(Settings::default()).unwrap();
        json["serverRules"] = serde_json::json!([{
            "path": "config/JourneyMapServer",
            "enabled": true,
            "conflict": "replace",
            "custom": false
        }]);
        fs::write(&file, serde_json::to_vec(&json).unwrap()).unwrap();
        let settings = load_settings(dir.path()).unwrap();
        assert_eq!(settings, Settings::default());
        save_settings(dir.path(), &settings).unwrap();
        let saved: serde_json::Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
        assert_eq!(saved["serverRules"], serde_json::json!([]));
        json["serverRules"] = serde_json::json!([
            {"path": "config/JourneyMapServer", "enabled": true, "conflict": "replace", "custom": false},
            {"path": "config/serverutilities.cfg", "enabled": true, "conflict": "ask", "custom": true}
        ]);
        fs::write(&file, serde_json::to_vec(&json).unwrap()).unwrap();
        let settings = load_settings(dir.path()).unwrap();
        assert_eq!(settings.server_rules.len(), 1);
        assert_eq!(settings.server_rules[0].path, "config/serverutilities.cfg");
    }
    #[test]
    fn server_rules_must_be_valid_custom_and_unique() {
        let rule = |path: &str, custom: bool| Rule {
            path: path.into(),
            enabled: true,
            conflict: Conflict::Ask,
            custom,
        };
        let with = |rules: Vec<Rule>| Settings {
            server_rules: rules,
            ..Settings::default()
        };
        with(vec![rule("config/a.cfg", true)]).validate().unwrap();
        assert!(with(vec![rule("config/a.cfg", false)]).validate().is_err());
        assert!(with(vec![rule("../a", true)]).validate().is_err());
        assert!(with(vec![rule("a/b", true), rule("A/B", true)])
            .validate()
            .is_err());
    }
    #[test]
    fn mod_rules_are_validated() {
        let add = |file: Option<&str>| ModRule {
            action: ModAction::Add,
            enabled: true,
            name: "Mod".into(),
            mod_ids: vec!["id".into()],
            stem: "mod".into(),
            file: file.map(Into::into),
        };
        let remove = |ids: &[&str]| ModRule {
            action: ModAction::Remove,
            file: None,
            mod_ids: ids.iter().map(|s| s.to_string()).collect(),
            ..add(None)
        };
        let with = |mods: Vec<ModRule>| Settings {
            client_mods: mods,
            ..Settings::default()
        };
        with(vec![add(Some("/m/a.jar")), remove(&["a", "b"])])
            .validate()
            .unwrap();
        assert!(with(vec![add(None)]).validate().is_err());
        assert!(with(vec![add(Some(""))]).validate().is_err());
        assert!(with(vec![add(Some("/m/a.jar")), add(Some("/M/A.jar"))])
            .validate()
            .is_err());
        assert!(with(vec![remove(&["a", "b"]), remove(&["b", "a"])])
            .validate()
            .is_err());
        let mut bad = remove(&["a"]);
        bad.file = Some("x.jar".into());
        assert!(with(vec![bad]).validate().is_err());
        let mut bad = remove(&[""]);
        assert!(with(vec![bad.clone()]).validate().is_err());
        bad.mod_ids = vec![];
        bad.name = "x".repeat(201);
        assert!(with(vec![bad.clone()]).validate().is_err());
        bad.name = String::new();
        assert!(with(vec![bad]).validate().is_err());
    }
    #[test]
    fn roundtrip_and_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_settings(dir.path()).unwrap(), Settings::default());
        let mut s = Settings::default();
        save_settings(dir.path(), &s).unwrap();
        s.include_prereleases = true;
        s.client_source = Some("/example/instance/.minecraft".into());
        s.language = Language::En;
        s.theme = Theme::Light;
        s.client_rules.push(Rule {
            path: "my-world".into(),
            enabled: true,
            custom: true,
            conflict: Conflict::Ask,
        });
        save_settings(dir.path(), &s).unwrap();
        assert_eq!(load_settings(dir.path()).unwrap(), s);
        assert!(load_settings(dir.path())
            .unwrap()
            .client_rules
            .iter()
            .any(|r| r.path == "my-world"));
    }
    #[test]
    fn corruption_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        fs::write(&file, b"broken").unwrap();
        assert!(save_settings(dir.path(), &Settings::default()).is_err());
        assert_eq!(fs::read(&file).unwrap(), b"broken");
    }
    #[test]
    fn unsupported_schema_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        let future = Settings {
            schema_version: 2,
            ..Settings::default()
        };
        let bytes = serde_json::to_vec(&future).unwrap();
        fs::write(&file, &bytes).unwrap();
        assert!(load_settings(dir.path()).is_err());
        assert!(save_settings(dir.path(), &Settings::default()).is_err());
        assert_eq!(fs::read(file).unwrap(), bytes);
    }
    #[test]
    fn io_failure_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let not_directory = dir.path().join("file");
        fs::write(&not_directory, b"unchanged").unwrap();
        assert!(save_settings(&not_directory, &Settings::default()).is_err());
        assert_eq!(fs::read(not_directory).unwrap(), b"unchanged");
    }
    #[test]
    fn reject_unsafe_portable_paths() {
        for path in [
            "",
            "/etc",
            "../mods",
            "foo/../bar",
            "C:/world",
            "a\\b",
            "a//b",
            "a/",
            "a/CON.txt",
            "a/LPT1",
            "foo.",
            "x\0y",
            "*.cfg",
        ] {
            assert!(!validate_rule_path(path), "{path}");
        }
        for path in ["config/test.cfg", "Мій світ", "saves/world-1"] {
            assert!(validate_rule_path(path));
        }
    }
    #[test]
    fn invalid_settings_preserve_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Settings::default();
        save_settings(dir.path(), &s).unwrap();
        let original = fs::read(dir.path().join("settings.json")).unwrap();
        s.client_rules[0].path = "../oops".into();
        assert!(save_settings(dir.path(), &s).is_err());
        assert_eq!(
            fs::read(dir.path().join("settings.json")).unwrap(),
            original
        );
    }
    #[test]
    fn reject_unknown_schema_and_duplicate_rules() {
        let mut s = Settings {
            schema_version: 2,
            ..Settings::default()
        };
        assert!(s.validate().is_err());
        s.schema_version = 1;
        s.client_rules.push(s.client_rules[0].clone());
        assert!(s.validate().is_err());
    }
}
