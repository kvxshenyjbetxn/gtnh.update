use crate::{validate_rule_path, Error};
use std::path::{Component, Path};

pub fn source_directory(path: &Path, client: bool) -> Result<String, Error> {
    let mut root = path.canonicalize()?;
    if !root.is_dir() {
        return Err(Error::Validation("Source must be a directory".into()));
    }
    if client {
        for name in [".minecraft", "minecraft"] {
            if root.join(name).is_dir() {
                root = root.join(name).canonicalize()?;
                break;
            }
        }
    }
    Ok(root.to_string_lossy().into_owned())
}

/// Canonicalize both sides so symlinks cannot select files outside the source.
pub fn relative_selections(root: &Path, selected: &[String]) -> Result<Vec<String>, Error> {
    let root = root.canonicalize()?;
    if !root.is_dir() {
        return Err(Error::Validation("Source must be a directory".into()));
    }
    selected
        .iter()
        .map(|path| {
            let canonical = Path::new(path).canonicalize()?;
            let relative = canonical.strip_prefix(&root).map_err(|_| {
                Error::Validation("Selection is outside the source directory".into())
            })?;
            let parts: Vec<_> = relative
                .components()
                .map(|part| match part {
                    Component::Normal(value) => value
                        .to_str()
                        .ok_or_else(|| Error::Validation("Non-Unicode path".into())),
                    _ => Err(Error::Validation("Invalid relative path".into())),
                })
                .collect::<Result<_, _>>()?;
            let portable = parts.join("/");
            if !validate_rule_path(&portable) {
                return Err(Error::Validation("Unsupported relative path".into()));
            }
            Ok(portable)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_is_relative_and_contained() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("instance/.minecraft");
        std::fs::create_dir_all(root.join("config")).unwrap();
        std::fs::write(root.join("config/test.cfg"), "test").unwrap();
        assert_eq!(
            relative_selections(
                &root,
                &[root.join("config/test.cfg").to_string_lossy().into_owned()]
            )
            .unwrap(),
            ["config/test.cfg"]
        );
        assert!(relative_selections(&root, &[dir.path().to_string_lossy().into_owned()]).is_err());
        assert!(relative_selections(&root, &[root.to_string_lossy().into_owned()]).is_err());
        assert_eq!(
            source_directory(&dir.path().join("instance"), true).unwrap(),
            root.canonicalize().unwrap().to_string_lossy()
        );
    }
    #[cfg(unix)]
    #[test]
    fn reject_outside_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("source");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(dir.path().join("secret"), "test").unwrap();
        std::os::unix::fs::symlink(dir.path().join("secret"), root.join("link")).unwrap();
        assert!(
            relative_selections(&root, &[root.join("link").to_string_lossy().into_owned()])
                .is_err()
        );
    }
}
