//! The tag row above the note: one chip for each tag, and a pin mark.
//!
//! The row changes no files itself. It sends the new tags as a [`TagRowEvent`].
//!
//! - In the field, `Enter` adds the typed tag and `Tab` adds the first suggestion.
//!   `←` or `Backspace` in an empty field moves the focus to the last chip.
//! - On a chip, `←` and `→` move between chips, and `Backspace` or `Delete` removes it.
//! - `Esc` or `↓` gives the focus back to the text.
//!
//! With no tags and no pin, the row shows only while it has the focus.

use gpui_kit::component::input::{
    Backspace, Enter, Escape, IndentInline, Input, InputState, MoveDown, MoveLeft, MoveUp,
};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, ElementId, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement, Render, Role,
    StatefulInteractiveElement as _, Styled, Subscription, Task, TestSupportExt as _, Window, div,
    px,
};
use holm_core::Core;
use holm_core::model::Tag;

use crate::text;

/// The most suggestions under the field.
const SUGGESTIONS: usize = 5;

/// What the tag row tells the pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TagRowEvent {
    /// The user added or removed a tag. These are all the tags of the note now.
    Changed(Vec<String>),
    /// Give the focus back to the text.
    Leave,
}

pub(crate) struct TagRow {
    core: Core,
    tags: Vec<String>,
    pinned: bool,
    /// Every tag name in the notes folder, for the suggestions.
    known: Vec<String>,
    /// Whether the row has the focus, so it shows with no tags.
    active: bool,
    /// The chip with the focus. With `None`, the field has it.
    chip: Option<usize>,
    focus: FocusHandle,
    input: Entity<InputState>,
    loading: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl TagRow {
    pub(crate) fn new(core: Core, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(text::ADD_TAG));
        let subscriptions = vec![
            cx.on_focus_out(&focus, window, |row, _, window, cx| row.close(window, cx)),
            cx.observe(&input, |_, _, cx| cx.notify()),
        ];
        Self {
            core,
            tags: Vec::new(),
            pinned: false,
            known: Vec::new(),
            active: false,
            chip: None,
            focus,
            input,
            loading: None,
            _subscriptions: subscriptions,
        }
    }

    /// Shows the tags and the pin of the open note.
    pub(crate) fn show(&mut self, tags: &[String], pinned: bool, cx: &mut Context<Self>) {
        if self.tags != tags || self.pinned != pinned {
            self.tags = tags.to_vec();
            self.pinned = pinned;
            self.chip = self.chip.filter(|at| *at < self.tags.len());
            cx.notify();
        }
    }

