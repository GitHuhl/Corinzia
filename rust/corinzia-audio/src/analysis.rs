//! Beats in recorded sound, for a tempo map that follows a recording.
//!
//! [`onset_strength`] is adapted from SoundCraft's `detect_transients`
//! (`crates/dsp/src/offline.rs` in <https://github.com/storytold/soundcraft>,
//! Copyright (c) 2026 ArtCraft Team and the SoundCraft contributors, MIT
//! licence; see `THIRD-PARTY-NOTICES.md`): how much a log-magnitude
//! spectrum on 24 bands an octave rises against the frame before, each
//! band against the loudest of its neighbours there so that vibrato does
//! not count (vibrato-suppressed spectral flux: Böck and Widmer, "Maximum
//! filter vibrato suppression for onset detection", DAFx 2013).
//!
//! [`track_beats`] follows the beat through that strength by dynamic
//! programming (D. P. W. Ellis, "Beat tracking by dynamic programming",
//! Journal of New Music Research 36(1), 2007): the best chain of onsets
//! whose gaps stay near one beat, each gap's departure from it costing
//! the square of its log ratio. The beat's length comes from the
//! strength's autocorrelation, weighted towards a guess (the score's
//! tempo, when there is one).

use rustfft::{num_complex::Complex, FftPlanner};
use std::f64::consts::PI;

/// Onset strength per analysis frame.
#[derive(Clone, Debug)]
pub struct Onsets {
    pub strength: Vec<f32>,
    /// Seconds per frame.
    pub hop: f64,
    /// Seconds from a frame's index × `hop` to the onset it shows.
    pub offset: f64,
}

/// How strongly each frame starts something (≥ 0).
pub fn onset_strength(channels: &[Vec<f32>], sample_rate: f64) -> Onsets {
    let sr = if sample_rate.is_finite() && sample_rate > 1000.0 { sample_rate } else { 48_000.0 };
    let n = ((sr * 0.012) as usize).clamp(64, 8192).next_power_of_two();
    let hop = n / 4;
    let len = channels.iter().map(Vec::len).max().unwrap_or(0);
    let out = |strength| Onsets { strength, hop: hop as f64 / sr, offset: n as f64 * 0.5 / sr };
    if len < n || channels.is_empty() {
        return out(vec![]);
    }
    let ch: Vec<&Vec<f32>> = channels.iter().take(8).collect();
    let nch = ch.len() as f32;
    let bands = log_bands(n, sr as f32);
    let fft = FftPlanner::<f32>::new().plan_fft_forward(n);
    let window: Vec<f32> = (0..n).map(|i| (0.5 - 0.5 * (2.0 * PI * i as f64 / n as f64).cos()) as f32).collect();
    // Magnitudes relative to a full-scale sine; below about -80 dB a band's changes barely count.
    let gain = 1.0e4 / (n as f32 / 4.0);
    let frames = (len - n) / hop + 1;
    let mut buf = vec![Complex::new(0.0f32, 0.0); n];
    let mut power = vec![0.0f32; n / 2 + 1];
    let mut prev: Vec<f32> = Vec::new();
    let mut flux = Vec::with_capacity(frames);
    for m in 0..frames {
        power.iter_mut().for_each(|p| *p = 0.0);
        for c in &ch {
            for (i, b) in buf.iter_mut().enumerate() {
                let v = c.get(m * hop + i).copied().unwrap_or(0.0);
                *b = Complex::new(if v.is_finite() { v } else { 0.0 } * window[i], 0.0);
            }
            fft.process(&mut buf);
            for (p, b) in power.iter_mut().zip(&buf) {
                *p += b.norm_sqr() / nch;
            }
        }
        let cur: Vec<f32> = bands.iter().map(|band| (1.0 + gain * band.iter().map(|&(k, w)| power[k].sqrt() * w).sum::<f32>()).log10()).collect();
        let rise = |b: usize, v: f32| (v - prev[b.saturating_sub(1)..(b + 2).min(prev.len())].iter().copied().fold(0.0, f32::max)).max(0.0);
        flux.push(if prev.is_empty() { 0.0 } else { cur.iter().enumerate().map(|(b, v)| rise(b, *v)).sum() });
        prev = cur;
    }
    out(flux)
}

