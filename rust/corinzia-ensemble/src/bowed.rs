//! A bowed string instrument, physically modelled: the strings, the bow on
//! one of them, and the body they all drive through the bridge.
//!
//! Adapted from Strings (`src/dsp/PhysicalModel.cpp`,
//! https://github.com/kenotron-ms/strings-vst, commit ca0c0aa), under the
//! MIT License (see `THIRD-PARTY-NOTICES.md`). Its bow-reflection equation
//! and waveguide topology are informed by the Synthesis ToolKit's
//! Bowed/BowTable (Perry R. Cook and Gary P. Scavone).
//!
//! - **Strings**: four, each with two transverse polarisations, every one
//!   a waveguide (a delay line each way, with loss and allpass dispersion
//!   in the loop), split at the bowing point. The open strings not played
//!   still ring with the bridge (sympathetic resonance).
//! - **Bow**: stick-slip friction at the contact point, solved each sample
//!   against the string's free velocity: the bow drags the string with it
//!   (sticking) until the string's pull exceeds static friction, then
//!   slips, with a friction that falls with the slip speed and with the
//!   rosin's heat.
//! - **Body**: eight modes behind the bridge, each a mass, spring and
//!   damper (integrated with the trapezoidal rule, so passive), coupled to
//!   both polarisations at an angle. Their admittance also corrects each
//!   string's tuning for the bridge's phase delay.
//!
//! The sound is the bridge's force and velocity, as a pickup on the bridge
//! would hear it, with the DC removed.
//!
//! **Players side by side.** An [`Instrument<N>`] is `N` players of one
//! note (a section's, or `N = 1` a soloist): each has their own strings,
//! body, bow and frequency, but they share the instrument, the string
//! played, the playing controls and the model's rate. Every per-player
//! quantity is an [`Fx<N>`]/[`Dx<N>`] (see [`crate::lanes`]), so the same
//! code computes one player or eight at once; the `side_by_side` test
//! checks that a player in a group sounds exactly as alone.

use crate::lanes::{Dx, Fx};

/// Instruments: violin, viola, cello, double bass.
pub const VIOLIN: usize = 0;
pub const VIOLA: usize = 1;
pub const CELLO: usize = 2;
pub const BASS: usize = 3;

/// One player's instrument.
pub type Player = Instrument<1>;

const TAU: f64 = std::f64::consts::TAU;
/// The open strings (MIDI keys), lowest first.
const OPEN: [[i32; 4]; 4] = [[55, 62, 69, 76], [48, 55, 62, 69], [36, 43, 50, 57], [28, 33, 38, 43]];
/// The strings' wave impedances.
const IMPEDANCE: [[f32; 4]; 4] = [[0.23, 0.20, 0.17, 0.13], [0.36, 0.29, 0.24, 0.20], [0.95, 0.73, 0.55, 0.42], [1.8, 1.5, 1.2, 0.95]];
/// The bodies' mode frequencies (Hz).
const BODY: [[f64; 8]; 4] = [
    [275.0, 460.0, 550.0, 720.0, 1100.0, 1600.0, 2250.0, 3100.0],
    [220.0, 350.0, 430.0, 620.0, 950.0, 1350.0, 1950.0, 2600.0],
    [95.0, 170.0, 240.0, 350.0, 520.0, 850.0, 1250.0, 1800.0],
    [55.0, 100.0, 160.0, 240.0, 380.0, 620.0, 900.0, 1350.0],
];
/// The bodies' masses (kg).
const MASS: [f64; 4] = [0.035, 0.055, 0.16, 0.32];
/// The body's radiation: above `shelf` Hz down 6 dB, and falling 12 dB an
/// octave above `top` Hz (a body radiates its high vibrations ever less;
/// the model's eight modes alone sound far too bright): violin to bass.
const RADIATION: [(f64, f64); 4] = [(800.0, 4500.0), (650.0, 3800.0), (450.0, 2800.0), (300.0, 1800.0)];

/// A biquad filter (direct form I): shared coefficients, each player's
/// own state.
#[derive(Clone, Copy, Default)]
struct Biquad<const N: usize> {
    b: [f32; 3],
    a: [f32; 2],
    x: [Fx<N>; 2],
    y: [Fx<N>; 2],
}

impl<const N: usize> Biquad<N> {
    /// A second-order low-pass (RBJ's cookbook) at `f` Hz.
    fn low_pass(f: f64, q: f64, rate: f64) -> Self {
        let w = TAU * f.min(rate * 0.45) / rate;
        let (s, c) = w.sin_cos();
        let alpha = s / (2.0 * q);
        let a0 = 1.0 + alpha;
        let b1 = (1.0 - c) / a0;
        Biquad { b: [(b1 / 2.0) as f32, b1 as f32, (b1 / 2.0) as f32], a: [(-2.0 * c / a0) as f32, ((1.0 - alpha) / a0) as f32], ..Default::default() }
    }
    /// A high shelf (RBJ's cookbook, slope 1) at `f` Hz, `gain_db` above it.
    fn high_shelf(f: f64, gain_db: f64, rate: f64) -> Self {
        let a = 10f64.powf(gain_db / 40.0);
        let w = TAU * f.min(rate * 0.45) / rate;
        let (s, c) = w.sin_cos();
        let alpha = s / 2.0 * 2f64.sqrt();
        let k = 2.0 * a.sqrt() * alpha;
        let a0 = (a + 1.0) - (a - 1.0) * c + k;
        Biquad {
            b: [(a * ((a + 1.0) + (a - 1.0) * c + k) / a0) as f32, (-2.0 * a * ((a - 1.0) + (a + 1.0) * c) / a0) as f32, (a * ((a + 1.0) + (a - 1.0) * c - k) / a0) as f32],
            a: [(2.0 * ((a - 1.0) - (a + 1.0) * c) / a0) as f32, (((a + 1.0) - (a - 1.0) * c - k) / a0) as f32],
            ..Default::default()
        }
    }
    #[inline(always)]
    fn tick(&mut self, x: Fx<N>) -> Fx<N> {
        let y = x * self.b[0] + self.x[0] * self.b[1] + self.x[1] * self.b[2] - self.y[0] * self.a[0] - self.y[1] * self.a[1];
        self.x = [x, self.x[0]];
        self.y = [y, self.y[0]];
        y
    }
    fn clear(&mut self) {
        (self.x, self.y) = ([Fx::default(); 2], [Fx::default(); 2]);
    }
}

/// The lowest note a delay line is long enough for (Hz).
const LOWEST_HZ: f64 = 30.0;

fn pitch(key: f32) -> f32 {
    440.0 * ((key - 69.0) / 12.0).exp2()
}

