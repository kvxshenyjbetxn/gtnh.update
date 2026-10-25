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
fn client_metadata_preserves_playtime_and_sets_display_name_without_replacing_new_settings() {
    for game in [".minecraft", "minecraft"] {
        for zipped in [false, true] {
            for select_instance in [false, true] {
                for modern in [false, true] {
                    let (dir, _, zip, _) = setup();
                    let instance = dir.path().join("Old instance");
                    let source = instance.join(game);
                    fs::create_dir_all(&source).unwrap();
                    let old = "[General]\nConfigVersion=1.3\nname=Old\ntotalTimePlayed=12345\nlastTimePlayed=600\nlastLaunchTime=1700000000000\nJavaPath=old-java\n[Other]\ntotalTimePlayed=999\n";
                    fs::write(instance.join("instance.cfg"), old).unwrap();
                    fs::write(instance.join("mmc-pack.json"), "old components").unwrap();
                    let header = if modern {
                        "[General]\r\nConfigVersion=1.3\r\n"
                    } else {
                        ""
                    };
                    let other = if modern {
                        "[Other]\r\nname=Other name\r\n"
                    } else {
                        ""
                    };
                    let new = format!("{header}InstanceType=OneSix\r\nname=Pack default\r\ntotalTimePlayed=0\r\nJavaPath=new-java\r\nWrapperCommand=keep-me\r\n{other}");
                    let pack = dir.path().join("local-pack");
                    fs::create_dir_all(pack.join(game)).unwrap();
                    fs::write(pack.join("instance.cfg"), &new).unwrap();
                    fs::write(pack.join("mmc-pack.json"), "new components").unwrap();
                    archive(
                        &zip,
                        &[
                            ("Pack/instance.cfg", &new),
                            ("Pack/mmc-pack.json", "new components"),
                            (&format!("Pack/{game}/options.txt"), "default"),
                        ],
                    );
                    let selected = gtnh_core::selection::source_directory(
                        if select_instance { &instance } else { &source },
                        true,
                    )
                    .unwrap();
                    let target = dir.path().join("@GTNH, Нова #1");
                    prepare(
                        if zipped { &zip } else { &pack },
                        Path::new(&selected),
                        &target,
                        true,
                        &[],
                        |_| {},
                    )
                    .unwrap()
                    .finish(&[], |_| {})
                    .unwrap();
                    let output = fs::read_to_string(target.join("instance.cfg")).unwrap();
                    assert!(output.contains("totalTimePlayed=12345\r\n"), "{output}");
                    assert!(output.contains("lastTimePlayed=600\r\n"));
                    assert!(output.contains("lastLaunchTime=1700000000000\r\n"));
                    let name = if modern {
                        "name=\"@@GTNH, Нова #1\"\r\n"
                    } else {
                        "name=\"@GTNH, Нова \\#1\"\r\n"
                    };
                    assert!(output.contains(name), "{output}");
                    assert!(output.contains("JavaPath=new-java\r\nWrapperCommand=keep-me\r\n"));
                    if modern {
                        assert!(output.contains("[Other]\r\nname=Other name\r\n"));
                    }
                    assert_eq!(
                        fs::read_to_string(target.join("mmc-pack.json")).unwrap(),
                        "new components"
                    );
                    assert_eq!(
                        fs::read_to_string(instance.join("instance.cfg")).unwrap(),
                        old
                    );
                    assert_eq!(fs::read_to_string(pack.join("instance.cfg")).unwrap(), new);
                }
            }
        }
    }
}