    /// Gives the field the focus, and reads the known tags for the suggestions.
    pub(crate) fn enter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.active = true;
        self.chip = None;
        self.input.update(cx, |input, cx| input.focus(window, cx));
        let core = self.core.clone();
        let job = cx.background_spawn(async move { core.tags() });
        self.loading = Some(cx.spawn(async move |row, cx| {
            let result = job.await;
            row.update(cx, |row, cx| {
                row.loading = None;
                match result {
                    Ok(tags) => {
                        row.known.clear();
                        flatten(&tags, &mut row.known);
                    }
                    Err(error) => {
                        tracing::warn!(error = %crate::chain(&error), "cannot read the tags");
                    }
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// Hides the row when it has no tags, after the focus left it.
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.active = false;
        self.chip = None;
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    fn leave(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close(window, cx);
        cx.emit(TagRowEvent::Leave);
    }

    fn typed(&self, cx: &App) -> String {
        self.input.read(cx).value().trim().to_owned()
    }

    /// The known tags that hold the typed text, and that the note does not have yet.
    fn suggestions(&self, cx: &App) -> Vec<&str> {
        let typed = self.typed(cx).to_lowercase();
        if typed.is_empty() {
            return Vec::new();
        }
        self.known
            .iter()
            .filter(|tag| tag.to_lowercase().contains(&typed) && !self.tags.contains(tag))
            .take(SUGGESTIONS)
            .map(String::as_str)
            .collect()
    }

    fn add(&mut self, tag: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tag) = holm_markdown::normalize_tag(tag) else {
            return;
        };
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
        if !self.tags.contains(&tag) {
            self.tags.push(tag);
            cx.emit(TagRowEvent::Changed(self.tags.clone()));
        }
        cx.notify();
    }

    fn remove(&mut self, at: usize, window: &mut Window, cx: &mut Context<Self>) {
        if at >= self.tags.len() {
            return;
        }
        self.tags.remove(at);
        cx.emit(TagRowEvent::Changed(self.tags.clone()));
        if self.tags.is_empty() {
            self.enter(window, cx);
        } else {
            self.chip = Some(at.min(self.tags.len() - 1));
        }
        cx.notify();
    }

    /// Moves the focus from an empty field to the last chip. It returns whether it did.
    fn focus_last_chip(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.typed(cx).is_empty() || self.tags.is_empty() {
            return false;
        }
        self.chip = Some(self.tags.len() - 1);
        window.focus(&self.focus, cx);
        cx.notify();
        true
    }

    /// The keys on a chip. The field takes its keys as actions, before this.
    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(at) = self.chip else {
            return;
        };
        if event.keystroke.modifiers.modified() {
            return;
        }
        match event.keystroke.key.as_str() {
            "left" => self.chip = Some(at.saturating_sub(1)),
            "right" if at + 1 < self.tags.len() => self.chip = Some(at + 1),
            "right" => self.enter(window, cx),
            "backspace" | "delete" => self.remove(at, window, cx),
            "escape" | "down" => self.leave(window, cx),
            _ => return,
        }
        cx.notify();
        cx.stop_propagation();
    }

    /// The field for a new tag, with the suggestions under it.
    fn field(&self, cx: &App) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let suggestions: Vec<String> = self
            .suggestions(cx)
            .into_iter()
            .map(str::to_owned)
            .collect();
        div()
            .w(px(160.))
            .child(
                Input::new(&self.input)
                    .id("tag-input")
                    .aria_label(text::ADD_TAG)
                    .appearance(false)
                    .text_size(px(12.)),
            )
            .when(!suggestions.is_empty(), |field| {
                field.child(
                    v_flex()
                        .id("tag-suggestions")
                        .role(Role::ListBox)
                        .test_support()
                        .absolute()
                        .mt_1()
                        .w(px(200.))
                        .py_1()
                        .bg(theme.popover)
                        .border_1()
                        .border_color(theme.border)
                        .rounded(theme.radius)
                        .children(suggestions.iter().enumerate().map(|(at, tag)| {
                            div()
                                .id(ElementId::NamedInteger("tag-suggestion".into(), at as u64))
                                .role(Role::ListBoxOption)
                                .aria_label(tag.clone())
                                .test_support()
                                .px_2()
                                .text_size(px(12.))
                                .when(at == 0, |row| row.bg(theme.selection))
                                .child(format!("#{tag}"))
                        })),
                )
            })
    }

    fn chip(&self, at: usize, tag: &str, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let selected = self.chip == Some(at);
        div()
            .id(ElementId::NamedInteger("tag-chip".into(), at as u64))
            .role(Role::ListItem)
            .aria_label(tag.to_owned())
            .aria_selected(selected)
            .test_support()
            .px_2()
            .rounded(theme.radius)
            .text_size(px(12.))
            .bg(if selected {
                theme.selection
            } else {
                theme.secondary
            })
            .text_color(theme.secondary_foreground)
            .child(format!("#{tag}"))
    }
}

/// Every tag name in the tree, parents first.
fn flatten(tags: &[Tag], names: &mut Vec<String>) {
    for tag in tags {
        names.push(tag.name.clone());
        flatten(&tag.children, names);
    }
}

impl EventEmitter<TagRowEvent> for TagRow {}

impl Focusable for TagRow {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TagRow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let visible = self.active || self.pinned || !self.tags.is_empty();
        let field = self.active.then(|| self.field(cx));
        let pin = self.pinned.then(|| {
            div()
                .id("pin-mark")
                .role(Role::Image)
                .aria_label(text::PINNED)
                .test_support()
                .text_size(px(10.))
                .text_color(theme.primary)
                .child(text::PIN_MARK)
        });
        let input_focus = self.input.focus_handle(cx);
        h_flex()
            .id("tag-row")
            .role(Role::List)
            .aria_label(text::TAGS_LABEL)
            .test_support()
            .track_focus(&self.focus)
            .key_context("TagRow")
            .on_key_down(cx.listener(Self::key_down))
            .capture_action(cx.listener(|row, _: &Enter, window, cx| {
                let typed = row.typed(cx);
                row.add(&typed, window, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|row, _: &IndentInline, window, cx| {
                let tag = row
                    .suggestions(cx)
                    .first()
                    .map_or_else(|| row.typed(cx), |tag| (*tag).to_owned());
                row.add(&tag, window, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|row, _: &Escape, window, cx| {
                row.leave(window, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|row, _: &MoveDown, window, cx| {
                row.leave(window, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|_, _: &MoveUp, _, cx| cx.stop_propagation()))
            .capture_action(cx.listener(move |row, _: &MoveLeft, window, cx| {
                if input_focus.is_focused(window) && row.focus_last_chip(window, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|row, _: &Backspace, window, cx| {
                if row.focus_last_chip(window, cx) {
                    cx.stop_propagation();
                }
            }))
            .when(visible, |row| row.min_h(px(28.)).py_1())
            .flex_wrap()
            .gap_1p5()
            .items_center()
            .children(pin)
            .children(
                self.tags
                    .iter()
                    .enumerate()
                    .map(|(at, tag)| self.chip(at, tag, cx)),
            )
            .children(field)
    }
}
