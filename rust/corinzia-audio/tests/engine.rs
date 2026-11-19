use corinzia_audio::engine::{self, Command};
use corinzia_audio::host::{self, Library, Plugin};
use corinzia_audio::pool::PluginPool;
use corinzia_audio::snapshot;
use corinzia_model::*;
use corinzia_score::Frac;
use std::sync::Arc;

const SR: f64 = 48000.0;

/// One flute, 4/4 at 120 bpm: c'' on beat 1 (a quarter), rest, rest, a'' on
/// beat 4. Played by the built-in sine (General MIDI 79): these tests are
/// about the engine, and it speaks at once and the same every time (a
/// flute takes a fifth of a second).
fn project() -> Project {
    let mut p = Project { title: "T".into(), sample_rate: SR, next_id: 1000, ..Default::default() };
    p.tracks.push(Track { id: 1, name: "Flute".into(), instrument_node: None, note_port: 0, output_bus: None, midi_channel: 0, gain_db: 0.0, pan: 0.0, mute: false, solo: false, inserts: vec![], sends: vec![], dynamics_cc: None, program: Some(79), bank: None, techniques: vec![], color: None, folder: String::new(), audio: false, clips: vec![] });
    p.instruments.push(Instrument { id: 2, player: 3, name: "Flute".into(), engraver_name: "Fl.".into(), transposition: corinzia_model::Interval::default(), clef: "treble".into(), track: Some(1), drums: vec![], tuning: vec![] });
    p.players.push(Player { id: 3, name: "Flute".into(), instruments: vec![2] });
    let note = |id, start: Frac, step| Event { id, kind: EventKind::Note, start, duration: Frac::new(1, 4), heads: vec![Head { id: id + 1, pitch: Pitch { step, alter: 0, octave: 5 }, tie: false, velocity: 0 }], articulations: 0, dynamic: Some(5), spans: 0, lyrics: vec![], graces: vec![], fingering: vec![] };
    let rest = |id, start: Frac| Event { id, kind: EventKind::Rest, start, duration: Frac::new(1, 2), heads: vec![], articulations: 0, dynamic: None, spans: 0, lyrics: vec![], graces: vec![], fingering: vec![] };
    p.movements.push(Movement {
        id: 9,
        ordinal: 0,
        title: String::new(),
        length: Frac::int(1),
        revision: 0,
        tempo: vec![Tempo { at: Frac::ZERO, bpm: 120.0, ramp: false, text: String::new(), hidden: false }],
        meter: vec![Meter { at: Frac::ZERO, num: 4, den: 4, pickup: false, beams: [0; 8] }],
        keys: vec![Key { at: Frac::ZERO, fifths: 0, minor: false }],
        barlines: vec![],
        rehearsal: vec![],
        navigation: vec![],
        slurs: vec![],
        endings: vec![],
        lanes: vec![Lane {
            player: 3,
            changes: vec![InstrumentChange { id: 4, at: Frac::ZERO, instrument: 2 }],
            clefs: vec![],
            texts: vec![],
            chords: vec![],
            lines: vec![],
            voices: vec![Voice { id: 5, index: 1, events: Arc::new(vec![note(10, Frac::ZERO, 0), rest(20, Frac::new(1, 4)), note(30, Frac::new(3, 4), 5)]), tuplets: vec![] }],
        }], breaks: vec![], cross: vec![], automation: vec![], tweaks: vec![],
    });
    p
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

#[test]
fn builtin_instrument_is_a_clap_plugin() {
    let lib = Library::builtin();
    let infos = lib.plugins();
    let ids: Vec<&str> = infos.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ids, ["org.corinzia.ensemble", "org.corinzia.gain", "org.corinzia.reverb", "org.corinzia.eq", "org.corinzia.limiter", "org.corinzia.compressor", "org.corinzia.gate"]);
    assert!(infos[0].is_instrument() && !infos[1].is_instrument());
    // Projects saved with Corinzia Sine get the ensemble.
    assert_eq!(Plugin::create(&lib, "org.corinzia.sine").unwrap().info.id, "org.corinzia.ensemble");
    assert!(Library::load(std::path::Path::new(host::OLD_BUILTIN)).is_ok());
    let p = Plugin::create(&lib, "org.corinzia.ensemble").unwrap();
    assert!(!p.midi_dialect && p.output_channels == 2);
    p.activate(SR, 256).unwrap();
    let state = p.save_state().unwrap();
    assert_eq!(&state[..4], b"CZE1");
    unsafe { corinzia_ensemble::set_gain(p.raw(), 0.25) };
    let changed = p.save_state().unwrap();
    assert_ne!(state, changed);
    let q = Plugin::create(&lib, "org.corinzia.ensemble").unwrap();
    assert!(q.load_state(&changed));
    assert_eq!(q.save_state().unwrap(), changed, "state round-trips");
    assert!(!q.load_state(b"junk"));
}

