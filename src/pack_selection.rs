//! Input validation shared by native local-pack selection and installation.
use std::path::{Path, PathBuf};

pub fn validate(path: &Path) -> Result<PathBuf, String> {
    let canonical = path.canonicalize().map_err(|e| e.to_string())?;
    let zip = path
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("zip"));
    if canonical.is_dir() || (canonical.is_file() && zip) {
        Ok(canonical)
    } else {
        Err("Choose a ZIP archive or a pack folder".into())
    }
}
