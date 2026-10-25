//! Narrow edits to Prism metadata in the staged copy, never the source instance.
use std::{collections::BTreeMap, fs, path::Path};

const STATISTICS: [&str; 3] = ["totalTimePlayed", "lastTimePlayed", "lastLaunchTime"];

fn regular_file(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_file())
}

// Qt stores root settings in [General]; older Prism files have no section at all.
// Do not confuse [%General] (an actual group named General) with the root group.
fn section(line: &str) -> Option<&str> {
    let line = line.trim().trim_start_matches('\u{feff}');
    line.strip_prefix('[')?
        .split_once(']')
        .map(|(name, _)| name.trim())
}

fn entry(line: &str) -> Option<(&str, &str)> {
    let line = line.trim().trim_start_matches('\u{feff}');
    if line.starts_with(['#', ';']) {
        return None;
    }
    let (key, value) = line.split_once('=')?;
    Some((key.trim(), value.trim()))
}

fn root_entries(text: &str) -> impl Iterator<Item = (&str, &str)> {
    let mut root = true;
    text.lines().filter_map(move |line| {
        if let Some(name) = section(line) {
            root = name.eq_ignore_ascii_case("General");
            return None;
        }
        root.then(|| entry(line)).flatten()
    })
}

fn is_modern(text: &str) -> bool {
    root_entries(text).any(|(key, _)| key == "ConfigVersion")
}

fn has_named_groups(text: &str) -> bool {
    text.lines()
        .filter_map(section)
        .any(|name| !name.eq_ignore_ascii_case("General"))
}

fn old_statistics(source_game: &Path) -> BTreeMap<&'static str, String> {
    let mut values = BTreeMap::new();
    if !matches!(
        source_game.file_name().and_then(|s| s.to_str()),
        Some(".minecraft" | "minecraft")
    ) {
        return values;
    }
    let Some(instance) = source_game.parent() else {
        return values;
    };
    let path = instance.join("instance.cfg");
    if !regular_file(&path) || !regular_file(&instance.join("mmc-pack.json")) {
        return values;
    }
    // Metadata is optional. Do not load arbitrary large or unreadable source files.
    if !fs::metadata(&path).is_ok_and(|metadata| metadata.len() <= 1024 * 1024) {
        return values;
    }
    let Ok(text) = fs::read_to_string(path) else {
        return values;
    };
    // Without ConfigVersion Prism ignores group headers. Hybrid files therefore
    // have ambiguous semantics; do not import a statistic from the wrong group.
    if !is_modern(&text) && has_named_groups(&text) {
        return values;
    }
    for (key, raw) in root_entries(&text) {
        if let Some(key) = STATISTICS
            .iter()
            .copied()
            .find(|candidate| *candidate == key)
        {
            // QSettings may quote scalar numbers. Never import a list, a variant,
            // a negative duration or a value overflowing Prism's signed 64-bit type.
            let raw = raw
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .unwrap_or(raw);
            if let Some(number) = raw.parse::<i64>().ok().filter(|number| *number >= 0) {
                values.insert(key, number.to_string());
            } else {
                values.remove(key); // The last occurrence wins, including invalid values.
            }
        }
    }
    values
}

// Match the relevant string rules of Qt QSettings and Prism's legacy INI reader.
// Keep the original format instead of migrating unrelated launch settings.
fn encode_name(name: &str, modern: bool) -> String {
    let mut value = String::new();
    if modern && name.starts_with('@') {
        value.push('@');
    }
    for ch in name.chars() {
        match ch {
            '\\' => value.push_str("\\\\"),
            '"' => value.push_str("\\\""),
            '\n' => value.push_str("\\n"),
            '\r' => value.push_str("\\r"),
            '\t' => value.push_str("\\t"),
            '#' if !modern => value.push_str("\\#"),
            ch => value.push(ch),
        }
    }
    if name.contains([';', '=', ',']) || name.starts_with(' ') || name.ends_with(' ') {
        format!("\"{value}\"")
    } else {
        value
    }
}

fn patch(text: &str, updates: &BTreeMap<&str, String>) -> String {
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lines: Vec<String> = text.split_inclusive('\n').map(str::to_owned).collect();
    let mut root = true;
    let mut found = BTreeMap::new();
    // Missing root keys go before the first named group, or at the end of the
    // first explicit [General] group when present. Preserve unrelated lines.
    let mut insertion = lines.len();
    let mut saw_section = false;
    let mut general = false;
    for (index, line) in lines.iter_mut().enumerate() {
        if let Some(name) = section(line) {
            if !saw_section {
                insertion = index;
                saw_section = true;
            }
            if general {
                insertion = index;
                general = false;
            }
            root = name.eq_ignore_ascii_case("General");
            if root {
                general = true;
                insertion = index + 1;
            }
            continue;
        }
        if general {
            insertion = index + 1;
        }
        if root {
            if let Some((key, _)) = entry(line) {
                if let Some(value) = updates.get(key) {
                    let key = key.to_owned();
                    let ending = if line.ends_with("\r\n") {
                        "\r\n"
                    } else if line.ends_with('\n') {
                        "\n"
                    } else {
                        ""
                    };
                    let bom = if line.starts_with('\u{feff}') {
                        "\u{feff}"
                    } else {
                        ""
                    };
                    *line = format!("{bom}{key}={value}{ending}");
                    found.insert(key, ());
                }
            }
        }
    }
    let missing: Vec<_> = updates
        .iter()
        .filter(|(key, _)| !found.contains_key(**key))
        .map(|(key, value)| format!("{key}={value}{eol}"))
        .collect();
    if !missing.is_empty() {
        if insertion > 0 && !lines[insertion - 1].ends_with('\n') {
            lines[insertion - 1].push_str(eol);
        }
        lines.splice(insertion..insertion, missing);
    }
    lines.concat()
}

pub(crate) fn update_client_metadata(
    source_game: &Path,
    staged_instance: &Path,
    name: &str,
) -> Result<(), String> {
    let path = staged_instance.join("instance.cfg");
    if !regular_file(&path) {
        return Err("Unsafe or missing staged instance.cfg".into());
    }
    let text = fs::read_to_string(&path).map_err(|e| format!("Cannot read instance.cfg: {e}"))?;
    let modern = is_modern(&text);
    if !modern && has_named_groups(&text) {
        return Err("Unsupported grouped, unversioned instance.cfg: open and save the pack in Prism before installing".into());
    }
    let mut updates = old_statistics(source_game);
    updates.insert("name", encode_name(name, modern));
    fs::write(path, patch(&text, &updates)).map_err(|e| format!("Cannot update instance.cfg: {e}"))
}
