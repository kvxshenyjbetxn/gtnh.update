use gtnh_core::{
    installation::Decision,
    server_update::{cleanup_leftovers, prepare, recover, Fault, JavaChange, Recovery},
    Conflict, ModAction, ModRule, Rule,
};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};
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
fn write(root: &Path, files: &[(&str, &str)]) {
    for (name, contents) in files {
        let path = root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
}
// Recursive listing with contents; directories map to None.
fn snapshot(root: &Path) -> BTreeMap<String, Option<Vec<u8>>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Option<Vec<u8>>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let relative = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if path.is_dir() {
                out.insert(relative, None);
                walk(root, &path, out);
            } else {
                out.insert(relative, Some(fs::read(&path).unwrap()));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}
fn staging_left(parent: &Path) -> Vec<String> {
    fs::read_dir(parent)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(".gtnh-update-"))
        .collect()
}

const OLD_SERVER: &[(&str, &str)] = &[
    ("world/level.dat", "world data"),
    ("server.properties", "motd=hello"),
    ("eula.txt", "eula=true"),
    ("whitelist.json", "[]"),
    ("serverutilities/x.cfg", "su"),
    ("custom/file", "custom"),
    ("mods/old.jar", "old mod"),
    ("config/a.cfg", "old config"),
    ("config/JourneyMapServer/world.uuid", "uuid"),
    ("libraries/old.jar", "old lib"),
    ("startserver.sh", "-Xmx6G old"),
    ("forge.jar", "forge"),
];
const NEW_PACK: &[(&str, &str)] = &[
    ("GTNH/mods/new.jar", "new mod"),
    ("GTNH/config/a.cfg", "new config"),
    ("GTNH/libraries/new.jar", "new lib"),
    ("GTNH/startserver.sh", "-Xmx8G new"),
    ("GTNH/java9args.txt", "--add-opens"),
    ("GTNH/newdir/x", "x"),
];

struct Setup {
    _dir: tempfile::TempDir,
    parent: PathBuf,
    server: PathBuf,
    zip: PathBuf,
    backup: PathBuf,
}
fn setup(server_files: &[(&str, &str)], pack_files: &[(&str, &str)]) -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let parent = dir.path().canonicalize().unwrap();
    let server = parent.join("server");
    fs::create_dir(&server).unwrap();
    write(&server, server_files);
    let zip = parent.join("pack.zip");
    archive(&zip, pack_files);
    let backup = parent.join("server.gtnh-backup");
    Setup {
        _dir: dir,
        parent,
        server,
        zip,
        backup,
    }
}

