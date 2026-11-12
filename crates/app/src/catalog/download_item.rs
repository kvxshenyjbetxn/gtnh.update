//! Port of `DownloadItem.svelte`: one download row inside a release.

use gpui::{div, prelude::*, px, relative, ClickEvent, Context, Div, FontWeight, SharedString};
use gtnh_core::releases::Artifact;

use super::{CatalogEvent, CatalogView};
use crate::backend::{DownloadState, DownloadStatus};
use crate::i18n::messages;
use crate::logic::format_bytes;
use crate::theme::{color, current_palette, mono_font};
use crate::ui::{directory, primary_button, secondary_button, status_block};

pub(super) fn download_item(
    view: &CatalogView,
    artifact: &Artifact,
    status: &DownloadStatus,
    cx: &mut Context<CatalogView>,
) -> Div {
    let p = current_palette(cx);
    let m = messages(view.props.language);
    let language = view.props.language;
    let muted = color(p.muted);
    let starting = !view.download_ready;
    let cancelling = view.cancelling.contains(&status.id);
    let confirming = view.confirming.contains(&status.url);
    let id = |kind: &str| SharedString::from(format!("{kind}:{}", status.id));
    let bytes = |value: u64| format_bytes(value, language);
    let percent = status
        .total
        .filter(|t| *t > 0)
        .map(|t| (status.received.saturating_mul(100) / t).min(100));

    let details = || {
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap(px(10.))
            .mt(px(10.))
            .text_size(px(12.))
            .text_color(muted)
    };
    let message = |text: &'static str| {
        div()
            .text_size(px(12.))
            .line_height(relative(1.65))
            .child(text)
    };

    let mut item = div()
        .flex()
        .flex_col()
        .border_1()
        .border_color(color(p.border))
        .rounded(px(8.))
        .p(px(16.))
        .mt(px(12.))
        .child(
            div()
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .mb(px(6.))
                .child(artifact.java.clone()),
        )
        .child(
            div()
                .font_family(mono_font())
                .text_size(px(12.))
                .mb(px(12.))
                .child(status.url.rsplit('/').next().unwrap_or("").to_string()),
        );

    match status.state {
        DownloadState::Downloading => {
            let fraction = match status.total {
                Some(t) if t > 0 => (status.received as f32 / t as f32).clamp(0., 1.),
                _ => 0.,
            };
            let mut label = bytes(status.received);
            if let (Some(total), Some(percent)) = (status.total.filter(|t| *t > 0), percent) {
                label = format!("{label} / {} ({percent}%)", bytes(total));
            }
            let target = status.clone();
            item = item
                .child(
                    div()
                        .w_full()
                        .h(px(12.))
                        .rounded(px(6.))
                        .bg(color(p.border))
                        .overflow_hidden()
                        .child(div().h_full().w(relative(fraction)).bg(color(p.accent))),
                )
                .child(
                    details().child(label).child(
                        secondary_button(
                            id("cancel"),
                            if cancelling {
                                m.cancelling
                            } else {
                                m.cancel_download
                            },
                            p,
                            cancelling,
                        )
                        .when(!cancelling, |button| {
                            button.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.cancel_download(&target, cx)
                            }))
                        }),
                    ),
                );
        }
        DownloadState::Completed => {
            let can_install = view.props.can_install;
            let install_disabled = !can_install || starting;
            let install = artifact.clone();
            let url = status.url.clone();
            let mut row = details().child(
                primary_button(id("install"), m.select_pack, p, install_disabled).when(
                    !install_disabled,
                    |button| {
                        button.on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                            cx.emit(CatalogEvent::Install(install.clone()))
                        }))
                    },
                ),
            );
            if confirming {
                let delete_url = url.clone();
                let cancel_url = url.clone();
                row = row
                    .child(m.confirm_delete_archive)
                    .child(
                        secondary_button(id("delete"), m.delete_archive, p, false).on_click(
                            cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.delete_archive(delete_url.clone(), cx)
                            }),
                        ),
                    )
                    .child(
                        secondary_button(id("keep"), m.cancel_download, p, false).on_click(
                            cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.confirming.remove(&cancel_url);
                                cx.notify();
                            }),
                        ),
                    );
            } else {
                row = row.child(
                    secondary_button(id("delete"), m.delete_archive, p, false).on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.confirming.insert(url.clone());
                            cx.notify();
                        }),
                    ),
                );
            }
            item = item
                .child(message(m.download_complete))
                .child(directory(p, status.path.clone().unwrap_or_default()).mb(px(0.)))
                .child(row);
            if !can_install {
                item = item.child(
                    div()
                        .mt(px(10.))
                        .text_size(px(11.))
                        .line_height(relative(1.65))
                        .text_color(muted)
                        .child(m.install_source_hint),
                );
            }
        }
        DownloadState::Cancelled => {
            item = item.child(message(m.download_cancelled)).child(retry(
                view,
                status,
                id("retry"),
                starting,
                cx,
            ));
        }
        DownloadState::Failed => {
            item = item
                .child(
                    status_block(p, p.danger)
                        .mb(px(12.))
                        .child(m.download_failed)
                        .child(
                            div()
                                .font_family(mono_font())
                                .text_size(px(12.))
                                .mt(px(8.))
                                .child(status.error.clone().unwrap_or_default()),
                        ),
                )
                .child(retry(view, status, id("retry"), starting, cx));
        }
        DownloadState::Deleted => {}
    }
    if let Some(error) = view.file_errors.get(&status.url).filter(|e| !e.is_empty()) {
        item = item.child(
            status_block(p, p.danger)
                .mt(px(12.))
                .mb(px(0.))
                .child(error.clone()),
        );
    }
    item
}

fn retry(
    view: &CatalogView,
    status: &DownloadStatus,
    id: SharedString,
    starting: bool,
    cx: &mut Context<CatalogView>,
) -> Div {
    let p = current_palette(cx);
    let url = status.url.clone();
    div().mt(px(10.)).child(
        secondary_button(id, messages(view.props.language).retry, p, starting).when(
            !starting,
            |button| {
                button.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.start_download(url.clone(), cx)
                }))
            },
        ),
    )
}
