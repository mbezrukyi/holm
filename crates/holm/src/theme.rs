//! The warm Holm themes for the window, light and dark, and the fonts.

use std::borrow::Cow;

use gpui_kit::component::{Theme as Ui, ThemeMode, ThemeRegistry};
use gpui_kit::{App, Pixels, Window, WindowAppearance, px};
use holm_editor::Appearance;

use crate::config::AppearanceSetting;

const THEMES: &str = include_str!("theme.json");
const LIGHT: &str = "Holm Light";
const DARK: &str = "Holm Dark";

/// The UI font that `theme.json` names. Holm ships its static files, because GPUI can't pick
/// a weight in a variable font.
const UI_FONT_FILES: [&[u8]; 4] = [
    include_bytes!("../fonts/Inter-Regular.ttf"),
    include_bytes!("../fonts/Inter-Medium.ttf"),
    include_bytes!("../fonts/Inter-SemiBold.ttf"),
    include_bytes!("../fonts/Inter-Bold.ttf"),
];

pub(crate) const NOTE_FONT: &str = "DejaVu Serif";
pub(crate) const NOTE_SIZE: Pixels = px(15.);
pub(crate) const NOTE_LINE: Pixels = px(25.);
/// The size of the line on the empty pane.
pub(crate) const EMPTY_LINE: Pixels = px(15.);

/// Adds the UI font and the Holm themes. Call it once, after `gpui_kit::init`.
pub(crate) fn init(cx: &mut App) {
    let fonts = UI_FONT_FILES
        .iter()
        .map(|file| Cow::Borrowed(*file))
        .collect();
    if let Err(error) = cx.text_system().add_fonts(fonts) {
        tracing::error!(%error, "cannot load the UI font");
    }
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

/// Sets light or dark mode. [`AppearanceSetting::System`] takes the mode of `window`.
pub(crate) fn set_mode(theme: AppearanceSetting, window: &mut Window, cx: &mut App) {
    let mode = match theme {
        AppearanceSetting::Light => ThemeMode::Light,
        AppearanceSetting::Dark => ThemeMode::Dark,
        AppearanceSetting::System => match window.appearance() {
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use gpui_kit::{FontWeight, font};

    use super::*;

    #[test]
    fn the_ui_font_has_a_face_for_each_weight() {
        let text_system = gpui_kit::platform::current_platform(true).text_system();
        let fonts = UI_FONT_FILES
            .iter()
            .map(|file| Cow::Borrowed(*file))
            .collect();
        text_system.add_fonts(fonts).unwrap();
        let face = |weight| {
            let mut ui = font("Inter");
            ui.weight = weight;
            text_system.font_id(&ui).unwrap()
        };
        let regular = face(FontWeight::NORMAL);
        assert_ne!(regular, face(FontWeight::MEDIUM));
        assert_ne!(regular, face(FontWeight::SEMIBOLD));
        assert_ne!(face(FontWeight::SEMIBOLD), face(FontWeight::BOLD));
    }
}