/// One body mode: a damped mass on a spring, driven at the bridge; each
/// player's instrument tuned a little differently.
#[derive(Clone, Copy, Default)]
struct BodyMode<const N: usize> {
    position: Dx<N>,
    velocity: Dx<N>,
    compliance: Dx<N>,
    stiffness: Dx<N>,
    resistance: Dx<N>,
    /// `1 / denominator` and `stiffness · step / (2 · mass)`, for `history`.
    inv_denominator: Dx<N>,
    spring_step: Dx<N>,
    mass: f64,
    step: f64,
    /// How it couples to the two polarisations.
    projection: [f64; 2],
}

impl<const N: usize> BodyMode<N> {
    /// Player `l`'s mode.
    #[allow(clippy::too_many_arguments, reason = "one mode's physical description")]
    fn configure(&mut self, l: usize, frequency: f64, quality: f64, mass: f64, angle: f64, amount: f64, rate: f64) {
        self.mass = mass;
        let angular = TAU * frequency.min(rate * 0.35);
        let stiffness = mass * angular * angular;
        let resistance = mass * angular / quality;
        self.step = 1.0 / rate;
        let denominator = 1.0 + resistance * self.step / (2.0 * mass) + stiffness * self.step * self.step / (4.0 * mass);
        self.stiffness.0[l] = stiffness;
        self.resistance.0[l] = resistance;
        self.compliance.0[l] = self.step / (2.0 * mass * denominator);
        self.inv_denominator.0[l] = 1.0 / denominator;
        self.spring_step.0[l] = stiffness * self.step / (2.0 * mass);
        self.projection = [amount * angle.cos(), amount * angle.sin()];
    }
    #[inline(always)]
    fn history(&self) -> Dx<N> {
        (self.velocity - self.spring_step * self.position) * self.inv_denominator
    }
}

/// The body behind the bridge: modes driven by both polarisations.
#[derive(Clone)]
struct Body<const N: usize> {
    modes: [BodyMode<N>; 8],
    force: [Fx<N>; 2],
    rate: f64,
    instrument: usize,
    players: [u32; N],
    coupling: f32,
}

impl<const N: usize> Body<N> {
    fn new(players: [u32; N]) -> Self {
        Body { modes: [BodyMode::default(); 8], force: [Fx::default(); 2], rate: 48000.0, instrument: 0, players, coupling: -1.0 }
    }
    fn prepare(&mut self, rate: f64, instrument: usize) {
        (self.rate, self.instrument) = (rate, instrument);
        self.coupling = -1.0;
        self.set_coupling(0.6);
        self.reset();
    }
    fn set_coupling(&mut self, amount: f32) {
        let amount = amount.clamp(0.0, 1.0);
        if amount == self.coupling {
            return;
        }
        self.coupling = amount;
        for l in 0..N {
            // Each player's instrument a little different.
            let spread = 1.0 + 0.0015 * self.players[l] as f64;
            for (i, m) in self.modes.iter_mut().enumerate() {
                let weight = (amount as f64).sqrt() * if i < 3 { 1.0 } else { 0.7 };
                m.configure(l, BODY[self.instrument][i] * spread, 9.0 + i as f64 * 2.0, MASS[self.instrument] * (1.0 + i as f64 * 0.25), 0.27 + i as f64 * 1.17, weight, self.rate);
            }
        }
    }
    /// A new internal rate (oversampling): the modes keep their motion.
    fn set_rate(&mut self, rate: f64) {
        self.rate = rate;
        let c = self.coupling;
        self.coupling = -1.0;
        self.set_coupling(c);
    }
    fn reset(&mut self) {
        for m in &mut self.modes {
            m.position = Dx::default();
            m.velocity = Dx::default();
        }
        self.force = [Fx::default(); 2];
    }
    /// The bridge's velocity under the strings' incident force (`load`:
    /// their summed impedance); advances the modes.
    #[inline(always)]
    fn scatter(&mut self, incident: [Dx<N>; 2], load: f64) -> [Fx<N>; 2] {
        let zero = Dx::<N>::default();
        let (mut hx, mut hy, mut axx, mut axy, mut ayy) = (zero, zero, zero, zero, zero);
        let histories: [Dx<N>; 8] = std::array::from_fn(|i| self.modes[i].history());
        for (m, &h) in self.modes.iter().zip(&histories) {
            let (x, y) = (m.projection[0], m.projection[1]);
            hx += h * x;
            hy += h * y;
            axx += m.compliance * (x * x);
            axy += m.compliance * (x * y);
            ayy += m.compliance * (y * y);
        }
        let (mxx, myy, mxy) = (axx * load + 1.0, ayy * load + 1.0, axy * load);
        let det = mxx * myy - mxy * mxy;
        let rx = hx + axx * incident[0] + axy * incident[1];
        let ry = hy + axy * incident[0] + ayy * incident[1];
        let vx = (myy * rx - mxy * ry) / det;
        let vy = (mxx * ry - mxy * rx) / det;
        let (fx, fy) = (incident[0] - vx * load, incident[1] - vy * load);
        self.force = [fx.narrow(), fy.narrow()];
        for (m, h) in self.modes.iter_mut().zip(histories) {
            let midpoint = h + m.compliance * (fx * m.projection[0] + fy * m.projection[1]);
            m.position += midpoint * m.step;
            m.velocity = midpoint * 2.0 - m.velocity;
        }
        [vx.narrow(), vy.narrow()]
    }
    /// The phase delay (samples) the bridge adds to player `l`'s string's
    /// round trip at `frequency`: what its tuning has to make up.
    fn reflection_delay(&self, l: usize, frequency: f32, impedance: f32, other_load: f64) -> f32 {
        #[derive(Clone, Copy)]
        struct C(f64, f64);
        let add = |a: C, b: C| C(a.0 + b.0, a.1 + b.1);
        let mul = |a: C, b: C| C(a.0 * b.0 - a.1 * b.1, a.0 * b.1 + a.1 * b.0);
        let scale = |a: C, k: f64| C(a.0 * k, a.1 * k);
        let div = |a: C, b: C| {
            let d = b.0 * b.0 + b.1 * b.1;
            C((a.0 * b.0 + a.1 * b.1) / d, (a.1 * b.0 - a.0 * b.1) / d)
        };
        let angular = TAU * frequency as f64 / self.rate;
        let warped = 2.0 * self.rate * (angular * 0.5).tan();
        let (mut xx, mut xy, mut yy) = (C(0.0, 0.0), C(0.0, 0.0), C(0.0, 0.0));
        for m in &self.modes {
            let response = div(C(0.0, warped), C(m.stiffness.0[l] - m.mass * warped * warped, m.resistance.0[l] * warped));
            xx = add(xx, scale(response, m.projection[0] * m.projection[0]));
            xy = add(xy, scale(response, m.projection[0] * m.projection[1]));
            yy = add(yy, scale(response, m.projection[1] * m.projection[1]));
        }
        let one = C(1.0, 0.0);
        let det = add(mul(add(one, scale(xx, other_load)), add(one, scale(yy, other_load))), scale(mul(xy, xy), -other_load * other_load));
        let cross = add(mul(xx, yy), scale(mul(xy, xy), -1.0));
        let lxx = div(add(xx, scale(cross, other_load)), det);
        let lyy = div(add(yy, scale(cross, other_load)), det);
        let lxy = div(xy, det);
        let projected = add(add(scale(lxx, 0.9775), scale(lxy, 0.2966058)), scale(lyy, 0.0225));
        let z = impedance as f64;
        let reflection = div(add(one, scale(projected, -z)), add(one, scale(projected, z)));
        (reflection.1.atan2(reflection.0) / angular) as f32
    }
}

