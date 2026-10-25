//! Platform-independent settings and migration rules. No GUI dependency.
pub mod downloads;
pub mod installation;
pub mod pack_selection;
pub mod releases;
pub mod selection;
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
const SERVER_PATHS: &[&str] = &["config/JourneyMapServer"];

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
    pub client_rules: Vec<Rule>,
    pub server_rules: Vec<Rule>,
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
            client_rules: defaults(CLIENT_PATHS),
            server_rules: defaults(SERVER_PATHS),
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
        for (rules, required) in [
            (&self.client_rules, CLIENT_PATHS),
            (&self.server_rules, SERVER_PATHS),
        ] {
            let mut seen = std::collections::HashSet::new();
            for rule in rules {
                if !validate_rule_path(&rule.path) || !seen.insert(rule.path.to_lowercase()) {
                    return Err(Error::Validation(format!(
                        "Invalid or duplicate relative path: {}",
                        rule.path
                    )));
                }
                if !rule.custom
                    && (!required.contains(&rule.path.as_str())
                        || rule.conflict != Conflict::Replace)
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
        }
        Ok(())
    }
}

pub fn load_settings(directory: &Path) -> Result<Settings, Error> {
    match fs::read(directory.join("settings.json")) {
        Ok(bytes) => {
            let mut json: serde_json::Value = serde_json::from_slice(&bytes)?;
            // Compatibility only: discard the removed setting from older config files.
            if let Some(object) = json.as_object_mut() {
                object.remove("includeNightlies");
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
        assert_eq!(s.server_rules[0].path, "config/JourneyMapServer");
    }
    #[test]
    fn older_settings_gain_optional_fields() {
        let mut json = serde_json::to_value(Settings::default()).unwrap();
        let object = json.as_object_mut().unwrap();
        for key in ["clientSource", "serverSource"] {
            object.remove(key);
        }
        let settings: Settings = serde_json::from_value(json).unwrap();
        settings.validate().unwrap();
        assert!(settings.client_source.is_none());
        assert!(settings.server_source.is_none());
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
    fn roundtrip_and_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_settings(dir.path()).unwrap(), Settings::default());
        let mut s = Settings::default();
        save_settings(dir.path(), &s).unwrap();
        s.include_prereleases = true;
        s.client_source = Some("/example/instance/.minecraft".into());
        s.language = Language::En;
        s.theme = Theme::Light;
        s.server_rules.push(Rule {
            path: "my-world".into(),
            enabled: true,
            custom: true,
            conflict: Conflict::Ask,
        });
        save_settings(dir.path(), &s).unwrap();
        assert_eq!(load_settings(dir.path()).unwrap(), s);
        assert!(!load_settings(dir.path())
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
