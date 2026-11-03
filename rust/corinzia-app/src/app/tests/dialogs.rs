//! The New score, Players, Layout and Preferences cards.

use super::*;

#[test]
fn preferences_change_the_theme_and_restart_audio_at_a_new_rate() {
    let p = files::new_project("P", "Flute", 2).unwrap();
    let mut a = App::new(p, None, View::Page, String::new());
    let _ = a.update(Message::Act(Action::Preferences));
    assert!(a.prefs_dialog.is_some());
    let _ = a.update(Message::Prefs(PrefsMsg::Theme("Light".into())));
    assert_eq!(theme::palette().text, theme::LIGHT.text, "seen at once");
    let _ = a.update(Message::Prefs(PrefsMsg::Cancel));
    assert_eq!(theme::palette().text, theme::DARK.text, "cancel goes back");
    let _ = a.update(Message::Prefs(PrefsMsg::Open));
    let _ = a.update(Message::Prefs(PrefsMsg::Theme("High contrast".into())));
    let _ = a.update(Message::Prefs(PrefsMsg::Rate(96000)));
    let _ = a.update(Message::Prefs(PrefsMsg::Buffer(512)));
    let _ = a.update(Message::Prefs(PrefsMsg::Paths("/opt/a; /opt/b".into())));
    let _ = a.update(Message::Prefs(PrefsMsg::Apply));
    assert!(a.prefs_dialog.is_none());
    assert_eq!((a.prefs.sample_rate, a.prefs.buffer, a.prefs.plugin_paths.len()), (96000, 512, 2));
    assert_eq!(a.playback.sample_rate, 96000.0, "a new engine");
    assert!(a.playback.search_paths[0].ends_with("opt/a"));
    assert_eq!(a.editor.status, "Preferences saved · 96000 Hz, 512 frames");
    theme::set("Dark");
}

#[test]
fn new_score_with_key_time_pickup_tempo_any_instrument_and_an_in_app_confirmation() {
    let mut a = app(View::Page);
    // An edit makes the score "unsaved": Create asks in the window first.
    a.editor.set_input_mode(true);
    a.editor.enter_note(0, false);
    let _ = a.update(Message::Act(Action::New));
    let _ = a.update(Message::New(NewMsg::Input(NewField::Instruments, files::TEMPLATES[2].1.to_string() + ", Kazoo")));
    for (f, v) in [(NewField::Title, "Suite"), (NewField::Composer, "J. Doe"), (NewField::Key, "Eb"), (NewField::Time, "3/4 pickup 1/4"), (NewField::Tempo, "Allegro 132"), (NewField::Bars, "8")] {
        let _ = a.update(Message::New(NewMsg::Input(f, v.into())));
    }
    // Create -> Guarded -> the confirmation card (no system dialog).
    let mut msg = Some(Message::New(NewMsg::Create));
    while let Some(m) = msg.take() {
        let _ = a.update(m.clone());
        msg = match m {
            Message::New(NewMsg::Create) => Some(Message::Guarded(Box::new(Message::Proceed(Box::new(Message::New(NewMsg::Create)))))),
            _ => None,
        };
    }
    assert!(a.confirm.is_some() && a.new_score.is_some(), "asked first");
    let _ = a.update(Message::ConfirmDiscard(true));
    let _ = a.update(Message::Proceed(Box::new(Message::New(NewMsg::Create))));
    assert!(a.new_score.is_none());
    let p = &a.editor.project;
    assert_eq!((p.title.as_str(), p.composer.as_str()), ("Suite", "J. Doe"));
    assert_eq!(p.instruments.last().unwrap().name, "Kazoo");
    let m = &p.movements[0];
    assert_eq!(m.lanes.len(), 5);
    assert_eq!(m.keys[0].fifths, -3);
    assert_eq!((m.tempo[0].bpm, m.tempo[0].text.as_str()), (132.0, "Allegro"));
    assert!(m.meter[0].pickup && (m.meter[1].num, m.meter[1].den) == (3, 4));
    assert_eq!(m.length, Frac::new(1, 4) + Frac::new(3, 4) * Frac::int(8));
    assert!(!a.pages.is_empty());
}