/// The sliding friction coefficient's shape against the slip speed:
/// `0.4·e^(−slip/0.01) + 0.45·e^(−slip/0.1) + 0.35`, tabulated (the
/// friction is solved by bisection every sample).
const CURVE: usize = 2048;
const CURVE_TOP: f32 = 0.8;

fn friction_table() -> &'static [f32; CURVE + 1] {
    static TABLE: std::sync::OnceLock<[f32; CURVE + 1]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| std::array::from_fn(|i| {
        let x = i as f32 * CURVE_TOP / CURVE as f32;
        0.4 * (-x / 0.01).exp() + 0.45 * (-x / 0.1).exp() + 0.35
    }))
}

/// Stick-slip friction at each player's bow, warmed by its own work.
#[derive(Clone, Copy, Default)]
struct Bow<const N: usize> {
    heat: Fx<N>,
}

impl<const N: usize> Bow<N> {
    /// The friction force on each string for the bow's speed relative to
    /// the string's free velocity (`relative`), the strings' compliance
    /// there, the bows' pressure and the rosin (0..1). A string either
    /// sticks to the bow, or slips at the friction where the curve meets
    /// its load line (found by bisection: the players together, each
    /// keeping what their own case needs).
    #[inline(always)]
    fn solve(&mut self, relative: Fx<N>, compliance: f32, normal: Fx<N>, rosin: f32, rate: f64) -> Fx<N> {
        let normal = normal.map(|n| n.max(0.0));
        let material = Fx::splat(0.8 + 0.4 * rosin) / (self.heat * 0.12 + 1.0);
        let static_limit = normal * 1.2 * material;
        let gripping: [bool; N] = std::array::from_fn(|l| normal.0[l] > 0.0 && compliance > 0.0);
        let sticking = relative / compliance;
        let sticks: [bool; N] = std::array::from_fn(|l| sticking.0[l].abs() <= static_limit.0[l]);
        let mut force = Fx::<N>::default();
        if (0..N).any(|l| gripping[l] && !sticks[l]) {
            let table = friction_table();
            let speed = relative.abs();
            let (mut lo, mut hi) = (normal * 0.35 * material, static_limit);
            let pull = normal * material;
            for _ in 0..14 {
                let candidate = (lo + hi) * 0.5;
                let slip = (speed - candidate * compliance).map(|x| x.max(0.0));
                let curve = Fx(std::array::from_fn(|l| {
                    let x = (slip.0[l] * (CURVE as f32 / CURVE_TOP)).min(CURVE as f32 - 1.0);
                    let i = x as usize;
                    table[i] + (x - i as f32) * (table[i + 1] - table[i])
                }));
                let friction = pull * curve;
                let over: [bool; N] = std::array::from_fn(|l| candidate.0[l] > friction.0[l]);
                hi = Fx::select(over, candidate, hi);
                lo = Fx::select(over, lo, candidate);
            }
            force = Fx(std::array::from_fn(|l| relative.0[l].signum() * (lo.0[l] + hi.0[l]) * 0.5));
        }
        let force = Fx(std::array::from_fn(|l| if !gripping[l] { 0.0 } else if sticks[l] { sticking.0[l] } else { force.0[l] }));
        let slip = relative - force * compliance;
        let power = (force * slip).map(|x| x.max(0.0));
        self.heat = Fx(std::array::from_fn(|l| (self.heat.0[l] + ((power.0[l] * 24.0 - self.heat.0[l] * 3.0) as f64 / rate) as f32).clamp(0.0, 10.0)));
        force
    }
}

/// The players' delay lines, interleaved (each sample holds every
/// player's), read at each player's own fractional delay (linear
/// interpolation); its length a power of two.
#[derive(Clone, Default)]
struct Delay<const N: usize> {
    samples: Vec<[f32; N]>,
    cursor: usize,
    mask: usize,
}

impl<const N: usize> Delay<N> {
    fn prepare(&mut self, capacity: usize) {
        let len = (capacity + 4).next_power_of_two();
        self.samples = vec![[0.0; N]; len];
        self.mask = len - 1;
        self.cursor = 0;
    }
    fn reset(&mut self) {
        self.samples.fill([0.0; N]);
        self.cursor = 0;
    }
    #[inline(always)]
    fn read(&self, delay: Fx<N>) -> Fx<N> {
        let delay = delay.clamp(1.0, (self.mask - 1) as f32);
        let whole: [u32; N] = std::array::from_fn(|l| delay.0[l] as u32);
        let frac = Fx(std::array::from_fn(|l| delay.0[l] - whole[l] as f32));
        let (lo, hi) = whole.iter().fold((u32::MAX, 0), |(lo, hi), &w| (lo.min(w), hi.max(w)));
        // `delay` back is between two samples (`a` the later). The players'
        // delays lie within a few samples of each other (their pitches are
        // cents apart): those few rows are read whole and each player's
        // values picked from them, all side by side.
        let (mut a, mut b) = ([0.0f32; N], [0.0f32; N]);
        if hi - lo <= 3 {
            let mut next = self.samples[self.cursor.wrapping_sub(lo as usize) & self.mask];
            for w in lo..=hi {
                let row = next;
                next = self.samples[self.cursor.wrapping_sub(w as usize + 1) & self.mask];
                for l in 0..N {
                    let here = whole[l] == w;
                    a[l] = if here { row[l] } else { a[l] };
                    b[l] = if here { next[l] } else { b[l] };
                }
            }
        } else {
            // Spread wide: each player's own two samples.
            let first: [usize; N] = std::array::from_fn(|l| self.cursor.wrapping_sub(whole[l] as usize) & self.mask);
            a = std::array::from_fn(|l| self.samples[first[l]][l]);
            b = std::array::from_fn(|l| self.samples[first[l].wrapping_sub(1) & self.mask][l]);
        }
        let (a, b) = (Fx(a), Fx(b));
        a + frac * (b - a)
    }
    #[inline(always)]
    fn write(&mut self, value: Fx<N>) {
        self.samples[self.cursor] = value.0;
        self.cursor = (self.cursor + 1) & self.mask;
    }
}

