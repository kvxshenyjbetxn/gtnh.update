//! Port of `Catalog.svelte`: the official release list with per-release downloads.

mod download_item;

use std::collections::{HashMap, HashSet};

use futures::StreamExt as _;
use gpui::{
    div, prelude::*, px, ClickEvent, Context, ElementId, Entity, EventEmitter, FontWeight,
    IntoElement, Render, SharedString, Window,
};
use gpui_component::{
    select::{SearchableVec, Select, SelectEvent, SelectState},
    IndexPath,
};
use gtnh_core::{
    releases::{Artifact, Channel, Release},
    Language,
};

use crate::backend::{Backend, DownloadState, DownloadStatus};
use crate::flow::Mode;
use crate::i18n::{messages, Messages};
use crate::logic::{downloads_for_artifacts, DownloadTracker};
use crate::theme::{color, current_palette};
use crate::ui::{muted_paragraph, secondary_button, status_block};

type Items = SearchableVec<SharedString>;
type JavaSelect = SelectState<Items>;

#[derive(Clone, PartialEq, Debug)]
pub struct CatalogProps {
    pub language: Language,
    pub include_prereleases: bool,
    pub can_install: bool,
}

pub enum CatalogEvent {
    Install(Artifact),
    Summary(String),
}

pub struct CatalogView {
    mode: Mode,
    props: CatalogProps,
    releases: Vec<Release>,
    busy: bool,
    error: Option<String>,
    /// One Java variant select per release version (only releases with artifacts of this mode).
    selects: HashMap<String, Entity<JavaSelect>>,
    tracker: DownloadTracker,
    statuses: Vec<DownloadStatus>,
    download_ready: bool,
    /// Download ids whose cancellation was requested and not yet reflected in a status.
    cancelling: HashSet<String>,
    /// URLs whose "delete archive?" confirmation is open.
    confirming: HashSet<String>,
    file_errors: HashMap<String, String>,
    last_summary: String,
}

impl EventEmitter<CatalogEvent> for CatalogView {}

/// Text of the summary line (port of the `$effect` in `Catalog.svelte`).
fn summarize(m: &Messages, statuses: &[DownloadStatus], urls: &HashSet<&str>) -> String {
    let jobs: Vec<&DownloadStatus> =
        statuses.iter().filter(|s| urls.contains(s.url.as_str())).collect();
    let running: Vec<&&DownloadStatus> =
        jobs.iter().filter(|j| j.state == DownloadState::Downloading).collect();
    let ready = jobs.iter().filter(|j| j.state == DownloadState::Completed).count();
    let failed = jobs.iter().filter(|j| j.state == DownloadState::Failed).count();
    let total: u64 = running.iter().map(|j| j.total.unwrap_or(0)).sum();
    let received: u64 = running.iter().map(|j| j.received).sum();
    let percent = if !running.is_empty() && running.iter().all(|j| j.total.unwrap_or(0) > 0) {
        format!(" ({}%)", (received.saturating_mul(100) / total).min(100))
    } else {
        String::new()
    };
    let mut parts = Vec::new();
    if !running.is_empty() {
        parts.push(format!("{}: {}{}", m.downloads_active, running.len(), percent));
    }
    if ready > 0 {
        parts.push(format!("{}: {}", m.downloads_ready, ready));
    }
    if failed > 0 {
        parts.push(format!("{}: {}", m.downloads_failed, failed));
    }
    parts.join(" · ")
}

impl CatalogView {
    pub fn new(mode: Mode, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let backend = Backend::get(cx);
        let mut tracker = DownloadTracker::new();
        let mut statuses = Vec::new();
        let mut download_ready = false;
        let mut error = None;
        match backend.watch_downloads() {
            Ok((mut receiver, snapshot)) => {
                if let Some(list) = tracker.accept_all(snapshot) {
                    statuses = list;
                }
                download_ready = true;
                cx.spawn(async move |this, cx| {
                    while let Some(status) = receiver.next().await {
                        if this.update(cx, |this, cx| this.accept_status(status, cx)).is_err() {
                            break;
                        }
                    }
                })
                .detach();
            }
            Err(e) => error = Some(e),
        }
        Self::begin_fetch(window, cx);
        Self {
            mode,
            props: CatalogProps {
                language: Language::Uk,
                include_prereleases: false,
                can_install: false,
            },
            releases: Vec::new(),
            busy: true,
            error,
            selects: HashMap::new(),
            tracker,
            statuses,
            download_ready,
            cancelling: HashSet::new(),
            confirming: HashSet::new(),
            file_errors: HashMap::new(),
            last_summary: String::new(),
        }
    }