#[test]
fn client_metadata_missing_invalid_and_unrelated_statistics_do_not_replace_pack_values() {
    for old in [
        None,
        Some(
            "name=Old\ntotalTimePlayed=42\nlastTimePlayed=-1\nlastLaunchTime=9223372036854775808\n",
        ),
        Some("[General]\ntotalTimePlayed=oops\n[Other]\ntotalTimePlayed=999\nlastTimePlayed=111\n"),
        Some("[General]\ntotalTimePlayed=1\ntotalTimePlayed=invalid\n"),
        Some("[General]\nlastTimePlayed=\"17\"\n"),
    ] {
        let (dir, _, zip, target) = setup();
        let instance = dir.path().join("Old instance");
        let source = instance.join(".minecraft");
        fs::create_dir_all(&source).unwrap();
        fs::write(instance.join("mmc-pack.json"), "{}").unwrap();
        if let Some(contents) = old {
            fs::write(instance.join("instance.cfg"), contents).unwrap();
        }
        archive(&zip, &[("instance.cfg", "[General]\nConfigVersion=1.3\nname=Default\ntotalTimePlayed=5\nlastTimePlayed=6\nlastLaunchTime=7\n"), ("mmc-pack.json", "{}")]);
        prepare(&zip, &source, &target, true, &[], |_| {})
            .unwrap()
            .finish(&[], |_| {})
            .unwrap();
        let output = fs::read_to_string(target.join("instance.cfg")).unwrap();
        assert!(output.contains("name=new\n"));
        let total = if old.is_some_and(|text| text.contains("totalTimePlayed=42")) {
            42
        } else {
            5
        };
        let last = if old.is_some_and(|text| text.contains("lastTimePlayed=\"17\"")) {
            17
        } else {
            6
        };
        assert!(
            output.contains(&format!("totalTimePlayed={total}\n")),
            "{output}"
        );
        assert!(
            output.contains(&format!("lastTimePlayed={last}\n")),
            "{output}"
        );
        assert!(output.contains("lastLaunchTime=7\n"));
    }
}

#[test]
fn client_display_names_are_encoded_for_qt_and_legacy_prism_without_adding_duplicate_keys() {
    for (modern, name, expected) in [
        (true, "GTNH Нова", "name=GTNH Нова"),
        (true, "GTNH; v=2, ready", "name=\"GTNH; v=2, ready\""),
        (true, "@GTNH #1", "name=@@GTNH #1"),
        (false, "GTNH #1", "name=GTNH \\#1"),
        (false, "GTNH; v=2, ready", "name=\"GTNH; v=2, ready\""),
    ] {
        let (dir, source, zip, _) = setup();
        let header = if modern {
            "[General]\nConfigVersion=1.3\n"
        } else {
            ""
        };
        let config = format!("{header}InstanceType=OneSix\nname=Default\nname=Duplicate");
        archive(&zip, &[("instance.cfg", &config), ("mmc-pack.json", "{}")]);
        let target = dir.path().join(name);
        prepare(&zip, &source, &target, true, &[], |_| {})
            .unwrap()
            .finish(&[], |_| {})
            .unwrap();
        let output = fs::read_to_string(target.join("instance.cfg")).unwrap();
        assert!(!output.contains("Default") && !output.contains("Duplicate"));
        assert!(
            output
                .lines()
                .filter(|line| line.starts_with("name="))
                .all(|line| line == expected),
            "{output}"
        );
        assert_eq!(
            output
                .lines()
                .filter(|line| line.starts_with("name="))
                .count(),
            2
        );
    }
}

#[test]
fn unversioned_grouped_destination_is_rejected_instead_of_publishing_a_wrong_prism_name() {
    let (_dir, source, zip, target) = setup();
    archive(
        &zip,
        &[
            (
                "instance.cfg",
                "[General]\nname=Default\n[Other]\nname=Wrong in legacy Prism\n",
            ),
            ("mmc-pack.json", "{}"),
        ],
    );
    let result = prepare(&zip, &source, &target, true, &[], |_| {});
    assert!(
        result.is_err(),
        "legacy Prism ignores groups and would keep the last name"
    );
    assert!(result.err().unwrap().contains("unversioned instance.cfg"));
    assert!(!target.exists());
}

