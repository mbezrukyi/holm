//! The warm Holm themes for the window, light and dark, and the fonts.

use gpui_kit::component::{Theme as Ui, ThemeMode, ThemeRegistry};
use gpui_kit::{App, Pixels, Window, WindowAppearance, px};
use holm_editor::Appearance;

use crate::config::Theme;

const THEMES: &str = include_str!("theme.json");
const LIGHT: &str = "Holm Light";
const DARK: &str = "Holm Dark";

pub(crate) const NOTE_FONT: &str = "DejaVu Serif";
pub(crate) const NOTE_SIZE: Pixels = px(15.);
pub(crate) const NOTE_LINE: Pixels = px(25.);

/// Adds the Holm themes. Call it once, after `gpui_kit::init`.
pub(crate) fn init(cx: &mut App) {
    let registry = ThemeRegistry::global_mut(cx);
    if let Err(error) = registry.load_themes_from_str(THEMES) {
        tracing::error!(%error, "cannot read the Holm themes");
        return;
    }
    let themes = registry.themes();
    let (Some(light), Some(dark)) = (themes.get(LIGHT).cloned(), themes.get(DARK).cloned()) else {
        tracing::error!("the Holm themes are missing");
        return;
    };
    Ui::update(cx, |theme| {
        theme.light_theme = light;
        theme.dark_theme = dark;
    });
}

/// Sets light or dark mode. [`Theme::System`] takes the mode of `window`.
pub(crate) fn apply(theme: Theme, window: &mut Window, cx: &mut App) {
    let mode = match theme {
        Theme::Light => ThemeMode::Light,
        Theme::Dark => ThemeMode::Dark,
        Theme::System => match window.appearance() {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => ThemeMode::Dark,
            WindowAppearance::Light | WindowAppearance::VibrantLight => ThemeMode::Light,
        },
    };
    Ui::change(mode, Some(window), cx);
}

/// The editor mode that matches the window.
pub(crate) fn appearance(cx: &App) -> Appearance {
    if Ui::global(cx).is_dark() {
        Appearance::Dark
    } else {
        Appearance::Light
    }
}
