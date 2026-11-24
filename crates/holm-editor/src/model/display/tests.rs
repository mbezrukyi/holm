use super::*;
use crate::model::Selection;

fn model_at(text: &str, cursor: usize) -> Model {
    let mut model = Model::new(text);
    model.set_cursor(SourceOffset::new(cursor), Selection::Collapse);
    model
}

#[test]
fn list_items_hide_their_markers_and_know_their_depth() {
    let model = model_at("x\n\n- one\n  - two\n1. three\n- [x] done\n", 0);
    let one = model.display_line(LineIndex(2), &mut plain_chip);
    assert_eq!(one.shown.text, "one");
    assert_eq!(one.mark, Some(Mark::Bullet));
    assert_eq!(one.depth, 1);
    let two = model.display_line(LineIndex(3), &mut plain_chip);
    assert_eq!(two.shown.text, "two");
    assert_eq!(two.depth, 2);
    assert_eq!(
        model.display_line(LineIndex(4), &mut plain_chip).mark,
        Some(Mark::Number("1.".into()))
    );
    let done = model.display_line(LineIndex(5), &mut plain_chip);
    assert_eq!(done.shown.text, "done");
    assert!(matches!(
        done.mark,
        Some(Mark::Task {
            state: TaskState::Checked,
            ..
        })
    ));
}

#[test]
fn a_chip_has_the_link_text_as_its_name() {
    let model = model_at(
        "x\n\n![photo.png](attachments/photo-3fa9c2e1.png) ![](a/b.pdf) ![a\\_b.toml](a_b.toml)\n",
        0,
    );
    let line = model.display_line(LineIndex(2), &mut plain_chip);
    let names: Vec<_> = line.shown.chips.iter().map(|chip| chip.name.as_str()).collect();
    assert_eq!(names, ["photo.png", "", "a_b.toml"]);
    assert_eq!(line.shown.chips[0].target, "attachments/photo-3fa9c2e1.png");
}

#[test]
fn a_list_line_with_the_cursor_shows_its_marker() {
    let model = model_at("- one\n", 3);
    let line = model.display_line(LineIndex(0), &mut plain_chip);
    assert_eq!(line.shown.text, "- one");
    assert_eq!(line.mark, None);
    assert_eq!(line.prefix, Some(SourceOffset::range(0..2)));
}

#[test]
fn a_code_block_shows_its_fences_while_the_cursor_is_inside() {
    let text = "a\n```rust\nfn x() {}\n```\nb";
    let away = model_at(text, 0);
    let fence = away.display_line(LineIndex(1), &mut plain_chip);
    assert_eq!(fence.syntax, Syntax::Hidden);
    assert!(matches!(
        fence.kind,
        LineKind::Code {
            line: CodeLine::Fence,
            ..
        }
    ));
    let LineKind::Code {
        language,
        code,
        line: CodeLine::Code,
    } = away.display_line(LineIndex(2), &mut plain_chip).kind
    else {
        panic!("line 2 is code");
    };
    assert_eq!(language, "rust");
    assert_eq!(&text[code.bytes()], "fn x() {}");
    assert!(matches!(
        away.display_line(LineIndex(3), &mut plain_chip).kind,
        LineKind::Code {
            line: CodeLine::Fence,
            ..
        }
    ));

    let inside = model_at(text, 12);
    assert_eq!(inside.display_line(LineIndex(1), &mut plain_chip).syntax, Syntax::Shown);
    assert_eq!(inside.display_line(LineIndex(3), &mut plain_chip).syntax, Syntax::Shown);
    assert_eq!(inside.display_line(LineIndex(1), &mut plain_chip).shown.text, "```rust");
}

#[test]
fn a_table_is_a_grid_until_the_cursor_is_inside() {
    let text = "a\n\n| x | **y** |\n|---|--:|\n| 1 | 2 |\n";
    let away = model_at(text, 0);
    let LineKind::Row { part, cells, .. } = away.display_line(LineIndex(2), &mut plain_chip).kind else {
        panic!("line 2 is a row");
    };
    assert_eq!(part, TablePart::Head);
    assert_eq!(cells[0].shown.text, "x");
    assert_eq!(cells[1].shown.text, "y");
    assert_eq!(cells[1].alignment, Alignment::Right);
    assert!(matches!(
        away.display_line(LineIndex(3), &mut plain_chip).kind,
        LineKind::TableRule
    ));

    let inside = model_at(text, text.find('2').unwrap_or(0));
    for line in LineIndex(2).through(LineIndex(4)) {
        let display = inside.display_line(line, &mut plain_chip);
        assert_eq!(display.syntax, Syntax::Shown);
        assert!(matches!(display.kind, LineKind::Text));
    }
}

#[test]
fn a_callout_shows_its_name_and_its_quote() {
    let model = model_at("a\n\n> [!tip]\n> Use it.\n", 0);
    let title = model.display_line(LineIndex(2), &mut plain_chip);
    assert!(matches!(title.kind, LineKind::CalloutTitle(CalloutKind::Tip)));
    let body = model.display_line(LineIndex(3), &mut plain_chip);
    assert_eq!(body.quotes, [Quote::Callout(CalloutKind::Tip)]);
    assert_eq!(body.shown.text, "Use it.");
}

#[test]
fn a_rule_away_from_the_cursor_shows_no_text() {
    let model = model_at("a\n\n---\n", 0);
    let line = model.display_line(LineIndex(2), &mut plain_chip);
    assert!(matches!(line.kind, LineKind::Rule));
    assert_eq!(line.shown.text, "");
}

#[test]
fn task_boxes_toggle_and_the_cursor_stays() {
    let mut model = model_at("- [ ] a\n- [x] b\n- c\n", 0);
    assert!(model.toggle_task(&SourceOffset::range(2..5)));
    assert_eq!(model.text(), "- [x] a\n- [x] b\n- c\n");
    assert_eq!(model.cursor(), SourceOffset::new(0));
    model.select(SourceOffset::range(0..12));
    assert!(model.toggle_tasks());
    assert_eq!(model.text(), "- [ ] a\n- [ ] b\n- c\n");
    model.set_cursor(SourceOffset::new(17), Selection::Collapse);
    assert!(!model.toggle_tasks());
    model.undo();
    assert_eq!(model.text(), "- [x] a\n- [x] b\n- c\n");
}
