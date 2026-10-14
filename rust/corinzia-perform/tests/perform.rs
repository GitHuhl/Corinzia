//! Ported from tests/test_engine.cpp (compile_*), plus MIDI/MusicXML export
//! straight from the Rust model.

use corinzia_edit::{make_event, Ids};
use corinzia_model::*;
use corinzia_perform::*;
use corinzia_score::score::{art, span};
use corinzia_score::Frac;
use std::sync::Arc;

type F = Frac;
fn f(n: i64, d: i64) -> F {
    F::new(n, d)
}
fn p(step: i8, alter: i8, octave: i8) -> Pitch {
    Pitch { step, alter, octave }
}

fn movement(length: F, events: Vec<Event>) -> Movement {
    Movement {
        id: 1,
        ordinal: 0,
        title: String::new(),
        length,
        revision: 0,
        tempo: vec![Tempo { at: F::ZERO, bpm: 120.0, ramp: false, text: String::new() }],
        meter: vec![Meter { at: F::ZERO, num: 4, den: 4, pickup: false }],
        keys: vec![Key { at: F::ZERO, fifths: 0, minor: false }],
        barlines: vec![],
        rehearsal: vec![],
        navigation: vec![],
        endings: vec![],
        lanes: vec![Lane {
            player: 2,
            changes: vec![InstrumentChange { id: 3, at: F::ZERO, instrument: 4 }],
            clefs: vec![],
            texts: vec![],
            chords: vec![],
            voices: vec![Voice { id: 5, index: 1, events: Arc::new(events), tuplets: vec![] }],
        }],
    }
}

/// c'4 d'4 <e' g'>2 | b''4 a''4-. r2   (the fixture)
fn fixture() -> Movement {
    let mut ids = Ids(100);
    let mut a = make_event(&mut ids, f(1, 1) + f(1, 4), f(1, 4), &[p(5, 0, 5)], art::STACCATO);
    a.articulations = art::STACCATO;
    let evs = vec![
        make_event(&mut ids, F::ZERO, f(1, 4), &[p(0, 0, 4)], 0),
        make_event(&mut ids, f(1, 4), f(1, 4), &[p(1, 0, 4)], 0),
        make_event(&mut ids, f(1, 2), f(1, 2), &[p(2, 0, 4), p(4, 0, 4)], 0),
        make_event(&mut ids, f(1, 1), f(1, 4), &[p(6, 0, 5)], 0),
        a,
        make_event(&mut ids, f(3, 2), f(1, 2), &[], 0),
    ];
    movement(F::int(2), evs)
}

#[test]
fn hairpins_and_slurs_shape_playback() {
    let mut ids = Ids(1);
    let mut v: Vec<Event> = (0..5).map(|i| make_event(&mut ids, f(i, 4), f(1, 4), &[p(i as i8, 0, 4)], 0)).collect();
    v[0].dynamic = Some(2);
    v[0].spans = span::CRESC_START | span::SLUR_START;
    v[3].spans = span::SLUR_END;
    v[4].dynamic = Some(5);
    let s = expressive_shape(&v);
    assert!((s[0].velocity - velocity_for_dynamic(Some(2))).abs() < 1e-6);
    assert!((s[4].velocity - velocity_for_dynamic(Some(5))).abs() < 1e-6);
    assert!(s[1].velocity > s[0].velocity && s[2].velocity > s[1].velocity && s[3].velocity > s[2].velocity);
    assert!((s[2].velocity - (velocity_for_dynamic(Some(2)) + velocity_for_dynamic(Some(5))) / 2.0).abs() < 1e-5);
    assert!(s[0].legato && s[2].legato && !s[3].legato && !s[4].legato);
    let mut d = v.clone();
    for e in d.iter_mut() {
        e.spans = 0;
        e.dynamic = None;
    }
    d[0].dynamic = Some(5);
    d[0].spans = span::DIM_START;
    d[2].spans = span::HAIRPIN_END;
    let ds = expressive_shape(&d);
    assert!((ds[2].velocity - velocity_for_dynamic(Some(3))).abs() < 1e-6);
    assert!((ds[4].velocity - velocity_for_dynamic(Some(3))).abs() < 1e-6);
}