#[test]
fn new_score_from_an_instrument_list() {
    let mut a = app(View::Page);
    let _ = a.update(Message::Act(Action::New));
    let _ = a.update(Message::New(NewMsg::Input(NewField::Instruments, "".into())));
    a.create_new();
    assert!(a.new_score.as_ref().unwrap().error.as_deref().unwrap().starts_with("Name at least one instrument"));
    let _ = a.update(Message::New(NewMsg::Input(NewField::Time, "7/5".into())));
    let _ = a.update(Message::New(NewMsg::Input(NewField::Instruments, "Flute".into())));
    a.create_new();
    assert!(a.new_score.as_ref().unwrap().error.as_deref().unwrap().starts_with("Time not understood"));
    let _ = a.update(Message::New(NewMsg::Input(NewField::Time, "4/4".into())));
    let _ = a.update(Message::New(NewMsg::Input(NewField::Instruments, "2 Violin, Viola, Cello".into())));
    let _ = a.update(Message::New(NewMsg::Input(NewField::Title, "Quartet".into())));
    let _ = a.update(Message::New(NewMsg::Input(NewField::Bars, "12".into())));
    a.create_new();
    assert!(a.new_score.is_none());
    assert_eq!(a.editor.project.title, "Quartet");
    let m = &a.editor.project.movements[0];
    assert_eq!((m.lanes.len(), m.length), (4, Frac::int(12)));
    assert!(a.editor.input_mode, "a new score starts in note input");
    assert_eq!(a.pages[0].staves.iter().filter(|s| s.system == 0).count(), 4);
}

#[test]
fn the_players_card_adds_removes_and_reorders_players() {
    let mut a = app(View::Page);
    let players = |a: &App| a.editor.project.players.iter().map(|p| a.editor.project.instrument(p.instruments[0]).unwrap().name.clone()).collect::<Vec<_>>();
    assert_eq!(players(&a), ["Flute", "Cello"]);
    let flute = a.editor.project.players[0].id;
    let pl = |m: PlayersMsg| Message::Players(m);
    let _ = a.update(pl(PlayersMsg::Open));
    assert_eq!(a.players.as_ref().unwrap().rows.len(), 2);
    // Escape and Cancel change nothing.
    let _ = a.update(pl(PlayersMsg::Cancel));
    assert!(a.players.is_none());
    let _ = a.update(pl(PlayersMsg::Open));
    // Add a piano (a grand staff) and two violins; a bad list says why.
    let _ = a.update(pl(PlayersMsg::Quick("Piano")));
    let _ = a.update(pl(PlayersMsg::AddInput(" , ".into())));
    let _ = a.update(pl(PlayersMsg::Add));
    assert!(a.players.as_ref().unwrap().error.is_some());
    let _ = a.update(pl(PlayersMsg::AddInput("2 Violin".into())));
    let _ = a.update(pl(PlayersMsg::Add));
    let d = a.players.as_ref().unwrap();
    assert!(d.error.is_none() && d.add.is_empty());
    assert_eq!(d.rows.iter().map(|r| r.instruments.as_str()).collect::<Vec<_>>(), ["Flute / Fl.", "Cello / Vc.", "Piano", "Violin 1", "Violin 2"]);
    // Remove the flute, put the piano first.
    let _ = a.update(pl(PlayersMsg::Remove(0)));
    let _ = a.update(pl(PlayersMsg::Move(1, -1)));
    let _ = a.update(pl(PlayersMsg::Move(0, -1))); // already first: nothing
    let tracks_before = a.editor.project.tracks.len();
    let _ = a.update(pl(PlayersMsg::Apply));
    assert!(a.players.is_none());
    assert_eq!(players(&a), ["Piano", "Cello", "Violin 1", "Violin 2"]);
    assert!(a.editor.project.player(flute).is_none());
    assert_eq!(a.editor.project.tracks.len(), tracks_before - 1 + 3);
    assert_eq!(a.editor.project.tracks.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["Piano", "Cello", "Violin 1", "Violin 2"], "mixer in score order");
    let m = &a.editor.project.movements[0];
    // Piano: two staves; every staff has music or rests through the movement.
    assert_eq!(m.lanes.len(), 5);
    assert_eq!(m.lanes[0].player, m.lanes[1].player);
    assert_eq!(m.lanes[1].clefs[0].clef, "bass");
    for l in &m.lanes {
        assert_eq!(corinzia_edit::voice_end(&l.voices[0].events), m.length);
    }
    assert_eq!(a.editor.status, "3 players added, 1 player removed");
    // The new instruments play: on their own MIDI channels.
    let mut channels: Vec<u8> = a.editor.project.tracks.iter().map(|t| t.midi_channel).collect();
    channels.sort();
    channels.dedup();
    assert_eq!(channels.len(), 4);
    // One undo step brings the flute back.
    a.editor.undo();
    assert_eq!(players(&a), ["Flute", "Cello"]);
    // Removing every player is not allowed.
    let _ = a.update(pl(PlayersMsg::Open));
    let _ = a.update(pl(PlayersMsg::Remove(0)));
    let _ = a.update(pl(PlayersMsg::Remove(0)));
    let _ = a.update(pl(PlayersMsg::Apply));
    assert_eq!(a.players.as_ref().unwrap().error.as_deref(), Some("A score needs at least one player"));
    assert_eq!(players(&a), ["Flute", "Cello"]);
}

