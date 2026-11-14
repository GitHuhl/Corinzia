use super::*;
use crate::demo;
use corinzia_score::score::art;

fn f(n: i64, d: i64) -> Frac {
    Frac::new(n, d)
}
fn blank() -> Editor {
    Editor::new(demo::blank("Test", &[("Flute", "Fl.", "treble"), ("Cello", "Vc.", "bass")], 2))
}
fn events(e: &Editor, lane: usize, voice: usize) -> Vec<Event> {
    e.movement().unwrap().lanes[lane].voices[voice].events.to_vec()
}
fn notes(e: &Editor, lane: usize) -> Vec<String> {
    events(e, lane, 0).iter().map(|e| if e.is_rest() { "r".into() } else { e.heads.iter().map(|h| pitch_name(h.pitch)).collect::<Vec<_>>().join("+") }).collect()
}

#[test]
fn transposed_input_types_written_pitches() {
    let mut e = Editor::new(crate::files::new_project("T", "Clarinet", 2).unwrap());
    e.transposed = true;
    e.set_input_mode(true);
    // Concert C major is D major for the clarinet: a typed C follows the
    // written key (C#5) and sounds a major second lower (B4).
    e.enter_note(0, false);
    e.enter_note(1, false); // written D5 sounds C5
    assert_eq!(notes(&e, 0)[..2], ["B4", "C5"]);
    e.transposed = false;
    e.enter_note(0, false); // concert C, next to C5
    assert_eq!(notes(&e, 0)[2], "C5");
}

/// Durations as quarters, rests as "r": "C5:1 r:1/2".
fn rhythm(e: &Editor, lane: usize) -> Vec<String> {
    events(e, lane, 0)
        .iter()
        .map(|ev| {
            let q = ev.duration * Frac::int(4);
            let name = if ev.is_rest() { "r".to_string() } else { pitch_name(ev.heads[0].pitch) };
            if q.den == 1 { format!("{name}:{}", q.num) } else { format!("{name}:{}/{}", q.num, q.den) }
        })
        .collect()
}

fn quarters_cdef(e: &mut Editor) {
    e.set_input_mode(true);
    for step in [0, 1, 2, 3] {
        e.enter_note(step, false);
    }
    e.set_input_mode(false);
}

fn select_nth(e: &mut Editor, n: usize) {
    let ev = &events(e, 0, 0)[n];
    let id = ev.heads.first().map(|h| h.id).unwrap_or(ev.id);
    e.select(Some(id));
}

#[test]
fn changing_a_selected_notes_duration() {
    let mut e = blank();
    quarters_cdef(&mut e);
    assert_eq!(rhythm(&e, 0)[..4], ["C5:1", "D5:1", "E5:1", "F5:1"]);
    // Shorter: D becomes an eighth and an eighth rest takes the rest of its beat.
    select_nth(&mut e, 1);
    e.spec.base = edit::BaseDuration::Eighth;
    e.set_selection_duration();
    assert_eq!(rhythm(&e, 0)[..5], ["C5:1", "D5:1/2", "r:1/2", "E5:1", "F5:1"]);
    assert_eq!(e.event_of(e.selection.unwrap()).unwrap().1.duration, f(1, 8), "D stays selected");
    // Longer: C becomes a half note and overwrites D and the rest.
    select_nth(&mut e, 0);
    e.spec.base = edit::BaseDuration::Half;
    e.set_selection_duration();
    assert_eq!(rhythm(&e, 0)[..3], ["C5:2", "E5:1", "F5:1"]);
    // Longer into the middle of a note: E's tail stays as a rest.
    select_nth(&mut e, 0);
    e.spec.base = edit::BaseDuration::Half;
    e.cycle_selection_dots(); // a dotted half: overwrites E, half of nothing else
    assert_eq!(rhythm(&e, 0)[..2], ["C5:3", "F5:1"]);
    // Dots cycle: dotted → double-dotted cuts into F, then back to plain.
    e.cycle_selection_dots();
    assert_eq!(rhythm(&e, 0)[..2], ["C5:7/2", "r:1/2"]);
    e.cycle_selection_dots();
    // The space left is one rest per bar (drawn as the meter wants).
    assert_eq!(rhythm(&e, 0)[..3], ["C5:2", "r:2", "r:4"]);
    // Undo walks back each step.
    e.undo();
    assert_eq!(rhythm(&e, 0)[..2], ["C5:7/2", "r:1/2"]);
    // The total stays two bars.
    let total: Frac = events(&e, 0, 0).iter().fold(Frac::ZERO, |a, ev| a + ev.duration);
    assert_eq!(total, Frac::int(2));
}

