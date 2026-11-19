//! Loudness of rendered sound, as ITU-R BS.1770 (EBU R128) measures it:
//! integrated loudness (LUFS) and the true peak (dBTP).
//!
//! The K-weighting filters' design and the true-peak interpolator are
//! adapted from SoundCraft's loudness meter (`crates/dsp/src/meter.rs` in
//! <https://github.com/storytold/soundcraft>, Copyright (c) 2026 ArtCraft
//! Team and the SoundCraft contributors, MIT licence; see
//! `THIRD-PARTY-NOTICES.md`). Here the whole sound is at hand, so the
//! gating is exact: 400 ms blocks every 100 ms, an absolute gate at
//! −70 LUFS, then a relative one 10 LU under the loudness of what passed.

/// What a render measures.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Loudness {
    /// Gated integrated loudness, LUFS (−144 for silence).
    pub integrated: f64,
    /// The highest true peak (4× oversampled), dBTP.
    pub true_peak: f64,
}

const SILENT: f64 = -144.0;

/// A biquad's coefficients (a0 normalised to 1).
#[derive(Clone, Copy)]
struct Coeffs {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

impl Coeffs {
    fn run(&self, x: &[f32]) -> Vec<f64> {
        let (mut z1, mut z2) = (0.0, 0.0);
        x.iter()
            .map(|&v| {
                let v = if v.is_finite() { v as f64 } else { 0.0 };
                let y = self.b0 * v + z1;
                z1 = self.b1 * v - self.a1 * y + z2;
                z2 = self.b2 * v - self.a2 * y;
                y
            })
            .collect()
    }
}

/// The K-weighting pre-filter (a high shelf) and its high-pass (RLB),
/// designed for any sample rate.
fn k_weighting(sr: f64) -> [Coeffs; 2] {
    let prewarp = |f0: f64| (std::f64::consts::PI * f0.min(sr * 0.49) / sr).tan();
    let (f0, g, q) = (1_681.974_450_955_533, 3.999_843_853_973_347, 0.707_175_236_955_419_6);
    let k = prewarp(f0);
    let vh = 10f64.powf(g / 20.0);
    let vb = vh.powf(0.499_666_774_154_541_6);
    let a0 = 1.0 + k / q + k * k;
    let shelf = Coeffs { b0: (vh + vb * k / q + k * k) / a0, b1: 2.0 * (k * k - vh) / a0, b2: (vh - vb * k / q + k * k) / a0, a1: 2.0 * (k * k - 1.0) / a0, a2: (1.0 - k / q + k * k) / a0 };
    let (f0, q) = (38.135_470_876_024_44, 0.500_327_037_323_877_3);
    let k = prewarp(f0);
    let a0 = 1.0 + k / q + k * k;
    let high_pass = Coeffs { b0: 1.0, b1: -2.0, b2: 1.0, a1: 2.0 * (k * k - 1.0) / a0, a2: (1.0 - k / q + k * k) / a0 };
    [shelf, high_pass]
}

fn lufs(energy: f64) -> f64 {
    if energy <= 1e-20 { SILENT } else { -0.691 + 10.0 * energy.log10() }
}

/// Measures a stereo (or mono: `right` empty) render.
pub fn measure(left: &[f32], right: &[f32], sample_rate: f64) -> Loudness {
    let sr = if sample_rate.is_finite() && sample_rate > 1000.0 { sample_rate } else { 48_000.0 };
    let [shelf, hp] = k_weighting(sr);
    let channels: Vec<&[f32]> = if right.is_empty() { vec![left] } else { vec![left, right] };
    let weighted: Vec<Vec<f64>> = channels.iter().map(|c| hp.run(&shelf.run(c).iter().map(|&v| v as f32).collect::<Vec<_>>())).collect();
    // Mean square per 100 ms step, summed over the channels.
    let step = (sr * 0.1).round().max(1.0) as usize;
    let frames = weighted.iter().map(Vec::len).min().unwrap_or(0);
    let steps: Vec<f64> = (0..frames / step).map(|k| weighted.iter().map(|w| w[k * step..(k + 1) * step].iter().map(|v| v * v).sum::<f64>() / step as f64).sum()).collect();
    // 400 ms blocks overlapping by 75%.
    let blocks: Vec<f64> = steps.windows(4).map(|w| w.iter().sum::<f64>() / 4.0).collect();
    let above: Vec<f64> = blocks.iter().copied().filter(|&e| lufs(e) > -70.0).collect();
    let integrated = if above.is_empty() {
        SILENT
    } else {
        let gate = lufs(above.iter().sum::<f64>() / above.len() as f64) - 10.0;
        let kept: Vec<f64> = above.into_iter().filter(|&e| lufs(e) > gate).collect();
        if kept.is_empty() { SILENT } else { lufs(kept.iter().sum::<f64>() / kept.len() as f64) }
    };
    let peak = channels.iter().map(|c| true_peak(c)).fold(0.0f32, f32::max);
    Loudness { integrated, true_peak: if peak > 1e-9 { 20.0 * (peak as f64).log10() } else { SILENT } }
}

const PHASES: usize = 4;
const TAPS: usize = 12;

/// The highest magnitude between samples too: 4× oversampled through a
/// Blackman-windowed sinc, each phase normalised to unity gain.
fn true_peak(x: &[f32]) -> f32 {
    let total = TAPS * PHASES;
    let centre = (total as f64 - 1.0) / 2.0;
    let mut coefs = [[0.0f32; PHASES]; TAPS];
    for i in 0..total {
        let t = (i as f64 - centre) / PHASES as f64;
        let sinc = if t.abs() < 1e-12 { 1.0 } else { (std::f64::consts::PI * t).sin() / (std::f64::consts::PI * t) };
        let u = i as f64 / (total as f64 - 1.0);
        let w = 0.42 - 0.5 * (2.0 * std::f64::consts::PI * u).cos() + 0.08 * (4.0 * std::f64::consts::PI * u).cos();
        coefs[i / PHASES][i % PHASES] = (sinc * w) as f32;
    }
    for p in 0..PHASES {
        let s: f32 = coefs.iter().map(|tap| tap[p]).sum();
        if s.abs() > 1e-6 {
            coefs.iter_mut().for_each(|tap| tap[p] /= s);
        }
    }
    let mut hist = [0.0f32; TAPS];
    let mut peak = 0.0f32;
    for &v in x {
        let v = if v.is_finite() { v } else { 0.0 };
        hist.copy_within(0..TAPS - 1, 1);
        hist[0] = v;
        peak = peak.max(v.abs());
        for p in 0..PHASES {
            let y: f32 = coefs.iter().zip(&hist).map(|(c, h)| c[p] * h).sum();
            peak = peak.max(y.abs());
        }
    }
    peak
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f64, amp: f32, seconds: f64, sr: f64) -> Vec<f32> {
        (0..(seconds * sr) as usize).map(|i| (std::f64::consts::TAU * freq * i as f64 / sr).sin() as f32 * amp).collect()
    }

