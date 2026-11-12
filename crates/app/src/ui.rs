use gpui::{div, prelude::*, px, relative, Div, ElementId, FontWeight, SharedString, Stateful};

use crate::theme::{color, mono_font, Palette};

/// `button` rule from style.css: padding 10px 15px, 1px border, radius 8, raised bg.
/// Hover border and click handling are added only when enabled; the caller attaches `on_click`.
pub fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    p: &Palette,
    secondary: bool,
    primary: bool,
    disabled: bool,
) -> Stateful<gpui::Div> {
    let muted = color(p.muted);
    let (bg, fg, border) = if primary {
        (color(p.accent), color(p.accent_text), color(p.accent))
    } else {
        (color(p.raised), color(p.text), color(p.border))
    };
    let mut el = div()
        .id(id)
        .px(px(15.))
        .py(px(10.))
        .border_1()
        .border_color(border)
        .rounded(px(8.))
        .bg(bg)
        .text_color(fg)
        .child(label.into());
    if secondary {
        el = el.text_size(px(12.));
    }
    if primary {
        el = el.font_weight(FontWeight::SEMIBOLD);
    }
    if disabled {
        el.opacity(0.45)
    } else {
        el.cursor_pointer().hover(move |s| s.border_color(muted))
    }
}

pub fn secondary_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    p: &Palette,
    disabled: bool,
) -> Stateful<gpui::Div> {
    button(id, label, p, true, false, disabled)
}

pub fn primary_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    p: &Palette,
    disabled: bool,
) -> Stateful<gpui::Div> {
    button(id, label, p, false, true, disabled)
}

// --- view helpers ---------------------------------------------------------------------------

/// `.notice` / `.error` rule from style.css.
pub fn status_block(p: &Palette, text_color: u32) -> Div {
    div()
        .border_1()
        .border_color(color(p.border))
        .rounded(px(10.))
        .px(px(18.))
        .py(px(15.))
        .mb(px(20.))
        .text_size(px(12.))
        .line_height(relative(1.6))
        .text_color(color(text_color))
}

/// Paragraph with `line-height: 1.65`.
pub fn paragraph(text: impl Into<SharedString>) -> Div {
    div().line_height(relative(1.65)).child(text.into())
}

pub fn muted_paragraph(p: &Palette, text: impl Into<SharedString>) -> Div {
    paragraph(text)
        .text_size(px(12.))
        .text_color(color(p.muted))
        .mb(px(12.))
}

pub fn card(p: &Palette, narrow: bool, title: &'static str) -> Div {
    div()
        .flex()
        .flex_col()
        .bg(color(p.panel))
        .border_1()
        .border_color(color(p.border))
        .rounded(px(12.))
        .p(px(if narrow { 18. } else { 22. }))
        .mb(px(20.))
        .child(
            div()
                .text_size(px(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .line_height(relative(1.2))
                .mb(px(14.))
                .child(title),
        )
}

/// `code.directory` rule: raised block, monospace 12px, wraps anywhere.
pub fn directory(p: &Palette, text: String) -> Div {
    div()
        .bg(color(p.raised))
        .p(px(12.))
        .rounded(px(7.))
        .my(px(16.))
        .text_size(px(12.))
        .font_family(mono_font())
        .child(text)
}

pub fn field(label: &'static str) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(10.))
        .text_size(px(12.))
        .child(label)
}

/// `progress` element: a track with an accent fill. `total == 0` is indeterminate (a fixed
/// partial fill, since the native element's animation has no equivalent here).
pub fn progress_bar(p: &Palette, done: usize, total: usize) -> Div {
    let fraction = if total == 0 {
        0.3
    } else {
        (done as f32 / total as f32).clamp(0., 1.)
    };
    div()
        .w_full()
        .h(px(12.))
        .rounded(px(6.))
        .bg(color(p.raised))
        .overflow_hidden()
        .child(div().h_full().w(relative(fraction)).bg(color(p.accent)))
}

/// `pre`: wrapped monospace 12px text.
pub fn pre(text: impl Into<SharedString>) -> Div {
    div()
        .font_family(mono_font())
        .text_size(px(12.))
        .child(text.into())
}

/// A left-aligned (not stretched) child of a column.
pub fn aligned(el: impl IntoElement) -> Div {
    div().flex().flex_row().child(el)
}
