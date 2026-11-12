use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{
    div, prelude::*, px, relative, transparent_black, ClickEvent, Context, Div, Entity,
    FontWeight, IntoElement, Render, SharedString, Window,
};
use gpui_component::{
    checkbox::Checkbox,
    select::{SearchableVec, Select, SelectEvent, SelectState},
    Disableable as _, IndexPath,
};
use gtnh_core::{Language, Settings, Theme};

use crate::backend::{Backend, Status};
use crate::catalog::{CatalogEvent, CatalogProps, CatalogView};
use crate::flow::{
    build_install_request, create_draft, transition_draft, DraftAction, Mode, PackChoice,
    PreparedRequest, UpdateDraft, UpdateStep,
};
use crate::i18n::{messages, Messages};
use crate::installation_view::{stage_label, InstallEvent, InstallProps, InstallationView};
use crate::logic::{pack_name, resolve_prism_directory};
use crate::rules::{
    mods::ModRulesView, renames::renames_valid, renames::RenameRulesView,
    transfer::TransferRulesView, RulesEvent, RulesProps,
};
use crate::theme::{color, mono_font, palette, sync_component_theme, ui_font, Palette};
use crate::ui::{
    aligned, card, directory, field, muted_paragraph, paragraph, pre, primary_button,
    progress_bar, secondary_button, status_block,
};
use crate::update_section::{step_actions, update_section, SectionProps};

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Client,
    Server,
    Settings,
}

