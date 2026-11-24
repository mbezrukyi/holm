use std::fs;
use std::path::{Path, PathBuf};

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Role, TestAppContext};
use holm_core::model::Order;
use holm_editor::WritingHelp;

use super::{
    Fixture, bounds_of, click, editor_text, focus, keys, label, labels, message, notes_list, open, question, read,
    read_only, screen, settle, state, three_notes, type_unsaved,
};
use crate::config::{AppearanceSetting, Config, Settings};
use crate::window::HolmWindow;
use crate::{text, theme};

/// As the settings dialog does.
fn change_settings(cx: &mut TestAppContext, handle: AnyWindowHandle, change: impl FnOnce(&mut Settings)) {
    cx.update_window(handle, |root, _, cx| {
        let root = root.downcast::<gpui_kit::base::Root>().unwrap();
        let view = root.read(cx).view().clone();
        let config = view.downcast::<HolmWindow>().unwrap().read(cx).config.clone();
        config.update(cx, |config, cx| config.change(change, cx));
    })
    .unwrap();
    settle(cx);
}

fn dark(cx: &mut TestAppContext) -> bool {
    cx.update(|cx| theme::appearance(cx) == holm_editor::Appearance::Dark)
}

#[gpui_kit::test]
fn a_new_theme_applies_at_once(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let handle = open(cx, fixture.startup(None));
    assert!(!dark(cx));

    change_settings(cx, handle, |settings| {
        settings.theme = AppearanceSetting::Dark;
    });
    assert!(dark(cx));
    change_settings(cx, handle, |settings| {
        settings.theme = AppearanceSetting::System;
    });
    assert!(!dark(cx));
}

#[gpui_kit::test]
fn a_new_note_order_sorts_the_sidebar_at_once(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    three_notes(&fixture.notes());
    let handle = open(cx, fixture.startup(None));
    assert_eq!(notes_list(cx, handle), ["Plan*", "Goals", "Top"]);

    change_settings(cx, handle, |settings| settings.note_order = Order::Title);
    assert_eq!(notes_list(cx, handle), ["Goals", "Plan*", "Top"]);
}

fn typed_with(cx: &mut TestAppContext, help: WritingHelp, text: &str, key: &str) -> String {
    let fixture = Fixture::new().with_folder();
    fs::write(fixture.notes().join("Note.md"), "").unwrap();
    let handle = open(cx, fixture.startup(None));
    change_settings(cx, handle, |settings| settings.writing_help = help);
    type_unsaved(cx, handle, text);
    keys(cx, handle, key);
    editor_text(cx, handle)
}

#[gpui_kit::test]
fn format_keys_off_apply_at_once(cx: &mut TestAppContext) {
    let off = WritingHelp {
        format_keys: false,
        ..WritingHelp::default()
    };
    assert_eq!(typed_with(cx, off, "a", "shift-left ctrl-b"), "a");
}

#[gpui_kit::test]
fn continue_lists_off_applies_at_once(cx: &mut TestAppContext) {
    let off = WritingHelp {
        continue_lists: false,
        ..WritingHelp::default()
    };
    assert_eq!(typed_with(cx, off, "- a", "enter"), "- a\n");
}

#[gpui_kit::test]
fn indent_lists_off_applies_at_once(cx: &mut TestAppContext) {
    let off = WritingHelp {
        indent_lists: false,
        ..WritingHelp::default()
    };
    assert_eq!(typed_with(cx, off, "- a", "tab"), "- a    ");
}

fn two_folders() -> (Fixture, PathBuf) {
    let fixture = Fixture::new().with_folder();
    fs::write(fixture.notes().join("Note.md"), "old\n").unwrap();
    let other = fixture.dir.path().join("other");
    fs::create_dir(&other).unwrap();
    fs::write(other.join("A.md"), "a\n").unwrap();
    (fixture, other)
}

#[gpui_kit::test]
fn a_new_notes_folder_saves_the_text_and_opens_at_once(cx: &mut TestAppContext) {
    let (fixture, other) = two_folders();
    let handle = open(cx, fixture.startup(None));
    type_unsaved(cx, handle, "x");

    change_settings(cx, handle, |settings| {
        settings.notes_folder = Some(other.clone());
    });

    assert_eq!(editor_text(cx, handle), "a\n");
    assert!(read(&fixture.notes().join("Note.md")).ends_with("\nxold\n"));
    let config = Config::load(fixture.config());
    assert_eq!(config.settings().notes_folder, Some(other));
    assert_eq!(
        config.settings().last_note.as_ref().map(AsRef::as_ref),
        Some(Path::new("A.md"))
    );
}

