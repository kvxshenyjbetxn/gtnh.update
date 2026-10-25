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
    let installed = store
        .mark_installed(SERVER, std::path::Path::new("result"))
        .unwrap()
        .unwrap();
    assert_eq!(installed.state, DownloadState::Installed);
    assert!(store.completed_path(SERVER).is_err());
    assert!(store.begin(SERVER.into()).is_ok());
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
