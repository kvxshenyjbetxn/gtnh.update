//! Pure, non-view helpers ported from `src/installation.ts`, `src/downloads.ts` and
//! `src/pack-dialog.ts`. Native dialogs (rfd) are left to the views.

use crate::backend::{DownloadState, DownloadStatus, Status};
use gtnh_core::{installation::Transfer, releases::Artifact, Conflict, Language};
use std::{collections::HashMap, future::Future};

/// Display name of a pack from a URL or path; supports `/` and `\`, trailing separators and an
/// upper-case `.ZIP`. Falls back to `GTNH`.
pub fn pack_name(path: &str) -> String {
    let last = path
        .split(['/', '\\'])
        .rfind(|part| !part.is_empty())
        .unwrap_or("");
    let name = if last.len() >= 4 && last[last.len() - 4..].eq_ignore_ascii_case(".zip") {
        &last[..last.len() - 4]
    } else {
        last
    };
    if name.is_empty() { "GTNH" } else { name }.to_string()
}

/// Validates the saved Prism instances folder at the point of use; a missing preference has no
/// filesystem side effects. `validate` is typically `Backend::instances_directory`.
pub async fn resolve_prism_directory<F, Fut>(
    saved: Option<&str>,
    validate: F,
) -> Result<Option<String>, String>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = Result<String, String>>,
{
    match saved {
        Some(path) if !path.is_empty() => validate(path.to_string()).await.map(Some),
        _ => Ok(None),
    }
}

/// Existing regular files with the `ask` policy that still lack a `replace`/`keep` decision.
pub fn unresolved_conflicts(
    files: &[Transfer],
    decisions: &HashMap<String, Conflict>,
) -> Vec<String> {
    files
        .iter()
        .filter(|file| {
            file.exists
                && !file.directory
                && file.conflict == Conflict::Ask
                && !matches!(
                    decisions.get(&file.path),
                    Some(Conflict::Replace | Conflict::Keep)
                )
        })
        .map(|file| file.path.clone())
        .collect()
}

/// Poll bookkeeping for `Backend::installation_status` (port of `watchInstallation`): never two
/// polls at once, revisions only move forward, nothing is accepted after `dispose`.
///
/// Loop: `if let Some(since) = w.begin_poll() { let r = backend.installation_status(since);
/// match r { Ok(s) => if let Some(s) = w.finish_poll(s) { render(s) }, Err(_) => w.abort_poll() } }`.
#[derive(Debug, Default)]
pub struct InstallWatcher {
    revision: Option<u64>,
    pending: bool,
    disposed: bool,
}
impl InstallWatcher {
    pub fn new() -> Self {
        Self::default()
    }
    /// `Some(since)` when a poll may start (pass `since` to `installation_status`); `None` while
    /// another poll is pending or after `dispose`.
    pub fn begin_poll(&mut self) -> Option<Option<u64>> {
        if self.disposed || self.pending {
            return None;
        }
        self.pending = true;
        Some(self.revision)
    }
    /// Ends the poll; returns the status only when it is newer than the last accepted one.
    pub fn finish_poll(&mut self, status: Option<Status>) -> Option<Status> {
        self.pending = false;
        let status = status?;
        if self.disposed || self.revision.is_some_and(|last| status.revision <= last) {
            return None;
        }
        self.revision = Some(status.revision);
        Some(status)
    }
    /// Ends a failed poll.
    pub fn abort_poll(&mut self) {
        self.pending = false;
    }
    #[allow(dead_code)] // the polling loop ends with its entity; kept for the tested contract
    pub fn dispose(&mut self) {
        self.disposed = true;
    }
}

