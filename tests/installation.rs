use gtnh_core::{
    installation::{prepare, Decision},
    Conflict, Rule,
};
use std::{fs, io::Write, path::Path};
use zip::{write::SimpleFileOptions, ZipWriter};

fn archive(path: &Path, files: &[(&str, &str)]) {
    let mut zip = ZipWriter::new(fs::File::create(path).unwrap());
    for (name, contents) in files {
        zip.start_file(
            *name,
            SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored)
                .unix_permissions(0o755),
        )
        .unwrap();
        zip.write_all(contents.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}
fn rule(path: &str, conflict: Conflict) -> Rule {
    Rule {
        path: path.into(),
        enabled: true,
        conflict,
        custom: true,
    }
}
fn setup() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    std::path::PathBuf,
    std::path::PathBuf,
) {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("old");
    fs::create_dir(&source).unwrap();
    let zip = dir.path().join("pack.zip");
    let target = dir.path().join("new");
    (dir, source, zip, target)
}

#[test]
fn selected_custom_folders_transfer_empty_directories_and_nested_contents_in_prism_layouts() {
    for game in [".minecraft", "minecraft"] {
        let (dir, _source, zip, target) = setup();
        let instance = dir.path().join("Old Prism instance");
        let source = instance.join(game);
        fs::create_dir_all(source.join("hui")).unwrap();
        fs::create_dir_all(source.join("custom folder/nested/empty")).unwrap();
        fs::write(source.join("custom folder/nested/data.txt"), "user data").unwrap();
        archive(
            &zip,
            &[
                ("Pack/instance.cfg", "[General]\nInstanceType=OneSix\n"),
                ("Pack/mmc-pack.json", "{}"),
                (&format!("Pack/{game}/config/new.cfg"), "new default"),
            ],
        );
        let selected = gtnh_core::selection::source_directory(&instance, true).unwrap();
        let prepared = prepare(
            &zip,
            Path::new(&selected),
            &target,
            true,
            &[
                rule("hui", Conflict::Replace),
                rule("custom folder", Conflict::Replace),
            ],
            |_| {},
        )
        .unwrap();
        assert!(prepared.preview().missing.is_empty());
        assert!(prepared
            .preview()
            .files
            .iter()
            .any(|file| file.path == "hui" && file.directory));
        prepared.finish(&[], |_| {}).unwrap();
        let result = target.join(game);
        assert!(result.join("hui").is_dir());
        assert!(result.join("custom folder/nested/empty").is_dir());
        assert_eq!(
            fs::read_to_string(result.join("custom folder/nested/data.txt")).unwrap(),
            "user data"
        );
        assert_eq!(
            fs::read_to_string(source.join("custom folder/nested/data.txt")).unwrap(),
            "user data"
        );
        assert_eq!(
            fs::read_to_string(result.join("config/new.cfg")).unwrap(),
            "new default"
        );
    }
}

#[test]
fn local_folder_is_staged_and_left_unchanged_after_transfer() {
    for client in [false, true] {
        let (dir, source, _zip, target) = setup();
        let pack = dir.path().join("downloaded-pack");
        let game = if client {
            pack.join(".minecraft")
        } else {
            pack.clone()
        };
        fs::create_dir_all(game.join("mods")).unwrap();
        fs::create_dir_all(game.join("config")).unwrap();
        fs::write(game.join("mods/new.jar"), "jar").unwrap();
        fs::write(game.join("options.txt"), "new").unwrap();
        fs::create_dir(game.join("empty")).unwrap();
        if client {
            fs::write(
                pack.join("instance.cfg"),
                "[General]\nInstanceType=OneSix\n",
            )
            .unwrap();
            fs::write(pack.join("mmc-pack.json"), "{}").unwrap();
        }
        fs::write(source.join("options.txt"), "mine").unwrap();
        prepare(
            &pack,
            &source,
            &target,
            client,
            &[rule("options.txt", Conflict::Replace)],
            |_| {},
        )
        .unwrap()
        .finish(&[], |_| {})
        .unwrap();
        let result_game = if client {
            target.join(".minecraft")
        } else {
            target.clone()
        };
        assert_eq!(
            fs::read_to_string(result_game.join("options.txt")).unwrap(),
            "mine"
        );
        assert_eq!(fs::read_to_string(game.join("options.txt")).unwrap(), "new");
        assert_eq!(
            fs::read_to_string(source.join("options.txt")).unwrap(),
            "mine"
        );
        assert!(result_game.join("mods/new.jar").is_file());
        assert!(result_game.join("empty").is_dir());
    }
}