/// One polarisation of one string, for each player: waves from the nut and
/// from the bridge meeting at the bow.
#[derive(Clone)]
struct Lane<const N: usize> {
    nut: Delay<N>,
    bridge_out: Delay<N>,
    bridge_return: Delay<N>,
    rate: f64,
    retune: f32,
    impedance: f32,
    period: Fx<N>,
    target_period: Fx<N>,
    bow_position: Fx<N>,
    loss_pole: Fx<N>,
    loss_state: Fx<N>,
    reflection: Fx<N>,
    dispersion: Fx<N>,
    dispersion_state: Fx<N>,
    from_nut: Fx<N>,
    from_bridge: Fx<N>,
    to_bridge: Fx<N>,
    tuned: [bool; N],
}

impl<const N: usize> Default for Lane<N> {
    fn default() -> Self {
        Lane {
            nut: Delay::default(),
            bridge_out: Delay::default(),
            bridge_return: Delay::default(),
            rate: 48000.0,
            retune: 0.025,
            impedance: 0.2,
            period: Fx::splat(100.0),
            target_period: Fx::splat(100.0),
            bow_position: Fx::splat(0.15),
            loss_pole: Fx::splat(0.2),
            loss_state: Fx::default(),
            reflection: Fx::splat(0.999),
            dispersion: Fx::splat(-0.3),
            dispersion_state: Fx::default(),
            from_nut: Fx::default(),
            from_bridge: Fx::default(),
            to_bridge: Fx::default(),
            tuned: [false; N],
        }
    }
}

impl<const N: usize> Lane<N> {
    fn prepare(&mut self, rate: f64) {
        self.rate = rate;
        self.retune = (1.0 - (-1.0 / (rate * 0.000823)).exp()) as f32;
        let capacity = (rate / LOWEST_HZ) as usize;
        self.nut.prepare(capacity);
        self.bridge_out.prepare(capacity);
        self.bridge_return.prepare(capacity);
        self.reset();
    }
    /// A new internal rate (oversampling): silences it.
    fn set_rate(&mut self, rate: f64) {
        self.rate = rate;
        self.retune = (1.0 - (-1.0 / (rate * 0.000823)).exp()) as f32;
        self.reset();
    }
    fn reset(&mut self) {
        self.nut.reset();
        self.bridge_out.reset();
        self.bridge_return.reset();
        let zero = Fx::default();
        (self.loss_state, self.dispersion_state, self.from_nut, self.from_bridge, self.to_bridge) = (zero, zero, zero, zero, zero);
        self.tuned = [false; N];
    }
    /// Tunes player `l`'s: the loop's length less the delays of its loss
    /// filter, its dispersion and the bridge; the bowing point; brightness
    /// (loss), stiffness (dispersion) and the decay (seconds to -60 dB).
    #[allow(clippy::too_many_arguments, reason = "one string's physical description")]
    fn tune(&mut self, l: usize, frequency: f32, position: f32, brightness: f32, stiffness: f32, decay: f32, bridge_delay: f32) {
        let frequency = frequency.clamp(LOWEST_HZ as f32, (self.rate * 0.12) as f32);
        self.bow_position.0[l] = position.clamp(0.04, 0.32);
        let pole = (0.65 - 0.5 * brightness).powf((48000.0 / self.rate) as f32);
        let dispersion = -0.15 - 0.45 * stiffness;
        self.loss_pole.0[l] = pole;
        self.dispersion.0[l] = dispersion;
        let angular = TAU * frequency as f64 / self.rate;
        let (pole, d) = (pole as f64, dispersion as f64);
        let lowpass = (pole * angular.sin()).atan2(1.0 - pole * angular.cos()) / angular;
        let allpass = -(-(1.0 - d * d) * angular.sin()).atan2(2.0 * d + (1.0 + d * d) * angular.cos()) / angular;
        self.target_period.0[l] = ((self.rate / frequency as f64 - lowpass - allpass) as f32 + bridge_delay).max(7.0);
        if !self.tuned[l] {
            self.period.0[l] = self.target_period.0[l];
            self.tuned[l] = true;
        }
        self.reflection.0[l] = 0.001f32.powf(1.0 / (frequency * decay));
    }
    #[inline(always)]
    fn read(&mut self, vibrato: Fx<N>) {
        self.period += (self.target_period - self.period) * self.retune;
        let period = self.period * vibrato;
        self.from_nut = self.nut.read(period * (-self.bow_position + 1.0));
        let half = period * self.bow_position * 0.5;
        self.from_bridge = self.bridge_return.read(half);
        self.to_bridge = self.bridge_out.read(half);
    }
    #[inline(always)]
    fn bow(&mut self, force: Fx<N>) {
        let injected = force / (2.0 * self.impedance);
        let towards_nut = self.from_bridge + injected;
        self.loss_state = towards_nut * (-self.loss_pole + 1.0) + self.loss_pole * self.loss_state;
        let dispersed = self.dispersion * self.loss_state + self.dispersion_state;
        self.dispersion_state = self.loss_state - self.dispersion * dispersed;
        self.nut.write(-self.reflection * dispersed);
        self.bridge_out.write(self.from_nut + injected);
    }
    #[inline(always)]
    fn bridge(&mut self, velocity: Fx<N>) {
        self.bridge_return.write(velocity - self.to_bridge);
    }
    #[inline(always)]
    fn free_velocity(&self) -> Fx<N> {
        self.from_nut + self.from_bridge
    }
}

/// How the instrument is played, each 0..1 unless said.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Controls {
    /// The bow's pressure and speed.
    pub force: f32,
    pub speed: f32,
    /// +1 down-bow, -1 up-bow.
    pub direction: f32,
    /// The bow on the string (0: lifted).
    pub engagement: f32,
    /// The contact point: 0 at the bridge (sul ponticello) to 1 over the
    /// fingerboard (sul tasto).
    pub position: f32,
    pub brightness: f32,
    /// How strongly the body takes the strings' vibration (lower: muted).
    pub coupling: f32,
    /// How much the open strings ring along.
    pub sympathetic: f32,
    pub rosin: f32,
}

impl Default for Controls {
    fn default() -> Self {
        Controls { force: 0.5, speed: 0.65, direction: 1.0, engagement: 1.0, position: 0.35, brightness: 0.55, coupling: 0.6, sympathetic: 0.45, rosin: 0.5 }
    }
}

