//! Placeholder for the release catalog; replaced by the real implementation on merge.
#![allow(dead_code)] // the stub never emits its events or reads its props

use gpui::{div, prelude::*, Context, EventEmitter, IntoElement, Render, Window};

use crate::flow::Mode;
use crate::theme::{color, current_palette};

pub struct CatalogProps {
    pub language: gtnh_core::Language,
    pub include_prereleases: bool,
    pub can_install: bool,
}

pub enum CatalogEvent {
    Install(gtnh_core::releases::Artifact),
    Summary(String),
}

pub struct CatalogView {
    _mode: Mode,
}

impl CatalogView {
    pub fn new(mode: Mode, _window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self { _mode: mode }
    }

    pub fn set_props(&mut self, _props: &CatalogProps, _window: &mut Window, _cx: &mut Context<Self>) {}
}

impl EventEmitter<CatalogEvent> for CatalogView {}

impl Render for CatalogView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().text_color(color(current_palette(cx).muted)).child("…")
    }
}
