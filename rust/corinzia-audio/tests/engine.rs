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
    p.tracks.push(Track { id: 1, name: "Flute".into(), instrument_node: None, note_port: 0, output_bus: None, midi_channel: 0, gain_db: 0.0, pan: 0.0, mute: false, solo: false });
    p.instruments.push(Instrument { id: 2, player: 3, name: "Flute".into(), engraver_name: "Fl.".into(), transposition: corinzia_model::Interval::default(), clef: "treble".into(), track: Some(1) });
    p.players.push(Player { id: 3, name: "Flute".into(), instruments: vec![2] });
    let note = |id, start: Frac, step| Event { id, kind: EventKind::Note, start, duration: Frac::new(1, 4), heads: vec![Head { id: id + 1, pitch: Pitch { step, alter: 0, octave: 5 }, tie: false, velocity: 0 }], articulations: 0, dynamic: Some(5), spans: 0, lyrics: vec![], graces: vec![] };
    let rest = |id, start: Frac| Event { id, kind: EventKind::Rest, start, duration: Frac::new(1, 2), heads: vec![], articulations: 0, dynamic: None, spans: 0, lyrics: vec![], graces: vec![] };
    p.movements.push(Movement {
        id: 9,
        ordinal: 0,
        title: String::new(),
        length: Frac::int(1),
        revision: 0,
        tempo: vec![Tempo { at: Frac::ZERO, bpm: 120.0, ramp: false, text: String::new() }],
        meter: vec![Meter { at: Frac::ZERO, num: 4, den: 4, pickup: false }],
        keys: vec![Key { at: Frac::ZERO, fifths: 0, minor: false }],
        barlines: vec![],
        rehearsal: vec![],
        navigation: vec![],
        endings: vec![],
        lanes: vec![Lane {
            player: 3,
            changes: vec![InstrumentChange { id: 4, at: Frac::ZERO, instrument: 2 }],
            clefs: vec![],
            texts: vec![],
            chords: vec![],
            lines: vec![],
            voices: vec![Voice { id: 5, index: 1, events: Arc::new(vec![note(10, Frac::ZERO, 0), rest(20, Frac::new(1, 4)), note(30, Frac::new(3, 4), 5)]), tuplets: vec![] }],
        }],
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
    assert_eq!(infos.len(), 1);
    assert_eq!(infos[0].id, "org.corinzia.sine");
    assert!(infos[0].is_instrument());
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
    let (l, _) = eng.render_offline(beat / 2);
    assert!(rms(&l) < 1e-6);
    // Hard left pan.
    ui.send(Command::Mix { node: 0, gain: 1.0, pan: -1.0, audible: true }).ok();
    ui.seek(0);
    let (l, r) = eng.render_offline(beat / 4);
    assert!(rms(&l) > 0.01 && rms(&r) < 1e-6);
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
    p.tracks.push(Track { id: 7, name: "Other".into(), instrument_node: None, note_port: 0, output_bus: None, midi_channel: 1, gain_db: 0.0, pan: 0.0, mute: false, solo: true });
    let m = snapshot::mix(&p);
    assert!(!m[0].3 && m[1].3, "solo silences the others");
}
