//! The quiet bar: one line of text across the top of the window, with small buttons.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex};
use gpui_kit::{
    App, ElementId, InteractiveElement as _, IntoElement, ParentElement, RenderOnce, Role,
    SharedString, StatefulInteractiveElement as _, Styled, TestSupportExt as _, Window, div,
};

#[derive(IntoElement)]
pub(crate) struct Bar {
    id: ElementId,
    text: SharedString,
    buttons: Vec<Button>,
}

impl Bar {
    pub(crate) fn new(id: impl Into<ElementId>, text: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            buttons: Vec::new(),
        }
    }

    /// Adds a button. Its click handler comes from the caller.
    pub(crate) fn button(mut self, button: Button) -> Self {
        self.buttons.push(button);
        self
    }
}

/// A button in the look of the bar.
pub(crate) fn button(id: impl Into<ElementId>, label: &'static str) -> Button {
    Button::new(id).label(label).ghost().small()
}

impl RenderOnce for Bar {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        h_flex()
            .id(self.id)
            .role(Role::Status)
            .aria_label(self.text.clone())
            .test_support()
            .w_full()
            .px_4()
            .py_1()
            .gap_2()
            .bg(theme.muted)
            .border_b_1()
            .border_color(theme.border)
            .text_sm()
            .child(div().flex_1().min_w_0().child(self.text))
            .children(self.buttons)
    }
}