#[test]
fn happy_path_replaces_only_managed_items() {
    let s = setup(OLD_SERVER, NEW_PACK);
    let prepared = prepare(&s.zip, &s.server, &[], &[], |_| {}).unwrap();
    let preview = prepared.preview().clone();
    assert_eq!(preview.java_change, Some(JavaChange::ToJava17));
    assert!(preview.journey_map);
    assert!(!preview.previous_backup);
    assert_eq!(preview.added, vec!["newdir".to_string()]);
    assert!(preview.recovered.is_none());
    let script = preview
        .scripts
        .iter()
        .find(|c| c.path == "startserver.sh")
        .unwrap();
    assert_eq!(script.old.as_deref(), Some("-Xmx6G old"));
    assert_eq!(script.new.as_deref(), Some("-Xmx8G new"));
    assert!(preview.replaced.iter().all(|i| i.path != "forge.jar"));
    // Preparing never touches the server.
    assert_eq!(
        fs::read_to_string(s.server.join("mods/old.jar")).unwrap(),
        "old mod"
    );
    assert_eq!(prepared.finish(&[], |_| {}).unwrap(), s.server);

    for (name, contents) in OLD_SERVER.iter().filter(|(n, _)| {
        [
            "world/",
            "server.properties",
            "eula",
            "whitelist",
            "serverutilities/",
            "custom/",
            "forge.jar",
        ]
        .iter()
        .any(|p| n.starts_with(p))
    }) {
        assert_eq!(fs::read_to_string(s.server.join(name)).unwrap(), *contents);
    }
    assert!(!s.server.join("mods/old.jar").exists());
    assert_eq!(
        fs::read_to_string(s.server.join("mods/new.jar")).unwrap(),
        "new mod"
    );
    assert_eq!(
        fs::read_to_string(s.server.join("config/a.cfg")).unwrap(),
        "new config"
    );
    assert_eq!(
        fs::read_to_string(s.server.join("config/JourneyMapServer/world.uuid")).unwrap(),
        "uuid"
    );
    assert!(!s.server.join("libraries/old.jar").exists());
    assert_eq!(
        fs::read_to_string(s.server.join("startserver.sh")).unwrap(),
        "-Xmx8G new"
    );
    assert!(s.server.join("java9args.txt").is_file());
    assert!(s.server.join("newdir/x").is_file());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(s.server.join("startserver.sh"))
            .unwrap()
            .permissions()
            .mode();
        assert_ne!(mode & 0o111, 0);
    }

    for (name, contents) in [
        ("mods/old.jar", "old mod"),
        ("config/a.cfg", "old config"),
        ("config/JourneyMapServer/world.uuid", "uuid"),
        ("libraries/old.jar", "old lib"),
        ("startserver.sh", "-Xmx6G old"),
    ] {
        assert_eq!(fs::read_to_string(s.backup.join(name)).unwrap(), contents);
    }
    assert!(s.backup.join("gtnh-backup.json").is_file());
    assert!(!s.server.join(".gtnh-update-journal.json").exists());
    assert!(staging_left(&s.parent).is_empty());
}

#[test]
fn second_update_replaces_previous_backup() {
    let s = setup(OLD_SERVER, NEW_PACK);
    prepare(&s.zip, &s.server, &[], &[], |_| {})
        .unwrap()
        .finish(&[], |_| {})
        .unwrap();
    archive(
        &s.zip,
        &[
            ("GTNH/mods/newer.jar", "newer"),
            ("GTNH/config/a.cfg", "newer config"),
        ],
    );
    let prepared = prepare(&s.zip, &s.server, &[], &[], |_| {}).unwrap();
    assert!(prepared.preview().previous_backup);
    prepared.finish(&[], |_| {}).unwrap();
    assert!(s.backup.join("mods/new.jar").is_file());
    assert!(!s.backup.join("mods/old.jar").exists());
    assert!(s.server.join("mods/newer.jar").is_file());
    let backups: Vec<_> = fs::read_dir(&s.parent)
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".gtnh-backup")
        })
        .collect();
    assert_eq!(backups.len(), 1);
    assert!(staging_left(&s.parent).is_empty());
}

#[test]
fn injected_failure_rolls_everything_back() {
    let s = setup(OLD_SERVER, NEW_PACK);
    let before = snapshot(&s.server);
    let prepared = prepare(&s.zip, &s.server, &[], &[], |_| {}).unwrap();
    let error = prepared
        .finish_with_fault(&[], Fault::FailAfterMoves(3), |_| {})
        .unwrap_err();
    assert!(error.contains("rolled back"), "{error}");
    assert_eq!(snapshot(&s.server), before);
    assert!(!s.backup.exists());
    assert!(staging_left(&s.parent).is_empty());
}

#[test]
fn crash_is_recovered_by_recover_and_by_next_prepare() {
    for use_prepare in [false, true] {
        let s = setup(OLD_SERVER, NEW_PACK);
        let before = snapshot(&s.server);
        let prepared = prepare(&s.zip, &s.server, &[], &[], |_| {}).unwrap();
        assert!(prepared
            .finish_with_fault(&[], Fault::CrashAfterMoves(3), |_| {})
            .is_err());
        assert!(s.server.join(".gtnh-update-journal.json").exists());
        assert_eq!(staging_left(&s.parent).len(), 1);
        if use_prepare {
            let again = prepare(&s.zip, &s.server, &[], &[], |_| {}).unwrap();
            assert_eq!(again.preview().recovered, Some(Recovery::RolledBack));
            drop(again);
        } else {
            assert_eq!(recover(&s.server).unwrap(), Some(Recovery::RolledBack));
            assert_eq!(recover(&s.server).unwrap(), None);
        }
        assert_eq!(snapshot(&s.server), before);
        assert!(!s.backup.exists());
        assert!(staging_left(&s.parent).is_empty());
    }
}

