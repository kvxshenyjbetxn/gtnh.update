//! Port of `RenameRules.svelte` and `rename-rules.ts`.

use std::path::PathBuf;

use gpui::{
    div, prelude::*, px, ClickEvent, Context, Entity, EventEmitter, IntoElement, Render,
    SharedString, Window,
};
use gpui_component::{
    checkbox::Checkbox,
    input::{Input, InputEvent, InputState},
    Disableable as _,
};
use gtnh_core::RenameRule;

use super::{
    add_form, heading, pick_paths, remove_button, rule_list, rule_row, PickKind, RulesEvent,
    RulesProps,
};
use crate::i18n::messages;
use crate::theme::{color, current_palette, mono_font};
use crate::ui::{muted_paragraph, secondary_button, status_block};

/// Last path component (`/` separated).
pub fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Default new name: the same file name, with a Russian locale switched to the English one.
pub fn default_rename_target(path: &str) -> String {
    file_name(path).replace("ru_RU", "en_US")
}

pub fn rename_rule_for(path: &str) -> RenameRule {
    RenameRule { path: path.to_string(), to: default_rename_target(path), enabled: true }
}

/// A rename keeps the file in its folder: one non-empty name that differs from the old one.
pub fn valid_rename_target(rule: &RenameRule) -> bool {
    !rule.to.is_empty()
        && rule.to.trim() == rule.to
        && !rule.to.contains(['\\', '/'])
        && rule.to.to_lowercase() != file_name(&rule.path).to_lowercase()
}

pub fn renames_valid(rules: &[RenameRule]) -> bool {
    rules.iter().all(valid_rename_target)
}

/// Appends a rule for every path not yet listed (case-insensitive); true when any was added.
pub fn add_rename_rules(rules: &mut Vec<RenameRule>, paths: Vec<String>) -> bool {
    let mut added = false;
    for path in paths {
        if rules.iter().any(|rule| rule.path.to_lowercase() == path.to_lowercase()) {
            continue;
        }
        rules.push(rename_rule_for(&path));
        added = true;
    }
    added
}

pub struct RenameRulesView {
    rules: Vec<RenameRule>,
    /// New-name input per rule, parallel to `rules`.
    inputs: Vec<Entity<InputState>>,
    props: RulesProps,
    picking: bool,
    /// A name was typed since the last commit (blur / Enter), like the DOM `change` event.
    dirty: bool,
    error: Option<String>,
    notice: Option<String>,
}

impl EventEmitter<RulesEvent> for RenameRulesView {}