#[test]
fn changing_durations_of_a_passage_and_at_the_end() {
    let mut e = blank();
    quarters_cdef(&mut e);
    // A passage of four quarters becomes four eighths and a half rest.
    e.select(None);
    e.set_range(Range { lane_from: 0, lane_to: 0, from: Frac::ZERO, to: Frac::int(1) });
    e.spec.base = edit::BaseDuration::Eighth;
    e.set_selection_duration();
    assert_eq!(rhythm(&e, 0)[..5], ["C5:1/2", "D5:1/2", "E5:1/2", "F5:1/2", "r:2"]);
    assert_eq!(e.range.unwrap().to, f(1, 2), "the passage follows the new rhythm");
    // A note on the last beat made a whole: the movement grows by a bar.
    e.set_input_mode(true);
    e.caret = f(7, 4);
    e.spec.base = edit::BaseDuration::Quarter;
    e.enter_note(4, false);
    e.set_input_mode(false);
    let n = events(&e, 0, 0).len();
    select_nth(&mut e, n - 1);
    e.spec.base = edit::BaseDuration::Whole;
    e.set_selection_duration();
    assert_eq!(e.movement().unwrap().length, Frac::int(3));
    assert_eq!(rhythm(&e, 0).last().unwrap(), "G5:4");
    // In voice 1 the rests between stay one per bar.
    assert_eq!(rhythm(&e, 0)[4..6], ["r:2".to_string(), "r:3".to_string()]);
}

#[test]
fn letters_and_accidentals_change_the_selected_note() {
    let mut e = blank();
    quarters_cdef(&mut e);
    select_nth(&mut e, 1); // D5
    e.repitch_selection(6); // B nearest D5 is B4
    assert_eq!(rhythm(&e, 0)[1], "B4:1");
    e.set_selection_accidental(-1);
    assert_eq!(rhythm(&e, 0)[1], "B♭4:1");
    e.set_selection_accidental(-1); // again: back to the key's
    assert_eq!(rhythm(&e, 0)[1], "B4:1");
    // A rest becomes a note.
    let n = events(&e, 0, 0).len();
    select_nth(&mut e, n - 1);
    assert!(events(&e, 0, 0)[n - 1].is_rest());
    e.repitch_selection(4);
    assert!(!events(&e, 0, 0)[n - 1].is_rest());
    assert_eq!(e.event_of(e.selection.unwrap()).unwrap().1.heads.len(), 1);
}

#[test]
fn alt_arrows_move_music_by_the_rhythmic_grid() {
    let mut e = blank();
    quarters_cdef(&mut e);
    assert_eq!(e.grid_value(), f(1, 8), "an eighth grid by default");
    // D moves an eighth later: an eighth rest opens before it, E loses its first half.
    select_nth(&mut e, 1);
    let id = e.selection.unwrap();
    e.nudge(1);
    assert_eq!(rhythm(&e, 0)[..5], ["C5:1", "r:1/2", "D5:1", "r:1/2", "F5:1"]);
    assert_eq!(e.selection, Some(id), "the note stays selected");
    assert_eq!(e.event_of(id).unwrap().1.start, f(3, 8));
    // And back: the rests join up again around it.
    e.nudge(-1);
    assert_eq!(rhythm(&e, 0)[..4], ["C5:1", "D5:1", "r:1", "F5:1"]);
    // Not before the start.
    select_nth(&mut e, 0);
    e.nudge(-1);
    assert_eq!(rhythm(&e, 0)[0], "C5:1");
    assert_eq!(e.status, "Nothing to move there");
    // A passage moves by a quarter grid.
    e.grid = 0;
    e.set_range(Range { lane_from: 0, lane_to: 0, from: Frac::ZERO, to: f(1, 2) });
    e.nudge(1);
    assert_eq!(rhythm(&e, 0)[..4], ["r:1", "C5:1", "D5:1", "F5:1"]);
    assert_eq!(e.range.map(|r| (r.from, r.to)), Some((f(1, 4), f(3, 4))));
    // The caret steps by the grid; Alt+] makes it finer.
    e.step_grid(1);
    e.caret = Frac::ZERO;
    e.move_caret(1);
    assert_eq!(e.caret, f(1, 8));
}