#[test]
fn crash_after_commit_is_completed() {
    let s = setup(OLD_SERVER, NEW_PACK);
    let prepared = prepare(&s.zip, &s.server, &[], &[], |_| {}).unwrap();
    assert!(prepared
        .finish_with_fault(&[], Fault::CrashAfterCommit, |_| {})
        .is_err());
    assert_eq!(recover(&s.server).unwrap(), Some(Recovery::Completed));
    assert!(s.backup.join("mods/old.jar").is_file());
    assert!(s.backup.join("gtnh-backup.json").is_file());
    assert!(s.server.join("mods/new.jar").is_file());
    assert!(!s.server.join(".gtnh-update-journal.json").exists());
    assert!(staging_left(&s.parent).is_empty());
}

#[test]
fn java_17_to_8_moves_leftovers_to_backup() {
    let mut server = OLD_SERVER.to_vec();
    server.push(("java9args.txt", "--add-opens"));
    server.push(("lwjgl3ify-forgePatches.jar", "patches"));
    let s = setup(
        &server,
        &[
            ("GTNH/mods/new.jar", "new mod"),
            ("GTNH/config/a.cfg", "new config"),
        ],
    );
    let prepared = prepare(&s.zip, &s.server, &[], &[], |_| {}).unwrap();
    let preview = prepared.preview().clone();
    assert_eq!(preview.java_change, Some(JavaChange::ToJava8));
    let args = preview
        .scripts
        .iter()
        .find(|c| c.path == "java9args.txt")
        .unwrap();
    assert_eq!(args.old.as_deref(), Some("--add-opens"));
    assert!(args.new.is_none());
    prepared.finish(&[], |_| {}).unwrap();
    assert!(!s.server.join("java9args.txt").exists());
    assert!(!s.server.join("lwjgl3ify-forgePatches.jar").exists());
    assert!(s.backup.join("java9args.txt").is_file());
    assert!(s.backup.join("lwjgl3ify-forgePatches.jar").is_file());
    assert!(s.server.join("forge.jar").is_file());
}

#[test]
fn invalid_server_and_nested_pack_are_rejected_and_drop_cleans_up() {
    let s = setup(&[("world/level.dat", "x")], NEW_PACK);
    let error = prepare(&s.zip, &s.server, &[], &[], |_| {}).err().unwrap();
    assert!(error.contains("not a GTNH server"), "{error}");

    let s = setup(OLD_SERVER, NEW_PACK);
    let pack = s.server.join("pack");
    write(&pack, &[("mods/new.jar", "n"), ("config/a.cfg", "n")]);
    assert!(prepare(&pack, &s.server, &[], &[], |_| {}).is_err());
    let inside = s.parent.join("outer");
    write(&inside, &[("mods/m.jar", "m"), ("config/c.cfg", "c")]);
    assert!(prepare(&s.parent, &inside, &[], &[], |_| {}).is_err());

    let before = snapshot(&s.server);
    let prepared = prepare(&s.zip, &s.server, &[], &[], |_| {}).unwrap();
    assert_eq!(staging_left(&s.parent).len(), 1);
    drop(prepared);
    assert_eq!(snapshot(&s.server), before);
    assert!(staging_left(&s.parent).is_empty());
}

#[test]
fn matching_is_case_insensitive() {
    let s = setup(
        &[("Mods/old.jar", "old"), ("Config/a.cfg", "old")],
        &[("GTNH/mods/new.jar", "new"), ("GTNH/config/a.cfg", "new")],
    );
    let prepared = prepare(&s.zip, &s.server, &[], &[], |_| {}).unwrap();
    assert!(prepared
        .preview()
        .replaced
        .iter()
        .any(|i| i.path == "mods" && i.old && i.new));
    prepared.finish(&[], |_| {}).unwrap();
    assert!(s.backup.join("Mods/old.jar").is_file());
    assert!(s.server.join("mods/new.jar").is_file());
    assert!(!s.server.join("mods/old.jar").exists());
    let names: Vec<_> = fs::read_dir(&s.server)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(names.contains(&"mods".to_string()) && !names.contains(&"Mods".to_string()));
}