#[test]
fn local_pack_cannot_contain_destination_or_be_overwritten() {
    let (dir, source, _zip, _target) = setup();
    let pack = dir.path().join("downloaded-pack");
    fs::create_dir_all(pack.join("mods")).unwrap();
    fs::create_dir(pack.join("config")).unwrap();
    assert!(prepare(&pack, &source, &pack.join("result"), false, &[], |_| {}).is_err());
    assert!(prepare(&pack, &source, &pack, false, &[], |_| {}).is_err());
    assert_eq!(fs::read_dir(&pack).unwrap().count(), 2);
}

#[cfg(unix)]
#[test]
fn local_folder_rejects_symlinks_without_following_or_altering_them() {
    let (dir, source, _zip, target) = setup();
    let pack = dir.path().join("downloaded-pack");
    fs::create_dir_all(pack.join("mods")).unwrap();
    fs::create_dir(pack.join("config")).unwrap();
    fs::write(dir.path().join("outside"), "secret").unwrap();
    std::os::unix::fs::symlink(dir.path().join("outside"), pack.join("mods/link.jar")).unwrap();
    assert!(prepare(&pack, &source, &target, false, &[], |_| {}).is_err());
    assert!(!target.exists());
    assert_eq!(
        fs::read_to_string(dir.path().join("outside")).unwrap(),
        "secret"
    );
    assert!(fs::symlink_metadata(pack.join("mods/link.jar"))
        .unwrap()
        .file_type()
        .is_symlink());
}

#[test]
fn server_merges_selected_files_and_keeps_new_defaults() {
    let (_dir, source, zip, target) = setup();
    archive(
        &zip,
        &[
            ("GTNH/config/new.cfg", "new"),
            ("GTNH/mods/mod.jar", "jar"),
            ("GTNH/config/user.cfg", "default"),
            ("GTNH/start.sh", "launch"),
        ],
    );
    fs::create_dir(source.join("config")).unwrap();
    fs::write(source.join("config/user.cfg"), "mine").unwrap();
    fs::write(source.join("unselected"), "skip").unwrap();
    let prepared = prepare(
        &zip,
        &source,
        &target,
        false,
        &[
            rule("config", Conflict::Replace),
            rule("absent", Conflict::Replace),
        ],
        |_| {},
    )
    .unwrap();
    assert!(!target.exists());
    assert_eq!(prepared.preview().missing, ["absent"]);
    let files: Vec<_> = prepared
        .preview()
        .files
        .iter()
        .filter(|f| !f.directory)
        .collect();
    assert_eq!(files.len(), 1);
    assert!(files[0].exists);
    prepared.finish(&[], |_| {}).unwrap();
    assert_eq!(
        fs::read_to_string(target.join("config/user.cfg")).unwrap(),
        "mine"
    );
    assert_eq!(
        fs::read_to_string(target.join("config/new.cfg")).unwrap(),
        "new"
    );
    assert!(!target.join("unselected").exists());
    assert_eq!(
        fs::read_to_string(source.join("config/user.cfg")).unwrap(),
        "mine"
    );
    assert!(zip.exists()); // Archive deletion belongs to the shell after success.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_ne!(
            fs::metadata(target.join("start.sh"))
                .unwrap()
                .permissions()
                .mode()
                & 0o111,
            0
        );
    }
}