/// `N` players' instruments for one note: four strings, a bow and a body
/// each (see the module notes).
#[derive(Clone)]
pub struct Instrument<const N: usize> {
    strings: [[Lane<N>; 2]; 4],
    body: Body<N>,
    bow: Bow<N>,
    settings: Controls,
    /// The output's sample rate, and the model's (a multiple of it, `os`:
    /// a high note's short string needs finer steps to be placed exactly).
    base_rate: f64,
    rate: f64,
    os: u32,
    instrument: usize,
    played: usize,
    pizzicato: bool,
    velocity: f32,
    /// The players' intonation: what the note's frequency is multiplied
    /// by to sound in tune (see [`intonation`]).
    intonation: f32,
    fundamental: Fx<N>,
    /// A pluck: how long until each player's starts and how long it still
    /// pulls (seconds), how long it pulls in all, and how hard.
    pluck_wait: Fx<N>,
    pluck_time: Fx<N>,
    pluck_width: Fx<N>,
    pluck_strength: Fx<N>,
    tuned: [[f32; N]; 4],
    dc_in: Fx<N>,
    dc_out: Fx<N>,
    dc_pole: f32,
    /// The body's radiation (see [`RADIATION`]).
    shelf: Biquad<N>,
    top: Biquad<N>,
}

impl<const N: usize> Instrument<N> {
    /// `N` players' instruments at a sample rate (`players`: who plays
    /// each, which makes each a little different). Allocates the delay lines.
    pub fn new(rate: f64, players: [u32; N]) -> Self {
        let mut i = Instrument {
            strings: Default::default(),
            body: Body::new(players),
            bow: Bow::default(),
            settings: Controls::default(),
            base_rate: rate,
            rate,
            os: 1,
            instrument: VIOLIN,
            played: 0,
            pizzicato: false,
            velocity: 1.0,
            intonation: 1.0,
            fundamental: Fx::splat(220.0),
            pluck_wait: Fx::default(),
            pluck_time: Fx::default(),
            pluck_width: Fx::splat(0.004),
            pluck_strength: Fx::splat(1.0),
            tuned: [[0.0; N]; 4],
            dc_in: Fx::default(),
            dc_out: Fx::default(),
            dc_pole: (-TAU * 8.0 / rate).exp() as f32,
            shelf: Biquad::default(),
            top: Biquad::default(),
        };
        for lane in i.strings.iter_mut().flatten() {
            lane.prepare(rate);
        }
        i.configure(VIOLIN);
        i
    }

    /// Silences it.
    pub fn reset(&mut self) {
        for lane in self.strings.iter_mut().flatten() {
            lane.reset();
        }
        self.body.reset();
        self.bow = Bow::default();
        (self.pluck_wait, self.pluck_time, self.dc_in, self.dc_out) = (Fx::default(), Fx::default(), Fx::default(), Fx::default());
        self.shelf.clear();
        self.top.clear();
        self.tuned = [[0.0; N]; 4];
    }

    /// Which instrument it is ([`VIOLIN`] … [`BASS`]).
    pub fn configure(&mut self, instrument: usize) {
        self.instrument = instrument.min(BASS);
        let (shelf, top) = RADIATION[self.instrument];
        self.shelf = Biquad::high_shelf(shelf, -6.0, self.base_rate);
        self.top = Biquad::low_pass(top, 0.6, self.base_rate);
        self.body.prepare(self.rate, self.instrument);
        self.tuned = [[0.0; N]; 4];
        for (s, string) in self.strings.iter_mut().enumerate() {
            for lane in string {
                lane.impedance = IMPEDANCE[self.instrument][s];
            }
        }
    }

    pub fn instrument(&self) -> usize {
        self.instrument
    }

    /// A note: on the highest string that has it; plucked or bowed; each
    /// player at their own frequency.
    pub fn note(&mut self, frequency: impl Into<Fx<N>>, key: i32, pluck: bool, velocity: f32) {
        let frequency = frequency.into();
        // A period of at least 64 steps (for the highest player's), as far
        // as the delay lines reach for the lowest open string.
        let highest = frequency.0.iter().fold(1.0f32, |m, &f| m.max(f)) as f64;
        let capacity = self.strings[0][0].nut.mask.saturating_sub(4) as f64;
        let lowest = pitch(OPEN[self.instrument][0] as f32) as f64;
        let mut os = 1;
        while (self.base_rate * os as f64 / highest) < 64.0 && self.base_rate * (os * 2) as f64 / lowest <= capacity && os < 8 {
            os *= 2;
        }
        self.set_oversampling(os);
        self.finger(frequency, key, pluck, velocity);
    }

    /// The next note of a slur: the finger moves, the string sounds on
    /// (its model's rate kept, so nothing restarts).
    pub fn slur(&mut self, frequency: impl Into<Fx<N>>, key: i32, velocity: f32) {
        self.finger(frequency.into(), key, false, velocity);
    }

    fn finger(&mut self, frequency: Fx<N>, key: i32, pluck: bool, velocity: f32) {
        self.intonation = (-intonation(self.base_rate, self.instrument, key) / 1200.0).exp2();
        self.fundamental = frequency * self.intonation;
        self.velocity = velocity;
        self.pizzicato = pluck;
        self.played = (0..4).rfind(|&s| key >= OPEN[self.instrument][s]).unwrap_or(0);
        if pluck {
            self.pluck_time = self.pluck_width;
        }
    }

    /// How the next pluck pulls each player's string: for `width` seconds
    /// (a finger's soft pad is longer than its tip), `strength` times as
    /// hard, after `wait` seconds (a section plucks a little apart).
    pub fn set_pluck(&mut self, width: impl Into<Fx<N>>, strength: impl Into<Fx<N>>, wait: impl Into<Fx<N>>) {
        self.pluck_width = width.into().clamp(0.001, 0.02);
        self.pluck_strength = strength.into().map(|s| s.max(0.0));
        self.pluck_wait = wait.into().map(|w| w.max(0.0));
    }

    fn set_oversampling(&mut self, os: u32) {
        if os == self.os {
            return;
        }
        self.os = os;
        self.rate = self.base_rate * os as f64;
        for lane in self.strings.iter_mut().flatten() {
            lane.set_rate(self.rate);
        }
        self.body.set_rate(self.rate);
        self.tuned = [[0.0; N]; 4];
    }

    /// The sounding frequencies, as they glide and bend.
    pub fn set_frequency(&mut self, frequency: impl Into<Fx<N>>) {
        self.fundamental = frequency.into() * self.intonation;
    }

