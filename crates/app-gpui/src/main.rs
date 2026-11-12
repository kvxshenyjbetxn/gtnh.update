#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod app;
mod backend;
mod flow;
mod i18n;
mod logic;
mod theme;
mod ui;

use gpui::{
    px, size, AppContext as _, Bounds, TitlebarOptions, WindowBounds, WindowOptions,
};
use gpui_component::Root;
use gpui_platform::application;

fn main() {
    // gpui-component draws its icons (checkbox tick, select chevron) from this asset source.
    application().with_assets(gpui_kit_assets::Assets).run(|cx| {
        gpui_component::init(cx);

        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let bounds = Bounds::centered(None, size(px(1120.), px(800.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(760.), px(600.))),
            titlebar: Some(TitlebarOptions {
                title: Some("GTNH Updater".into()),
                ..Default::default()
            }),
            ..Default::default()
        };

        cx.open_window(options, |window, cx| {
            let view = cx.new(|cx| app::AppView::new(window, cx));
            cx.new(|cx| Root::new(view, window, cx))
        })
        .expect("failed to open window");
        cx.activate(true);
    });
}
