use super::*;
const SERVER: &str = "https://downloads.gtnewhorizons.com/ServerPacks/test.zip";
const CLIENT: &str = "https://downloads.gtnewhorizons.com/Multi_mc_downloads/test.zip";
const OTHER: &str = "https://downloads.gtnewhorizons.com/Multi_mc_downloads/java8.zip";

#[test]
fn distinct_archives_run_concurrently_but_duplicate_active_url_is_rejected() {
    let store = DownloadStore::default();
    assert!(store.status().unwrap().is_empty());
    let (server, _) = store.begin(SERVER.into()).unwrap();
    let (client, _) = store.begin(CLIENT.into()).unwrap();
    let (other, _) = store.begin(OTHER.into()).unwrap();
    assert_ne!(server.id, client.id);
    assert_ne!(client.id, other.id);
    assert!(store.begin(SERVER.into()).is_err());
    assert!(store.begin(CLIENT.into()).is_err());
    let snapshot = store.status().unwrap();
    assert_eq!(snapshot.len(), 3);
    assert!(snapshot
        .iter()
        .all(|s| s.state == DownloadState::Downloading));
}

#[test]
fn cancelling_one_job_leaves_others_running_and_waits_for_cleanup() {
    let store = DownloadStore::default();
    let (server, server_token) = store.begin(SERVER.into()).unwrap();
    let (client, client_token) = store.begin(CLIENT.into()).unwrap();
    store.cancel(&server.id).unwrap();
    assert!(server_token.is_cancelled());
    assert!(!client_token.is_cancelled());
    assert!(store.begin(SERVER.into()).is_err());
    let terminal = store
        .finish(&server.id, Err("cancelled".into()))
        .unwrap()
        .unwrap();
    assert_eq!(terminal.state, DownloadState::Cancelled);
    let (retry, retry_token) = store.begin(SERVER.into()).unwrap();
    assert_ne!(retry.id, server.id);
    assert!(retry.revision > terminal.revision);
    // A late cancel from the previous attempt cannot cancel its replacement.
    store.cancel(&server.id).unwrap();
    assert!(!retry_token.is_cancelled());
    assert!(!client_token.is_cancelled());
    assert_eq!(
        store
            .status()
            .unwrap()
            .iter()
            .find(|s| s.id == client.id)
            .unwrap()
            .state,
        DownloadState::Downloading
    );
}

#[test]
fn completion_and_failure_do_not_overwrite_other_jobs() {
    let store = DownloadStore::default();
    let (server, _) = store.begin(SERVER.into()).unwrap();
    let (client, _) = store.begin(CLIENT.into()).unwrap();
    let (other, _) = store.begin(OTHER.into()).unwrap();
    store
        .progress(
            &server.id,
            DownloadProgress {
                received: 4,
                total: Some(4),
            },
        )
        .unwrap();
    store
        .progress(
            &client.id,
            DownloadProgress {
                received: 2,
                total: None,
            },
        )
        .unwrap();
    // Publication wins a cancellation arriving after the file was published.
    store.cancel(&server.id).unwrap();
    store
        .finish(&server.id, Ok(PathBuf::from("server.zip")))
        .unwrap();
    store
        .finish(&other.id, Err("network error".into()))
        .unwrap();
    let snapshot = store.status().unwrap();
    let server = snapshot.iter().find(|s| s.url == SERVER).unwrap();
    assert_eq!(server.state, DownloadState::Completed);
    assert_eq!(server.path.as_deref(), Some("server.zip"));
    assert_eq!(server.received, 4);
    let client = snapshot.iter().find(|s| s.url == CLIENT).unwrap();
    assert_eq!(client.state, DownloadState::Downloading);
    assert_eq!(client.received, 2);
    assert_eq!(client.total, None);
    let failed = snapshot.iter().find(|s| s.url == OTHER).unwrap();
    assert_eq!(failed.state, DownloadState::Failed);
    assert_eq!(failed.error.as_deref(), Some("network error"));
    assert!(store.begin(OTHER.into()).is_ok());
}

