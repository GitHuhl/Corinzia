//! The Mix tab: effects, buses, presets, clips, meters, shared instruments, the piano roll, automation and expression maps.

use super::*;

#[test]
fn effects_buses_and_sends_from_the_mix_tab() {
    let mut a = app(View::Page);
    let _ = a.update(Message::Act(Action::ShowMix));
    let mx = |m: mix::MixMsg| Message::Mix(m);
    let flute = a.editor.project.tracks[0].id;
    let index = |a: &App, id: &str| a.playback.plugins.iter().position(|p| p.id == id).unwrap();
    // A gain insert on the flute, its level from the parameter list.
    let gain = index(&a, "org.corinzia.gain");
    let _ = a.update(mx(mix::MixMsg::AddEffect(flute, gain)));
    let node = a.mix.fx.expect("the new effect is selected");
    assert_eq!(a.editor.project.tracks[0].inserts, [node]);
    a.sync();
    assert!(a.playback.pool.effect(node).is_some(), "loaded with the new plan");
    let _ = a.update(mx(mix::MixMsg::Param(node, 0, -60.0)));
    assert_eq!(a.playback.pool.effect(node).unwrap().param_value(0), Some(-60.0));
    // A bus with a reverb; the flute sends to it.
    let _ = a.update(mx(mix::MixMsg::AddBus));
    let bus = a.editor.project.buses[0].id;
    assert_eq!((a.mix.strip, a.editor.project.buses[0].name.as_str()), (Some(bus), "Bus 1"));
    let reverb = index(&a, "org.corinzia.reverb");
    let _ = a.update(mx(mix::MixMsg::AddEffect(bus, reverb)));
    let _ = a.update(mx(mix::MixMsg::ToggleSend(flute, bus)));
    let _ = a.update(mx(mix::MixMsg::SendLevel(flute, 0, -3.0)));
    let _ = a.update(mx(mix::MixMsg::Released));
    assert_eq!(a.editor.project.tracks[0].sends, [corinzia_model::SendLevel { bus, gain_db: -3.0, pre: false }]);
    // The cello into the bus; the master down 6 dB.
    let cello = a.editor.project.tracks[1].id;
    let _ = a.update(mx(mix::MixMsg::Output(cello, "Bus 1".into())));
    assert_eq!(a.editor.project.tracks[1].output_bus, Some(bus));
    let _ = a.update(mx(mix::MixMsg::Gain(editor::MASTER, -6.0)));
    assert_eq!(a.editor.project.master.gain_db, -6.0);
    a.sync();
    let _ = a.update(Message::Act(Action::PlayStop));
    assert!(render(&mut a, 9600) > 0.0003, "heard through the bus");
    // Bypass and removal; one undo brings the bus back.
    let _ = a.update(mx(mix::MixMsg::Bypass(node)));
    assert!(a.editor.project.node(node).unwrap().bypassed);
    let _ = a.update(mx(mix::MixMsg::RemoveBus(bus)));
    assert!(a.editor.project.buses.is_empty() && a.editor.project.tracks[0].sends.is_empty() && a.editor.project.tracks[1].output_bus.is_none());
    a.editor.undo();
    assert_eq!(a.editor.project.buses.len(), 1);
    a.sync();
}

#[test]
fn presets_and_programs_from_the_mix_tab() {
    let mut a = app(View::Page);
    let _ = a.update(Message::Act(Action::ShowMix));
    let mx = |m: mix::MixMsg| Message::Mix(m);
    let flute = a.editor.project.tracks[0].id;
    let reverb = a.playback.plugins.iter().position(|p| p.id == "org.corinzia.reverb").unwrap();
    let _ = a.update(mx(mix::MixMsg::AddEffect(flute, reverb)));
    a.sync();
    let node = a.mix.fx.unwrap();
    let _ = a.update(mx(mix::MixMsg::Param(node, 0, 0.9)));
    let _ = a.update(mx(mix::MixMsg::PresetName("Wet".into())));
    let _ = a.update(mx(mix::MixMsg::SavePreset(mix::Target::Effect(node))));
    assert_eq!(a.mix.preset_names, ["Wet"], "{}", a.editor.status);
    let _ = a.update(mx(mix::MixMsg::Param(node, 0, 0.1)));
    let _ = a.update(mx(mix::MixMsg::LoadPreset(mix::Target::Effect(node), "Wet".into())));
    assert_eq!(a.playback.pool.effect(node).unwrap().param_value(0), Some(0.9));
    assert!(a.editor.project.plugin_state.contains_key(&node), "kept with the project");
    let _ = a.update(mx(mix::MixMsg::DeletePreset(mix::Target::Effect(node), "Wet".into())));
    assert!(a.mix.preset_names.is_empty());
    // A program for the flute: in the plan.
    let _ = a.update(mx(mix::MixMsg::Program(flute, "74 Flute".into())));
    assert_eq!(a.editor.project.tracks[0].program, Some(73));
    a.sync();
    let _ = a.update(mx(mix::MixMsg::Program(flute, "None".into())));
    assert_eq!(a.editor.project.tracks[0].program, None);
}