    #[test]
    fn a_full_scale_1khz_tone_measures_as_the_standard_says() {
        // EBU Tech 3341: a 0 dBFS 1 kHz sine in one channel reads −3.01 LUFS.
        let sr = 48_000.0;
        let x = sine(1000.0, 1.0, 10.0, sr);
        let l = measure(&x, &[], sr);
        assert!((l.integrated + 3.01).abs() < 0.05, "{}", l.integrated);
        // In both channels, 3 dB more.
        let s = measure(&x, &x, sr);
        assert!((s.integrated - 0.0).abs() < 0.06, "{}", s.integrated);
        // −20 dB: 20 LU less; true peak about the sample peak for a 1 kHz tone.
        let q: Vec<f32> = x.iter().map(|v| v * 0.1).collect();
        let m = measure(&q, &[], sr);
        assert!((m.integrated + 23.01).abs() < 0.05, "{}", m.integrated);
        assert!((m.true_peak + 20.0).abs() < 0.1, "{}", m.true_peak);
        assert!(measure(&vec![0.0; 48_000], &[], sr).integrated <= -144.0);
    }

    #[test]
    fn the_true_peak_finds_what_lies_between_samples() {
        // A sine at a quarter of the rate, sampled 45° off its peaks: the
        // samples reach 0.707, the wave 1.0.
        let sr = 48_000.0;
        let x: Vec<f32> = (0..4800).map(|i| (std::f64::consts::FRAC_PI_2 * i as f64 + std::f64::consts::FRAC_PI_4).sin() as f32).collect();
        let sample_peak = x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!((sample_peak - 0.7071).abs() < 1e-3);
        let l = measure(&x, &[], sr);
        assert!(l.true_peak > -0.5, "{} dBTP", l.true_peak);
    }

    #[test]
    fn quiet_passages_do_not_pull_the_loudness_down() {
        // Loud then very quiet: the relative gate leaves the quiet out.
        let sr = 48_000.0;
        let mut x = sine(1000.0, 0.5, 5.0, sr);
        x.extend(sine(1000.0, 0.005, 5.0, sr));
        let loud = measure(&sine(1000.0, 0.5, 5.0, sr), &[], sr).integrated;
        assert!((measure(&x, &[], sr).integrated - loud).abs() < 0.2);
    }
}