#[test]
fn players_change_instrument_and_take_up_doublings() {
    let mut a = app(View::Page);
    let pl = |m: PlayersMsg| Message::Players(m);
    let flute = a.editor.project.players[0].id;
    let flute_inst = a.editor.project.players[0].instruments[0];
    let notes_before = a.editor.project.movements[0].lanes[0].voices[0].events.clone();
    let track = a.editor.project.instrument(flute_inst).unwrap().track;
    // Applying the list as it is changes nothing, not even the undo list.
    let _ = a.update(pl(PlayersMsg::Open));
    let _ = a.update(pl(PlayersMsg::Apply));
    assert_eq!(a.editor.status, "No changes to the players");
    assert!(!a.editor.can_undo());
    // The flute player becomes an oboist who doubles cor anglais.
    let _ = a.update(pl(PlayersMsg::Open));
    let _ = a.update(pl(PlayersMsg::Edit(0, "Oboe, Cor anglais".into())));
    let _ = a.update(pl(PlayersMsg::Apply));
    assert!(a.players.is_none(), "{:?}", a.players.as_ref().and_then(|d| d.error.clone()));
    let p = &a.editor.project;
    let player = p.player(flute).unwrap();
    assert_eq!(player.instruments.len(), 2);
    let oboe = p.instrument(player.instruments[0]).unwrap();
    assert_eq!((oboe.id, oboe.name.as_str(), oboe.engraver_name.as_str()), (flute_inst, "Oboe", "Ob."), "the instrument changes in place");
    assert_eq!(oboe.track, track, "keeping its track");
    assert_eq!(p.track(track.unwrap()).unwrap().name, "Oboe");
    let ca = p.instrument(player.instruments[1]).unwrap();
    assert_eq!((ca.name.as_str(), ca.transposition), ("Cor anglais", corinzia_model::Interval::new(4, 7)));
    assert!(ca.track.is_some() && ca.track != track, "a doubling sounds on its own track");
    assert!(std::sync::Arc::ptr_eq(&a.editor.project.movements[0].lanes[0].voices[0].events, &notes_before), "the music stays");
    assert_eq!(a.editor.status, "2 instruments changed");
    assert_eq!(a.editor.project.tracks.len(), 3);
    // Dropping the doubling takes its track.
    let _ = a.update(pl(PlayersMsg::Open));
    assert_eq!(a.players.as_ref().unwrap().rows[0].instruments, "Oboe / Ob., Cor anglais / C. a.");
    let _ = a.update(pl(PlayersMsg::Edit(0, "Oboe".into())));
    let _ = a.update(pl(PlayersMsg::Apply));
    assert_eq!(a.editor.project.player(flute).unwrap().instruments.len(), 1);
    assert_eq!(a.editor.project.tracks.len(), 2);
    // A new short name for the staff's later systems.
    let _ = a.update(pl(PlayersMsg::Open));
    let _ = a.update(pl(PlayersMsg::Edit(0, "Oboe / Hb.".into())));
    let _ = a.update(pl(PlayersMsg::Apply));
    assert_eq!(a.editor.project.instrument(flute_inst).unwrap().engraver_name, "Hb.");
    assert_eq!(a.editor.status, "1 instrument changed");
    // A row without an instrument is refused.
    let _ = a.update(pl(PlayersMsg::Open));
    let _ = a.update(pl(PlayersMsg::Edit(1, " ".into())));
    let _ = a.update(pl(PlayersMsg::Apply));
    assert_eq!(a.players.as_ref().unwrap().error.as_deref(), Some("Player 2: Each player needs an instrument"));
}