#[test]
fn client_uses_instance_root_and_minecraft_directory() {
    for game in [".minecraft", "minecraft"] {
        let (_dir, source, zip, target) = setup();
        archive(
            &zip,
            &[
                ("Pack/instance.cfg", "[General]\nInstanceType=OneSix\n"),
                (
                    "Pack/mmc-pack.json",
                    "{\"formatVersion\":1,\"components\":[]}",
                ),
                (&format!("Pack/{game}/options.txt"), "new"),
            ],
        );
        fs::write(source.join("options.txt"), "mine").unwrap();
        prepare(
            &zip,
            &source,
            &target,
            true,
            &[rule("options.txt", Conflict::Replace)],
            |_| {},
        )
        .unwrap()
        .finish(&[], |_| {})
        .unwrap();
        assert!(target.join("mmc-pack.json").is_file());
        assert_eq!(
            fs::read_to_string(target.join(game).join("options.txt")).unwrap(),
            "mine"
        );
    }
}

#[test]
fn ask_requires_explicit_decision_and_keep_preserves_new() {
    let (_dir, source, zip, target) = setup();
    archive(&zip, &[("config/user.cfg", "new"), ("mods/mod.jar", "jar")]);
    fs::create_dir(source.join("config")).unwrap();
    fs::write(source.join("config/user.cfg"), "old").unwrap();
    let rules = [rule("config/user.cfg", Conflict::Ask)];
    let prepared = prepare(&zip, &source, &target, false, &rules, |_| {}).unwrap();
    assert!(prepared.finish(&[], |_| {}).is_err());
    assert!(!target.exists());
    prepare(&zip, &source, &target, false, &rules, |_| {})
        .unwrap()
        .finish(
            &[Decision {
                path: "config/user.cfg".into(),
                conflict: Conflict::Keep,
            }],
            |_| {},
        )
        .unwrap();
    assert_eq!(
        fs::read_to_string(target.join("config/user.cfg")).unwrap(),
        "new"
    );
}

#[test]
fn unsafe_archives_never_publish_or_modify_source() {
    for path in [
        "../escape",
        "/absolute",
        "C:/drive",
        "config/../escape",
        "mods/CON.txt",
        "a\\evil",
    ] {
        let (_dir, source, zip, target) = setup();
        archive(
            &zip,
            &[
                ("mods/a.jar", "jar"),
                ("config/a.cfg", "cfg"),
                (path, "bad"),
            ],
        );
        assert!(
            prepare(&zip, &source, &target, false, &[], |_| {}).is_err(),
            "{path}"
        );
        assert!(!target.exists());
        assert!(zip.exists());
    }
}

#[test]
fn case_aliases_and_file_directory_collisions_are_rejected() {
    for files in [
        vec![("mods/a.jar", "a"), ("Mods/b.jar", "b"), ("config/a", "c")],
        vec![("mods/a", "a"), ("mods/a/child", "b"), ("config/a", "c")],
    ] {
        let (_dir, source, zip, target) = setup();
        archive(&zip, &files);
        assert!(prepare(&zip, &source, &target, false, &[], |_| {}).is_err());
        assert!(!target.exists());
    }
}

#[test]
fn existing_nested_and_replaced_destinations_are_protected() {
    let (_dir, source, zip, target) = setup();
    archive(&zip, &[("mods/a.jar", "jar"), ("config/a.cfg", "cfg")]);
    assert!(prepare(&zip, &source, &source.join("new"), false, &[], |_| {}).is_err());
    fs::create_dir(&target).unwrap();
    fs::write(target.join("sentinel"), "safe").unwrap();
    assert!(prepare(&zip, &source, &target, false, &[], |_| {}).is_err());
    fs::remove_dir_all(&target).unwrap();
    let prepared = prepare(&zip, &source, &target, false, &[], |_| {}).unwrap();
    fs::create_dir(&target).unwrap();
    fs::write(target.join("sentinel"), "safe").unwrap();
    assert!(prepared.finish(&[], |_| {}).is_err());
    assert_eq!(fs::read_to_string(target.join("sentinel")).unwrap(), "safe");
}

