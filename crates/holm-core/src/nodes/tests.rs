use super::*;

fn note(path: &str) -> Node {
    Node::Note(NotePath::new(path).unwrap())
}

fn folder(path: &str) -> Node {
    Node::Folder(FolderPath::new(path).unwrap())
}

#[test]
fn a_folder_node_holds_itself_and_what_is_below_it() {
    let work = folder("work");
    assert!(work.holds(&work));
    assert!(work.holds(&note("work/a.md")));
    assert!(work.holds(&folder("work/x")));
    assert!(!work.holds(&folder("workshop")));
    assert!(folder("").holds(&note("a.md")));
    let a = note("a.md");
    assert!(a.holds(&a));
    assert!(!a.holds(&folder("a.md")));
}

#[test]
fn nodes_drop_a_node_inside_another_and_repeats() {
    let nodes = Nodes::new([
        note("work/a.md"),
        folder("work/x"),
        folder("work"),
        note("b.md"),
        note("b.md"),
        folder("workshop"),
    ]);
    assert_eq!(nodes.0, [note("b.md"), folder("work"), folder("workshop")]);
    assert_eq!(Nodes::new([note("a.md"), folder("")]).0, [folder("")]);
}