impl RenameRulesView {
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self {
            rules: Vec::new(),
            inputs: Vec::new(),
            props: RulesProps::default(),
            picking: false,
            dirty: false,
            error: None,
            notice: None,
        }
    }

    pub fn rules(&self) -> &[RenameRule] {
        &self.rules
    }

    pub fn set_rules(&mut self, rules: Vec<RenameRule>, window: &mut Window, cx: &mut Context<Self>) {
        self.rules = rules;
        self.rebuild_inputs(window, cx);
        cx.notify();
    }

    pub fn set_props(&mut self, props: &RulesProps, _window: &mut Window, cx: &mut Context<Self>) {
        if &self.props == props {
            return;
        }
        self.props = props.clone();
        cx.notify();
    }

    fn disabled(&self) -> bool {
        self.props.disabled || self.picking
    }

    fn rebuild_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dirty = false;
        let mut inputs = Vec::with_capacity(self.rules.len());
        for rule in &self.rules {
            let input = cx.new(|cx| InputState::new(window, cx));
            input.update(cx, |state, cx| state.set_value(rule.to.clone(), window, cx));
            let path = rule.path.clone();
            cx.subscribe_in(&input, window, move |this, input, event: &InputEvent, _, cx| {
                match event {
                    InputEvent::Change => {
                        let value = input.read(cx).value().to_string();
                        if let Some(rule) = this.rules.iter_mut().find(|rule| rule.path == path) {
                            if rule.to != value {
                                rule.to = value;
                                this.dirty = true;
                                cx.notify();
                            }
                        }
                    }
                    InputEvent::Blur | InputEvent::PressEnter { .. } => {
                        if std::mem::take(&mut this.dirty) {
                            cx.emit(RulesEvent::Changed);
                        }
                    }
                    InputEvent::Focus => {}
                }
            })
            .detach();
            inputs.push(input);
        }
        self.inputs = inputs;
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
        self.rebuild_inputs(window, cx);
        cx.emit(RulesEvent::Changed);
        cx.notify();
    }

    fn pick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled() || self.props.source.is_empty() {
            return;
        }
        self.picking = true;
        self.error = None;
        self.notice = None;
        cx.emit(RulesEvent::Picking(true));
        cx.notify();
        let title = messages(self.props.language).add_file;
        let source = PathBuf::from(&self.props.source);
        cx.spawn_in(window, async move |this, cx| {
            let dialog = rfd::AsyncFileDialog::new().set_title(title).set_directory(&source);
            let picked = pick_paths(dialog, PickKind::Files).await;
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
                        if add_rename_rules(&mut this.rules, paths) {
                            this.rebuild_inputs(window, cx);
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

impl Render for RenameRulesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = current_palette(cx);
        let m = messages(self.props.language);
        let narrow = window.viewport_size().width < px(900.);
        let disabled = self.disabled();
        let picker_disabled = disabled || self.props.source.is_empty();

        let mut list = rule_list();
        for (index, rule) in self.rules.iter().enumerate() {
            let invalid = !valid_rename_target(rule);
            let mut row = rule_row(p, narrow)
                .child(
                    Checkbox::new(("rename-enabled", index))
                        .checked(rule.enabled)
                        .disabled(disabled)
                        .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                            this.set_enabled(index, *checked, cx)
                        })),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .font_family(mono_font())
                        .text_size(px(13.))
                        .child(rule.path.clone()),
                )
                .child("→");
            if let Some(input) = self.inputs.get(index) {
                row = row.child(
                    div().w(px(200.)).flex_shrink_0().child(
                        Input::new(input)
                            .aria_label(SharedString::from(format!(
                                "{}: {}",
                                m.rename_new_name, rule.path
                            )))
                            .disabled(disabled)
                            .bg(color(p.raised))
                            .border_color(color(p.border))
                            .text_color(color(p.text))
                            .rounded(px(8.))
                            .h(px(24.))
                            .px(px(2.))
                            .py(px(1.))
                            .text_size(px(14.)),
                    ),
                );
            }
            row = row.child(remove_button(("rename-remove", index), p, disabled).when(
                !disabled,
                |button| {
                    button.on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.remove(index, window, cx)
                    }))
                },
            ));
            if invalid {
                row = row.child(
                    div().text_size(px(12.)).text_color(color(p.danger)).child(m.rename_invalid),
                );
            }
            list = list.child(row);
        }

        let mut root = div()
            .flex()
            .flex_col()
            .child(heading(m.rename_title))
            .child(muted_paragraph(p, m.rename_hint))
            .child(list)
            .child(add_form(p).child(
                secondary_button("rename-add-file", format!("+ {}", m.add_file), p, picker_disabled)
                    .when(!picker_disabled, |button| {
                        button.on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.pick(window, cx)
                        }))
                    }),
            ));
        if let Some(error) = &self.error {
            root = root.child(status_block(p, p.danger).mt(px(8.)).child(error.clone()));
        }
        if let Some(notice) = &self.notice {
            root = root.child(muted_paragraph(p, notice.clone()));
        }
        root
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const P: &str = "config/txloader/load/BetterQuesting[betterquesting]/lang/ru_RU.lang";

    fn rule(path: &str, to: &str, enabled: bool) -> RenameRule {
        RenameRule { path: path.into(), to: to.into(), enabled }
    }

    #[test]
    fn default_rename_target_swaps_ru_ru_for_en_us_and_otherwise_keeps_the_name() {
        assert_eq!(default_rename_target(P), "en_US.lang");
        assert_eq!(default_rename_target("config/a/other.lang"), "other.lang");
        assert_eq!(rename_rule_for(P), rule(P, "en_US.lang", true));
    }

    #[test]
    fn rename_target_validity_non_empty_one_component_different_from_file_name() {
        assert!(valid_rename_target(&rule(P, "en_US.lang", true)));
        assert!(!valid_rename_target(&rule(P, "", true)));
        assert!(!valid_rename_target(&rule(P, "a/b.lang", true)));
        assert!(!valid_rename_target(&rule(P, "a\\b.lang", true)));
        assert!(!valid_rename_target(&rule(P, "RU_ru.LANG", true)));
        assert!(!valid_rename_target(&rule("other.lang", "other.lang", true)));
        assert!(!valid_rename_target(&rule(P, " en_US.lang", true)));
        assert!(!renames_valid(&[rule(P, "en_US.lang", true), rule(P, "", false)]));
        assert!(renames_valid(&[rule(P, "en_US.lang", true)]));
    }

    #[test]
    fn added_rename_file_with_default_name_is_invalid_until_renamed() {
        let mut rules = Vec::new();
        assert!(add_rename_rules(&mut rules, vec!["extra.cfg".into()]));
        assert_eq!(rules, vec![rule("extra.cfg", "extra.cfg", true)]);
        assert!(!renames_valid(&rules));
        assert!(!add_rename_rules(&mut rules, vec!["EXTRA.cfg".into()]));
        rules[0].to = "extra.old".into();
        assert!(renames_valid(&rules));
    }
}
