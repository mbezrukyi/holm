use std::fs;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Role, TestAppContext};

use super::{
    Fixture, app_menu, bounds_of, click, editor_text, empty_pane, focus, keys, labels, open, question, read, settle,
    sidebar, window_open,
};
use crate::text;

fn click_menu_item(cx: &mut TestAppContext, handle: AnyWindowHandle, at: u64) {
    cx.update_window(handle, |_, window, cx| {
        window.click(gpui_kit::ElementId::NamedInteger("menu-item".into(), at), cx);
    })
    .unwrap();
    settle(cx);
}

#[gpui_kit::test]
fn f10_and_the_menu_button_open_the_app_menu_under_the_button(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    fs::write(fixture.notes().join("Note.md"), "a\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "f10");
    assert_eq!(
        app_menu(cx, handle),
        ["Settings*", "Shortcuts", "Pin note", "About Holm", "Quit"]
    );
    let menu = bounds_of(cx, handle, "app-menu").unwrap();
    let button = bounds_of(cx, handle, "app-menu-button").unwrap();
    assert_eq!(menu.left(), button.left());
    assert!(menu.top() >= button.bottom());
    keys(cx, handle, "tab tab tab tab");
    assert_eq!(
        app_menu(cx, handle),
        ["Settings", "Shortcuts", "Pin note", "About Holm", "Quit*"]
    );
    keys(cx, handle, "escape");
    assert_eq!(app_menu(cx, handle), Vec::<String>::new());
    assert_eq!(focus(cx, handle), "editor");

    keys(cx, handle, "f10");
    keys(cx, handle, "f10");
    assert_eq!(app_menu(cx, handle), Vec::<String>::new());
    assert_eq!(focus(cx, handle), "editor");
    click(cx, handle, "app-menu-button");
    assert_eq!(app_menu(cx, handle).len(), 5);
    click(cx, handle, "app-menu-button");
    assert_eq!(app_menu(cx, handle), Vec::<String>::new());
    assert_eq!(focus(cx, handle), "editor");

    click(cx, handle, "app-menu-button");
    click(cx, handle, "editor");
    assert_eq!(app_menu(cx, handle), Vec::<String>::new());
    assert_eq!(editor_text(cx, handle), "a\n");
}

#[gpui_kit::test]
fn each_app_menu_item_runs_its_action_by_keys_and_by_click(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let note = fixture.notes().join("Note.md");
    fs::write(&note, "a\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "f10 enter");
    assert_eq!(question(cx, handle).as_deref(), Some(text::SETTINGS));
    keys(cx, handle, "escape");
    assert_eq!(focus(cx, handle), "editor");
    keys(cx, handle, "f10");
    click_menu_item(cx, handle, 0);
    assert_eq!(question(cx, handle).as_deref(), Some(text::SETTINGS));
    keys(cx, handle, "escape");

    keys(cx, handle, "f10 tab enter");
    assert_eq!(question(cx, handle).as_deref(), Some(text::SHORTCUT_LIST));
    keys(cx, handle, "escape");
    keys(cx, handle, "f10");
    click_menu_item(cx, handle, 1);
    assert_eq!(question(cx, handle).as_deref(), Some(text::SHORTCUT_LIST));
    keys(cx, handle, "escape");
    assert_eq!(focus(cx, handle), "editor");

    keys(cx, handle, "f10 tab tab enter");
    assert!(read(&note).contains("pinned: true"));
    keys(cx, handle, "f10");
    click_menu_item(cx, handle, 2);
    assert!(!read(&note).contains("pinned: true"));
    assert_eq!(focus(cx, handle), "editor");

    keys(cx, handle, "f10 tab tab tab enter");
    assert_eq!(question(cx, handle).as_deref(), Some(text::ABOUT));
    keys(cx, handle, "escape");
    keys(cx, handle, "f10");
    click_menu_item(cx, handle, 3);
    assert_eq!(question(cx, handle).as_deref(), Some(text::ABOUT));
    keys(cx, handle, "escape");
    assert_eq!(focus(cx, handle), "editor");

    keys(cx, handle, "f10");
    click_menu_item(cx, handle, 4);
    assert!(!window_open(cx, handle), "Quit closes the window");
}

/// The lines of About Holm, by their accessible names.
fn about_lines(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Vec<String> {
    labels(cx, handle, "about", Role::Label)
}

/// The link under the cursor has `*`.
fn about_links(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Vec<String> {
    labels(cx, handle, "about", Role::Link)
}

#[gpui_kit::test]
fn about_holm_shows_the_legal_notices_and_enter_opens_the_licence_link(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    fs::write(fixture.notes().join("Note.md"), "a\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "f10 tab tab tab enter");
    assert_eq!(question(cx, handle).as_deref(), Some(text::ABOUT));
    assert_eq!(
        about_lines(cx, handle),
        [
            text::version(env!("CARGO_PKG_VERSION")).as_str(),
            text::LICENCE,
            text::NO_WARRANTY,
            text::COPYRIGHT,
        ]
    );
    // A build outside the AppImage has no notices file beside it.
    assert_eq!(about_links(cx, handle), ["Licence*"]);
    keys(cx, handle, "tab");
    assert_eq!(about_links(cx, handle), ["Licence*"]);
    keys(cx, handle, "ctrl-n");
    assert_eq!(
        question(cx, handle).as_deref(),
        Some(text::ABOUT),
        "the notes view keys do nothing"
    );

    keys(cx, handle, "enter");
    assert_eq!(
        cx.opened_url().as_deref(),
        Some("https://www.gnu.org/licenses/gpl-3.0.html")
    );
    keys(cx, handle, "escape");
    assert_eq!(question(cx, handle), None);
    assert_eq!(focus(cx, handle), "editor");
    assert_eq!(editor_text(cx, handle), "a\n");
}

#[gpui_kit::test]
fn quit_in_the_app_menu_closes_the_window_by_keys(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    fs::write(fixture.notes().join("Note.md"), "a\n").unwrap();
    let handle = open(cx, fixture.startup(None));

    keys(cx, handle, "f10 up down");
    assert_eq!(
        app_menu(cx, handle),
        ["Settings*", "Shortcuts", "Pin note", "About Holm", "Quit"],
        "an arrow does nothing"
    );
    keys(cx, handle, "shift-tab");
    assert_eq!(
        app_menu(cx, handle),
        ["Settings", "Shortcuts", "Pin note", "About Holm", "Quit*"],
        "Shift+Tab wraps to the last item"
    );
    keys(cx, handle, "tab");
    assert_eq!(
        app_menu(cx, handle),
        ["Settings*", "Shortcuts", "Pin note", "About Holm", "Quit"],
        "Tab wraps to the first item"
    );
    keys(cx, handle, "shift-tab enter");
    assert!(!window_open(cx, handle));
}

#[gpui_kit::test]
fn pin_note_is_off_with_no_note_open(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let handle = open(cx, fixture.startup(None));
    assert!(empty_pane(cx, handle));

    keys(cx, handle, "f10 tab tab");
    assert_eq!(
        app_menu(cx, handle),
        ["Settings", "Shortcuts", "Pin note", "About Holm*", "Quit"]
    );
    keys(cx, handle, "shift-tab");
    assert_eq!(
        app_menu(cx, handle),
        ["Settings", "Shortcuts*", "Pin note", "About Holm", "Quit"]
    );
    click_menu_item(cx, handle, 2);
    assert_eq!(app_menu(cx, handle).len(), 5, "a click on Pin note does nothing");
    keys(cx, handle, "escape");
    assert_eq!(focus(cx, handle), "sidebar");
}

#[gpui_kit::test]
fn f10_with_the_sidebar_hidden_opens_the_menu_at_the_corner(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    fs::write(fixture.notes().join("Note.md"), "a\n").unwrap();
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-shift-l ctrl-shift-l");
    assert_eq!(sidebar(cx, handle), "hidden");

    keys(cx, handle, "f10");
    assert_eq!(app_menu(cx, handle).len(), 5);
    assert_eq!(sidebar(cx, handle), "hidden");
    let menu = bounds_of(cx, handle, "app-menu").unwrap();
    assert_eq!((menu.left(), menu.top()), (gpui_kit::px(4.), gpui_kit::px(4.)));

    keys(cx, handle, "enter");
    assert_eq!(question(cx, handle).as_deref(), Some(text::SETTINGS));
    assert_eq!(sidebar(cx, handle), "hidden");
}