#[test]
fn tampered_journal_is_rejected() {
    let s = setup(OLD_SERVER, NEW_PACK);
    fs::write(
        s.server.join(".gtnh-update-journal.json"),
        format!(
            r#"{{"version":1,"staging":{:?},"payloadRoot":"x","backup":"x","finalBackup":{:?},"items":[],"committed":true}}"#,
            s.parent.join("elsewhere").to_string_lossy(),
            s.parent.join("server.gtnh-backup").to_string_lossy()
        ),
    )
    .unwrap();
    assert!(recover(&s.server).is_err());
    assert!(s.server.join("mods/old.jar").is_file());
}

#[test]
fn cleanup_leftovers_removes_only_stale_staging_without_journal() {
    let s = setup(OLD_SERVER, NEW_PACK);
    let stale = s.parent.join(".gtnh-update-abc123");
    write(&stale, &[("payload/mods/x.jar", "x"), ("backup/a", "a")]);
    let long = s.parent.join(".gtnh-update-toolong1");
    fs::create_dir(&long).unwrap();
    let file = s.parent.join(".gtnh-update-abc124");
    fs::write(&file, "file").unwrap();
    assert_eq!(cleanup_leftovers(&s.server).unwrap(), 1);
    assert!(!stale.exists());
    assert!(long.is_dir());
    assert!(file.is_file());
    assert_eq!(cleanup_leftovers(&s.server).unwrap(), 0);
}

#[test]
fn cleanup_leftovers_keeps_staging_of_interrupted_update() {
    let s = setup(OLD_SERVER, NEW_PACK);
    let before = snapshot(&s.server);
    let prepared = prepare(&s.zip, &s.server, &[], &[], |_| {}).unwrap();
    assert!(prepared
        .finish_with_fault(&[], Fault::CrashAfterMoves(3), |_| {})
        .is_err());
    assert_eq!(cleanup_leftovers(&s.server).unwrap(), 0);
    assert_eq!(staging_left(&s.parent).len(), 1);
    assert_eq!(recover(&s.server).unwrap(), Some(Recovery::RolledBack));
    assert_eq!(snapshot(&s.server), before);
    assert!(staging_left(&s.parent).is_empty());
}