#[test]
fn unrolls_repeats() {
    let mut m = fixture();
    assert_eq!(playback_order(&m), vec![(F::ZERO, F::int(2))]);
    m.barlines = vec![Barline { id: 1, at: F::int(1), kind: BarKind::RepeatEnd }];
    assert_eq!(playback_order(&m), vec![(F::ZERO, F::int(1)), (F::ZERO, F::int(2))]);
    m.barlines = vec![Barline { id: 1, at: F::int(1), kind: BarKind::RepeatStart }, Barline { id: 2, at: F::int(2), kind: BarKind::RepeatEnd }];
    assert_eq!(playback_order(&m), vec![(F::ZERO, F::int(2)), (F::int(1), F::int(2))]);
    let perf = perform(&m);
    assert!((perf.length_q - 12.0).abs() < 1e-9);
    // Bar 2 twice: the a'' note-on at q5 and again at q9.
    let ons: Vec<f64> = perf.notes.iter().filter(|n| n.key == 81).map(|n| n.on_q).collect();
    assert_eq!(ons, vec![5.0, 9.0]);
    assert!(perf.notes.iter().filter(|n| n.key == 81).all(|n| n.gate == 0.5), "staccato");
    assert!((perf.written_quarter(9.0) - 5.0).abs() < 1e-9);
    assert!((perf.written_quarter(3.0) - 3.0).abs() < 1e-9);
}

#[test]
fn ornament_figures() {
    let c5 = p(0, 0, 5);
    let fig = ornament_figure(c5, f(1, 4), 0, 0);
    assert!(fig.len() == 1 && fig[0].length == f(1, 4));
    let fig = ornament_figure(c5, f(1, 4), art::TRILL, 0);
    assert!(fig.len() == 7 && fig[0].pitch.step == 0 && fig[1].pitch.step == 1 && fig.last().unwrap().pitch.step == 0);
    let mut sum = F::ZERO;
    for n in &fig {
        assert_eq!(n.offset, sum);
        sum = sum + n.length;
    }
    assert_eq!(sum, f(1, 4));
    let fig = ornament_figure(p(2, -1, 5), f(1, 4), art::TRILL, -3);
    assert!(fig[1].pitch.step == 3 && fig[1].pitch.alter == 0);
    let fig = ornament_figure(p(1, 0, 5), f(1, 2), art::TURN, 2);
    assert!(fig.len() == 4 && fig[0].pitch.step == 2 && fig[2].pitch.step == 0 && fig[2].pitch.alter == 1 && fig[3].length == f(13, 32));
    let fig = ornament_figure(c5, f(1, 8), art::MORDENT, 0);
    assert!(fig.len() == 3 && fig[1].pitch.step == 6 && fig[1].pitch.octave == 4 && fig[1].length == f(1, 32));
    assert_eq!(ornament_figure(c5, f(1, 16), art::MORDENT, 0)[0].length, f(1, 64));
    let fig = ornament_figure(c5, f(1, 4), art::TREMOLO, 0);
    assert!(fig.len() == 8 && fig[5].pitch.midi_key() == c5.midi_key());
}

#[test]
fn plays_ornaments_and_rolls_chords() {
    let mut m = fixture();
    let mut evs = (*m.lanes[0].voices[0].events).clone();
    evs[2].articulations = art::ARPEGGIO;
    evs[3].articulations = art::TRILL;
    m.lanes[0].voices[0].events = Arc::new(evs);
    let perf = perform(&m);
    let e = perf.notes.iter().find(|n| n.key == 64).unwrap();
    let g = perf.notes.iter().find(|n| n.key == 67).unwrap();
    assert!(e.on_q == 2.0 && e.roll_sec == 0.0 && g.on_q == 2.0 && (g.roll_sec - 0.03).abs() < 1e-9);
    assert_eq!(perf.notes.iter().filter(|n| n.on_q >= 4.0 && n.on_q < 5.0).count(), 7);
}

