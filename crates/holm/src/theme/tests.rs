use gpui_kit::{FontWeight, font};

use super::*;

#[test]
fn the_ui_font_has_a_face_for_each_weight() {
    let text_system = gpui_kit::platform::current_platform(true).text_system();
    let fonts = UI_FONT_FILES.iter().map(|file| Cow::Borrowed(*file)).collect();
    text_system.add_fonts(fonts).unwrap();
    let face = |weight| {
        let mut ui = font("Inter");
        ui.weight = weight;
        text_system.font_id(&ui).unwrap()
    };
    let regular = face(FontWeight::NORMAL);
    assert_ne!(regular, face(FontWeight::MEDIUM));
    assert_ne!(regular, face(FontWeight::SEMIBOLD));
    assert_ne!(face(FontWeight::SEMIBOLD), face(FontWeight::BOLD));
}
