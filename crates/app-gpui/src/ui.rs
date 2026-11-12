use gpui::{
    div, prelude::*, px, ElementId, FontWeight, SharedString, Stateful,
};

use crate::theme::{color, Palette};

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

#[allow(dead_code)]
pub fn primary_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    p: &Palette,
    disabled: bool,
) -> Stateful<gpui::Div> {
    button(id, label, p, false, true, disabled)
}
