use std::cell::RefCell;
use std::fs;
use std::rc::Rc;

use gpui_kit::{Entity, TestAppContext};
use holm_core::testing::{folder_path, note_path, tag_name};
use tempfile::TempDir;

use super::*;

fn core_door(cx: &mut TestAppContext) -> (TempDir, Entity<CoreDoor>) {
    let (dir, core_door, _) = fed_core_door(cx);
    (dir, core_door)
}

/// A door to a notes folder with `A.md`, and the feed that tells its core about outside changes.
fn fed_core_door(cx: &mut TestAppContext) -> (TempDir, Entity<CoreDoor>, holm_core::Feed) {
    let dir = tempfile::tempdir().unwrap();
    let notes = dir.path().join("notes");
    fs::create_dir(&notes).unwrap();
    fs::write(notes.join("A.md"), "# A\n").unwrap();
    let (core, feed) = Core::open_with_feed(&notes, dir.path().join("index")).unwrap();
    let window = cx.add_window(|_, _| gpui_kit::Empty);
    let core_door = window
        .update(cx, |_, window, cx| {
            cx.new(|cx| CoreDoor::new(core, notes, Duration::ZERO, window, cx))
        })
        .unwrap();
    (dir, core_door, feed)
}

#[gpui_kit::test]
fn a_write_after_a_failed_write_still_runs(cx: &mut TestAppContext) {
    let (dir, core_door) = core_door(cx);
    let steps = Rc::new(RefCell::new(Vec::new()));
    core_door.update(cx, |core_door, cx| {
        let step = |name: &'static str| {
            let steps = steps.clone();
            Done::result(move |result: Result<FolderPath, AppError>, _: &mut App| {
                if result.is_ok() {
                    steps.borrow_mut().push(name);
                }
            })
        };
        core_door.rename_folder(folder_path("gone"), "x".into(), step("after the fail"), cx);
        core_door.create_folder(FolderPath::root(), "B".into(), step("after the folder"), cx);
    });
    cx.run_until_parked();

    assert_eq!(*steps.borrow(), ["after the folder"]);
    assert!(dir.path().join("notes/B").is_dir());
    let folders: Vec<FolderPath> =
        core_door.read_with(cx, |core_door, _| core_door.snapshot().folders().cloned().collect());
    assert_eq!(folders, [folder_path("B")]);
}

#[gpui_kit::test]
fn a_failed_job_sends_its_answer_and_skips_its_value_step(cx: &mut TestAppContext) {
    let (_dir, core_door) = core_door(cx);
    let steps = Rc::new(RefCell::new(Vec::new()));
    cx.update(|cx| {
        let steps = steps.clone();
        cx.subscribe(&core_door, move |_, JobFailed(answer), _| {
            steps.borrow_mut().push(format!("answer {answer}"));
        })
        .detach();
    });
    core_door.update(cx, |core_door, cx| {
        let value = {
            let steps = steps.clone();
            Done::value(move |_, _| steps.borrow_mut().push("value".to_owned()))
        };
        core_door.rename_folder(folder_path("gone"), "x".into(), value, cx);
        let result = {
            let steps = steps.clone();
            Done::result(move |result: Result<FolderPath, AppError>, _| {
                let message = result.err().map(|error| error.message());
                steps.borrow_mut().push(format!("result {message:?}"));
            })
        };
        core_door.rename_folder(folder_path("gone"), "x".into(), result, cx);
    });
    cx.run_until_parked();

    let answer = crate::text::READ;
    assert_eq!(
        *steps.borrow(),
        [
            format!("answer {answer}"),
            format!("answer {answer}"),
            format!("result Some({answer:?})")
        ]
    );
}

#[gpui_kit::test]
fn each_write_sends_its_moves_and_then_gives_its_result(cx: &mut TestAppContext) {
    let (_dir, core_door) = core_door(cx);
    let steps = Rc::new(RefCell::new(Vec::new()));
    cx.update(|cx| {
        let steps = steps.clone();
        cx.subscribe(&core_door, move |_, event: &DoorEvent, _| {
            steps
                .borrow_mut()
                .push(format!("event {:?}", event.changes().collect::<Vec<_>>()));
        })
        .detach();
    });
    core_door.update(cx, |core_door, cx| {
        let step = |name: &'static str| {
            let steps = steps.clone();
            Done::result(move |result: Result<FolderPath, AppError>, _: &mut App| {
                let end = if result.is_ok() { "ok" } else { "failed" };
                steps.borrow_mut().push(format!("{name} {end}"));
            })
        };
        core_door.create_folder(FolderPath::root(), "work".into(), step("create"), cx);
        core_door.rename_folder(folder_path("work"), "job".into(), step("rename"), cx);
        core_door.rename_folder(folder_path("gone"), "x".into(), step("fail"), cx);
    });
    cx.run_until_parked();

    assert_eq!(
        *steps.borrow(),
        [
            "event [Folders]",
            "create ok",
            r#"event [FolderMoved { from: "work", to: "job" }, Folders]"#,
            "rename ok",
            "event []",
            "fail failed",
        ]
    );
}

#[gpui_kit::test]
fn the_writes_in_the_line_reach_the_files_at_quit_with_the_draft_last(cx: &mut TestAppContext) {
    let (dir, core_door) = core_door(cx);
    core_door.update(cx, |core_door, cx| {
        core_door.set_text(note_path("A.md"), "# A\n\nfirst\n".into(), cx);
        core_door.change_tag(
            note_path("A.md"),
            TagChange::Add(tag_name("work")),
            Done::value(|(), _| {}),
            cx,
        );
        core_door.create_folder(FolderPath::root(), "B".into(), Done::value(|_, _| {}), cx);
        core_door.set_text(note_path("A.md"), "# A\n\nlast\n".into(), cx);
    });
    cx.quit();

    let file = fs::read_to_string(dir.path().join("notes/A.md")).unwrap();
    assert!(file.contains("  - work\n"), "{file}");
    assert!(file.ends_with("# A\n\nlast\n"), "{file}");
    assert!(dir.path().join("notes/B").is_dir());
}

