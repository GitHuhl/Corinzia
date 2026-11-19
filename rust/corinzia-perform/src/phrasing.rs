//! Performance rules: what players do that the score does not write.
//!
//! Always: fermatas are held (about twice their length).
//!
//! With [`Style::phrasing`](crate::Style) on, rules after the KTH rule
//! system for music performance (Friberg, Bresin and Sundberg, "Overview
//! of the KTH rule system for musical performance", Advances in Cognitive
//! Psychology 2, 2006, and the papers it reviews):
//!
//! - **phrase arch**: each phrase (between rests and breaths) swells
//!   towards a point a little after its middle and relaxes at its ends;
//! - **final ritardando**: the last bar slows as a runner stops
//!   (Friberg and Sundberg 1999: v(x) = (1 + (w^q − 1)·x)^(1/q)), and the
//!   beat before a fermata broadens;
//! - **high loud**: higher notes of a line a little louder;
//! - **metrical accents**: the downbeat and the strong beats a little
//!   louder, off-beats softer;
//! - **melodic peaks**: a note above both its neighbours is leant on;
//! - **duration contrast**: short notes a little softer and shorter, long
//!   ones a little louder;
//! - **leap articulation**: a small break before a wide leap that is not
//!   slurred;
//! - **melodic lead**: the top note of a chord sings over the others;
//! - **timing**: no player exactly on the beat (see [`crate::timing`]).
//!
//! Dynamic changes are in decibels of an instrument's typical range
//! (about 30 dB from ppp to fff over velocity 0.16..1), a few dB at most,
//! and each line's average level stays as written: the rules shape the
//! music without making it louder.

use crate::tempo::{TempoMap, TempoPoint};
use crate::{Performance, Style};
use corinzia_score::score::art;
use std::collections::BTreeMap;

/// Velocity (0..1) per decibel.
const PER_DB: f32 = 0.84 / 30.0;

/// The fermata's hold (tempo factor while it sounds).
const FERMATA_HOLD: f64 = 0.5;

/// `followed`: from here (quarters) the tempo follows a recording (see
/// [`crate::follow`]), which already has its holds and slowings: no
/// tempo rule changes it.
pub fn apply(perf: &mut Performance, fermatas: &[(f64, f64)], style: &Style, followed: Option<f64>) {
    // Tempo regions: (from, to, factor at from, factor at to, curve).
    let mut regions: Vec<Region> = Vec::new();
    for (a, b) in merge(fermatas) {
        regions.push(Region { from: a, to: b, f0: FERMATA_HOLD, f1: FERMATA_HOLD, runner: false });
        if style.phrasing && a > 1.0 {
            regions.push(Region { from: a - 1.0, to: a, f0: 1.0, f1: 0.85, runner: true });
        }
    }
    if style.phrasing && perf.length_q > 0.0 {
        // The last bar, or the last two beats of a short one.
        let bar = perf.tempo.bar_at((perf.length_q - 1e-6).max(0.0));
        let len = (perf.length_q - bar.start_quarter).max(2.0f64.min(perf.length_q));
        let from = (perf.length_q - len).max(0.0);
        regions.push(Region { from, to: perf.length_q, f0: 1.0, f1: 0.72, runner: true });
    }
    if let Some(f) = followed {
        regions.retain(|r| r.to <= f + 1e-9);
    }
    if !regions.is_empty() {
        perf.tempo_points = warp(&perf.tempo, &perf.tempo_points, &regions, perf.length_q);
        perf.tempo = TempoMap::new(&perf.tempo_points, &perf.meter_points);
    }
    if style.phrasing {
        dynamics_and_articulation(perf);
    }
}

#[derive(Clone, Copy, Debug)]
struct Region {
    from: f64,
    to: f64,
    f0: f64,
    f1: f64,
    /// The runner's curve (q = 2) rather than a straight line.
    runner: bool,
}

