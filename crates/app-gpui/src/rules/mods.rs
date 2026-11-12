//! Port of `ModRules.svelte` and `mod-rules.ts`.

use std::path::PathBuf;

use gpui::{div, prelude::*, px, ClickEvent, Context, EventEmitter, FontWeight, IntoElement, Render, Window};
use gpui_component::{checkbox::Checkbox, Disableable as _};
use gtnh_core::mods::ModInfo;
use gtnh_core::{ModAction, ModRule};

use super::{
    add_form, code, heading, pick_paths, remove_button, rule_list, rule_row, summary, tag,
    PickKind, RulesEvent, RulesProps,
};
use crate::i18n::messages;
use crate::theme::{color, current_palette};
use crate::ui::{muted_paragraph, secondary_button, status_block};

/// Same identity: shared mod ids, or (when either side has no ids) the same file stem.
pub fn same_mod(a: &ModRule, b: &ModRule) -> bool {
    if !a.mod_ids.is_empty() && !b.mod_ids.is_empty() {
        let ids: Vec<String> = a.mod_ids.iter().map(|id| id.to_lowercase()).collect();
        return b.mod_ids.iter().any(|id| ids.contains(&id.to_lowercase()));
    }
    a.stem.to_lowercase() == b.stem.to_lowercase()
}

pub fn mod_rule_from(info: &ModInfo, action: ModAction) -> ModRule {
    ModRule {
        action,
        enabled: true,
        name: info.name.clone(),
        mod_ids: info.mod_ids.clone(),
        stem: info.stem.clone(),
        file: if action == ModAction::Add { Some(info.file.clone()) } else { None },
    }
}

