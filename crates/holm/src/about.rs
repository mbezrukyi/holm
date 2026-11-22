//! About Holm, over the notes view: the app icon, the version, the licence, the copyright, and links to the licence
//! text and the third-party notices. These are the Appropriate Legal Notices of GPLv3 §0. The binary holds no licence
//! or notices text: the links carry it.
//!
//! `Tab` and `Shift+Tab` move the cursor between the links, and `Enter` opens the link under it. `Esc` closes the
//! screen.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Context, EventEmitter, FocusHandle, Focusable, FontWeight, Image, ImageFormat, InteractiveElement as _,
    IntoElement, MouseButton, ParentElement, Pixels, Render, Role, StatefulInteractiveElement as _, Styled,
    TestSupportExt as _, Window, div, img, px,
};

use crate::list_cursor::{self, Toward};
use crate::shortcuts::{Command, Place, Run};
use crate::{popup, text};

const LICENCE_URL: &str = "https://www.gnu.org/licenses/gpl-3.0.html";

const APP_ICON: &[u8] = include_bytes!("../../../packaging/linux/io.github.mbezrukyi.Holm.svg");
const ICON_SIZE: Pixels = px(48.);

/// Where the `AppImage` build puts the notices file, from its `usr` folder, which holds `bin/holm`.
const NOTICES_FROM_USR: &str = "share/doc/holm/THIRD-PARTY-NOTICES.txt";

/// The notices file beside the running binary. A build outside the `AppImage` has none.
pub(crate) fn notices_file() -> Option<PathBuf> {
    let binary = std::env::current_exe().ok()?;
    let notices = notices_beside(&binary)?;
    notices.is_file().then_some(notices)
}

fn notices_beside(binary: &Path) -> Option<PathBuf> {
    let usr = binary.parent()?.parent()?;
    Some(usr.join(NOTICES_FROM_USR))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Link {
    Licence,
    Notices,
}

/// The screen asks its owner to close it.
pub(crate) struct Close;

pub(crate) struct About {
    notices: Option<PathBuf>,
    cursor: usize,
    focus: FocusHandle,
}

impl About {
    /// It takes the focus. With no `notices` file, the screen has no notices link.
    pub(crate) fn new(notices: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self {
            notices,
            cursor: 0,
            focus,
        }
    }

    fn links(&self) -> Vec<Link> {
        let notices = self.notices.as_ref().map(|_| Link::Notices);
        [Some(Link::Licence), notices].into_iter().flatten().collect()
    }

    fn open(&self, link: Link, cx: &App) {
        match (link, &self.notices) {
            (Link::Licence, _) => cx.open_url(LICENCE_URL),
            (Link::Notices, Some(notices)) => cx.open_with_system(notices),
            (Link::Notices, None) => {}
        }
    }

    fn run(&mut self, Run(command): &Run, _window: &mut Window, cx: &mut Context<Self>) {
        let links = self.links();
        match command {
            Command::Close => cx.emit(Close),
            Command::Pick => self.open(links[self.cursor], cx),
            Command::Next | Command::Previous => {
                let toward = if *command == Command::Next {
                    Toward::Next
                } else {
                    Toward::Previous
                };
                if let Some(at) = list_cursor::step(Some(self.cursor), links.len(), toward, |_| true) {
                    self.cursor = at;
                    cx.notify();
                }
            }
            _ => cx.propagate(),
        }
    }

    fn link(&self, at: usize, link: Link, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let label = match link {
            Link::Licence => text::LICENCE_LINK,
            Link::Notices => text::NOTICES_LINK,
        };
        let current = at == self.cursor;
        div()
            .id(label)
            .role(Role::Link)
            .aria_label(label)
            .aria_selected(current)
            .test_support()
            .px_1()
            .rounded(theme.radius)
            .text_color(theme.link)
            .text_decoration_1()
            .text_decoration_color(theme.link.opacity(0.5))
            .cursor_pointer()
            .when(current, |link| link.bg(theme.accent))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |about, _, _, cx| {
                    about.cursor = at;
                    about.open(link, cx);
                    cx.notify();
                }),
            )
            .child(label)
    }
}

impl EventEmitter<Close> for About {}

impl Focusable for About {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for About {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let line = |id: &'static str, words: String| {
            div()
                .id(id)
                .role(Role::Label)
                .aria_label(words.clone())
                .test_support()
                .child(words)
        };
        let links: Vec<_> = self
            .links()
            .into_iter()
            .enumerate()
            .map(|(at, link)| self.link(at, link, cx))
            .collect();
        let icon = img(Arc::new(Image::from_bytes(ImageFormat::Svg, APP_ICON.to_vec()))).size(ICON_SIZE);
        let panel = popup::panel(px(440.), cx)
            .p_5()
            .items_center()
            .text_center()
            .child(icon)
            .child(
                div()
                    .mt_2()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(text::HOLM),
            )
            .child(
                line("version", text::version(env!("CARGO_PKG_VERSION")))
                    .text_sm()
                    .text_color(muted),
            )
            .child(div().self_stretch().h(px(1.)).my_4().bg(border))
            .child(
                v_flex()
                    .text_sm()
                    .child(line("licence", text::LICENCE.to_owned()))
                    .child(line("warranty", text::NO_WARRANTY.to_owned())),
            )
            .child(h_flex().mt_3().gap_4().text_sm().children(links))
            .child(
                line("copyright", text::COPYRIGHT.to_owned())
                    .mt_4()
                    .text_sm()
                    .text_color(muted),
            );
        popup::layer(cx)
            .id("about")
            .role(Role::Dialog)
            .aria_label(text::ABOUT)
            .test_support()
            .key_context(Place::About.key_context())
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::run))
            .child(panel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_notices_lie_in_the_doc_folder_beside_the_binary_folder() {
        assert_eq!(
            notices_beside(Path::new("/tmp/.mount_holm/usr/bin/holm")).as_deref(),
            Some(Path::new("/tmp/.mount_holm/usr/share/doc/holm/THIRD-PARTY-NOTICES.txt"))
        );
    }
}
