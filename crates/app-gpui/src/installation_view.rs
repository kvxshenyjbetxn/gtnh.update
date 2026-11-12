//! Port of `Installation.svelte`: prepare, preview, confirm and cancel one installation.
//!
//! The parent hands over a [`PreparedRequest`] with `set_request`; the entity polls the backend
//! status (every 500 ms, forever, like the Svelte `onMount` watcher) and reports `busy`, status and
//! "open settings" through [`InstallEvent`].

use std::{collections::HashMap, sync::Arc, time::Duration};

use gpui::{
    div, prelude::*, px, ClickEvent, Context, Div, ElementId, Entity, EventEmitter, FontWeight,
    IntoElement, Render, SharedString, Window,
};
use gpui_component::{
    input::{Input, InputEvent, InputState},
    select::{SearchableVec, Select, SelectEvent, SelectState},
    IndexPath,
};
use gtnh_core::{
    installation::{Decision, Transfer},
    mods::ModPlan,
    server_update::{JavaChange, Recovery, ServerItem},
    Conflict, Language,
};

use crate::backend::{Backend, Status};
use crate::flow::{transaction_busy, PreparedRequest};
use crate::i18n::{messages, Messages};
use crate::logic::{
    format_bytes, group_transfers, pack_name, unresolved_conflicts, InstallWatcher, TransferGroup,
};
use crate::rules::{code, rule_row};
use crate::theme::{color, current_palette, mono_font};
use crate::theme::Palette;
use crate::ui::{aligned, directory, muted_paragraph, paragraph, pre, primary_button, progress_bar, secondary_button, status_block};

type Items = SearchableVec<SharedString>;
type LabelSelect = SelectState<Items>;

const CONFLICTS: [Conflict; 3] = [Conflict::Ask, Conflict::Replace, Conflict::Keep];
const FIRST_PAGE: usize = 200;
const PAGE: usize = 500;

#[derive(Clone, PartialEq)]
pub struct InstallProps {
    pub language: Language,
    /// Prism instances folder the new instance is created in (client installs).
    pub default_parent: Option<String>,
}

pub enum InstallEvent {
    /// `transactionBusy(status.phase, sending)` changed.
    Busy(bool),
    /// A newer status was accepted (`onStatus`).
    Status(Option<Arc<Status>>),
    /// "Configure Prism" was pressed (`onSettings`).
    Settings,
    /// The request was consumed by preparing or cancelling (Svelte clears the bound `request`).
    RequestConsumed,
}

#[derive(Default)]
struct Derived {
    groups: Vec<TransferGroup>,
    files: usize,
    folders: usize,
    /// Existing regular files with the `ask` policy.
    asks: Vec<Transfer>,
}

pub struct InstallationView {
    backend: Arc<Backend>,
    props: InstallProps,
    request: Option<PreparedRequest>,
    status: Option<Arc<Status>>,
    derived: Arc<Derived>,
    name: String,
    name_input: Entity<InputState>,
    /// The name input must be refilled from `name` on the next render (needs a `Window`).
    reset_name: bool,
    error: Option<String>,
    sending: bool,
    busy: bool,
    decisions: HashMap<String, Conflict>,
    limit: usize,
    /// `<details>` open state overrides by key.
    toggles: HashMap<String, bool>,
    /// Conflict selects by (list scope, path); created lazily while rendering.
    selects: HashMap<(u8, String), Entity<LabelSelect>>,
    watcher: InstallWatcher,
}

impl EventEmitter<InstallEvent> for InstallationView {}

pub fn stage_label(m: &Messages, stage: &str) -> &'static str {
    match stage {
        "extracting" => m.extracting,
        "preparingPack" => m.preparing_pack,
        "copying" => m.copying,
        "replacing" => m.replacing,
        "publishing" => m.publishing,
        _ => "",
    }
}

fn conflict_label(m: &Messages, conflict: Conflict) -> &'static str {
    match conflict {
        Conflict::Ask => m.ask,
        Conflict::Replace => m.replace,
        Conflict::Keep => m.keep,
    }
}

