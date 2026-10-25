use gtnh_core::pack_selection::{browse, validate};
use std::fs;

#[test]
fn unified_listing_includes_folders_and_zip_files_and_hides_other_files() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("pack")).unwrap();
    fs::write(dir.path().join("archive.ZIP"), "zip").unwrap();
    fs::write(dir.path().join("notes.txt"), "notes").unwrap();
    let listing = browse(dir.path()).unwrap();
    assert_eq!(
        listing
            .entries
            .iter()
            .map(|e| (e.name.as_str(), e.directory))
            .collect::<Vec<_>>(),
        [("pack", true), ("archive.ZIP", false)]
    );
    assert!(listing.parent.is_some());
    assert!(validate(&dir.path().join("pack")).unwrap().is_dir());
    assert!(validate(&dir.path().join("archive.ZIP")).unwrap().is_file());
    assert!(validate(&dir.path().join("notes.txt")).is_err());
    assert!(validate(&dir.path().join("missing.zip")).is_err());
    assert!(browse(&dir.path().join("archive.ZIP")).is_err());
}

#[cfg(unix)]
#[test]
fn picker_does_not_traverse_symlink_entries() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("pack")).unwrap();
    std::os::unix::fs::symlink(dir.path().join("pack"), dir.path().join("link")).unwrap();
    let listing = browse(dir.path()).unwrap();
    assert_eq!(listing.entries.len(), 1);
    assert_eq!(listing.entries[0].name, "pack");
}