    /// The playing controls; retunes the strings where needed.
    pub fn controls(&mut self, values: Controls) {
        let s = self.settings;
        let retune = values.position != s.position || values.brightness != s.brightness || values.coupling != s.coupling || values.sympathetic != s.sympathetic;
        self.settings = values;
        self.body.set_coupling(values.coupling);
        for (string, &open) in OPEN[self.instrument].iter().enumerate() {
            let other_load: f32 = (0..4).filter(|&o| o != string).map(|o| self.strings[o][0].impedance * values.sympathetic * values.sympathetic).sum();
            let impedance = self.strings[string][0].impedance;
            for l in 0..N {
                let frequency = if string == self.played { self.fundamental.0[l] } else { pitch(open as f32) };
                if !retune && self.tuned[string][l] == frequency {
                    continue;
                }
                self.tuned[string][l] = frequency;
                let delay = self.body.reflection_delay(l, frequency, impedance, other_load as f64);
                for (p, lane) in self.strings[string].iter_mut().enumerate() {
                    // The two polarisations a hair apart: the beating of a real string.
                    let split = if p == 0 { 1.0 } else { 1.0008 };
                    lane.tune(l, frequency * split, 0.055 + values.position * 0.22, values.brightness, 0.25 + self.instrument as f32 * 0.15, if self.pizzicato { 2.5 } else { 5.0 }, delay);
                }
            }
        }
    }

    /// One sample, each player's. `envelope`: how much of the bow's speed
    /// and pressure reaches the string (0..1); `vibrato`: the played
    /// string's period factor (1: none); `gesture`: the bow's motion (1:
    /// steady; a sine for tremolo, its sign the stroke).
    pub fn tick(&mut self, envelope: impl Into<Fx<N>>, vibrato: impl Into<Fx<N>>, gesture: impl Into<Fx<N>>) -> Fx<N> {
        let (envelope, vibrato, gesture) = (envelope.into(), vibrato.into(), gesture.into());
        // The same code compiled twice: for the processor's wider vectors
        // where it has them (the results are the same to the bit: no fused
        // operations, the same order).
        #[cfg(target_arch = "x86_64")]
        if avx2() {
            // SAFETY: the processor has AVX2 (checked once, above).
            return unsafe { self.tick_avx2(envelope, vibrato, gesture) };
        }
        self.tick_here(envelope, vibrato, gesture)
    }

    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "avx2")]
    unsafe fn tick_avx2(&mut self, envelope: Fx<N>, vibrato: Fx<N>, gesture: Fx<N>) -> Fx<N> {
        self.tick_here(envelope, vibrato, gesture)
    }

    #[inline(always)]
    fn tick_here(&mut self, envelope: Fx<N>, vibrato: Fx<N>, gesture: Fx<N>) -> Fx<N> {
        let mut signal = Fx::default();
        for _ in 0..self.os {
            signal += self.step(envelope, vibrato, gesture);
        }
        let signal = signal / self.os as f32;
        let out = signal - self.dc_in + self.dc_out * self.dc_pole;
        self.dc_in = signal;
        self.dc_out = out;
        self.top.tick(self.shelf.tick(out))
    }

    /// One step of the model at its own rate: the bridge's signal.
    #[inline(always)]
    fn step(&mut self, envelope: Fx<N>, vibrato: Fx<N>, gesture: Fx<N>) -> Fx<N> {
        const AXIS: [f32; 2] = [0.988686, 0.149999];
        let mut load = 0.0;
        let mut incident = [Dx::<N>::default(); 2];
        let one = Fx::splat(1.0);
        for (s, string) in self.strings.iter_mut().enumerate() {
            let played = s == self.played;
            let transformer = if played { 1.0 } else { self.settings.sympathetic };
            let impedance = string[0].impedance;
            load += (transformer * transformer * impedance) as f64;
            for (p, lane) in string.iter_mut().enumerate() {
                lane.read(if played { vibrato } else { one });
                incident[p] += (lane.to_bridge * (2.0 * transformer * impedance)).wide();
            }
        }
        let bridge = self.body.scatter(incident, load);
        let played = &self.strings[self.played];
        let impedance = played[0].impedance;
        let free = played[0].free_velocity() * AXIS[0] + played[1].free_velocity() * AXIS[1];
        let compliance = AXIS[0] * AXIS[0] / (2.0 * impedance) + AXIS[1] * AXIS[1] / (2.0 * impedance);
        let c = self.settings;
        let bow_velocity = gesture.map(|g| c.direction * c.speed * 0.45 * g) * envelope;
        let normal = if self.pizzicato { Fx::default() } else { envelope * (impedance * c.force * 3.5 * c.engagement * (0.4 + 0.6 * self.velocity)) };
        let relative = bow_velocity - free;
        let mut force = self.bow.solve(relative, compliance, normal, c.rosin, self.rate);
        if self.pizzicato {
            // A plucked string: a short half-sine of force, after the wait.
            let dt = (1.0 / self.rate) as f32;
            for l in 0..N {
                if self.pluck_wait.0[l] > 0.0 {
                    self.pluck_wait.0[l] -= dt;
                } else if self.pluck_time.0[l] > 0.0 {
                    force.0[l] = impedance * self.velocity * self.pluck_strength.0[l] * 0.7 * (std::f32::consts::PI * self.pluck_time.0[l] / self.pluck_width.0[l]).sin();
                    self.pluck_time.0[l] = (self.pluck_time.0[l] - dt).max(0.0);
                }
            }
        }
        let zero = Fx::default();
        for (s, string) in self.strings.iter_mut().enumerate() {
            let active = s == self.played;
            let transformer = if active { 1.0 } else { c.sympathetic };
            for (p, lane) in string.iter_mut().enumerate() {
                lane.bow(if active { force * AXIS[p] } else { zero });
                lane.bridge(bridge[p] * transformer);
            }
        }
        let f = self.body.force;
        (f[0] * 0.16 + f[1] * 0.08 + bridge[0] * 11.0 + bridge[1] * 7.0) / IMPEDANCE[self.instrument][0].sqrt()
    }
}

/// Whether this processor has AVX2 (asked once).
#[cfg(target_arch = "x86_64")]
fn avx2() -> bool {
    static HAS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *HAS.get_or_init(|| std::arch::is_x86_feature_detected!("avx2"))
}