/// A new rule replaces an existing rule of the same action and identity (the user is updating a mod version).
pub fn upsert_mod_rule(rules: &[ModRule], next: ModRule) -> (Vec<ModRule>, bool) {
    let index = rules.iter().position(|rule| rule.action == next.action && same_mod(rule, &next));
    let mut rules = rules.to_vec();
    match index {
        None => {
            rules.push(next);
            (rules, false)
        }
        Some(index) => {
            rules[index] = next;
            (rules, true)
        }
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct PickOutcome {
    pub changed: bool,
    pub replaced: bool,
    pub existing: bool,
    pub errors: Vec<String>,
}

/// Applies inspected picker results to `mods`, mirroring the loop of `pickMods` in `ModRules.svelte`.
pub fn apply_picked(
    mods: &mut Vec<ModRule>,
    action: ModAction,
    results: Vec<(String, Result<ModInfo, String>)>,
) -> PickOutcome {
    let adding = action == ModAction::Add;
    let mut outcome = PickOutcome::default();
    for (path, result) in results {
        match result {
            Ok(info) => {
                let (rules, replaced) = upsert_mod_rule(mods, mod_rule_from(&info, action));
                if !adding && replaced {
                    outcome.existing = true;
                    continue;
                }
                *mods = rules;
                outcome.changed = true;
                outcome.replaced |= replaced;
            }
            Err(error) => outcome.errors.push(format!("{path}: {error}")),
        }
    }
    outcome
}

pub struct ModRulesView {
    mods: Vec<ModRule>,
    /// `props.source` is the mods directory.
    props: RulesProps,
    picking: bool,
    error: Option<String>,
    notice: Option<String>,
}

impl EventEmitter<RulesEvent> for ModRulesView {}

impl ModRulesView {
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self {
            mods: Vec::new(),
            props: RulesProps::default(),
            picking: false,
            error: None,
            notice: None,
        }
    }

    pub fn rules(&self) -> &[ModRule] {
        &self.mods
    }

    pub fn set_rules(&mut self, rules: Vec<ModRule>, _window: &mut Window, cx: &mut Context<Self>) {
        self.mods = rules;
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

    fn set_enabled(&mut self, index: usize, enabled: bool, cx: &mut Context<Self>) {
        if self.disabled() {
            return;
        }
        if let Some(rule) = self.mods.get_mut(index) {
            if rule.enabled != enabled {
                rule.enabled = enabled;
                cx.emit(RulesEvent::Changed);
                cx.notify();
            }
        }
    }

    fn remove(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.disabled() || index >= self.mods.len() {
            return;
        }
        self.mods.remove(index);
        cx.emit(RulesEvent::Changed);
        cx.notify();
    }

    fn pick(&mut self, action: ModAction, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled() {
            return;
        }
        self.picking = true;
        self.error = None;
        self.notice = None;
        cx.emit(RulesEvent::Picking(true));
        cx.notify();
        let m = messages(self.props.language);
        let adding = action == ModAction::Add;
        let title = if adding { m.add_mod_title } else { m.remove_mod_title };
        let directory = if adding || self.props.source.is_empty() {
            None
        } else {
            Some(PathBuf::from(&self.props.source))
        };
        cx.spawn_in(window, async move |this, cx| {
            let mut dialog = rfd::AsyncFileDialog::new()
                .set_title(title)
                .add_filter("Minecraft mod", &["jar"]);
            if let Some(directory) = directory {
                dialog = dialog.set_directory(directory);
            }
            let kind = if adding { PickKind::Files } else { PickKind::File };
            let picked = pick_paths(dialog, kind).await;
            let results = match picked {
                None => None,
                Some(paths) => Some(
                    cx.background_executor()
                        .spawn(async move {
                            paths
                                .into_iter()
                                .map(|path| {
                                    let inspected = gtnh_core::mods::inspect(&path);
                                    (path.to_string_lossy().into_owned(), inspected)
                                })
                                .collect::<Vec<_>>()
                        })
                        .await,
                ),
            };
            this.update(cx, |this, cx| {
                this.picking = false;
                if let Some(results) = results {
                    let m = messages(this.props.language);
                    let outcome = apply_picked(&mut this.mods, action, results);
                    this.error = Some(outcome.errors.join("\n")).filter(|text| !text.is_empty());
                    if outcome.changed {
                        cx.emit(RulesEvent::Changed);
                        if outcome.replaced {
                            this.notice = Some(m.mod_rule_updated.to_string());
                        }
                    } else if outcome.existing {
                        this.notice = Some(m.mod_rule_exists.to_string());
                    }
                }
                cx.emit(RulesEvent::Picking(false));
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

impl Render for ModRulesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = current_palette(cx);
        let m = messages(self.props.language);
        let narrow = window.viewport_size().width < px(900.);
        let disabled = self.disabled();
        let enabled = self.mods.iter().filter(|rule| rule.enabled).count();

        let mut list = rule_list();
        for (index, rule) in self.mods.iter().enumerate() {
            let mut main = div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(4.))
                .text_size(px(12.))
                .child(div().font_weight(FontWeight::BOLD).child(rule.name.clone()));
            if rule.mod_ids.is_empty() {
                main = main
                    .child(code(format!("{}: {}", m.mod_by_name, rule.stem)))
                    .child(div().text_color(color(p.muted)).child(m.mod_by_name_hint));
            } else {
                main = main.child(code(rule.mod_ids.join(", ")));
            }
            if rule.action == ModAction::Add {
                if let Some(file) = &rule.file {
                    main = main.child(
                        div()
                            .flex()
                            .flex_row()
                            .flex_wrap()
                            .gap(px(4.))
                            .text_color(color(p.muted))
                            .child(format!("{}:", m.mod_source_file))
                            .child(code(file.clone()).text_color(color(p.text))),
                    );
                }
            }
            let mut row = rule_row(p, narrow).child(
                Checkbox::new(("mod-enabled", index))
                    .checked(rule.enabled)
                    .disabled(disabled)
                    .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                        this.set_enabled(index, *checked, cx)
                    })),
            );
            if !narrow {
                row = row.child(tag(
                    p,
                    if rule.action == ModAction::Add { m.mod_action_add } else { m.mod_action_remove },
                ));
            }
            row = row.child(main).child(remove_button(("mod-remove", index), p, disabled).when(
                !disabled,
                |button| {
                    button.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.remove(index, cx)
                    }))
                },
            ));
            list = list.child(row);
        }

        let mut root = div()
            .flex()
            .flex_col()
            .child(heading(m.my_mods))
            .child(muted_paragraph(p, m.my_mods_hint))
            .child(summary(m.mods_enabled, enabled, self.mods.len()))
            .child(list)
            .child(
                add_form(p)
                    .child(
                        secondary_button("add-mod", format!("+ {}", m.add_mod), p, disabled)
                            .when(!disabled, |button| {
                                button.on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                    this.pick(ModAction::Add, window, cx)
                                }))
                            }),
                    )
                    .child(
                        secondary_button("remove-mod", format!("− {}", m.remove_mod), p, disabled)
                            .when(!disabled, |button| {
                                button.on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                    this.pick(ModAction::Remove, window, cx)
                                }))
                            }),
                    ),
            );
        if let Some(error) = &self.error {
            root = root.child(
                status_block(p, p.danger)
                    .mt(px(8.))
                    .child(div().font_family(crate::theme::mono_font()).child(error.clone())),
            );
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

    fn info(name: &str, ids: &[&str], stem: &str, file: &str) -> ModInfo {
        ModInfo {
            name: name.into(),
            mod_ids: ids.iter().map(|id| id.to_string()).collect(),
            stem: stem.into(),
            file: file.into(),
        }
    }

    fn files(rules: &[ModRule]) -> Vec<Option<&str>> {
        rules.iter().map(|rule| rule.file.as_deref()).collect()
    }

    #[test]
    fn upsert_mod_rule_replaces_the_same_identity_and_keeps_different_mods() {
        let a = mod_rule_from(&info("A", &["a"], "a-1", "/a-1.jar"), ModAction::Add);
        let a2 = mod_rule_from(&info("A", &["a"], "a-2", "/a-2.jar"), ModAction::Add);
        let b = mod_rule_from(&info("B", &[], "b", "/b.jar"), ModAction::Add);
        let b2 = mod_rule_from(&info("B", &[], "B", "/other/B.jar"), ModAction::Add);
        let (rules, replaced) = upsert_mod_rule(&[], a.clone());
        assert!(!replaced);
        let (rules, _) = upsert_mod_rule(&rules, b);
        assert_eq!(rules.len(), 2);
        let (rules, replaced) = upsert_mod_rule(&rules, a2);
        assert!(replaced);
        assert_eq!(files(&rules), vec![Some("/a-2.jar"), Some("/b.jar")]);
        let (rules, _) = upsert_mod_rule(&rules, b2);
        assert_eq!(files(&rules), vec![Some("/a-2.jar"), Some("/other/B.jar")]);
        let remove = mod_rule_from(&info("A", &["a"], "a", "/x"), ModAction::Remove);
        assert_eq!(upsert_mod_rule(&[a], remove).0.len(), 2);
    }

    #[test]
    fn remove_rule_has_no_file_and_mod_ids_compare_case_insensitively() {
        let remove = mod_rule_from(&info("J", &["JourneyMap"], "j", "/j.jar"), ModAction::Remove);
        assert_eq!(remove.file, None);
        assert!(remove.enabled);
        let other = mod_rule_from(&info("J", &["journeymap"], "x", "/x.jar"), ModAction::Remove);
        assert!(same_mod(&remove, &other));
    }

    #[test]
    fn picking_mods_inspects_each_jar_and_replaces_the_same_mod() {
        let mut mods = Vec::new();
        let first = vec![
            ("/d/alpha-1.0.jar".to_string(), Ok(info("alpha", &["alpha"], "alpha-1.0", "/d/alpha-1.0.jar"))),
            ("/d/beta-2.3.jar".to_string(), Ok(info("beta", &["beta"], "beta-2.3", "/d/beta-2.3.jar"))),
        ];
        let outcome = apply_picked(&mut mods, ModAction::Add, first);
        assert!(outcome.changed && !outcome.replaced);
        assert_eq!(files(&mods), vec![Some("/d/alpha-1.0.jar"), Some("/d/beta-2.3.jar")]);
        let second = vec![(
            "/e/alpha-1.1.jar".to_string(),
            Ok(info("alpha", &["alpha"], "alpha-1.1", "/e/alpha-1.1.jar")),
        )];
        let outcome = apply_picked(&mut mods, ModAction::Add, second);
        assert!(outcome.changed && outcome.replaced);
        assert_eq!(files(&mods), vec![Some("/e/alpha-1.1.jar"), Some("/d/beta-2.3.jar")]);
    }

    #[test]
    fn removing_an_already_listed_mod_reports_existing_and_inspect_failures_are_collected() {
        let mut mods = Vec::new();
        let jm = || Ok(info("jm", &["journeymap"], "journeymap-1", "/old/mods/journeymap-1.jar"));
        let outcome = apply_picked(&mut mods, ModAction::Remove, vec![("/old/mods/journeymap-1.jar".into(), jm())]);
        assert!(outcome.changed);
        assert_eq!(mods[0].action, ModAction::Remove);
        assert_eq!(mods[0].file, None);
        let outcome = apply_picked(&mut mods, ModAction::Remove, vec![("/old/mods/journeymap-1.jar".into(), jm())]);
        assert!(!outcome.changed && outcome.existing);
        assert_eq!(mods.len(), 1);
        let outcome = apply_picked(
            &mut mods,
            ModAction::Add,
            vec![("/d/readme.jar".into(), Err("not a mod".to_string()))],
        );
        assert!(!outcome.changed);
        assert_eq!(outcome.errors, vec!["/d/readme.jar: not a mod".to_string()]);
        assert_eq!(mods.len(), 1);
    }
}
