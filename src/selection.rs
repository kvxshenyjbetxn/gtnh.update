use crate::{validate_rule_path, Error};
use std::path::{Component, Path};

/// Validate a remembered parent without treating it as an individual game instance.
pub fn instances_directory(path: &Path) -> Result<String, Error> {
    let root = path.canonicalize()?;
    if !root.is_dir() {
        return Err(Error::Validation(
            "Instances path must be a directory".into(),
        ));
    }
    // Opening the listing detects an inaccessible directory before a native dialog uses it.
    std::fs::read_dir(&root)?;
    Ok(root.to_string_lossy().into_owned())
}

pub fn source_directory(path: &Path, client: bool) -> Result<String, Error> {
    let mut root = path.canonicalize()?;
    if !root.is_dir() {
        return Err(Error::Validation("Source must be a directory".into()));
    }
    if client {
        // Match Prism's gameRoot(): minecraft takes precedence if both exist.
        for name in ["minecraft", ".minecraft"] {
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
    fn instances_directory_requires_readable_directory_and_keeps_instance_root() {
        let dir = tempfile::tempdir().unwrap();
        let instances = dir.path().join("Prism instances");
        std::fs::create_dir_all(instances.join("minecraft")).unwrap();
        assert_eq!(
            instances_directory(&instances).unwrap(),
            instances.canonicalize().unwrap().to_string_lossy()
        );
        assert!(instances_directory(&dir.path().join("missing")).is_err());
        std::fs::write(dir.path().join("file"), "test").unwrap();
        assert!(instances_directory(&dir.path().join("file")).is_err());
    }
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
    #[test]
    fn prism_source_prefers_minecraft_when_both_directories_exist() {
        let dir = tempfile::tempdir().unwrap();
        let instance = dir.path().join("instance");
        std::fs::create_dir_all(instance.join(".minecraft")).unwrap();
        std::fs::create_dir_all(instance.join("minecraft")).unwrap();
        std::fs::write(instance.join(".minecraft/options.txt"), "stale").unwrap();
        std::fs::write(instance.join("minecraft/options.txt"), "active").unwrap();
        let source = source_directory(&instance, true).unwrap();
        assert_eq!(
            std::fs::read_to_string(Path::new(&source).join("options.txt")).unwrap(),
            "active"
        );
        assert_eq!(
            source_directory(&instance.join(".minecraft"), true).unwrap(),
            instance
                .join(".minecraft")
                .canonicalize()
                .unwrap()
                .to_string_lossy()
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
