use corinzia_audio::engine::{self, Command};
use corinzia_audio::host::{self, Library, Plugin};
use corinzia_audio::pool::PluginPool;
use corinzia_audio::snapshot;
use corinzia_model::*;
use corinzia_score::Frac;
use std::sync::Arc;

const SR: f64 = 48000.0;

/// One flute, 4/4 at 120 bpm: c'' on beat 1 (a quarter), rest, rest, a'' on beat 4.
fn project() -> Project {
    let mut p = Project { title: "T".into(), sample_rate: SR, next_id: 1000, ..Default::default() };
    p.tracks.push(Track { id: 1, name: "Flute".into(), instrument_node: None, note_port: 0, output_bus: None, midi_channel: 0, gain_db: 0.0, pan: 0.0, mute: false, solo: false, inserts: vec![], sends: vec![], program: None, bank: None });
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
        tempo: vec![Tempo { at: Frac::ZERO, bpm: 120.0, ramp: false, text: String::new() }],
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
        }], breaks: vec![], cross: vec![],
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
    assert_eq!(ids, ["org.corinzia.sine", "org.corinzia.gain", "org.corinzia.reverb"]);
    assert!(infos[0].is_instrument() && !infos[1].is_instrument());
    let p = Plugin::create(&lib, "org.corinzia.sine").unwrap();
    assert!(!p.midi_dialect && p.output_channels == 2);
    p.activate(SR, 256).unwrap();
    let state = p.save_state().unwrap();
    assert_eq!(&state[..4], b"CZS1");
    unsafe { corinzia_sine::set_gain(p.raw(), 0.25) };
    let changed = p.save_state().unwrap();
    assert_ne!(state, changed);
    let q = Plugin::create(&lib, "org.corinzia.sine").unwrap();
    assert!(q.load_state(&changed));
    assert_eq!(q.save_state().unwrap(), changed, "state round-trips");
    assert!(!q.load_state(b"junk"));
}

#[test]
fn loads_the_bundle_from_disk() {
    // The same plugin as a real .clap shared object, through dlopen.
    let target = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../target");
    let so = ["debug", "release"].iter().map(|d| target.join(d).join("libcorinzia_sine.so")).find(|p| p.exists());
    let Some(so) = so else {
        eprintln!("libcorinzia_sine.so not built; skipping");
        return;
    };
    let dir = std::env::temp_dir().join(format!("corinzia-clap-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("vendor")).unwrap();
    let bundle = dir.join("vendor/Corinzia Sine.clap");
    std::fs::copy(&so, &bundle).unwrap();
    std::fs::write(dir.join("not-a-plugin.clap"), b"nope").unwrap();
    assert_eq!(host::find_bundles(&[dir.clone()]).len(), 2);
    let (infos, errors) = host::scan(&[dir.clone()]);
    assert_eq!(infos.iter().filter(|i| i.id == "org.corinzia.sine").count(), 2, "built-in + bundle: {infos:?}");
    assert_eq!(errors.len(), 1, "{errors:?}");
    let lib = Library::load(&bundle).unwrap();
    let p = Plugin::create(&lib, "org.corinzia.sine").unwrap();
    p.activate(SR, 128).unwrap();
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn renders_the_score_sample_accurately() {
    let p = project();
    let mut pool = PluginPool::new(SR, 256);
    let snap = snapshot::build(&p, SR, 1, &[0], |t| pool.plugin_for(&p, t));
    assert_eq!(snap.nodes.len(), 1);
    assert_eq!(snap.notes.len(), 4);
    let beat = (SR * 0.5) as i64; // 120 bpm
    assert_eq!(snap.notes[0].at, 0);
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
    ui.send(Command::Mix { node: 0, gain: 1.0, pan: 0.0, audible: false }).ok();
    ui.play();
    // (Mixer changes ramp over one 256-frame block: no clicks.)
    let (l, _) = eng.render_offline(beat / 2);
    assert!(rms(&l[256..]) < 1e-6);
    // Hard left pan.
    ui.send(Command::Mix { node: 0, gain: 1.0, pan: -1.0, audible: true }).ok();
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
    p.tracks.push(Track { id: 7, name: "Other".into(), instrument_node: None, note_port: 0, output_bus: None, midi_channel: 1, gain_db: 0.0, pan: 0.0, mute: false, solo: true, inserts: vec![], sends: vec![], program: None, bank: None });
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
    p.buses.push(Bus { id: 60, name: "Strings".into(), inserts: vec![], gain_db: 0.0, pan: 0.0, mute: true });
    p.tracks[0].output_bus = Some(60);
    assert!(rms(&render(&p, 2 * beat)[1000..beat / 2]) < 1e-6);
    p.buses[0].mute = false;
    assert!((rms(&render(&p, 2 * beat)[1000..beat / 2]) - rms(&dry[1000..beat / 2])).abs() < 1e-4);

    // A send to a reverb bus: a tail after the note's release.
    let mut p = project();
    effect(&mut p, 501, "org.corinzia.reverb", &[1.0, 0.8, 0.3]);
    p.buses.push(Bus { id: 61, name: "Reverb".into(), inserts: vec![501], gain_db: 0.0, pan: 0.0, mute: false });
    p.tracks[0].sends = vec![SendLevel { bus: 61, gain_db: 0.0 }];
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
    let organ = render(&project(), beat);
    let mut p = project();
    p.tracks[0].program = Some(1); // Corinzia Sine: pure sine
    let pure = render(&p, beat);
    // The same note, without the octave partial: a different waveform, less energy.
    assert!(rms(&pure[1000..beat / 2]) < rms(&organ[1000..beat / 2]) * 0.995, "{} {}", rms(&pure[1000..beat / 2]), rms(&organ[1000..beat / 2]));
    let mut pool = PluginPool::new(SR, 256);
    let snap = snapshot::build_from_pool(&p, SR, 1, &[0], &mut pool);
    assert_eq!(snap.nodes[0].program, Some((0, 1, None)));
}
