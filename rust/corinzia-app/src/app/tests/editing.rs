//! Editor behaviour that needs the app's pieces: the demo piece, the page
//! layout and the key map.

use crate::demo;
use crate::editor::*;
use corinzia_model::*;
use corinzia_edit as edit;
use corinzia_score::score::{art, span};
use corinzia_score::Frac;

fn f(n: i64, d: i64) -> Frac {
    Frac::new(n, d)
}
fn events(e: &Editor, lane: usize, voice: usize) -> Vec<Event> {
    e.movement().unwrap().lanes[lane].voices[voice].events.to_vec()
}
fn notes(e: &Editor, lane: usize) -> Vec<String> {
    events(e, lane, 0).iter().map(|e| if e.is_rest() { "r".into() } else { e.heads.iter().map(|h| corinzia_edit::names::pitch_name(h.pitch)).collect::<Vec<_>>().join("+") }).collect()
}

fn blank() -> Editor {
    Editor::new(crate::editor::instruments::blank("Test", &[("Flute", "Fl.", "treble"), ("Cello", "Vc.", "bass")], 2))
}
fn quarters_cdef(e: &mut Editor) {
    e.set_input_mode(true);
    for step in [0, 1, 2, 3] {
        e.enter_note(step, false);
    }
    e.set_input_mode(false);
}

#[test]
fn movements_are_added_switched_renamed_and_deleted() {
    let mut e = blank();
    quarters_cdef(&mut e);
    e.apply_popover(PopoverKind::Key, "D").unwrap();
    e.add_movement();
    assert_eq!(e.project.movements.len(), 2);
    assert_eq!(e.movement, 1);
    let m = e.movement().unwrap();
    // The same staves and key, empty.
    assert_eq!(m.lanes.len(), 2);
    assert_eq!(m.keys[0].fifths, 2);
    assert!(m.lanes.iter().all(|l| l.voices[0].events.is_empty()));
    assert_eq!(m.title, "Movement 2");
    e.apply_popover(PopoverKind::MovementTitle, "II. Scherzo").unwrap();
    assert_eq!(e.movement().unwrap().title, "II. Scherzo");
    // Notes typed here stay here.
    e.set_input_mode(true);
    e.enter_note(4, false);
    e.set_input_mode(false);
    e.switch_movement(0);
    assert_eq!(notes(&e, 0)[..4], ["C5", "D5", "E5", "F5"]);
    e.switch_movement(1);
    assert_eq!(notes(&e, 0)[0], "G4");
    // Delete it; undo brings it back.
    e.delete_movement();
    assert_eq!((e.project.movements.len(), e.movement), (1, 0));
    e.undo();
    assert_eq!(e.project.movements.len(), 2);
    assert_eq!(e.project.movements[1].title, "II. Scherzo");
    // The last movement cannot go.
    e.switch_movement(0);
    e.delete_movement();
    e.delete_movement();
    assert_eq!(e.project.movements.len(), 1);
    assert_eq!(e.status, "A score needs at least one movement");
    // The PDF has every movement, numbered on.
    e.undo();
    let pages = crate::files::work_pages(&e.project, false);
    assert!(pages.len() >= 2);
}

#[test]
fn keys_reach_the_selection_outside_note_input() {
    use crate::keys::{map, Action};
    use iced::keyboard::{Key, Modifiers};
    assert_eq!(map(&Key::Character("e".into()), Modifiers::empty(), false), Some(Action::Pitch(2)));
    assert_eq!(map(&Key::Character("e".into()), Modifiers::empty(), true), Some(Action::Note { step: 2, chord: false }));
}

#[test]
fn range_selection_copy_paste_and_repeat() {
    let mut e = Editor::new(demo::project());
    let flute: Vec<Id> = events(&e, 0, 0).iter().take(2).map(|x| x.heads[0].id).collect();
    e.select(Some(flute[0]));
    e.select_elements(&[flute[1]], true);
    assert_eq!(e.range, Some(Range { lane_from: 0, lane_to: 0, from: Frac::ZERO, to: f(3, 8) }));
    assert_eq!(e.selection_label(), "Bar 1 · 1 staff");
    e.copy();
    e.repeat_selection();
    let r = e.range.unwrap();
    assert_eq!((r.from, r.to), (f(3, 8), f(3, 4)));
    let after = events(&e, 0, 0);
    let at = |t| edit::event_at(&after, t).unwrap();
    assert_eq!(pitch_name(at(f(3, 8)).heads[0].pitch), "E5");
    assert_eq!(pitch_name(at(f(5, 8)).heads[0].pitch), "D5");
    // Paste at a note in the cello.
    let cello = events(&e, 1, 0)[2].heads[0].id;
    e.select(Some(cello));
    e.paste();
    assert_eq!(e.range.unwrap().lane_from, 1);
    assert_eq!(pitch_name(edit::event_at(&events(&e, 1, 0), f(1, 1)).unwrap().heads[0].pitch), "E5");
}