    /// May be called from the parent's render: `cx.notify()` only when something changed.
    pub fn set_props(
        &mut self,
        props: &CatalogProps,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if &self.props == props {
            return;
        }
        self.props = props.clone();
        self.update_summary(cx);
        cx.notify();
    }

    fn update_summary(&mut self, cx: &mut Context<Self>) {
        let urls: HashSet<&str> = self
            .releases
            .iter()
            .flat_map(|r| r.artifacts.iter())
            .filter(|a| a.mode == self.mode.as_str())
            .map(|a| a.url.as_str())
            .collect();
        let summary = summarize(messages(self.props.language), &self.statuses, &urls);
        if summary != self.last_summary {
            self.last_summary = summary.clone();
            cx.emit(CatalogEvent::Summary(summary));
        }
    }

    fn accept_status(&mut self, status: DownloadStatus, cx: &mut Context<Self>) {
        let Some(list) = self.tracker.accept(status) else { return };
        self.statuses = list;
        let statuses = &self.statuses;
        self.cancelling.retain(|id| {
            statuses.iter().any(|s| &s.id == id && s.state == DownloadState::Downloading)
        });
        self.confirming.retain(|url| {
            statuses.iter().any(|s| &s.url == url && s.state == DownloadState::Completed)
        });
        self.update_summary(cx);
        cx.notify();
    }

    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        Self::begin_fetch(window, cx);
        cx.notify();
    }

    fn begin_fetch(window: &mut Window, cx: &mut Context<Self>) {
        let backend = Backend::get(cx);
        cx.spawn_in(window, async move |this, cx| {
            let result = backend.list_releases().await;
            let urls = result
                .as_ref()
                .map(|releases| {
                    releases
                        .iter()
                        .flat_map(|r| r.artifacts.iter().map(|a| a.url.clone()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(releases) => {
                        this.releases = releases;
                        this.rebuild_selects(window, cx);
                    }
                    Err(e) => this.error = Some(e),
                }
                this.update_summary(cx);
                cx.notify();
            })
            .ok();
            // Non-fatal; adopted archives arrive through the download events.
            if !urls.is_empty() {
                let _ = backend.adopt_downloads(urls).await;
            }
        })
        .detach();
    }

    fn rebuild_selects(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let previous: HashMap<String, usize> = self
            .selects
            .iter()
            .map(|(version, select)| (version.clone(), selected_row(select, cx)))
            .collect();
        let mut selects = HashMap::new();
        for release in &self.releases {
            let labels: Vec<SharedString> = release
                .artifacts
                .iter()
                .filter(|a| a.mode == self.mode.as_str())
                .map(|a| SharedString::from(a.java.clone()))
                .collect();
            if labels.is_empty() {
                continue;
            }
            let row = previous
                .get(&release.version)
                .copied()
                .filter(|r| *r < labels.len())
                .unwrap_or(0);
            let select = cx.new(|cx| {
                SelectState::new(SearchableVec::new(labels), Some(IndexPath::new(row)), window, cx)
            });
            cx.subscribe_in(&select, window, |_, _, _: &SelectEvent<Items>, _, cx| cx.notify())
                .detach();
            selects.insert(release.version.clone(), select);
        }
        self.selects = selects;
    }

    fn open_release(&mut self, url: String, cx: &mut Context<Self>) {
        if let Err(e) = Backend::get(cx).open_release_url(url) {
            self.error = Some(e);
            cx.notify();
        }
    }

    fn start_download(&mut self, url: String, cx: &mut Context<Self>) {
        let running = self
            .statuses
            .iter()
            .any(|s| s.url == url && s.state == DownloadState::Downloading);
        if running || !self.download_ready {
            return;
        }
        self.file_errors.remove(&url);
        if let Err(e) = Backend::get(cx).start_download(url.clone()) {
            self.file_errors.insert(url, e);
        }
        cx.notify();
    }

    fn cancel_download(&mut self, status: &DownloadStatus, cx: &mut Context<Self>) {
        if self.cancelling.contains(&status.id) {
            return;
        }
        self.cancelling.insert(status.id.clone());
        self.file_errors.remove(&status.url);
        if let Err(e) = Backend::get(cx).cancel_download(status.id.clone()) {
            self.file_errors.insert(status.url.clone(), e);
            self.cancelling.remove(&status.id);
        }
        cx.notify();
    }

    fn delete_archive(&mut self, url: String, cx: &mut Context<Self>) {
        self.file_errors.remove(&url);
        if let Err(e) = Backend::get(cx).delete_download(url.clone()) {
            self.file_errors.insert(url.clone(), e);
        }
        self.confirming.remove(&url);
        cx.notify();
    }
}

