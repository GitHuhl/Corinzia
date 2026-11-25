//! Players' timing (`corinzia_perform::timing`): near the beat, never on it.

use corinzia_edit::{make_event, Ids};
use corinzia_model::*;
use corinzia_perform::timing::{Feel, PlayerFeel};
use corinzia_perform::*;
use corinzia_score::Frac;
use std::sync::Arc;

fn p(step: i8, octave: i8) -> Pitch {
    Pitch { step, alter: 0, octave }
}

/// One lane per (instrument, notes): each note `len` long, one after another.
fn movement(bars: i64, lanes: Vec<(Id, Vec<Vec<Pitch>>, Frac)>) -> Movement {
    let mut ids = Ids(1000);
    Movement {
        id: 1,
        ordinal: 0,
        title: String::new(),
        length: Frac::int(bars),
        revision: 0,
        tempo: vec![Tempo { at: Frac::ZERO, bpm: 120.0, ramp: false, text: String::new(), hidden: false }],
        meter: vec![Meter { at: Frac::ZERO, num: 4, den: 4, pickup: false, beams: [0; 8] }],
        keys: vec![Key { at: Frac::ZERO, fifths: 0, minor: false }],
        barlines: vec![],
        rehearsal: vec![],
        navigation: vec![],
        slurs: vec![],
        endings: vec![],
        breaks: vec![],
        cross: vec![],
        automation: vec![],
        tweaks: vec![],
        lanes: lanes
            .into_iter()
            .enumerate()
            .map(|(k, (inst, notes, len))| {
                let events = notes.iter().enumerate().map(|(i, ps)| make_event(&mut ids, len * Frac::int(i as i64), len, ps, 0)).collect();
                Lane {
                    player: 10 + k as Id,
                    changes: vec![InstrumentChange { id: 20 + k as Id, at: Frac::ZERO, instrument: inst }],
                    clefs: vec![],
                    texts: vec![],
                    chords: vec![],
                    lines: vec![],
                    voices: vec![Voice { id: 30 + k as Id, index: 1, events: Arc::new(events), tuplets: vec![] }],
                }
            })
            .collect(),
    }
}

fn style(feels: &[(Id, PlayerFeel)]) -> Style {
    Style { phrasing: true, feels: feels.iter().cloned().collect(), ..Style::default() }
}

fn offsets(perf: &Performance, inst: Id) -> Vec<f64> {
    perf.notes.iter().filter(|n| n.instrument == inst).map(|n| n.timing_sec * 1000.0).collect()
}

fn mean_sd(v: &[f64]) -> (f64, f64) {
    let m = v.iter().sum::<f64>() / v.len() as f64;
    (m, (v.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / v.len() as f64).sqrt())
}

#[test]
fn players_are_near_the_beat_but_never_exactly_on_it() {
    // 64 bars of quarters for a violin.
    let m = movement(64, vec![(4, vec![vec![p(0, 5)]; 256], Frac::new(1, 4))]);
    let st = style(&[(4, PlayerFeel::One(Feel::STRINGS))]);
    // Off: as written.
    assert!(perform_with(&m, Style { phrasing: false, ..st.clone() }).notes.iter().all(|n| n.timing_sec == 0.0));
    let perf = perform_with(&m, st.clone());
    let v = offsets(&perf, 4);
    let (mean, sd) = mean_sd(&v[1..]);
    // Close on average, spread like a string player (drift 14, jitter 8 ms).
    assert!(mean.abs() < 8.0, "mean {mean} ms");
    assert!(sd > 7.0 && sd < 25.0, "spread {sd} ms");
    assert!(v.iter().all(|x| x.abs() < 70.0), "never far: {:?}", v.iter().fold(0.0f64, |a, x| a.max(x.abs())));
    assert!(v[1..].iter().filter(|x| x.abs() < 0.5).count() < v.len() / 10, "hardly ever exactly on the beat");
    // Correlated in time, not white noise: neighbours lean the same way.
    let (a, b) = (&v[1..v.len() - 1], &v[2..]);
    let r = a.iter().zip(b).map(|(x, y)| (x - mean) * (y - mean)).sum::<f64>() / (a.len() as f64 * sd * sd);
    assert!(r > 0.3, "lag-1 correlation {r}");
    // The same every time.
    assert_eq!(v, offsets(&perform_with(&m, st), 4));
}

