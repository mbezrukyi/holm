use std::fs;

use gpui_kit::TestAppContext;

use super::{
    Fixture, app_menu, click, editor_text, empty_pane, files, folder_screen, keys, open, question, window_open,
};
use crate::config::Config;
use crate::text;

#[gpui_kit::test]
fn the_first_start_asks_for_the_folder_and_keeps_the_pick(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    fs::write(fixture.notes().join("Plans.md"), "# Plans\n").unwrap();
    let handle = open(cx, fixture.startup(None));
    assert_eq!(folder_screen(cx, handle).as_deref(), Some(text::ASK_FOLDER));

    click(cx, handle, "pick-folder");
    assert!(cx.did_prompt_for_paths());
    let notes = fixture.notes();
    cx.simulate_path_prompt_response(|_| Some(vec![notes]));
    cx.run_until_parked();

    assert_eq!(editor_text(cx, handle), "# Plans\n");
    let config = Config::load(fixture.config());
    assert_eq!(config.settings().notes_folder, Some(fixture.notes()));
}

#[gpui_kit::test]
fn the_first_start_can_make_the_default_folder(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let handle = open(cx, fixture.startup(None));

    click(cx, handle, "use-default");

    assert_eq!(folder_screen(cx, handle), None);
    assert!(empty_pane(cx, handle));
    let default = fixture.dir.path().join("Notes");
    assert_eq!(files(&default), Vec::<String>::new());
    let config = Config::load(fixture.config());
    assert_eq!(config.settings().notes_folder, Some(default));
}

#[gpui_kit::test]
fn a_missing_folder_offers_to_pick_another(cx: &mut TestAppContext) {
    let fixture = Fixture::new().with_folder();
    fs::remove_dir(fixture.notes()).unwrap();
    let handle = open(cx, fixture.startup(None));
    assert_eq!(
        folder_screen(cx, handle),
        Some(text::folder_missing(&fixture.notes()).to_string())
    );

    let other = fixture.dir.path().join("other");
    fs::create_dir(&other).unwrap();
    fs::write(other.join("A.md"), "a\n").unwrap();
    click(cx, handle, "pick-folder");
    cx.simulate_path_prompt_response(|_| Some(vec![other]));
    cx.run_until_parked();

    assert_eq!(editor_text(cx, handle), "a\n");
}

#[gpui_kit::test]
fn the_folder_screen_ignores_ctrl_shift_s(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let handle = open(cx, fixture.startup(None));
    assert_eq!(folder_screen(cx, handle).as_deref(), Some(text::ASK_FOLDER));

    keys(cx, handle, "ctrl-shift-s");
    assert_eq!(question(cx, handle), None);
}

#[gpui_kit::test]
fn the_folder_screen_ignores_ctrl_question_mark(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let handle = open(cx, fixture.startup(None));
    assert_eq!(folder_screen(cx, handle).as_deref(), Some(text::ASK_FOLDER));

    keys(cx, handle, "ctrl-?");
    assert_eq!(question(cx, handle), None);
}

#[gpui_kit::test]
fn the_folder_screen_ignores_f10_and_quits_with_ctrl_shift_q(cx: &mut TestAppContext) {
    let fixture = Fixture::new();
    let handle = open(cx, fixture.startup(None));
    assert_eq!(folder_screen(cx, handle).as_deref(), Some(text::ASK_FOLDER));

    keys(cx, handle, "f10");
    assert_eq!(app_menu(cx, handle), Vec::<String>::new());
    assert_eq!(folder_screen(cx, handle).as_deref(), Some(text::ASK_FOLDER));
    keys(cx, handle, "ctrl-shift-q");
    assert!(!window_open(cx, handle));
}
