//! Port of `TransferRules.svelte` (client rules, and the `server` variant).

use std::path::PathBuf;

use gpui::{
    div, prelude::*, px, ClickEvent, Context, EventEmitter, FontWeight, IntoElement, Render,
    SharedString, Window,
};
use gpui_component::{
    checkbox::Checkbox,
    select::{SearchableVec, Select, SelectEvent, SelectState},
    Disableable as _, IndexPath,
};
use gtnh_core::{Conflict, Rule};

use super::{
    add_form, heading, pick_paths, remove_button, rule_list, rule_row, summary, tag,
    PickKind, RulesEvent, RulesProps,
};
use crate::i18n::{messages, Messages};
use crate::theme::{color, current_palette, mono_font};
use crate::ui::{muted_paragraph, secondary_button, status_block};

type Items = SearchableVec<SharedString>;
type LabelSelect = SelectState<Items>;

const CONFLICTS: [Conflict; 3] = [Conflict::Ask, Conflict::Replace, Conflict::Keep];

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

/// Appends a custom `ask` rule for every path not yet listed (case-insensitive); true when any was added.
pub fn add_custom_rules(rules: &mut Vec<Rule>, paths: Vec<String>) -> bool {
    let mut added = false;
    for path in paths {
        if rules.iter().any(|rule| rule.path.to_lowercase() == path.to_lowercase()) {
            continue;
        }
        rules.push(Rule { path, enabled: true, conflict: Conflict::Ask, custom: true });
        added = true;
    }
    added
}

pub struct TransferRulesView {
    server: bool,
    rules: Vec<Rule>,
    /// One conflict select per custom rule (`None` for standard rules), parallel to `rules`.
    selects: Vec<Option<gpui::Entity<LabelSelect>>>,
    props: RulesProps,
    picking: bool,
    open: bool,
    error: Option<String>,
    notice: Option<String>,
}

impl EventEmitter<RulesEvent> for TransferRulesView {}

impl TransferRulesView {
    pub fn new(server: bool, _window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self {
            server,
            rules: Vec::new(),
            selects: Vec::new(),
            props: RulesProps::default(),
            picking: false,
            open: server,
            error: None,
            notice: None,
        }
    }

    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    pub fn set_rules(&mut self, rules: Vec<Rule>, window: &mut Window, cx: &mut Context<Self>) {
        self.rules = rules;
        self.rebuild_selects(window, cx);
        cx.notify();
    }

    pub fn set_props(&mut self, props: &RulesProps, window: &mut Window, cx: &mut Context<Self>) {
        if &self.props == props {
            return;
        }
        let language_changed = self.props.language != props.language;
        self.props = props.clone();
        if language_changed {
            self.rebuild_selects(window, cx);
        }
        cx.notify();
    }

    fn disabled(&self) -> bool {
        self.props.disabled || self.picking
    }

    fn rebuild_selects(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let m = messages(self.props.language);
        let mut selects = Vec::with_capacity(self.rules.len());
        for rule in &self.rules {
            if !rule.custom {
                selects.push(None);
                continue;
            }
            let row = CONFLICTS.iter().position(|c| *c == rule.conflict).unwrap_or(0);
            let select = cx.new(|cx| {
                SelectState::new(conflict_items(m), Some(IndexPath::new(row)), window, cx)
            });
            let path = rule.path.clone();
            cx.subscribe_in(&select, window, move |this, _, event: &SelectEvent<Items>, _, cx| {
                if let SelectEvent::Confirm(Some(value)) = event {
                    let m = messages(this.props.language);
                    let conflict = CONFLICTS
                        .iter()
                        .copied()
                        .find(|c| conflict_label(m, *c) == value.as_ref())
                        .unwrap_or(Conflict::Ask);
                    if let Some(rule) = this.rules.iter_mut().find(|rule| rule.path == path) {
                        if rule.conflict != conflict {
                            rule.conflict = conflict;
                            cx.emit(RulesEvent::Changed);
                            cx.notify();
                        }
                    }
                }
            })
            .detach();
            selects.push(Some(select));
        }
        self.selects = selects;
    }

    fn set_enabled(&mut self, index: usize, enabled: bool, cx: &mut Context<Self>) {
        if self.disabled() {
            return;
        }
        if let Some(rule) = self.rules.get_mut(index) {
            if rule.enabled != enabled {
                rule.enabled = enabled;
                cx.emit(RulesEvent::Changed);
                cx.notify();
            }
        }
    }

    fn remove(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled() || index >= self.rules.len() {
            return;
        }
        self.rules.remove(index);
        self.rebuild_selects(window, cx);
        cx.emit(RulesEvent::Changed);
        cx.notify();
    }