#[test]
fn staged_client_is_not_visible_as_an_instance_before_confirmation() {
    let (dir, source, zip, target) = setup();
    archive(
        &zip,
        &[
            ("instance.cfg", "[General]\nInstanceType=OneSix\n"),
            ("mmc-pack.json", "{\"formatVersion\":1,\"components\":[]}"),
            (".minecraft/mods/a.jar", "jar"),
        ],
    );
    let prepared = prepare(&zip, &source, &target, true, &[], |_| {}).unwrap();
    for entry in fs::read_dir(dir.path()).unwrap() {
        assert!(!entry.unwrap().path().join("instance.cfg").exists());
    }
    drop(prepared);
}

#[test]
fn source_to_pack_type_conflicts_are_refused_without_mutating_source() {
    for source_is_directory in [false, true] {
        let (_dir, source, zip, target) = setup();
        archive(
            &zip,
            &[("mods/a.jar", "jar"), ("config/entry/child", "new")],
        );
        let relative = if source_is_directory {
            "mods/a.jar"
        } else {
            "config/entry"
        };
        let selected = source.join(relative);
        fs::create_dir_all(selected.parent().unwrap()).unwrap();
        if source_is_directory {
            fs::create_dir(&selected).unwrap();
            fs::write(selected.join("old"), "old").unwrap();
        } else {
            fs::write(&selected, "old").unwrap();
        }
        assert!(prepare(
            &zip,
            &source,
            &target,
            false,
            &[rule(relative, Conflict::Replace)],
            |_| {}
        )
        .is_err());
        assert!(!target.exists());
        let old_file = if source_is_directory {
            selected.join("old")
        } else {
            selected
        };
        assert_eq!(fs::read_to_string(old_file).unwrap(), "old");
    }
}

#[test]
fn an_empty_destination_created_after_preview_is_never_replaced() {
    let (_dir, source, zip, target) = setup();
    archive(&zip, &[("mods/a.jar", "jar"), ("config/a.cfg", "cfg")]);
    let prepared = prepare(&zip, &source, &target, false, &[], |_| {}).unwrap();
    fs::create_dir(&target).unwrap();
    assert!(prepared.finish(&[], |_| {}).is_err());
    assert!(target.is_dir());
    assert_eq!(fs::read_dir(target).unwrap().count(), 0);
}

#[test]
fn archive_symlinks_and_bad_crc_are_rejected() {
    let (_dir, source, zip, target) = setup();
    let mut writer = ZipWriter::new(fs::File::create(&zip).unwrap());
    writer
        .add_symlink("config/link", "../../outside", SimpleFileOptions::default())
        .unwrap();
    writer.finish().unwrap();
    assert!(prepare(&zip, &source, &target, false, &[], |_| {}).is_err());
    archive(
        &zip,
        &[
            ("mods/a.jar", "unique-original-payload"),
            ("config/a.cfg", "cfg"),
        ],
    );
    let mut bytes = fs::read(&zip).unwrap();
    // Our fixture uses ZIP's default Stored method, so the payload is literal.
    let start = bytes
        .windows(23)
        .position(|w| w == b"unique-original-payload");
    if let Some(index) = start {
        bytes[index] ^= 1;
        fs::write(&zip, bytes).unwrap();
    } else {
        panic!("Expected stored fixture payload");
    }
    assert!(prepare(&zip, &source, &target, false, &[], |_| {}).is_err());
    assert!(!target.exists());
}

#[test]
fn duplicate_exact_zip_names_are_rejected_before_extraction() {
    let (_dir, source, zip, target) = setup();
    archive(
        &zip,
        &[
            ("mods/a.jar", "first"),
            ("mods/b.jar", "second"),
            ("config/a.cfg", "cfg"),
        ],
    );
    let mut bytes = fs::read(&zip).unwrap();
    let name = b"mods/b.jar";
    for index in 0..=bytes.len() - name.len() {
        if &bytes[index..index + name.len()] == name {
            bytes[index + 5] = b'a';
        }
    }
    fs::write(&zip, bytes).unwrap();
    assert!(prepare(&zip, &source, &target, false, &[], |_| {}).is_err());
    assert!(!target.exists());
}