#[test]
fn two_players_are_out_of_step_with_each_other() {
    let notes = vec![vec![p(0, 5)]; 64];
    let m = movement(16, vec![(4, notes.clone(), Frac::new(1, 4)), (6, notes, Frac::new(1, 4))]);
    let perf = perform_with(&m, style(&[(4, PlayerFeel::One(Feel::STRINGS)), (6, PlayerFeel::One(Feel::STRINGS))]));
    let (a, b) = (offsets(&perf, 4), offsets(&perf, 6));
    let diffs: Vec<f64> = a.iter().zip(&b).map(|(x, y)| x - y).collect();
    let (_, sd) = mean_sd(&diffs);
    // Rasch measured 30-50 ms asynchronies in trios; strings here a little tighter.
    assert!(sd > 8.0 && sd < 50.0, "asynchrony {sd} ms");
}

#[test]
fn a_string_players_double_stop_is_one_gesture() {
    let m = movement(4, vec![(4, vec![vec![p(0, 4), p(4, 4)]; 16], Frac::new(1, 4))]);
    let perf = perform_with(&m, style(&[(4, PlayerFeel::One(Feel::STRINGS))]));
    for pair in perf.notes.chunks(2) {
        assert_eq!(pair[0].timing_sec, pair[1].timing_sec);
    }
}

#[test]
fn a_pianists_melody_leads_the_chord() {
    let m = movement(16, vec![(4, vec![vec![p(0, 4), p(4, 4), p(0, 5)]; 64], Frac::new(1, 4))]);
    let perf = perform_with(&m, style(&[(4, PlayerFeel::for_instrument(corinzia_model::catalog::kind("piano"), &[]))]));
    let mut lead = Vec::new();
    for chord in perf.notes.chunks(3).skip(1) {
        let top = chord.iter().max_by_key(|n| n.key).unwrap();
        let rest = chord.iter().filter(|n| n.key != top.key).map(|n| n.timing_sec).sum::<f64>() / 2.0;
        lead.push((rest - top.timing_sec) * 1000.0);
    }
    let (mean, _) = mean_sd(&lead);
    assert!(mean > 10.0 && mean < 20.0, "melody lead {mean} ms");
}

#[test]
fn a_drum_kit_keeps_time_on_the_hats_and_lays_the_snare_back() {
    // Hi-hat (written g'', GM 42) in eighths; snare (c'', GM 38) on 2 and 4;
    // bass drum (f', GM 36) on 1 and 3: three lanes of one kit.
    let kit = PlayerFeel::for_instrument(corinzia_model::catalog::kind("drum-set"), &[(79, 42), (72, 38), (65, 36)]);
    assert!(matches!(kit, PlayerFeel::Kit(_)));
    let hats = vec![vec![p(4, 5)]; 256];
    let snare: Vec<Vec<Pitch>> = (0..64).map(|i| if i % 2 == 1 { vec![p(0, 5)] } else { vec![] }).collect();
    let kick: Vec<Vec<Pitch>> = (0..64).map(|i| if i % 2 == 0 { vec![p(3, 4)] } else { vec![] }).collect();
    let m = movement(32, vec![(7, hats, Frac::new(1, 8)), (7, snare, Frac::new(1, 2)), (7, kick, Frac::new(1, 2))]);
    let perf = perform_with(&m, style(&[(7, kit)]));
    let by_key = |k: i32| perf.notes.iter().filter(|n| n.key == k).skip(1).map(|n| n.timing_sec * 1000.0).collect::<Vec<_>>();
    let (hat_mean, hat_sd) = mean_sd(&by_key(79));
    let (snare_mean, _) = mean_sd(&by_key(72));
    let (kick_mean, _) = mean_sd(&by_key(65));
    // The steadiest of all: a few ms.
    assert!(hat_sd > 2.0 && hat_sd < 10.0, "hi-hat spread {hat_sd} ms");
    assert!(snare_mean > hat_mean + 3.0, "snare laid back: {snare_mean} vs hats {hat_mean}");
    assert!(kick_mean < snare_mean, "bass drum ahead of the snare");
}
