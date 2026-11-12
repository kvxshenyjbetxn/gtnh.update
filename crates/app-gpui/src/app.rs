use std::path::{Path, PathBuf};

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

use crate::i18n::{messages, Messages};
use crate::rules::{
    mods::ModRulesView, renames::renames_valid, renames::RenameRulesView,
    transfer::TransferRulesView, RulesEvent, RulesProps,
};
use crate::theme::{color, mono_font, palette, sync_component_theme, ui_font, Palette};
use crate::ui::{card, directory, field, muted_paragraph, paragraph, secondary_button, status_block};

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Client,
    Server,
    Settings,
}

type Items = SearchableVec<SharedString>;
type LabelSelect = SelectState<Items>;

/// Temporary client rule editors shown when `GTNH_GPUI_PREVIEW_RULES=1`; replaced by the wizard.
struct RulesPreview {
    transfer: Entity<TransferRulesView>,
    mods: Entity<ModRulesView>,
    renames: Entity<RenameRulesView>,
}

pub struct AppView {
    settings: Settings,
    /// Serialized settings as last written to (or read from) disk.
    saved: Option<serde_json::Value>,
    /// Serialized settings of the most recent save attempt; prevents retry loops after a failure.
    attempted: Option<serde_json::Value>,
    loaded: bool,
    loading: bool,
    saving: bool,
    picking: bool,
    tab: Tab,
    error: Option<String>,
    save_error: Option<String>,
    notice: Option<String>,
    dir: PathBuf,
    language_select: Entity<LabelSelect>,
    theme_select: Entity<LabelSelect>,
    preview: Option<RulesPreview>,
}

fn theme_items(m: &Messages) -> Items {
    SearchableVec::new(vec![SharedString::from(m.amoled), SharedString::from(m.light)])
}

