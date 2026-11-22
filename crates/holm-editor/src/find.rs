use std::ops::Range;

use gpui_kit::{InteractiveElement, IntoElement, MouseButton, ParentElement, Styled, div, px, rems};

use crate::element::Theme;
use crate::model::Model;

/// Which match [`Find::step`] selects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Toward {
    Next,
    /// The match before.
    Previous,
}

pub(crate) struct Find {
    query: String,
    /// Where the search starts: the selection start when find opened.
    origin: usize,
    pub(crate) matches: Vec<Range<usize>>,
    current: Option<usize>,
}

impl Find {
    pub(crate) fn new(query: String, origin: usize) -> Self {
        Self {
            query,
            origin,
            matches: Vec::new(),
            current: None,
        }
    }

    /// Only the first line of `text` counts.
    pub(crate) fn push(&mut self, text: &str) {
        self.query.push_str(text.lines().next().unwrap_or_default());
    }

    /// Removes the last char.
    pub(crate) fn pop(&mut self) {
        self.query.pop();
    }

    pub(crate) fn clear(&mut self) {
        self.query.clear();
    }

    /// Selects the first match from the origin on.
    pub(crate) fn refresh(&mut self, model: &mut Model) {
        self.matches = model.find_all(&self.query);
        self.current = if self.matches.is_empty() {
            None
        } else {
            let after = self.matches.partition_point(|m| m.start < self.origin);
            Some(after % self.matches.len())
        };
        self.select(model);
    }

    /// It goes round at the ends.
    pub(crate) fn step(&mut self, toward: Toward, model: &mut Model) {
        let count = self.matches.len();
        let Some(current) = self.current.filter(|_| count > 0) else {
            return;
        };
        self.current = Some(match toward {
            Toward::Next => (current + 1) % count,
            Toward::Previous => (current + count - 1) % count,
        });
        if let Some(found) = self.current.and_then(|ix| self.matches.get(ix)) {
            self.origin = found.start;
        }
        self.select(model);
    }

    fn select(&self, model: &mut Model) {
        if let Some(found) = self.current.and_then(|ix| self.matches.get(ix)) {
            model.select(found.clone());
        }
    }

    pub(crate) fn bar(&self, theme: &Theme) -> impl IntoElement {
        let count = match self.current {
            Some(ix) => format!("{} of {}", ix + 1, self.matches.len()),
            None if self.query.is_empty() => String::new(),
            None => "No matches".to_owned(),
        };
        let caret = div().w(px(2.)).h(rems(1.)).bg(theme.cursor);
        let field = div().flex().items_center().min_w(px(120.));
        let field = if self.query.is_empty() {
            field.child(caret).child(div().text_color(theme.muted).child("Find"))
        } else {
            field.child(self.query.clone()).child(caret)
        };
        div()
            .absolute()
            .top(px(8.))
            .right(px(16.))
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(10.))
            .py(px(4.))
            .rounded(px(6.))
            .bg(theme.panel)
            .border_1()
            .border_color(theme.line)
            .text_color(theme.text)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(field)
            .child(div().text_color(theme.muted).child(count))
    }
}