#[gpui_kit::test]
fn a_new_notes_folder_with_a_failed_save_keeps_the_old_folder(cx: &mut TestAppContext) {
    let (fixture, other) = two_folders();
    let handle = open(cx, fixture.startup(None));
    read_only(&fixture.notes(), true);
    type_unsaved(cx, handle, "x");

    change_settings(cx, handle, |settings| {
        settings.notes_folder = Some(other.clone());
    });

    assert_eq!(editor_text(cx, handle), "xold\n");
    assert_eq!(state(cx, handle).as_deref(), Some(text::NOT_SAVED));
    let config = Config::load(fixture.config());
    assert_eq!(config.settings().notes_folder, Some(fixture.notes()));

    read_only(&fixture.notes(), false);
    change_settings(cx, handle, |settings| {
        settings.notes_folder = Some(other.clone());
    });
    assert_eq!(editor_text(cx, handle), "a\n");
    assert!(read(&fixture.notes().join("Note.md")).ends_with("\nxold\n"));
}

/// The row with the cursor has `*`.
fn setting_rows(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Vec<String> {
    labels(cx, handle, "settings", Role::ListItem)
}

#[gpui_kit::test]
fn ctrl_shift_s_opens_the_settings_and_esc_gives_the_focus_back(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    fs::write(fixture.notes().join("Note.md"), "a\n").unwrap();
    let handle = open(cx, fixture.startup(None));
    assert_eq!(focus(cx, handle), "editor");

    keys(cx, handle, "ctrl-shift-s");
    assert_eq!(question(cx, handle).as_deref(), Some(text::SETTINGS));
    assert_eq!(
        setting_rows(cx, handle),
        [
            "Notes folder*",
            "Theme",
            "Scale",
            "Text width",
            "Note order",
            "Format keys",
            "Continue lists on Enter",
            "Indent lists with Tab",
        ]
    );

    keys(cx, handle, "escape");
    assert_eq!(question(cx, handle), None);
    assert_eq!(focus(cx, handle), "editor");
    assert_eq!(editor_text(cx, handle), "a\n");
}

#[gpui_kit::test]
fn each_setting_changes_by_keys_only(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let handle = open(cx, fixture.startup(None));
    let settings = |fixture: &Fixture| Config::load(fixture.config()).settings().clone();
    keys(cx, handle, "ctrl-shift-s");

    keys(cx, handle, "tab space space");
    assert_eq!(setting_rows(cx, handle)[1], "Theme*");
    assert!(dark(cx));
    keys(cx, handle, "space");
    assert_eq!(settings(&fixture).theme, AppearanceSetting::System, "Space wraps");
    keys(cx, handle, "shift-space");
    assert_eq!(settings(&fixture).theme, AppearanceSetting::Dark, "Shift+Space wraps");
    keys(cx, handle, "shift-space");
    assert_eq!(settings(&fixture).theme, AppearanceSetting::Light);
    assert!(!dark(cx));
    keys(cx, handle, "left right");
    assert_eq!(
        settings(&fixture).theme,
        AppearanceSetting::Light,
        "an arrow does nothing"
    );

    keys(cx, handle, "tab space");
    assert_eq!(setting_rows(cx, handle)[2], "Scale*");
    assert_eq!(settings(&fixture).scale.percent(), 110);
    assert_eq!(zoom(cx, handle), 1.1);
    keys(cx, handle, "shift-space shift-space shift-space");
    assert_eq!(settings(&fixture).scale.percent(), 80);
    assert_eq!(zoom(cx, handle), 0.8);
    keys(cx, handle, "shift-space");
    assert_eq!(settings(&fixture).scale.percent(), 200, "Shift+Space wraps");
    keys(cx, handle, "space");
    assert_eq!(settings(&fixture).scale.percent(), 80, "Space wraps");
    keys(cx, handle, "space space");
    assert_eq!(settings(&fixture).scale.percent(), 100);

    keys(cx, handle, "tab space");
    assert_eq!(setting_rows(cx, handle)[3], "Text width*");
    assert!(settings(&fixture).text_width.limited);
    keys(cx, handle, "shift-space");
    assert!(!settings(&fixture).text_width.limited);

    keys(cx, handle, "tab space");
    assert_eq!(settings(&fixture).note_order, Order::Title);
    keys(cx, handle, "shift-space");
    assert_eq!(settings(&fixture).note_order, Order::LastChange);

    keys(cx, handle, "tab space");
    assert!(!settings(&fixture).writing_help.format_keys);
    keys(cx, handle, "tab space");
    assert!(!settings(&fixture).writing_help.continue_lists);
    keys(cx, handle, "tab space");
    assert!(!settings(&fixture).writing_help.indent_lists);
    keys(cx, handle, "space");
    assert!(settings(&fixture).writing_help.indent_lists);
    keys(cx, handle, "shift-space");
    assert!(
        settings(&fixture).writing_help.indent_lists,
        "Shift+Space does nothing on a switch"
    );

    keys(cx, handle, "tab");
    assert_eq!(setting_rows(cx, handle)[0], "Notes folder*", "Tab wraps");
    keys(cx, handle, "shift-tab");
    assert_eq!(setting_rows(cx, handle)[7], "Indent lists with Tab*", "Shift+Tab wraps");
    assert_eq!(question(cx, handle).as_deref(), Some(text::SETTINGS));
}

/// The scale of Holm as a factor.
fn zoom(cx: &mut TestAppContext, handle: AnyWindowHandle) -> f32 {
    cx.update_window(handle, |_, window, _| window.zoom()).unwrap()
}

#[gpui_kit::test]
fn space_on_the_scale_row_wraps_from_the_largest(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    fixture.write_config(&format!(
        "version = 1\nnotes_folder = {:?}\nscale = 200\n",
        fixture.notes().to_str().unwrap()
    ));
    let handle = open(cx, fixture.startup(None));
    assert_eq!(zoom(cx, handle), 2.);

    keys(cx, handle, "ctrl-shift-s tab tab space");
    assert_eq!(setting_rows(cx, handle)[2], "Scale*");
    assert_eq!(Config::load(fixture.config()).settings().scale.percent(), 80);
    assert_eq!(zoom(cx, handle), 0.8);
}

/// The left and the width of the text column, from the left of the editor. The tag row fills the column.
fn text_column(cx: &mut TestAppContext, handle: AnyWindowHandle) -> (f32, f32) {
    screen(cx, handle, |window| {
        let editor = window.find("editor").bounds();
        let row = window.find("tag-row").bounds();
        (f32::from(row.left() - editor.left()), f32::from(row.size.width))
    })
}

fn editor_width(cx: &mut TestAppContext, handle: AnyWindowHandle) -> f32 {
    screen(cx, handle, |window| {
        f32::from(window.find("editor").bounds().size.width)
    })
}

/// `None` when it does not show.
fn chars_field(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Option<String> {
    screen(cx, handle, |window| {
        window
            .try_find("text-width-chars")
            .and_then(|field| field.value().map(str::to_owned))
    })
}

fn text_width(fixture: &Fixture) -> crate::text_width::TextWidthSetting {
    Config::load(fixture.config()).settings().text_width
}

fn open_text_width(cx: &mut TestAppContext) -> (Fixture, AnyWindowHandle) {
    let fixture = Fixture::new().with_folder();
    fs::write(fixture.notes().join("Note.md"), "a\n").unwrap();
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-shift-s tab tab tab");
    assert_eq!(setting_rows(cx, handle)[3], "Text width*");
    (fixture, handle)
}

#[gpui_kit::test]
fn space_on_the_text_width_row_switches_the_mode_and_the_editor_follows(cx: &mut TestAppContext) {
    let (fixture, handle) = open_text_width(cx);
    let full = text_column(cx, handle);
    assert_eq!(full.0, 32.);
    assert_eq!(chars_field(cx, handle), None);

    keys(cx, handle, "space");
    assert!(text_width(&fixture).limited);
    assert_eq!(chars_field(cx, handle).as_deref(), Some("80"));
    let (left, width) = text_column(cx, handle);
    assert!(width < full.1, "{width} is not less than {}", full.1);
    let right = editor_width(cx, handle) - left - width;
    assert!((left - right).abs() <= 1., "{left} and {right} differ");

    keys(cx, handle, "space");
    assert!(!text_width(&fixture).limited);
    assert_eq!(text_column(cx, handle), full);
    assert_eq!(chars_field(cx, handle), None);
}

#[gpui_kit::test]
fn typing_a_width_on_the_row_changes_the_column_at_once(cx: &mut TestAppContext) {
    let (fixture, handle) = open_text_width(cx);
    keys(cx, handle, "space");
    let (_, eighty) = text_column(cx, handle);

    keys(cx, handle, "4 5");
    assert_eq!(
        text_width(&fixture).chars(),
        45,
        "the first digit takes the place of 80"
    );
    assert_eq!(setting_rows(cx, handle)[3], "Text width*");
    let (_, width) = text_column(cx, handle);
    assert!(
        (width / eighty - 45. / 80.).abs() < 0.02,
        "{width} for 45, {eighty} for 80"
    );
    keys(cx, handle, "backspace backspace 1 2 0");
    assert_eq!(text_width(&fixture).chars(), 120);

    let box_bounds = |cx: &mut TestAppContext| screen(cx, handle, |window| window.find("text-width-chars").bounds());
    let wide = box_bounds(cx);
    keys(cx, handle, "backspace backspace");
    assert_eq!(box_bounds(cx), wide, "the box keeps its place and size");
    keys(cx, handle, "tab");
    assert_eq!(setting_rows(cx, handle)[4], "Note order*", "Tab goes to the next row");
    assert_eq!(chars_field(cx, handle).as_deref(), Some("120"));
}

#[gpui_kit::test]
fn a_width_out_of_the_range_shows_the_old_width_again_when_the_cursor_leaves(cx: &mut TestAppContext) {
    let (fixture, handle) = open_text_width(cx);
    keys(cx, handle, "space 2 0");
    assert_eq!(chars_field(cx, handle).as_deref(), Some("20"));
    assert_eq!(text_width(&fixture).chars(), 80);

    keys(cx, handle, "x 5 7");
    assert_eq!(
        chars_field(cx, handle).as_deref(),
        Some("205"),
        "digits only, 3 at most"
    );
    assert_eq!(text_width(&fixture).chars(), 80);

    keys(cx, handle, "shift-tab tab");
    assert_eq!(setting_rows(cx, handle)[3], "Text width*");
    assert_eq!(chars_field(cx, handle).as_deref(), Some("80"));
    assert_eq!(question(cx, handle).as_deref(), Some(text::SETTINGS));
}

#[gpui_kit::test]
fn the_text_width_row_keeps_its_height_with_full_and_limited(cx: &mut TestAppContext) {
    let (_fixture, handle) = open_text_width(cx);
    let row = |cx: &mut TestAppContext| {
        screen(cx, handle, |window| {
            let rows = window.within("settings").find_all_by_role(Role::ListItem);
            let parts = ["text-width-chars"]
                .iter()
                .filter_map(|id| window.try_find(*id).map(|part| part.bounds().size.height))
                .collect::<Vec<_>>();
            (rows[3].bounds().size.height, parts)
        })
    };
    let full = row(cx);
    keys(cx, handle, "space");
    let limited = row(cx);
    assert_eq!(full.0, limited.0, "full {full:?}, limited {limited:?}");
}

#[gpui_kit::test]
fn digits_on_a_full_text_width_do_nothing(cx: &mut TestAppContext) {
    let (fixture, handle) = open_text_width(cx);
    keys(cx, handle, "1 2 0");
    assert_eq!(text_width(&fixture), crate::text_width::TextWidthSetting::default());
    assert_eq!(chars_field(cx, handle), None);
}

#[gpui_kit::test]
fn full_and_then_limited_again_shows_the_last_width(cx: &mut TestAppContext) {
    let (fixture, handle) = open_text_width(cx);
    keys(cx, handle, "space 1 2 0 space");
    assert!(!text_width(&fixture).limited);
    assert_eq!(chars_field(cx, handle), None);

    keys(cx, handle, "space");
    assert_eq!(chars_field(cx, handle).as_deref(), Some("120"));
    assert_eq!(text_width(&fixture).text_width(), holm_editor::TextWidth::Limited(120));
    keys(cx, handle, "escape");
    assert_eq!(question(cx, handle), None);
    assert_eq!(focus(cx, handle), "editor");
}

#[gpui_kit::test]
fn ctrl_plus_and_ctrl_minus_step_the_scale_and_answer(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    fs::write(fixture.notes().join("Note.md"), "a\n").unwrap();
    let handle = open(cx, fixture.startup(None));
    let percent = || Config::load(fixture.config()).settings().scale.percent();
    assert_eq!(zoom(cx, handle), 1.);

    keys(cx, handle, "ctrl-+");
    assert_eq!(message(cx, handle).as_deref(), Some("Scale 110 %"));
    assert_eq!(percent(), 110);
    assert_eq!(zoom(cx, handle), 1.1);

    keys(cx, handle, "ctrl-= ctrl-= ctrl-= ctrl-=");
    assert_eq!(message(cx, handle).as_deref(), Some("Scale 200 %, the largest"));
    keys(cx, handle, "ctrl-+");
    assert_eq!(message(cx, handle).as_deref(), Some("Scale 200 %, the largest"));
    assert_eq!(percent(), 200);
    assert_eq!(zoom(cx, handle), 2.);

    keys(cx, handle, "ctrl--");
    assert_eq!(message(cx, handle).as_deref(), Some("Scale 175 %"));
    keys(cx, handle, "ctrl-_ ctrl-_ ctrl-_ ctrl-_ ctrl-_ ctrl-_ ctrl-_");
    assert_eq!(message(cx, handle).as_deref(), Some("Scale 80 %, the smallest"));
    assert_eq!(percent(), 80);
    assert_eq!(zoom(cx, handle), 0.8);
    assert_eq!(editor_text(cx, handle), "a\n", "the keys type nothing");
}

#[gpui_kit::test]
fn a_click_at_a_zoom_of_1_5_hits_the_option_under_it(cx: &mut TestAppContext) {
    use gpui_kit::MouseButton;
    use gpui_kit::test::TestEventExt as _;

    let fixture = Fixture::new().with_folder();
    fixture.write_config(&format!(
        "version = 1\nnotes_folder = {:?}\nscale = 150\n",
        fixture.notes().to_str().unwrap()
    ));
    let handle = open(cx, fixture.startup(None));
    let (viewport, screen_size) = cx
        .update_window(handle, |_, window, _| (window.viewport_size(), window.bounds().size))
        .unwrap();
    assert_eq!(viewport.width * 1.5, screen_size.width);

    keys(cx, handle, "ctrl-shift-s");
    let dark = bounds_of(cx, handle, "Dark").unwrap();
    // The platform gives positions in the pixels of the screen.
    let at = dark.center().map(|c| c * 1.5);
    cx.update_window(handle, |_, window, cx| {
        window.pointer_down(at, MouseButton::Left, cx);
        window.pointer_up(at, MouseButton::Left, cx);
    })
    .unwrap();
    settle(cx);

    assert_eq!(Config::load(fixture.config()).settings().theme, AppearanceSetting::Dark);
}

#[gpui_kit::test]
fn the_theme_options_keep_their_place_when_the_choice_moves(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-shift-s tab");
    // The chosen option covers the line on its left, so only the right edges stay.
    let places = |cx: &mut TestAppContext| {
        let bounds = ["System", "Light", "Dark"].map(|id| bounds_of(cx, handle, id).unwrap());
        (bounds[0].left(), bounds.map(|bounds| bounds.right()))
    };

    let first = places(cx);
    keys(cx, handle, "space");
    assert_eq!(places(cx), first);
    keys(cx, handle, "space");
    assert_eq!(places(cx), first);
}

#[gpui_kit::test]
fn enter_on_the_folder_row_picks_a_new_notes_folder(cx: &mut TestAppContext) {
    let (fixture, other) = two_folders();
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-shift-s");

    keys(cx, handle, "tab enter");
    assert!(!cx.did_prompt_for_paths(), "Enter on a row with options picks nothing");
    keys(cx, handle, "shift-tab enter");
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|_| None);
    settle(cx);
    assert_eq!(question(cx, handle).as_deref(), Some(text::SETTINGS));

    keys(cx, handle, "space");
    cx.simulate_path_prompt_response(|_| Some(vec![other.clone()]));
    settle(cx);

    assert_eq!(editor_text(cx, handle), "a\n");
    assert_eq!(Config::load(fixture.config()).settings().notes_folder, Some(other));
}

#[gpui_kit::test]
fn the_format_keys_row_shows_the_keys_of_the_editor_table(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-shift-s");

    assert_eq!(
        labels(cx, handle, "format-keys", Role::Label),
        ["Ctrl+B", "Ctrl+I", "Ctrl+1–6"]
    );
}

#[gpui_kit::test]
fn a_click_on_the_folder_button_picks_a_new_notes_folder(cx: &mut TestAppContext) {
    let (fixture, other) = two_folders();
    let handle = open(cx, fixture.startup(None));
    keys(cx, handle, "ctrl-shift-s");
    assert_eq!(
        label(cx, handle, "notes-folder"),
        Some(text::home_path(&fixture.notes()))
    );

    click(cx, handle, "notes-folder");
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|_| Some(vec![other.clone()]));
    settle(cx);

    assert_eq!(Config::load(fixture.config()).settings().notes_folder, Some(other));
}