#[test]
fn ties_merge() {
    let mut ids = Ids(1);
    let mut a = make_event(&mut ids, F::ZERO, f(1, 2), &[p(0, 0, 4)], 0);
    a.heads[0].tie = true;
    let b = make_event(&mut ids, f(1, 2), f(1, 2), &[p(0, 0, 4)], art::STACCATO);
    let perf = perform(&movement(F::int(1), vec![a, b]));
    assert_eq!(perf.notes.len(), 1);
    assert!((perf.notes[0].full_off_q - 4.0).abs() < 1e-9 && perf.notes[0].gate == 0.5, "the last note's articulation decides");
}

#[test]
fn playback_order_with_endings() {
    let mut m = movement(F::int(6), vec![]);
    let bar = |m: &mut Movement, at: i64, kind| {
        let id = 900 + m.barlines.len() as u64;
        m.barlines.push(Barline { id, at: F::int(at), kind })
    };
    let end = |m: &mut Movement, from: i64, to: i64, passes| {
        let id = 950 + m.endings.len() as u64;
        m.endings.push(Ending { id, from: F::int(from), to: F::int(to), passes })
    };
    let o = |v: &[(i64, i64)]| v.iter().map(|&(a, b)| (F::int(a), F::int(b))).collect::<Vec<_>>();
    use BarKind::*;
    bar(&mut m, 3, RepeatEnd);
    end(&mut m, 2, 3, 1);
    end(&mut m, 3, 4, 2);
    assert_eq!(playback_order(&m), o(&[(0, 3), (0, 2), (3, 6)]));
    bar(&mut m, 1, RepeatStart);
    assert_eq!(playback_order(&m), o(&[(0, 3), (1, 2), (3, 6)]));
    m.barlines.clear();
    m.endings.clear();
    bar(&mut m, 1, RepeatStart);
    bar(&mut m, 3, RepeatEnd);
    bar(&mut m, 4, RepeatEnd);
    end(&mut m, 2, 3, 1);
    end(&mut m, 3, 4, 2);
    end(&mut m, 4, 5, 4);
    assert_eq!(playback_order(&m), o(&[(0, 3), (1, 2), (3, 4), (1, 2), (4, 6)]));
    m.barlines.clear();
    m.endings.clear();
    bar(&mut m, 1, RepeatStart);
    bar(&mut m, 3, RepeatEnd);
    end(&mut m, 2, 3, 3);
    end(&mut m, 3, 4, 4);
    assert_eq!(playback_order(&m), o(&[(0, 3), (1, 3), (1, 2), (3, 6)]));
    m.barlines.clear();
    m.endings.clear();
    bar(&mut m, 2, RepeatEnd);
    end(&mut m, 1, 2, 1);
    end(&mut m, 2, 3, 2);
    bar(&mut m, 6, RepeatEnd);
    assert_eq!(playback_order(&m), o(&[(0, 2), (0, 1), (2, 6), (3, 6)]));
}

#[test]
fn exports_midi_and_musicxml_from_the_model() {
    let m = fixture();
    let project = Project {
        title: "T".into(),
        players: vec![Player { id: 2, name: "P".into(), instruments: vec![4] }],
        instruments: vec![Instrument { id: 4, player: 2, name: "Flute".into(), engraver_name: "Fl.".into(), transposition: corinzia_model::Interval::default(), clef: "treble".into(), track: Some(6) }],
        tracks: vec![Track { id: 6, name: "Flute".into(), instrument_node: None, note_port: 0, output_bus: None, midi_channel: 0, gain_db: 0.0, pan: 0.0, mute: false, solo: false }],
        movements: vec![m.clone()],
        ..Default::default()
    };
    let doc = document(&project, &m);
    assert_eq!(doc.performance.tracks.len(), 1);
    assert_eq!(doc.performance.tracks[0].notes.len(), 6); // c d e g b a
    let bytes = midi(&project, &m).unwrap();
    assert_eq!(&bytes[..4], b"MThd");
    let xml = musicxml(&project, &m).unwrap();
    assert!(xml.contains("<score-partwise") && xml.contains("Flute"));
}