#[test]
fn stale_callbacks_cannot_overwrite_retries_or_other_archives() {
    let store = DownloadStore::default();
    let (first, _) = store.begin(SERVER.into()).unwrap();
    store
        .finish(&first.id, Err("network error".into()))
        .unwrap();
    let (second, _) = store.begin(SERVER.into()).unwrap();
    let (client, _) = store.begin(CLIENT.into()).unwrap();
    assert!(store
        .progress(
            &first.id,
            DownloadProgress {
                received: 99,
                total: None
            }
        )
        .unwrap()
        .is_none());
    assert!(store
        .finish(&first.id, Ok(PathBuf::from("old.zip")))
        .unwrap()
        .is_none());
    let snapshot = store.status().unwrap();
    assert_eq!(snapshot.len(), 2);
    assert!(snapshot
        .iter()
        .any(|s| s.id == second.id && s.received == 0 && s.state == DownloadState::Downloading));
    assert!(snapshot
        .iter()
        .any(|s| s.id == client.id && s.received == 0 && s.state == DownloadState::Downloading));
}

#[test]
fn only_completed_downloads_can_be_installed() {
    let store = DownloadStore::default();
    assert!(store.completed_path(SERVER).is_err());
    let (job, _) = store.begin(SERVER.into()).unwrap();
    assert!(store.completed_path(SERVER).is_err());
    store
        .finish(&job.id, Ok(PathBuf::from("archive.zip")))
        .unwrap();
    assert_eq!(
        store.completed_path(SERVER).unwrap(),
        PathBuf::from("archive.zip")
    );
    // Installing never consumes the archive.
    assert_eq!(
        store.completed_path(SERVER).unwrap(),
        PathBuf::from("archive.zip")
    );
}

#[test]
fn simultaneous_starts_of_the_same_url_have_one_winner() {
    let store = std::sync::Arc::new(DownloadStore::default());
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let store = store.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store.begin(SERVER.into()).is_ok()
            })
        })
        .collect();
    let winners = threads
        .into_iter()
        .filter_map(|t| t.join().unwrap().then_some(()))
        .count();
    assert_eq!(winners, 1);
    assert_eq!(store.status().unwrap().len(), 1);
}

fn entry(url: &str, file: &str, bytes: u64) -> IndexEntry {
    IndexEntry {
        url: url.into(),
        file: file.into(),
        bytes,
    }
}

#[test]
fn index_round_trips_and_corrupt_index_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    assert!(read_index(dir.path()).is_empty());
    let entries = vec![entry(SERVER, "gtnh-a.zip", 3)];
    write_index(dir.path(), entries.clone()).unwrap();
    assert_eq!(read_index(dir.path()), entries);
    write_index(dir.path(), vec![]).unwrap();
    assert!(read_index(dir.path()).is_empty());
    std::fs::write(dir.path().join(INDEX_FILE), b"{not json").unwrap();
    assert!(read_index(dir.path()).is_empty());
    let store = DownloadStore::default();
    store.restore(dir.path().into()).unwrap();
    assert!(store.status().unwrap().is_empty());
}

#[test]
fn restore_registers_completed_and_drops_invalid_entries() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("gtnh-a.zip"), b"abc").unwrap();
    std::fs::write(dir.path().join("gtnh-b.zip"), b"abcd").unwrap();
    std::fs::write(dir.path().join("other.zip"), b"abc").unwrap();
    std::fs::write(dir.path().join("x.zip"), b"abc").unwrap();
    write_index(
        dir.path(),
        vec![
            entry(SERVER, "gtnh-a.zip", 3),
            entry(CLIENT, "gtnh-missing.zip", 3),
            entry(OTHER, "gtnh-b.zip", 3),
            entry(OTHER, "../x.zip", 3),
            entry(OTHER, "a/b.zip", 3),
            entry(OTHER, "other.zip", 3),
            entry("https://example.com/test.zip", "gtnh-b.zip", 4),
        ],
    )
    .unwrap();
    let store = DownloadStore::default();
    store.restore(dir.path().into()).unwrap();
    let snapshot = store.status().unwrap();
    assert_eq!(snapshot.len(), 1);
    let job = &snapshot[0];
    assert_eq!(job.url, SERVER);
    assert_eq!(job.state, DownloadState::Completed);
    assert_eq!((job.received, job.total), (3, Some(3)));
    assert_eq!(
        store.completed_path(SERVER).unwrap(),
        dir.path().join("gtnh-a.zip")
    );
    assert_eq!(read_index(dir.path()), vec![entry(SERVER, "gtnh-a.zip", 3)]);
}

