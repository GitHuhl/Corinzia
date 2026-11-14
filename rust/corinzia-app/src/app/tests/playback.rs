//! Playback, auditioning, the cycle and the MIDI keyboard.

use super::*;

#[test]
fn plays_the_score_and_follows_the_playhead() {
    let mut a = app(View::Page);
    let _ = a.update(Message::Act(Action::PlayStop));
    assert!(render(&mut a, 4800) > 0.005, "the flute and cello sound");
    assert!(a.playback.is_playing());
    // Andante 84: a quarter is 60/84 s; after 1.6 s (76 800 samples) the
    // playhead is 1.6 · 84/60 = 2.24 quarters in.
    render(&mut a, 72000);
    let (m, q) = a.playback.playhead().unwrap();
    assert_eq!(m, a.editor.project.movements[0].id);
    assert!((q - 2.24).abs() < 0.01, "playhead at {q}");
    let _ = a.update(Message::Act(Action::PlayStop));
    render(&mut a, 9600); // the 150 ms release
    assert!(!a.playback.is_playing());
    assert!(render(&mut a, 4800) < 1e-4, "silent once stopped and released");
    // Play from a selected note in bar 5.
    let id = a.editor.project.movements[0].lanes[1].voices[0].events[8].heads[0].id;
    let _ = a.update(Message::Canvas(ViewEvent::Click { hit: Some(id), at: None, shift: false }));
    let _ = a.update(Message::Act(Action::PlayStop));
    render(&mut a, 256);
    let (_, q) = a.playback.playhead().unwrap();
    assert!((q - 16.0).abs() < 0.1, "from bar 5: {q}");
}

#[test]
fn the_cycle_is_set_from_a_passage_or_the_ruler_and_toggled() {
    let mut a = app(View::Page);
    let _ = a.act(Action::Cycle);
    assert!(a.playback.cycle.is_none() && a.editor.status.contains("Select a passage"));
    // Bars 2 and 3 (4/4): quarters 4 to 12.
    let lane = 0;
    a.editor.select_bar(lane, Frac::int(1), false);
    a.editor.select_bar(lane, Frac::int(2), true);
    let _ = a.act(Action::Cycle);
    let m = a.editor.movement().unwrap().id;
    assert_eq!(a.playback.cycle, Some((m, 4.0, 12.0)));
    assert!(a.playback.cycling && a.editor.status.contains("bars 2–3"), "{}", a.editor.status);
    // Alt+L with nothing selected: off, and on again.
    a.editor.clear_selection();
    let alt_l = keys::map(&iced::keyboard::Key::Character("l".into()), iced::keyboard::Modifiers::ALT, false);
    assert_eq!(alt_l, Some(Action::Cycle));
    let _ = a.act(Action::Cycle);
    assert!(!a.playback.cycling);
    let _ = a.act(Action::Cycle);
    assert!(a.playback.cycling);
    // Shift+drag on the arrange view's ruler sets it anew.
    let _ = a.update(Message::Arrange(arrange::ArrangeEvent::Cycle { from: 2.0, to: 6.0 }));
    assert_eq!(a.playback.cycle, Some((m, 2.0, 6.0)));
}

#[test]
fn a_midi_keyboard_enters_notes_and_chords() {
    let mut a = app(View::Page);
    a.editor.select(None);
    a.editor.set_input_mode(true);
    a.editor.caret_lane = 0;
    a.editor.caret = Frac::ZERO;
    let first = a.editor.movement().unwrap().lanes[0].voices[0].events.len();
    // C E G held together (a chord), let go; then D alone.
    let path = std::env::temp_dir().join(format!("corinzia-midi-{}", std::process::id()));
    std::fs::write(&path, [0x90, 72, 100, 76, 90, 79, 80, 0x80, 72, 0, 76, 0, 79, 0, 0x90, 74, 70, 74, 0]).unwrap();
    a.open_midi(&path.to_string_lossy());
    assert!(a.midi.is_some());
    let mut tries = 0;
    while a.editor.movement().unwrap().lanes[0].voices[0].events.iter().filter(|e| e.heads.len() == 3).count() == 0 || a.midi_held.len() > 0 {
        std::thread::sleep(std::time::Duration::from_millis(10));
        a.midi_poll();
        tries += 1;
        assert!(tries < 200, "the keys arrive");
    }
    std::thread::sleep(std::time::Duration::from_millis(30));
    a.midi_poll();
    let evs = a.editor.movement().unwrap().lanes[0].voices[0].events.clone();
    let names = |e: &corinzia_model::Event| e.heads.iter().map(|h| editor::pitch_name(h.pitch)).collect::<Vec<_>>().join("+");
    assert_eq!(names(&evs[0]), "C5+E5+G5");
    assert_eq!(names(&evs[1]), "D5");
    assert!(evs.len() >= first.min(2));
    a.open_midi("");
    assert!(a.midi.is_none());
    std::fs::remove_file(&path).ok();
}

#[test]
fn entering_a_note_auditions_it() {
    let mut a = App::new(demo::blank("T", &[("Flute", "Fl.", "treble")], 2), None, View::Page, String::new());
    render(&mut a, 1024);
    assert!(render(&mut a, 1024) < 1e-6);
    key(&mut a, "n");
    key(&mut a, "c");
    assert!(render(&mut a, 2048) > 0.005, "the new note sounds");
    assert!(!a.playback.is_playing(), "without starting playback");
}