fn conflict_items(m: &Messages) -> Items {
    SearchableVec::new(
        CONFLICTS
            .iter()
            .map(|conflict| SharedString::from(conflict_label(m, *conflict)))
            .collect::<Vec<_>>(),
    )
}

fn conflict_row(conflict: Conflict) -> usize {
    CONFLICTS.iter().position(|c| *c == conflict).unwrap_or(0)
}

fn policy(m: &Messages, file: &Transfer) -> &'static str {
    if file.directory {
        m.copy_folder
    } else if file.exists {
        if file.conflict == Conflict::Keep {
            m.keep
        } else {
            m.replace
        }
    } else {
        m.copy_file
    }
}

fn item_label(m: &Messages, item: &ServerItem) -> &'static str {
    match (item.old, item.new) {
        (true, true) => m.item_replace,
        (true, false) => m.item_remove,
        _ => m.item_add,
    }
}

/// Phases in which a transaction is active (`preparing`, `preview`, `copying`).
fn phase_active(phase: &str) -> bool {
    matches!(phase, "preparing" | "preview" | "copying")
}

fn h3(text: impl Into<SharedString>) -> Div {
    div().font_weight(FontWeight::BOLD).text_size(px(16.)).my(px(16.)).child(text.into())
}

fn h4(text: impl Into<SharedString>) -> Div {
    div().font_weight(FontWeight::BOLD).my(px(18.)).child(text.into())
}

fn para(text: impl Into<SharedString>) -> Div {
    paragraph(text).mb(px(14.))
}

fn muted_p(p: &Palette, text: impl Into<SharedString>) -> Div {
    muted_paragraph(p, text).mb(px(14.))
}

