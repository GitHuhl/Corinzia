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
    let node = a.fx.expect("the new effect is selected");
    assert_eq!(a.editor.project.tracks[0].inserts, [node]);
    a.sync();
    assert!(a.playback.pool.effect(node).is_some(), "loaded with the new plan");
    let _ = a.update(mx(mix::MixMsg::Param(node, 0, -60.0)));
    assert_eq!(a.playback.pool.effect(node).unwrap().param_value(0), Some(-60.0));
    // A bus with a reverb; the flute sends to it.
    let _ = a.update(mx(mix::MixMsg::AddBus));
    let bus = a.editor.project.buses[0].id;
    assert_eq!((a.strip, a.editor.project.buses[0].name.as_str()), (Some(bus), "Bus 1"));
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
    assert!(render(&mut a, 9600) > 0.001, "heard through the bus");
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
    let node = a.fx.unwrap();
    let _ = a.update(mx(mix::MixMsg::Param(node, 0, 0.9)));
    let _ = a.update(mx(mix::MixMsg::PresetName("Wet".into())));
    let _ = a.update(mx(mix::MixMsg::SavePreset(mix::Target::Effect(node))));
    assert_eq!(a.preset_names, ["Wet"], "{}", a.editor.status);
    let _ = a.update(mx(mix::MixMsg::Param(node, 0, 0.1)));
    let _ = a.update(mx(mix::MixMsg::LoadPreset(mix::Target::Effect(node), "Wet".into())));
    assert_eq!(a.playback.pool.effect(node).unwrap().param_value(0), Some(0.9));
    assert!(a.editor.project.plugin_state.contains_key(&node), "kept with the project");
    let _ = a.update(mx(mix::MixMsg::DeletePreset(mix::Target::Effect(node), "Wet".into())));
    assert!(a.preset_names.is_empty());
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
    let (track, clip) = a.selected_clip.expect("a clip, selected");
    let t = a.editor.project.track(track).unwrap();
    assert!(t.audio && t.name == "tone" && t.clips.len() == 1);
    let sec = |a: &App, q: f64| arrange::seconds_of(a.movement_section(), q);
    let c = |a: &App| a.editor.clip(track, clip).unwrap().clone();
    assert!((c(&a).at - sec(&a, 4.0)).abs() < 1e-9, "at the caret");
    // Drawn on its lane, ten seconds long.
    a.sync();
    let b = a.arrange_clips.iter().find(|b| b.clip == clip).unwrap();
    assert!((b.start_q - 4.0).abs() < 1e-6 && (b.length - 10.0).abs() < 1e-9);
    assert!(a.arrange.lanes[b.lane].audio);

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
    let copy = a.selected_clip.unwrap().1;
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
    assert!(a.selected_clip.is_none());

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
    let (_, clip) = a.selected_clip.unwrap();
    let drawn = |a: &App| a.pages.iter().flat_map(|p| p.items.iter()).filter(|i| i.id() == clip).cloned().collect::<Vec<Item>>();
    a.sync();
    assert!(drawn(&a).is_empty(), "not cued: not in the score");
    let player = a.editor.project.players[1].name.clone();
    let _ = a.update(Message::Mix(mix::MixMsg::ClipCue(player)));
    a.sync();
    let items = drawn(&a);
    assert!(items.iter().any(|i| matches!(i, Item::Text { text, .. } if text == "rain ▸")), "{items:?}");
    assert!(!items.iter().any(|i| matches!(i, Item::Line { .. })), "no line yet");
    let _ = a.update(Message::Mix(mix::MixMsg::ClipCueLine));
    a.sync();
    assert!(drawn(&a).iter().filter(|i| matches!(i, Item::Line { .. })).count() >= 2, "a line and its hook");
    // On the second player's staff: below the first staff's top.
    let first = a.pages[0].staves.iter().find(|s| s.part == 0).unwrap().top;
    let y = drawn(&a).iter().find_map(|i| match i { Item::Text { at, .. } => Some(at.y), _ => None }).unwrap();
    assert!(y > first + 4.0, "{y} under the first staff ({first})");
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
    assert_eq!(a.arrange.rows[..4], [Row::Folder { name: "Winds".into(), collapsed: false, tracks: 2 }, Row::Lane(0), Row::Lane(2), Row::Lane(1)]);
    assert_eq!(a.arrange.lanes[1].color, mix::color_of(mix::TRACK_COLORS[6]));
    let _ = a.update(Message::Arrange(arrange::ArrangeEvent::ToggleFolder("Winds".into())));
    a.sync();
    assert_eq!(a.arrange.rows[..2], [Row::Folder { name: "Winds".into(), collapsed: true, tracks: 2 }, Row::Lane(1)]);
    assert_eq!((a.arrange.row_of[0], a.arrange.row_of[2], a.arrange.row_of[1]), (None, None, Some(1)));
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
    let _ = a.update(Message::Mix(mix::MixMsg::Gain(first, 12.0)));
    let _ = a.update(Message::Mix(mix::MixMsg::Gain(master, 12.0)));
    a.sync();
    a.playback.play_from(a.editor.project.movements[0].id, 0.0);
    render(&mut a, 24000);
    a.playback.tick();
    assert!(a.playback.clipped(master));
    let _ = a.update(Message::Mix(mix::MixMsg::ClearClip(master)));
    assert!(!a.playback.clipped(master));
}