/// Top-level transfer entry with the totals of everything below it.
#[derive(Debug, Clone)]
pub struct TransferGroup {
    pub path: String,
    pub directory: bool,
    /// The entry itself, when it is in the list (intermediate folders may be absent).
    pub file: Option<Transfer>,
    pub files: usize,
    pub bytes: u64,
}
/// Groups transfers under their highest ancestor that is itself an entry; counts and sizes of
/// descendant files are credited to that top-level entry. Linear in the number of entries.
pub fn group_transfers(files: &[Transfer]) -> Vec<TransferGroup> {
    let known: std::collections::HashSet<&str> = files.iter().map(|f| f.path.as_str()).collect();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut groups: Vec<TransferGroup> = Vec::new();
    for file in files {
        // The shortest known ancestor wins; the file itself otherwise.
        let top = file
            .path
            .match_indices('/')
            .map(|(cut, _)| cut)
            .filter(|&cut| cut > 0)
            .map(|cut| &file.path[..cut])
            .find(|parent| known.contains(parent))
            .unwrap_or(file.path.as_str());
        let slot = *index.entry(top.to_string()).or_insert_with(|| {
            groups.push(TransferGroup {
                path: top.to_string(),
                directory: false,
                file: None,
                files: 0,
                bytes: 0,
            });
            groups.len() - 1
        });
        let group = &mut groups[slot];
        if top == file.path {
            group.file = Some(file.clone());
            group.directory = file.directory;
        }
        if !file.directory {
            group.files += 1;
            group.bytes += file.bytes;
        }
    }
    groups
}

/// Binary size with at most one decimal; Ukrainian uses a decimal comma.
pub fn format_bytes(value: u64, language: Language) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut size = value as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    let mut number = if unit == 0 {
        format!("{size:.0}")
    } else {
        format!("{size:.1}")
    };
    if number.contains('.') {
        number = number
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string();
    }
    if language == Language::Uk {
        number = number.replace('.', ",");
    }
    format!("{number} {}", UNITS[unit])
}

/// Merges the download snapshot and events (port of `watchDownloads`). Revisions are compared per
/// URL, a `deleted` status stays as a tombstone so stale events cannot bring the job back, and the
/// visible list excludes tombstones. Subscribe first (`Backend::watch_downloads`), then `accept`
/// the snapshot and every event.
#[derive(Debug, Default)]
pub struct DownloadTracker {
    latest: Vec<DownloadStatus>,
}
impl DownloadTracker {
    pub fn new() -> Self {
        Self::default()
    }
    /// Returns the new visible list when the status was accepted, `None` when it is stale.
    pub fn accept(&mut self, status: DownloadStatus) -> Option<Vec<DownloadStatus>> {
        match self.latest.iter_mut().find(|s| s.url == status.url) {
            Some(known) if status.revision <= known.revision => return None,
            Some(known) => *known = status,
            None => self.latest.push(status),
        }
        Some(self.visible())
    }
    /// Accepts a whole snapshot; returns the visible list if anything changed.
    pub fn accept_all(
        &mut self,
        statuses: impl IntoIterator<Item = DownloadStatus>,
    ) -> Option<Vec<DownloadStatus>> {
        let mut changed = false;
        for status in statuses {
            changed |= self.accept(status).is_some();
        }
        changed.then(|| self.visible())
    }
    pub fn visible(&self) -> Vec<DownloadStatus> {
        self.latest
            .iter()
            .filter(|s| s.state != DownloadState::Deleted)
            .cloned()
            .collect()
    }
}