#[gpui_kit::test]
fn a_job_sees_the_draft_and_a_newer_draft_waits_for_the_next_job(cx: &mut TestAppContext) {
    let (_dir, core_door) = core_door(cx);
    let bodies = Rc::new(RefCell::new(Vec::new()));
    let read = |core_door: &mut CoreDoor, cx: &mut Context<CoreDoor>| {
        let bodies = bodies.clone();
        core_door.read(
            note_path("A.md"),
            Done::value(move |note: Note, _| bodies.borrow_mut().push(note.body)),
            cx,
        );
    };
    core_door.update(cx, |core_door, cx| {
        core_door.set_text(note_path("A.md"), "# A\n\nold\n".into(), cx);
        core_door.set_text(note_path("A.md"), "# A\n\nnew\n".into(), cx);
        read(core_door, cx);
        read(core_door, cx);
        core_door.set_text(note_path("A.md"), "# A\n\nlater\n".into(), cx);
    });
    cx.run_until_parked();
    core_door.update(cx, |core_door, cx| read(core_door, cx));
    cx.run_until_parked();

    assert_eq!(*bodies.borrow(), ["# A\n\nnew\n", "# A\n\nnew\n", "# A\n\nlater\n"]);
}

#[gpui_kit::test]
fn a_failed_save_keeps_the_text_for_the_next_save(cx: &mut TestAppContext) {
    let (dir, core_door) = core_door(cx);
    let notes = dir.path().join("notes");
    let read_only = |read_only: bool| {
        let mut permissions = fs::metadata(&notes).unwrap().permissions();
        permissions.set_readonly(read_only);
        fs::set_permissions(&notes, permissions).unwrap();
    };
    let steps = Rc::new(RefCell::new(Vec::new()));
    cx.update(|cx| {
        let steps = steps.clone();
        cx.subscribe(&core_door, move |_, event: &SaveEvent, _| {
            let end = match event {
                SaveEvent::Saved => "saved",
                SaveEvent::Failed => "failed",
                SaveEvent::Changed => "changed",
            };
            steps.borrow_mut().push(end.to_owned());
        })
        .detach();
    });
    cx.update(|cx| {
        let steps = steps.clone();
        cx.subscribe(&core_door, move |_, JobFailed(answer), _| {
            steps.borrow_mut().push(format!("answer {answer}"));
        })
        .detach();
    });
    let wait = |cx: &mut TestAppContext| {
        let steps = steps.clone();
        core_door.update(cx, |core_door, cx| {
            core_door.wait_saved(move |saved, _| steps.borrow_mut().push(format!("wait {saved:?}")), cx);
        });
        cx.run_until_parked();
    };

    read_only(true);
    core_door.update(cx, |core_door, cx| {
        core_door.set_text(note_path("A.md"), "# A\n\nmine\n".into(), cx);
        core_door.save_text(cx);
    });
    cx.run_until_parked();
    wait(cx);
    assert!(core_door.read_with(cx, |core_door, _| core_door.unsaved()));

    read_only(false);
    wait(cx);
    assert!(!core_door.read_with(cx, |core_door, _| core_door.unsaved()));
    assert_eq!(
        *steps.borrow(),
        [
            format!("answer {}", crate::text::WRITE),
            "failed".to_owned(),
            "failed".to_owned(),
            "wait Err(NotSaved)".to_owned(),
            "saved".to_owned(),
            "wait Ok(())".to_owned()
        ]
    );
    let text = fs::read_to_string(notes.join("A.md")).unwrap();
    assert!(text.ends_with("\n---\n# A\n\nmine\n"), "{text}");
}

#[gpui_kit::test]
fn a_save_keeps_a_tag_from_another_tool(cx: &mut TestAppContext) {
    let (dir, core_door, feed) = fed_core_door(cx);
    let file = dir.path().join("notes/A.md");
    fs::write(&file, "---\ntags: [theirs]\n---\n# A\n").unwrap();
    feed.changed(["A.md"]).unwrap();
    cx.run_until_parked();

    core_door.update(cx, |core_door, cx| {
        core_door.set_text(note_path("A.md"), "# A\n\nmine\n".into(), cx);
        core_door.save_text(cx);
    });
    cx.run_until_parked();

    let text = fs::read_to_string(&file).unwrap();
    assert!(text.contains("tags: [theirs]\n"), "{text}");
    assert!(text.ends_with("# A\n\nmine\n"), "{text}");
}

#[gpui_kit::test]
fn a_tag_removal_keeps_a_tag_from_another_tool(cx: &mut TestAppContext) {
    let (dir, core_door) = core_door(cx);
    let file = dir.path().join("notes/A.md");
    let change = |change: TagChange, cx: &mut TestAppContext| {
        core_door.update(cx, |core_door, cx| {
            core_door.change_tag(note_path("A.md"), change, Done::value(|(), _| {}), cx);
        });
        cx.run_until_parked();
    };

    change(TagChange::Add(tag_name("a")), cx);
    fs::write(&file, "---\ntags: [a, b]\n---\n# A\n").unwrap();
    change(TagChange::Remove(tag_name("a")), cx);

    let tags = holm_markdown::NoteFile::parse(&fs::read_to_string(&file).unwrap()).tags();
    assert_eq!(tags, ["b"]);
}
