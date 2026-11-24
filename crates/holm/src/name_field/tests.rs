use std::cell::{Cell, RefCell};
use std::path::PathBuf;

use gpui_kit::{TestAppContext, VisualTestContext};

use super::*;

type Events = Rc<RefCell<Vec<NameEvent<String>>>>;

/// A job that refuses the first name, and then writes each name.
fn refuse_once() -> NameWrite<String> {
    let calls = Rc::new(Cell::new(0));
    Rc::new(move |name, done, _, cx| {
        calls.set(calls.get() + 1);
        let result = if calls.get() == 1 {
            Err(AppError::Core(holm_core::Error::NameTaken {
                path: PathBuf::from(&name),
            }))
        } else {
            Ok(name)
        };
        done(result, cx);
    })
}

fn field(cx: &mut TestAppContext) -> (Entity<NameField<String>>, Events, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::shortcuts::bind(cx);
    });
    let wording = Wording {
        id: "name",
        label: "Name".into(),
        placeholder: SharedString::default(),
    };
    let (field, cx) = cx.add_window_view(|window, cx| {
        let home = cx.focus_handle();
        NameField::rename(wording, "Old", refuse_once(), home, window, cx)
    });
    let events: Events = Rc::default();
    let seen = Rc::clone(&events);
    cx.update(|_, cx| {
        cx.subscribe(&field, move |_, event: &NameEvent<String>, _| {
            seen.borrow_mut().push(event.clone());
        })
        .detach();
    });
    (field, events, cx)
}

#[gpui_kit::test]
fn a_refused_name_keeps_its_text_and_the_next_write_closes_the_field(cx: &mut TestAppContext) {
    let (field, events, cx) = field(cx);

    cx.simulate_input("Taken");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    field.read_with(cx, |field, cx| {
        assert!(!field.writing());
        assert_eq!(field.field.read(cx).text(), "Taken");
    });
    assert!(events.borrow().is_empty());

    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(*events.borrow(), [NameEvent::Written("Taken".to_owned())]);
}

#[gpui_kit::test]
fn the_unchanged_name_closes_the_field_with_no_write(cx: &mut TestAppContext) {
    let (_, events, cx) = field(cx);

    cx.simulate_input(" Old ");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(*events.borrow(), [NameEvent::Closed]);
}