#[test]
fn audio_clips_are_added_moved_trimmed_faded_copied_and_deleted() {
    use arrange::{ArrangeEvent as E, ClipEdit};
    let mut a = app(View::Page);
    let _ = a.update(Message::Act(Action::ShowMix));
    let rate = a.playback.sample_rate;
    // Ten seconds of sound, already decoded under its name.
    a.playback.pool.media.insert("tone.wav", corinzia_audio::media::AudioData::from_channels(vec![vec![0.5; (rate * 10.0) as usize]], rate));
    a.editor.caret = Frac::int(1); // bar 2 in 4/4: four quarters in
    let _ = a.update(Message::AudioChosen(0, Some("tone.wav".into()))); // no such track: a new one
    let (track, clip) = a.mix.selected_clip.expect("a clip, selected");
    let t = a.editor.project.track(track).unwrap();
    assert!(t.audio && t.name == "tone" && t.clips.len() == 1);
    let sec = |a: &App, q: f64| arrange::seconds_of(a.movement_section(), q);
    let c = |a: &App| a.editor.clip(track, clip).unwrap().clone();
    assert!((c(&a).at - sec(&a, 4.0)).abs() < 1e-9, "at the caret");
    // Drawn on its lane, ten seconds long.
    a.sync();
    let b = a.mix.arrange_clips.iter().find(|b| b.clip == clip).unwrap();
    assert!((b.start_q - 4.0).abs() < 1e-6 && (b.length - 10.0).abs() < 1e-9);
    assert!(a.mix.arrange.lanes[b.lane].audio);

    let ar = |a: &mut App, e: E| {
        let _ = a.update(Message::Arrange(e));
    };
    // Move it to quarter 8: one undo step for the whole drag; playback
    // rebuilt once, when let go.
    let music = a.editor.music_revision;
    ar(&mut a, E::ClipPress { track, clip, copy: false });
    ar(&mut a, E::ClipDrag { edit: ClipEdit::Move, q: 6.0 });
    ar(&mut a, E::ClipDrag { edit: ClipEdit::Move, q: 8.0 });
    assert_eq!(a.editor.music_revision, music, "not while dragging");
    ar(&mut a, E::ClipRelease);
    assert!(a.editor.music_revision > music);
    assert!((c(&a).at - sec(&a, 8.0)).abs() < 1e-9);
    a.editor.undo();
    assert!((c(&a).at - sec(&a, 4.0)).abs() < 1e-9, "one undo puts it back");
    a.editor.redo();

    // Trim the front by a quarter: the sound stays where it was.
    let before = c(&a);
    ar(&mut a, E::ClipPress { track, clip, copy: false });
    ar(&mut a, E::ClipDrag { edit: ClipEdit::TrimStart, q: 9.0 });
    ar(&mut a, E::ClipRelease);
    let d = sec(&a, 9.0) - before.at;
    let now = c(&a);
    assert!((now.at - sec(&a, 9.0)).abs() < 1e-9 && (now.offset - d).abs() < 1e-9 && (now.length - (10.0 - d)).abs() < 1e-9);
    // Trimming the end past the file stops at the file's end.
    ar(&mut a, E::ClipPress { track, clip, copy: false });
    ar(&mut a, E::ClipDrag { edit: ClipEdit::TrimEnd, q: 400.0 });
    ar(&mut a, E::ClipRelease);
    assert!((c(&a).length - (10.0 - d)).abs() < 1e-9);
    // Fades from the corners.
    ar(&mut a, E::ClipPress { track, clip, copy: false });
    ar(&mut a, E::ClipDrag { edit: ClipEdit::FadeIn, q: 10.0 });
    ar(&mut a, E::ClipRelease);
    assert!((c(&a).fade_in - (sec(&a, 10.0) - now.at)).abs() < 1e-9);

    // Ctrl+drag: a copy moves, the original stays.
    let at = c(&a).at;
    ar(&mut a, E::ClipPress { track, clip, copy: true });
    ar(&mut a, E::ClipRelease);
    assert_eq!(a.editor.project.track(track).unwrap().clips.len(), 1, "a click alone copies nothing");
    ar(&mut a, E::ClipPress { track, clip, copy: true });
    ar(&mut a, E::ClipDrag { edit: ClipEdit::Move, q: 20.0 });
    ar(&mut a, E::ClipRelease);
    let clips = &a.editor.project.track(track).unwrap().clips;
    assert_eq!(clips.len(), 2);
    assert!(clips.iter().any(|x| x.id == clip && x.at == at));
    let copy = a.mix.selected_clip.unwrap().1;
    assert_ne!(copy, clip);

    // Delete removes the selected clip.
    let _ = a.update(Message::Key(iced::keyboard::Event::KeyPressed {
        key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Delete),
        modified_key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Delete),
        physical_key: iced::keyboard::key::Physical::Unidentified(iced::keyboard::key::NativeCode::Unidentified),
        location: iced::keyboard::Location::Standard,
        modifiers: Default::default(),
        text: None,
        repeat: false,
    }));
    assert_eq!(a.editor.project.track(track).unwrap().clips.len(), 1);
    assert!(a.mix.selected_clip.is_none());

    // The plan plays it: a node with the clip.
    a.sync();
    assert!(a.playback.clip_errors.is_empty(), "{:?}", a.playback.clip_errors);
}

#[test]
fn a_clip_cued_on_a_staff_is_engraved_there() {
    let mut a = app(View::Page);
    let rate = a.playback.sample_rate;
    a.playback.pool.media.insert("rain.wav", corinzia_audio::media::AudioData::from_channels(vec![vec![0.1; (rate * 3.0) as usize]], rate));
    a.editor.caret = Frac::new(1, 2);
    let _ = a.update(Message::AudioChosen(0, Some("rain.wav".into())));
    let (_, clip) = a.mix.selected_clip.unwrap();
    let drawn = |a: &App| a.score.pages.iter().flat_map(|p| p.items.iter()).filter(|i| i.id() == clip).cloned().collect::<Vec<Item>>();
    a.sync();
    // Not cued: only on its track's own staff, as the file's name.
    let texts = |a: &App| drawn(a).into_iter().filter_map(|i| match i { Item::Text { text, .. } => Some(text), _ => None }).collect::<Vec<_>>();
    assert_eq!(texts(&a), ["rain.wav"]);
    let lines = |a: &App| drawn(a).iter().filter(|i| matches!(i, Item::Line { .. })).count();
    let on_its_staff = lines(&a);
    let player = a.editor.project.players[1].name.clone();
    let _ = a.update(Message::Mix(mix::MixMsg::ClipCue(player)));
    a.sync();
    let items = drawn(&a);
    assert!(items.iter().any(|i| matches!(i, Item::Text { text, .. } if text == "rain ▸")), "{items:?}");
    assert_eq!(lines(&a), on_its_staff, "no cue line yet");
    let _ = a.update(Message::Mix(mix::MixMsg::ClipCueLine));
    a.sync();
    assert!(lines(&a) >= on_its_staff + 2, "a line and its hook besides the clip's own staff");
    // On the second player's staff: below the first staff's top.
    let first = a.score.pages[0].staves.iter().find(|s| s.part == 0).unwrap().top;
    let y = drawn(&a).iter().find_map(|i| match i { Item::Text { at, text, .. } if text == "rain ▸" => Some(at.y), _ => None }).unwrap();
    assert!(y > first + 4.0, "{y} under the first staff ({first})");
}

