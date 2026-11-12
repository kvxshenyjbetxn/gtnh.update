//! Port of `UpdateSection.svelte`: one numbered, collapsible wizard section.

use gpui::{
    div, prelude::*, px, relative, App, ClickEvent, Div, ElementId, FontWeight, SharedString,
    Window,
};

use crate::theme::{color, Palette};

pub struct SectionProps {
    pub id: ElementId,
    pub number: u8,
    pub title: SharedString,
    pub summary: String,
    pub expanded: bool,
    pub locked: bool,
    /// Shown before the summary while the section is locked.
    pub locked_reason: Option<&'static str>,
    /// Viewport narrower than 740px (smaller toggle paddings, content padding).
    pub very_narrow: bool,
}

/// Header plus (when expanded) `content`; build the content only for expanded sections.
pub fn update_section(
    p: &Palette,
    props: SectionProps,
    on_toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    content: Option<Div>,
) -> Div {
    let SectionProps {
        id,
        number,
        title,
        summary,
        expanded,
        locked,
        locked_reason,
        very_narrow,
    } = props;
    let muted = color(p.muted);
    let summary = match locked_reason {
        Some(reason) if locked => format!("{reason} · {summary}"),
        _ => summary,
    };

    let number_circle = div()
        .flex_shrink_0()
        .w(px(32.))
        .h(px(32.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .border_1()
        .border_color(if expanded {
            color(p.accent)
        } else {
            color(p.border)
        })
        .when(expanded, |el| el.bg(color(p.info)))
        .text_size(px(13.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(color(p.accent))
        .child(number.to_string());

    let label = div()
        .flex_1()
        .min_w_0()
        .text_color(if locked { muted } else { color(p.text) })
        .child(
            div()
                .text_size(px(if very_narrow { 15. } else { 16. }))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(
            div()
                .mt(px(4.))
                .text_size(px(12.))
                .line_height(relative(1.7))
                .font_weight(FontWeight::NORMAL)
                .text_color(muted)
                .child(summary),
        );

    let mut toggle = div()
        .id(id)
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(if very_narrow { 12. } else { 16. }))
        .p(px(if very_narrow { 18. } else { 22. }))
        .child(number_circle)
        .child(label)
        .child(
            div()
                .text_color(muted)
                .text_size(px(20.))
                .child(if expanded { "−" } else { "+" }),
        );
    if !locked {
        toggle = toggle.cursor_pointer().on_click(on_toggle);
    }

    let mut section = div()
        .flex()
        .flex_col()
        .min_w_0()
        .overflow_hidden()
        .mb(px(14.))
        .rounded(px(14.))
        .border_1()
        .border_color(if expanded {
            color(p.accent)
        } else {
            color(p.border)
        })
        .bg(color(p.panel))
        .child(toggle);
    if expanded {
        if let Some(content) = content {
            section = section.child(
                div()
                    .min_w_0()
                    .pt(px(16.))
                    .pb(px(24.))
                    .pr(px(24.))
                    .pl(px(if very_narrow { 24. } else { 70. }))
                    .child(content),
            );
        }
    }
    section
}

/// `.step-actions`: right-aligned row above which a border separates the buttons.
pub fn step_actions(p: &Palette) -> Div {
    div()
        .flex()
        .flex_row()
        .flex_wrap()
        .items_center()
        .justify_end()
        .gap(px(18.))
        .border_t_1()
        .border_color(color(p.border))
        .pt(px(20.))
        .mt(px(24.))
}
