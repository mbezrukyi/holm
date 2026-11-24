use super::*;

#[test]
fn each_group_shows_each_of_its_rows_once_and_the_plain_rows_first() {
    let list = shortcuts::list();
    let mut shown = Vec::new();
    for group in [Group::Text, Group::Note, Group::Sidebar, Group::Window] {
        let sections = sections(&list, group);
        let mut headings: Vec<_> = sections.iter().map(|(heading, _)| *heading).collect();
        assert!(headings.iter().skip(1).all(Option::is_some), "{group:?}");
        headings.dedup();
        assert_eq!(headings.len(), sections.len(), "one section per heading");
        for (heading, rows) in sections {
            for at in rows {
                assert_eq!(list[at].group, group);
                assert_eq!(super::heading(list[at].place), heading);
                shown.push(at);
            }
        }
    }
    shown.sort_unstable();
    assert_eq!(shown, (0..list.len()).collect::<Vec<_>>());
}
