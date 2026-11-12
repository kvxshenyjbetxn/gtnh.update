//! Rule editors ported from `TransferRules.svelte`, `ModRules.svelte` and `RenameRules.svelte`.
//! Each editor is a GPUI entity that owns its rules and reports edits through `RulesEvent`.

pub mod mods;
pub mod renames;
pub mod transfer;

use std::path::PathBuf;

use gpui::{div, prelude::*, px, Div, ElementId, FontWeight, SharedString, Stateful};
use gtnh_core::Language;

use crate::theme::{color, mono_font, Palette};

/// Properties the parent pushes into an editor (`source` is the mods directory for `ModRulesView`).
#[derive(Clone, PartialEq)]
pub struct RulesProps {
    pub source: String,
    pub language: Language,
    pub disabled: bool,
}

impl Default for RulesProps {
    fn default() -> Self {
        Self { source: String::new(), language: Language::Uk, disabled: true }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum RulesEvent {
    /// A user edit changed the rules; the parent reads `rules()`.
    Changed,
    /// A native dialog opened (`true`) or closed (`false`).
    Picking(bool),
}

#[derive(Clone, Copy)]
pub enum PickKind {
    Files,
    File,
    Folders,
}

/// Opens a native dialog and returns the chosen paths (cancel or empty selection gives `None`).
pub async fn pick_paths(dialog: rfd::AsyncFileDialog, kind: PickKind) -> Option<Vec<PathBuf>> {
    let paths: Option<Vec<PathBuf>> = match kind {
        PickKind::Files => dialog
            .pick_files()
            .await
            .map(|list| list.into_iter().map(|handle| handle.path().to_path_buf()).collect()),
        PickKind::Folders => dialog
            .pick_folders()
            .await
            .map(|list| list.into_iter().map(|handle| handle.path().to_path_buf()).collect()),
        PickKind::File => dialog.pick_file().await.map(|handle| vec![handle.path().to_path_buf()]),
    };
    paths.filter(|list| !list.is_empty())
}

// --- shared view pieces (CSS: .rule-row, .tag, .remove, .add-form, .picker-actions, h4) ----------

/// `.rule-row`: flex row, 1px top border, 12px vertical padding, min height 48.
pub fn rule_row(p: &Palette, narrow: bool) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(if narrow { 7. } else { 10. }))
        .py(px(12.))
        .min_h(px(48.))
        .border_t_1()
        .border_color(color(p.border))
}

/// `.rule-list`.
pub fn rule_list() -> Div {
    div().mt(px(20.))
}

/// `.tag`.
pub fn tag(p: &Palette, text: impl Into<SharedString>) -> Div {
    div()
        .flex_shrink_0()
        .text_size(px(10.))
        .text_color(color(p.muted))
        .bg(color(p.raised))
        .px(px(7.))
        .py(px(4.))
        .rounded(px(5.))
        .child(text.into())
}

/// `.remove` button ("×").
pub fn remove_button(id: impl Into<ElementId>, p: &Palette, disabled: bool) -> Stateful<Div> {
    let muted = color(p.muted);
    let el = div()
        .id(id)
        .flex_shrink_0()
        .px(px(8.))
        .py(px(2.))
        .border_1()
        .border_color(color(p.border))
        .rounded(px(8.))
        .bg(color(p.raised))
        .text_size(px(20.))
        .text_color(muted)
        .child("×");
    if disabled {
        el.opacity(0.45)
    } else {
        el.cursor_pointer().hover(move |style| style.border_color(muted))
    }
}

/// `.add-form > .picker-actions`.
pub fn add_form(p: &Palette) -> Div {
    div()
        .mt(px(10.))
        .pt(px(20.))
        .border_t_1()
        .border_color(color(p.border))
        .flex()
        .flex_row()
        .flex_wrap()
        .gap(px(10.))
}

/// `h4` heading of a rules block (bold, 14px, margin 0 0 8px).
pub fn heading(text: impl Into<SharedString>) -> Div {
    div().font_weight(FontWeight::BOLD).mb(px(8.)).child(text.into())
}

/// Monospace `code` text, 12px.
pub fn code(text: impl Into<SharedString>) -> Div {
    div().font_family(mono_font()).text_size(px(12.)).child(text.into())
}

/// `.transfer-summary`: "label: <strong>n</strong> / total".
pub fn summary(label: &str, enabled: usize, total: usize) -> Div {
    div()
        .flex()
        .flex_row()
        .text_size(px(13.))
        .mb(px(13.))
        .child(format!("{label}: "))
        .child(div().font_weight(FontWeight::BOLD).child(enabled.to_string()))
        .child(format!(" / {total}"))
}