impl InstallationView {
    pub fn new(props: InstallProps, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name_input = cx.new(|cx| InputState::new(window, cx));
        cx.subscribe(&name_input, |this, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let value = input.read(cx).value().to_string();
                if this.name != value {
                    this.name = value;
                    cx.notify();
                }
            }
        })
        .detach();

        cx.spawn(async move |this, cx| loop {
            if this.update(cx, |this, cx| this.poll(cx)).is_err() {
                break;
            }
            cx.background_executor().timer(Duration::from_millis(500)).await;
        })
        .detach();

        Self {
            backend: Backend::get(cx),
            props,
            request: None,
            status: None,
            derived: Arc::default(),
            name: String::new(),
            name_input,
            reset_name: false,
            error: None,
            sending: false,
            busy: false,
            decisions: HashMap::new(),
            limit: FIRST_PAGE,
            toggles: HashMap::new(),
            selects: HashMap::new(),
            watcher: InstallWatcher::new(),
        }
    }

    pub fn request(&self) -> Option<&PreparedRequest> {
        self.request.as_ref()
    }

    /// Hands over (or clears) the request to prepare. A new request resets name and decisions.
    pub fn set_request(&mut self, request: Option<PreparedRequest>, cx: &mut Context<Self>) {
        if let Some(next) = &request {
            let pack = match next {
                PreparedRequest::Client(client) => &client.pack,
                PreparedRequest::Server(server) => &server.pack,
            };
            self.name = pack_name(pack.label());
            self.reset_name = true;
            self.error = None;
            self.reset_decisions();
            self.toggles.clear();
            self.limit = FIRST_PAGE;
        }
        self.request = request;
        cx.notify();
    }

    pub fn set_props(&mut self, props: &InstallProps, cx: &mut Context<Self>) {
        if &self.props == props {
            return;
        }
        if self.props.language != props.language {
            // Select labels are language dependent; they are recreated from `decisions`.
            self.selects.clear();
        }
        self.props = props.clone();
        cx.notify();
    }

    fn reset_decisions(&mut self) {
        self.decisions.clear();
        self.selects.clear();
    }

    fn parent(&self) -> String {
        self.props.default_parent.clone().unwrap_or_default()
    }

    fn phase(&self) -> &str {
        self.status.as_ref().map_or("", |status| status.phase.as_str())
    }

    fn sync_busy(&mut self, cx: &mut Context<Self>) {
        let busy = transaction_busy(self.phase(), self.sending);
        if busy != self.busy {
            self.busy = busy;
            cx.emit(InstallEvent::Busy(busy));
        }
    }

    fn poll(&mut self, cx: &mut Context<Self>) {
        let Some(since) = self.watcher.begin_poll() else {
            return;
        };
        match self.backend.installation_status(since) {
            Ok(status) => {
                if let Some(status) = self.watcher.finish_poll(status) {
                    if self.status.as_ref().is_none_or(|current| status.revision > current.revision) {
                        self.set_status(status, cx);
                    }
                }
            }
            Err(error) => {
                self.watcher.abort_poll();
                self.error = Some(error);
                cx.notify();
            }
        }
    }

    /// Unconditionally takes `status` (the result of a command) and recomputes derived data.
    fn set_status(&mut self, status: Status, cx: &mut Context<Self>) {
        let entries = match (status.phase.as_str(), &status.preview, &status.server_preview) {
            ("preview", Some(preview), _) => Some(&preview.files),
            ("preview", None, Some(server)) if !status.client => Some(&server.files),
            _ => None,
        };
        self.derived = Arc::new(match entries {
            Some(entries) => Derived {
                groups: group_transfers(entries),
                files: entries.iter().filter(|file| !file.directory).count(),
                folders: entries.iter().filter(|file| file.directory).count(),
                asks: entries
                    .iter()
                    .filter(|file| {
                        file.exists && !file.directory && file.conflict == Conflict::Ask
                    })
                    .cloned()
                    .collect(),
            },
            None => Derived::default(),
        });
        let status = Arc::new(status);
        self.status = Some(status.clone());
        cx.emit(InstallEvent::Status(Some(status)));
        self.sync_busy(cx);
        cx.notify();
    }

    /// `status = await installation_status() ?? status`.
    fn refresh_status(&mut self, cx: &mut Context<Self>) -> Result<(), String> {
        if let Some(status) = self.backend.installation_status(None)? {
            self.set_status(status, cx);
        }
        Ok(())
    }

    fn take_request(&mut self, cx: &mut Context<Self>) {
        self.request = None;
        cx.emit(InstallEvent::RequestConsumed);
    }

    fn prepare(&mut self, cx: &mut Context<Self>) {
        let Some(request) = self.request.clone() else {
            return;
        };
        if self.sending {
            return;
        }
        let parent = self.parent();
        if matches!(request, PreparedRequest::Client(_)) && (parent.is_empty() || self.name.is_empty())
        {
            return;
        }
        self.sending = true;
        self.error = None;
        self.reset_decisions();
        let started = match request {
            PreparedRequest::Client(client) => self
                .backend
                .prepare_installation(client.into_install_request(parent, self.name.clone())),
            PreparedRequest::Server(server) => {
                self.backend.prepare_server_update(server.into_update_request())
            }
        };
        match started.and_then(|_| self.refresh_status(cx)) {
            Ok(()) => self.take_request(cx),
            Err(error) => self.error = Some(error),
        }
        self.sending = false;
        self.sync_busy(cx);
        cx.notify();
    }

    fn finish(&mut self, cx: &mut Context<Self>) {
        self.sending = true;
        self.error = None;
        let decisions = self
            .decisions
            .iter()
            .map(|(path, conflict)| Decision { path: path.clone(), conflict: *conflict })
            .collect();
        let result = self.backend.finish_installation(decisions).and_then(|_| self.refresh_status(cx));
        if let Err(error) = result {
            self.error = Some(error);
        }
        self.sending = false;
        self.sync_busy(cx);
        cx.notify();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        self.sending = true;
        self.error = None;
        match self.backend.cancel_installation().and_then(|_| self.refresh_status(cx)) {
            Ok(()) => self.take_request(cx),
            Err(error) => self.error = Some(error),
        }
        self.sending = false;
        self.sync_busy(cx);
        cx.notify();
    }

    fn toggle(&mut self, key: &str, open: bool, cx: &mut Context<Self>) {
        self.toggles.insert(key.to_string(), open);
        if key == "all" && !open {
            self.limit = FIRST_PAGE;
        }
        cx.notify();
    }

    fn unanswered(&self) -> usize {
        unresolved_conflicts(&self.derived.asks, &self.decisions).len()
    }

    fn decision_select(
        &mut self,
        scope: u8,
        path: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<LabelSelect> {
        if let Some(select) = self.selects.get(&(scope, path.to_string())) {
            return select.clone();
        }
        let m = messages(self.props.language);
        let row = conflict_row(self.decisions.get(path).copied().unwrap_or(Conflict::Ask));
        let select =
            cx.new(|cx| SelectState::new(conflict_items(m), Some(IndexPath::new(row)), window, cx));
        let key = path.to_string();
        cx.subscribe_in(&select, window, move |this, _, event: &SelectEvent<Items>, window, cx| {
            if let SelectEvent::Confirm(Some(value)) = event {
                let m = messages(this.props.language);
                let conflict = CONFLICTS
                    .iter()
                    .copied()
                    .find(|c| conflict_label(m, *c) == value.as_ref())
                    .unwrap_or(Conflict::Ask);
                this.decisions.insert(key.clone(), conflict);
                // Keep the same file's select in the other list in sync.
                for ((_, path), other) in this.selects.iter() {
                    if path == &key {
                        other.update(cx, |state, cx| {
                            state.set_selected_index(
                                Some(IndexPath::new(conflict_row(conflict))),
                                window,
                                cx,
                            );
                        });
                    }
                }
                cx.notify();
            }
        })
        .detach();
        self.selects.insert((scope, path.to_string()), select.clone());
        select
    }

    // --- render pieces ----------------------------------------------------------------------

    /// A `<details>` with a clickable summary; `body` runs only while open.
    #[allow(clippy::too_many_arguments)]
    fn details<R: IntoElement>(
        &mut self,
        p: &'static Palette,
        key: &str,
        default_open: bool,
        summary: String,
        window: &mut Window,
        cx: &mut Context<Self>,
        body: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) -> R,
    ) -> Div {
        let open = self.toggles.get(key).copied().unwrap_or(default_open);
        let id = ElementId::from(SharedString::from(format!("details-{key}")));
        let toggle_key = key.to_string();
        let head = div()
            .id(id)
            .cursor_pointer()
            .py(px(8.))
            .text_size(px(12.))
            .text_color(color(p.muted))
            .child(format!("{} {summary}", if open { "▾" } else { "▸" }))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.toggle(&toggle_key, !open, cx)
            }));
        let mut block = div().flex().flex_col().my(px(14.)).child(head);
        if open {
            block = block.child(body(self, window, cx));
        }
        block
    }

    fn mod_item(p: &Palette) -> Div {
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .py(px(12.))
            .border_t_1()
            .border_color(color(p.border))
            .text_size(px(12.))
    }

    fn mods_block(
        &mut self,
        p: &'static Palette,
        m: &'static Messages,
        plan: &ModPlan,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut block = div().flex().flex_col();
        if plan.removed.is_empty() && plan.added.is_empty() && plan.duplicates.is_empty() {
            return block;
        }
        block = block.child(h4(m.my_mods));
        if !plan.duplicates.is_empty() {
            let mut error = status_block(p, p.danger)
                .child(div().font_weight(FontWeight::BOLD).child(m.mod_duplicates));
            for group in &plan.duplicates {
                error = error.child(directory(p, group.join(" · ")));
            }
            block = block.child(error);
        }
        if !plan.removed.is_empty() {
            let removed = plan.removed.clone();
            block = block.child(self.details(
                p,
                "mods-removed",
                true,
                format!("{} ({})", m.mods_removed_title, plan.removed.len()),
                window,
                cx,
                move |_, _, _| {
                    let mut list = div().flex().flex_col();
                    for item in &removed {
                        let mut row = Self::mod_item(p)
                            .child(div().font_weight(FontWeight::BOLD).child(item.name.clone()));
                        if item.files.is_empty() {
                            row = row.child(aligned(
                                status_block(p, p.muted).mb(px(0.)).py(px(2.)).child(m.mod_no_match),
                            ));
                        } else {
                            for file in &item.files {
                                row = row.child(code(file.clone()));
                            }
                        }
                        list = list.child(row);
                    }
                    list
                },
            ));
        }
        if !plan.added.is_empty() {
            let added = plan.added.clone();
            block = block.child(self.details(
                p,
                "mods-added",
                true,
                format!("{} ({})", m.mods_added_title, plan.added.len()),
                window,
                cx,
                move |_, _, _| {
                    let mut list = div().flex().flex_col();
                    for item in &added {
                        let mut row = Self::mod_item(p)
                            .child(div().font_weight(FontWeight::BOLD).child(item.name.clone()))
                            .child(code(item.file.clone()))
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .flex_wrap()
                                    .gap(px(4.))
                                    .text_color(color(p.muted))
                                    .child(format!("{}:", m.mod_source_file))
                                    .child(code(item.source.clone())),
                            );
                        if !item.replaces.is_empty() {
                            row = row.child(
                                div()
                                    .text_color(color(p.muted))
                                    .child(format!("{}: {}", m.mod_replaces, item.replaces.join(", "))),
                            );
                        }
                        if item.missing {
                            row = row.child(
                                div()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(color(p.danger))
                                    .child(m.mod_missing),
                            );
                        }
                        list = list.child(row);
                    }
                    list
                },
            ));
        }
        block
    }

    fn row(
        &mut self,
        scope: u8,
        file: &Transfer,
        p: &'static Palette,
        m: &'static Messages,
        narrow: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let path = div()
            .flex_1()
            .min_w_0()
            .font_family(mono_font())
            .text_size(px(12.))
            .child(file.path.clone());
        let row = rule_row(p, narrow).child(path);
        if !file.directory && file.exists && file.conflict == Conflict::Ask {
            let select = self.decision_select(scope, &file.path, window, cx);
            row.child(
                Select::new(&select)
                    .w(px(130.))
                    .flex_shrink_0()
                    .bg(color(p.raised))
                    .border_color(color(p.border))
                    .text_color(color(p.text))
                    .rounded(px(8.))
                    .px(px(6.))
                    .text_size(px(11.)),
            )
        } else {
            row.child(self.policy_label(p, policy(m, file).to_string()))
        }
    }

    fn policy_label(&self, p: &Palette, text: String) -> Div {
        div()
            .flex_shrink_0()
            .max_w(gpui::relative(0.55))
            .text_size(px(11.))
            .text_color(color(p.muted))
            .text_right()
            .child(text)
    }

    fn transfer_list(id: impl Into<ElementId>) -> gpui::Stateful<Div> {
        div().id(id).flex().flex_col().max_h(px(340.)).overflow_y_scroll()
    }

    #[allow(clippy::too_many_arguments)]
    fn transfers(
        &mut self,
        p: &'static Palette,
        m: &'static Messages,
        entries: &[Transfer],
        missing: &[String],
        notify_empty: bool,
        narrow: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let derived = self.derived.clone();
        let language = self.props.language;
        let mut block = div().flex().flex_col().child(para(format!(
            "{}: {} · {}: {}",
            m.transfer_count, derived.files, m.transfer_folders, derived.folders
        )));
        if notify_empty && entries.is_empty() {
            block = block.child(status_block(p, p.muted).child(m.nothing_to_transfer));
        }
        if !missing.is_empty() {
            let list = missing.to_vec();
            block = block.child(self.details(
                p,
                "missing",
                entries.is_empty(),
                format!("{} ({})", m.skipped_missing, missing.len()),
                window,
                cx,
                move |_, _, _| {
                    let mut body = div().flex().flex_col();
                    for path in list {
                        body = body.child(directory(p, path));
                    }
                    body
                },
            ));
        }
        if !derived.asks.is_empty() {
            block = block.child(h4(m.needs_decision));
            let mut list = Self::transfer_list("asks");
            for file in &derived.asks {
                list = list.child(self.row(0, file, p, m, narrow, window, cx));
            }
            block = block.child(list);
        }
        if !entries.is_empty() {
            let groups = derived.clone();
            block = block.child(self.details(
                p,
                "top",
                true,
                format!("{} ({})", m.transfer_top_level, derived.groups.len()),
                window,
                cx,
                move |this, _, _| {
                    let mut list = Self::transfer_list("groups");
                    for group in &groups.groups {
                        let text = if group.directory {
                            format!(
                                "{} · {} {} · {}",
                                m.copy_folder,
                                group.files,
                                m.files_summary,
                                format_bytes(group.bytes, language)
                            )
                        } else {
                            group.file.as_ref().map(|file| policy(m, file).to_string()).unwrap_or_default()
                        };
                        list = list.child(
                            rule_row(p, narrow)
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .font_family(mono_font())
                                        .text_size(px(12.))
                                        .child(group.path.clone()),
                                )
                                .child(this.policy_label(p, text)),
                        );
                    }
                    list
                },
            ));
            let all: Vec<Transfer> = entries.to_vec();
            block = block.child(self.details(
                p,
                "all",
                false,
                format!("{} ({})", m.all_transfer_files, entries.len()),
                window,
                cx,
                move |this, window, cx| {
                    let mut body = div().flex().flex_col();
                    let mut list = Self::transfer_list("all-files");
                    for file in all.iter().take(this.limit) {
                        list = list.child(this.row(1, file, p, m, narrow, window, cx));
                    }
                    body = body.child(list);
                    if this.limit < all.len() {
                        body = body.child(aligned(
                            secondary_button("show-more", m.show_more, p, false)
                                .mt(px(8.))
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.limit += PAGE;
                                    cx.notify();
                                })),
                        ));
                    }
                    body
                },
            ));
        }
        let unanswered = self.unanswered();
        if unanswered > 0 {
            block = block.child(muted_p(p, format!("{}: {unanswered}", m.resolve_conflicts)));
        }
        block
    }

    fn confirm_button(
        &self,
        p: &'static Palette,
        id: &'static str,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> Div {
        let disabled = self.sending || self.unanswered() > 0;
        aligned(
            primary_button(id, label, p, disabled).mt(px(16.)).when(!disabled, |button| {
                button.on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.finish(cx)))
            }),
        )
    }

    fn render_server_preview(
        &mut self,
        p: &'static Palette,
        m: &'static Messages,
        status: &Status,
        narrow: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let Some(preview) = &status.server_preview else {
            return div();
        };
        let mut block = div()
            .flex()
            .flex_col()
            .child(h3(m.server_preview_title))
            .child(muted_p(p, m.server_folder_label).mb(px(0.)))
            .child(directory(p, preview.server.clone()));
        if let Some(recovered) = preview.recovered {
            block = block.child(status_block(p, p.muted).child(match recovered {
                Recovery::RolledBack => m.server_rolled_back,
                Recovery::Completed => m.server_completed,
            }));
        }
        if let Some(java) = preview.java_change {
            block = block.child(status_block(p, p.muted).child(match java {
                JavaChange::ToJava17 => m.java_to_java17,
                JavaChange::ToJava8 => m.java_to_java8,
            }));
        }
        block = block.child(status_block(p, p.muted).child(m.server_stop_warning));
        if !preview.replaced.is_empty() {
            let items = preview.replaced.clone();
            block = block.child(self.details(
                p,
                "server-replaced",
                true,
                format!("{} ({})", m.server_replaced_items, preview.replaced.len()),
                window,
                cx,
                move |this, _, _| {
                    let mut list = Self::transfer_list("server-replaced-list");
                    for item in &items {
                        list = list.child(
                            rule_row(p, narrow)
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .font_family(mono_font())
                                        .text_size(px(12.))
                                        .child(format!(
                                            "{}{}",
                                            item.path,
                                            if item.directory { "/" } else { "" }
                                        )),
                                )
                                .child(this.policy_label(p, item_label(m, item).to_string())),
                        );
                    }
                    list
                },
            ));
        }
        if !preview.added.is_empty() {
            let added = preview.added.clone();
            block = block.child(self.details(
                p,
                "server-added",
                false,
                format!("{} ({})", m.server_added, preview.added.len()),
                window,
                cx,
                move |this, _, _| {
                    let mut list = Self::transfer_list("server-added-list");
                    for path in &added {
                        list = list.child(
                            rule_row(p, narrow)
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .font_family(mono_font())
                                        .text_size(px(12.))
                                        .child(path.clone()),
                                )
                                .child(this.policy_label(p, m.item_add.to_string())),
                        );
                    }
                    list
                },
            ));
        }
        block = block.child(self.mods_block(p, m, &preview.mods, window, cx));
        if !preview.files.is_empty() || !preview.missing.is_empty() {
            block = block.child(h4(m.server_files_title));
        }
        block = block.child(self.transfers(p, m, &preview.files, &preview.missing, false, narrow, window, cx));
        if preview.journey_map {
            block = block.child(muted_p(p, m.server_journey_kept));
        }
        if !preview.scripts.is_empty() {
            block = block.child(h4(format!("{} ({})", m.server_scripts, preview.scripts.len())));
            for (index, script) in preview.scripts.iter().enumerate() {
                let script = script.clone();
                let title = format!(
                    "{}{}",
                    script.path,
                    if script.new.is_none() { format!(" · {}", m.script_removed) } else { String::new() }
                );
                block = block.child(self.details(
                    p,
                    &format!("script-{index}"),
                    false,
                    title,
                    window,
                    cx,
                    move |_, _, _| {
                        let side = |label: &'static str, text: &String, id: String| {
                            div()
                                .flex_1()
                                .min_w(px(260.))
                                .flex()
                                .flex_col()
                                .child(div().font_weight(FontWeight::BOLD).child(label))
                                .child(
                                    div()
                                        .id(SharedString::from(id))
                                        .bg(color(p.raised))
                                        .p(px(10.))
                                        .rounded(px(7.))
                                        .mt(px(8.))
                                        .max_h(px(260.))
                                        .overflow_y_scroll()
                                        .child(pre(text.clone())),
                                )
                        };
                        let mut diff = div().flex().flex_row().flex_wrap().gap(px(12.));
                        if let Some(old) = &script.old {
                            diff = diff.child(side(m.script_old, old, format!("script-old-{index}")));
                        }
                        if let Some(new) = &script.new {
                            diff = diff.child(side(m.script_new, new, format!("script-new-{index}")));
                        }
                        diff
                    },
                ));
            }
            block = block.child(muted_p(p, m.scripts_note));
        }
        block = block
            .child(muted_p(p, m.server_backup_label).mb(px(0.)))
            .child(directory(p, preview.backup.clone()));
        if preview.previous_backup {
            block = block.child(status_block(p, p.muted).child(m.backup_replaced));
        }
        block.child(self.confirm_button(p, "confirm-server", m.confirm_server_update, cx))
    }

    fn render_client_preview(
        &mut self,
        p: &'static Palette,
        m: &'static Messages,
        status: &Status,
        narrow: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let Some(preview) = &status.preview else {
            return div();
        };
        let mut block = div()
            .flex()
            .flex_col()
            .child(h3(m.transfer_preview))
            .child(directory(p, preview.destination.clone()))
            .child(status_block(p, p.muted).child(m.install_warning))
            .child(self.mods_block(p, m, &preview.mods, window, cx));
        if !preview.renames.is_empty() {
            block = block.child(h4(m.rename_title));
            for item in &preview.renames {
                let mut row = Self::mod_item(p).child(code(format!("{} → {}", item.path, item.to)));
                if !item.found {
                    row = row.child(div().text_color(color(p.muted)).child(m.rename_not_found));
                } else if item.replaces {
                    row = row.child(div().text_color(color(p.muted)).child(m.rename_replaces));
                }
                block = block.child(row);
            }
        }
        block = block.child(self.transfers(p, m, &preview.files, &preview.missing, true, narrow, window, cx));
        block.child(self.confirm_button(p, "confirm-install", m.confirm_installation, cx))
    }
}