#[test]
fn a_track_shares_another_tracks_instrument_on_a_free_channel() {
    let mut a = app(View::Page);
    let (t0, t1) = (a.editor.project.tracks[0].id, a.editor.project.tracks[1].id);
    a.editor.set_track_plugin(t0, Some(("Corinzia Sine", corinzia_audio::host::BUILTIN, "org.corinzia.sine")));
    a.editor.set_track_plugin(t1, Some(("Corinzia Sine", corinzia_audio::host::BUILTIN, "org.corinzia.sine")));
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
    let notes = a.roll.as_ref().unwrap().content.notes.clone();
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
    let r = a.roll.as_ref().unwrap();
    assert_eq!(r.content.notes[0].start, notes[0].start, "the roll keeps the written place; the tweak moves it");
    // R resets the selection: played as written again.
    let _ = a.update(Message::Roll(RollMsg::Reset));
    assert!(a.editor.tweak_of(h0).is_none() && a.editor.tweak_of(h1).is_some());
    let _ = a.update(Message::Roll(RollMsg::Close));
    assert!(a.roll.is_none());
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
    let block = arrange::ClipBlock { lane: 0, track: 1, clip: 2, name: "x".into(), start_q: 2.0, end_q: 6.0, fade_in_q: 2.0, fade_out_q: 6.0, muted: false, data: None, offset: 0.0, length: 2.0 };
    let cache = canvas::Cache::new();
    let sel = HashSet::new();
    let clips = [block];
    let v = arrange::ArrangeView { content: &content, pixels_per_quarter: 40.0, scroll_x: 0.0, lane_height: 56.0, selected: &sel, playhead: None, cache: &cache, curves: &[None], clips: &clips, selected_clip: None, cycle: None };
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
    assert_eq!(a.arrange_curves[0].as_ref().map(|c| (c.min, c.max, c.points.len())), Some((fader::MIN_DB, 6.0, 0)));
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
    assert!(a.arrange_curves[0].is_none());
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
    assert_eq!(a.strip, Some(track), "the click selects the strip");
    assert!(!a.playback.pool.editor_is_open(track));
    assert_eq!(a.editor.status, "Corinzia Sine has no editor of its own");
    assert!(!a.playback.editors_busy());
    // Selecting by name leaves editors alone.
    let cello = a.editor.project.tracks[1].id;
    let _ = a.update(Message::Mix(mix::MixMsg::Select(cello)));
    assert_eq!(a.strip, Some(cello));
}

#[test]
fn mixer_edits_are_live_and_undoable() {
    let mut a = app(View::Page);
    let _ = a.update(Message::Act(Action::ShowMix));
    assert!(a.playback.plugins.iter().any(|p| p.id == "org.corinzia.sine"));
    let (engraved, track) = (a.engraved, a.editor.project.tracks[0].id);
    for db in [-1.0, -2.5, -4.0] {
        let _ = a.update(Message::Mix(mix::MixMsg::Gain(track, db)));
    }
    let _ = a.update(Message::Mix(mix::MixMsg::Released));
    assert_eq!(a.editor.project.tracks[0].gain_db, -4.0);
    assert_eq!(a.engraved, engraved, "mixer edits do not re-engrave");
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
    let sine = a.playback.plugins.iter().position(|p| p.id == "org.corinzia.sine").unwrap();
    let _ = a.update(Message::Mix(mix::MixMsg::Assign(track, sine)));
    assert_eq!(a.editor.project.tracks[0].instrument_node, None);
    assert_eq!(a.playback.plugin_name(track), "Corinzia Sine");
}