/// Triangular filters 24 to the octave from 30 Hz to 20 kHz over the bins
/// of an `n`-point FFT, without duplicates where bins are coarser than
/// that; each filter's weights sum to 1.
fn log_bands(n: usize, sr: f32) -> Vec<Vec<(usize, f32)>> {
    let bin_hz = sr / n as f32;
    let top = 20_000.0f32.min(sr * 0.45);
    let mut centres: Vec<usize> = Vec::new();
    let mut f = 30.0f32;
    while f <= top {
        let k = (f / bin_hz).round() as usize;
        if centres.last().is_none_or(|&c| k > c) {
            centres.push(k);
        }
        f *= 2f32.powf(1.0 / 24.0);
    }
    centres
        .windows(3)
        .filter_map(|w| {
            let (lo, mid, hi) = (w[0], w[1], w[2]);
            let tri: Vec<(usize, f32)> = (lo..=hi.min(n / 2))
                .map(|k| (k, if k <= mid { (k - lo) as f32 / (mid - lo).max(1) as f32 } else { (hi - k) as f32 / (hi - mid).max(1) as f32 }))
                .filter(|(_, v)| *v > 0.0)
                .collect();
            let sum: f32 = tri.iter().map(|(_, v)| v).sum();
            (sum > 0.0).then(|| tri.into_iter().map(|(k, v)| (k, v / sum)).collect())
        })
        .collect()
}

/// The beats of a recording, in seconds from its start. `guess`: about
/// how long a beat is (seconds), from the score; without one, about half a
/// second (120 a minute). Empty when there is too little to go on.
pub fn track_beats(channels: &[Vec<f32>], sample_rate: f64, guess: Option<f64>) -> Vec<f64> {
    let o = onset_strength(channels, sample_rate);
    beats_from(&o, guess)
}

/// [`track_beats`] on an onset strength already found.
pub fn beats_from(o: &Onsets, guess: Option<f64>) -> Vec<f64> {
    let env = &o.strength;
    let fps = 1.0 / o.hop;
    if env.len() < (fps * 2.0) as usize {
        return vec![];
    }
    // Normalized: a unit spread.
    let mean = env.iter().map(|&v| v as f64).sum::<f64>() / env.len() as f64;
    let sd = (env.iter().map(|&v| (v as f64 - mean).powi(2)).sum::<f64>() / env.len() as f64).sqrt();
    if sd <= 1e-9 {
        return vec![];
    }
    let env: Vec<f64> = env.iter().map(|&v| v as f64 / sd).collect();
    let period = beat_period(&env, fps, guess);
    // Each frame's own score: the strength smoothed over about a beat.
    let p = period.round().max(2.0) as isize;
    let kernel: Vec<f64> = (-p..=p).map(|i| (-0.5 * (i as f64 * 32.0 / period).powi(2)).exp()).collect();
    let local: Vec<f64> = (0..env.len() as isize)
        .map(|t| kernel.iter().enumerate().map(|(k, w)| env.get((t + k as isize - p).max(-1) as usize).copied().unwrap_or(0.0) * w).sum())
        .collect();
    // The best chain ending at each frame, and the beat before it there.
    const TIGHTNESS: f64 = 100.0;
    let n = local.len();
    let mut score = vec![0.0f64; n];
    let mut back = vec![usize::MAX; n];
    let (lo, hi) = ((period * 2.0).round() as usize, (period / 2.0).round().max(1.0) as usize);
    for t in 0..n {
        let mut best = (f64::NEG_INFINITY, usize::MAX);
        for prev in t.saturating_sub(lo)..t.saturating_sub(hi).min(t) {
            let gap = (t - prev) as f64;
            let s = score[prev] - TIGHTNESS * (gap / period).ln().powi(2);
            if s > best.0 {
                best = (s, prev);
            }
        }
        if best.1 != usize::MAX && t >= hi {
            score[t] = local[t] + best.0.max(0.0);
            back[t] = if best.0 > 0.0 { best.1 } else { usize::MAX };
        } else {
            score[t] = local[t];
        }
    }
    // The chain's end: the last of the strong local peaks of the score.
    let peaks: Vec<usize> = (1..n.saturating_sub(1)).filter(|&t| score[t] > score[t - 1] && score[t] >= score[t + 1]).collect();
    if peaks.is_empty() {
        return vec![];
    }
    let mut sorted: Vec<f64> = peaks.iter().map(|&t| score[t]).collect();
    sorted.sort_by(f64::total_cmp);
    let median = sorted[sorted.len() / 2];
    let Some(&end) = peaks.iter().rev().find(|&&t| score[t] > 0.5 * median) else { return vec![] };
    let mut beats = vec![end];
    while let Some(&b) = beats.last() {
        if back[b] == usize::MAX {
            break;
        }
        beats.push(back[b]);
    }
    beats.reverse();
    // Weak beats at the ends (before the music starts, after it stops) go.
    let rms = (beats.iter().map(|&b| local[b].powi(2)).sum::<f64>() / beats.len() as f64).sqrt();
    let strong = |b: &usize| local[*b] >= 0.5 * rms;
    let first = beats.iter().position(strong).unwrap_or(0);
    let last = beats.iter().rposition(strong).unwrap_or(beats.len() - 1);
    beats[first..=last].iter().map(|&b| b as f64 * o.hop + o.offset).collect()
}