#[test]
fn a_clip_splits_into_two_that_play_as_it_did() {
    use iced::keyboard::{Key, Modifiers};
    let mut a = app(View::Page);
    let _ = a.update(Message::Act(Action::ShowMix));
    let rate = a.playback.sample_rate;
    a.playback.pool.media.insert("tone.wav", corinzia_audio::media::AudioData::from_channels(vec![vec![0.5; (rate * 10.0) as usize]], rate));
    a.editor.caret = Frac::int(1); // quarter 4
    let _ = a.update(Message::AudioChosen(0, Some("tone.wav".into())));
    let (track, clip) = a.mix.selected_clip.unwrap();
    a.editor.edit_clip(track, clip, "fades", false, |c| (c.fade_in, c.fade_out) = (0.5, 1.0));
    let sec = |a: &App, q: f64| arrange::seconds_of(a.movement_section(), q);
    let clips = |a: &App| a.editor.project.track(track).unwrap().clips.clone();
    // S splits the selected clip at the caret (stopped): quarter 8.
    a.editor.caret = Frac::int(2);
    let _ = a.press(&Key::Character("s".into()), Modifiers::default());
    let cs = clips(&a);
    assert_eq!(cs.len(), 2);
    let cut = sec(&a, 8.0) - sec(&a, 4.0);
    let (first, second) = (&cs[0], &cs[1]);
    assert!((first.length - cut).abs() < 1e-9 && first.fade_in == 0.5 && first.fade_out == 0.0, "{first:?}");
    assert!((second.at - sec(&a, 8.0)).abs() < 1e-9 && (second.offset - cut).abs() < 1e-9 && (second.length - (10.0 - cut)).abs() < 1e-9, "{second:?}");
    assert!(second.fade_in == 0.0 && second.fade_out == 1.0 && second.file == first.file);
    assert_eq!(a.mix.selected_clip, Some((track, second.id)), "the second half selected");
    // One undo puts it back whole.
    a.editor.undo();
    assert_eq!(clips(&a).len(), 1);
    assert_eq!(clips(&a)[0].length, 0.0);
    a.editor.redo();
    // Outside the clip: nothing, and a word why.
    a.editor.caret = Frac::int(30);
    let _ = a.update(Message::Mix(mix::MixMsg::ClipSplit));
    assert_eq!(clips(&a).len(), 2);
    assert!(a.editor.status.contains("split"), "{}", a.editor.status);
    // Alt+click on a clip: split there.
    let second = a.mix.selected_clip.unwrap().1;
    let _ = a.update(Message::Arrange(arrange::ArrangeEvent::ClipSplit { track, clip: second, q: 10.0 }));
    let cs = clips(&a);
    assert_eq!(cs.len(), 3);
    assert!(cs.iter().any(|c| (c.at - sec(&a, 10.0)).abs() < 1e-9));
    // Played back to back, the three cover what the one did.
    let total: f64 = cs.iter().map(|c| c.length).sum();
    assert!((total - 10.0).abs() < 1e-9, "{total}");
    a.sync();
    assert!(a.playback.clip_errors.is_empty());
}

#[test]
fn sounds_dropped_from_a_file_manager_land_where_they_fall() {
    let mut a = app(View::Page);
    let rate = a.playback.sample_rate;
    // Decoded already (files no clip uses are let go at each sync).
    let load = |a: &mut App, f: &str| a.playback.pool.media.insert(f, corinzia_audio::media::AudioData::from_channels(vec![vec![0.2; (rate * 2.0) as usize]], rate));
    load(&mut a, "a.wav");
    let tracks = |a: &App| a.editor.project.tracks.iter().filter(|t| t.audio).count();
    // On the score: a new track, at the caret.
    a.editor.caret = Frac::int(1);
    let _ = a.update(Message::DragOver(Some(iced::Point::new(300.0, 300.0))));
    let _ = a.view(); // the window outlined
    let _ = a.update(Message::DroppedAt(vec!["a.wav".into()], iced::Point::new(300.0, 300.0)));
    assert_eq!(tracks(&a), 1);
    let (track, _) = a.mix.selected_clip.expect("the clip, selected");
    let _ = a.update(Message::Act(Action::ShowMix));
    a.sync();
    let sec = |a: &App, q: f64| arrange::seconds_of(a.movement_section(), q);
    assert!((a.editor.project.track(track).unwrap().clips[0].at - sec(&a, 4.0)).abs() < 1e-9);
    // Over the arrange view: onto the audio track under the pointer, at
    // the beat under it; a second file on a track of its own.
    let layout = a.prefs.layout;
    let row = a.mix.arrange.rows.iter().position(|r| matches!(r, arrange::Row::Lane(l) if a.mix.arrange.lanes[*l].track == track)).unwrap();
    let at = iced::Point::new(layout.header_width + 12.0 * a.mix.arrange_ppq - a.mix.arrange_scroll + 1.0, 47.0 + arrange::RULER + (row as f32 + 0.5) * layout.lane_height - a.mix.arrange_scroll_y);
    assert_eq!(a.drop_target(at), Some((Some(track), 12.0)));
    let _ = a.update(Message::DragOver(Some(at)));
    let _ = a.view(); // highlighted where it would land
    assert!(a.drag_over.is_some() && a.editor.status == "Drop to add the sound to a", "{}", a.editor.status);
    load(&mut a, "b.wav");
    load(&mut a, "c.flac");
    let _ = a.update(Message::DroppedAt(vec!["b.wav".into(), "c.flac".into(), "notes.txt".into()], at));
    assert!(a.drag_over.is_none());
    let clips = &a.editor.project.track(track).unwrap().clips;
    assert_eq!(clips.len(), 2);
    assert!(clips.iter().any(|c| c.file == "b.wav" && (c.at - sec(&a, 12.0)).abs() < 1e-9));
    assert_eq!(tracks(&a), 2, "c.flac on a new track");
    assert!(a.editor.status.contains("notes.txt"), "the rest is not added: {}", a.editor.status);
    // In the score: a staff for each audio track, its clips by file name.
    let _ = a.update(Message::Act(Action::ShowScore));
    a.sync();
    let texts: Vec<String> = a.score.pages.iter().flat_map(|p| p.items.iter()).filter_map(|i| match i { Item::Text { text, bold: true, .. } => Some(text.clone()), _ => None }).collect();
    for f in ["a.wav", "b.wav", "c.flac"] {
        assert!(texts.iter().any(|t| t == f), "{f} in {texts:?}");
    }
    assert!(a.score.pages[0].staves.iter().filter(|s| s.single).count() >= 2);
    // A click on an audio staff is no player's.
    let audio = a.score.pages[0].staves.iter().find(|s| s.single).unwrap().part;
    assert_eq!(a.lane_of_staff(audio), None);
    assert_eq!(a.lane_of_staff(0), Some(0));
}