fn tuplets(e: &Editor) -> Vec<Tuplet> {
    e.movement().unwrap().lanes[0].voices[0].tuplets.clone()
}

#[test]
fn tuplet_input_fills_tuplets_one_after_another() {
    let mut e = blank();
    e.set_input_mode(true);
    e.spec.base = edit::BaseDuration::Eighth;
    e.apply_popover(PopoverKind::Tuplet, "3").unwrap();
    for step in [0, 1, 2, 3, 4, 5] {
        e.enter_note(step, false);
    }
    // Two triplets of eighths, one per beat.
    let ts = tuplets(&e);
    assert_eq!(ts.len(), 2);
    assert_eq!((ts[0].start, ts[0].length, ts[1].start), (Frac::ZERO, f(1, 4), f(1, 4)));
    assert_eq!(rhythm(&e, 0)[..6].iter().map(|r| r.split(':').nth(1).unwrap()).collect::<Vec<_>>(), ["1/3"; 6]);
    assert_eq!(e.caret, f(1, 2));
    // ':' ends tuplet input: the next eighth is plain.
    e.tuplet_input = None;
    e.enter_note(6, false);
    assert_eq!(rhythm(&e, 0)[6], "B5:1/2");
    assert_eq!(tuplets(&e).len(), 2);
    // A quintuplet of sixteenths over the next beat.
    e.caret = f(3, 4);
    e.spec.base = edit::BaseDuration::Sixteenth;
    e.apply_popover(PopoverKind::Tuplet, "5").unwrap();
    for step in [0, 1, 2, 3, 4] {
        e.enter_note(step, false);
    }
    let q = tuplets(&e).into_iter().find(|t| t.actual == 5).unwrap();
    assert_eq!((q.start, q.length, q.normal), (f(3, 4), f(1, 4), 4));
    assert_eq!(e.caret, f(1, 1));
}

#[test]
fn a_selected_note_becomes_a_tuplet_and_edits_keep_it_sound() {
    let mut e = blank();
    quarters_cdef(&mut e);
    // D (a quarter) starts a quarter-note triplet over two beats.
    select_nth(&mut e, 1);
    e.apply_popover(PopoverKind::Tuplet, "3").unwrap();
    assert_eq!(tuplets(&e), vec![Tuplet { start: f(1, 4), length: f(1, 2), actual: 3, normal: 2 }]);
    assert_eq!(rhythm(&e, 0)[..4], ["C5:1", "D5:2/3", "r:4/3", "F5:1"]);
    // Duration keys inside it are written values: an eighth is 1/12.
    e.spec.base = edit::BaseDuration::Eighth;
    e.set_selection_duration();
    assert_eq!(rhythm(&e, 0)[1], "D5:1/3");
    assert_eq!(tuplets(&e).len(), 1, "still inside");
    // A note now crossing its end removes it.
    e.spec.base = edit::BaseDuration::Whole;
    e.set_selection_duration();
    assert!(tuplets(&e).is_empty());
    e.undo();
    e.undo();
    assert_eq!(tuplets(&e).len(), 1);
    // Copy the triplet's beats, paste them on beat 1 of bar 2: it comes along.
    e.set_range(Range { lane_from: 0, lane_to: 0, from: f(1, 4), to: f(3, 4) });
    e.copy();
    e.set_range(Range { lane_from: 0, lane_to: 0, from: f(1, 1), to: f(1, 1) });
    e.paste();
    assert_eq!(tuplets(&e).iter().map(|t| t.start).collect::<Vec<_>>(), [f(1, 4), f(1, 1)]);
    // Deleting a passage holding it removes it; inserting a bar before moves it.
    e.set_range(Range { lane_from: 0, lane_to: 0, from: f(1, 1), to: f(3, 2) });
    e.delete_selection();
    assert_eq!(tuplets(&e).len(), 1);
    e.clear_selection();
    e.caret = Frac::ZERO;
    e.insert_bars(1);
    assert_eq!(tuplets(&e)[0].start, f(5, 4));
}