impl Region {
    fn factor(&self, q: f64) -> f64 {
        let x = ((q - self.from) / (self.to - self.from)).clamp(0.0, 1.0);
        if self.runner {
            // The tempo ratio's square falls linearly (a constant deceleration).
            let w = self.f1 / self.f0;
            self.f0 * (1.0 + (w * w - 1.0) * x).max(1e-4).sqrt()
        } else {
            self.f0 + (self.f1 - self.f0) * x
        }
    }
}

/// Overlapping fermatas (several staves) as one.
fn merge(spans: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut v: Vec<(f64, f64)> = spans.iter().copied().filter(|s| s.1 > s.0).collect();
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out: Vec<(f64, f64)> = Vec::new();
    for (a, b) in v {
        match out.last_mut() {
            Some(l) if a <= l.1 + 1e-9 => l.1 = l.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// The tempo points with the regions' factors applied: each region's
/// curve in short linear ramps.
fn warp(base: &TempoMap, points: &[TempoPoint], regions: &[Region], end: f64) -> Vec<TempoPoint> {
    let factor = |q: f64| regions.iter().filter(|r| r.from <= q && q < r.to).map(|r| r.factor(q)).product::<f64>();
    let factor_left = |q: f64| regions.iter().filter(|r| r.from < q && q <= r.to).map(|r| r.factor(q)).product::<f64>();
    let mut cuts: Vec<f64> = points.iter().map(|p| p.quarter).collect();
    for r in regions {
        let n = if r.f0 == r.f1 { 1 } else { 8 };
        for k in 0..=n {
            cuts.push(r.from + (r.to - r.from) * k as f64 / n as f64);
        }
    }
    cuts.push(0.0);
    cuts.retain(|q| *q >= 0.0 && *q <= end.max(0.0) + 1e-9);
    cuts.sort_by(|a, b| a.total_cmp(b));
    cuts.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    let ramps_at = |q: f64| points.iter().filter(|p| p.quarter <= q + 1e-12).last().is_some_and(|p| p.ramp_to_next);
    let mut out = Vec::new();
    for (i, &q) in cuts.iter().enumerate() {
        let bpm = base.tempo_at(q) * factor(q);
        let next = cuts.get(i + 1).copied();
        let varies = next.is_some_and(|b| {
            let mid = (q + b) / 2.0;
            ramps_at(q) || regions.iter().any(|r| r.from <= mid && mid < r.to && r.f0 != r.f1)
        });
        out.push(TempoPoint { quarter: q, bpm, ramp_to_next: varies });
        // A ramp ends at its own left value, then the next segment starts.
        if let (true, Some(b)) = (varies, next) {
            let left = base.tempo_at(b - 1e-9) * factor_left(b);
            let right = base.tempo_at(b) * factor(b);
            if (left - right).abs() > 1e-6 {
                out.push(TempoPoint { quarter: b - 1e-6, bpm: left, ramp_to_next: false });
            }
        }
    }
    out
}

fn dynamics_and_articulation(perf: &mut Performance) {
    let tempo = perf.tempo.clone();
    // Each line: its notes in time order.
    let mut lines: BTreeMap<(u64, u32), Vec<usize>> = BTreeMap::new();
    for (i, n) in perf.notes.iter().enumerate() {
        lines.entry((n.instrument, n.voice)).or_default().push(i);
    }
    let mut db = vec![0.0f32; perf.notes.len()];
    let mut gate = vec![1.0f64; perf.notes.len()];
    for idx in lines.values_mut() {
        idx.sort_by(|a, b| perf.notes[*a].on_q.total_cmp(&perf.notes[*b].on_q).then(perf.notes[*b].key.cmp(&perf.notes[*a].key)));
        // The line's melody: one note per onset, the top of a chord.
        let mut melody: Vec<usize> = Vec::new();
        for &i in idx.iter() {
            match melody.last() {
                Some(&l) if (perf.notes[l].on_q - perf.notes[i].on_q).abs() < 1e-9 => {}
                _ => melody.push(i),
            }
        }
        // Melodic lead: the chord's top note over the rest.
        for &i in idx.iter() {
            if !melody.contains(&i) {
                db[i] -= 1.5;
            }
        }
        let n = &perf.notes;
        let mean = melody.iter().map(|&i| n[i].key as f32).sum::<f32>() / melody.len().max(1) as f32;
        // Phrases: broken by rests (half a beat or more) and breaths.
        let mut phrases: Vec<(usize, usize)> = Vec::new();
        let mut start = 0;
        for k in 1..=melody.len() {
            let brk = k == melody.len() || {
                let (a, b) = (&n[melody[k - 1]], &n[melody[k]]);
                let sounded_end = a.on_q + (a.full_off_q - a.on_q) * a.gate.max(0.9);
                b.on_q - sounded_end >= 0.5 || a.arts & (art::BREATH | art::CAESURA | art::FERMATA) != 0
            };
            if brk {
                phrases.push((start, k));
                start = k;
            }
        }
        for (a, b) in phrases {
            let (t0, t1) = (n[melody[a]].on_q, n[melody[b - 1]].full_off_q);
            for k in a..b {
                let i = melody[k];
                let x = if t1 > t0 { ((n[i].on_q - t0) / (t1 - t0)).clamp(0.0, 1.0) } else { 0.5 };
                let arch = 1.6 * (std::f64::consts::PI * x.powf(1.4)).sin() as f32;
                // Melodic peak.
                let peak = k > a && k + 1 < b && n[i].key > n[melody[k - 1]].key && n[i].key > n[melody[k + 1]].key;
                // Leap articulation.
                if k + 1 < b && !n[i].slurred && (n[melody[k + 1]].key - n[i].key).abs() >= 7 {
                    gate[i] *= 0.9;
                }
                let shift = arch + if peak { 0.6 } else { 0.0 } + ((n[i].key as f32 - mean) * 0.08).clamp(-1.2, 1.2);
                for &j in idx.iter().filter(|&&j| (n[j].on_q - n[i].on_q).abs() < 1e-9) {
                    db[j] += shift;
                }
            }
        }
        for &i in idx.iter() {
            let note = &n[i];
            // Metrical accents.
            let bar = tempo.bar_at(note.on_q);
            let beat = 4.0 / bar.den as f64 * if bar.den == 8 && bar.num % 3 == 0 { 3.0 } else { 1.0 };
            let pos = note.on_q - bar.start_quarter;
            let beats = (4.0 * bar.num as f64 / bar.den as f64 / beat).round().max(1.0) as i64;
            let on_beat = (pos / beat - (pos / beat).round()).abs() < 1e-6;
            let which = (pos / beat).round() as i64;
            db[i] += if pos.abs() < 1e-6 {
                0.8
            } else if on_beat && beats % 2 == 0 && which == beats / 2 {
                0.4
            } else if on_beat {
                0.0
            } else {
                -0.3
            };
            // Duration contrast.
            let secs = tempo.seconds_at(note.full_off_q) - tempo.seconds_at(note.on_q);
            if secs < 0.18 {
                db[i] -= 1.0;
                if !note.slurred && note.gate < 1.0 {
                    gate[i] *= 0.92;
                }
            } else if secs > 1.0 {
                db[i] += 0.4;
            }
        }
        // Shaping, not louder: the line's average level stays as written.
        let mean_db = idx.iter().map(|&i| db[i]).sum::<f32>() / idx.len().max(1) as f32;
        for &i in idx.iter() {
            db[i] -= mean_db;
        }
    }
    for (i, note) in perf.notes.iter_mut().enumerate() {
        let d = db[i] * PER_DB;
        note.velocity = (note.velocity + d).clamp(0.02, 1.0);
        note.end_velocity = (note.end_velocity + d).clamp(0.02, 1.0);
        note.sustain = (note.sustain + d).clamp(0.02, 1.0);
        note.gate *= gate[i];
    }
}