#[test]
fn buses_route_into_buses_without_loops_and_folders_fold_in_the_arrange_view() {
    let mut a = app(View::Page);
    let mx = |a: &mut App, m: mix::MixMsg| {
        let _ = a.update(Message::Mix(m));
    };
    mx(&mut a, mix::MixMsg::AddBus);
    mx(&mut a, mix::MixMsg::AddBus);
    let (b1, b2) = (a.editor.project.buses[0].id, a.editor.project.buses[1].id);
    let name = |a: &App, b: u64| a.editor.project.bus(b).unwrap().name.clone();
    let n2 = name(&a, b2);
    mx(&mut a, mix::MixMsg::BusOutput(b1, n2));
    assert_eq!(a.editor.project.bus(b1).unwrap().output, Some(b2));
    let n1 = name(&a, b1);
    mx(&mut a, mix::MixMsg::BusOutput(b2, n1));
    assert_eq!(a.editor.project.bus(b2).unwrap().output, None, "a loop is refused");
    assert!(a.editor.status.contains("cannot"));
    mx(&mut a, mix::MixMsg::Solo(b1));
    assert!(a.editor.project.bus(b1).unwrap().solo);
    // Removing a bus: what went into it goes to the master.
    mx(&mut a, mix::MixMsg::RemoveBus(b2));
    assert_eq!(a.editor.project.bus(b1).unwrap().output, None);

    // Folders: the first and third tracks in "Winds", shown together under its header.
    mx(&mut a, mix::MixMsg::AddAudioTrack);
    let ids: Vec<u64> = a.editor.project.tracks.iter().map(|t| t.id).collect();
    assert!(ids.len() >= 3);
    mx(&mut a, mix::MixMsg::Folder(ids[0], "Winds".into()));
    mx(&mut a, mix::MixMsg::Folder(ids[2], "Winds".into()));
    mx(&mut a, mix::MixMsg::Color(ids[1], Some(mix::TRACK_COLORS[6])));
    a.sync();
    use arrange::Row;
    assert_eq!(a.mix.arrange.rows[..4], [Row::Folder { name: "Winds".into(), collapsed: false, tracks: 2 }, Row::Lane(0), Row::Lane(2), Row::Lane(1)]);
    assert_eq!(a.mix.arrange.lanes[1].color, mix::color_of(mix::TRACK_COLORS[6]));
    let _ = a.update(Message::Arrange(arrange::ArrangeEvent::ToggleFolder("Winds".into())));
    a.sync();
    assert_eq!(a.mix.arrange.rows[..2], [Row::Folder { name: "Winds".into(), collapsed: true, tracks: 2 }, Row::Lane(1)]);
    assert_eq!((a.mix.arrange.row_of[0], a.mix.arrange.row_of[2], a.mix.arrange.row_of[1]), (None, None, Some(1)));
}

#[test]
fn strips_meter_their_levels_and_latch_a_clip() {
    let mut a = app(View::Page);
    a.sync();
    a.playback.play_from(a.editor.project.movements[0].id, 0.0);
    render(&mut a, 24000);
    a.playback.tick();
    let first = a.editor.project.tracks[0].id;
    let master = corinzia_model::MASTER;
    assert!(a.playback.level(master).0 > 0.01 && a.playback.level(first).0 > 0.0);
    assert!(!a.playback.clipped(master));
    // Stopped: the levels fall away over the frames that follow.
    a.playback.stop();
    render(&mut a, 256);
    for _ in 0..200 {
        render(&mut a, 256);
        a.playback.tick();
    }
    assert_eq!(a.playback.level(master), (0.0, 0.0));
    assert!(!a.playback.busy(), "no more frames needed");
    // Too loud: the master's light latches until clicked.
    let tracks: Vec<_> = a.editor.project.tracks.iter().map(|t| t.id).collect();
    for t in tracks {
        let _ = a.update(Message::Mix(mix::MixMsg::Gain(t, 12.0)));
    }
    let _ = a.update(Message::Mix(mix::MixMsg::Gain(master, 12.0)));
    // The instruments leave an orchestra's headroom: two gain inserts more.
    if a.playback.plugins.is_empty() {
        a.playback.scan();
    }
    let gain = a.playback.plugins.iter().position(|p| p.id == "org.corinzia.gain").unwrap();
    for _ in 0..2 {
        let _ = a.update(Message::Mix(mix::MixMsg::AddEffect(master, gain)));
        let node = a.mix.fx.unwrap();
        a.sync();
        let _ = a.update(Message::Mix(mix::MixMsg::Param(node, 0, 12.0)));
    }
    a.sync();
    a.playback.play_from(a.editor.project.movements[0].id, 0.0);
    render(&mut a, 24000);
    a.playback.tick();
    assert!(a.playback.clipped(master), "{:?}", a.playback.level(master));
    let _ = a.update(Message::Mix(mix::MixMsg::ClearClip(master)));
    assert!(!a.playback.clipped(master));
}

#[test]
fn a_track_shares_another_tracks_instrument_on_a_free_channel() {
    let mut a = app(View::Page);
    let (t0, t1) = (a.editor.project.tracks[0].id, a.editor.project.tracks[1].id);
    a.editor.set_track_plugin(t0, Some(("Corinzia Ensemble", corinzia_audio::host::BUILTIN, "org.corinzia.ensemble")));
    a.editor.set_track_plugin(t1, Some(("Corinzia Ensemble", corinzia_audio::host::BUILTIN, "org.corinzia.ensemble")));
    let own = a.editor.project.track(t1).unwrap().instrument_node.unwrap();
    let ch0 = a.editor.project.track(t0).unwrap().midi_channel;
    let _ = a.update(Message::Mix(mix::MixMsg::Share(t1, t0)));
    let (n0, n1) = (a.editor.project.track(t0).unwrap().instrument_node, a.editor.project.track(t1).unwrap().instrument_node);
    assert_eq!(n0, n1, "one plugin node");
    assert!(a.editor.project.node(own).is_none(), "its own plugin went");
    assert_ne!(a.editor.project.track(t1).unwrap().midi_channel, ch0, "on another channel");
    a.sync();
    assert!(std::sync::Arc::ptr_eq(a.playback.pool.plugin_of_track(t0).unwrap(), a.playback.pool.plugin_of_track(t1).unwrap()), "one instance");
    a.playback.play_from(a.editor.project.movements[0].id, 0.0);
    assert!(render(&mut a, 24000) > 0.001, "it plays");
    a.editor.undo();
    a.sync();
    assert!(!std::sync::Arc::ptr_eq(a.playback.pool.plugin_of_track(t0).unwrap(), a.playback.pool.plugin_of_track(t1).unwrap()));
}