    fn pick(&mut self, folder: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled() || self.props.source.is_empty() {
            return;
        }
        self.picking = true;
        self.error = None;
        self.notice = None;
        cx.emit(RulesEvent::Picking(true));
        cx.notify();
        let m = messages(self.props.language);
        let title = if folder { m.add_folder } else { m.add_file };
        let source = PathBuf::from(&self.props.source);
        cx.spawn_in(window, async move |this, cx| {
            let dialog = rfd::AsyncFileDialog::new().set_title(title).set_directory(&source);
            let kind = if folder { PickKind::Folders } else { PickKind::Files };
            let picked = pick_paths(dialog, kind).await;
            let result = match picked {
                None => None,
                Some(paths) => {
                    let selected: Vec<String> =
                        paths.iter().map(|path| path.to_string_lossy().into_owned()).collect();
                    let root = source.clone();
                    Some(
                        cx.background_executor()
                            .spawn(async move {
                                gtnh_core::selection::relative_selections(&root, &selected)
                                    .map_err(|error| error.to_string())
                            })
                            .await,
                    )
                }
            };
            this.update_in(cx, |this, window, cx| {
                this.picking = false;
                match result {
                    Some(Ok(paths)) => {
                        if add_custom_rules(&mut this.rules, paths) {
                            this.rebuild_selects(window, cx);
                            cx.emit(RulesEvent::Changed);
                        } else {
                            this.notice = Some(messages(this.props.language).duplicate.to_string());
                        }
                    }
                    Some(Err(error)) => this.error = Some(error),
                    None => {}
                }
                cx.emit(RulesEvent::Picking(false));
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

impl Render for TransferRulesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = current_palette(cx);
        let m = messages(self.props.language);
        let narrow = window.viewport_size().width < px(900.);
        let disabled = self.disabled();
        let no_source = self.props.source.is_empty();
        let enabled = self.rules.iter().filter(|rule| rule.enabled).count();

        let mut root = div().flex().flex_col();
        if self.server {
            root = root
                .child(heading(m.server_rules_title))
                .child(muted_paragraph(p, m.server_rules_hint));
        }
        root = root.child(summary(m.enabled_rules, enabled, self.rules.len()));

        let mut details = div().child(
            div()
                .id("rules-summary")
                .cursor_pointer()
                .text_size(px(12.))
                .text_color(color(p.muted))
                .py(px(8.))
                .child(format!("{} {}", if self.open { "▾" } else { "▸" }, m.all_rules))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.open = !this.open;
                    cx.notify();
                })),
        );
        if self.open {
            let mut list = rule_list();
            for (index, rule) in self.rules.iter().enumerate() {
                let path_label = div()
                    .id(("rule-path", index))
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        Checkbox::new(("rule-enabled", index))
                            .checked(rule.enabled)
                            .disabled(disabled)
                            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                this.set_enabled(index, *checked, cx)
                            })),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .font_family(mono_font())
                            .text_size(px(12.))
                            .child(rule.path.clone()),
                    );
                let path_label = if disabled {
                    path_label
                } else {
                    let enabled = rule.enabled;
                    path_label.cursor_pointer().on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| this.set_enabled(index, !enabled, cx),
                    ))
                };

                let mut row = rule_row(p, narrow).child(path_label);
                if !narrow {
                    row = row.child(tag(p, if rule.custom { m.custom } else { m.standard }));
                }
                if rule.custom {
                    if let Some(Some(select)) = self.selects.get(index) {
                        row = row.child(
                            Select::new(select)
                                .disabled(disabled)
                                .bg(color(p.raised))
                                .border_color(color(p.border))
                                .text_color(color(p.text))
                                .rounded(px(8.))
                                .w(px(130.))
                                .h(px(28.))
                                .px(px(6.))
                                .py(px(6.))
                                .text_size(px(11.)),
                        );
                    }
                    row = row.child(remove_button(("rule-remove", index), p, disabled).when(
                        !disabled,
                        |button| {
                            button.on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                this.remove(index, window, cx)
                            }))
                        },
                    ));
                } else {
                    let policy = div();
                    let policy = if narrow { policy } else { policy.w(px(100.)) };
                    row = row.child(
                        policy
                            .flex_shrink_0()
                            .text_size(px(11.))
                            .text_color(color(p.muted))
                            .text_right()
                            .child(m.replace),
                    );
                }
                list = list.child(row);
            }
            let picker_disabled = disabled || no_source;
            details = details.child(list).child(
                add_form(p)
                    .child(
                        secondary_button("add-file", format!("+ {}", m.add_file), p, picker_disabled)
                            .when(!picker_disabled, |button| {
                                button.on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                    this.pick(false, window, cx)
                                }))
                            }),
                    )
                    .child(
                        secondary_button(
                            "add-folder",
                            format!("+ {}", m.add_folder),
                            p,
                            picker_disabled,
                        )
                        .when(!picker_disabled, |button| {
                            button.on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.pick(true, window, cx)
                            }))
                        }),
                    ),
            );
        }
        root = root.child(details);
        if let Some(error) = &self.error {
            root = root.child(status_block(p, p.danger).mt(px(8.)).child(error.clone()));
        }
        if let Some(notice) = &self.notice {
            root = root.child(muted_paragraph(p, notice.clone()));
        }
        root.font_weight(FontWeight::NORMAL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(path: &str) -> Rule {
        Rule { path: path.into(), enabled: true, conflict: Conflict::Replace, custom: false }
    }

    #[test]
    fn add_custom_rules_skips_existing_paths_case_insensitively() {
        let mut rules = vec![rule("Config/A.cfg")];
        let added = add_custom_rules(&mut rules, vec!["config/a.cfg".into(), "mods".into()]);
        assert!(added);
        assert_eq!(rules.len(), 2);
        assert_eq!(
            rules[1],
            Rule { path: "mods".into(), enabled: true, conflict: Conflict::Ask, custom: true }
        );
        assert!(!add_custom_rules(&mut rules, vec!["MODS".into()]));
        assert_eq!(rules.len(), 2);
    }
}
