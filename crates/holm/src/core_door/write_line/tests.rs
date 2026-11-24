use std::cell::Cell;
use std::rc::Rc;
use std::task::Waker;

use gpui_kit::TestAppContext;
use holm_core::FolderPath;
use tempfile::TempDir;

use super::*;

fn line() -> (TempDir, WriteLine<u32>) {
    let dir = tempfile::tempdir().unwrap();
    let notes = dir.path().join("notes");
    std::fs::create_dir(&notes).unwrap();
    let (core, _) = Core::open_with_feed(&notes, dir.path().join("index")).unwrap();
    let (_, changes) = core.follow();
    (dir, WriteLine::new(core, changes))
}

fn folder(name: &'static str) -> Work {
    Box::new(move |core| {
        core.create_folder(&FolderPath::root(), name)
            .err()
            .map(|error| error.to_string())
    })
}

/// The ends in their order, and the count of the lists.
fn ends(run: &Run<u32>) -> (Vec<u32>, usize) {
    (run.ended.clone(), run.lists.len())
}

#[gpui_kit::test]
fn the_jobs_in_the_budget_end_in_order_with_the_lists_of_all(cx: &mut TestAppContext) {
    let (_dir, mut line) = line();
    line.push(folder("B"), 1);
    line.push(Box::new(|_| None), 2);
    let last = line.push(folder("C"), 3);

    let run = cx.update(|cx| line.run(Some(last), Duration::from_secs(10), cx));

    assert_eq!(ends(&run), (vec![1, 2, 3], 2));
    assert!(run.wake.is_none());
    assert!(!line.is_late());
}

#[gpui_kit::test]
fn a_job_past_its_budget_is_late_and_its_wake_lets_the_next_run_end_it(cx: &mut TestAppContext) {
    let (_dir, mut line) = line();
    let number = line.push(folder("B"), 1);
    let run = cx.update(|cx| line.run(Some(number), Duration::ZERO, cx));
    assert_eq!(ends(&run), (vec![], 0));
    assert!(line.is_late());
    let mut waker = task::Context::from_waker(Waker::noop());
    assert!(line.outside(&mut waker).is_none());

    let seen = Rc::new(Cell::new(false));
    let wake = run.wake.unwrap();
    cx.update(|cx| {
        let seen = seen.clone();
        cx.spawn(async move |_| {
            wake.await;
            seen.set(true);
        })
        .detach();
    });
    cx.run_until_parked();
    assert!(seen.get());

    let run = cx.update(|cx| line.run(None, Duration::ZERO, cx));
    assert_eq!(ends(&run), (vec![1], 1));
    assert!(run.wake.is_none());
    assert!(!line.is_late());
    assert_eq!(line.outside(&mut waker).map(|lists| lists.len()), Some(0));
}

#[gpui_kit::test]
fn the_drain_runs_the_jobs_and_then_the_line_gives_no_end(cx: &mut TestAppContext) {
    let (dir, mut line) = line();
    let number = line.push(folder("B"), 1);
    cx.update(|cx| line.run(Some(number), Duration::ZERO, cx));
    line.push(folder("C"), 2);

    line.drain(folder("D"));
    let later = line.push(folder("E"), 3);
    cx.run_until_parked();
    let run = cx.update(|cx| line.run(Some(later), Duration::from_secs(10), cx));

    assert_eq!(ends(&run), (vec![], 0));
    assert!(run.wake.is_none());
    let notes = dir.path().join("notes");
    for name in ["B", "C", "D"] {
        assert!(notes.join(name).is_dir(), "{name}");
    }
    assert!(!notes.join("E").exists());
}