#[test]
fn the_piano_roll_tweaks_how_notes_play_and_leaves_the_score_alone() {
    use roll::{Drag, RollEvent as R};
    let mut a = app(View::Page);
    let _ = a.act(Action::ShowMix);
    let track = a.editor.project.tracks[0].id;
    let _ = a.update(Message::Arrange(arrange::ArrangeEvent::OpenRoll(track)));
    let notes = a.mix.roll.as_ref().unwrap().content.notes.clone();
    assert!(notes.len() > 4, "the track's notes");
    let score_before = format!("{:?}", a.editor.project.movements[0].lanes);
    let (h0, h1) = (notes[0].head, notes[1].head);
    let ev = |a: &mut App, e: R| {
        let _ = a.update(Message::Roll(RollMsg::Event(e)));
    };
    // Two notes selected, dragged a sixteenth later: one undo step.
    ev(&mut a, R::Select { head: Some(h0), add: false });
    ev(&mut a, R::Select { head: Some(h1), add: true });
    let music = a.editor.music_revision;
    ev(&mut a, R::Drag { drag: Drag::Shift, dq: 0.125 });
    ev(&mut a, R::Drag { drag: Drag::Shift, dq: 0.25 });
    assert_eq!(a.editor.music_revision, music, "playback catches up on release");
    ev(&mut a, R::Release);
    assert!(a.editor.music_revision > music);
    assert_eq!(a.editor.tweak_of(h0).unwrap().shift, 0.25);
    assert_eq!(a.editor.tweak_of(h1).unwrap().shift, 0.25);
    a.editor.undo();
    assert!(a.editor.tweak_of(h0).is_none(), "one undo puts both back");
    a.editor.redo();
    // Longer, from the right edge; softer, from the velocity lane.
    ev(&mut a, R::Select { head: Some(h0), add: false });
    ev(&mut a, R::Drag { drag: Drag::Length, dq: 1.0 });
    ev(&mut a, R::Release);
    assert!((a.editor.tweak_of(h0).unwrap().length.unwrap() - (notes[0].length + 1.0)).abs() < 1e-9);
    ev(&mut a, R::Velocity { head: h0, velocity: 0.2 });
    ev(&mut a, R::Release);
    assert!((a.editor.tweak_of(h0).unwrap().velocity.unwrap() - 0.2).abs() < 0.01);
    // The score is as it was; the performance is not.
    assert_eq!(format!("{:?}", a.editor.project.movements[0].lanes), score_before);
    let perf = corinzia_perform::perform(&a.editor.project.movements[0]);
    let n0 = perf.notes.iter().find(|n| n.element == h0).unwrap();
    assert!((n0.velocity - 0.2).abs() < 0.01);
    a.sync();
    let r = a.mix.roll.as_ref().unwrap();
    assert_eq!(r.content.notes[0].start, notes[0].start, "the roll keeps the written place; the tweak moves it");
    // R resets the selection: played as written again.
    let _ = a.update(Message::Roll(RollMsg::Reset));
    assert!(a.editor.tweak_of(h0).is_none() && a.editor.tweak_of(h1).is_some());
    let _ = a.update(Message::Roll(RollMsg::Close));
    assert!(a.mix.roll.is_none());
}

#[test]
fn a_roll_note_is_grabbed_by_its_body_or_its_right_edge() {
    let r = roll::Roll { track: 1, name: "F".into(), notes: vec![roll::RollNote { head: 9, key: 60, start: 2.0, length: 1.0, velocity: 0.5 }], bars: vec![0.0, 4.0], length_q: 4.0, high: 72 };
    let tweaks = [corinzia_model::Tweak { head: 9, velocity: None, shift: 0.5, length: Some(2.0) }];
    let v = roll::RollView { roll: &r, tweaks: &tweaks, selected: &[], ppq: 40.0, scroll_x: 0.0, top_key: 72, playhead: None };
    let rect = v.note_rect(&r.notes[0]);
    assert_eq!((rect.x, rect.width), (v.x(2.5), 80.0), "where and how long it plays");
    let mid = rect.y + rect.height / 2.0;
    assert_eq!(v.note_at(iced::Point::new(rect.x + 20.0, mid)).map(|x| x.1), Some(false));
    assert_eq!(v.note_at(iced::Point::new(rect.x + rect.width - 2.0, mid)).map(|x| x.1), Some(true));
    assert!(v.note_at(iced::Point::new(rect.x - 10.0, mid)).is_none());
}

#[test]
fn a_clip_is_grabbed_by_its_body_edges_and_corners() {
    let content = arrange::Content { lanes: vec![arrange::Lane { track: 1, name: "A".into(), audio: true, color: iced::Color::WHITE, folder: String::new(), mixer: false, min_key: 60, max_key: 72 }], rows: vec![arrange::Row::Lane(0)], row_of: vec![Some(0)], length_q: 16.0, ..Default::default() };
    let block = arrange::ClipBlock { lane: 0, track: 1, clip: 2, name: "x".into(), start_q: 2.0, end_q: 6.0, fade_in_q: 2.0, fade_out_q: 6.0, in_shape: Default::default(), out_shape: Default::default(), gain_db: 0.0, muted: false, data: None, offset: 0.0, length: 2.0 };
    let cache = canvas::Cache::new();
    let sel = HashSet::new();
    let clips = [block];
    let v = arrange::ArrangeView { content: &content, pixels_per_quarter: 40.0, scroll_x: 0.0, scroll_y: 0.0, header: arrange::HEADER, mute_solo: vec![], lane_height: 56.0, selected: &sel, playhead: None, cache: &cache, curves: &[None], clips: &clips, selected_clip: None, cycle: None, drop_at: None, selected_track: None, levels: vec![] };
    let r = v.clip_rect(&clips[0]);
    let at = |x: f32, y: f32| v.clip_at(iced::Point::new(x, y)).map(|(_, e)| e);
    let mid_y = r.y + r.height / 2.0;
    assert_eq!(at(r.x + r.width / 2.0, mid_y), Some(arrange::ClipEdit::Move));
    assert_eq!(at(r.x + 2.0, mid_y), Some(arrange::ClipEdit::TrimStart));
    assert_eq!(at(r.x + r.width - 2.0, mid_y), Some(arrange::ClipEdit::TrimEnd));
    assert_eq!(at(r.x + 4.0, r.y + 3.0), Some(arrange::ClipEdit::FadeIn));
    assert_eq!(at(r.x + r.width - 4.0, r.y + 3.0), Some(arrange::ClipEdit::FadeOut));
    assert_eq!(at(r.x - 10.0, mid_y), None);
}

