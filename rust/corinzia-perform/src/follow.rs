//! A tempo map that follows a recording: beats found in an audio clip
//! become the movement's tempo, so that its bars and beats (the score, the
//! built-in orchestra, the clips' places on the page) fall where the
//! recording's do.
//!
//! The beats (seconds after the movement's start) are laid on a grid of
//! the score's beats from the one nearest the first: a tempo for each beat
//! (or bar), hidden on the page, makes each grid point sound when its beat
//! does; a hidden tempo on the beat before the first brings the music in
//! on time. Before that the tempo is as it was; after the last beat, the
//! last beat's tempo holds until the next tempo mark. Tempo marks inside
//! the stretch stay on the page with the tempo they now have.

use crate::{perform_with, playback_order, Style};
use corinzia_model::{Movement, Tempo};
use corinzia_score::Frac;

/// How closely the tempo follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Follow {
    /// A tempo on every beat: each lands where it was played.
    Beats,
    /// A tempo on each bar: the downbeats land where they were played,
    /// the beats between them evenly.
    Bars,
}

/// The tempo map found, and what goes with it.
#[derive(Clone, Debug, PartialEq)]
pub struct Followed {
    /// The movement's new tempo marks.
    pub tempo: Vec<Tempo>,
    /// Where the first beat now falls (quarters).
    pub first_q: f64,
    /// Beats followed.
    pub beats: usize,
    /// Their tempo's middle value (beats a minute).
    pub bpm: f64,
    /// Seconds to move the clip by first (its first beat on the movement's
    /// downbeat, when it fell there: nothing can come before it).
    pub shift: f64,
}

/// The movement's tempo following `beats` (seconds after its start,
/// ascending), each `unit` quarters long (1: crotchets; 1.5: dotted).
pub fn follow(m: &Movement, style: &Style, beats: &[f64], unit: f64, how: Follow) -> Result<Followed, String> {
    if !(unit > 0.0 && unit.is_finite()) {
        return Err("no beat length".into());
    }
    if playback_order(m).len() > 1 {
        return Err("following a recording is not possible in a movement with repeats or jumps yet".into());
    }
    let beats: Vec<f64> = beats.iter().copied().filter(|b| b.is_finite() && *b >= 0.0).collect();
    if beats.len() < 2 {
        return Err("too few beats found to follow".into());
    }
    let length = crate::q(m.length);
    let perf = perform_with(m, style.clone());
    // The grid: the score's beat nearest the first recorded one.
    let mut q0 = (perf.tempo.quarter_at(beats[0]) / unit).round() * unit;
    let mut shift = 0.0;
    if q0 < unit / 2.0 {
        // On the downbeat: the clip moves so that its first beat is there.
        shift = -beats[0];
        q0 = 0.0;
    }
    let beats: Vec<f64> = beats.iter().map(|b| b + shift).collect();
    let grid: Vec<(f64, f64)> = beats.iter().enumerate().map(|(i, &t)| (q0 + i as f64 * unit, t)).take_while(|(q, _)| *q < length - 1e-9).collect();
    if grid.len() < 2 {
        return Err("the recording's beats start after the movement ends".into());
    }
    // The points whose times are kept: every beat, or the downbeats (and
    // the first and last beat).
    let anchors: Vec<(f64, f64)> = match how {
        Follow::Beats => grid.clone(),
        Follow::Bars => {
            let last = grid.len() - 1;
            grid.iter()
                .enumerate()
                .filter(|(i, (q, _))| *i == 0 || *i == last || (perf.tempo.bar_at(*q).start_quarter - q).abs() < 1e-6)
                .map(|(_, p)| *p)
                .collect()
        }
    };
    let at = |q: f64| Frac::new((q * 256.0).round() as i64, 1024);
    let hidden = |q: f64, bpm: f64| Tempo { at: at(q), bpm, ramp: false, text: String::new(), hidden: true };
    let mut points: Vec<Tempo> = Vec::new();
    // The lead-in: the beat before the first takes the time up to it.
    if q0 > 0.0 {
        let from = (q0 - unit).max(0.0);
        let took = beats[0] - perf.tempo.seconds_at(from);
        let bpm = 60.0 * (q0 - from) / took;
        if took > 0.0 && (10.0..=600.0).contains(&bpm) {
            points.push(hidden(from, bpm));
        }
    }
    for w in anchors.windows(2) {
        let ((qa, ta), (qb, tb)) = (w[0], w[1]);
        if tb > ta {
            points.push(hidden(qa, 60.0 * (qb - qa) / (tb - ta)));
        }
    }
    let (from, to) = (points.first().map(|p| p.at).unwrap_or(at(q0)), at(grid.last().unwrap().0));
    // Kept: what comes before and after; marks on the page inside take the
    // tempo there.
    let mut tempo: Vec<Tempo> = m.tempo.iter().filter(|t| t.at < from || t.at > to || !t.hidden).cloned().collect();
    let in_force = |points: &[Tempo], a: Frac| points.iter().filter(|p| p.at <= a).last().map(|p| p.bpm);
    for t in tempo.iter_mut().filter(|t| t.at >= from && t.at <= to) {
        if let Some(bpm) = in_force(&points, t.at) {
            t.bpm = bpm;
            t.ramp = false;
        }
    }
    points.retain(|p| !tempo.iter().any(|t| t.at == p.at));
    tempo.extend(points);
    tempo.sort_by(|a, b| a.at.cmp(&b.at));
    let mut gaps: Vec<f64> = grid.windows(2).map(|w| w[1].1 - w[0].1).filter(|g| *g > 0.0).collect();
    gaps.sort_by(f64::total_cmp);
    let bpm = gaps.get(gaps.len() / 2).map(|g| 60.0 / g).unwrap_or(0.0);
    Ok(Followed { tempo, first_q: q0, beats: grid.len(), bpm, shift })
}

/// The beat (quarters) the score counts in at `q`: a dotted crotchet in
/// 6/8, 9/8 and 12/8, else the meter's note.
pub fn beat_unit(m: &Movement, q: f64) -> f64 {
    let perf_meter = m.meter.iter().filter(|x| crate::q(x.at) <= q + 1e-9).last().or(m.meter.first());
    match perf_meter {
        Some(x) if x.den == 8 && x.num % 3 == 0 && x.num > 3 => 1.5,
        Some(x) if x.den > 0 => 4.0 / x.den as f64,
        _ => 1.0,
    }
}