#[test]
fn disk_budget_excludes_kept_files_and_defers_ask_until_decided() {
    let (_dir, source, zip, target) = setup();
    archive(
        &zip,
        &[
            ("mods/a.jar", "jar"),
            ("config/keep", "new"),
            ("config/ask", "new"),
        ],
    );
    fs::create_dir(source.join("config")).unwrap();
    fs::write(source.join("config/keep"), "12345").unwrap();
    fs::write(source.join("config/ask"), "1234567").unwrap();
    fs::write(source.join("new-file"), "123").unwrap();
    let prepared = prepare(
        &zip,
        &source,
        &target,
        false,
        &[
            rule("config/keep", Conflict::Keep),
            rule("config/ask", Conflict::Ask),
            rule("new-file", Conflict::Keep),
        ],
        |_| {},
    )
    .unwrap();
    assert_eq!(prepared.preview().required_bytes(&[]).unwrap(), 3);
    assert_eq!(
        prepared
            .preview()
            .required_bytes(&[Decision {
                path: "config/ask".into(),
                conflict: Conflict::Replace
            }])
            .unwrap(),
        10
    );
    assert_eq!(
        prepared
            .preview()
            .required_bytes(&[Decision {
                path: "config/ask".into(),
                conflict: Conflict::Keep
            }])
            .unwrap(),
        3
    );
}

#[test]
fn most_specific_rule_controls_overlapping_file_conflicts() {
    let (_dir, source, zip, target) = setup();
    archive(
        &zip,
        &[
            ("mods/a.jar", "jar"),
            ("config/a.cfg", "new-a"),
            ("config/b.cfg", "new-b"),
        ],
    );
    fs::create_dir(source.join("config")).unwrap();
    fs::write(source.join("config/a.cfg"), "old-a").unwrap();
    fs::write(source.join("config/b.cfg"), "old-b").unwrap();
    prepare(
        &zip,
        &source,
        &target,
        false,
        &[
            rule("config/a.cfg", Conflict::Keep),
            rule("config", Conflict::Replace),
        ],
        |_| {},
    )
    .unwrap()
    .finish(&[], |_| {})
    .unwrap();
    assert_eq!(
        fs::read_to_string(target.join("config/a.cfg")).unwrap(),
        "new-a"
    );
    assert_eq!(
        fs::read_to_string(target.join("config/b.cfg")).unwrap(),
        "old-b"
    );
}

#[test]
fn dropping_preparation_cleans_staging_and_retains_archive() {
    let (dir, source, zip, target) = setup();
    archive(&zip, &[("mods/a.jar", "jar"), ("config/a.cfg", "cfg")]);
    let before = fs::read_dir(dir.path()).unwrap().count();
    drop(prepare(&zip, &source, &target, false, &[], |_| {}).unwrap());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), before);
    assert!(!target.exists());
    assert!(zip.exists());
}

#[cfg(unix)]
#[test]
fn source_symlinks_and_symlink_swap_after_preview_are_rejected() {
    use std::os::unix::fs::symlink;
    let (dir, source, zip, target) = setup();
    archive(&zip, &[("mods/a.jar", "jar"), ("config/a.cfg", "cfg")]);
    fs::write(dir.path().join("secret"), "secret").unwrap();
    symlink(dir.path().join("secret"), source.join("options.txt")).unwrap();
    let rules = [rule("options.txt", Conflict::Replace)];
    assert!(prepare(&zip, &source, &target, false, &rules, |_| {}).is_err());
    fs::remove_file(source.join("options.txt")).unwrap();
    fs::write(source.join("options.txt"), "mine").unwrap();
    let prepared = prepare(&zip, &source, &target, false, &rules, |_| {}).unwrap();
    fs::remove_file(source.join("options.txt")).unwrap();
    symlink(dir.path().join("secret"), source.join("options.txt")).unwrap();
    assert!(prepared.finish(&[], |_| {}).is_err());
    assert!(!target.exists());
}