#[test]
fn automation_lanes_in_the_arrange_view() {
    let mut a = app(View::Page);
    let _ = a.update(Message::Act(Action::ShowMix));
    let flute = a.editor.project.tracks[0].id;
    let mx = |m: mix::MixMsg| Message::Mix(m);
    let _ = a.update(mx(mix::MixMsg::AutoShow(flute, "Gain".into())));
    a.sync();
    assert_eq!(a.mix.arrange_curves[0].as_ref().map(|c| (c.min, c.max, c.points.len())), Some((fader::MIN_DB, 6.0, 0)));
    let ar = |e: arrange::ArrangeEvent| Message::Arrange(e);
    let _ = a.update(ar(arrange::ArrangeEvent::AutoAdd { lane: 0, q: 0.0, value: 0.0 }));
    let _ = a.update(ar(arrange::ArrangeEvent::AutoAdd { lane: 0, q: 8.0, value: -20.0 }));
    // A drag: one undo step.
    let _ = a.update(ar(arrange::ArrangeEvent::AutoMove { lane: 0, index: 1, q: 6.0, value: -24.0 }));
    let _ = a.update(ar(arrange::ArrangeEvent::AutoMove { lane: 0, index: 1, q: 7.0, value: -30.0 }));
    let _ = a.update(ar(arrange::ArrangeEvent::AutoRelease));
    a.sync();
    let lane = a.editor.auto_lane(corinzia_model::AutoTarget::Gain(flute)).unwrap().clone();
    assert_eq!(lane.points.iter().map(|p| (p.at, p.value)).collect::<Vec<_>>(), [(Frac::ZERO, 0.0), (Frac::new(7, 4), -30.0)]);
    a.editor.undo();
    assert_eq!(a.editor.auto_lane(corinzia_model::AutoTarget::Gain(flute)).unwrap().points[1].value, -20.0);
    let _ = a.update(ar(arrange::ArrangeEvent::AutoDelete { lane: 0, index: 1 }));
    let _ = a.update(ar(arrange::ArrangeEvent::AutoDelete { lane: 0, index: 0 }));
    assert!(a.editor.auto_lane(corinzia_model::AutoTarget::Gain(flute)).is_none(), "an empty lane goes");
    // Plugin parameters are offered too; dynamics as CC11.
    let gain = a.playback.plugins.iter().position(|p| p.id == "org.corinzia.gain").unwrap();
    let _ = a.update(mx(mix::MixMsg::AddEffect(flute, gain)));
    a.sync();
    let labels: Vec<String> = mix::auto_options(&a.editor.project, &a.playback, flute).into_iter().map(|o| o.0).collect();
    assert!(labels.contains(&"Corinzia Gain: Gain".to_string()), "{labels:?}");
    let _ = a.update(mx(mix::MixMsg::DynamicsCc(flute, mix::DYNAMICS_CC[2].into())));
    assert_eq!(a.editor.project.tracks[0].dynamics_cc, Some(11));
    let _ = a.update(mx(mix::MixMsg::AutoShow(flute, "Off".into())));
    a.sync();
    assert!(a.mix.arrange_curves[0].is_none());
}

#[test]
fn expression_maps_from_the_mix_tab() {
    let mut a = app(View::Page);
    let _ = a.update(Message::Act(Action::ShowMix));
    let flute = a.editor.project.tracks[0].id;
    let mx = |m: mix::MixMsg| Message::Mix(m);
    let _ = a.update(mx(mix::MixMsg::Select(flute)));
    let _ = a.update(mx(mix::MixMsg::TechAdd(flute)));
    let _ = a.update(mx(mix::MixMsg::TechAdd(flute)));
    let tech = |a: &App| a.editor.project.tracks[0].techniques.clone();
    assert_eq!(tech(&a).iter().map(|t| t.technique.as_str()).collect::<Vec<_>>(), ["ord.", "pizz."]);
    use corinzia_model::SwitchAction::*;
    let _ = a.update(mx(mix::MixMsg::TechEdit(flute, 1, mix::TechField::Kind, "Program".into())));
    let _ = a.update(mx(mix::MixMsg::TechEdit(flute, 1, mix::TechField::Value, "46 Pizzicato Strings".into())));
    let _ = a.update(mx(mix::MixMsg::TechEdit(flute, 0, mix::TechField::Value, "D1".into())));
    let _ = a.update(mx(mix::MixMsg::TechEdit(flute, 0, mix::TechField::Kind, "Controller".into())));
    let _ = a.update(mx(mix::MixMsg::TechEdit(flute, 0, mix::TechField::Value, "CC58".into())));
    let _ = a.update(mx(mix::MixMsg::TechEdit(flute, 0, mix::TechField::Value2, "64".into())));
    assert_eq!(tech(&a).iter().map(|t| t.action).collect::<Vec<_>>(), [Cc(58, 64), Program(45)]);
    let _ = a.update(mx(mix::MixMsg::TechEdit(flute, 1, mix::TechField::Kind, "Channel".into())));
    let _ = a.update(mx(mix::MixMsg::TechEdit(flute, 1, mix::TechField::Value, "5".into())));
    assert_eq!(tech(&a)[1].action, Channel(4));
    let _ = a.update(mx(mix::MixMsg::TechRemove(flute, 0)));
    assert_eq!(tech(&a).len(), 1);
    a.sync();
    assert_eq!(mix::key_name(60), "C4");
}

#[test]
fn a_strip_click_shows_the_plugin_editor() {
    let mut a = app(View::Page);
    let _ = a.update(Message::Act(Action::ShowMix));
    let track = a.editor.project.tracks[0].id;
    // The built-in instrument has no editor: the strip says so.
    let _ = a.update(Message::Mix(mix::MixMsg::Editor(track)));
    assert_eq!(a.mix.strip, Some(track), "the click selects the strip");
    assert!(!a.playback.pool.editor_is_open(track));
    assert_eq!(a.editor.status, "Corinzia Ensemble has no editor of its own");
    assert!(!a.playback.editors_busy());
    // Selecting by name leaves editors alone.
    let cello = a.editor.project.tracks[1].id;
    let _ = a.update(Message::Mix(mix::MixMsg::Select(cello)));
    assert_eq!(a.mix.strip, Some(cello));
}

#[test]
fn mixer_edits_are_live_and_undoable() {
    let mut a = app(View::Page);
    let _ = a.update(Message::Act(Action::ShowMix));
    assert!(a.playback.plugins.iter().any(|p| p.id == "org.corinzia.ensemble"));
    let (engraved, track) = (a.score.engraved, a.editor.project.tracks[0].id);
    for db in [-1.0, -2.5, -4.0] {
        let _ = a.update(Message::Mix(mix::MixMsg::Gain(track, db)));
    }
    let _ = a.update(Message::Mix(mix::MixMsg::Released));
    assert_eq!(a.editor.project.tracks[0].gain_db, -4.0);
    assert_eq!(a.score.engraved, engraved, "mixer edits do not re-engrave");
    // A second drag is a step of its own.
    let _ = a.update(Message::Mix(mix::MixMsg::Gain(track, -10.0)));
    let _ = a.update(Message::Mix(mix::MixMsg::Released));
    a.editor.undo();
    assert_eq!(a.editor.project.tracks[0].gain_db, -4.0, "the second drag undone alone");
    a.editor.undo();
    assert_eq!(a.editor.project.tracks[0].gain_db, 0.0, "one drag, one undo step");
    // Solo the cello: the flute is silent.
    let cello = a.editor.project.tracks[1].id;
    let _ = a.update(Message::Mix(mix::MixMsg::Solo(cello)));
    assert!(a.editor.project.tracks[1].solo);
    // Choose the built-in instrument explicitly: no node needed.
    let sine = a.playback.plugins.iter().position(|p| p.id == "org.corinzia.ensemble").unwrap();
    let _ = a.update(Message::Mix(mix::MixMsg::Assign(track, sine)));
    assert_eq!(a.editor.project.tracks[0].instrument_node, None);
    assert_eq!(a.playback.plugin_name(track), "Corinzia Ensemble");
}