fn selected_row(select: &Entity<JavaSelect>, cx: &gpui::App) -> usize {
    select.read(cx).selected_index(cx).map(|i| i.row).unwrap_or(0)
}

impl Render for CatalogView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = current_palette(cx);
        let m = messages(self.props.language);
        let width = window.viewport_size().width;
        let column = width < px(620.);
        let mode = self.mode.as_str();
        let muted = color(p.muted);

        let mut root = div()
            .flex()
            .flex_col()
            .border_1()
            .border_color(color(p.border))
            .rounded(px(12.))
            .p(px(18.))
            .mt(px(20.))
            .font_weight(FontWeight::NORMAL)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(12.))
                    .child(
                        div()
                            .text_size(px(16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .line_height(gpui::relative(1.2))
                            .child(m.catalog),
                    )
                    .child(
                        secondary_button(
                            "catalog-refresh",
                            if self.busy { m.loading } else { m.refresh },
                            p,
                            self.busy,
                        )
                        .when(!self.busy, |button| {
                            button.on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.refresh(window, cx)
                            }))
                        }),
                    ),
            );
        if let Some(error) = &self.error {
            root = root
                .child(status_block(p, p.danger).mt(px(12.)).mb(px(0.)).child(error.clone()));
        }

        let visible: Vec<&Release> = self
            .releases
            .iter()
            .filter(|r| self.props.include_prereleases || r.channel == Channel::Stable)
            .collect();
        if visible.is_empty() && !self.busy && self.error.is_none() {
            root = root.child(div().mt(px(12.)).child(muted_paragraph(p, m.empty)));
        }

        let mut list = div()
            .id("release-list")
            .flex()
            .flex_col()
            .mt(px(12.))
            .mb(px(12.))
            .max_h(px(360.))
            .overflow_y_scroll();
        for (row, release) in visible.iter().enumerate() {
            let artifacts: Vec<&Artifact> =
                release.artifacts.iter().filter(|a| a.mode == mode).collect();
            let select = self.selects.get(&release.version);
            let index = select.map(|s| selected_row(s, cx)).unwrap_or(0);
            let artifact = artifacts.get(index).or(artifacts.first()).copied();
            let jobs = downloads_for_artifacts(&self.statuses, &release.artifacts, mode);

            let mut actions = div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(8.))
                .when(!column, |d| d.justify_end())
                .when(width < px(740.) && !column, |d| d.max_w(px(180.)));
            if let Some(artifact) = artifact {
                if let Some(select) = select {
                    actions = actions.child(
                        Select::new(select)
                            .bg(color(p.raised))
                            .border_color(color(p.border))
                            .text_color(color(p.text))
                            .rounded(px(8.))
                            .w(px(110.))
                            .h(px(28.))
                            .px(px(6.))
                            .py(px(6.))
                            .text_size(px(11.)),
                    );
                }
                let running = self
                    .statuses
                    .iter()
                    .any(|s| s.url == artifact.url && s.state == DownloadState::Downloading);
                let disabled = !self.download_ready || running;
                let url = artifact.url.clone();
                actions = actions.child(
                    secondary_button(
                        SharedString::from(format!("download:{}", release.version)),
                        m.download,
                        p,
                        disabled,
                    )
                    .when(!disabled, |button| {
                        button.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.start_download(url.clone(), cx)
                        }))
                    }),
                );
            }
            let page_url = release.page_url.clone();
            actions = actions.child(
                div()
                    .id(SharedString::from(format!("notes:{}", release.version)))
                    .p(px(8.))
                    .text_size(px(11.))
                    .text_color(muted)
                    .cursor_pointer()
                    .hover(move |s| s.text_color(color(p.text)))
                    .child(format!("{} ↗", m.release_page))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.open_release(page_url.clone(), cx)
                    })),
            );

            let main = div()
                .flex()
                .justify_between()
                .gap(px(16.))
                .map(|d| {
                    if column {
                        d.flex_col()
                    } else if width < px(740.) {
                        d.items_start()
                    } else {
                        d.items_center()
                    }
                })
                .child(
                    div()
                        .child(
                            div()
                                .text_size(px(14.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(release.version.clone()),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(10.))
                                .mt(px(9.))
                                .child(crate::rules::tag(
                                    p,
                                    if release.channel == Channel::Stable {
                                        m.stable
                                    } else {
                                        m.beta_label
                                    },
                                ))
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(muted)
                                        .child(release.date.clone()),
                                ),
                        ),
                )
                .child(actions);

            let mut article = div()
                .id(ElementId::from(SharedString::from(format!("release:{row}"))))
                .flex()
                .flex_col()
                .border_t_1()
                .border_color(color(p.border))
                .px(px(4.))
                .py(px(16.))
                .child(main);
            for (job_artifact, status) in &jobs {
                article =
                    article.child(download_item::download_item(self, job_artifact, status, cx));
            }
            if let Some(artifact) = artifact {
                if let Some(error) = self.file_errors.get(&artifact.url).filter(|e| !e.is_empty())
                {
                    if !jobs.iter().any(|(_, s)| s.url == artifact.url) {
                        article = article.child(
                            status_block(p, p.danger).mt(px(12.)).mb(px(0.)).child(error.clone()),
                        );
                    }
                }
            }
            list = list.child(article);
        }
        root.child(list)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(url: &str, state: DownloadState, received: u64, total: Option<u64>) -> DownloadStatus {
        DownloadStatus {
            id: url.into(),
            revision: 1,
            url: url.into(),
            state,
            received,
            total,
            path: None,
            error: None,
        }
    }

    #[test]
    fn summary_counts_only_known_urls_and_computes_percent() {
        let m = messages(Language::En);
        let urls: HashSet<&str> = ["a", "b", "c", "d"].into_iter().collect();
        let statuses = vec![
            status("a", DownloadState::Downloading, 25, Some(100)),
            status("b", DownloadState::Downloading, 75, Some(100)),
            status("c", DownloadState::Completed, 1, Some(1)),
            status("d", DownloadState::Failed, 0, None),
            status("zzz", DownloadState::Completed, 1, Some(1)),
        ];
        assert_eq!(
            summarize(m, &statuses, &urls),
            format!(
                "{}: 2 (50%) · {}: 1 · {}: 1",
                m.downloads_active, m.downloads_ready, m.downloads_failed
            )
        );
    }

    #[test]
    fn summary_omits_percent_when_a_total_is_unknown_and_is_empty_when_idle() {
        let m = messages(Language::En);
        let urls: HashSet<&str> = ["a", "b"].into_iter().collect();
        let statuses = vec![
            status("a", DownloadState::Downloading, 25, Some(100)),
            status("b", DownloadState::Downloading, 5, None),
        ];
        assert_eq!(summarize(m, &statuses, &urls), format!("{}: 2", m.downloads_active));
        assert_eq!(summarize(m, &[], &urls), "");
        let cancelled = vec![status("a", DownloadState::Cancelled, 0, None)];
        assert_eq!(summarize(m, &cancelled, &urls), "");
    }
}
