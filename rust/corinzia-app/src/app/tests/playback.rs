//! Playback, auditioning, the cycle and the MIDI keyboard.

use super::*;

#[test]
fn plays_the_score_and_follows_the_playhead() {
    let mut a = app(View::Page);
    let _ = a.update(Message::Act(Action::PlayStop));
    // (a flute speaks in about a fifth of a second)
    let level = render(&mut a, 9600);
    assert!(level > 0.001, "the flute and cello sound: {level}");
    assert!(a.playback.is_playing());
    // Andante 84: a quarter is 60/84 s; after 1.6 s (76 800 samples) the
    // playhead is 1.6 · 84/60 = 2.24 quarters in.
    render(&mut a, 67200);
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
    assert!(a.midi.input.is_some());
    let mut tries = 0;
    while a.editor.movement().unwrap().lanes[0].voices[0].events.iter().filter(|e| e.heads.len() == 3).count() == 0 || a.midi.held.len() > 0 {
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
    assert!(a.midi.input.is_none());
    std::fs::remove_file(&path).ok();
}

#[test]
fn entering_a_note_auditions_it() {
    let mut a = App::new(crate::editor::instruments::blank("T", &[("Flute", "Fl.", "treble")], 2), None, View::Page, String::new());
    render(&mut a, 1024);
    assert!(render(&mut a, 1024) < 1e-6);
    key(&mut a, "n");
    key(&mut a, "c");
    assert!(render(&mut a, 2048) > 0.005, "the new note sounds");
    assert!(!a.playback.is_playing(), "without starting playback");
}

#[test]
fn go_to_a_bar_or_a_rehearsal_mark_and_the_metronome() {
    let mut a = App::new(crate::demo::orchestra(), None, View::Page, String::new());
    a.sync();
    // Ctrl+G: the go-to popover; "17" and "B" (the Allegro) are the same bar.
    let ctrl_g = keys::map(&iced::keyboard::Key::Character("g".into()), iced::keyboard::Modifiers::CTRL, false);
    assert_eq!(ctrl_g, Some(Action::Popover(PopoverKind::GoTo)));
    let _ = a.update(Message::Act(Action::Popover(PopoverKind::GoTo)));
    type_into_popover(&mut a, "B");
    let _ = a.update(Message::Popover(PopoverMsg::Submit));
    assert!(a.score.popover.is_none(), "{:?}", a.score.popover.as_ref().and_then(|p| p.error.clone()));
    let at_b = a.editor.caret;
    assert_eq!(corinzia_edit::bar_number(a.editor.movement().unwrap(), at_b).0, 17);
    assert_eq!(a.command("goto 1").unwrap(), serde_json::Value::Null);
    assert_eq!(a.editor.caret, corinzia_score::Frac::ZERO);
    assert_eq!(a.command("goto 17").unwrap(), serde_json::Value::Null);
    assert_eq!(a.editor.caret, at_b);
    assert!(a.command("goto 99").is_err() && a.command("goto Q").is_err());
    // The metronome: Alt+M toggles it, remembered with the preferences.
    let alt_m = keys::map(&iced::keyboard::Key::Character("m".into()), iced::keyboard::Modifiers::ALT, false);
    assert_eq!(alt_m, Some(Action::Metronome));
    let _ = a.update(Message::Act(Action::Metronome));
    assert!(a.prefs.metronome);
    let _ = a.update(Message::Act(Action::Metronome));
    assert!(!a.prefs.metronome);
}

#[test]
fn scores_saved_and_opened_are_remembered() {
    let dir = std::env::temp_dir().join(format!("corinzia-recent-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut a = app(View::Page);
    let (one, two) = (dir.join("one.project"), dir.join("two.project"));
    a.save(one.clone());
    a.save(two.clone());
    a.save(one.clone());
    let canon = |p: &std::path::Path| std::fs::canonicalize(p).unwrap().to_string_lossy().into_owned();
    assert_eq!(a.prefs.recent, [canon(&one), canon(&two)], "the latest first, once each");
    let _ = a.update(Message::OpenRecent(two.clone()));
    assert_eq!(a.doc.path.as_deref(), Some(two.as_path()));
    assert_eq!(a.prefs.recent[0], canon(&two), "opened: first again");
    for k in 0..10 {
        a.prefs.remember(&dir.join(format!("{k}.project")));
    }
    assert_eq!(a.prefs.recent.len(), 8);
    std::fs::remove_dir_all(&dir).ok();
}

/// Two staves with notes starting together: (lower lane, its head, upper
/// lane, its head, the time).
fn two_staves_together(a: &App) -> (usize, u64, usize, u64, Frac) {
    let m = a.editor.movement().unwrap();
    for (li, l) in m.lanes.iter().enumerate().rev() {
        for e in l.voices[0].events.iter().filter(|e| !e.heads.is_empty()) {
            for (lj, k) in m.lanes.iter().enumerate().take(li) {
                if let Some(o) = k.voices[0].events.iter().find(|o| o.start == e.start && !o.heads.is_empty()) {
                    return (li, e.heads[0].id, lj, o.heads[0].id, e.start);
                }
            }
        }
    }
    panic!("no two staves sound together");
}

#[test]
fn shift_click_on_another_staff_selects_across_the_staves() {
    let mut a = app(View::Page);
    let (low_lane, low, high_lane, high, at) = two_staves_together(&a);
    let _ = a.update(Message::Canvas(ViewEvent::Click { hit: Some(low), at: None, shift: false }));
    assert_eq!(a.editor.selection, Some(low));
    // Shift+click the note above on the same beat: a passage over both
    // staves (and any between), from that beat.
    let _ = a.update(Message::Canvas(ViewEvent::Click { hit: Some(high), at: None, shift: true }));
    let r = a.editor.range.expect("a passage");
    assert_eq!((r.lane_from, r.lane_to, r.from), (high_lane, low_lane, at));
    assert!(r.to > at);
}

#[test]
fn play_sounds_only_the_selected_passage_and_stops_at_its_end() {
    let mut a = app(View::Page);
    a.sync();
    let (low_lane, _, high_lane, _, at) = two_staves_together(&a);
    let (mid, m) = (a.editor.movement().unwrap().id, a.editor.movement().unwrap().clone());
    let track_of = |a: &App, lane: usize| a.editor.project.instrument(m.lanes[lane].instrument_at(at).unwrap()).unwrap().track.unwrap();
    let (low_track, high_track) = (track_of(&a, low_lane), track_of(&a, high_lane));
    // The bar on the upper staff only.
    a.editor.select_bar(high_lane, at, false);
    let r = a.editor.range.unwrap();
    let _ = a.update(Message::Act(Action::PlayStop));
    assert!(a.playback.playing_only() && a.playback.until == Some((mid, quarters(r.to))));
    for _ in 0..20 {
        render(&mut a, 2400);
        a.playback.tick();
    }
    assert!(a.playback.level(high_track).0 > 0.0, "the selected staff sounds");
    assert_eq!(a.playback.level(low_track), (0.0, 0.0), "the other is silent");
    // At the passage's end it stops, and every track sounds again.
    let mut frames = 0.0;
    while a.playback.is_playing() && frames < 10.0 * a.playback.sample_rate {
        render(&mut a, 2400);
        let _ = a.update(Message::Frame);
        frames += 2400.0;
    }
    assert!(!a.playback.is_playing(), "stopped at the end of the bar: {:?} until {:?}", a.playback.playhead(), a.playback.until);
    // (a bar of 4/4 at the demo's tempo is well under ten seconds)
    assert!(!a.playback.playing_only());
    // Off in the preferences: everything plays from the passage on.
    render(&mut a, 9600); // the release
    a.prefs.play_selection_only = false;
    let _ = a.update(Message::Act(Action::PlayStop));
    render(&mut a, 256);
    assert!(a.playback.is_playing() && !a.playback.playing_only() && a.playback.until.is_none());
    let _ = a.update(Message::Act(Action::PlayStop));
    render(&mut a, 9600);
    // A single note selected: from it, everything.
    a.prefs.play_selection_only = true;
    a.editor.select(Some(m.lanes[high_lane].voices[0].events.iter().find(|e| e.start == at).unwrap().heads[0].id));
    let _ = a.update(Message::Act(Action::PlayStop));
    render(&mut a, 256);
    assert!(a.playback.is_playing() && !a.playback.playing_only());
}