#[test]
fn grace_notes_are_typed_before_their_note() {
    let mut e = blank();
    e.set_input_mode(true);
    e.enter_note(0, false); // C5
    e.spec.base = edit::BaseDuration::Sixteenth;
    e.toggle_grace_input(true);
    e.enter_note(4, false); // grace G
    e.enter_note(5, false); // grace A
    assert_eq!(e.caret, f(1, 4), "graces leave the caret where it is");
    e.toggle_grace_input(true);
    e.spec.base = edit::BaseDuration::Quarter;
    e.enter_note(6, false); // the main note B, after its graces
    let ev = &events(&e, 0, 0)[1];
    assert_eq!(pitch_name(ev.heads[0].pitch), "B4");
    let g: Vec<String> = ev.graces.iter().map(|g| pitch_name(g.heads[0].pitch)).collect();
    assert_eq!(g, ["G4", "A4"]);
    assert!(ev.graces.iter().all(|g| g.slash && g.base == 16));
    // A grace selected transposes alone; Backspace on the caret removes the last grace.
    let a = ev.graces[1].heads[0].id;
    e.set_input_mode(false);
    e.select(Some(a));
    e.transpose_selection(1);
    assert_eq!(pitch_name(events(&e, 0, 0)[1].graces[1].heads[0].pitch), "B4");
    e.set_input_mode(true);
    e.caret = f(1, 4);
    e.backspace();
    assert_eq!(events(&e, 0, 0)[1].graces.len(), 1);
    // Delete with the remaining grace selected removes it, not its note.
    e.set_input_mode(false);
    let g = events(&e, 0, 0)[1].graces[0].id;
    e.select(Some(g));
    e.delete_selection();
    assert!(events(&e, 0, 0)[1].graces.is_empty());
    assert!(!events(&e, 0, 0)[1].is_rest());
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
fn arrow_keys_walk_notes_bars_staves_and_passages() {
    let mut e = blank();
    e.set_input_mode(true);
    for step in [0, 1, 2, 3, 4, 5, 6, 0] {
        e.enter_note(step, false); // two bars of quarters: C D E F | G A B C
    }
    e.set_input_mode(false);
    e.clear_selection();
    e.caret = Frac::ZERO;
    let name = |e: &Editor| {
        let id = e.selection.unwrap();
        let (_, ev) = e.event_of(id).unwrap();
        pitch_name(ev.heads[0].pitch)
    };
    e.select_step(1, false); // nothing selected: the note at the caret
    assert_eq!(name(&e), "C5");
    e.select_step(1, false);
    e.select_step(1, false);
    assert_eq!(name(&e), "E5");
    e.select_step(-1, false);
    assert_eq!(name(&e), "D5");
    e.select_step(1, true); // next bar's first note
    assert_eq!(name(&e), "G5");
    e.select_step(-1, true); // back to this bar's start... already there: the previous bar's
    assert_eq!(name(&e), "C5");
    e.select_edge(true);
    assert_eq!(e.event_of(e.selection.unwrap()).unwrap().1.start, events(&e, 0, 0).last().unwrap().start);
    e.select_edge(false);
    assert_eq!(name(&e), "C5");
    // Shift+Right grows a passage by notes, Shift+Left shrinks it.
    e.extend_step(1);
    e.extend_step(1);
    let r = e.range.unwrap();
    assert_eq!((r.from, r.to), (Frac::ZERO, f(3, 4)));
    e.extend_step(-1);
    assert_eq!(e.range.unwrap().to, f(1, 2));
    // Right collapses the passage to its end.
    e.select_step(1, false);
    assert!(e.range.is_none() && e.selection.is_some());
    // Alt+Down: the cello's event at the same time (its whole-bar rest);
    // Alt+Up from there: the flute's note where that rest starts.
    e.select(Some(events(&e, 0, 0)[1].heads[0].id));
    e.select_staff(1);
    assert_eq!(e.locate(e.selection.unwrap()).unwrap().lane, 1);
    e.select_staff(-1);
    assert_eq!(name(&e), "C5");
}

#[test]
fn typing_notes_follows_the_caret_and_octaves() {
    let mut e = blank();
    e.set_input_mode(true);
    for step in [0, 4, 2, 0] {
        e.enter_note(step, false); // C (near B4 -> C5), G, E, C
    }
    assert_eq!(notes(&e, 0)[..4], ["C5", "G4", "E4", "C4"]);
    assert_eq!(e.caret, f(1, 1));
    assert_eq!(e.caret_label(), "Bar 2, beat 1 · Flute · voice 1");
    // Voice 1 stays contiguous: the rest of the piece is still rests.
    let ev = events(&e, 0, 0);
    assert_eq!(ev.last().unwrap().end(), f(2, 1));
    assert!(ev.windows(2).all(|w| w[0].end() == w[1].start));
    // One undo step per note.
    e.undo();
    assert_eq!(notes(&e, 0)[3], "r");
    e.redo();
    assert_eq!(notes(&e, 0)[3], "C4");
}

#[test]
fn chords_accidentals_rests_and_durations() {
    let mut e = blank();
    e.set_input_mode(true);
    e.spec.base = edit::BaseDuration::Half;
    e.enter_note(0, false); // C5
    e.enter_note(2, true); // E5 above
    e.toggle_accidental(1);
    e.enter_note(4, true); // G#5 above
    assert_eq!(notes(&e, 0)[0], "C5+E5+G♯5");
    assert_eq!(e.caret, f(1, 2));
    assert_eq!(e.accidental, None, "an accidental lasts one note");
    e.cycle_dots();
    e.spec.base = edit::BaseDuration::Quarter;
    e.enter_rest(); // dotted quarter rest
    assert_eq!(e.caret, f(7, 8));
    e.backspace();
    assert_eq!(e.caret, f(1, 2), "backspace steps over a voice-1 rest");
}

#[test]
fn voice_two_is_created_with_holes() {
    let mut e = blank();
    e.set_input_mode(true);
    e.voice = 2;
    e.caret = f(1, 2);
    e.enter_note(6, false);
    let m = e.movement().unwrap();
    assert_eq!(m.lanes[0].voices.len(), 2);
    assert_eq!(m.lanes[0].voices[1].index, 2);
    assert_eq!(m.lanes[0].voices[1].events.len(), 1, "no filler rests in voice 2");
}

#[test]
fn entering_past_the_end_adds_bars() {
    let mut e = blank();
    e.set_input_mode(true);
    e.caret = f(7, 4);
    e.spec.base = edit::BaseDuration::Half;
    e.enter_note(0, false);
    assert_eq!(e.movement().unwrap().length, f(3, 1));
}

#[test]
fn mouse_input_snaps_to_the_grid() {
    let mut e = blank();
    e.spec.base = edit::BaseDuration::Eighth;
    // Quarter 1.4 -> eighth grid 2.8 + 0.35 -> 3rd eighth = beat 2.5 (3/8)
    e.place_note_at(1, 1.4, 19, false); // A2 on the cello (diatonic 5 + 7*2)
    let ev = events(&e, 1, 0);
    let n = ev.iter().find(|x| !x.is_rest()).unwrap();
    assert_eq!(n.start, f(3, 8));
    assert_eq!(pitch_name(n.heads[0].pitch), "A2");
    e.place_note_at(1, 1.45, 21, true); // shift: add C3 to it
    assert_eq!(events(&e, 1, 0).iter().find(|x| !x.is_rest()).unwrap().heads.len(), 2);
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
fn slurs_nest_overlap_and_follow_their_notes() {
    let mut e = blank();
    quarters_cdef(&mut e);
    let n = events(&e, 0, 0);
    let slurs = |e: &Editor| {
        let n = events(e, 0, 0);
        let at = |id| n.iter().position(|x| x.id == id).unwrap();
        let mut v: Vec<(usize, usize)> = e.movement().unwrap().slurs.iter().map(|s| (at(s.from), at(s.to))).collect();
        v.sort();
        v
    };
    // A phrase over all four, a slur inside it, one overlapping its end.
    e.set_range(Range { lane_from: 0, lane_to: 0, from: Frac::ZERO, to: f(1, 1) });
    e.add_slur();
    e.set_range(Range { lane_from: 0, lane_to: 0, from: f(1, 4), to: f(3, 4) });
    e.add_slur();
    assert_eq!(slurs(&e), [(0, 3), (1, 2)], "nested");
    // Moving the inner slur's end past the phrase's end overlaps it.
    let inner = e.movement().unwrap().slurs.iter().find(|s| s.from == n[1].id).unwrap().id;
    assert!(e.select_slur(inner));
    e.step_slur(SlurHandle::End, 1);
    assert_eq!(slurs(&e), [(0, 3), (1, 3)]);
    // A start can't pass the end.
    e.step_slur(SlurHandle::Start, 5);
    assert_eq!(slurs(&e), [(0, 3), (2, 3)]);
    // S on the same notes again takes that slur away.
    e.set_range(Range { lane_from: 0, lane_to: 0, from: f(2, 4), to: f(1, 1) });
    e.add_slur();
    assert_eq!(slurs(&e), [(0, 3)]);
    // Copy and paste carries the slur; deleting an end note drops it.
    e.set_range(Range { lane_from: 0, lane_to: 0, from: Frac::ZERO, to: f(1, 1) });
    e.copy();
    e.set_range(Range { lane_from: 0, lane_to: 0, from: f(1, 1), to: f(2, 1) });
    e.paste();
    assert_eq!(e.movement().unwrap().slurs.len(), 2);
    let last = events(&e, 0, 0).iter().filter(|x| !x.is_rest()).last().unwrap().heads[0].id;
    e.select(Some(last));
    e.delete_selection();
    assert_eq!(e.movement().unwrap().slurs.len(), 1, "a slur goes with its note");
    e.undo();
    assert_eq!(e.movement().unwrap().slurs.len(), 2);
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

#[test]
fn drum_sets_get_a_map_and_channel_ten() {
    let e = Editor::new(crate::files::new_project("D", "Flute, Drum set", 2).unwrap());
    let drums = e.project.instruments.iter().find(|i| i.name == "Drum set").unwrap().clone();
    assert!(!drums.drums.is_empty());
    let track = e.project.tracks.iter().find(|t| Some(t.id) == drums.track).unwrap();
    assert_eq!(track.midi_channel, 9);
    assert!(e.project.tracks.iter().filter(|t| Some(t.id) != drums.track).all(|t| t.midi_channel != 9));
    assert!(e.lane_is_percussion(1) && !e.lane_is_percussion(0));
    // The snare sits on C5 and sounds key 38.
    let snare = Pitch { step: 0, alter: 0, octave: 5 };
    assert_eq!(e.drum_name(1, Frac::ZERO, snare).as_deref(), Some("Snare drum"));
    assert_eq!(e.project.sounding_key(drums.id, snare.midi_key()), 38);
}

#[test]
fn cues_and_part_only_texts() {
    let mut e = blank();
    quarters_cdef(&mut e);
    e.set_range(Range { lane_from: 0, lane_to: 0, from: Frac::ZERO, to: f(1, 1) });
    e.copy();
    e.select(None);
    e.set_range(Range { lane_from: 1, lane_to: 1, from: f(1, 1), to: f(2, 1) });
    e.paste_cue();
    let l = &e.movement().unwrap().lanes[1];
    let cue = l.voices.iter().find(|v| v.index == 2).unwrap();
    let notes: Vec<&Event> = cue.events.iter().filter(|x| !x.is_rest()).collect();
    assert_eq!(notes.len(), 4);
    assert!(notes.iter().all(|x| x.articulations & art::CUE != 0 && x.start >= f(1, 1)));
    assert_eq!((l.texts[0].text.as_str(), l.texts[0].at), ("Flute", f(1, 1)));
    assert_eq!(e.status, "Cue from Flute");
    // Cues are silent.
    assert_eq!(corinzia_perform::perform(e.movement().unwrap()).notes.len(), 4, "the flute's four notes only");
    // A text for the parts only: not in the score, in the part.
    e.set_range(Range { lane_from: 0, lane_to: 0, from: Frac::ZERO, to: f(1, 4) });
    e.apply_popover(PopoverKind::Expression, "parts: solo").unwrap();
    let t = e.movement().unwrap().lanes[0].texts.iter().find(|t| t.text == "solo").unwrap().clone();
    assert_eq!(t.shown, corinzia_model::Shown::Parts);
    assert_eq!(edit::shown_text(&t), "parts: solo");
    let m = e.movement().unwrap();
    let has = |s: &corinzia_score::score::Score| s.parts.iter().any(|p| p.texts.iter().any(|x| x.text == "solo"));
    assert!(!has(&corinzia_model::notation::score(&e.project, m)));
    assert!(has(&corinzia_model::notation::part_score(&e.project, m, m.lanes[0].player)));
}

#[test]
fn cross_staff_notes() {
    let mut e = Editor::new(crate::files::new_project("P", "Piano", 2).unwrap());
    assert_eq!(e.movement().unwrap().lanes.len(), 2, "a grand staff");
    quarters_cdef(&mut e);
    select_nth(&mut e, 1);
    e.cross_staff(1);
    let id = events(&e, 0, 0)[1].id;
    assert_eq!(e.movement().unwrap().cross, [(id, 1)]);
    assert_eq!(e.status, "Cross staff: 1 note");
    let s = corinzia_model::notation::score(&e.project, e.movement().unwrap());
    assert_eq!(s.parts[0].cross, [(id, 1)]);
    // No staff below the bass staff; back up goes home.
    e.cross_staff(1);
    assert_eq!(e.movement().unwrap().cross, [(id, 1)]);
    assert_eq!(e.status, "No staff of this player there");
    e.cross_staff(-1);
    assert!(e.movement().unwrap().cross.is_empty());
}

#[test]
fn guitar_tablature() {
    let mut e = Editor::new(crate::files::new_project("G", "Electric guitar, Guitar, Bass guitar", 1).unwrap());
    let tunings: Vec<usize> = e.project.instruments.iter().map(|i| i.tuning.len()).collect();
    assert_eq!(tunings, [6, 0, 4]);
    e.caret_lane = 1;
    e.toggle_tab();
    assert_eq!(e.project.instruments[1].tuning, GUITAR_TUNING);
    assert_eq!(e.status, "Tablature on for Guitar");
    let s = corinzia_model::notation::score(&e.project, e.movement().unwrap());
    assert_eq!(s.parts[1].tab, GUITAR_TUNING);
    e.undo();
    assert!(e.project.instruments[1].tuning.is_empty());
}

#[test]
fn musicxml_round_trip_keeps_the_newer_notation() {
    let mut p = crate::files::new_project("RT", "Flute, Piano, Drum set, Electric guitar", 4).unwrap();
    p.subtitle = "for four".into();
    p.lyricist = "Words: A".into();
    p.arranger = "arr. B".into();
    p.copyright = "© C".into();
    let mut e = Editor::new(p);
    let fill = |e: &mut Editor, lane: usize, from: Frac, steps: &[i32], eighths: bool| {
        e.select(None);
        e.set_input_mode(true);
        e.caret_lane = lane;
        e.caret = from;
        e.spec.base = if eighths { edit::BaseDuration::Eighth } else { edit::BaseDuration::Quarter };
        for &s in steps {
            e.enter_note(s, false);
        }
        e.set_input_mode(false);
    };
    // Flute: eighths in bar 1 (a beam broken by hand, one flagged), quarters in bar 3.
    fill(&mut e, 0, Frac::ZERO, &[0, 1, 2, 3, 4, 3, 2, 1], true);
    fill(&mut e, 0, f(2, 1), &[4, 4, 4, 4], false);
    // Piano: quarters in bar 1, the second written on the left hand's staff.
    fill(&mut e, 1, Frac::ZERO, &[0, 1, 2, 3], false);
    // Drums: snare (C5) and kick (F4); guitar: a cue of the flute in bar 2.
    fill(&mut e, 3, Frac::ZERO, &[0, 3], false);
    fill(&mut e, 4, f(1, 1), &[4, 5, 6, 0], false);
    let piccolo_at = f(2, 1);
    {
        let p = &mut e.project;
        let flute_player = p.players[0].id;
        let picc = p.allocate_id();
        let track = p.instruments[0].track;
        p.instruments.push(Instrument { id: picc, player: flute_player, name: "Piccolo".into(), engraver_name: "Picc.".into(), transposition: Default::default(), clef: "treble".into(), track, drums: vec![], tuning: vec![] });
        p.players[0].instruments.push(picc);
        let m = &mut p.movements[0];
        m.lanes[0].changes.push(InstrumentChange { id: 9001, at: piccolo_at, instrument: picc });
        let mut flute = (*m.lanes[0].voices[0].events).clone();
        flute[2].spans |= span::BEAM_BREAK;
        flute[7].spans |= span::NO_BEAM;
        m.lanes[0].voices[0].events = Arc::new(flute);
        let second = m.lanes[1].voices[0].events[1].id;
        m.cross.push((second, 1));
        let mut gtr = (*m.lanes[4].voices[0].events).clone();
        for ev in gtr.iter_mut().filter(|x| !x.is_rest()) {
            ev.articulations |= art::CUE;
        }
        m.lanes[4].voices[0].events = Arc::new(gtr);
        m.breaks = vec![Break { id: 9002, at: f(1, 1), page: false, part: None }, Break { id: 9003, at: f(2, 1), page: true, part: None }];
        m.meter.push(Meter { at: f(3, 1), num: 7, den: 8, pickup: false, beams: [2, 2, 3, 0, 0, 0, 0, 0] });
    }
    let p = &e.project;
    let xml = corinzia_perform::musicxml(p, &p.movements[0]).unwrap();
    assert!(xml.contains("<staves>2</staves>"), "the piano is one part");
    let q = corinzia_import::musicxml(xml.as_bytes()).unwrap();
    let (a, b) = (&p.movements[0], &q.movements[0]);
    assert_eq!((q.subtitle.as_str(), q.lyricist.as_str(), q.arranger.as_str(), q.copyright.as_str()), ("for four", "Words: A", "arr. B", "© C"));
    assert_eq!(q.players.len(), 4, "{:?}", q.players.iter().map(|p| &p.name).collect::<Vec<_>>());
    assert_eq!(b.lanes.len(), 5);
    let clef_of = |m: &Movement, li: usize| m.lanes[li].clefs.iter().find(|c| c.at.is_zero()).map(|c| c.clef.clone()).unwrap_or_default();
    assert_eq!(clef_of(b, 2), "bass", "the left hand");
    // Beams by hand.
    let spans = |m: &Movement| m.lanes[0].voices[0].events.iter().map(|x| x.spans & span::BEAMS).collect::<Vec<_>>();
    assert_eq!((spans(a)[2], spans(a)[7]), (span::BEAM_BREAK, span::NO_BEAM));
    assert_eq!(spans(b)[..8], spans(a)[..8]);
    if let Ok(path) = std::env::var("RT_XML") {
        std::fs::write(path, &xml).unwrap();
    }
    // The doubling.
    let change = b.lanes[0].changes.iter().find(|c| c.at == piccolo_at).expect("a change to the piccolo");
    assert_eq!(q.instrument(change.instrument).unwrap().name, "Piccolo");
    assert!(!b.lanes[0].texts.iter().any(|t| t.text == "Piccolo"), "the change is not also a text");
    // Cross-staff.
    let second = b.lanes[1].voices[0].events[1].id;
    assert_eq!(b.cross, [(second, 1)]);
    // Drums: their map and noteheads.
    let kit = q.instrument(b.lanes[3].changes[0].instrument).unwrap();
    let snare = kit.drums.iter().find(|d| d.position == Pitch { step: 0, alter: 0, octave: 5 }).unwrap();
    assert_eq!((snare.name.as_str(), snare.key), ("Snare drum", 38));
    assert_eq!(kit.clef, "percussion");
    // Cue notes, the guitar's tuning, breaks and the grouped meter.
    assert!(b.lanes[4].voices[0].events.iter().filter(|x| !x.is_rest()).all(|x| x.articulations & art::CUE != 0));
    assert_eq!(b.lanes[4].voices[0].events.iter().filter(|x| !x.is_rest()).count(), 4);
    assert_eq!(q.instrument(b.lanes[4].changes[0].instrument).unwrap().tuning, GUITAR_TUNING);
    assert_eq!(b.breaks.iter().map(|x| (x.at, x.page)).collect::<Vec<_>>(), [(f(1, 1), false), (f(2, 1), true)]);
    assert_eq!(b.meter.iter().find(|m| m.at == f(3, 1)).map(|m| m.beams), Some([2, 2, 3, 0, 0, 0, 0, 0]));
}