#[test]
fn select_all_delete_and_articulations() {
    let mut e = Editor::new(demo::project());
    let first = events(&e, 0, 0)[0].heads[0].id;
    e.select(Some(first));
    e.toggle_articulation(art::STACCATO);
    assert!(events(&e, 0, 0)[0].articulations & art::STACCATO != 0);
    e.select_all();
    e.toggle_articulation(art::ACCENT);
    assert!(events(&e, 1, 0).iter().filter(|x| !x.is_rest()).all(|x| x.articulations & art::ACCENT != 0));
    e.toggle_articulation(art::ACCENT); // all have it: removed
    assert!(events(&e, 1, 0).iter().all(|x| x.articulations & art::ACCENT == 0));
    e.set_dynamic(Some(5));
    assert_eq!(events(&e, 0, 0)[0].dynamic, Some(5));
    e.delete_selection();
    assert!(events(&e, 0, 0).iter().all(|x| x.is_rest()));
    assert!(e.can_undo());
    for _ in 0..4 {
        e.undo();
    }
    assert_eq!(events(&e, 0, 0)[0].articulations, art::STACCATO);
}

#[test]
fn slurs_hairpins_ties_and_transpose() {
    let mut e = Editor::new(demo::project());
    let cello = events(&e, 1, 0);
    e.select(Some(cello[0].heads[0].id));
    e.add_slur();
    let ev = events(&e, 1, 0);
    let slurs = |e: &Editor| e.movement().unwrap().slurs.iter().map(|s| (s.from, s.to)).collect::<Vec<_>>();
    assert!(slurs(&e).contains(&(ev[0].id, ev[1].id)));
    assert!(e.selected_slur().is_some(), "the new slur is selected");
    e.select(Some(cello[0].heads[0].id));
    e.add_hairpin(true);
    assert!(events(&e, 1, 0)[0].spans & span::CRESC_START != 0);
    e.select(Some(cello[0].heads[0].id));
    e.toggle_tie();
    assert!(events(&e, 1, 0)[0].heads[0].tie);
    e.transpose_selection(7);
    assert_eq!(pitch_name(events(&e, 1, 0)[0].heads[0].pitch), "C4");
}

#[test]
fn popovers_and_bars() {
    let mut e = Editor::new(demo::project());
    let note = events(&e, 0, 0)[3].heads[0].id; // bar 1
    e.select(Some(note));
    assert_eq!(e.popover_position(), "Bar 1");
    assert!(e.apply_popover(PopoverKind::Key, "H").unwrap_err().starts_with("Key not understood"));
    e.apply_popover(PopoverKind::Key, "Eb").unwrap();
    assert_eq!(e.movement().unwrap().keys[0].fifths, -3);
    assert_eq!(e.status, "Key signature · bar 1");
    e.apply_popover(PopoverKind::Time, "3/4").unwrap();
    assert_eq!(e.movement().unwrap().meter[0].num, 3);
    assert_eq!(e.movement().unwrap().length, f(33, 4), "rounded up to whole 3/4 bars");
    e.undo();
    e.apply_popover(PopoverKind::Tempo, "Allegro").unwrap();
    let tempo = &e.movement().unwrap().tempo;
    assert_eq!((tempo.len(), tempo[1].at, tempo[1].bpm), (2, f(1, 2), 132.0), "at the selected note");
    e.apply_popover(PopoverKind::Expression, "espress.").unwrap();
    assert_eq!(e.movement().unwrap().lanes[0].texts.len(), 2);
    e.apply_popover(PopoverKind::Ornament, "tr").unwrap();
    assert!(events(&e, 0, 0)[3].articulations & art::TRILL != 0);
    e.insert_bars(2);
    assert_eq!(e.movement().unwrap().length, f(10, 1));
    assert_eq!(e.status, "Inserted 2 bars before bar 1");
    e.delete_bars();
    assert_eq!(e.movement().unwrap().length, f(9, 1));
    // Structure items by id: the demo's rehearsal mark.
    e.select(Some(71));
    assert_eq!(e.selection_label(), "Rehearsal mark");
    e.delete_selection();
    assert!(e.movement().unwrap().rehearsal.is_empty());
    e.clear_selection();
    e.caret = f(1, 1);
    e.add_first_second_endings();
    assert_eq!(e.movement().unwrap().endings.len(), 2);
}