/// Downloads of the artifacts of one mode, matched by file URL (not by selected variant).
pub fn downloads_for_artifacts<'a>(
    statuses: &'a [DownloadStatus],
    artifacts: &'a [Artifact],
    mode: &str,
) -> Vec<(&'a Artifact, &'a DownloadStatus)> {
    artifacts
        .iter()
        .filter(|artifact| artifact.mode == mode)
        .filter_map(|artifact| {
            statuses
                .iter()
                .find(|status| status.url == artifact.url)
                .map(|status| (artifact, status))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;

    #[test]
    fn pack_names_support_urls_windows_paths_folder_paths_and_uppercase_zip() {
        assert_eq!(pack_name("https://example.test/GTNH_2.8.zip"), "GTNH_2.8");
        assert_eq!(pack_name("C:\\Downloads\\GTNH.ZIP"), "GTNH");
        assert_eq!(pack_name("/home/me/New Pack/"), "New Pack");
        assert_eq!(pack_name("/"), "GTNH");
        assert_eq!(pack_name("/x/.zip"), "GTNH");
    }

    fn transfer(
        path: &str,
        exists: bool,
        directory: bool,
        conflict: Conflict,
        bytes: u64,
    ) -> Transfer {
        Transfer {
            path: path.into(),
            exists,
            conflict,
            directory,
            bytes,
        }
    }

    #[test]
    fn only_existing_regular_files_with_ask_policy_require_a_decision() {
        let files = vec![
            transfer("a", true, false, Conflict::Ask, 1),
            transfer("b", false, false, Conflict::Ask, 1),
            transfer("dir", true, true, Conflict::Ask, 0),
            transfer("c", true, false, Conflict::Replace, 1),
        ];
        let decide = |items: &[(&str, Conflict)]| {
            let decisions = items.iter().map(|(p, c)| (p.to_string(), *c)).collect();
            unresolved_conflicts(&files, &decisions)
        };
        assert_eq!(decide(&[]), vec!["a"]);
        assert_eq!(decide(&[("a", Conflict::Ask)]), vec!["a"]);
        assert!(decide(&[("a", Conflict::Keep)]).is_empty());
        assert!(decide(&[("a", Conflict::Replace)]).is_empty());
    }

    fn status(revision: u64, phase: &str) -> Status {
        Status {
            revision,
            phase: phase.into(),
            ..Default::default()
        }
    }

    #[test]
    fn installation_watcher_rejects_stale_polls_and_stops_publishing_after_dispose() {
        let mut watcher = InstallWatcher::new();
        assert_eq!(watcher.begin_poll(), Some(None));
        // No overlapping filesystem-status calls.
        assert_eq!(watcher.begin_poll(), None);
        assert_eq!(
            watcher
                .finish_poll(Some(status(4, "preview")))
                .unwrap()
                .phase,
            "preview"
        );
        assert!(watcher.begin_poll().is_some());
        assert!(watcher.finish_poll(Some(status(3, "preparing"))).is_none());
        assert!(watcher.begin_poll().is_some());
        watcher.dispose();
        assert!(watcher.finish_poll(Some(status(5, "completed"))).is_none());
        assert_eq!(watcher.begin_poll(), None);
    }

    #[test]
    fn installation_watcher_sends_last_accepted_revision_and_ignores_unchanged_responses() {
        let mut watcher = InstallWatcher::new();
        assert_eq!(watcher.begin_poll(), Some(None));
        assert_eq!(
            watcher
                .finish_poll(Some(status(2, "preview")))
                .unwrap()
                .revision,
            2
        );
        assert_eq!(watcher.begin_poll(), Some(Some(2)));
        assert!(watcher.finish_poll(None).is_none());
        // A failed poll frees the slot without changing the revision.
        assert_eq!(watcher.begin_poll(), Some(Some(2)));
        watcher.abort_poll();
        assert_eq!(watcher.begin_poll(), Some(Some(2)));
    }

    #[test]
    fn top_level_transfer_groups_summarize_descendants_and_stay_fast_for_12000_entries() {
        let dir = |path: &str| transfer(path, true, true, Conflict::Replace, 0);
        let file = |path: &str, bytes| transfer(path, false, false, Conflict::Replace, bytes);
        let small = group_transfers(&[
            dir("saves"),
            dir("saves/a"),
            file("saves/a/x", 5),
            file("saves/y", 7),
            file("options.txt", 3),
        ]);
        let summary: Vec<_> = small
            .iter()
            .map(|g| (g.path.as_str(), g.directory, g.files, g.bytes))
            .collect();
        assert_eq!(
            summary,
            vec![("saves", true, 2, 12), ("options.txt", false, 1, 3)]
        );
        let mut big = vec![dir("saves")];
        for i in 0..12000 {
            big.push(file(&format!("saves/w{}/f{i}", i % 50), 1));
        }
        let started = std::time::Instant::now();
        let groups = group_transfers(&big);
        assert!(started.elapsed() < std::time::Duration::from_millis(500));
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].files, 12000);
    }

    #[test]
    fn missing_intermediate_folders_group_under_the_highest_known_ancestor() {
        let file = |path: &str, bytes| transfer(path, false, false, Conflict::Replace, bytes);
        let groups = group_transfers(&[
            transfer("a", true, true, Conflict::Replace, 0),
            file("a/b/c", 2),
            file("z/q", 1),
        ]);
        let summary: Vec<_> = groups.iter().map(|g| (g.path.as_str(), g.files)).collect();
        assert_eq!(summary, vec![("a", 1), ("z/q", 1)]);
    }

    #[test]
    fn byte_sizes_use_binary_units() {
        assert_eq!(format_bytes(0, Language::En), "0 B");
        assert_eq!(format_bytes(1536, Language::En), "1.5 KiB");
        assert_eq!(format_bytes(1536, Language::Uk), "1,5 KiB");
        assert_eq!(format_bytes(2 * 1024 * 1024, Language::En), "2 MiB");
        assert_eq!(format_bytes(5 * 1024 * 1024 * 1024, Language::En), "5 GiB");
    }

    #[test]
    fn prism_preference_is_validated_at_point_of_use_and_missing_has_no_side_effects() {
        let none = block_on(resolve_prism_directory(None, |_| async {
            panic!("must not run")
        }));
        assert_eq!(none, Ok(None));
        let ok = block_on(resolve_prism_directory(Some("/saved"), |path| async move {
            assert_eq!(path, "/saved");
            Ok("/canonical".to_string())
        }));
        assert_eq!(ok, Ok(Some("/canonical".to_string())));
        let err = block_on(resolve_prism_directory(Some("/moved"), |_| async {
            Err("missing".to_string())
        }));
        assert_eq!(err, Err("missing".to_string()));
    }

    const CLIENT: &str = "https://downloads.gtnewhorizons.com/Multi_mc_downloads/client.zip";
    const SERVER: &str = "https://downloads.gtnewhorizons.com/ServerPacks/server.zip";
    const OTHER: &str = "https://downloads.gtnewhorizons.com/Multi_mc_downloads/java8.zip";
    fn download(url: &str, revision: u64, state: DownloadState) -> DownloadStatus {
        DownloadStatus {
            id: format!("job-{url}"),
            revision,
            url: url.into(),
            state,
            received: revision,
            total: None,
            path: None,
            error: None,
        }
    }
    fn received(list: &[DownloadStatus], url: &str) -> u64 {
        list.iter().find(|s| s.url == url).unwrap().received
    }

    #[test]
    fn late_snapshots_merge_independent_jobs_and_out_of_order_events_only_affect_their_url() {
        let mut tracker = DownloadTracker::new();
        let mut latest = tracker
            .accept(download(CLIENT, 10, DownloadState::Downloading))
            .unwrap();
        latest = tracker
            .accept_all([
                download(CLIENT, 1, DownloadState::Downloading),
                download(SERVER, 2, DownloadState::Downloading),
            ])
            .unwrap_or(latest);
        assert_eq!(latest.len(), 2);
        assert_eq!(received(&latest, CLIENT), 10);
        assert_eq!(received(&latest, SERVER), 2);
        // A global high-water revision would incorrectly drop server progress.
        tracker.accept(download(SERVER, 4, DownloadState::Downloading));
        assert!(tracker
            .accept(download(CLIENT, 9, DownloadState::Downloading))
            .is_none());
        assert!(tracker
            .accept(download(SERVER, 3, DownloadState::Downloading))
            .is_none());
        let latest = tracker.visible();
        assert_eq!(received(&latest, CLIENT), 10);
        assert_eq!(received(&latest, SERVER), 4);
    }

    #[test]
    fn retry_replaces_only_its_file_and_stale_terminal_events_cannot_restore_the_old_attempt() {
        let mut tracker = DownloadTracker::new();
        tracker.accept_all([
            download(CLIENT, 3, DownloadState::Failed),
            download(SERVER, 2, DownloadState::Downloading),
        ]);
        let mut retry = download(CLIENT, 5, DownloadState::Downloading);
        retry.id = "retry".into();
        retry.received = 0;
        tracker.accept(retry);
        assert!(tracker
            .accept(download(CLIENT, 4, DownloadState::Failed))
            .is_none());
        let latest = tracker.visible();
        assert_eq!(latest.len(), 2);
        let client = latest.iter().find(|s| s.url == CLIENT).unwrap();
        assert_eq!(client.id, "retry");
        assert_eq!(client.state, DownloadState::Downloading);
        assert_eq!(received(&latest, SERVER), 2);
    }

    #[test]
    fn remount_rehydrates_all_files() {
        let snapshot = || {
            [
                download(CLIENT, 8, DownloadState::Downloading),
                download(SERVER, 9, DownloadState::Downloading),
            ]
        };
        let mut first = DownloadTracker::new();
        assert_eq!(first.accept_all(snapshot()).unwrap().len(), 2);
        let mut again = DownloadTracker::new();
        let list = again.accept_all(snapshot()).unwrap();
        assert_eq!(
            list.iter().map(|s| s.received).collect::<Vec<_>>(),
            vec![8, 9]
        );
    }

    #[test]
    fn deleted_status_drops_the_job_and_stale_events_cannot_bring_it_back() {
        let mut tracker = DownloadTracker::new();
        let list = tracker
            .accept_all([
                download(CLIENT, 3, DownloadState::Completed),
                download(SERVER, 2, DownloadState::Completed),
            ])
            .unwrap();
        assert_eq!(list.len(), 2);
        let urls = |list: Vec<DownloadStatus>| list.into_iter().map(|s| s.url).collect::<Vec<_>>();
        assert_eq!(
            urls(
                tracker
                    .accept(download(CLIENT, 5, DownloadState::Deleted))
                    .unwrap()
            ),
            vec![SERVER]
        );
        assert!(tracker
            .accept(download(CLIENT, 4, DownloadState::Completed))
            .is_none());
        assert_eq!(urls(tracker.visible()), vec![SERVER]);
        // A new download after deletion has a higher revision and reappears.
        let mut back = urls(
            tracker
                .accept(download(CLIENT, 6, DownloadState::Downloading))
                .unwrap(),
        );
        back.sort();
        assert_eq!(back, vec![CLIENT, SERVER]);
    }

    #[test]
    fn each_release_row_shows_only_its_own_files_and_current_mode() {
        let states = [
            download(CLIENT, 3, DownloadState::Downloading),
            download(SERVER, 2, DownloadState::Downloading),
            download(OTHER, 4, DownloadState::Downloading),
        ];
        let artifact = |mode: &str, java: &str, url: &str| Artifact {
            mode: mode.into(),
            java: java.into(),
            url: url.into(),
        };
        let artifacts = [
            artifact("client", "Java 17", CLIENT),
            artifact("server", "Java 17", SERVER),
            artifact("client", "Java 8", OTHER),
        ];
        let client: Vec<_> = downloads_for_artifacts(&states, &artifacts, "client")
            .iter()
            .map(|(a, s)| (a.java.clone(), s.url.clone()))
            .collect();
        assert_eq!(
            client,
            vec![
                ("Java 17".to_string(), CLIENT.to_string()),
                ("Java 8".to_string(), OTHER.to_string())
            ]
        );
        let server: Vec<_> = downloads_for_artifacts(&states, &artifacts, "server")
            .iter()
            .map(|(_, s)| s.url.clone())
            .collect();
        assert_eq!(server, vec![SERVER]);
        let unrelated = [artifact("client", "Java 17", "another-release.zip")];
        assert!(downloads_for_artifacts(&states, &unrelated, "client").is_empty());
    }
}
