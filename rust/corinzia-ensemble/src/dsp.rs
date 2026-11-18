//! Small signal-processing parts the instruments are built from.

use std::f32::consts::{PI, TAU};

/// Decibels to a linear gain.
pub fn db(x: f32) -> f32 {
    10f32.powf(x / 20.0)
}

/// A state-variable filter (topology-preserving transform): low-, band-
/// and high-pass outputs of one 12 dB/octave stage.
#[derive(Clone, Copy, Default)]
pub struct Svf {
    g: f32,
    k: f32,
    a1: f32,
    a2: f32,
    a3: f32,
    ic1: f32,
    ic2: f32,
}

#[derive(Clone, Copy, Default)]
pub struct SvfOut {
    pub low: f32,
    pub band: f32,
    pub high: f32,
}

impl Svf {
    /// Cutoff in Hz (kept below Nyquist), resonance `q` (0.5 .. 20).
    pub fn set(&mut self, cutoff: f32, q: f32, sample_rate: f32) {
        let f = cutoff.clamp(10.0, sample_rate * 0.45);
        self.g = (PI * f / sample_rate).tan();
        self.k = 1.0 / q.max(0.3);
        self.a1 = 1.0 / (1.0 + self.g * (self.g + self.k));
        self.a2 = self.g * self.a1;
        self.a3 = self.g * self.a2;
    }
    pub fn tick(&mut self, x: f32) -> SvfOut {
        let v3 = x - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        SvfOut { low: v2, band: v1, high: x - self.k * v1 - v2 }
    }
    pub fn reset(&mut self) {
        self.ic1 = 0.0;
        self.ic2 = 0.0;
    }
}

/// The PolyBLEP correction for a step at phase 0 (`dt`: phase step).
fn blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

/// A band-limited sawtooth at phase `t` (0..1).
pub fn saw(t: f32, dt: f32) -> f32 {
    2.0 * t - 1.0 - blep(t, dt)
}

/// A band-limited pulse of width `w` (0.5: square).
pub fn pulse(t: f32, dt: f32, w: f32) -> f32 {
    let mut y = if t < w { 1.0 } else { -1.0 };
    y += blep(t, dt);
    y -= blep((t - w + 1.0).fract(), dt);
    y - (2.0 * w - 1.0)
}

pub fn sine(t: f32) -> f32 {
    (t * TAU).sin()
}

/// A small fast random generator (xorshift), deterministic per seed.
#[derive(Clone, Copy)]
pub struct Rng(pub u32);

impl Default for Rng {
    fn default() -> Self {
        Rng(0x1234_5678)
    }
}

impl Rng {
    pub fn next(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }
    /// -1..1
    pub fn noise(&mut self) -> f32 {
        (self.next() >> 8) as f32 / (1u32 << 23) as f32 - 1.0
    }
    /// 0..1
    pub fn unit(&mut self) -> f32 {
        (self.next() >> 8) as f32 / (1u32 << 24) as f32
    }
}

/// The coefficient of a one-pole smoother with time constant `seconds`.
pub fn smoothing(seconds: f32, sample_rate: f32) -> f32 {
    1.0 - (-1.0 / (seconds.max(1e-4) * sample_rate)).exp()
}

/// The per-sample factor that decays by 60 dB in `seconds`.
pub fn decay(seconds: f32, sample_rate: f32) -> f32 {
    (-6.907_755 / (seconds.max(1e-3) * sample_rate)).exp()
}
