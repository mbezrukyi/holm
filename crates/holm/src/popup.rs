//! The look of a small window over the notes: a dim layer over the whole window, and a
//! panel in its middle.

use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::{App, Div, Pixels, Styled, div};

/// A layer over the whole window that centers its child.
pub(crate) fn layer(cx: &App) -> Div {
    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(cx.theme().overlay)
}

/// The panel in the layer, `width` wide.
pub(crate) fn panel(width: Pixels, cx: &App) -> Div {
    let theme = cx.theme();
    v_flex()
        .w(width)
        .bg(theme.popover)
        .border_1()
        .border_color(theme.border)
        .rounded(theme.radius)
}
