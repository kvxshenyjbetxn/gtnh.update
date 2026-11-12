use gpui::{rgb, App, Hsla};
use gpui_component::{
    theme::{Theme, ThemeMode},
    ActiveTheme as _,
};
use gtnh_core::Theme as ThemeSetting;

/// Design tokens ported from the `:root` / `:root[data-theme=light]` rules in `src/style.css`.
pub struct Palette {
    pub bg: u32,
    pub panel: u32,
    pub raised: u32,
    pub border: u32,
    pub text: u32,
    pub muted: u32,
    pub accent: u32,
    pub accent_text: u32,
    pub danger: u32,
    pub info: u32,
}

pub const AMOLED: Palette = Palette {
    bg: 0x000000,
    panel: 0x0b0b0d,
    raised: 0x121215,
    border: 0x252529,
    text: 0xf3f3f5,
    muted: 0x96969e,
    accent: 0xb8a0ff,
    accent_text: 0x1c1233,
    danger: 0xff9c9c,
    info: 0x211b30,
};

pub const LIGHT: Palette = Palette {
    bg: 0xf7f8f5,
    panel: 0xffffff,
    raised: 0xf0f2ec,
    border: 0xdce0d5,
    text: 0x1b2017,
    muted: 0x636b5b,
    accent: 0x6842af,
    accent_text: 0xffffff,
    danger: 0xb22525,
    info: 0xf0eafb,
};

pub fn palette(theme: ThemeSetting) -> &'static Palette {
    match theme {
        ThemeSetting::Amoled => &AMOLED,
        ThemeSetting::Light => &LIGHT,
    }
}

pub fn color(value: u32) -> Hsla {
    rgb(value).into()
}

/// Palette matching the mode last applied by `sync_component_theme`; for views that do not know the setting.
pub fn current_palette(cx: &App) -> &'static Palette {
    if cx.theme().mode.is_dark() {
        &AMOLED
    } else {
        &LIGHT
    }
}

/// CSS font-family is `Inter, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif`.
pub fn ui_font() -> &'static str {
    if cfg!(target_os = "windows") {
        "Segoe UI"
    } else if cfg!(target_os = "macos") {
        ".SystemUIFont"
    } else {
        "Inter"
    }
}

pub fn mono_font() -> &'static str {
    if cfg!(target_os = "windows") {
        "Consolas"
    } else if cfg!(target_os = "macos") {
        "Menlo"
    } else {
        "DejaVu Sans Mono"
    }
}

/// Make gpui-component's global `Theme` (used by Select, Checkbox and popups) follow the palette.
pub fn sync_component_theme(setting: ThemeSetting, cx: &mut App) {
    let p = palette(setting);
    let mode = match setting {
        ThemeSetting::Amoled => ThemeMode::Dark,
        ThemeSetting::Light => ThemeMode::Light,
    };
    // Loading a mode replaces the colors, so edit them in a second update.
    if cx.theme().mode != mode {
        Theme::change(mode, None, cx);
    }
    Theme::update(cx, |theme| {
        theme.background = color(p.bg);
        theme.foreground = color(p.text);
        theme.border = color(p.border);
        theme.input = color(p.border);
        theme.ring = color(p.accent);
        theme.muted = color(p.raised);
        theme.muted_foreground = color(p.muted);
        theme.primary = color(p.accent);
        theme.primary_hover = color(p.accent);
        theme.primary_active = color(p.accent);
        theme.primary_foreground = color(p.accent_text);
        theme.accent = color(p.info);
        theme.accent_foreground = color(p.accent);
        theme.popover = color(p.raised);
        theme.popover_foreground = color(p.text);
        theme.colors.list = color(p.raised);
        theme.list_even = color(p.raised);
        theme.list_active = color(p.info);
        theme.list_active_border = color(p.accent);
        theme.list_hover = color(p.info);
        theme.selection = color(p.info);
        theme.danger = color(p.danger);
    });
}