impl Tab {
    fn mode(self) -> Option<Mode> {
        match self {
            Tab::Client => Some(Mode::Client),
            Tab::Server => Some(Mode::Server),
            Tab::Settings => None,
        }
    }
    fn of(mode: Mode) -> Tab {
        match mode {
            Mode::Client => Tab::Client,
            Mode::Server => Tab::Server,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum PackMethod {
    Download,
    Local,
}

type Items = SearchableVec<SharedString>;
type LabelSelect = SelectState<Items>;

/// Everything the wizard keeps per mode (`drafts`, `expandedStep`, `packMethod`, `downloadSummary`
/// and the editor / catalog components of that mode).
struct ModeState {
    draft: UpdateDraft,
    expanded: Option<UpdateStep>,
    pack_method: Option<PackMethod>,
    download_summary: String,
    transfer: Entity<TransferRulesView>,
    mods: Entity<ModRulesView>,
    /// Client only.
    renames: Option<Entity<RenameRulesView>>,
    catalog: Entity<CatalogView>,
}

pub struct AppView {
    backend: Arc<Backend>,
    settings: Settings,
    /// Serialized settings as last written to (or read from) disk.
    saved: Option<serde_json::Value>,
    /// Serialized settings of the most recent save attempt; prevents retry loops after a failure.
    attempted: Option<serde_json::Value>,
    /// Prism folder of the last saved (or loaded) settings (`savedPrism`).
    saved_prism: Option<String>,
    loaded: bool,
    loading: bool,
    saving: bool,
    picking: bool,
    tab: Tab,
    last_mode: Mode,
    error: Option<String>,
    save_error: Option<String>,
    notice: Option<String>,
    dir: String,
    language_select: Entity<LabelSelect>,
    theme_select: Entity<LabelSelect>,
    /// Indexed like `Mode` (client, server).
    modes: [ModeState; 2],
    installation: Entity<InstallationView>,
    default_parent: Option<String>,
    install_busy: bool,
    install_status: Option<Arc<Status>>,
}

fn slot(mode: Mode) -> usize {
    match mode {
        Mode::Client => 0,
        Mode::Server => 1,
    }
}

fn theme_items(m: &Messages) -> Items {
    SearchableVec::new(vec![SharedString::from(m.amoled), SharedString::from(m.light)])
}

/// `resolvePrismDirectory(savedPrism, instances_directory)`.
async fn prism_parent(backend: Arc<Backend>, saved: Option<String>) -> Result<Option<String>, String> {
    resolve_prism_directory(saved.as_deref(), |path| backend.instances_directory(path)).await
}

impl AppView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = Settings::default();
        let m = messages(settings.language);
        let backend = Backend::get(cx);
        let language_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(vec![
                    SharedString::from("Українська"),
                    SharedString::from("English"),
                ]),
                Some(IndexPath::new(0)),
                window,
                cx,
            )
        });
        let theme_select =
            cx.new(|cx| SelectState::new(theme_items(m), Some(IndexPath::new(0)), window, cx));

        cx.subscribe_in(
            &language_select,
            window,
            |this, _, event: &SelectEvent<Items>, window, cx| {
                if let SelectEvent::Confirm(Some(value)) = event {
                    let language = if value.as_ref() == "English" {
                        Language::En
                    } else {
                        Language::Uk
                    };
                    this.set_language(language, window, cx);
                }
            },
        )
        .detach();
        cx.subscribe_in(
            &theme_select,
            window,
            |this, _, event: &SelectEvent<Items>, _window, cx| {
                if let SelectEvent::Confirm(Some(value)) = event {
                    let light = messages(this.settings.language).light;
                    let theme = if value.as_ref() == light {
                        Theme::Light
                    } else {
                        Theme::Amoled
                    };
                    this.set_theme(theme, cx);
                }
            },
        )
        .detach();

        sync_component_theme(settings.theme, cx);

        let installation = cx.new(|cx| {
            InstallationView::new(
                InstallProps { language: settings.language, default_parent: None },
                window,
                cx,
            )
        });
        cx.subscribe(&installation, |this, _, event: &InstallEvent, cx| {
            match event {
                InstallEvent::Busy(busy) => this.install_busy = *busy,
                InstallEvent::Status(status) => this.install_status = status.clone(),
                InstallEvent::Settings => this.switch_tab(Tab::Settings, cx),
                InstallEvent::RequestConsumed => {}
            }
            cx.notify();
        })
        .detach();

        let modes = [
            Self::new_mode_state(Mode::Client, window, cx),
            Self::new_mode_state(Mode::Server, window, cx),
        ];

        let mut view = Self {
            dir: backend.config_directory(),
            backend,
            settings,
            saved: None,
            attempted: None,
            saved_prism: None,
            loaded: false,
            loading: true,
            saving: false,
            picking: false,
            tab: Tab::Client,
            last_mode: Mode::Client,
            error: None,
            save_error: None,
            notice: None,
            language_select,
            theme_select,
            modes,
            installation,
            default_parent: None,
            install_busy: false,
            install_status: None,
        };
        view.load(window, cx);
        view
    }

    fn new_mode_state(mode: Mode, window: &mut Window, cx: &mut Context<Self>) -> ModeState {
        let transfer = cx.new(|cx| TransferRulesView::new(mode == Mode::Server, window, cx));
        let mods = cx.new(|cx| ModRulesView::new(window, cx));
        let renames = (mode == Mode::Client).then(|| cx.new(|cx| RenameRulesView::new(window, cx)));
        let catalog = cx.new(|cx| CatalogView::new(mode, window, cx));

        cx.subscribe(&transfer, move |this, editor, event: &RulesEvent, cx| match event {
            RulesEvent::Changed => {
                let rules = editor.read(cx).rules().to_vec();
                match mode {
                    Mode::Client => this.settings.client_rules = rules,
                    Mode::Server => this.settings.server_rules = rules,
                }
                this.rules_changed(mode, cx);
                this.changed(cx);
            }
            RulesEvent::Picking(picking) => this.set_picking(*picking, cx),
        })
        .detach();
        cx.subscribe(&mods, move |this, editor, event: &RulesEvent, cx| match event {
            RulesEvent::Changed => {
                let rules = editor.read(cx).rules().to_vec();
                match mode {
                    Mode::Client => this.settings.client_mods = rules,
                    Mode::Server => this.settings.server_mods = rules,
                }
                this.rules_changed(mode, cx);
                this.changed(cx);
            }
            RulesEvent::Picking(picking) => this.set_picking(*picking, cx),
        })
        .detach();
        if let Some(renames) = &renames {
            cx.subscribe(renames, |this, editor, event: &RulesEvent, cx| match event {
                RulesEvent::Changed => {
                    this.settings.client_renames = editor.read(cx).rules().to_vec();
                    this.rules_changed(Mode::Client, cx);
                    this.changed(cx);
                }
                RulesEvent::Picking(picking) => this.set_picking(*picking, cx),
            })
            .detach();
        }
        cx.subscribe(&catalog, move |this, _, event: &CatalogEvent, cx| match event {
            CatalogEvent::Install(artifact) => {
                let mode = if artifact.mode == "server" { Mode::Server } else { Mode::Client };
                this.select_download(mode, artifact.url.clone(), cx);
            }
            CatalogEvent::Summary(summary) => {
                this.modes[slot(mode)].download_summary = summary.clone();
                cx.notify();
            }
        })
        .detach();

        ModeState {
            draft: create_draft(None),
            expanded: Some(UpdateStep::Source),
            pack_method: None,
            download_summary: String::new(),
            transfer,
            mods,
            renames,
            catalog,
        }
    }

    fn set_picking(&mut self, picking: bool, cx: &mut Context<Self>) {
        self.picking = picking;
        cx.notify();
    }

    fn locked(&self) -> bool {
        !self.loaded || self.install_busy || self.picking
    }

    fn current_mode(&self) -> Mode {
        self.tab.mode().unwrap_or(self.last_mode)
    }

    fn rules_of(&self, mode: Mode) -> &[gtnh_core::Rule] {
        match mode {
            Mode::Client => &self.settings.client_rules,
            Mode::Server => &self.settings.server_rules,
        }
    }

    fn mods_of(&self, mode: Mode) -> &[gtnh_core::ModRule] {
        match mode {
            Mode::Client => &self.settings.client_mods,
            Mode::Server => &self.settings.server_mods,
        }
    }

    /// Pushes the loaded rules into the editors.
    fn sync_rules(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (rules, mods, renames) = (
            [self.settings.client_rules.clone(), self.settings.server_rules.clone()],
            [self.settings.client_mods.clone(), self.settings.server_mods.clone()],
            self.settings.client_renames.clone(),
        );
        for (index, (rules, mods)) in rules.into_iter().zip(mods).enumerate() {
            let state = &self.modes[index];
            state.transfer.update(cx, |view, cx| view.set_rules(rules, window, cx));
            state.mods.update(cx, |view, cx| view.set_rules(mods, window, cx));
        }
        if let Some(editor) = &self.modes[0].renames {
            editor.update(cx, |view, cx| view.set_rules(renames, window, cx));
        }
    }

    /// Pushes state-derived properties into the child entities (they notify only on change).
    fn sync_children(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let language = self.settings.language;
        let disabled = self.locked();
        for mode in [Mode::Client, Mode::Server] {
            let state = &self.modes[slot(mode)];
            let source = state.draft.source.clone().unwrap_or_default();
            let mods_directory = if source.is_empty() {
                String::new()
            } else {
                Path::new(&source).join("mods").to_string_lossy().into_owned()
            };
            let props = RulesProps { source, language, disabled };
            let mods_props = RulesProps { source: mods_directory, ..props.clone() };
            state.transfer.update(cx, |view, cx| view.set_props(&props, window, cx));
            state.mods.update(cx, |view, cx| view.set_props(&mods_props, window, cx));
            if let Some(editor) = &state.renames {
                editor.update(cx, |view, cx| view.set_props(&props, window, cx));
            }
            let catalog = CatalogProps {
                language,
                include_prereleases: self.settings.include_prereleases,
                can_install: !disabled && state.draft.completed >= 1,
            };
            state.catalog.update(cx, |view, cx| view.set_props(&catalog, window, cx));
        }
        let parent = (self.settings.prism_instances_directory == self.saved_prism)
            .then(|| self.default_parent.clone())
            .flatten();
        let props = InstallProps { language, default_parent: parent };
        self.installation.update(cx, |view, cx| view.set_props(&props, cx));
    }

    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.loading = true;
        self.loaded = false;
        self.error = None;
        self.notice = None;
        cx.notify();
        let backend = self.backend.clone();
        let task = cx.background_spawn(async move { backend.load_settings() });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                this.loading = false;
                match result {
                    Ok(settings) => {
                        let snapshot = serde_json::to_value(&settings).ok();
                        this.saved = snapshot.clone();
                        this.attempted = snapshot;
                        this.saved_prism = settings.prism_instances_directory.clone();
                        this.modes[0].draft = create_draft(settings.client_source.clone());
                        this.modes[1].draft = create_draft(settings.server_source.clone());
                        this.settings = settings;
                        this.loaded = true;
                        sync_component_theme(this.settings.theme, cx);
                        this.sync_selects(window, cx);
                        this.sync_rules(window, cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn sync_selects(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let m = messages(self.settings.language);
        let language_row = match self.settings.language {
            Language::Uk => 0,
            Language::En => 1,
        };
        let theme_row = match self.settings.theme {
            Theme::Amoled => 0,
            Theme::Light => 1,
        };
        self.language_select.update(cx, |state, cx| {
            state.set_selected_index(Some(IndexPath::new(language_row)), window, cx);
        });
        self.theme_select.update(cx, |state, cx| {
            state.set_items(theme_items(m), window, cx);
            state.set_selected_index(Some(IndexPath::new(theme_row)), window, cx);
        });
    }

    fn set_language(&mut self, language: Language, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings.language == language {
            return;
        }
        self.settings.language = language;
        self.sync_selects(window, cx);
        self.changed(cx);
    }

    fn set_theme(&mut self, theme: Theme, cx: &mut Context<Self>) {
        if self.settings.theme == theme {
            return;
        }
        self.settings.theme = theme;
        sync_component_theme(theme, cx);
        self.changed(cx);
    }

    /// Called after every user edit of the settings.
    fn changed(&mut self, cx: &mut Context<Self>) {
        self.notice = None;
        cx.notify();
        self.maybe_save(cx);
    }

    fn dirty(&self) -> bool {
        self.loaded && serde_json::to_value(&self.settings).ok() != self.saved
    }

    fn maybe_save(&mut self, cx: &mut Context<Self>) {
        // Rows with an invalid new name are shown inline; they are saved once corrected.
        if !self.loaded || self.saving || !renames_valid(&self.settings.client_renames) {
            return;
        }
        let Ok(snapshot) = serde_json::to_value(&self.settings) else {
            return;
        };
        if Some(&snapshot) == self.saved.as_ref() || Some(&snapshot) == self.attempted.as_ref() {
            return;
        }
        self.saving = true;
        self.save_error = None;
        self.notice = None;
        self.attempted = Some(snapshot.clone());
        let submitted_prism = self.settings.prism_instances_directory.clone();
        let backend = self.backend.clone();
        let settings = self.settings.clone();
        let task = cx.background_spawn(async move { backend.save_settings(settings).map(|_| ()) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.saving = false;
                match result {
                    Ok(()) => {
                        this.saved = Some(snapshot);
                        this.save_error = None;
                        this.notice = Some(messages(this.settings.language).saved.to_string());
                        this.after_save(submitted_prism, cx);
                    }
                    Err(error) => this.save_error = Some(error),
                }
                cx.notify();
                // Save the latest state if it changed while this save was running.
                this.maybe_save(cx);
            })
            .ok();
        })
        .detach();
    }

    /// The Prism folder changed on disk: re-resolve the default parent of a pending client
    /// request (`savePending` in App.svelte).
    fn after_save(&mut self, submitted_prism: Option<String>, cx: &mut Context<Self>) {
        let prism_changed = self.saved_prism != submitted_prism;
        self.saved_prism = submitted_prism;
        let request = self.installation.read(cx).request().cloned();
        let client_request = matches!(request, Some(PreparedRequest::Client(_)));
        if !(prism_changed && client_request && self.settings.prism_instances_directory == self.saved_prism)
        {
            return;
        }
        let prism = self.saved_prism.clone();
        self.default_parent = None;
        let backend = self.backend.clone();
        cx.spawn(async move |this, cx| {
            let result = prism_parent(backend, prism.clone()).await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(parent) => {
                        let current = this.installation.read(cx).request().cloned();
                        if current == request && this.settings.prism_instances_directory == prism {
                            this.default_parent = parent;
                        }
                    }
                    Err(error) => {
                        let m = messages(this.settings.language);
                        this.error = Some(format!("{}\n{}", m.prism_unavailable, error));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    // --- wizard ---------------------------------------------------------------------------

    fn set_install_request(&mut self, request: Option<PreparedRequest>, cx: &mut Context<Self>) {
        self.installation.update(cx, |view, cx| view.set_request(request, cx));
    }

    fn has_install_request(&self, cx: &Context<Self>) -> bool {
        self.installation.read(cx).request().is_some()
    }

    fn act(&mut self, mode: Mode, action: DraftAction) {
        let locked = self.locked();
        let state = &mut self.modes[slot(mode)];
        state.draft = transition_draft(state.draft.clone(), action, locked);
    }

    fn open_step(&mut self, mode: Mode, step: UpdateStep, cx: &mut Context<Self>) {
        if self.locked() || step.index() > self.modes[slot(mode)].draft.completed {
            return;
        }
        let collapsing = self.modes[slot(mode)].expanded == Some(step);
        self.act(mode, DraftAction::Open(step));
        self.modes[slot(mode)].expanded = if collapsing { None } else { Some(step) };
        cx.notify();
    }

    fn set_source(&mut self, mode: Mode, root: String) {
        let state = &mut self.modes[slot(mode)];
        state.draft = transition_draft(state.draft.clone(), DraftAction::Source(root.clone()), false);
        match mode {
            Mode::Client => self.settings.client_source = Some(root),
            Mode::Server => self.settings.server_source = Some(root),
        }
    }

    fn pick_source(&mut self, mode: Mode, cx: &mut Context<Self>) {
        if self.locked() {
            return;
        }
        self.picking = true;
        self.error = None;
        self.notice = None;
        cx.notify();
        let client = mode == Mode::Client;
        let backend = self.backend.clone();
        let m = messages(self.settings.language);
        let mut initial = self.modes[slot(mode)].draft.source.clone().map(PathBuf::from);
        let saved_prism = self.saved_prism.clone();
        let title = if client { m.old_build } else { m.server_folder };
        cx.spawn(async move |this, cx| {
            if client && saved_prism.is_some() {
                match prism_parent(backend.clone(), saved_prism).await {
                    Ok(parent) => initial = parent.map(PathBuf::from),
                    Err(error) => {
                        initial = None;
                        this.update(cx, |this, cx| {
                            this.error = Some(format!("{}\n{}", m.prism_unavailable, error));
                            cx.notify();
                        })
                        .ok();
                    }
                }
            }
            let mut dialog = rfd::AsyncFileDialog::new().set_title(title);
            if let Some(directory) = initial {
                dialog = dialog.set_directory(directory);
            }
            let picked = dialog.pick_folder().await;
            let root = match picked {
                Some(handle) => Some(
                    backend
                        .source_directory(handle.path().to_string_lossy().into_owned(), client)
                        .await,
                ),
                None => None,
            };
            this.update(cx, |this, cx| {
                match root {
                    Some(Ok(root)) => {
                        this.set_source(mode, root);
                        this.set_install_request(None, cx);
                        this.changed(cx);
                    }
                    Some(Err(error)) => this.error = Some(error),
                    None => {}
                }
                this.picking = false;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn pick_local(&mut self, mode: Mode, zip: bool, cx: &mut Context<Self>) {
        if self.locked() || self.modes[slot(mode)].draft.completed < 1 {
            return;
        }
        self.picking = true;
        self.error = None;
        cx.notify();
        let m = messages(self.settings.language);
        let title = if zip { m.choose_zip } else { m.choose_folder };
        cx.spawn(async move |this, cx| {
            let mut dialog = rfd::AsyncFileDialog::new().set_title(title);
            let picked = if zip {
                dialog = dialog.add_filter("ZIP", &["zip"]);
                dialog.pick_file().await
            } else {
                dialog.pick_folder().await
            };
            this.update(cx, |this, cx| {
                if let Some(handle) = picked {
                    let path = handle.path().to_string_lossy().into_owned();
                    this.act(mode, DraftAction::Pack(PackChoice::Local(path)));
                    // Picking is not "locked" for the transition itself (see `pick_local` in Svelte).
                    this.set_install_request(None, cx);
                }
                this.picking = false;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn select_download(&mut self, mode: Mode, url: String, cx: &mut Context<Self>) {
        if self.locked() {
            return;
        }
        self.act(mode, DraftAction::Pack(PackChoice::Url(url)));
        self.set_install_request(None, cx);
        cx.notify();
    }

    fn continue_draft(&mut self, mode: Mode, cx: &mut Context<Self>) {
        if self.locked() {
            return;
        }
        self.picking = true;
        self.error = None;
        cx.notify();
        let client = mode == Mode::Client;
        let backend = self.backend.clone();
        let draft = self.modes[slot(mode)].draft.clone();
        let saved_prism = self.saved_prism.clone();
        cx.spawn(async move |this, cx| {
            if draft.step == UpdateStep::Source {
                if let Some(source) = draft.source.clone() {
                    match backend.source_directory(source, client).await {
                        Ok(root) => {
                            this.update(cx, |this, cx| {
                                this.set_source(mode, root);
                                cx.notify();
                            })
                            .ok();
                        }
                        Err(error) => {
                            this.update(cx, |this, cx| {
                                this.error = Some(error);
                                this.picking = false;
                                cx.notify();
                            })
                            .ok();
                            return;
                        }
                    }
                }
            }
            let needs_parent = this
                .update(cx, |this, cx| {
                    this.modes[slot(mode)].draft.step == UpdateStep::Transfer
                        && !this.has_install_request(cx)
                })
                .unwrap_or(false);
            let parent = if needs_parent && client {
                Some(prism_parent(backend, saved_prism).await)
            } else {
                None
            };
            this.update(cx, |this, cx| {
                if needs_parent {
                    this.default_parent = None;
                }
                match parent {
                    Some(Ok(parent)) => this.default_parent = parent,
                    Some(Err(error)) => {
                        let m = messages(this.settings.language);
                        this.error = Some(format!("{}\n{}", m.prism_unavailable, error));
                    }
                    None => {}
                }
                this.act_unlocked(mode, DraftAction::Continue);
                let step = this.modes[slot(mode)].draft.step;
                this.modes[slot(mode)].expanded = Some(step);
                if step == UpdateStep::Review && !this.has_install_request(cx) {
                    this.set_request(mode, cx);
                }
                this.picking = false;
                this.changed(cx);
            })
            .ok();
        })
        .detach();
    }

    /// `transitionDraft(..., false)`.
    fn act_unlocked(&mut self, mode: Mode, action: DraftAction) {
        let state = &mut self.modes[slot(mode)];
        state.draft = transition_draft(state.draft.clone(), action, false);
    }

    fn continue_from_review(&mut self, cx: &mut Context<Self>) {
        if self.locked() {
            return;
        }
        let mode = self.current_mode();
        let state = &mut self.modes[slot(mode)];
        state.draft = UpdateDraft { step: UpdateStep::Transfer, completed: 2, ..state.draft.clone() };
        self.continue_draft(mode, cx);
    }

    fn set_request(&mut self, mode: Mode, cx: &mut Context<Self>) {
        let client = mode == Mode::Client;
        let renames: &[gtnh_core::RenameRule] =
            if client { &self.settings.client_renames } else { &[] };
        let request = build_install_request(
            &self.modes[slot(mode)].draft,
            client,
            self.rules_of(mode),
            self.mods_of(mode),
            renames,
        );
        self.set_install_request(request, cx);
    }

    fn rules_changed(&mut self, mode: Mode, cx: &mut Context<Self>) {
        if self.install_busy {
            return;
        }
        let state = &mut self.modes[slot(mode)];
        state.draft = UpdateDraft {
            completed: state.draft.completed.min(2),
            step: UpdateStep::Transfer,
            ..state.draft.clone()
        };
        state.expanded = Some(UpdateStep::Transfer);
        self.set_install_request(None, cx);
    }

    fn switch_tab(&mut self, next: Tab, cx: &mut Context<Self>) {
        let other_mode = next.mode().is_some_and(|mode| mode != self.last_mode);
        if self.picking || (self.install_busy && other_mode) {
            return;
        }
        if other_mode {
            self.set_install_request(None, cx);
            self.default_parent = None;
            // An already completed draft returns to its transfer step to rebuild its request/destination.
            if let Some(mode) = next.mode() {
                let state = &mut self.modes[slot(mode)];
                if state.draft.completed == 3 {
                    state.draft = UpdateDraft {
                        step: UpdateStep::Transfer,
                        completed: 2,
                        ..state.draft.clone()
                    };
                    state.expanded = Some(UpdateStep::Transfer);
                }
            }
        }
        self.tab = next;
        if let Some(mode) = next.mode() {
            self.last_mode = mode;
        }
        self.notice = None;
        cx.notify();
    }

    fn transfer_summary(&self, mode: Mode, m: &Messages) -> String {
        let counts = format!(
            "{}: {} · {}: {}",
            m.enabled_rules,
            self.rules_of(mode).iter().filter(|rule| rule.enabled).count(),
            m.mods_enabled,
            self.mods_of(mode).iter().filter(|rule| rule.enabled).count(),
        );
        match mode {
            Mode::Client => counts,
            Mode::Server => format!("{} · {counts}", m.server_step_summary),
        }
    }

    fn pack_summary(&self, mode: Mode, m: &Messages) -> String {
        let state = &self.modes[slot(mode)];
        let selected = match &state.draft.pack {
            Some(pack) => pack_name(pack.label()),
            None => m.pack_not_selected.to_string(),
        };
        [selected, state.download_summary.clone()]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" · ")
    }

    // --- settings tab ---------------------------------------------------------------------

    fn pick_prism_directory(&mut self, cx: &mut Context<Self>) {
        if self.locked() {
            return;
        }
        self.picking = true;
        self.error = None;
        self.notice = None;
        cx.notify();
        let backend = self.backend.clone();
        let m = messages(self.settings.language);
        let current = self.settings.prism_instances_directory.clone();
        cx.spawn(async move |this, cx| {
            let mut initial: Option<PathBuf> = None;
            if let Some(current) = current {
                match backend.instances_directory(current).await {
                    Ok(path) => initial = Some(PathBuf::from(path)),
                    Err(error) => {
                        this.update(cx, |this, cx| {
                            this.error = Some(format!("{}\n{}", m.prism_unavailable, error));
                            cx.notify();
                        })
                        .ok();
                    }
                }
            }
            let mut dialog = rfd::AsyncFileDialog::new().set_title(m.prism_instances);
            if let Some(directory) = initial {
                dialog = dialog.set_directory(directory);
            }
            let picked = dialog.pick_folder().await;
            let resolved = match picked {
                Some(handle) => Some(
                    backend
                        .instances_directory(handle.path().to_string_lossy().into_owned())
                        .await,
                ),
                None => None,
            };
            this.update(cx, |this, cx| {
                match resolved {
                    Some(Ok(path)) => {
                        this.settings.prism_instances_directory = Some(path);
                        this.changed(cx);
                    }
                    Some(Err(error)) => this.error = Some(error),
                    None => {}
                }
                this.picking = false;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn clear_prism_directory(&mut self, cx: &mut Context<Self>) {
        self.settings.prism_instances_directory = None;
        self.changed(cx);
    }

    fn open_storage_directory(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = self.backend.open_config_directory() {
            self.error = Some(error);
        }
        cx.notify();
    }

    fn set_beta(&mut self, value: bool, cx: &mut Context<Self>) {
        if self.settings.include_prereleases == value {
            return;
        }
        self.settings.include_prereleases = value;
        self.changed(cx);
    }
}

impl Render for AppView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(self.settings.theme);
        let m = messages(self.settings.language);
        self.sync_children(window, cx);
        let width = window.viewport_size().width;
        let narrow = width < px(900.);
        let very_narrow = width < px(740.);

        div()
            .size_full()
            .flex()
            .flex_row()
            .bg(color(p.bg))
            .text_color(color(p.text))
            .text_size(px(14.))
            .line_height(relative(1.2))
            .font_family(ui_font())
            .child(self.render_sidebar(p, m, narrow, cx))
            .child(self.render_main(p, m, narrow, very_narrow, cx))
    }
}

impl AppView {
    fn render_sidebar(
        &self,
        p: &'static Palette,
        m: &'static Messages,
        narrow: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let items: [(Tab, &str, &str); 3] = [
            (Tab::Client, "◈", m.client),
            (Tab::Server, "▤", m.server),
            (Tab::Settings, "⚙", m.settings),
        ];
        let muted = color(p.muted);
        let nav = items.into_iter().enumerate().fold(
            div().flex().flex_col().gap(px(6.)),
            |nav, (index, (tab, symbol, label))| {
                let active = self.tab == tab;
                let blocked = self.picking
                    || (self.install_busy
                        && tab.mode().is_some_and(|mode| mode != self.last_mode));
                let button = div()
                    .id(("nav", index))
                    .flex()
                    .items_center()
                    .px(px(12.))
                    .py(px(13.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(if active {
                        color(p.border)
                    } else {
                        transparent_black()
                    })
                    .bg(if active {
                        color(p.info)
                    } else {
                        transparent_black()
                    })
                    .text_color(if active { color(p.accent) } else { muted })
                    .child(div().w(px(27.)).text_size(px(18.)).child(symbol))
                    .child(label);
                nav.child(if blocked {
                    button.opacity(0.45)
                } else {
                    button
                        .cursor_pointer()
                        .hover(move |style| style.border_color(muted))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.switch_tab(tab, cx)
                        }))
                })
            },
        );

        div()
            .w(px(if narrow { 180. } else { 220. }))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(color(p.border))
            .px(px(if narrow { 12. } else { 20. }))
            .py(px(if narrow { 25. } else { 30. }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        div()
                            .w(px(36.))
                            .h(px(40.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(color(p.accent))
                            .text_color(color(p.accent_text))
                            .font_weight(FontWeight::BLACK)
                            .rounded(px(10.))
                            .text_size(px(24.))
                            .child("G"),
                    )
                    .child(
                        div()
                            .flex()
                            .whitespace_nowrap()
                            .text_size(px(if narrow { 15. } else { 17. }))
                            .child(div().font_weight(FontWeight::BOLD).child("GTNH"))
                            .child(
                                div()
                                    .pl(px(4.))
                                    .font_weight(FontWeight::NORMAL)
                                    .child("Updater"),
                            ),
                    ),
            )
            .child(
                paragraph(m.workspace.to_uppercase())
                    .text_size(px(10.))
                    .text_color(muted)
                    .mt(px(48.))
                    .mx(px(12.))
                    .mb(px(12.)),
            )
            .child(nav)
            .child(
                div()
                    .mt_auto()
                    .text_color(muted)
                    .text_size(px(11.))
                    .px(px(10.))
                    .py(px(20.))
                    .child(format!("v{}", env!("CARGO_PKG_VERSION"))),
            )
    }

    fn render_main(
        &self,
        p: &'static Palette,
        m: &'static Messages,
        narrow: bool,
        very_narrow: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let settings_tab = self.tab == Tab::Settings;
        let mode_label = if self.tab == Tab::Client { m.client } else { m.server };
        let eyebrow = if settings_tab {
            m.settings.to_string()
        } else {
            format!("{} · {}", m.update_build, mode_label)
        };
        let heading = if settings_tab { m.settings } else { m.update_build };

        let mut content = div()
            .w_full()
            .max_w(px(1120.))
            .mx_auto()
            .px(px(if narrow { 24. } else { 36. }))
            .pt(px(if narrow { 28. } else { 38. }))
            .pb(px(if narrow { 28. } else { 30. }))
            .child(
                div()
                    .pb(px(24.))
                    .child(
                        paragraph(eyebrow.to_uppercase())
                            .text_color(color(p.accent))
                            .text_size(px(10.))
                            .mb(px(12.)),
                    )
                    .child(
                        div()
                            .text_size(px(if very_narrow { 25. } else { 30. }))
                            .font_weight(FontWeight::SEMIBOLD)
                            .line_height(relative(1.2))
                            .mb(px(14.))
                            .child(heading),
                    ),
            );

        if self.loading {
            content = content.child(paragraph(m.loading).mb(px(14.)));
        }
        if let Some(error) = &self.error {
            content = content.child(self.error_block(
                p,
                if self.loaded { m.error } else { m.load_error },
                error.clone(),
                !self.loaded,
                m,
                cx,
            ));
        }
        if let Some(error) = &self.save_error {
            content = content.child(self.error_block(p, m.error, error.clone(), false, m, cx));
        }
        if settings_tab {
            if let Some(block) = self.render_install_status(p, m, cx) {
                content = content.child(block);
            }
        }

        if settings_tab {
            content = content.child(self.render_settings(p, m, narrow, cx));
        } else if self.loaded {
            content = content.child(self.render_wizard(p, m, very_narrow, cx));
        }

        let footer_text: String = if self.saving {
            m.saving.to_string()
        } else if self.dirty() {
            m.unsaved.to_string()
        } else if let Some(notice) = &self.notice {
            notice.clone()
        } else if self.loaded {
            m.clean.to_string()
        } else {
            String::new()
        };
        content = content.child(
            div()
                .border_t_1()
                .border_color(color(p.border))
                .pt(px(20.))
                .mt(px(24.))
                .text_size(px(12.))
                .text_color(color(p.muted))
                .child(footer_text),
        );

        div()
            .id("main")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .child(content)
    }

    fn error_block(
        &self,
        p: &'static Palette,
        title: &'static str,
        message: String,
        retry: bool,
        m: &'static Messages,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut block = status_block(p, p.danger)
            .child(div().font_weight(FontWeight::BOLD).child(title))
            .child(
                div()
                    .font_family(mono_font())
                    .text_size(px(12.))
                    .my(px(12.))
                    .child(message),
            );
        if retry {
            block = block.child(div().flex().flex_row().child(
                secondary_button("retry", m.retry, p, false).on_click(cx.listener(
                    |this, _: &ClickEvent, window, cx| this.load(window, cx),
                )),
            ));
        }
        block
    }

    /// The installation block of the Settings tab (progress, completed / failed texts).
    fn render_install_status(
        &self,
        p: &'static Palette,
        m: &'static Messages,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let status = self.install_status.as_deref();
        let phase = status.map_or("", |s| s.phase.as_str());
        if !self.install_busy && phase.is_empty() {
            return None;
        }
        let failed = phase == "failed";
        let client = status.is_none_or(|s| s.client);
        let title = if self.install_busy {
            m.install_in_progress
        } else if phase == "completed" {
            if client {
                m.install_complete
            } else {
                m.server_updated
            }
        } else if !client {
            m.server_updated_failed
        } else {
            m.install_failed
        };
        let mut block = status_block(p, if failed { p.danger } else { p.muted })
            .child(div().font_weight(FontWeight::BOLD).child(title));
        match status.and_then(|s| s.progress.as_ref()) {
            Some(progress) if self.install_busy => {
                block = block
                    .child(
                        paragraph(format!(
                            "{} · {} / {}",
                            stage_label(m, progress.stage),
                            progress.done,
                            progress.total
                        ))
                        .mb(px(14.)),
                    )
                    .child(progress_bar(p, progress.done, progress.total).mb(px(14.)));
            }
            _ if phase == "preview" => {
                block = block.child(paragraph(m.transfer_preview).mb(px(14.)));
            }
            _ => {}
        }
        if phase == "completed" {
            block = block.child(directory(
                p,
                status.and_then(|s| s.destination.clone()).unwrap_or_default(),
            ));
        }
        if failed {
            block = block.child(pre(status.and_then(|s| s.error.clone()).unwrap_or_default()));
        }
        let back = secondary_button("install-back", m.back_to_update, p, false)
            .when(!failed, |button| button.ml(px(10.)))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.switch_tab(Tab::of(this.last_mode), cx)
            }));
        Some(block.child(aligned(back)))
    }

    // --- wizard rendering -----------------------------------------------------------------

    fn toggle_handler(
        cx: &mut Context<Self>,
        mode: Mode,
        step: UpdateStep,
    ) -> impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static + use<> {
        let this = cx.weak_entity();
        move |_, _, cx| {
            this.update(cx, |this, cx| this.open_step(mode, step, cx)).ok();
        }
    }

    fn continue_button(
        &self,
        id: (&'static str, usize),
        mode: Mode,
        disabled: bool,
        p: &'static Palette,
        m: &'static Messages,
        cx: &mut Context<Self>,
    ) -> Div {
        step_actions(p).child(
            primary_button(id, format!("{} →", m.r#continue), p, disabled)
                .flex_shrink_0()
                .when(!disabled, |button| {
                    button.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.continue_draft(mode, cx)
                    }))
                }),
        )
    }

    fn render_wizard(
        &self,
        p: &'static Palette,
        m: &'static Messages,
        very_narrow: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let mode = self.current_mode();
        let index = slot(mode);
        let state = &self.modes[index];
        let locked = self.locked();
        let draft = &state.draft;
        let client = mode == Mode::Client;
        let section = |step: UpdateStep| -> (ElementIdOwned, bool) {
            (ElementIdOwned(format!("{}-step-{}", mode.as_str(), step.index())), state.expanded == Some(step))
        };

        // 1. Old build / server folder.
        let (id, expanded) = section(UpdateStep::Source);
        let mut source = div().flex().flex_col();
        if !client {
            source = source.child(muted_paragraph(p, m.server_folder_hint).mb(px(14.)));
        }
        if let Some(path) = &draft.source {
            source = source.child(directory(p, path.clone()));
        }
        source = source
            .child(aligned(
                secondary_button(("pick-source", index), format!("{}…", m.select_source), p, locked)
                    .when(!locked, |button| {
                        button.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.pick_source(mode, cx)
                        }))
                    }),
            ))
            .child(self.continue_button(
                ("continue-source", index),
                mode,
                locked || draft.source.is_none(),
                p,
                m,
                cx,
            ));
        let source_section = update_section(
            p,
            SectionProps {
                id: id.into(),
                number: 1,
                title: if client { m.old_build } else { m.server_folder }.into(),
                summary: draft.source.clone().unwrap_or_else(|| m.select_first.to_string()),
                expanded,
                locked,
                locked_reason: None,
                very_narrow,
            },
            Self::toggle_handler(cx, mode, UpdateStep::Source),
            expanded.then_some(source),
        );

        // 2. New build.
        let (id, expanded) = section(UpdateStep::Pack);
        let pack_locked = locked || draft.completed < 1;
        let method_button = |kind: PackMethod, name: &'static str, symbol: &'static str, text: &'static str, cx: &mut Context<Self>| {
            let selected = state.pack_method == Some(kind);
            let button = div()
                .id((name, index))
                .flex_1()
                .min_w_0()
                .p(px(22.))
                .rounded(px(8.))
                .border_1()
                .border_color(if selected { color(p.accent) } else { color(p.border) })
                .bg(if selected { color(p.info) } else { color(p.raised) })
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(16.))
                        .mb(px(8.))
                        .child(format!("{symbol} {}", name_label(m, kind))),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .line_height(relative(1.8))
                        .text_color(color(p.muted))
                        .child(text),
                );
            if locked {
                button.opacity(0.45)
            } else {
                let muted = color(p.muted);
                button
                    .cursor_pointer()
                    .hover(move |style| style.border_color(muted))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.modes[index].pack_method = Some(kind);
                        cx.notify();
                    }))
            }
        };
        let mut methods = div().flex().gap(px(14.));
        methods = if very_narrow { methods.flex_col() } else { methods.flex_row() };
        methods = methods
            .child(method_button(PackMethod::Download, "pack-download", "↓", m.download_choice_hint, cx))
            .child(method_button(PackMethod::Local, "pack-local", "▤", m.local_choice_hint, cx));
        let mut pack = div().flex().flex_col().child(methods);
        match state.pack_method {
            Some(PackMethod::Download) => {
                pack = pack.child(div().mt(px(20.)).child(state.catalog.clone()));
            }
            Some(PackMethod::Local) => {
                let picker = |id: &'static str, label: String, zip: bool, cx: &mut Context<Self>| {
                    let disabled = locked;
                    secondary_button((id, index), label, p, disabled).when(!disabled, |button| {
                        button.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.pick_local(mode, zip, cx)
                        }))
                    })
                };
                pack = pack.child(
                    div()
                        .mt(px(20.))
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .gap(px(10.))
                        .child(picker("pick-zip", format!("{}…", m.choose_zip), true, cx))
                        .child(picker("pick-folder", format!("{}…", m.choose_folder), false, cx)),
                );
            }
            None => {}
        }
        if let Some(choice) = &draft.pack {
            pack = pack.child(directory(p, choice.label().to_string()));
        }
        pack = pack.child(self.continue_button(
            ("continue-pack", index),
            mode,
            locked || draft.pack.is_none(),
            p,
            m,
            cx,
        ));
        let pack_section = update_section(
            p,
            SectionProps {
                id: id.into(),
                number: 2,
                title: m.new_build.into(),
                summary: self.pack_summary(mode, m),
                expanded,
                locked: pack_locked,
                locked_reason: (draft.completed < 1).then_some(m.complete_previous),
                very_narrow,
            },
            Self::toggle_handler(cx, mode, UpdateStep::Pack),
            expanded.then_some(pack),
        );

        // 3. What to transfer / server changes.
        let (id, expanded) = section(UpdateStep::Transfer);
        let transfer_locked = locked || draft.completed < 2;
        let mut transfer = div().flex().flex_col();
        if client {
            transfer = transfer.child(state.transfer.clone()).child(div().mt(px(24.)).child(state.mods.clone()));
            if let Some(renames) = &state.renames {
                transfer = transfer.child(div().mt(px(24.)).child(renames.clone()));
            }
        } else {
            let mut info = div().flex().flex_col().text_size(px(13.)).line_height(relative(1.7));
            for text in [
                m.server_replaced_info,
                m.server_kept_info,
                m.server_journey_info,
                m.server_config_info,
                m.server_mods_info,
            ] {
                info = info.child(
                    div().flex().flex_row().mb(px(8.)).child(div().w(px(20.)).pl(px(6.)).child("•")).child(div().flex_1().min_w_0().child(text)),
                );
            }
            transfer = transfer
                .child(info)
                .child(div().mt(px(24.)).child(state.transfer.clone()))
                .child(div().mt(px(24.)).child(state.mods.clone()));
        }
        transfer = transfer.child(self.continue_button(
            ("continue-transfer", index),
            mode,
            locked || draft.pack.is_none(),
            p,
            m,
            cx,
        ));
        let transfer_section = update_section(
            p,
            SectionProps {
                id: id.into(),
                number: 3,
                title: if client { m.what_to_transfer } else { m.server_changes }.into(),
                summary: self.transfer_summary(mode, m),
                expanded,
                locked: transfer_locked,
                locked_reason: (draft.completed < 2).then_some(m.complete_previous),
                very_narrow,
            },
            Self::toggle_handler(cx, mode, UpdateStep::Transfer),
            expanded.then_some(transfer),
        );

        // 4. Review and install.
        let (id, expanded) = section(UpdateStep::Review);
        let review_locked = locked || draft.completed < 3;
        let enabled_rules = self.rules_of(mode).iter().filter(|rule| rule.enabled).count();
        let enabled_mods = self.mods_of(mode).iter().filter(|rule| rule.enabled).count();
        let rows: [(&str, String); 4] = [
            (
                if client { m.install_source } else { m.server_folder },
                draft.source.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| "—".into()),
            ),
            (m.new_build, self.pack_summary(mode, m)),
            (m.enabled_rules, enabled_rules.to_string()),
            (m.mods_enabled, enabled_mods.to_string()),
        ];
        let mut summary = div().flex().flex_col().text_size(px(12.)).mb(px(20.));
        for (label, value) in rows {
            let row = div()
                .flex()
                .gap(px(if very_narrow { 5. } else { 16. }))
                .py(px(12.))
                .border_b_1()
                .border_color(color(p.border));
            let row = if very_narrow { row.flex_col() } else { row.flex_row() };
            summary = summary.child(
                row.child(div().w(px(110.)).flex_shrink_0().text_color(color(p.muted)).child(label.to_string()))
                    .child(div().flex_1().min_w_0().child(value)),
            );
        }
        let mut review = div().flex().flex_col().child(summary).child(self.installation.clone());
        if !self.has_install_request(cx)
            && !self.install_busy
            && draft.completed == 3
            && self.install_status.as_ref().is_none_or(|s| s.phase != "completed")
        {
            review = review.child(
                div().mt(px(16.)).child(aligned(
                    primary_button("retry-preparation", m.retry_preparation, p, locked).when(
                        !locked,
                        |button| {
                            button.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.continue_from_review(cx)
                            }))
                        },
                    ),
                )),
            );
        }
        let review_section = update_section(
            p,
            SectionProps {
                id: id.into(),
                number: 4,
                title: m.review_install.into(),
                summary: if self.install_busy {
                    m.install_in_progress.to_string()
                } else if client {
                    m.review_hint.to_string()
                } else {
                    m.server_review_hint.to_string()
                },
                expanded,
                locked: review_locked,
                locked_reason: (draft.completed < 3).then_some(m.complete_previous),
                very_narrow,
            },
            Self::toggle_handler(cx, mode, UpdateStep::Review),
            expanded.then_some(review),
        );

        div()
            .child(source_section)
            .child(pack_section)
            .child(transfer_section)
            .child(review_section)
    }

    fn render_settings(
        &self,
        p: &'static Palette,
        m: &'static Messages,
        narrow: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let disabled = !self.loaded || self.picking;
        let prism_disabled = disabled || self.install_busy;
        let prism = self.settings.prism_instances_directory.clone();
        let select_style = |select: Select<Items>| {
            select
                .disabled(disabled)
                .bg(color(p.raised))
                .border_color(color(p.border))
                .text_color(color(p.text))
                .rounded(px(8.))
                .h(px(37.))
                .px(px(12.))
                .py(px(10.))
                .text_size(px(12.))
        };

        let appearance = card(p, narrow, m.appearance).child(
            div()
                .flex()
                .gap(px(22.))
                .child(field(m.language).child(select_style(Select::new(&self.language_select))))
                .child(field(m.theme).child(select_style(Select::new(&self.theme_select)))),
        );

        let beta = self.settings.include_prereleases;
        let channel = card(p, narrow, m.channel)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .mb(px(10.))
                    .child(
                        Checkbox::new("beta")
                            .checked(beta)
                            .disabled(disabled)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.set_beta(*checked, cx)
                            })),
                    )
                    .child({
                        let label = div()
                            .id("beta-label")
                            .font_weight(FontWeight::MEDIUM)
                            .child(m.beta);
                        if disabled {
                            label
                        } else {
                            label.cursor_pointer().on_click(cx.listener(
                                move |this, _: &ClickEvent, _, cx| this.set_beta(!beta, cx),
                            ))
                        }
                    }),
            )
            .child(muted_paragraph(p, m.beta_hint));

        let prism_card = card(p, narrow, m.prism_instances)
            .child(muted_paragraph(p, m.prism_instances_hint).mb(px(0.)))
            .child(directory(
                p,
                prism.clone().unwrap_or_else(|| "—".to_string()),
            ))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(10.))
                    .child(
                        secondary_button(
                            "prism-select",
                            format!("{}…", m.select_source),
                            p,
                            prism_disabled,
                        )
                        .when(!prism_disabled, |button| {
                            button.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.pick_prism_directory(cx)
                            }))
                        }),
                    )
                    .child(
                        secondary_button(
                            "prism-clear",
                            m.clear,
                            p,
                            prism_disabled || prism.is_none(),
                        )
                        .when(!prism_disabled && prism.is_some(), |button| {
                            button.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.clear_prism_directory(cx)
                            }))
                        }),
                    ),
            );

        let storage = card(p, narrow, m.storage)
            .child(muted_paragraph(p, m.storage_hint).mb(px(0.)))
            .child(directory(p, self.dir.clone()))
            .child(
                div().flex().flex_row().child(
                    secondary_button("storage-open", format!("{} ↗", m.open), p, disabled)
                        .when(!disabled, |button| {
                            button.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.open_storage_directory(cx)
                            }))
                        }),
                ),
            );

        div()
            .child(appearance)
            .child(channel)
            .child(prism_card)
            .child(storage)
    }
}

/// Owned element id text for a wizard section.
struct ElementIdOwned(String);

impl From<ElementIdOwned> for gpui::ElementId {
    fn from(id: ElementIdOwned) -> Self {
        gpui::ElementId::from(SharedString::from(id.0))
    }
}

fn name_label(m: &Messages, kind: PackMethod) -> &'static str {
    match kind {
        PackMethod::Download => m.download,
        PackMethod::Local => m.already_downloaded,
    }
}
