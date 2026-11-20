//! The motif bank: taken from the score, named and described, kept,
//! shown, heard and removed.

use super::*;
use corinzia_model::motif::Role;

#[test]
fn a_passage_on_one_staff_becomes_a_motif() {
    let mut a = app(View::Page);
    let mm = Message::Motif;
    // Two bars of the first staff.
    a.editor.select_bar(0, Frac::ZERO, false);
    a.editor.select_bar(0, Frac::int(1), true);
    let _ = a.update(Message::Act(Action::AddMotif));
    let d = a.motif_dialog.as_ref().expect("the card asks what it is");
    assert!(d.new && !d.draft.notes.is_empty() && d.draft.roles.is_empty());
    assert!(d.name.contains("bars 1–2"), "{}", d.name);
    assert!(d.facts.iter().any(|f| f.0 == "Range"));
    // Named, a melody and a call, tagged.
    let _ = a.update(mm(MotifMsg::Name("Opening call".into())));
    let _ = a.update(mm(MotifMsg::Role(Role::Melody)));
    let _ = a.update(mm(MotifMsg::Role(Role::Call)));
    let _ = a.update(mm(MotifMsg::Tags("lyrical, rising ,".into())));
    let _ = a.update(mm(MotifMsg::Save));
    assert!(a.motif_dialog.is_none());
    let bank = &a.editor.project.motifs;
    assert_eq!(bank.len(), 1);
    let m = &bank[0];
    assert_eq!((m.name.as_str(), m.roles.clone(), m.tags.clone()), ("Opening call", vec![Role::Melody, Role::Call], vec!["lyrical".to_string(), "rising".to_string()]));
    assert_eq!(m.source.as_ref().map(|s| (s.lane, s.bars)), Some((0, (1, 2))));
    let id = m.id;
    // One undo step, and back.
    let _ = a.update(Message::Act(Action::Undo));
    assert!(a.editor.project.motifs.is_empty());
    let _ = a.update(Message::Act(Action::Redo));
    assert_eq!(a.editor.project.motifs.len(), 1);
    // Edited: renamed, a role dropped.
    let _ = a.update(Message::Act(Action::EditMotif(id)));
    assert!(!a.motif_dialog.as_ref().unwrap().new);
    let _ = a.update(mm(MotifMsg::Name("Call".into())));
    let _ = a.update(mm(MotifMsg::Role(Role::Melody)));
    let _ = a.update(mm(MotifMsg::Save));
    assert_eq!((a.editor.project.motifs[0].name.as_str(), a.editor.project.motifs[0].roles.clone()), ("Call", vec![Role::Call]));
    // Shown: its passage selected in the score.
    a.editor.clear_selection();
    let _ = a.update(Message::Act(Action::ShowExplore));
    let _ = a.update(Message::Act(Action::ShowMotif(id)));
    assert_eq!(a.tab, Tab::Score);
    let r = a.editor.range.unwrap();
    assert_eq!((r.lane_from, r.lane_to, r.from, r.to), (0, 0, Frac::ZERO, Frac::int(2)));
    // Heard on its instrument.
    let _ = a.update(Message::Act(Action::AuditionMotif(id)));
    render(&mut a, 4800);
    assert!(a.playback.is_playing());
    let _ = a.update(Message::Act(Action::PlayStop));
    // Deleted from its card.
    let _ = a.update(Message::Act(Action::EditMotif(id)));
    let _ = a.update(mm(MotifMsg::Delete));
    assert!(a.editor.project.motifs.is_empty());
}

#[test]
fn a_motif_needs_one_staff_and_a_name() {
    let mut a = app(View::Page);
    // Nothing selected, or two staves: refused with a reason.
    a.editor.clear_selection();
    let _ = a.update(Message::Act(Action::AddMotif));
    assert!(a.motif_dialog.is_none() && a.editor.status.contains("one staff"), "{}", a.editor.status);
    a.editor.select_bar(0, Frac::ZERO, false);
    a.editor.select_bar(1, Frac::ZERO, true);
    let _ = a.update(Message::Act(Action::AddMotif));
    assert!(a.motif_dialog.is_none() && a.editor.status.contains("one staff"));
    // An unnamed motif is not filed.
    a.editor.select_bar(0, Frac::ZERO, false);
    let _ = a.update(Message::Act(Action::AddMotif));
    let _ = a.update(Message::Motif(MotifMsg::Name("  ".into())));
    let _ = a.update(Message::Motif(MotifMsg::Save));
    assert!(a.motif_dialog.as_ref().is_some_and(|d| d.error.is_some()));
    assert!(a.editor.project.motifs.is_empty());
    // The keys: Ctrl+M adds, Ctrl+3 shows Explore.
    let ctrl = |c: &str| keys::map(&iced::keyboard::Key::Character(c.into()), iced::keyboard::Modifiers::CTRL, false);
    assert_eq!((ctrl("m"), ctrl("3")), (Some(Action::AddMotif), Some(Action::ShowExplore)));
}