/// A beat's length in frames: the autocorrelation's best lag between 0.25
/// and 2 seconds, weighted towards the guess (log-normal, an octave and a
/// half wide without a guess, about half an octave with one).
fn beat_period(env: &[f64], fps: f64, guess: Option<f64>) -> f64 {
    let (centre, width) = match guess.filter(|g| g.is_finite() && *g > 0.1 && *g < 4.0) {
        Some(g) => (g * fps, 0.5),
        None => (0.5 * fps, 1.4),
    };
    let (min, max) = ((0.25 * fps) as usize, ((2.0 * fps) as usize).min(env.len() - 1));
    let ac = |lag: usize| env.iter().zip(&env[lag..]).map(|(a, b)| a * b).sum::<f64>() / (env.len() - lag) as f64;
    let weighted = |lag: usize| ac(lag) * (-0.5 * ((lag as f64 / centre).log2() / width).powi(2)).exp();
    let values: Vec<f64> = (min..=max).map(weighted).collect();
    let Some(k) = (0..values.len()).max_by(|&a, &b| values[a].total_cmp(&values[b])) else { return centre };
    // Between frames: the parabola through the peak and its neighbours.
    let refined = if k > 0 && k + 1 < values.len() {
        let (a, b, c) = (values[k - 1], values[k], values[k + 1]);
        let d = a - 2.0 * b + c;
        if d.abs() > 1e-12 { k as f64 + 0.5 * (a - c) / d } else { k as f64 }
    } else {
        k as f64
    };
    (min as f64 + refined).max(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Clicks (short noise bursts over a decaying tone) at these times.
    fn clicks(times: &[f64], seconds: f64, sr: f64) -> Vec<Vec<f32>> {
        let mut x = vec![0.0f32; (seconds * sr) as usize];
        let mut seed = 1u32;
        for &t in times {
            let s = (t * sr) as usize;
            for i in 0..(0.08 * sr) as usize {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let noise = (seed >> 9) as f32 / (1u32 << 23) as f32 - 0.5;
                let env = (-(i as f32) / (0.01 * sr as f32)).exp();
                let tone = (2.0 * std::f32::consts::PI * 220.0 * i as f32 / sr as f32).sin() * (-(i as f32) / (0.05 * sr as f32)).exp();
                if let Some(v) = x.get_mut(s + i) {
                    *v += 0.5 * noise * env + 0.3 * tone;
                }
            }
        }
        vec![x]
    }

    #[test]
    fn a_steady_beat_is_found_where_it_is() {
        let sr = 44_100.0;
        let times: Vec<f64> = (0..24).map(|i| 0.5 + i as f64 * 0.5).collect();
        let beats = track_beats(&clicks(&times, 13.0, sr), sr, None);
        assert!(beats.len() >= 22, "{beats:?}");
        for b in &beats {
            let near = times.iter().map(|t| (t - b).abs()).fold(f64::MAX, f64::min);
            assert!(near < 0.02, "beat at {b:.3} is {near:.3} s from a click");
        }
    }

    #[test]
    fn a_beat_that_slows_is_followed() {
        // 120 a minute slowing to 80 over twenty beats.
        let sr = 44_100.0;
        let mut times = vec![0.4];
        for i in 0..20 {
            let bpm = 120.0 - 40.0 * i as f64 / 19.0;
            times.push(times.last().unwrap() + 60.0 / bpm);
        }
        let beats = track_beats(&clicks(&times, times.last().unwrap() + 1.5, sr), sr, Some(0.5));
        let hits = times.iter().filter(|t| beats.iter().any(|b| (*t - b).abs() < 0.03)).count();
        assert!(hits >= 19, "{hits} of {} clicks found: {beats:?}", times.len());
    }

    #[test]
    fn the_scores_tempo_picks_the_level_of_the_beat() {
        // Clicks every quarter second: with a guess of half a second,
        // every other one is a beat.
        let sr = 44_100.0;
        let times: Vec<f64> = (0..40).map(|i| 0.5 + i as f64 * 0.25).collect();
        let beats = track_beats(&clicks(&times, 11.0, sr), sr, Some(0.5));
        let gaps: Vec<f64> = beats.windows(2).map(|w| w[1] - w[0]).collect();
        let median = { let mut g = gaps.clone(); g.sort_by(f64::total_cmp); g[g.len() / 2] };
        assert!((median - 0.5).abs() < 0.02, "{median}");
    }

    #[test]
    fn silence_has_no_beats() {
        assert!(track_beats(&[vec![0.0; 44_100 * 3]], 44_100.0, None).is_empty());
        assert!(track_beats(&[], 44_100.0, None).is_empty());
    }
}