#[test]
fn grace_notes_play_on_the_beat_and_take_their_time_from_the_note() {
    let mut ids = Ids(100);
    let mut main = make_event(&mut ids, F::ZERO, f(1, 4), &[p(0, 0, 5)], 0);
    main.graces = vec![
        Grace { id: 900, heads: vec![Head { id: 901, pitch: p(1, 0, 5), tie: false, velocity: 0 }], base: 16, slash: false },
        Grace { id: 902, heads: vec![Head { id: 903, pitch: p(2, 0, 5), tie: false, velocity: 0 }], base: 16, slash: false },
    ];
    let m = movement(F::int(1), vec![main, make_event(&mut ids, f(1, 4), f(3, 4), &[], 0)]);
    let perf = perform(&m);
    let mut ns: Vec<(i32, f64, f64)> = perf.notes.iter().map(|n| (n.key, n.on_q, n.full_off_q)).collect();
    ns.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    // D and E, a 32nd (1/8 quarter) each from the beat; C after them.
    assert_eq!(ns.len(), 3);
    assert_eq!((ns[0].0, ns[0].1), (74, 0.0));
    assert_eq!((ns[1].0, ns[1].1), (76, 0.125));
    assert_eq!((ns[2].0, ns[2].1, ns[2].2), (72, 0.25, 1.0));
}

#[test]
fn da_capo_dal_segno_fine_and_coda() {
    let bars = |m: &Movement| playback_order(m).iter().map(|&(a, b)| (a.num / a.den, b.num / b.den)).collect::<Vec<_>>();
    let mut m = movement(F::int(6), vec![]);
    // Six bars; D.C. al Fine at the end of bar 6, Fine at the end of bar 2.
    m.navigation = vec![NavMark { id: 1, at: F::int(2), kind: Nav::Fine }, NavMark { id: 2, at: F::int(6), kind: Nav::DaCapo(NavTo::Fine) }];
    assert_eq!(bars(&m), [(0, 6), (0, 2)]);
    // D.S. al Coda: segno at bar 2, To Coda at the end of bar 3, coda from bar 5; the jump at bar 5's start.
    m.navigation = vec![
        NavMark { id: 1, at: F::int(1), kind: Nav::Segno },
        NavMark { id: 2, at: F::int(3), kind: Nav::ToCoda },
        NavMark { id: 3, at: F::int(4), kind: Nav::DalSegno(NavTo::Coda) },
        NavMark { id: 4, at: F::int(4), kind: Nav::Coda },
    ];
    assert_eq!(bars(&m), [(0, 4), (1, 3), (4, 6)]);
    // With a repeat of bars 1-2 played twice first, but not after the D.C.
    m.navigation = vec![NavMark { id: 2, at: F::int(6), kind: Nav::DaCapo(NavTo::End) }];
    m.barlines = vec![Barline { id: 9, at: F::int(2), kind: BarKind::RepeatEnd }];
    assert_eq!(bars(&m), [(0, 2), (0, 6), (0, 6)]);
    // First and second endings: after the D.C. only the second.
    m.barlines = vec![Barline { id: 9, at: F::int(2), kind: BarKind::RepeatEnd }];
    m.endings = vec![Ending { id: 7, from: F::int(1), to: F::int(2), passes: 1 }, Ending { id: 8, from: F::int(2), to: F::int(3), passes: 2 }];
    assert_eq!(bars(&m), [(0, 2), (0, 1), (2, 6), (0, 1), (2, 6)]);
}