/// How many cents sharp a note plays before its player corrects it: the
/// model's own deviation, measured over each instrument's range at the
/// usual sample rates (`bowed_tuning.rs`, made by the `make_tuning_table`
/// test). A player hears it and puts the finger that much lower, as on
/// the real instrument; this is that correction. 0 outside the table.
pub fn intonation(rate: f64, instrument: usize, key: i32) -> f32 {
    use crate::bowed_tuning::{CENTS, LOWEST, RATES};
    let r = (0..RATES.len()).min_by(|&a, &b| (RATES[a] - rate).abs().total_cmp(&(RATES[b] - rate).abs())).unwrap_or(0);
    let table = CENTS[r][instrument.min(BASS)];
    usize::try_from(key - LOWEST[instrument.min(BASS)]).ok().and_then(|i| table.get(i)).copied().unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The strongest period (samples) by autocorrelation, between `lo` and `hi`.
    pub(super) fn period_of(x: &[f32], lo: usize, hi: usize) -> f32 {
        let corr = |lag: usize| x.iter().zip(&x[lag..]).map(|(a, b)| a * b).sum::<f32>();
        let peak = |lo: usize, hi: usize| {
            let best = (lo..hi).max_by(|a, b| corr(*a).total_cmp(&corr(*b))).unwrap();
            // Parabolic refinement.
            let (a, b, c) = (corr(best - 1), corr(best), corr(best + 1));
            best as f32 + 0.5 * (a - c) / (a - 2.0 * b + c)
        };
        let coarse = peak(lo, hi);
        // Refined over many periods: the peak near n periods, divided by n.
        let n = ((x.len() / 3) as f32 / coarse).floor().max(1.0);
        let at = (coarse * n).round() as usize;
        let reach = (coarse * 0.25).max(2.0) as usize;
        peak(at - reach, at + reach) / n
    }

    fn bowed(instrument: usize, key: i32, seconds: f32) -> Vec<f32> {
        let rate = 48000.0;
        let mut i = Player::new(rate, [0]);
        i.configure(instrument);
        i.note(pitch(key as f32), key, false, 0.8);
        i.controls(Controls::default());
        (0..(rate as f32 * seconds) as usize).map(|_| i.tick(1.0, 1.0, 1.0).sum()).collect()
    }

    #[test]
    fn a_bowed_note_sounds_at_its_pitch() {
        for (instrument, key) in [(VIOLIN, 55), (VIOLIN, 69), (VIOLIN, 81), (VIOLIN, 91), (VIOLIN, 98), (VIOLA, 60), (CELLO, 41), (CELLO, 48), (BASS, 32), (BASS, 34)] {
            let x = bowed(instrument, key, 1.0);
            assert!(x.iter().all(|v| v.is_finite()), "finite");
            let rms = (x[24000..].iter().map(|v| v * v).sum::<f32>() / 24000.0).sqrt();
            assert!(rms > 1e-3, "{instrument} {key}: sounds ({rms})");
            let hz = pitch(key as f32);
            let expected = 48000.0 / hz;
            let found = period_of(&x[24000..], (expected * 0.8) as usize, (expected * 1.25) as usize);
            let cents = 1200.0 * (expected / found).log2();
            assert!(cents.abs() < 8.0, "{instrument} {key}: {cents:.1} cents off");
        }
    }

    #[test]
    fn it_falls_silent_when_the_bow_lifts() {
        let mut i = Player::new(48000.0, [0]);
        i.note(440.0, 69, false, 0.8);
        i.controls(Controls::default());
        for _ in 0..24000 {
            i.tick(1.0, 1.0, 1.0).sum();
        }
        let tail: Vec<f32> = (0..96000).map(|_| i.tick(0.0, 1.0, 1.0).sum()).collect();
        let late = tail[90000..].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(late < 1e-3, "rings on: {late}");
    }

    #[test]
    fn a_plucked_note_decays() {
        let mut i = Player::new(48000.0, [0]);
        i.configure(CELLO);
        i.note(pitch(48.0), 48, true, 0.9);
        i.controls(Controls::default());
        let x: Vec<f32> = (0..96000).map(|_| i.tick(0.0, 1.0, 1.0).sum()).collect();
        let rms = |a: &[f32]| (a.iter().map(|v| v * v).sum::<f32>() / a.len() as f32).sqrt();
        assert!(rms(&x[2400..7200]) > 4.0 * rms(&x[72000..]), "decays");
    }
}

#[cfg(test)]
mod side_by_side {
    use super::*;

    /// Eight players computed together sound exactly as each computed
    /// alone: the same code, the same arithmetic (bowed, then plucked).
    #[test]
    fn a_player_in_a_group_sounds_exactly_as_alone() {
        let rate = 48000.0;
        let players: [u32; 8] = [3, 4, 5, 6, 7, 8, 9, 10];
        // Each a little out of tune, entering and bowing in their own way.
        let freq: [f32; 8] = std::array::from_fn(|l| 440.0 * (1.0 + 0.002 * l as f32));
        let env = |l: usize, t: usize| if t > 40 * l { 1.0 } else { 0.0 };
        let vib = |l: usize, t: usize| 1.0 + 0.003 * ((t as f32 * 0.001 + l as f32).sin());
        for pluck in [false, true] {
            let mut group = Instrument::<8>::new(rate, players);
            group.configure(CELLO);
            group.set_pluck(Fx(std::array::from_fn(|l| 0.003 + 0.0005 * l as f32)), 1.0, Fx(std::array::from_fn(|l| 0.002 * l as f32)));
            group.note(Fx(freq), 57, pluck, 0.7);
            group.controls(Controls::default());
            let mut alone: Vec<Player> = (0..8)
                .map(|l| {
                    let mut p = Player::new(rate, [players[l]]);
                    p.configure(CELLO);
                    p.set_pluck(0.003 + 0.0005 * l as f32, 1.0, 0.002 * l as f32);
                    p.note(freq[l], 57, pluck, 0.7);
                    p.controls(Controls::default());
                    p
                })
                .collect();
            for t in 0..9600 {
                let g = group.tick(Fx(std::array::from_fn(|l| env(l, t))), Fx(std::array::from_fn(|l| vib(l, t))), 1.0);
                for (l, p) in alone.iter_mut().enumerate() {
                    let a = p.tick(env(l, t), vib(l, t), 1.0).sum();
                    assert!(g.0[l] == a, "player {l} at sample {t} (pluck {pluck}): {} in the group, {a} alone", g.0[l]);
                }
            }
        }
    }
}

#[cfg(test)]
mod survey {
    use super::*;

    /// Cents sharp a note sounds (with the current intonation table).
    fn error(rate: f64, instrument: usize, key: i32) -> f32 {
        let mut i = Player::new(rate, [0]);
        i.configure(instrument);
        i.note(pitch(key as f32), key, false, 0.8);
        i.controls(Controls::default());
        let n = (rate * 0.6) as usize;
        let x: Vec<f32> = (0..n).map(|_| i.tick(1.0, 1.0, 1.0).sum()).collect();
        let expected = rate as f32 / pitch(key as f32);
        let found = super::tests::period_of(&x[n / 3..], (expected * 0.8) as usize, (expected * 1.25) as usize + 2);
        1200.0 * (expected / found).log2()
    }