impl Render for InstallationView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = current_palette(cx);
        let m = messages(self.props.language);
        let narrow = window.viewport_size().width < px(900.);
        let status = self.status.clone();
        let phase = self.phase().to_string();
        let shown = self.request.is_some() || !phase.is_empty();
        if !shown {
            return div();
        }
        if std::mem::take(&mut self.reset_name) {
            let name = self.name.clone();
            self.name_input.update(cx, |state, cx| state.set_value(name, window, cx));
        }

        let active = phase_active(&phase);
        let working = phase == "preparing" || phase == "copying";
        let close_disabled = working || self.sending;
        let client = status.as_ref().is_none_or(|s| s.client);

        let mut card = div()
            .flex()
            .flex_col()
            .border_1()
            .border_color(color(p.border))
            .rounded(px(12.))
            .p(px(if narrow { 18. } else { 22. }))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap(px(12.))
                    .mb(px(12.))
                    .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child(m.install))
                    .child(
                        secondary_button(
                            "install-close",
                            if active { m.cancel_download } else { m.close },
                            p,
                            close_disabled,
                        )
                        .when(!close_disabled, |button| {
                            button.on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close(cx)))
                        }),
                    ),
            );
        if let Some(error) = &self.error {
            card = card.child(status_block(p, p.danger).child(error.clone()));
        }

        if let (Some(request), false) = (&self.request, active) {
            match request {
                PreparedRequest::Client(_) => {
                    if self.parent().is_empty() {
                        let sending = self.sending;
                        card = card.child(aligned(
                            secondary_button("configure-prism", m.configure_prism, p, sending)
                                .when(!sending, |button| {
                                    button.on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                                        cx.emit(InstallEvent::Settings)
                                    }))
                                }),
                        ));
                    }
                    let disabled = self.parent().is_empty() || self.name.is_empty() || self.sending;
                    card = card
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(8.))
                                .my(px(18.))
                                .text_size(px(12.))
                                .child(m.new_instance_name)
                                .child(
                                    Input::new(&self.name_input)
                                        .disabled(self.sending)
                                        .bg(color(p.raised))
                                        .border_color(color(p.border))
                                        .text_color(color(p.text))
                                        .rounded(px(8.))
                                        .h(px(39.))
                                        .px(px(4.))
                                        .text_size(px(14.)),
                                ),
                        )
                        .child(aligned(
                            primary_button("prepare-install", m.prepare_installation, p, disabled)
                                .when(!disabled, |button| {
                                    button.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.prepare(cx)
                                    }))
                                }),
                        ));
                }
                PreparedRequest::Server(server) => {
                    let sending = self.sending;
                    card = card.child(directory(p, server.server.clone())).child(aligned(
                        primary_button("prepare-server", m.prepare_server_update, p, sending)
                            .when(!sending, |button| {
                                button.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.prepare(cx)
                                }))
                            }),
                    ));
                }
            }
        }

        if working {
            let progress = status.as_ref().and_then(|s| s.progress.clone());
            let label = match &progress {
                Some(progress) => stage_label(m, progress.stage),
                None if !client => m.prepare_server_update,
                None => m.prepare_installation,
            };
            card = card
                .child(para(format!("{label}…")))
                .child(progress_bar(
                    p,
                    progress.as_ref().map_or(0, |pr| pr.done),
                    progress.as_ref().map_or(0, |pr| pr.total),
                ));
            if let Some(progress) = progress {
                card = card.child(
                    muted_p(p, format!("{} / {}", progress.done, progress.total)).mt(px(10.)),
                );
            }
        }

        if let Some(status) = &status {
            if phase == "preview" {
                if status.preview.is_some() {
                    card = card.child(self.render_client_preview(p, m, status, narrow, window, cx));
                }
                if !status.client && status.server_preview.is_some() {
                    card = card.child(self.render_server_preview(p, m, status, narrow, window, cx));
                }
            }
            if self.request.is_none() {
                if phase == "completed" {
                    let destination = status.destination.clone().unwrap_or_default();
                    if status.client {
                        card = card
                            .child(para(m.install_complete))
                            .child(directory(p, destination))
                            .child(muted_p(p, m.prism_ready));
                    } else {
                        card = card
                            .child(para(m.server_updated))
                            .child(directory(p, destination));
                        if let Some(preview) = &status.server_preview {
                            card = card
                                .child(muted_p(p, m.server_backup_label).mb(px(0.)))
                                .child(directory(p, preview.backup.clone()));
                        }
                        card = card.child(muted_p(p, m.server_done_hint));
                    }
                } else if phase == "failed" {
                    card = card.child(
                        status_block(p, p.danger)
                            .child(if status.client { m.install_failed } else { m.server_updated_failed })
                            .child(pre(status.error.clone().unwrap_or_default())),
                    );
                }
            }
        }

        card
    }
}