#[test]
fn the_mix_tab_panes_scroll_and_resize() {
    let mut a = App::new(crate::demo::orchestra(), None, View::Page, String::new());
    let _ = a.update(Message::Act(Action::ShowMix));
    a.sync();
    let rows = a.mix.arrange.rows.len();
    assert!(rows >= 14, "the orchestra's tracks (and the master)");
    // The tracks scroll down, no further than the last one at the top.
    let _ = a.update(Message::Arrange(arrange::ArrangeEvent::ScrollY(120.0)));
    assert_eq!(a.mix.arrange_scroll_y, 120.0);
    let _ = a.update(Message::Arrange(arrange::ArrangeEvent::ScrollY(1.0e6)));
    assert_eq!(a.mix.arrange_scroll_y, (rows - 1) as f32 * a.prefs.layout.lane_height);
    let _ = a.update(Message::Arrange(arrange::ArrangeEvent::ScrollY(-1.0e6)));
    assert_eq!(a.mix.arrange_scroll_y, 0.0);
    // Taller tracks, within limits; a wider header.
    let _ = a.update(Message::Arrange(arrange::ArrangeEvent::LaneHeight(90.0)));
    assert_eq!(a.prefs.layout.lane_height, 90.0);
    let _ = a.update(Message::Arrange(arrange::ArrangeEvent::LaneHeight(5.0)));
    assert_eq!(a.prefs.layout.lane_height, arrange::LANE_HEIGHTS.0);
    let _ = a.update(Message::Arrange(arrange::ArrangeEvent::HeaderWidth(260.0)));
    assert_eq!(a.prefs.layout.header_width, 260.0);
    // The dividers: the mixer taller when dragged up, the side panel wider
    // when dragged left; both within limits.
    let h = a.prefs.layout.mixer_height;
    let _ = a.update(Message::SplitMixer(Some(-100.0)));
    assert_eq!(a.prefs.layout.mixer_height, h + 100.0);
    let _ = a.update(Message::SplitMixer(Some(1.0e6)));
    assert_eq!(a.prefs.layout.mixer_height, crate::prefs::Layout::MIXER.0);
    let w = a.prefs.layout.side_width;
    let _ = a.update(Message::Mix(mix::MixMsg::SplitSide(Some(-50.0))));
    assert_eq!(a.prefs.layout.side_width, w + 50.0);
    let _ = a.update(Message::SplitMixer(None));
    // Kept with the preferences, sane when read back.
    let saved = serde_json::to_string(&a.prefs).unwrap();
    let back: crate::prefs::Prefs = serde_json::from_str(&saved).unwrap();
    assert_eq!(back.layout, a.prefs.layout);
    // A header's mute and solo buttons: the track's own.
    let t = a.editor.project.tracks[0].id;
    let _ = a.update(Message::Arrange(arrange::ArrangeEvent::Mute(t)));
    assert!(a.editor.project.tracks[0].mute);
    let _ = a.update(Message::Arrange(arrange::ArrangeEvent::Solo(t)));
    assert!(a.editor.project.tracks[0].solo);
    let old: crate::prefs::Prefs = serde_json::from_str(r#"{"theme": "Dark"}"#).unwrap();
    assert_eq!(old.layout, crate::prefs::Layout::default(), "preferences from before keep the defaults");
}

#[test]
fn the_score_follows_a_recordings_tempo() {
    use corinzia_perform::{perform_with, Style};
    let mut a = app(View::Page);
    let _ = a.update(Message::Act(Action::ShowMix));
    let rate = a.playback.sample_rate;
    let style = Style::of(&a.editor.project);
    let m = a.editor.movement().unwrap().clone();
    // A recording placed two bars in, its beat a little slower than the
    // score's and slowing further: clicks over a decaying tone.
    let tempo = perform_with(&m, style.clone()).tempo;
    let at = tempo.seconds_at(8.0);
    let unit = corinzia_perform::follow::beat_unit(&m, 8.0);
    let beat = unit * 60.0 / tempo.tempo_at(8.0);
    let mut clicks = vec![0.3];
    for i in 0..16 {
        clicks.push(clicks.last().unwrap() + beat * (1.1 + 0.25 * i as f64 / 15.0));
    }
    let mut x = vec![0.0f32; ((clicks.last().unwrap() + 1.0) * rate) as usize];
    for &t in &clicks {
        let s = (t * rate) as usize;
        for i in 0..(0.06 * rate) as usize {
            let e = (-(i as f64) / (0.012 * rate)).exp() as f32;
            let tone = (2.0 * std::f64::consts::PI * 330.0 * i as f64 / rate).sin() as f32;
            if let Some(v) = x.get_mut(s + i) {
                *v += 0.6 * e * tone + 0.3 * e * if i % 3 == 0 { 1.0 } else { -0.5 };
            }
        }
    }
    a.playback.pool.media.insert("take.wav", corinzia_audio::media::AudioData::from_channels(vec![x], rate));
    a.editor.caret = Frac::new(8, 4);
    let _ = a.update(Message::AudioChosen(0, Some("take.wav".into())));
    let (track, clip) = a.mix.selected_clip.unwrap();
    assert!((a.editor.clip(track, clip).unwrap().at - at).abs() < 1e-9);
    let marks = |a: &App| corinzia_model::notation::score_as(&a.editor.project, a.editor.movement().unwrap(), false).tempo.len();
    let shown = marks(&a);
    let _ = a.update(Message::Mix(mix::MixMsg::FollowClip));
    assert!(a.editor.status.starts_with("The score follows"), "{}", a.editor.status);
    // The score's beats now fall on the clicks.
    let m2 = a.editor.movement().unwrap().clone();
    assert!(m2.tempo.iter().any(|t| t.hidden));
    let tempo2 = perform_with(&m2, style).tempo;
    let first = (tempo2.quarter_at(at + clicks[0]) / unit).round() * unit;
    let mut hit = 0;
    for (i, c) in clicks.iter().enumerate() {
        let t = tempo2.seconds_at(first + i as f64 * unit);
        if (t - (at + c)).abs() < 0.03 {
            hit += 1;
        }
    }
    assert!(hit >= clicks.len() - 2, "{hit} of {} beats on their clicks", clicks.len());
    // Nothing new on the page; one undo puts the tempo back; Forget removes it.
    assert_eq!(marks(&a), shown);
    a.editor.undo();
    assert!(!a.editor.movement().unwrap().tempo.iter().any(|t| t.hidden));
    a.editor.redo();
    let _ = a.update(Message::Mix(mix::MixMsg::ForgetFollowed));
    assert_eq!(a.editor.movement().unwrap().tempo, m.tempo);
}

#[test]
fn the_mix_tab_answers_daw_keys_and_wheel() {
    use iced::keyboard::{key::Named, Key, Modifiers};
    let mut a = app(View::Page);
    let _ = a.update(Message::Act(Action::ShowMix));
    a.sync();
    let press = |a: &mut App, k: Key, m: Modifiers| {
        if let Some(t) = a.press(&k, m) {
            drop(t);
        }
        a.sync();
    };
    let ch = |c: &str| Key::Character(c.into());
    // Ctrl+T: a new track; double-clicking below the tracks does the same.
    let tracks = a.editor.project.tracks.len();
    let _ = a.mix_key(&ch("t"), Modifiers::CTRL);
    assert_eq!(a.editor.project.tracks.len(), tracks + 1);
    let _ = a.update(Message::Arrange(arrange::ArrangeEvent::AddTrack));
    assert_eq!(a.editor.project.tracks.len(), tracks + 2);
    assert_eq!(a.mix.strip, a.editor.project.tracks.last().map(|t| t.id), "the new one selected");
    // Ctrl+M hides the mixer, and back.
    let _ = a.mix_key(&ch("m"), Modifiers::CTRL);
    assert!(a.mix.mixer_hidden);
    let _ = a.view();
    let _ = a.mix_key(&ch("m"), Modifiers::CTRL);
    assert!(!a.mix.mixer_hidden);
    // + and − zoom along time; PageUp and PageDown the tracks' height.
    let ppq = a.mix.arrange_ppq;
    press(&mut a, ch("+"), Modifiers::default());
    let _ = a.update(Message::Arrange(arrange::ArrangeEvent::Zoom(1.25)));
    assert!(a.mix.arrange_ppq > ppq);
    let h = a.prefs.layout.lane_height;
    let _ = a.update(Message::Arrange(arrange::ArrangeEvent::LaneHeight(h * 1.25)));
    assert!(a.prefs.layout.lane_height > h);
    assert!(a.mix_key(&Key::Named(Named::PageUp), Modifiers::default()).is_some());
    // Ctrl+PageDown: the whole movement in view.
    let _ = a.mix_key(&Key::Named(Named::PageDown), Modifiers::CTRL);
    let width = a.window.width - a.prefs.layout.header_width;
    assert!((a.mix.arrange.length_q as f32 * a.mix.arrange_ppq - width).abs() < 1.0 || a.mix.arrange_ppq == 2.0);
    // Ctrl+Alt+Down selects the next track.
    a.mix.strip = a.mix.arrange.lanes.first().map(|l| l.track);
    let _ = a.mix_key(&Key::Named(Named::ArrowDown), Modifiers::CTRL | Modifiers::ALT);
    assert_eq!(a.mix.strip, a.mix.arrange.lanes.get(1).map(|l| l.track));
    // The wheel zooms around the pointer: the beat under it stays there.
    a.mix.arrange_ppq = 20.0;
    a.mix.arrange_scroll = 100.0;
    let header = a.prefs.layout.header_width;
    let x = header + 300.0;
    let q = (x - header + a.mix.arrange_scroll) / a.mix.arrange_ppq;
    let _ = a.update(Message::Arrange(arrange::ArrangeEvent::ZoomAt { factor: 1.5, x }));
    assert!((a.mix.arrange_ppq - 30.0).abs() < 1e-4);
    assert!(((x - header + a.mix.arrange_scroll) / a.mix.arrange_ppq - q).abs() < 1e-3, "the beat under the pointer stays");
}

#[test]
fn notes_gather_into_items_split_by_a_bar_of_rest() {
    let a = app(View::Page);
    let c = &a.mix.arrange;
    // Every note lies inside an item of its lane, and items start and end
    // on bar lines.
    for b in &c.blocks {
        assert!(c.items.iter().any(|&(l, x, y)| l == b.lane && x <= b.start_q + 1e-9 && b.start_q + b.length_q <= y + 1e-9), "{b:?}");
    }
    for &(_, x, y) in &c.items {
        assert!(x < y && c.bars.iter().any(|b| (b - x).abs() < 1e-9) && c.bars.iter().any(|b| (b - y).abs() < 1e-9));
    }
    // Beats include every bar line.
    assert!(c.bars[..c.bars.len() - 1].iter().all(|b| c.beats.iter().any(|x| (x - b).abs() < 1e-9)));
}

#[test]
fn silence_strips_gain_drags_and_fades_take_shapes() {
    use arrange::{ArrangeEvent as E, ClipEdit};
    let mut a = app(View::Page);
    let _ = a.update(Message::Act(Action::ShowMix));
    let rate = a.playback.sample_rate;
    // 1 s of sound, 1 s of silence, 0.5 s of sound.
    let n = rate as usize;
    let mut x = vec![0.3f32; n];
    x.extend(vec![0.0; n]);
    x.extend(vec![0.3; n / 2]);
    a.playback.pool.media.insert("take.wav", corinzia_audio::media::AudioData::from_channels(vec![x], rate));
    a.editor.caret = Frac::int(1);
    let _ = a.update(Message::AudioChosen(0, Some("take.wav".into())));
    let (track, clip) = a.mix.selected_clip.unwrap();
    let at = a.editor.clip(track, clip).unwrap().at;
    // Fade shapes from the clip panel.
    let _ = a.update(Message::Mix(mix::MixMsg::FadeInShape("S-curve".into())));
    let _ = a.update(Message::Mix(mix::MixMsg::FadeOutShape("Equal power".into())));
    let c = a.editor.clip(track, clip).unwrap().clone();
    assert_eq!((c.fade_in_shape, c.fade_out_shape), (corinzia_model::FadeShape::SCurve, corinzia_model::FadeShape::EqualPower));
    // The gain line, dragged: one undo step.
    let _ = a.update(Message::Arrange(E::ClipPress { track, clip, copy: false }));
    let _ = a.update(Message::Arrange(E::ClipDrag { edit: ClipEdit::Gain, q: -6.0 }));
    let _ = a.update(Message::Arrange(E::ClipRelease));
    assert_eq!(a.editor.clip(track, clip).unwrap().gain_db, -6.0);
    // Strip silence: two parts where the sound is, in place.
    let _ = a.update(Message::Mix(mix::MixMsg::StripSilence));
    let clips = a.editor.project.track(track).unwrap().clips.clone();
    assert_eq!(clips.len(), 2, "{}", a.editor.status);
    let (p, q) = (&clips[0], &clips[1]);
    assert!((p.at - at).abs() < 0.02 && (p.length - 1.0).abs() < 0.03, "{p:?}");
    assert!((q.at - (at + 2.0)).abs() < 0.03 && (q.offset - 2.0).abs() < 0.03 && (q.length - 0.5).abs() < 0.03, "{q:?}");
    assert!(p.gain_db == -6.0 && q.fade_out_shape == corinzia_model::FadeShape::EqualPower);
    a.editor.undo();
    assert_eq!(a.editor.project.track(track).unwrap().clips.len(), 1);
    a.sync();
    assert!(a.playback.clip_errors.is_empty());
}
