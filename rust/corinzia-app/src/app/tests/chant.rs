//! Byzantine chants in the Score tab.

use super::*;
use corinzia_byz::{Element, Genus, Quantity, TimeSign};

fn shown(a: &App) -> &corinzia_byz::Chant {
    a.editor.shown_chant().expect("a chant shown")
}

fn neume(a: &App, i: usize) -> &corinzia_byz::Neume {
    match &shown(a).elements[i] {
        Element::Neume(n) => n,
        e => panic!("element {i} is {e:?}"),
    }
}

#[test]
fn the_demo_chant_is_shown_in_place_of_the_movement_and_back() {
    let mut a = app(View::Page);
    let western = a.pages[0].items.len();
    let _ = a.update(Message::Act(Action::Chant(Some(0))));
    assert_eq!(shown(&a).title, "Κύριε ἐλέησον");
    assert_eq!(a.editor.chant_caret, shown(&a).elements.len(), "the caret at the end");
    let byz = corinzia_layout::byz_font::BYZ;
    assert!(a.pages[0].items.iter().any(|i| matches!(i, Item::Glyph { glyph, .. } if glyph & byz != 0)));
    assert!(a.editor.caret_label().starts_with("Ἦχος α΄"), "{}", a.editor.caret_label());
    // The element before the caret is highlighted.
    assert!(a.selected.contains(&corinzia_layout::byz::element_id(a.editor.chant_caret - 1)));
    let _ = a.update(Message::Act(Action::Movement(0)));
    assert!(a.editor.chant.is_none());
    assert_eq!(a.pages[0].items.len(), western);
}

#[test]
fn neumes_signs_and_syllables_are_typed() {
    let mut a = app(View::Page);
    let _ = a.update(Message::Act(Action::AddChant));
    assert_eq!(a.editor.project.chants.len(), 2);
    assert!(shown(&a).elements.is_empty());
    for k in ["0", "1", "1", "a", "shift+e", "y"] {
        key(&mut a, k);
    }
    let qs: Vec<Quantity> = (0..6).map(|i| neume(&a, i).quantity.unwrap()).collect();
    use Quantity::*;
    assert_eq!(qs, [Ison, Oligon, Oligon, Apostrofos, ElafronApostrofos, Yporroi]);
    // Signs go on the neume before the caret.
    key(&mut a, ".");
    assert_eq!(neume(&a, 5).time, Some(TimeSign::Klasma));
    key(&mut a, "left");
    key(&mut a, "g");
    key(&mut a, "g");
    assert_eq!(neume(&a, 4).time, Some(TimeSign::Digorgon));
    key(&mut a, "f");
    key(&mut a, "f");
    let f = neume(&a, 4).fthora.unwrap();
    assert_eq!(f.genus, Genus::HardChromatic);
    // Pa, Vou, Ga, Vou, then elafron with apostrofos down to the low Zo: the fthora names Zo.
    assert_eq!(f.degree, corinzia_byz::Degree::Zo);
    key(&mut a, "end");
    key(&mut a, "m");
    assert!(matches!(shown(&a).elements[6], Element::Martyria(_)));
    key(&mut a, "return");
    assert_eq!(shown(&a).elements[7], Element::Break);
    key(&mut a, "bs");
    key(&mut a, "bs");
    assert_eq!(shown(&a).elements.len(), 6);
    // Syllables as a stream, from the start.
    key(&mut a, "home");
    key(&mut a, "ctrl+l");
    assert_eq!(a.popover.as_ref().map(|p| p.kind), Some(PopoverKind::ChantLyric));
    type_into_popover(&mut a, "Κύ-ρι-ε ");
    assert_eq!([neume(&a, 0).lyric.as_str(), neume(&a, 1).lyric.as_str(), neume(&a, 2).lyric.as_str()], ["Κύ-", "ρι-", "ε"]);
    // The mode, typed.
    let _ = a.update(Message::Popover(PopoverMsg::Close));
    key(&mut a, "shift+m");
    retype(&mut a, "πλ. β΄");
    assert_eq!(shown(&a).mode.echos, corinzia_byz::Echos::PlagalSecond);
    // One undo step each.
    let _ = a.update(Message::Act(Action::Undo));
    assert_eq!(shown(&a).mode.echos, corinzia_byz::Echos::First);
}

#[test]
fn a_click_puts_the_caret_after_a_neume_and_a_chant_exports_to_pdf() {
    let mut a = app(View::Page);
    let _ = a.update(Message::Act(Action::Chant(Some(0))));
    let _ = a.update(Message::Canvas(ViewEvent::Click { hit: Some(corinzia_layout::byz::element_id(2)), at: None, shift: false }));
    assert_eq!(a.editor.chant_caret, 3);
    let path = std::env::temp_dir().join(format!("corinzia-chant-{}.pdf", std::process::id()));
    let _ = a.update(Message::ExportTo(files::Format::Pdf, Some(path.clone())));
    let pdf = std::fs::read(&path).unwrap();
    assert!(pdf.starts_with(b"%PDF"), "{}", a.editor.status);
    std::fs::remove_file(&path).ok();
    let _ = a.update(Message::Act(Action::DeleteChant));
    assert!(a.editor.project.chants.is_empty() && a.editor.chant.is_none());
    let _ = a.update(Message::Act(Action::Undo));
    assert_eq!(a.editor.project.chants.len(), 1);
}