#[test]
fn an_instrument_change_switches_sound_clef_and_written_key() {
    let mut a = app(View::Page);
    let flute = a.editor.project.players[0].id;
    // Bar 3 of the flute: change to clarinet (added as a doubling).
    let m = a.editor.movement().unwrap();
    let bar3 = corinzia_edit::bar_starts(m)[2];
    let first_in_bar3 = m.lanes[0].voices[0].events.iter().find(|e| e.start >= bar3 && !e.is_rest()).unwrap().heads[0].id;
    a.editor.select(Some(first_in_bar3));
    a.editor.apply_popover(PopoverKind::Instrument, "clarinet").unwrap();
    let p = &a.editor.project;
    let insts = &p.player(flute).unwrap().instruments;
    assert_eq!(insts.len(), 2);
    let clarinet = p.instrument(insts[1]).unwrap();
    assert_eq!(clarinet.name, "Clarinet in B♭");
    let lane = &p.movements[0].lanes[0];
    assert_eq!(lane.changes.len(), 2);
    assert_eq!((lane.changes[1].at, lane.changes[1].instrument), (bar3, clarinet.id));
    assert_eq!(a.editor.status, "Clarinet in B♭ · bar 3");
    assert!(lane.clefs.is_empty(), "both treble: no clef change");
    // Playback: notes from bar 3 sound on the clarinet's track.
    let perf = corinzia_perform::perform(&p.movements[0]);
    let flute_notes: Vec<_> = perf.notes.iter().filter(|n| insts.contains(&n.instrument)).collect();
    assert!(flute_notes.iter().filter(|n| n.on_q < 8.0).all(|n| n.instrument == insts[0]));
    assert!(flute_notes.iter().filter(|n| n.on_q >= 8.0).all(|n| n.instrument == clarinet.id));
    assert!(flute_notes.iter().any(|n| n.on_q >= 8.0));
    // Written pitch: the clarinet's bars show its key (D major in C).
    let s = corinzia_model::notation::score_as(p, &p.movements[0], true);
    assert_eq!(s.parts[0].key_shift, 0);
    assert_eq!(s.parts[0].key_shifts, vec![(bar3, 2)]);
    a.editor.transposed = true;
    a.engrave();
    assert!(!a.pages.is_empty());
    // Back to the flute later, then to a bass-clef instrument: the clef follows.
    a.editor.select(None);
    a.editor.range = None;
    a.editor.caret_lane = 0;
    a.editor.caret = corinzia_edit::bar_starts(a.editor.movement().unwrap())[5];
    a.editor.apply_popover(PopoverKind::Instrument, "Bass clarinet").unwrap();
    let lane = &a.editor.project.movements[0].lanes[0];
    assert_eq!(lane.clefs.len(), 1);
    assert_eq!(lane.clefs[0].clef, "bass");
    // Empty text removes the change at the caret.
    a.editor.apply_popover(PopoverKind::Instrument, "").unwrap();
    assert_eq!(a.editor.project.movements[0].lanes[0].changes.len(), 2);
    assert!(a.editor.apply_popover(PopoverKind::Instrument, "").is_err());
    // One undo step each.
    a.editor.undo();
    a.editor.undo();
    a.editor.undo();
    assert_eq!(a.editor.project.player(flute).unwrap().instruments.len(), 1);
}