    /// Prints the largest error left over each instrument's range (run by hand).
    #[test]
    #[ignore]
    fn residual_tuning() {
        use crate::bowed_tuning::{HIGHEST, LOWEST, RATES};
        for &rate in &RATES {
            for instrument in VIOLIN..=BASS {
                let errs: Vec<(i32, f32)> = (LOWEST[instrument]..=HIGHEST[instrument]).map(|k| (k, error(rate, instrument, k))).collect();
                let worst = errs.iter().max_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).unwrap();
                let over: Vec<String> = errs.iter().filter(|e| e.1.abs() > 5.0).map(|e| format!("{}:{:+.0}", e.0, e.1)).collect();
                println!("{rate} {instrument}: worst {}:{:+.1}; over 5 cents: {}", worst.0, worst.1, over.join(" "));
            }
        }
    }

    /// Writes `src/bowed_tuning.rs`: run by hand when the model changes
    /// (`cargo test --release -p corinzia-ensemble make_tuning_table -- --ignored`).
    #[test]
    #[ignore]
    fn make_tuning_table() {
        use crate::bowed_tuning::{HIGHEST, LOWEST, RATES};
        let mut out = String::from("//! The bowed strings' intonation table (see `bowed::intonation`): made\n//! by the `make_tuning_table` test; not edited by hand.\n\n");
        out += &format!("pub const RATES: [f64; {}] = {:?};\n", RATES.len(), RATES);
        out += &format!("pub const LOWEST: [i32; 4] = {LOWEST:?};\n/// The highest key measured (the table's length), for the generator.\n#[cfg_attr(not(test), allow(dead_code))]\npub const HIGHEST: [i32; 4] = {HIGHEST:?};\n\n");
        out += "/// Cents sharp, per sample rate, instrument and key from `LOWEST`.\npub static CENTS: [[&[f32]; 4]; RATES.len()] = [\n";
        let mut worst: f32 = 0.0;
        for &rate in &RATES {
            out += "    [\n";
            for instrument in VIOLIN..=BASS {
                let keys = LOWEST[instrument]..=HIGHEST[instrument];
                // Measured, corrected, measured again: the correction moves
                // the note against the open strings, so several times.
                let mut cents: Vec<f32> = keys.clone().map(|_| 0.0).collect();
                for _ in 0..7 {
                    let table: Vec<f32> = cents.clone();
                    for (k, key) in keys.clone().enumerate() {
                        let mut i = Player::new(rate, [0]);
                        i.configure(instrument);
                        // Played with this correction, measured.
                        let correction = (-table[k] / 1200.0).exp2();
                        i.note(pitch(key as f32) * correction, key, false, 0.8);
                        i.intonation = 1.0;
                        i.fundamental = Fx::splat(pitch(key as f32) * correction);
                        i.controls(Controls::default());
                        let n = (rate * 0.6) as usize;
                        let x: Vec<f32> = (0..n).map(|_| i.tick(1.0, 1.0, 1.0).sum()).collect();
                        let expected = rate as f32 / pitch(key as f32);
                        let found = super::tests::period_of(&x[n / 3..], (expected * 0.8) as usize, (expected * 1.25) as usize + 2);
                        // Damped: the open strings' pull changes with the correction.
                        cents[k] = table[k] + 0.75 * 1200.0 * (expected / found).log2();
                    }
                }
                let line: Vec<String> = cents.iter().map(|c| format!("{c:.1}")).collect();
                out += &format!("        &[{}],\n", line.join(", "));
                let _ = &mut worst;
            }
            out += "    ],\n";
        }
        out += "];\n";
        std::fs::write(concat!(env!("CARGO_MANIFEST_DIR"), "/src/bowed_tuning.rs"), out).unwrap();
        for &rate in &RATES {
            for instrument in VIOLIN..=BASS {
                for key in (LOWEST[instrument]..=HIGHEST[instrument]).step_by(5) {
                    worst = worst.max(error(rate, instrument, key).abs());
                }
            }
        }
        println!("worst after: {worst:.1} cents (sampled; the table is read on the next build)");
    }
}

#[cfg(test)]
mod bench {
    use super::*;

    /// Prints a section of eight's cost, together and one by one (run by
    /// hand, in release).
    #[test]
    #[ignore]
    fn cost_of_a_group() {
        let rate = 48000.0;
        let seconds = 5;
        let mut group = Instrument::<8>::new(rate, [0, 1, 2, 3, 4, 5, 6, 7]);
        group.note(Fx(std::array::from_fn(|l| 440.0 + l as f32)), 69, false, 0.8);
        group.controls(Controls::default());
        let t = std::time::Instant::now();
        let mut acc = 0.0;
        for _ in 0..(rate as usize * seconds) {
            acc += group.tick(1.0, 1.0, 1.0).sum();
        }
        let together = t.elapsed().as_secs_f64() / seconds as f64;
        let mut alone: Vec<Player> = (0..8)
            .map(|l| {
                let mut p = Player::new(rate, [l]);
                p.note(440.0 + l as f32, 69, false, 0.8);
                p.controls(Controls::default());
                p
            })
            .collect();
        let t = std::time::Instant::now();
        for _ in 0..(rate as usize * seconds) {
            for p in alone.iter_mut() {
                acc += p.tick(1.0, 1.0, 1.0).sum();
            }
        }
        let apart = t.elapsed().as_secs_f64() / seconds as f64;
        println!("eight players: together {:.1} ms a second of sound, one by one {:.1} ms ({:.2}x; {acc})", together * 1000.0, apart * 1000.0, apart / together);
        // The bow lifted: no friction to solve.
        let t = std::time::Instant::now();
        for _ in 0..(rate as usize * seconds) {
            acc += group.tick(0.0, 1.0, 1.0).sum();
        }
        let together = t.elapsed().as_secs_f64() / seconds as f64;
        let t = std::time::Instant::now();
        for _ in 0..(rate as usize * seconds) {
            for p in alone.iter_mut() {
                acc += p.tick(0.0, 1.0, 1.0).sum();
            }
        }
        let apart = t.elapsed().as_secs_f64() / seconds as f64;
        println!("  bow lifted: together {:.1} ms, one by one {:.1} ms ({:.2}x; {acc})", together * 1000.0, apart * 1000.0, apart / together);
    }

    /// Prints the cost of a player-second (run by hand, in release).
    #[test]
    #[ignore]
    fn cost_per_player() {
        let rate = 48000.0;
        for (label, controls, envelope) in [("bowed", Controls::default(), 1.0), ("bow lifted (no friction)", Controls::default(), 0.0), ("no sympathetic strings", Controls { sympathetic: 0.0, ..Controls::default() }, 1.0)] {
            let mut i = Player::new(rate, [0]);
            i.note(440.0, 69, false, 0.8);
            i.controls(controls);
            let t = std::time::Instant::now();
            let mut acc = 0.0;
            for _ in 0..(rate as usize * 10) {
                acc += i.tick(envelope, 1.0, 1.0).sum();
            }
            let per_second = t.elapsed().as_secs_f64() / 10.0;
            println!("{label}: {:.2} ms per second of sound ({:.0} ns a sample; {acc})", per_second * 1000.0, per_second * 1e9 / rate);
        }
    }
}