#[test]
fn unversioned_grouped_source_does_not_import_ambiguous_statistics() {
    let (dir, _, zip, target) = setup();
    let instance = dir.path().join("old-instance");
    let source = instance.join(".minecraft");
    fs::create_dir_all(&source).unwrap();
    fs::write(instance.join("mmc-pack.json"), "{}").unwrap();
    fs::write(
        instance.join("instance.cfg"),
        "totalTimePlayed=42\n[Other]\ntotalTimePlayed=999\n",
    )
    .unwrap();
    archive(
        &zip,
        &[
            (
                "instance.cfg",
                "[General]\nConfigVersion=1.3\nname=Default\ntotalTimePlayed=5\n",
            ),
            ("mmc-pack.json", "{}"),
        ],
    );
    prepare(&zip, &source, &target, true, &[], |_| {})
        .unwrap()
        .finish(&[], |_| {})
        .unwrap();
    let output = fs::read_to_string(target.join("instance.cfg")).unwrap();
    assert!(
        output.contains("totalTimePlayed=5\n"),
        "ambiguous source must not import either value: {output}"
    );
}

#[test]
fn qt_general_section_aliases_preserve_playtime_name_and_unrelated_groups() {
    for section in ["[general]", "[ General ]"] {
        let (dir, _, zip, target) = setup();
        let instance = dir.path().join("old-instance");
        let source = instance.join(".minecraft");
        fs::create_dir_all(&source).unwrap();
        fs::write(instance.join("mmc-pack.json"), "{}").unwrap();
        fs::write(instance.join("instance.cfg"), format!("{section}\nConfigVersion=1.3\ntotalTimePlayed=42\n[%General]\ntotalTimePlayed=999\n")).unwrap();
        let config = format!(
            "{section}\nConfigVersion=1.3\nname=Default\n[%General]\nname=Real group untouched\n"
        );
        archive(&zip, &[("instance.cfg", &config), ("mmc-pack.json", "{}")]);
        prepare(&zip, &source, &target, true, &[], |_| {})
            .unwrap()
            .finish(&[], |_| {})
            .unwrap();
        assert_eq!(fs::read_to_string(target.join("instance.cfg")).unwrap(), format!("{section}\nConfigVersion=1.3\nname=new\ntotalTimePlayed=42\n[%General]\nname=Real group untouched\n"));
    }
}

#[test]
fn server_instance_cfg_is_not_modified() {
    let (_dir, source, zip, target) = setup();
    let config = "name=Server\ntotalTimePlayed=0\n";
    archive(
        &zip,
        &[
            ("mods/a.jar", "jar"),
            ("config/a.cfg", "default"),
            ("instance.cfg", config),
        ],
    );
    prepare(&zip, &source, &target, false, &[], |_| {})
        .unwrap()
        .finish(&[], |_| {})
        .unwrap();
    assert_eq!(
        fs::read_to_string(target.join("instance.cfg")).unwrap(),
        config
    );
}

#[cfg(unix)]
#[test]
fn client_metadata_does_not_follow_old_metadata_symlinks_or_unrecognized_parent() {
    for linked in [false, true] {
        let (dir, _, zip, target) = setup();
        let instance = dir.path().join("Old instance");
        let source = instance.join(".minecraft");
        fs::create_dir_all(&source).unwrap();
        let secret = dir.path().join("secret.cfg");
        fs::write(&secret, "totalTimePlayed=123456\n").unwrap();
        if linked {
            fs::write(instance.join("mmc-pack.json"), "{}").unwrap();
            std::os::unix::fs::symlink(&secret, instance.join("instance.cfg")).unwrap();
        } else {
            fs::write(instance.join("instance.cfg"), "totalTimePlayed=123456\n").unwrap();
        }
        archive(
            &zip,
            &[
                (
                    "instance.cfg",
                    "[General]\nConfigVersion=1.3\nname=Default\n",
                ),
                ("mmc-pack.json", "{}"),
            ],
        );
        prepare(&zip, &source, &target, true, &[], |_| {})
            .unwrap()
            .finish(&[], |_| {})
            .unwrap();
        let output = fs::read_to_string(target.join("instance.cfg")).unwrap();
        assert!(!output.contains("123456"));
        assert!(output.contains("name=new\n"));
        assert_eq!(
            fs::read_to_string(secret).unwrap(),
            "totalTimePlayed=123456\n"
        );
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