#[test]
fn layout_card_page_texts_and_breaks() {
    let mut a = app(View::Page);
    let lm = |m: LayoutMsg| Message::Layout(m);
    let _ = a.update(Message::Act(Action::Layout));
    assert_eq!(a.layout.as_ref().unwrap().fields[0].size, "A4");
    // Letter, landscape, a 7 mm staff, four bars a system, page texts.
    let _ = a.update(lm(LayoutMsg::Size("Letter".into())));
    let _ = a.update(lm(LayoutMsg::Landscape(true)));
    let _ = a.update(lm(LayoutMsg::Field(LayoutField::Staff, "7".into())));
    let _ = a.update(lm(LayoutMsg::Field(LayoutField::Bars, "4".into())));
    let _ = a.update(lm(LayoutMsg::Text(0, "for flute and cello".into())));
    let _ = a.update(lm(LayoutMsg::Text(3, "© 2026".into())));
    // The parts keep their own.
    let _ = a.update(lm(LayoutMsg::Tab(true)));
    assert_eq!(a.layout.as_ref().unwrap().fields[1].staff, "7");
    let _ = a.update(lm(LayoutMsg::Field(LayoutField::Spacing, "300".into())));
    let _ = a.update(lm(LayoutMsg::Apply));
    assert_eq!(a.layout.as_ref().unwrap().error.as_deref(), Some("Parts: Note spacing (%): a number from 50 to 250"));
    let _ = a.update(lm(LayoutMsg::Field(LayoutField::Spacing, "120".into())));
    let _ = a.update(lm(LayoutMsg::Apply));
    assert!(a.layout.is_none());
    let l = &a.editor.project.layout;
    assert_eq!((l.page_width_mm, l.landscape, l.staff_mm, l.bars_per_system), (215.9, true, 7.0, 4));
    assert_eq!(a.editor.project.part_layout.spacing, 1.2);
    assert_eq!(a.editor.project.subtitle, "for flute and cello");
    // The page: 279.4 mm wide in 1.75 mm spaces, printed at 4.96 pt.
    a.engrave();
    assert!((a.pages[0].width - 279.4 / 1.75).abs() < 0.01);
    assert!((a.pages[0].space_pt - 1.75 * 72.0 / 25.4).abs() < 0.01);
    let texts: Vec<String> = a.pages[0].items.iter().filter_map(|i| if let Item::Text { text, .. } = i { Some(text.clone()) } else { None }).collect();
    assert!(texts.contains(&"for flute and cello".into()) && texts.contains(&"© 2026".into()));
    // Four bars, then the next four: two systems.
    let systems = |a: &App| a.pages.iter().map(|p| p.staves.iter().map(|s| s.system).max().map_or(0, |m| m + 1)).sum::<u32>();
    assert_eq!(systems(&a), 2);
    a.editor.undo();
    assert_eq!(a.editor.project.layout, corinzia_model::Layout::default(), "one undo step");
    // A system break before bar 3 (Enter on a note there).
    let bar3 = corinzia_edit::bar_starts(a.editor.movement().unwrap())[2];
    let note = a.editor.movement().unwrap().lanes[0].voices[0].events.iter().find(|e| e.start >= bar3 && !e.is_rest()).unwrap().heads[0].id;
    a.editor.select(Some(note));
    let _ = a.update(Message::Act(Action::Break { page: false }));
    assert_eq!(a.editor.status, "System break before bar 3");
    a.engrave();
    let starts: Vec<f64> = a.pages[0].columns.iter().filter(|c| c.system == 1).map(|c| c.quarter).collect();
    assert_eq!(starts.iter().cloned().fold(f64::MAX, f64::min), 8.0, "the second system starts at bar 3");
    // Ctrl+Enter there turns it into a page break; Enter again removes it.
    let _ = a.update(Message::Act(Action::Break { page: true }));
    a.engrave();
    assert_eq!(a.pages.len(), 2);
    let _ = a.update(Message::Act(Action::Break { page: true }));
    assert_eq!(a.editor.status, "Page break removed before bar 3");
    assert!(a.editor.movement().unwrap().breaks.is_empty());
    // In a part, the break is the part's alone.
    let cello = a.editor.project.players[1].id;
    let _ = a.update(Message::SetPart(Some(cello)));
    a.editor.select(Some(note));
    let _ = a.update(Message::Act(Action::Break { page: false }));
    let b = a.editor.movement().unwrap().breaks[0];
    assert_eq!(b.part, Some(cello));
    assert!(corinzia_model::notation::score(&a.editor.project, a.editor.movement().unwrap()).breaks.is_empty());
    // Not before the first bar.
    a.editor.select(None);
    a.editor.caret = corinzia_score::Frac::ZERO;
    assert!(a.editor.toggle_break(false, None).is_err());
}
