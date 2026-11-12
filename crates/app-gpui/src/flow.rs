//! Wizard logic of the update flow, a port of `src/update-flow.ts`. Pure state transitions, no UI.
#![allow(dead_code)] // used by the views in the next step

use crate::backend::{InstallRequest, ServerUpdateRequest};
use gtnh_core::{ModRule, RenameRule, Rule};
use std::path::PathBuf;

/// Which build the wizard updates (the TS `Mode = 'client' | 'server'`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    Client,
    Server,
}

impl Mode {
    /// Matches `gtnh_core::releases::Artifact::mode`.
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Client => "client",
            Mode::Server => "server",
        }
    }
}

/// Wizard section. `index()` is the TS step number (0..=3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum UpdateStep {
    Source,
    Pack,
    Transfer,
    Review,
}
impl UpdateStep {
    pub fn index(self) -> u8 {
        self as u8
    }
    pub fn from_index(index: u8) -> Option<Self> {
        Some(match index {
            0 => Self::Source,
            1 => Self::Pack,
            2 => Self::Transfer,
            3 => Self::Review,
            _ => return None,
        })
    }
}

/// The chosen pack: a downloaded official archive (by URL) or a local ZIP/folder path.
#[derive(Debug, Clone, PartialEq)]
pub enum PackChoice {
    Local(String),
    Url(String),
}
impl PackChoice {
    /// The URL or path, e.g. for `logic::pack_name`.
    pub fn label(&self) -> &str {
        match self {
            Self::Local(value) | Self::Url(value) => value,
        }
    }
    fn into_parts(self) -> (Option<String>, Option<PathBuf>) {
        match self {
            Self::Local(path) => (None, Some(PathBuf::from(path))),
            Self::Url(url) => (Some(url), None),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct UpdateDraft {
    /// Old client game folder or server folder.
    pub source: Option<String>,
    pub pack: Option<PackChoice>,
    pub step: UpdateStep,
    /// Number of explicitly completed sections (0..=3); section `n` can be opened when `n <= completed`.
    pub completed: u8,
}

pub fn create_draft(source: Option<String>) -> UpdateDraft {
    UpdateDraft {
        source,
        pack: None,
        step: UpdateStep::Source,
        completed: 0,
    }
}
pub fn change_source(draft: UpdateDraft, source: String) -> UpdateDraft {
    if draft.source.as_deref() == Some(source.as_str()) {
        return draft;
    }
    create_draft(Some(source))
}
pub fn change_pack(draft: UpdateDraft, pack: PackChoice) -> UpdateDraft {
    if draft.source.is_none() || draft.completed < 1 {
        return draft;
    }
    UpdateDraft {
        pack: Some(pack),
        step: UpdateStep::Pack,
        completed: 1,
        ..draft
    }
}
pub fn continue_step(draft: UpdateDraft) -> UpdateDraft {
    if draft.step == UpdateStep::Review
        || draft.source.is_none()
        || (draft.step > UpdateStep::Source && draft.pack.is_none())
    {
        return draft;
    }
    let next = draft.step.index() + 1;
    UpdateDraft {
        step: UpdateStep::from_index(next).unwrap_or(UpdateStep::Review),
        completed: draft.completed.max(next),
        ..draft
    }
}
pub fn open_step(draft: UpdateDraft, step: UpdateStep) -> UpdateDraft {
    if step.index() <= draft.completed {
        UpdateDraft { step, ..draft }
    } else {
        draft
    }
}

/// Client installation inputs captured from the draft. The view adds the destination
/// (`parent`, `name`) with [`ClientRequest::into_install_request`].
#[derive(Debug, Clone, PartialEq)]
pub struct ClientRequest {
    pub pack: PackChoice,
    /// Old instance's game folder.
    pub source: String,
    pub rules: Vec<Rule>,
    pub mods: Vec<ModRule>,
    pub renames: Vec<RenameRule>,
}
impl ClientRequest {
    /// Request for `Backend::prepare_installation`. `parent` is the Prism instances folder and
    /// `name` the new instance folder name.
    pub fn into_install_request(self, parent: String, name: String) -> InstallRequest {
        let (url, local_path) = self.pack.into_parts();
        InstallRequest {
            url,
            local_path,
            source: self.source,
            parent,
            name,
            client: true,
            rules: self.rules,
            mods: self.mods,
            renames: self.renames,
        }
    }
}
/// Server update inputs captured from the draft.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerRequest {
    pub pack: PackChoice,
    /// Server folder to update.
    pub server: String,
    pub rules: Vec<Rule>,
    pub mods: Vec<ModRule>,
}
impl ServerRequest {
    /// Request for `Backend::prepare_server_update`.
    pub fn into_update_request(self) -> ServerUpdateRequest {
        let (url, local_path) = self.pack.into_parts();
        ServerUpdateRequest {
            url,
            local_path,
            server: self.server,
            rules: self.rules,
            mods: self.mods,
        }
    }
}
/// Result of [`build_install_request`]; the view calls the matching backend method.
#[derive(Debug, Clone, PartialEq)]
pub enum PreparedRequest {
    Client(ClientRequest),
    Server(ServerRequest),
}

/// Captures an owned copy of the inputs, only after all three sections are completed.
/// The server variant ignores `renames`.
pub fn build_install_request(
    draft: &UpdateDraft,
    client: bool,
    rules: &[Rule],
    mods: &[ModRule],
    renames: &[RenameRule],
) -> Option<PreparedRequest> {
    if draft.completed < 3 {
        return None;
    }
    let (source, pack) = (draft.source.clone()?, draft.pack.clone()?);
    Some(if client {
        PreparedRequest::Client(ClientRequest {
            pack,
            source,
            rules: rules.to_vec(),
            mods: mods.to_vec(),
            renames: renames.to_vec(),
        })
    } else {
        PreparedRequest::Server(ServerRequest {
            pack,
            server: source,
            rules: rules.to_vec(),
            mods: mods.to_vec(),
        })
    })
}

/// True while a transaction is being sent or runs (`phase` from `installation::Status`).
pub fn transaction_busy(phase: &str, sending: bool) -> bool {
    sending || matches!(phase, "preparing" | "preview" | "copying")
}

#[derive(Debug, Clone, PartialEq)]
pub enum DraftAction {
    Source(String),
    Pack(PackChoice),
    Continue,
    Open(UpdateStep),
}
/// Applies `action`; while a transaction is active (`locked`) the draft is frozen.
pub fn transition_draft(draft: UpdateDraft, action: DraftAction, locked: bool) -> UpdateDraft {
    if locked {
        return draft;
    }
    match action {
        DraftAction::Source(source) => change_source(draft, source),
        DraftAction::Pack(pack) => change_pack(draft, pack),
        DraftAction::Continue => continue_step(draft),
        DraftAction::Open(step) => open_step(draft, step),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtnh_core::{Conflict, ModAction};

    fn local(path: &str) -> PackChoice {
        PackChoice::Local(path.into())
    }
    fn ready() -> UpdateDraft {
        let d = continue_step(create_draft(Some("/old".into())));
        let d = continue_step(change_pack(d, local("/new.zip")));
        continue_step(d)
    }
    fn rule(path: &str, conflict: Conflict, custom: bool) -> Rule {
        Rule {
            path: path.into(),
            enabled: true,
            conflict,
            custom,
        }
    }

    #[test]
    fn forward_sections_stay_locked_until_explicit_completion_even_with_a_remembered_source() {
        let d = create_draft(Some("/old".into()));
        assert_eq!(open_step(d.clone(), UpdateStep::Transfer).step, UpdateStep::Source);
        assert_eq!(continue_step(create_draft(None)).completed, 0);
        assert_eq!(continue_step(d.clone()).step, UpdateStep::Pack);
        assert_eq!(
            continue_step(continue_step(d.clone())).step,
            UpdateStep::Pack
        );
        assert_eq!(build_install_request(&d, true, &[], &[], &[]), None);
        let url = PackChoice::Url("https://example.test/a.zip".into());
        assert_eq!(
            continue_step(change_pack(continue_step(d), url)).step,
            UpdateStep::Transfer
        );
    }

    #[test]
    fn opening_completed_sections_preserves_values_source_changes_invalidate_downstream() {
        let client = ready();
        let server = create_draft(Some("/server".into()));
        assert_eq!(
            open_step(client.clone(), UpdateStep::Source),
            UpdateDraft {
                step: UpdateStep::Source,
                ..client.clone()
            }
        );
        assert_eq!(
            change_source(client.clone(), "/other".into()),
            UpdateDraft {
                source: Some("/other".into()),
                pack: None,
                step: UpdateStep::Source,
                completed: 0
            }
        );
        // The same source keeps everything.
        assert_eq!(change_source(client.clone(), "/old".into()), client);
        assert_eq!(
            server,
            UpdateDraft {
                source: Some("/server".into()),
                pack: None,
                step: UpdateStep::Source,
                completed: 0
            }
        );
        assert_eq!(client.source.as_deref(), Some("/old"));
        let replacement = change_pack(client, local("/different"));
        assert_eq!(replacement.completed, 1);
        assert_eq!(replacement.step, UpdateStep::Pack);
        assert_eq!(build_install_request(&replacement, true, &[], &[], &[]), None);
    }

    #[test]
    fn request_captures_rules_deeply_only_after_transfer_step_is_explicitly_completed() {
        let mut rules = vec![rule("saves", Conflict::Replace, false)];
        let d = ready();
        let request = build_install_request(&d, true, &rules, &[], &[]);
        rules[0].enabled = false;
        rules.push(rule("options.txt", Conflict::Replace, false));
        assert_eq!(
            request,
            Some(PreparedRequest::Client(ClientRequest {
                pack: local("/new.zip"),
                source: "/old".into(),
                rules: vec![rule("saves", Conflict::Replace, false)],
                mods: vec![],
                renames: vec![],
            }))
        );
        let downloading = change_pack(d, PackChoice::Url("https://example.test/a.zip".into()));
        assert_eq!(build_install_request(&downloading, false, &[], &[], &[]), None);
    }

    #[test]
    fn active_transaction_freezes_draft_transitions_failures_and_cancelled_preview_unlock_them() {
        let d = ready();
        for phase in ["preparing", "preview", "copying"] {
            assert!(transaction_busy(phase, false));
            assert_eq!(
                transition_draft(d.clone(), DraftAction::Source("/other".into()), true),
                d
            );
            assert_eq!(
                transition_draft(d.clone(), DraftAction::Pack(local("/other.zip")), true),
                d
            );
        }
        assert!(transaction_busy("", true));
        for phase in ["", "failed", "completed"] {
            assert!(!transaction_busy(phase, false));
        }
        assert_eq!(
            transition_draft(d, DraftAction::Source("/other".into()), false).completed,
            0
        );
    }

    fn sample_mod() -> ModRule {
        ModRule {
            action: ModAction::Add,
            enabled: true,
            name: "X".into(),
            mod_ids: vec!["x".into()],
            stem: "x-1".into(),
            file: Some("/mods/x-1.jar".into()),
        }
    }

    #[test]
    fn server_request_carries_the_server_folder_rules_and_mods() {
        let d = ready();
        let rules = vec![rule("config/serverutilities.cfg", Conflict::Ask, true)];
        let mods = vec![sample_mod()];
        assert_eq!(
            build_install_request(&d, false, &rules, &mods, &[]),
            Some(PreparedRequest::Server(ServerRequest {
                pack: local("/new.zip"),
                server: "/old".into(),
                rules,
                mods,
            }))
        );
        assert_eq!(
            build_install_request(&d, false, &[], &[], &[]),
            Some(PreparedRequest::Server(ServerRequest {
                pack: local("/new.zip"),
                server: "/old".into(),
                rules: vec![],
                mods: vec![],
            }))
        );
        let url = change_pack(d, PackChoice::Url("https://example.test/s.zip".into()));
        let url = UpdateDraft { completed: 3, ..url };
        let Some(PreparedRequest::Server(server)) = build_install_request(&url, false, &[], &[], &[])
        else {
            panic!("expected a server request");
        };
        let request = server.into_update_request();
        assert_eq!(request.url.as_deref(), Some("https://example.test/s.zip"));
        assert_eq!(request.local_path, None);
        assert_eq!(request.server, "/old");
    }

    #[test]
    fn client_request_carries_a_deep_copy_of_mod_rules() {
        let mut mods = vec![ModRule {
            action: ModAction::Remove,
            enabled: true,
            name: "JM".into(),
            mod_ids: vec!["journeymap".into()],
            stem: "journeymap".into(),
            file: None,
        }];
        let expected = mods.clone();
        let Some(PreparedRequest::Client(request)) =
            build_install_request(&ready(), true, &[], &mods, &[])
        else {
            panic!("expected a client request");
        };
        mods[0].mod_ids.push("other".into());
        mods[0].enabled = false;
        assert_eq!(request.mods, expected);
    }

    #[test]
    fn client_request_carries_a_deep_copy_of_rename_rules_and_converts_to_the_backend_type() {
        let mut renames = vec![RenameRule {
            path: "config/a/ru_RU.lang".into(),
            to: "en_US.lang".into(),
            enabled: true,
        }];
        let expected = renames.clone();
        let Some(PreparedRequest::Client(request)) =
            build_install_request(&ready(), true, &[], &[], &renames)
        else {
            panic!("expected a client request");
        };
        renames[0].to = "x.lang".into();
        assert_eq!(request.renames, expected);
        let install = request.into_install_request("/instances".into(), "New".into());
        assert!(install.client);
        assert_eq!(install.url, None);
        assert_eq!(install.local_path, Some(PathBuf::from("/new.zip")));
        assert_eq!((install.parent.as_str(), install.name.as_str()), ("/instances", "New"));
        assert_eq!(install.renames, expected);
    }
}