#[test]
fn finish_records_entry_and_redownload_replaces_old_file() {
    let dir = tempfile::tempdir().unwrap();
    let store = DownloadStore::default();
    store.restore(dir.path().into()).unwrap();
    let first = dir.path().join("gtnh-one.zip");
    let second = dir.path().join("gtnh-two.zip");
    std::fs::write(&first, b"12").unwrap();
    std::fs::write(&second, b"12345").unwrap();
    let (job, _) = store.begin(SERVER.into()).unwrap();
    store.finish(&job.id, Ok(first.clone())).unwrap();
    assert_eq!(
        read_index(dir.path()),
        vec![entry(SERVER, "gtnh-one.zip", 2)]
    );
    let (job, _) = store.begin(SERVER.into()).unwrap();
    store.finish(&job.id, Ok(second.clone())).unwrap();
    assert_eq!(
        read_index(dir.path()),
        vec![entry(SERVER, "gtnh-two.zip", 5)]
    );
    assert!(!first.exists());
    assert!(second.exists());
}

#[test]
fn delete_removes_file_index_entry_and_job() {
    let dir = tempfile::tempdir().unwrap();
    let store = DownloadStore::default();
    store.restore(dir.path().into()).unwrap();
    let file = dir.path().join("gtnh-one.zip");
    std::fs::write(&file, b"12").unwrap();
    let (a, _) = store.begin(SERVER.into()).unwrap();
    store.finish(&a.id, Ok(file.clone())).unwrap();
    let other = dir.path().join("gtnh-two.zip");
    std::fs::write(&other, b"1").unwrap();
    let (b, _) = store.begin(CLIENT.into()).unwrap();
    store.finish(&b.id, Ok(other.clone())).unwrap();
    assert_eq!(read_index(dir.path()).len(), 2);
    let tombstone = store.delete(SERVER).unwrap();
    assert_eq!(tombstone.state, DownloadState::Deleted);
    assert!(tombstone.revision > a.revision);
    assert!(!file.exists());
    assert!(other.exists());
    assert_eq!(
        read_index(dir.path()),
        vec![entry(CLIENT, "gtnh-two.zip", 1)]
    );
    let snapshot = store.status().unwrap();
    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].url, CLIENT);
    assert!(store.delete(SERVER).is_err());
}

#[test]
fn delete_refuses_non_completed_jobs() {
    let dir = tempfile::tempdir().unwrap();
    let store = DownloadStore::default();
    store.restore(dir.path().into()).unwrap();
    assert!(store.delete(SERVER).is_err());
    let (job, _) = store.begin(SERVER.into()).unwrap();
    assert!(store.delete(SERVER).is_err());
    store.finish(&job.id, Err("network error".into())).unwrap();
    assert!(store.delete(SERVER).is_err());
    assert_eq!(store.status().unwrap().len(), 1);
}

#[test]
fn delete_never_removes_files_outside_the_downloads_directory() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("downloads");
    std::fs::create_dir(&dir).unwrap();
    let outside = root.path().join("gtnh-outside.zip");
    let badly_named = dir.join("notes.zip");
    std::fs::write(&outside, b"12").unwrap();
    std::fs::write(&badly_named, b"12").unwrap();
    let store = DownloadStore::default();
    store.restore(dir.clone()).unwrap();
    for (url, path) in [(SERVER, &outside), (CLIENT, &badly_named)] {
        let (job, _) = store.begin(url.into()).unwrap();
        store.finish(&job.id, Ok(path.clone())).unwrap();
        assert_eq!(store.delete(url).unwrap().state, DownloadState::Deleted);
    }
    assert!(outside.exists());
    assert!(badly_named.exists());
    assert!(store.status().unwrap().is_empty());
}

fn sizes(items: &[(&str, u64)]) -> BTreeMap<String, u64> {
    items.iter().map(|(u, s)| (u.to_string(), *s)).collect()
}
fn orphan_store(dir: &Path) -> DownloadStore {
    let store = DownloadStore::default();
    store.restore(dir.into()).unwrap();
    store
}
fn adopt_now(store: &DownloadStore, urls: &[&str], remote: &[(&str, u64)]) -> Vec<DownloadStatus> {
    let urls = urls.iter().map(|u| u.to_string()).collect();
    let Some((orphans, _)) = store.adoption_input(urls).unwrap() else {
        return vec![];
    };
    store
        .apply_adoption(plan_adoption(&orphans, &sizes(remote)))
        .unwrap()
}