fn jar(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut zip = ZipWriter::new(fs::File::create(path).unwrap());
    zip.start_file("mcmod.info", SimpleFileOptions::default())
        .unwrap();
    zip.write_all(br#"[{"modid":"extra"}]"#).unwrap();
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
fn mod_rule(action: ModAction, name: &str, stem: &str, file: Option<&Path>) -> ModRule {
    ModRule {
        action,
        enabled: true,
        name: name.into(),
        mod_ids: vec![],
        stem: stem.into(),
        file: file.map(|f| f.to_string_lossy().into_owned()),
    }
}
// Server with custom config files and a folder pack that also carries a mod to remove.
fn rules_setup() -> (Setup, PathBuf, PathBuf) {
    let mut server = OLD_SERVER.to_vec();
    server.push(("config/custom.cfg", "old custom"));
    server.push(("config/ask.cfg", "old ask"));
    let s = setup(&server, &[]);
    let pack = s.parent.join("pack");
    write(
        &pack,
        &[
            ("mods/new.jar", "new mod"),
            ("mods/passivemobs-1.3.0.jar", "passive"),
            ("config/a.cfg", "new config"),
            ("config/custom.cfg", "new custom"),
            ("config/ask.cfg", "new ask"),
        ],
    );
    let extra = s.parent.join("elsewhere/extra-1.0.jar");
    jar(&extra);
    (s, pack, extra)
}

#[test]
fn server_mod_and_transfer_rules_apply_with_decisions() {
    let (s, pack, extra) = rules_setup();
    let rules = [
        rule("config/custom.cfg", Conflict::Replace),
        rule("config/ask.cfg", Conflict::Ask),
        rule("config/missing.cfg", Conflict::Ask),
    ];
    let mods = [
        mod_rule(ModAction::Remove, "Passive", "passivemobs", None),
        mod_rule(ModAction::Add, "Extra", "extra", Some(&extra)),
    ];
    let before = snapshot(&s.server);

    let prepared = prepare(&pack, &s.server, &rules, &mods, |_| {}).unwrap();
    let preview = prepared.preview().clone();
    assert_eq!(preview.missing, ["config/missing.cfg"]);
    assert_eq!(preview.files.len(), 2);
    assert!(preview.files.iter().all(|f| f.exists));
    assert_eq!(
        preview.mods.removed[0].files,
        ["mods/passivemobs-1.3.0.jar"]
    );
    assert_eq!(preview.mods.added[0].file, "mods/extra-1.0.jar");
    // An unanswered Ask conflict fails before the server is touched.
    let error = prepared.finish(&[], |_| {}).unwrap_err();
    assert!(error.contains("Choose replace or keep"), "{error}");
    assert_eq!(snapshot(&s.server), before);
    assert!(staging_left(&s.parent).is_empty());

    for (conflict, expected) in [(Conflict::Keep, "new ask"), (Conflict::Replace, "old ask")] {
        let (s, pack, extra) = rules_setup();
        let mods = [
            mod_rule(ModAction::Remove, "Passive", "passivemobs", None),
            mod_rule(ModAction::Add, "Extra", "extra", Some(&extra)),
        ];
        let decisions = [Decision {
            path: "config/ask.cfg".into(),
            conflict,
        }];
        prepare(&pack, &s.server, &rules, &mods, |_| {})
            .unwrap()
            .finish(&decisions, |_| {})
            .unwrap();
        let read = |name: &str| fs::read_to_string(s.server.join(name)).unwrap();
        assert_eq!(read("config/ask.cfg"), expected);
        assert_eq!(read("config/custom.cfg"), "old custom");
        assert_eq!(read("config/a.cfg"), "new config");
        assert!(s.server.join("mods/extra-1.0.jar").is_file());
        assert!(s.server.join("mods/new.jar").is_file());
        assert!(!s.server.join("mods/passivemobs-1.3.0.jar").exists());
        assert_eq!(
            fs::read_to_string(s.backup.join("mods/old.jar")).unwrap(),
            "old mod"
        );
        assert_eq!(
            fs::read_to_string(s.backup.join("config/ask.cfg")).unwrap(),
            "old ask"
        );
        assert!(staging_left(&s.parent).is_empty());
    }
}

#[test]
fn rollback_restores_server_when_mods_and_transfers_are_present() {
    let (s, pack, extra) = rules_setup();
    let rules = [rule("config/custom.cfg", Conflict::Replace)];
    let mods = [mod_rule(ModAction::Add, "Extra", "extra", Some(&extra))];
    let before = snapshot(&s.server);
    let error = prepare(&pack, &s.server, &rules, &mods, |_| {})
        .unwrap()
        .finish_with_fault(&[], Fault::FailAfterMoves(3), |_| {})
        .unwrap_err();
    assert!(error.contains("rolled back"), "{error}");
    assert_eq!(snapshot(&s.server), before);
    assert!(!s.backup.exists());
    assert!(staging_left(&s.parent).is_empty());
}

#[test]
fn server_mod_addition_that_disappears_leaves_server_untouched() {
    let (s, pack, extra) = rules_setup();
    let mods = [mod_rule(ModAction::Add, "Extra", "extra", Some(&extra))];
    let before = snapshot(&s.server);
    let prepared = prepare(&pack, &s.server, &[], &mods, |_| {}).unwrap();
    fs::remove_file(&extra).unwrap();
    let error = prepared.finish(&[], |_| {}).unwrap_err();
    assert!(error.contains("Mod file disappeared"), "{error}");
    assert_eq!(snapshot(&s.server), before);
    assert!(staging_left(&s.parent).is_empty());
}
