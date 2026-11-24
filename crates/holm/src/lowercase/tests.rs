use super::{Lowercase, LowercaseList};

#[test]
fn a_text_holds_a_part_with_no_regard_to_case() {
    let text = Lowercase::new("Garden Plans");
    assert!(text.holds(&Lowercase::new("PLAN")));
    assert!(text.holds(&Lowercase::new("")));
    assert!(!text.holds(&Lowercase::new("plants")));
    assert!(Lowercase::new("Über Öl").holds(&Lowercase::new("öl")));
}

#[test]
fn a_list_keeps_its_texts_in_order() {
    let list = LowercaseList::new(["Plan", "", "Über"].into_iter());
    let texts: Vec<String> = list.iter().map(|text| text.as_str().to_owned()).collect();
    assert_eq!(texts, ["plan", "", "über"]);
}