#[test]
fn no_orphans_means_no_input() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("gtnh-a.zip"), b"abc").unwrap();
    write_index(dir.path(), vec![entry(SERVER, "gtnh-a.zip", 3)]).unwrap();
    let store = orphan_store(dir.path());
    assert!(store.adoption_input(vec![CLIENT.into()]).unwrap().is_none());
    assert!(DownloadStore::default()
        .adoption_input(vec![CLIENT.into()])
        .unwrap()
        .is_none());
    assert!(adopt_now(&store, &[CLIENT], &[(CLIENT, 3)]).is_empty());
    assert_eq!(read_index(dir.path()), vec![entry(SERVER, "gtnh-a.zip", 3)]);
}

#[test]
fn unique_size_match_adopts_and_indexes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("gtnh-a.zip"), b"abc").unwrap();
    std::fs::write(dir.path().join("gtnh-b.zip"), b"abcdef").unwrap();
    std::fs::write(dir.path().join("notes.zip"), b"abc").unwrap();
    let store = orphan_store(dir.path());
    let adopted = adopt_now(&store, &[CLIENT, OTHER], &[(CLIENT, 3), (OTHER, 99)]);
    assert_eq!(adopted.len(), 1);
    assert_eq!(adopted[0].url, CLIENT);
    assert_eq!(adopted[0].state, DownloadState::Completed);
    assert_eq!(
        store.completed_path(CLIENT).unwrap(),
        dir.path().join("gtnh-a.zip")
    );
    assert_eq!(read_index(dir.path()), vec![entry(CLIENT, "gtnh-a.zip", 3)]);
    // Non-matching orphans stay untouched.
    assert!(dir.path().join("gtnh-b.zip").exists());
    assert!(dir.path().join("notes.zip").exists());
}

#[test]
fn duplicate_orphans_keep_newest_and_delete_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("gtnh-old.zip");
    let new = dir.path().join("gtnh-new.zip");
    std::fs::write(&old, b"abc").unwrap();
    std::fs::write(&new, b"abc").unwrap();
    let now = SystemTime::now();
    std::fs::File::options()
        .write(true)
        .open(&old)
        .unwrap()
        .set_modified(now - Duration::from_secs(3600))
        .unwrap();
    std::fs::File::options()
        .write(true)
        .open(&new)
        .unwrap()
        .set_modified(now)
        .unwrap();
    let store = orphan_store(dir.path());
    let adopted = adopt_now(&store, &[CLIENT], &[(CLIENT, 3)]);
    assert_eq!(adopted.len(), 1);
    assert!(new.exists());
    assert!(!old.exists());
    assert_eq!(
        read_index(dir.path()),
        vec![entry(CLIENT, "gtnh-new.zip", 3)]
    );
}

#[test]
fn ambiguous_sizes_are_never_adopted() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("gtnh-a.zip"), b"abc").unwrap();
    let store = orphan_store(dir.path());
    let adopted = adopt_now(&store, &[CLIENT, OTHER], &[(CLIENT, 3), (OTHER, 3)]);
    assert!(adopted.is_empty());
    assert!(store.status().unwrap().is_empty());
    assert!(dir.path().join("gtnh-a.zip").exists());
    assert!(read_index(dir.path()).is_empty());
}

#[test]
fn indexed_and_active_files_are_not_orphans_and_busy_urls_are_skipped() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("gtnh-a.zip"), b"abc").unwrap();
    std::fs::write(dir.path().join("gtnh-b.zip"), b"abc").unwrap();
    write_index(dir.path(), vec![entry(SERVER, "gtnh-a.zip", 3)]).unwrap();
    let store = orphan_store(dir.path());
    let (orphans, candidates) = store
        .adoption_input(vec![
            SERVER.into(),
            CLIENT.into(),
            CLIENT.into(),
            "bad".into(),
        ])
        .unwrap()
        .unwrap();
    assert_eq!(orphans.len(), 1);
    assert_eq!(orphans[0].file, "gtnh-b.zip");
    // SERVER is already completed, duplicates and invalid URLs are dropped.
    assert_eq!(candidates, vec![CLIENT.to_string()]);
    // A file owned by a job is not an orphan either.
    let (job, _) = store.begin(OTHER.into()).unwrap();
    store
        .finish(&job.id, Ok(dir.path().join("gtnh-b.zip")))
        .unwrap();
    assert!(store.adoption_input(vec![CLIENT.into()]).unwrap().is_none());
}

#[test]
fn only_one_adoption_runs_at_a_time() {
    let store = DownloadStore::default();
    assert!(store.set_adopting(true));
    assert!(!store.set_adopting(true));
    assert!(store.set_adopting(false));
    assert!(store.set_adopting(true));
}