#[test]
fn loads_the_bundle_from_disk() {
    // The same plugin as a real .clap shared object, through dlopen.
    let target = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../target");
    let so = ["debug", "release"].iter().map(|d| target.join(d).join("libcorinzia_ensemble.so")).find(|p| p.exists());
    let Some(so) = so else {
        eprintln!("libcorinzia_ensemble.so not built; skipping");
        return;
    };
    let dir = std::env::temp_dir().join(format!("corinzia-clap-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("vendor")).unwrap();
    let bundle = dir.join("vendor/Corinzia Ensemble.clap");
    std::fs::copy(&so, &bundle).unwrap();
    std::fs::write(dir.join("not-a-plugin.clap"), b"nope").unwrap();
    assert_eq!(host::find_bundles(&[dir.clone()]).len(), 2);
    let (infos, errors) = host::scan(&[dir.clone()]);
    assert_eq!(infos.iter().filter(|i| i.id == "org.corinzia.ensemble").count(), 2, "built-in + bundle: {infos:?}");
    assert_eq!(errors.len(), 1, "{errors:?}");
    let lib = Library::load(&bundle).unwrap();
    let p = Plugin::create(&lib, "org.corinzia.ensemble").unwrap();
    p.activate(SR, 128).unwrap();
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn renders_the_score_sample_accurately() {
    let p = project();
    let mut pool = PluginPool::new(SR, 256);
    let snap = snapshot::build(&p, SR, 1, &[0], |t| pool.plugin_for(&p, t));
    assert_eq!(snap.nodes.len(), 1);
    let played: Vec<&snapshot::ScheduledNote> = snap.notes.iter().filter(|n| n.kind == snapshot::Kind::Note).collect();
    assert_eq!(played.len(), 4);
    let beat = (SR * 0.5) as i64; // 120 bpm
    assert_eq!(played[0].at, 0);
    // The track's program; without one, the flute's sound.
    assert_eq!(snap.nodes[0].program, Some((0, 79, None)));
    let mut flute = p.clone();
    flute.tracks[0].program = None;
    assert_eq!(snapshot::builtin_program(&flute, 1), Some((0, 73, Some(0))));
    // Its dynamic (f) as CC1, the normal technique.
    assert!(snap.notes.iter().any(|n| n.kind == snapshot::Kind::Cc(1) && (n.velocity - 0.75).abs() < 0.01));
    assert!(snap.notes.iter().any(|n| n.kind == snapshot::Kind::Cc(3) && n.velocity == 0.0));
    assert_eq!(snap.notes.iter().find(|n| n.on && n.key == 81).unwrap().at, 3 * beat);
    assert_eq!(snap.end(), 4 * beat);
    let timeline = snap.timeline();
    assert_eq!(timeline.locate(3 * beat), Some((9, 3.0)));
    assert_eq!(timeline.sample_of(9, 2.0), Some(2 * beat));

    let (mut ui, mut eng) = engine::new(SR, 256);
    ui.swap(snap);
    ui.play();
    let (l, r) = eng.render_offline((5 * beat) as usize);
    let b = beat as usize;
    assert!(rms(&l[1000..b / 2]) > 0.01, "c'' sounds");
    assert!(rms(&l[3 * b / 2..3 * b - 10]) < 1e-4, "rests are silent once the 150 ms release is over");
    assert!(rms(&l[3 * b + 1000..3 * b + 4000]) > 0.01, "a'' starts on beat 4");
    assert!(rms(&l[3 * b - 300..3 * b - 10]) < 1e-4, "not a moment before");
    assert_eq!(l, r, "centred");
    assert!(!ui.is_playing(), "stops at the end");
    assert_eq!(ui.position(), 4 * beat);
}

#[test]
fn transport_mix_and_swaps() {
    let p = project();
    let mut pool = PluginPool::new(SR, 256);
    let beat = (SR * 0.5) as usize;
    let (mut ui, mut eng) = engine::new(SR, 256);
    ui.swap(snapshot::build(&p, SR, 1, &[0], |t| pool.plugin_for(&p, t)));
    // Seek to beat 4 and play: only the a''.
    ui.seek(3 * beat as i64);
    ui.play();
    let (l, _) = eng.render_offline(beat / 2);
    assert!(rms(&l[500..]) > 0.01);
    // Stop releases the sounding note.
    ui.stop();
    let (l, _) = eng.render_offline(beat / 2);
    assert!(rms(&l[l.len() - 2000..]) < 1e-4, "released");
    assert!(!ui.is_playing());
    // Mute through the mixer: plays, but silently.
    ui.seek(0);
    ui.send(Command::Mix { node: 0, gain: 1.0, pan: 0.0, audible: false, direct: true }).ok();
    ui.play();
    // (Mixer changes ramp over one 256-frame block: no clicks.)
    let (l, _) = eng.render_offline(beat / 2);
    assert!(rms(&l[256..]) < 1e-6);
    // Hard left pan.
    ui.send(Command::Mix { node: 0, gain: 1.0, pan: -1.0, audible: true, direct: true }).ok();
    ui.seek(0);
    let (l, r) = eng.render_offline(beat / 4);
    assert!(rms(&l[256..]) > 0.01 && rms(&r[256..]) < 1e-6);
    // The ramp: the first block rises from silence instead of jumping.
    assert!(l[10].abs() < l[200].abs() + 1e-3);
    // An edit swaps in a new plan mid-play; the same plugin instance is kept
    // and the old plan comes back to the UI thread.
    let plugin = pool.plugin_of_track(1).unwrap().clone();
    ui.swap(snapshot::build(&p, SR, 2, &[0], |t| pool.plugin_for(&p, t)));
    eng.render_offline(256);
    ui.collect();
    assert!(Arc::ptr_eq(&plugin, pool.plugin_of_track(1).unwrap()));
    assert_eq!(ui.shared.generation.load(std::sync::atomic::Ordering::Acquire), 2);
    // Preview notes sound while stopped.
    ui.stop();
    eng.render_offline(beat);
    ui.send(Command::Preview { node: 0, key: 60, channel: 0, velocity: 0.8 }).ok();
    let (l, _) = eng.render_offline(2000);
    assert!(rms(&l[500..]) > 0.01);
    assert_eq!(ui.shared.dropped.load(std::sync::atomic::Ordering::Relaxed), 0);
}

#[test]
fn solo_and_gain() {
    let mut p = project();
    p.tracks[0].gain_db = -6.0;
    let m = snapshot::mix(&p);
    assert!((m[0].1 - 0.501).abs() < 0.01 && m[0].3);
    p.tracks.push(Track { id: 7, name: "Other".into(), instrument_node: None, note_port: 0, output_bus: None, midi_channel: 1, gain_db: 0.0, pan: 0.0, mute: false, solo: true, inserts: vec![], sends: vec![], dynamics_cc: None, program: None, bank: None, techniques: vec![], color: None, folder: String::new(), audio: false, clips: vec![] });
    let m = snapshot::mix(&p);
    assert!(!m[0].3 && m[1].3, "solo silences the others");
}

/// An effect node with its parameters as Corinzia's effects keep them.
fn effect(p: &mut Project, id: Id, plugin: &str, params: &[f64]) {
    p.nodes.push(AudioNode { id, kind: NodeKind::Plugin, name: plugin.into(), bundle: host::BUILTIN.into(), plugin_id: plugin.into(), bypassed: false });
    let mut state = b"CZF1".to_vec();
    for v in params {
        state.extend_from_slice(&v.to_le_bytes());
    }
    p.plugin_state.insert(id, Arc::new(state));
}

fn render(p: &Project, frames: usize) -> Vec<f32> {
    let mut pool = PluginPool::new(SR, 256);
    let (mut ui, mut eng) = engine::new(SR, 256);
    ui.swap(snapshot::build_from_pool(p, SR, 1, &[0], &mut pool));
    ui.play();
    eng.render_offline(frames).0
}

#[test]
fn inserts_buses_sends_and_the_master() {
    let beat = (SR * 0.5) as usize;
    let dry = render(&project(), 2 * beat);
    assert!(rms(&dry[1000..beat / 2]) > 0.01);

    // A gain insert at -6 dB halves the level.
    let mut p = project();
    effect(&mut p, 500, "org.corinzia.gain", &[-6.0, 0.0]);
    p.tracks[0].inserts = vec![500];
    let half = render(&p, 2 * beat);
    let ratio = rms(&half[1000..beat / 2]) / rms(&dry[1000..beat / 2]);
    assert!((ratio - 0.501).abs() < 0.01, "{ratio}");
    // Bypassed: as if not there.
    p.nodes[0].bypassed = true;
    assert!((rms(&render(&p, 2 * beat)[1000..beat / 2]) - rms(&dry[1000..beat / 2])).abs() < 1e-4);

    // Routed to a muted bus: silent; unmuted: heard.
    let mut p = project();
    p.buses.push(Bus { id: 60, name: "Strings".into(), inserts: vec![], gain_db: 0.0, pan: 0.0, mute: true, solo: false, output: None });
    p.tracks[0].output_bus = Some(60);
    assert!(rms(&render(&p, 2 * beat)[1000..beat / 2]) < 1e-6);
    p.buses[0].mute = false;
    assert!((rms(&render(&p, 2 * beat)[1000..beat / 2]) - rms(&dry[1000..beat / 2])).abs() < 1e-4);

    // A send to a reverb bus: a tail after the note's release.
    let mut p = project();
    effect(&mut p, 501, "org.corinzia.reverb", &[1.0, 0.8, 0.3]);
    p.buses.push(Bus { id: 61, name: "Reverb".into(), inserts: vec![501], gain_db: 0.0, pan: 0.0, mute: false, solo: false, output: None });
    p.tracks[0].sends = vec![SendLevel { bus: 61, gain_db: 0.0, pre: false }];
    let wet = render(&p, 2 * beat);
    let tail = 3 * beat / 2 - 2000..3 * beat / 2;
    assert!(rms(&dry[tail.clone()]) < 1e-5, "dry is silent there");
    assert!(rms(&wet[tail]) > 1e-4, "the reverb rings on");

    // The master fader at the bottom silences everything.
    p.master.gain_db = -100.0;
    assert!(rms(&render(&p, 2 * beat)[1000..beat]) < 1e-6);
}

#[test]
fn effect_parameters_set_while_idle_and_while_playing() {
    let lib = Library::builtin();
    let gain = Plugin::create(&lib, "org.corinzia.gain").unwrap();
    gain.activate(SR, 256).unwrap();
    let names: Vec<String> = gain.params().iter().map(|p| p.name.clone()).collect();
    assert_eq!(names, ["Gain", "Pan"]);
    assert_eq!(gain.param_value(0), Some(0.0));
    gain.set_param(0, -12.0); // not processing: flushed at once
    assert_eq!(gain.param_value(0), Some(-12.0));
    assert_eq!(gain.param_text(0, -12.0), "-12.0 dB");

    // While playing: the change reaches the plugin with the next block.
    let mut p = project();
    effect(&mut p, 500, "org.corinzia.gain", &[0.0, 0.0]);
    p.tracks[0].inserts = vec![500];
    let mut pool = PluginPool::new(SR, 256);
    let (mut ui, mut eng) = engine::new(SR, 256);
    ui.swap(snapshot::build_from_pool(&p, SR, 1, &[0], &mut pool));
    ui.play();
    let before = eng.render_offline(4096).0;
    assert!(rms(&before[1000..]) > 0.01);
    pool.effect(500).unwrap().set_param(0, -60.0);
    let _ = eng.render_offline(512); // the change, then the fade over a block
    let after = eng.render_offline(4096).0;
    assert!(rms(&after) < 1e-6, "{}", rms(&after));
    assert_eq!(pool.effect(500).unwrap().param_value(0), Some(-60.0));
}

#[test]
fn a_tracks_program_reaches_the_instrument_when_playing_starts() {
    let beat = (SR * 0.5) as usize;
    let mut f = project();
    f.tracks[0].program = None; // the flute's own sound
    let flute = render(&f, beat);
    let mut p = project();
    p.tracks[0].program = Some(56); // General MIDI: trumpet
    let trumpet = render(&p, beat);
    // The same note, louder and another sound.
    assert!(rms(&trumpet[2000..beat / 2]) > rms(&flute[2000..beat / 2]) * 1.5, "{} {}", rms(&trumpet[2000..beat / 2]), rms(&flute[2000..beat / 2]));
    let mut pool = PluginPool::new(SR, 256);
    let snap = snapshot::build_from_pool(&p, SR, 1, &[0], &mut pool);
    assert_eq!(snap.nodes[0].program, Some((0, 56, None)));
}

#[test]
fn automation_and_dynamics_as_a_controller() {
    let beat = (SR * 0.5) as usize;
    // A long note: c'' for the whole bar.
    let long = || {
        let mut p = project();
        let v = &mut p.movements[0].lanes[0].voices[0];
        let mut evs = (*v.events).clone();
        evs.truncate(1);
        evs[0].duration = Frac::int(1);
        v.events = Arc::new(evs);
        p
    };
    let flat = render(&long(), 4 * beat);
    // The fader from 0 dB to -60 dB (silence) over the bar.
    let mut p = long();
    p.movements[0].automation = vec![AutoLane { id: 90, target: AutoTarget::Gain(1), points: vec![AutoPoint { at: Frac::ZERO, value: 0.0, hold: false }, AutoPoint { at: Frac::int(1), value: -60.0, hold: false }] }];
    let fade = render(&p, 4 * beat);
    let early = rms(&fade[2000..beat / 2]) / rms(&flat[2000..beat / 2]);
    let late = rms(&fade[3 * beat..4 * beat - 1000]) / rms(&flat[3 * beat..4 * beat - 1000]);
    assert!(early > 0.5 && late < 0.1, "{early} {late}"); // about -3.5 dB early on (linear in dB)

    // An effect's parameter: the gain insert down 20 dB from beat 3.
    let mut p = long();
    effect(&mut p, 500, "org.corinzia.gain", &[0.0, 0.0]);
    p.tracks[0].inserts = vec![500];
    p.movements[0].automation = vec![AutoLane { id: 91, target: AutoTarget::Param { node: 500, param: 0 }, points: vec![AutoPoint { at: Frac::new(1, 2), value: 0.0, hold: false }, AutoPoint { at: Frac::new(1, 2), value: -20.0, hold: false }] }];
    let step = render(&p, 4 * beat);
    let before = rms(&step[beat..2 * beat - 500]) / rms(&flat[beat..2 * beat - 500]);
    let after = rms(&step[2 * beat + 2000..3 * beat]) / rms(&flat[2 * beat + 2000..3 * beat]);
    assert!((before - 1.0).abs() < 0.02 && (after - 0.1).abs() < 0.02, "{before} {after}");

    // Dynamics as CC11: the instrument follows expression, so p (CC11 ≈ velocity) is softer twice over.
    let mut p = long();
    p.movements[0].lanes[0].voices[0].events = Arc::new({
        let mut e = (*p.movements[0].lanes[0].voices[0].events).clone();
        e[0].dynamic = Some(2); // p
        e
    });
    let vel_only = render(&p, beat);
    p.tracks[0].dynamics_cc = Some(11);
    let with_cc = render(&p, beat);
    let r = rms(&with_cc[2000..beat]) / rms(&vel_only[2000..beat]);
    assert!(r < 0.7, "{r}");
    let mut pool = PluginPool::new(SR, 256);
    let snap = snapshot::build_from_pool(&p, SR, 1, &[0], &mut pool);
    assert!(snap.notes.iter().any(|n| n.kind == snapshot::Kind::Cc(11)));
}

#[test]
fn expression_maps_switch_sounds() {
    // c'' (beat 1) then "pizz." over a'' (beat 4).
    let mut p = project();
    p.movements[0].lanes[0].texts.push(TextItem { id: 70, at: Frac::new(1, 2), text: "pizz.".into(), kind: TextKind::Technique, shown: Shown::Both });
    p.tracks[0].techniques = vec![
        TechniqueSwitch { technique: "pizz.".into(), action: SwitchAction::Program(1) },
        TechniqueSwitch { technique: "ord.".into(), action: SwitchAction::Program(0) },
        TechniqueSwitch { technique: "staccato".into(), action: SwitchAction::Keyswitch(24) },
    ];
    let mut pool = PluginPool::new(SR, 256);
    let snap = snapshot::build_from_pool(&p, SR, 1, &[0], &mut pool);
    let programs: Vec<(i64, i16)> = snap.notes.iter().filter(|n| n.kind == snapshot::Kind::Program).map(|n| (n.at, n.key)).collect();
    let beat = (SR * 0.5) as i64;
    assert_eq!(programs, [(0, 0), (3 * beat - 3, 1)], "ord. from the start, pizz. just before beat 4");
    // A staccato note gets its keyswitch (and the next one goes back).
    let mut evs = (*p.movements[0].lanes[0].voices[0].events).clone();
    evs[0].articulations = corinzia_score::score::art::STACCATO;
    p.movements[0].lanes[0].voices[0].events = Arc::new(evs);
    let snap = snapshot::build_from_pool(&p, SR, 1, &[0], &mut pool);
    let keys: Vec<(i64, i16, bool)> = snap.notes.iter().filter(|n| n.note_id == -2).map(|n| (n.at, n.key, n.on)).collect();
    assert_eq!(keys, [(0, 24, true), (1, 24, false)]);
    assert!(snap.notes.iter().any(|n| n.kind == snapshot::Kind::Program && n.at == 3 * beat - 3 && n.key == 1));
    // In the MIDI file too.
    let doc = corinzia_perform::document(&p, &p.movements[0]);
    let t = &doc.performance.tracks[0];
    assert_eq!(t.programs.iter().map(|x| x.1).collect::<Vec<_>>(), [1]);
    assert!(t.notes.iter().any(|n| n.key == 24));
}

/// `n` copies of the flute, each panned, with a gain insert and a send to
/// a reverb bus (ids offset per copy).
fn ensemble(n: u64) -> Project {
    let one = project();
    let mut p = one.clone();
    p.tracks.clear();
    p.instruments.clear();
    p.players.clear();
    p.movements[0].lanes.clear();
    effect(&mut p, 900, "org.corinzia.reverb", &[1.0, 0.8, 0.3]);
    p.buses.push(Bus { id: 901, name: "Reverb".into(), inserts: vec![900], gain_db: 0.0, pan: 0.0, mute: false, solo: false, output: None });
    for i in 0..n {
        let o = 1000 + i * 100;
        let mut t = one.tracks[0].clone();
        (t.id, t.pan, t.inserts, t.sends) = (o + 1, i as f64 / n as f64 * 2.0 - 1.0, vec![o + 50], vec![SendLevel { bus: 901, gain_db: -6.0, pre: false }]);
        effect(&mut p, o + 50, "org.corinzia.gain", &[-(i as f64), 0.0]);
        let mut ins = one.instruments[0].clone();
        (ins.id, ins.player, ins.track) = (o + 2, o + 3, Some(o + 1));
        let mut pl = one.players[0].clone();
        (pl.id, pl.instruments) = (o + 3, vec![o + 2]);
        let mut lane = one.movements[0].lanes[0].clone();
        lane.player = o + 3;
        lane.changes[0].instrument = o + 2;
        lane.changes[0].id = o + 4;
        lane.voices[0].id = o + 5;
        lane.voices[0].events = Arc::new(lane.voices[0].events.iter().cloned().map(|mut e| {
            e.id += o;
            for h in &mut e.heads {
                h.id += o;
                h.pitch.octave = 3 + (i % 3) as i8;
            }
            e
        }).collect());
        p.tracks.push(t);
        p.instruments.push(ins);
        p.players.push(pl);
        p.movements[0].lanes.push(lane);
    }
    p
}

#[test]
fn rendering_on_worker_threads_matches_one_thread() {
    let p = ensemble(24);
    let frames = SR as usize * 2;
    let run = |workers| {
        let mut pool = PluginPool::new(SR, 256);
        let (mut ui, mut eng) = engine::with_workers(SR, 256, corinzia_audio::workers::Workers::new(workers));
        ui.swap(snapshot::build_from_pool(&p, SR, 1, &[0], &mut pool));
        ui.play();
        eng.render_offline(frames)
    };
    let one = run(0);
    assert!(rms(&one.0[1000..20000]) > 0.01);
    for w in [1, 3, 7] {
        assert!(run(w) == one, "{w} workers");
    }
}

#[test]
fn audio_clips_play_at_fixed_times_with_trims_fades_and_gain() {
    use corinzia_audio::media::AudioData;
    // The flute's movement is 2 seconds long (one bar of 4/4 at 120).
    let mut p = project();
    let mut clip = AudioClip::new(70, "Tape", "dc.wav", 9, 1.0);
    // 1.5 s of the file from 0.25 s in, 6 dB down, 0.1 s fades: it plays
    // from 1.0 s to 2.5 s, past the score's end.
    (clip.offset, clip.length, clip.gain_db, clip.fade_in, clip.fade_out) = (0.25, 1.5, -6.0206, 0.1, 0.1);
    p.tracks.push(Track { id: 50, name: "Tape".into(), instrument_node: None, note_port: 0, output_bus: None, midi_channel: 0, gain_db: 0.0, pan: 0.0, mute: false, solo: false, inserts: vec![], sends: vec![], dynamics_cc: None, program: None, bank: None, techniques: vec![], color: None, folder: String::new(), audio: true, clips: vec![clip] });
    p.tracks[0].mute = true; // just the clip
    let mut pool = PluginPool::new(SR, 256);
    // A file of constant 0.8, mono, with 0.5 from 0.25 s on (the trim is seen).
    let file: Vec<f32> = (0..(SR * 3.0) as usize).map(|i| if i < (SR * 0.25) as usize { 0.8 } else { 0.5 }).collect();
    pool.media.insert("dc.wav", AudioData::from_channels(vec![file], SR));
    let (mut ui, mut eng) = engine::new(SR, 256);
    let snap = snapshot::build_from_pool(&p, SR, 1, &[0], &mut pool);
    assert!(snap.clip_errors.is_empty(), "{:?}", snap.clip_errors);
    assert_eq!(snap.end(), (SR * 2.5) as i64, "playback runs to the clip's end");
    ui.swap(snap);
    ui.play();
    let (l, r) = eng.render_offline((SR * 3.0) as usize);
    let at = |s: f64| (SR * s) as usize;
    assert!(l[..at(0.99)].iter().all(|v| v.abs() < 1e-6), "silent before the clip");
    // Inside: 0.5 at -6 dB, panned centre (equal power: 0.5 * 0.5 * √2 · cos 45° = 0.25).
    assert!((l[at(1.5)] - 0.25).abs() < 1e-3, "{}", l[at(1.5)]);
    assert_eq!(l[at(1.5)], r[at(1.5)], "mono to both sides");
    // Fades: half way through the fade in, half the level.
    assert!((l[at(1.05)] - 0.125).abs() < 2e-3, "{}", l[at(1.05)]);
    assert!((l[at(2.45)] - 0.125).abs() < 2e-3, "{}", l[at(2.45)]);
    assert!(l[at(2.51)..].iter().all(|v| v.abs() < 1e-6), "silent after");
    // Seeking into the clip plays from there.
    ui.seek((SR * 2.0) as i64);
    ui.play();
    let (l, _) = eng.render_offline(at(0.1));
    assert!((l[at(0.05)] - 0.25).abs() < 1e-3);
}

#[test]
fn a_missing_clip_file_is_reported() {
    let mut p = project();
    p.tracks.push(Track { id: 50, name: "Tape".into(), instrument_node: None, note_port: 0, output_bus: None, midi_channel: 0, gain_db: 0.0, pan: 0.0, mute: false, solo: false, inserts: vec![], sends: vec![], dynamics_cc: None, program: None, bank: None, techniques: vec![], color: None, folder: String::new(), audio: true, clips: vec![AudioClip::new(70, "Gone", "/nowhere/gone.wav", 9, 0.0)] });
    let mut pool = PluginPool::new(SR, 256);
    let snap = snapshot::build_from_pool(&p, SR, 1, &[0], &mut pool);
    assert_eq!(snap.clip_errors.len(), 1);
    assert_eq!(snap.nodes.len(), 2, "the track still has its strip");
}

#[test]
fn pre_fader_sends_buses_into_buses_and_solo_through_buses() {
    let beat = (SR * 0.5) as usize;
    let level = |p: &Project| rms(&render(p, beat)[1000..beat / 2]);
    let dry = level(&project());
    let bus = |id, mute| Bus { id, name: format!("B{id}"), inserts: vec![], gain_db: 0.0, pan: 0.0, mute, solo: false, output: None };

    // The fader all the way down: a post-fader send is silent, a pre-fader one is not.
    let mut p = project();
    p.buses.push(bus(60, false));
    p.tracks[0].gain_db = -100.0;
    p.tracks[0].sends = vec![SendLevel { bus: 60, gain_db: 0.0, pre: false }];
    assert!(level(&p) < 1e-6);
    p.tracks[0].sends[0].pre = true;
    assert!(level(&p) > 0.5 * dry, "a pre-fader send ignores the fader");

    // A bus into another: the second's mute silences both.
    let mut p = project();
    p.buses = vec![bus(70, true), bus(71, false)];
    p.buses[1].output = Some(70); // listed after its destination: the plan reorders
    p.tracks[0].output_bus = Some(71);
    assert!(level(&p) < 1e-6);
    p.buses[0].mute = false;
    assert!((level(&p) - dry).abs() < 1e-4, "through both buses at unity");

    // Solo a reverb bus: only what the send feeds it is heard.
    let mut p = project();
    effect(&mut p, 501, "org.corinzia.gain", &[-6.0, 0.0]);
    p.buses = vec![Bus { inserts: vec![501], ..bus(80, false) }];
    p.tracks[0].sends = vec![SendLevel { bus: 80, gain_db: 0.0, pre: false }];
    let both = level(&p);
    p.buses[0].solo = true;
    let m = snapshot::mix(&p);
    assert!(m[0].3 && !m[0].4, "the track is heard, through the bus only");
    let solo = level(&p);
    assert!(solo > 0.1 * dry && solo < both - 0.1 * dry, "{solo} of {both}");
    // Solo the track instead: its bus stays heard.
    p.buses[0].solo = false;
    p.tracks[0].solo = true;
    assert!(snapshot::bus_mix(&p)[0].3);
    assert!((level(&p) - both).abs() < 1e-4);
}

#[test]
fn automation_of_buses_the_master_sends_controllers_and_steps() {
    let beat = (SR * 0.5) as usize;
    let lane = |id, target, points: &[(i64, f64, bool)]| AutoLane { id, target, points: points.iter().map(|&(q, value, hold)| AutoPoint { at: Frac::new(q, 4), value, hold }).collect() };
    let rms_at = |x: &[f32], q: f64| rms(&x[(q * beat as f64) as usize + 500..(q * beat as f64) as usize + 3000]);
    // The flute holds a long note: c'' for the whole bar.
    let mut p = project();
    p.movements[0].lanes[0].voices[0].events = Arc::new(vec![Event { duration: Frac::int(1), ..p.movements[0].lanes[0].voices[0].events[0].clone() }]);
    let base = render(&p, 4 * beat);

    // The master fader stepping down at beat 2 (held at 0 dB until then).
    p.movements[0].automation = vec![lane(90, AutoTarget::Gain(corinzia_model::MASTER), &[(0, 0.0, true), (2, -100.0, false)])];
    let x = render(&p, 4 * beat);
    assert!((rms_at(&x, 1.5) - rms_at(&base, 1.5)).abs() < 1e-3, "held at 0 dB just before the step");
    assert!(rms_at(&x, 2.5) < 1e-4, "silent after it");

    // A bus's fader, and a send's level.
    let mut p2 = p.clone();
    p2.buses = vec![Bus { id: 60, name: "B".into(), inserts: vec![], gain_db: 0.0, pan: 0.0, mute: false, solo: false, output: None }];
    p2.tracks[0].output_bus = Some(60);
    p2.movements[0].automation = vec![lane(91, AutoTarget::Gain(60), &[(0, 0.0, true), (2, -100.0, false)])];
    let x = render(&p2, 4 * beat);
    assert!(rms_at(&x, 1.5) > 0.5 * rms_at(&base, 1.5) && rms_at(&x, 2.5) < 1e-4, "the bus's fader");
    let mut p3 = p.clone();
    p3.buses = vec![Bus { id: 61, name: "R".into(), inserts: vec![], gain_db: 0.0, pan: 0.0, mute: false, solo: false, output: None }];
    p3.tracks[0].gain_db = -100.0; // only the (pre-fader) send is heard
    p3.tracks[0].sends = vec![SendLevel { bus: 61, gain_db: 0.0, pre: true }];
    p3.movements[0].automation = vec![lane(92, AutoTarget::Send { track: 1, bus: 61 }, &[(0, 0.0, true), (2, -100.0, false)])];
    let x = render(&p3, 4 * beat);
    assert!(rms_at(&x, 1.5) > 0.5 * rms_at(&base, 1.5) && rms_at(&x, 2.5) < 1e-4, "the send's level");

    // Controllers and pitch bend become MIDI events on the track's channel.
    let mut p4 = project();
    p4.tracks[0].midi_channel = 3;
    p4.movements[0].automation = vec![lane(93, AutoTarget::Cc { track: 1, cc: 21 }, &[(0, 127.0, false)]), lane(94, AutoTarget::PitchBend(1), &[(0, -1.0, false), (4, 1.0, false)])];
    let mut pool = PluginPool::new(SR, 256);
    let snap = snapshot::build_from_pool(&p4, SR, 1, &[0], &mut pool);
    let cc: Vec<_> = snap.notes.iter().filter(|n| n.kind == snapshot::Kind::Cc(21)).collect();
    assert!(cc.len() == 1 && cc[0].channel == 3 && (cc[0].velocity - 1.0).abs() < 1e-6);
    let bends: Vec<f32> = snap.notes.iter().filter(|n| n.kind == snapshot::Kind::PitchBend).map(|n| n.velocity).collect();
    assert!(bends.len() > 8 && bends[0] == -1.0 && *bends.last().unwrap() == 1.0, "a ramp from -1 to 1: {bends:?}");
}

#[test]
fn a_cycle_plays_round_until_it_is_turned_off() {
    // The flute: c'' on beat 1, a'' on beat 4 (1.5 s); a cycle over the first second.
    let p = project();
    let mut pool = PluginPool::new(SR, 256);
    let (mut ui, mut eng) = engine::new(SR, 256);
    ui.swap(snapshot::build_from_pool(&p, SR, 1, &[0], &mut pool));
    ui.send(Command::Loop(Some((0, SR as i64)))).ok();
    ui.play();
    let s = SR as usize;
    let (l, _) = eng.render_offline(3 * s);
    assert!(ui.is_playing(), "still going past the score's end (2 s)");
    assert!(ui.position() < SR as i64);
    // Each pass sounds like the first: the c'' again, never the a''.
    let first = rms(&l[1000..s / 4]);
    for pass in 1..3 {
        let x = rms(&l[pass * s + 1000..pass * s + s / 4]);
        assert!((x - first).abs() < 0.1 * first, "pass {pass}: {x} vs {first}");
        assert!(rms(&l[pass * s - s / 4..pass * s - 1000]) < 0.2 * first, "the rest before the cycle's end");
    }
    // Off: it plays on to the end and stops.
    ui.send(Command::Loop(None)).ok();
    eng.render_offline(3 * s);
    assert!(!ui.is_playing());
}

#[test]
fn a_movement_is_exported_as_a_wav_file_like_it_plays() {
    let dir = std::env::temp_dir().join(format!("corinzia-bounce-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("out.wav");
    let p = project();
    let mut b = corinzia_audio::bounce::Bounce::new(&p, 0, SR, 256, None, path.clone()).unwrap();
    assert!(b.warnings.is_empty());
    let mut steps = 0;
    while !b.step(0.5) {
        steps += 1;
        assert!(b.progress() < 1.0);
    }
    assert!(steps >= 8, "two seconds and the tail, half a second at a time");
    let summary = b.finish().unwrap();
    assert!((summary.seconds - (2.0 + corinzia_audio::bounce::TAIL_SECONDS)).abs() < 0.01);
    assert!(summary.peak_db < 0.0 && summary.peak_db > -40.0 && summary.clipped == 0, "{summary:?}");
    assert!(summary.lufs < 0.0 && summary.lufs > -60.0 && summary.true_peak_db >= summary.peak_db - 0.01, "{summary:?}");
    // The file reads back as what playback renders (to 24 bits).
    let (ch, rate) = corinzia_audio::media::decode(&path).unwrap();
    assert_eq!((ch.len(), rate), (2, SR as u32));
    let live = render(&p, SR as usize);
    let err = live.iter().zip(&ch[0]).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
    assert!(err < 1e-5, "{err}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn peaks_are_metered_per_track_and_at_the_master() {
    use corinzia_audio::engine::Meter;
    let mut p = project();
    let mut pool = PluginPool::new(SR, 256);
    let (mut ui, mut eng) = engine::new(SR, 256);
    ui.swap(snapshot::build_from_pool(&p, SR, 1, &[0], &mut pool));
    ui.play();
    let (l, _) = eng.render_offline(4096);
    let out = l.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let master = ui.shared.take_peak(Meter::Master);
    assert!((master.0 - out).abs() < 1e-6 && out > 0.01);
    let track = ui.shared.take_peak(Meter::Node(0));
    assert!((track.0 - out).abs() < 1e-3, "the only track is the mix: {track:?} vs {out}");
    assert_eq!(ui.shared.take_peak(Meter::Master), (0.0, 0.0), "taken: reset");
    // Its fader down 6 dB: its meter shows it.
    p.tracks[0].gain_db = -6.0206;
    ui.swap(snapshot::build_from_pool(&p, SR, 2, &[0], &mut pool));
    ui.seek(0);
    ui.play();
    eng.render_offline(2048);
    ui.shared.take_peak(Meter::Node(0));
    eng.render_offline(2048);
    let half = ui.shared.take_peak(Meter::Node(0));
    assert!((half.0 / out - 0.5).abs() < 0.05, "{half:?}");
}

#[test]
fn tracks_sharing_a_plugin_node_play_through_one_instance_on_their_channels() {
    // Two flutes, one Corinzia Ensemble node between them (channels 0 and 5).
    let mut p = ensemble(2);
    p.nodes.push(AudioNode { id: 700, kind: NodeKind::Plugin, name: "Sine".into(), bundle: host::BUILTIN.into(), plugin_id: "org.corinzia.sine".into(), bypassed: false });
    for (k, t) in p.tracks.iter_mut().enumerate() {
        (t.instrument_node, t.midi_channel, t.inserts, t.sends) = (Some(700), if k == 0 { 0 } else { 5 }, vec![], vec![]);
    }
    p.tracks[1].gain_db = -12.0;
    let mut pool = PluginPool::new(SR, 256);
    let snap = snapshot::build_from_pool(&p, SR, 1, &[0], &mut pool);
    assert_eq!(snap.nodes.len(), 2);
    assert!(snap.nodes[0].host.is_none() && snap.nodes[0].plugin.is_some());
    assert_eq!((snap.nodes[1].host, snap.nodes[1].channel), (Some(0), 5));
    assert_eq!(snap.nodes.iter().filter(|n| n.plugin.is_some()).count(), 1, "one instance");
    // Every note goes to the host's instance, each on its track's channel.
    let notes: Vec<_> = snap.notes.iter().filter(|n| n.kind == snapshot::Kind::Note).collect();
    assert!(notes.iter().all(|n| n.node == 0));
    assert!(notes.iter().any(|n| n.channel == 5) && notes.iter().any(|n| n.channel == 0));
    // It plays; the guest's fader is MIDI volume, its own strip silent.
    let (mut ui, mut eng) = engine::new(SR, 256);
    ui.swap(snap);
    ui.play();
    let (l, _) = eng.render_offline(SR as usize);
    assert!(rms(&l[1000..20000]) > 0.01);
    assert_eq!(ui.shared.take_peak(corinzia_audio::engine::Meter::Node(1)), (0.0, 0.0), "the guest has no audio of its own");
    // One track takes another plugin: the instance is no longer shared.
    p.nodes.push(AudioNode { id: 701, kind: NodeKind::Plugin, name: "Sine 2".into(), bundle: host::BUILTIN.into(), plugin_id: "org.corinzia.sine".into(), bypassed: false });
    p.tracks[1].instrument_node = Some(701);
    let snap = snapshot::build_from_pool(&p, SR, 2, &[0], &mut pool);
    assert!(snap.nodes[1].host.is_none() && snap.nodes.iter().filter(|n| n.plugin.is_some()).count() == 2);
}

/// The dominant frequency of a stretch of samples, by zero crossings.
fn frequency(x: &[f32]) -> f64 {
    let crossings = x.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
    crossings as f64 * SR / x.len() as f64
}

#[test]
fn tones_play_at_their_bent_pitch_with_a_drone_unbent() {
    let mut p = project();
    p.tracks[0].program = Some(79); // the built-in sine: one zero crossing a period
    let mut pool = PluginPool::new(SR, 256);
    // A' (69) bent up a semitone on channel 0 = 466.16 Hz; nothing else for
    // the first beat, then a drone on channel 1 joins, its own bend at 0.
    let tones = [
        snapshot::Tone { on: 0.0, off: 2.0, key: 69, bend: 0.5, channel: 0, velocity: 0.8, element: 1 },
        snapshot::Tone { on: 1.0, off: 2.0, key: 57, bend: 0.0, channel: 1, velocity: 0.8, element: 0 },
    ];
    let snap = snapshot::build_tones_from_pool(&p, SR, 1, 77, 60.0, 2.0, 1, &tones, &mut pool);
    assert_eq!(snap.sections.len(), 1);
    assert_eq!(snap.end(), (2.0 * SR) as i64);
    assert_eq!(snap.timeline().locate((SR * 1.5) as i64), Some((77, 1.5)));
    let bends: Vec<_> = snap.notes.iter().filter(|n| n.kind == snapshot::Kind::PitchBend).collect();
    assert_eq!(bends.len(), 2);
    let (mut ui, mut eng) = engine::new(SR, 256);
    ui.swap(snap);
    ui.play();
    let (l, _) = eng.render_offline((2.0 * SR) as usize);
    let f = frequency(&l[4800..43200]);
    assert!((f - 466.16).abs() < 3.0, "the bent A sounds as B flat: {f}");
}

#[test]
fn the_builtin_instruments_get_slurs_hairpins_and_techniques() {
    // The flute's c'' and a'' slurred, with a crescendo from c''; "pizz."
    // is not a flute's, but the controller goes the same way.
    let mut p = project();
    let mut evs = (*p.movements[0].lanes[0].voices[0].events).clone();
    evs[0].duration = Frac::new(3, 4); // c'' held to a''
    evs.remove(1); // no rest between
    evs[0].spans = corinzia_score::score::span::CRESC_START;
    evs[1].dynamic = Some(7);
    let (a, b) = (evs[0].id, evs[1].id);
    p.movements[0].lanes[0].voices[0].events = Arc::new(evs);
    p.movements[0].slurs.push(corinzia_score::score::Slur { id: 99, from: a, to: b, dir: 0 });
    p.movements[0].lanes[0].texts.push(TextItem { id: 70, at: Frac::new(3, 4), text: "pizz.".into(), kind: TextKind::Technique, shown: Shown::Both });
    let mut pool = PluginPool::new(SR, 256);
    let snap = snapshot::build_from_pool(&p, SR, 1, &[0], &mut pool);
    let beat = (SR * 0.5) as i64;
    let cc = |n: u8| snap.notes.iter().filter(|e| e.kind == snapshot::Kind::Cc(n)).map(|e| (e.at, e.velocity)).collect::<Vec<_>>();
    // Legato on after c'' starts, off after a'' starts.
    assert_eq!(cc(68), [(1, 1.0), (3 * beat + 1, 0.0)]);
    // c'' rises through the hairpin to fff: many CC1 steps, upwards.
    let dynamics = cc(1);
    assert!(dynamics.len() > 20 && dynamics.windows(2).all(|w| w[1].1 >= w[0].1 - 1e-6), "{dynamics:?}");
    assert!(dynamics.last().unwrap().1 > 0.95);
    // The technique just before a''.
    assert_eq!(cc(3), [(0, 0.0), (3 * beat - 4, 1.0 / 127.0)]);
    // c'' held into a'' so they join.
    let off = snap.notes.iter().find(|e| e.kind == snapshot::Kind::Note && !e.on && e.key == 72).unwrap().at;
    assert!(off > 3 * beat);
    // And it renders: one sound throughout, no gap at the join.
    let (mut ui, mut eng) = engine::new(SR, 256);
    ui.swap(snap);
    ui.play();
    let (l, _) = eng.render_offline((4 * beat) as usize);
    let b = beat as usize;
    assert!(rms(&l[3 * b - 2000..3 * b + 2000]) > 0.5 * rms(&l[2 * b..2 * b + 4000]), "joined");
}

/// Live-like rendering of a project file, block by block, timed:
/// CORINZIA_PROJECT=file CORINZIA_BLOCK=512 [CORINZIA_WAV=out.wav]
/// cargo test --release -p corinzia-audio --test engine live_blocks -- --ignored --nocapture
#[test]
#[ignore]
fn live_blocks() {
    let Ok(path) = std::env::var("CORINZIA_PROJECT") else { return };
    let block: usize = std::env::var("CORINZIA_BLOCK").ok().and_then(|b| b.parse().ok()).unwrap_or(512);
    let p = corinzia_model::store::load(std::path::Path::new(&path)).unwrap();
    let mut pool = PluginPool::new(SR, block as u32);
    let snap = snapshot::build_from_pool(&p, SR, 1, &[0], &mut pool);
    let total = snap.end() as usize + SR as usize;
    let workers = match std::env::var("CORINZIA_WORKERS").ok().and_then(|w| w.parse().ok()) {
        Some(n) => corinzia_audio::workers::Workers::new(n),
        None => corinzia_audio::workers::Workers::for_this_machine(),
    };
    let (mut ui, mut eng) = engine::with_workers(SR, block as u32, workers);
    ui.swap(snap);
    ui.play();
    let (mut l, mut r) = (vec![0.0f32; block], vec![0.0f32; block]);
    let mut out = Vec::with_capacity(total);
    let mut times = Vec::new();
    let mut at = 0;
    let start = std::time::Instant::now();
    while at < total {
        // CORINZIA_RT=1: the audio thread realtime (as PipeWire makes it)
        // from the second block on, the workers left as they were made
        // (normal priority, as without the right to raise them).
        if times.len() == 1 && std::env::var("CORINZIA_RT").is_ok() {
            #[cfg(target_os = "linux")]
            unsafe {
                let param = libc::sched_param { sched_priority: 50 };
                assert_eq!(libc::pthread_setschedparam(libc::pthread_self(), libc::SCHED_FIFO, &param), 0, "no right to realtime");
            }
        }
        // CORINZIA_RAGGED=1: each cycle asks for another number of frames
        // (as PipeWire's may), up to the block.
        let n = if std::env::var("CORINZIA_RAGGED").is_ok() { [block, block / 2, 37, block - 1, 1, block / 3 + 5][(at / 7 + times.len()) % 6].clamp(1, block) } else { block };
        let t = std::time::Instant::now();
        eng.process(&mut l[..n], &mut r[..n]);
        times.push(t.elapsed().as_secs_f64());
        // CORINZIA_PACE=1: in real time, as a sound card asks.
        if std::env::var("CORINZIA_PACE").is_ok() {
            let due = start + std::time::Duration::from_secs_f64((at + n) as f64 / SR);
            if let Some(wait) = due.checked_duration_since(std::time::Instant::now()) {
                std::thread::sleep(wait);
            }
        }
        out.extend_from_slice(&l[..n]);
        at += n;
    }
    let budget = block as f64 / SR;
    for (i, t) in times.iter().enumerate().filter(|(_, &t)| t > budget * 0.3) {
        eprintln!("  slow block {i} at {:.2} s: {:.2} ms", (i * block) as f64 / SR, t * 1e3);
    }
    times.sort_by(|a, b| a.total_cmp(b));
    let late = times.iter().filter(|&&t| t > budget).count();
    eprintln!(
        "block {block}: budget {:.2} ms, median {:.2} ms, p99 {:.2} ms, max {:.2} ms, over budget {late}/{}, dropped events {}",
        budget * 1e3,
        times[times.len() / 2] * 1e3,
        times[times.len() * 99 / 100] * 1e3,
        times.last().unwrap() * 1e3,
        times.len(),
        ui.shared.dropped.load(std::sync::atomic::Ordering::Relaxed)
    );
    if let Ok(wav) = std::env::var("CORINZIA_WAV") {
        let mut b = Vec::new();
        let data = (out.len() * 2) as u32;
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&[16, 0, 0, 0, 1, 0, 1, 0]);
        b.extend_from_slice(&(SR as u32).to_le_bytes());
        b.extend_from_slice(&(SR as u32 * 2).to_le_bytes());
        b.extend_from_slice(&[2, 0, 16, 0]);
        b.extend_from_slice(b"data");
        b.extend_from_slice(&data.to_le_bytes());
        for v in &out {
            b.extend_from_slice(&((v.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
        }
        std::fs::write(wav, b).unwrap();
    }
}

#[test]
fn the_engine_reports_its_load_and_late_blocks() {
    let p = project();
    let mut pool = PluginPool::new(SR, 256);
    let (mut ui, mut eng) = engine::new(SR, 256);
    ui.swap(snapshot::build_from_pool(&p, SR, 1, &[0], &mut pool));
    ui.play();
    eng.render_offline(SR as usize);
    let load = ui.take_load();
    assert!(load > 0.0 && load < 1.0, "{load}");
    assert_eq!(ui.take_load(), 0.0, "taken");
    assert_eq!(ui.late(), 0);
}

#[test]
fn notes_tell_the_builtin_instruments_how_they_speak() {
    use corinzia_ensemble::voice::articulation as a;
    use corinzia_score::score::art;
    // c'' staccato, then a'' marcato with a down-bow; then eight quick
    // sixteenths at 120 (an eighth of a second each): spiccato.
    let mut p = project();
    p.tracks[0].program = None;
    let mut evs = (*p.movements[0].lanes[0].voices[0].events).clone();
    evs[0].articulations = art::STACCATO;
    evs[2].articulations = art::MARCATO | art::DOWN_BOW;
    p.movements[0].lanes[0].voices[0].events = Arc::new(evs);
    let mut pool = PluginPool::new(SR, 256);
    let snap = snapshot::build_from_pool(&p, SR, 1, &[0], &mut pool);
    let codes: Vec<u8> = snap.notes.iter().filter(|e| e.kind == snapshot::Kind::Cc(9)).map(|e| (e.velocity * 127.0).round() as u8).collect();
    assert_eq!(codes, [a::STACCATO, a::MARCATO | a::DOWN_BOW]);
    let mut q = project();
    q.tracks[0].program = None;
    let sixteenth = |id: u64, k: i64| Event { id, kind: EventKind::Note, start: Frac::new(k, 16), duration: Frac::new(1, 16), heads: vec![Head { id: id + 1, pitch: Pitch { step: 0, alter: 0, octave: 5 }, tie: false, velocity: 0 }], articulations: 0, dynamic: None, spans: 0, lyrics: vec![], graces: vec![], fingering: vec![] };
    q.movements[0].lanes[0].voices[0].events = Arc::new((0..8).map(|k| sixteenth(100 + 2 * k as u64, k)).collect());
    let snap = snapshot::build_from_pool(&q, SR, 1, &[0], &mut pool);
    let codes: Vec<u8> = snap.notes.iter().filter(|e| e.kind == snapshot::Kind::Cc(9)).map(|e| (e.velocity * 127.0).round() as u8).collect();
    assert_eq!(codes, [a::SPICCATO], "quick unslurred notes: off the string");
}

#[test]
fn a_tremolo_is_one_note_in_the_tremolo_technique_on_the_builtin_instruments() {
    use corinzia_score::score::art;
    let mut p = project();
    p.tracks[0].program = None;
    let mut evs = (*p.movements[0].lanes[0].voices[0].events).clone();
    evs[0].articulations = art::TREMOLO; // c'' a quarter: eight 32nds written out
    p.movements[0].lanes[0].voices[0].events = Arc::new(evs);
    let mut pool = PluginPool::new(SR, 256);
    let snap = snapshot::build_from_pool(&p, SR, 1, &[0], &mut pool);
    let ons: Vec<i16> = snap.notes.iter().filter(|e| e.kind == snapshot::Kind::Note && e.on).map(|e| e.key).collect();
    assert_eq!(ons, [72, 81], "one c'', one a''");
    assert!(snap.notes.iter().any(|e| e.kind == snapshot::Kind::Cc(3) && (e.velocity * 127.0).round() as u8 == corinzia_ensemble::voice::tech::TREM));
    // Exported MIDI keeps the written-out notes.
    let doc = corinzia_perform::document(&p, &p.movements[0]);
    assert!(doc.performance.tracks[0].notes.iter().filter(|n| n.key == 72).count() >= 8);
}

#[test]
fn the_metronome_clicks_on_the_beats_and_counts_in() {
    // The flute muted: only the clicks. 120 bpm, 4/4: a beat is 24 000 samples.
    let mut p = project();
    p.tracks[0].mute = true;
    let beat = (SR * 0.5) as usize;
    let mut pool = PluginPool::new(SR, 256);
    let (mut ui, mut eng) = engine::new(SR, 256);
    ui.swap(snapshot::build_from_pool(&p, SR, 1, &[0], &mut pool));
    ui.metronome(true, 0);
    ui.play();
    let (l, _) = eng.render_offline(2 * beat);
    let at = |x: &[f32], k: usize| rms(&x[k * beat..k * beat + 400]);
    assert!(at(&l, 0) > 0.05 && at(&l, 1) > 0.05, "a click on beats 1 and 2: {} {}", at(&l, 0), at(&l, 1));
    assert!(rms(&l[beat / 4..3 * beat / 4]) < 1e-4, "silence between");
    assert!(at(&l, 0) > at(&l, 1) * 1.2, "the downbeat louder");
    // Counted in: a bar of clicks (4 beats) with the transport still, then playing.
    let mut p = project();
    p.tracks[0].mute = false;
    let (mut ui, mut eng) = engine::new(SR, 256);
    ui.swap(snapshot::build_from_pool(&p, SR, 1, &[0], &mut pool));
    ui.metronome(false, 1);
    ui.play();
    let (l, _) = eng.render_offline(4 * beat - 1000);
    assert_eq!(ui.position(), 0, "not moving while counting in");
    assert!((0..4).all(|k| at(&l, k) > 0.05), "four clicks");
    let (l, _) = eng.render_offline(beat);
    assert!(ui.is_playing() && ui.position() > 0);
    assert!(rms(&l[2000..]) > 0.01, "then the music (c''): {} at {}", rms(&l[2000..]), ui.position());
}