impl AppView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = Settings::default();
        let m = messages(settings.language);
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

        let preview = (std::env::var("GTNH_GPUI_PREVIEW_RULES").as_deref() == Ok("1"))
            .then(|| Self::new_preview(window, cx));

        let mut view = Self {
            settings,
            saved: None,
            attempted: None,
            loaded: false,
            loading: true,
            saving: false,
            picking: false,
            tab: Tab::Settings,
            error: None,
            save_error: None,
            notice: None,
            dir: dirs::config_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("app.gtnh.updater"),
            language_select,
            theme_select,
            preview,
        };
        view.load(window, cx);
        view
    }

    fn new_preview(window: &mut Window, cx: &mut Context<Self>) -> RulesPreview {
        let transfer = cx.new(|cx| TransferRulesView::new(false, window, cx));
        let mods = cx.new(|cx| ModRulesView::new(window, cx));
        let renames = cx.new(|cx| RenameRulesView::new(window, cx));
        cx.subscribe(&transfer, |this, editor, event: &RulesEvent, cx| match event {
            RulesEvent::Changed => {
                this.settings.client_rules = editor.read(cx).rules().to_vec();
                this.changed(cx);
            }
            RulesEvent::Picking(picking) => this.set_picking(*picking, cx),
        })
        .detach();
        cx.subscribe(&mods, |this, editor, event: &RulesEvent, cx| match event {
            RulesEvent::Changed => {
                this.settings.client_mods = editor.read(cx).rules().to_vec();
                this.changed(cx);
            }
            RulesEvent::Picking(picking) => this.set_picking(*picking, cx),
        })
        .detach();
        cx.subscribe(&renames, |this, editor, event: &RulesEvent, cx| match event {
            RulesEvent::Changed => {
                this.settings.client_renames = editor.read(cx).rules().to_vec();
                this.changed(cx);
            }
            RulesEvent::Picking(picking) => this.set_picking(*picking, cx),
        })
        .detach();
        RulesPreview { transfer, mods, renames }
    }

    fn set_picking(&mut self, picking: bool, cx: &mut Context<Self>) {
        self.picking = picking;
        cx.notify();
    }

    /// Pushes the loaded rules into the preview editors.
    fn sync_preview_rules(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(preview) = &self.preview else { return };
        let (rules, mods, renames) = (
            self.settings.client_rules.clone(),
            self.settings.client_mods.clone(),
            self.settings.client_renames.clone(),
        );
        preview.transfer.update(cx, |view, cx| view.set_rules(rules, window, cx));
        preview.mods.update(cx, |view, cx| view.set_rules(mods, window, cx));
        preview.renames.update(cx, |view, cx| view.set_rules(renames, window, cx));
    }

    /// Pushes source / language / disabled into the preview editors (cheap; they notify only on change).
    fn sync_preview_props(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(preview) = &self.preview else { return };
        let source = self.settings.client_source.clone().unwrap_or_default();
        let mods_directory = if source.is_empty() {
            String::new()
        } else {
            Path::new(&source).join("mods").to_string_lossy().into_owned()
        };
        let language = self.settings.language;
        let disabled = !self.loaded || self.picking;
        let props = RulesProps { source, language, disabled };
        let mods_props = RulesProps { source: mods_directory, ..props.clone() };
        preview.transfer.update(cx, |view, cx| view.set_props(&props, window, cx));
        preview.mods.update(cx, |view, cx| view.set_props(&mods_props, window, cx));
        preview.renames.update(cx, |view, cx| view.set_props(&props, window, cx));
    }

    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.loading = true;
        self.loaded = false;
        self.error = None;
        self.notice = None;
        cx.notify();
        let dir = self.dir.clone();
        let task = cx.background_spawn(async move {
            gtnh_core::load_settings(&dir).map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                this.loading = false;
                match result {
                    Ok(settings) => {
                        let snapshot = serde_json::to_value(&settings).ok();
                        this.saved = snapshot.clone();
                        this.attempted = snapshot;
                        this.settings = settings;
                        this.loaded = true;
                        sync_component_theme(this.settings.theme, cx);
                        this.sync_selects(window, cx);
                        this.sync_preview_rules(window, cx);
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
        self.attempted = Some(snapshot.clone());
        let dir = self.dir.clone();
        let settings = self.settings.clone();
        let task = cx.background_spawn(async move {
            gtnh_core::save_settings(&dir, &settings).map_err(|error| error.to_string())
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.saving = false;
                match result {
                    Ok(()) => {
                        this.saved = Some(snapshot);
                        this.save_error = None;
                        this.notice = Some(messages(this.settings.language).saved.to_string());
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

    fn switch_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        if self.picking {
            return;
        }
        self.tab = tab;
        self.notice = None;
        cx.notify();
    }

    fn pick_prism_directory(&mut self, cx: &mut Context<Self>) {
        if self.picking || !self.loaded {
            return;
        }
        self.picking = true;
        self.error = None;
        self.notice = None;
        let m = messages(self.settings.language);
        let mut initial: Option<PathBuf> = None;
        if let Some(current) = self.settings.prism_instances_directory.clone() {
            match gtnh_core::selection::instances_directory(Path::new(&current)) {
                Ok(path) => initial = Some(PathBuf::from(path)),
                Err(error) => self.error = Some(format!("{}\n{}", m.prism_unavailable, error)),
            }
        }
        cx.notify();
        let title = m.prism_instances;
        cx.spawn(async move |this, cx| {
            let mut dialog = rfd::AsyncFileDialog::new().set_title(title);
            if let Some(directory) = initial {
                dialog = dialog.set_directory(directory);
            }
            let picked = dialog.pick_folder().await;
            this.update(cx, |this, cx| {
                if let Some(handle) = picked {
                    match gtnh_core::selection::instances_directory(handle.path()) {
                        Ok(path) => {
                            this.settings.prism_instances_directory = Some(path);
                            this.changed(cx);
                        }
                        Err(error) => this.error = Some(error.to_string()),
                    }
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
        let result = std::fs::create_dir_all(&self.dir)
            .map_err(|error| error.to_string())
            .and_then(|_| open::that_detached(&self.dir).map_err(|error| error.to_string()));
        if let Err(error) = result {
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
        self.sync_preview_props(window, cx);
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
                nav.child(if self.picking {
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
            content = content.child(self.render_settings(p, m, narrow, cx));
        } else if let (Tab::Client, Some(preview)) = (self.tab, &self.preview) {
            content = content
                .child(preview.transfer.clone())
                .child(div().mt(px(24.)).child(preview.mods.clone()))
                .child(div().mt(px(24.)).child(preview.renames.clone()));
        } else {
            content = content.child(status_block(p, p.muted).child(m.prototype_pending));
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

    fn render_settings(
        &self,
        p: &'static Palette,
        m: &'static Messages,
        narrow: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let disabled = !self.loaded || self.picking;
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
                            disabled,
                        )
                        .when(!disabled, |button| {
                            button.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.pick_prism_directory(cx)
                            }))
                        }),
                    )
                    .child(
                        secondary_button("prism-clear", m.clear, p, disabled || prism.is_none())
                            .when(!disabled && prism.is_some(), |button| {
                                button.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.clear_prism_directory(cx)
                                }))
                            }),
                    ),
            );

        let storage = card(p, narrow, m.storage)
            .child(muted_paragraph(p, m.storage_hint).mb(px(0.)))
            .child(directory(p, self.dir.display().to_string()))
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
