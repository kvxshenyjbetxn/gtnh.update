use gtnh_core::pack_selection::validate;
use std::fs;

#[test]
fn native_pack_inputs_are_validated_independently_of_dialog_filters() {
    let dir = tempfile::tempdir().unwrap();
    let pack = dir.path().join("pack");
    fs::create_dir(&pack).unwrap();
    let zip = dir.path().join("archive.ZIP");
    fs::write(&zip, "zip").unwrap();
    fs::write(dir.path().join("notes.txt"), "notes").unwrap();
    assert_eq!(validate(&pack).unwrap(), pack.canonicalize().unwrap());
    assert_eq!(validate(&zip).unwrap(), zip.canonicalize().unwrap());
    assert!(validate(&dir.path().join("notes.txt")).is_err());
    assert!(validate(&dir.path().join("missing.zip")).is_err());
}
