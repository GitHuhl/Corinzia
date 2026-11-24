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

/// Instruments: violin, viola, cello, double bass.
pub const VIOLIN: usize = 0;
pub const VIOLA: usize = 1;
pub const CELLO: usize = 2;
pub const BASS: usize = 3;

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
/// The lowest note a delay line is long enough for (Hz).
const LOWEST_HZ: f64 = 20.0;

fn pitch(key: f32) -> f32 {
    440.0 * ((key - 69.0) / 12.0).exp2()
}

/// One body mode: a damped mass on a spring, driven at the bridge.
#[derive(Clone, Copy, Default)]
struct BodyMode {
    position: f64,
    velocity: f64,
    compliance: f64,
    denominator: f64,
    stiffness: f64,
    mass: f64,
    resistance: f64,
    step: f64,
    /// `1 / denominator` and `stiffness · step / (2 · mass)`, for `history`.
    inv_denominator: f64,
    spring_step: f64,
    /// How it couples to the two polarisations.
    projection: [f64; 2],
}

impl BodyMode {
    fn configure(&mut self, frequency: f64, quality: f64, mass: f64, angle: f64, amount: f64, rate: f64) {
        self.mass = mass;
        let angular = TAU * frequency.min(rate * 0.35);
        self.stiffness = mass * angular * angular;
        self.resistance = mass * angular / quality;
        self.step = 1.0 / rate;
        self.denominator = 1.0 + self.resistance * self.step / (2.0 * mass) + self.stiffness * self.step * self.step / (4.0 * mass);
        self.compliance = self.step / (2.0 * mass * self.denominator);
        self.inv_denominator = 1.0 / self.denominator;
        self.spring_step = self.stiffness * self.step / (2.0 * mass);
        self.projection = [amount * angle.cos(), amount * angle.sin()];
    }
    fn history(&self) -> f64 {
        (self.velocity - self.spring_step * self.position) * self.inv_denominator
    }
}

/// The body behind the bridge: modes driven by both polarisations.
#[derive(Clone, Default)]
struct Body {
    modes: [BodyMode; 8],
    force: [f32; 2],
    rate: f64,
    instrument: usize,
    player: u32,
    coupling: f32,
}

impl Body {
    fn prepare(&mut self, rate: f64, instrument: usize, player: u32) {
        (self.rate, self.instrument, self.player) = (rate, instrument, player);
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
        // Each player's instrument a little different.
        let spread = 1.0 + 0.0015 * self.player as f64;
        for (i, m) in self.modes.iter_mut().enumerate() {
            let weight = (amount as f64).sqrt() * if i < 3 { 1.0 } else { 0.7 };
            m.configure(BODY[self.instrument][i] * spread, 9.0 + i as f64 * 2.0, MASS[self.instrument] * (1.0 + i as f64 * 0.25), 0.27 + i as f64 * 1.17, weight, self.rate);
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
            m.position = 0.0;
            m.velocity = 0.0;
        }
        self.force = [0.0; 2];
    }
    /// The bridge's velocity under the strings' incident force (`load`:
    /// their summed impedance); advances the modes.
    fn scatter(&mut self, incident: [f64; 2], load: f64) -> [f32; 2] {
        let (mut hx, mut hy, mut axx, mut axy, mut ayy) = (0.0, 0.0, 0.0, 0.0, 0.0);
        let histories: [f64; 8] = std::array::from_fn(|i| self.modes[i].history());
        for (m, &h) in self.modes.iter().zip(&histories) {
            let (x, y) = (m.projection[0], m.projection[1]);
            hx += x * h;
            hy += y * h;
            axx += x * x * m.compliance;
            axy += x * y * m.compliance;
            ayy += y * y * m.compliance;
        }
        let (mxx, myy, mxy) = (1.0 + load * axx, 1.0 + load * ayy, load * axy);
        let det = mxx * myy - mxy * mxy;
        let rx = hx + axx * incident[0] + axy * incident[1];
        let ry = hy + axy * incident[0] + ayy * incident[1];
        let vx = (myy * rx - mxy * ry) / det;
        let vy = (mxx * ry - mxy * rx) / det;
        let (fx, fy) = (incident[0] - load * vx, incident[1] - load * vy);
        self.force = [fx as f32, fy as f32];
        for (m, h) in self.modes.iter_mut().zip(histories) {
            let midpoint = h + m.compliance * (m.projection[0] * fx + m.projection[1] * fy);
            m.position += m.step * midpoint;
            m.velocity = 2.0 * midpoint - m.velocity;
        }
        [vx as f32, vy as f32]
    }
    /// The phase delay (samples) the bridge adds to a string's round trip
    /// at `frequency`: what its tuning has to make up.
    fn reflection_delay(&self, frequency: f32, impedance: f32, other_load: f64) -> f32 {
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
            let response = div(C(0.0, warped), C(m.stiffness - m.mass * warped * warped, m.resistance * warped));
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
fn friction_curve(slip: f32) -> f32 {
    const N: usize = 2048;
    const TOP: f32 = 0.8;
    static TABLE: std::sync::OnceLock<[f32; N + 1]> = std::sync::OnceLock::new();
    let t = TABLE.get_or_init(|| std::array::from_fn(|i| {
        let x = i as f32 * TOP / N as f32;
        0.4 * (-x / 0.01).exp() + 0.45 * (-x / 0.1).exp() + 0.35
    }));
    let x = (slip.max(0.0) * (N as f32 / TOP)).min(N as f32 - 1.0);
    let i = x as usize;
    t[i] + (x - i as f32) * (t[i + 1] - t[i])
}

/// Stick-slip friction at the bow, warmed by its own work.
#[derive(Clone, Copy, Default)]
struct Bow {
    heat: f32,
}

impl Bow {
    /// The friction force on the string for the bow's speed relative to the
    /// string's free velocity (`relative`), the string's compliance there,
    /// the bow's pressure and the rosin (0..1). Also the slip.
    fn solve(&mut self, relative: f32, compliance: f32, normal: f32, rosin: f32, rate: f64) -> (f32, f32) {
        let normal = normal.max(0.0);
        let material = (0.8 + 0.4 * rosin) / (1.0 + self.heat * 0.12);
        let static_limit = normal * 1.2 * material;
        let mut force = 0.0;
        if normal > 0.0 && compliance > 0.0 {
            let sticking = relative / compliance;
            if sticking.abs() <= static_limit {
                force = sticking;
            } else {
                // Slipping: the friction where its curve meets the string's
                // load line, by bisection.
                let speed = relative.abs();
                let (mut lo, mut hi) = (normal * 0.35 * material, static_limit);
                for _ in 0..14 {
                    let candidate = (lo + hi) * 0.5;
                    let slip = (speed - compliance * candidate).max(0.0);
                    let friction = normal * material * friction_curve(slip);
                    if candidate > friction {
                        hi = candidate;
                    } else {
                        lo = candidate;
                    }
                }
                force = relative.signum() * (lo + hi) * 0.5;
            }
        }
        let slip = relative - compliance * force;
        let power = (force * slip).max(0.0);
        self.heat = (self.heat + ((power * 24.0 - self.heat * 3.0) as f64 / rate) as f32).clamp(0.0, 10.0);
        (force, slip)
    }
}

/// A delay line read at a fractional delay (linear interpolation); its
/// length a power of two.
#[derive(Clone, Default)]
struct Delay {
    samples: Vec<f32>,
    cursor: usize,
    mask: usize,
}

impl Delay {
    fn prepare(&mut self, capacity: usize) {
        let len = (capacity + 4).next_power_of_two();
        self.samples = vec![0.0; len];
        self.mask = len - 1;
        self.cursor = 0;
    }
    fn reset(&mut self) {
        self.samples.fill(0.0);
        self.cursor = 0;
    }
    fn read(&self, delay: f32) -> f32 {
        let delay = delay.clamp(1.0, (self.mask - 1) as f32);
        let whole = delay as usize;
        let frac = delay - whole as f32;
        // `delay` back is between these two (`first` the later).
        let first = self.cursor.wrapping_sub(whole) & self.mask;
        let second = first.wrapping_sub(1) & self.mask;
        self.samples[first] + frac * (self.samples[second] - self.samples[first])
    }
    fn write(&mut self, value: f32) {
        self.samples[self.cursor] = value;
        self.cursor = (self.cursor + 1) & self.mask;
    }
}

/// One polarisation of one string: waves from the nut and from the bridge
/// meeting at the bow.
#[derive(Clone)]
struct Lane {
    nut: Delay,
    bridge_out: Delay,
    bridge_return: Delay,
    rate: f64,
    period: f32,
    target_period: f32,
    bow_position: f32,
    retune: f32,
    loss_pole: f32,
    loss_state: f32,
    reflection: f32,
    dispersion: f32,
    dispersion_state: f32,
    from_nut: f32,
    from_bridge: f32,
    to_bridge: f32,
    tuned: bool,
    impedance: f32,
}

impl Default for Lane {
    fn default() -> Self {
        Lane {
            nut: Delay::default(),
            bridge_out: Delay::default(),
            bridge_return: Delay::default(),
            rate: 48000.0,
            period: 100.0,
            target_period: 100.0,
            bow_position: 0.15,
            retune: 0.025,
            loss_pole: 0.2,
            loss_state: 0.0,
            reflection: 0.999,
            dispersion: -0.3,
            dispersion_state: 0.0,
            from_nut: 0.0,
            from_bridge: 0.0,
            to_bridge: 0.0,
            tuned: false,
            impedance: 0.2,
        }
    }
}

impl Lane {
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
        (self.loss_state, self.dispersion_state, self.from_nut, self.from_bridge, self.to_bridge) = (0.0, 0.0, 0.0, 0.0, 0.0);
        self.tuned = false;
    }
    /// Tunes it: the loop's length less the delays of its loss filter, its
    /// dispersion and the bridge; the bowing point; brightness (loss),
    /// stiffness (dispersion) and the decay (seconds to -60 dB, in periods).
    fn tune(&mut self, frequency: f32, position: f32, brightness: f32, stiffness: f32, decay: f32, bridge_delay: f32) {
        let frequency = frequency.clamp(LOWEST_HZ as f32, (self.rate * 0.12) as f32);
        self.bow_position = position.clamp(0.04, 0.32);
        self.loss_pole = (0.65 - 0.5 * brightness).powf((48000.0 / self.rate) as f32);
        self.dispersion = -0.15 - 0.45 * stiffness;
        let angular = TAU * frequency as f64 / self.rate;
        let (pole, d) = (self.loss_pole as f64, self.dispersion as f64);
        let lowpass = (pole * angular.sin()).atan2(1.0 - pole * angular.cos()) / angular;
        let allpass = -(-(1.0 - d * d) * angular.sin()).atan2(2.0 * d + (1.0 + d * d) * angular.cos()) / angular;
        self.target_period = ((self.rate / frequency as f64 - lowpass - allpass) as f32 + bridge_delay).max(7.0);
        if !self.tuned {
            self.period = self.target_period;
            self.tuned = true;
        }
        self.reflection = 0.001f32.powf(1.0 / (frequency * decay));
    }
    fn read(&mut self, vibrato: f32) {
        self.period += self.retune * (self.target_period - self.period);
        let period = self.period * vibrato;
        self.from_nut = self.nut.read(period * (1.0 - self.bow_position));
        self.from_bridge = self.bridge_return.read(period * self.bow_position * 0.5);
        self.to_bridge = self.bridge_out.read(period * self.bow_position * 0.5);
    }
    fn bow(&mut self, force: f32) {
        let injected = force / (2.0 * self.impedance);
        let towards_nut = self.from_bridge + injected;
        self.loss_state = (1.0 - self.loss_pole) * towards_nut + self.loss_pole * self.loss_state;
        let dispersed = self.dispersion * self.loss_state + self.dispersion_state;
        self.dispersion_state = self.loss_state - self.dispersion * dispersed;
        self.nut.write(-self.reflection * dispersed);
        self.bridge_out.write(self.from_nut + injected);
    }
    fn bridge(&mut self, velocity: f32) {
        self.bridge_return.write(velocity - self.to_bridge);
    }
    fn free_velocity(&self) -> f32 {
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

/// One player's instrument: four strings, a bow and a body.
#[derive(Clone)]
pub struct Instrument {
    strings: [[Lane; 2]; 4],
    body: Body,
    bow: Bow,
    settings: Controls,
    /// The output's sample rate, and the model's (a multiple of it, `os`:
    /// a high note's short string needs finer steps to be placed exactly).
    base_rate: f64,
    rate: f64,
    os: u32,
    player: u32,
    instrument: usize,
    played: usize,
    fundamental: f32,
    /// The player's intonation: what the note's frequency is multiplied
    /// by to sound in tune (see [`intonation`]).
    intonation: f32,
    velocity: f32,
    pluck_time: f32,
    tuned: [f32; 4],
    dc_in: f32,
    dc_out: f32,
    dc_pole: f32,
    pizzicato: bool,
}

impl Instrument {
    /// One player's instrument at a sample rate (`player`: who plays it,
    /// which makes each a little different). Allocates its delay lines.
    pub fn new(rate: f64, player: u32) -> Instrument {
        let mut i = Instrument {
            strings: Default::default(),
            body: Body::default(),
            bow: Bow::default(),
            settings: Controls::default(),
            base_rate: rate,
            rate,
            os: 1,
            player,
            instrument: VIOLIN,
            played: 0,
            fundamental: 220.0,
            intonation: 1.0,
            velocity: 1.0,
            pluck_time: 0.0,
            tuned: [0.0; 4],
            dc_in: 0.0,
            dc_out: 0.0,
            dc_pole: (-TAU * 8.0 / rate).exp() as f32,
            pizzicato: false,
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
        (self.pluck_time, self.dc_in, self.dc_out) = (0.0, 0.0, 0.0);
        self.tuned = [0.0; 4];
    }

    /// Which instrument it is ([`VIOLIN`] … [`BASS`]).
    pub fn configure(&mut self, instrument: usize) {
        self.instrument = instrument.min(BASS);
        self.body.prepare(self.rate, self.instrument, self.player);
        self.tuned = [0.0; 4];
        for (s, string) in self.strings.iter_mut().enumerate() {
            for lane in string {
                lane.impedance = IMPEDANCE[self.instrument][s];
            }
        }
    }

    pub fn instrument(&self) -> usize {
        self.instrument
    }

    /// A note: on the highest string that has it; plucked or bowed.
    pub fn note(&mut self, frequency: f32, key: i32, pluck: bool, velocity: f32) {
        // A period of at least 64 steps, as far as the delay lines reach
        // for the lowest open string.
        let capacity = self.strings[0][0].nut.mask.saturating_sub(4) as f64;
        let lowest = pitch(OPEN[self.instrument][0] as f32) as f64;
        let mut os = 1;
        while (self.base_rate * os as f64 / frequency.max(1.0) as f64) < 64.0 && self.base_rate * (os * 2) as f64 / lowest <= capacity && os < 8 {
            os *= 2;
        }
        self.set_oversampling(os);
        self.intonation = (-intonation(self.base_rate, self.instrument, key) / 1200.0).exp2();
        self.fundamental = frequency * self.intonation;
        self.velocity = velocity;
        self.pizzicato = pluck;
        self.played = (0..4).rfind(|&s| key >= OPEN[self.instrument][s]).unwrap_or(0);
        if pluck {
            self.pluck_time = 0.004;
        }
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
        self.tuned = [0.0; 4];
    }

    /// The sounding frequency, as it glides and bends.
    pub fn set_frequency(&mut self, frequency: f32) {
        self.fundamental = frequency * self.intonation;
    }

    /// The playing controls; retunes the strings where needed.
    pub fn controls(&mut self, values: Controls) {
        let s = self.settings;
        let retune = values.position != s.position || values.brightness != s.brightness || values.coupling != s.coupling || values.sympathetic != s.sympathetic;
        self.settings = values;
        self.body.set_coupling(values.coupling);
        for (string, &open) in OPEN[self.instrument].iter().enumerate() {
            let frequency = if string == self.played { self.fundamental } else { pitch(open as f32) };
            if !retune && self.tuned[string] == frequency {
                continue;
            }
            self.tuned[string] = frequency;
            let other_load: f32 = (0..4).filter(|&o| o != string).map(|o| self.strings[o][0].impedance * values.sympathetic * values.sympathetic).sum();
            let delay = self.body.reflection_delay(frequency, self.strings[string][0].impedance, other_load as f64);
            for (p, lane) in self.strings[string].iter_mut().enumerate() {
                // The two polarisations a hair apart: the beating of a real string.
                let split = if p == 0 { 1.0 } else { 1.0008 };
                lane.tune(frequency * split, 0.055 + values.position * 0.22, values.brightness, 0.25 + self.instrument as f32 * 0.15, if self.pizzicato { 2.5 } else { 5.0 }, delay);
            }
        }
    }

    /// One sample. `envelope`: how much of the bow's speed and pressure
    /// reaches the string (0..1); `vibrato`: the played string's period
    /// factor (1: none); `gesture`: the bow's motion (1: steady; a sine
    /// for tremolo, its sign the stroke).
    pub fn tick(&mut self, envelope: f32, vibrato: f32, gesture: f32) -> f32 {
        let mut signal = 0.0;
        for _ in 0..self.os {
            signal += self.step(envelope, vibrato, gesture);
        }
        let signal = signal / self.os as f32;
        let out = signal - self.dc_in + self.dc_pole * self.dc_out;
        self.dc_in = signal;
        self.dc_out = out;
        out
    }

    /// One step of the model at its own rate: the bridge's signal.
    fn step(&mut self, envelope: f32, vibrato: f32, gesture: f32) -> f32 {
        const AXIS: [f32; 2] = [0.988686, 0.149999];
        let mut load = 0.0;
        let mut incident = [0.0f64; 2];
        for (s, string) in self.strings.iter_mut().enumerate() {
            let played = s == self.played;
            let transformer = if played { 1.0 } else { self.settings.sympathetic };
            let impedance = string[0].impedance;
            load += (transformer * transformer * impedance) as f64;
            for (p, lane) in string.iter_mut().enumerate() {
                lane.read(if played { vibrato } else { 1.0 });
                incident[p] += (2.0 * transformer * impedance * lane.to_bridge) as f64;
            }
        }
        let bridge = self.body.scatter(incident, load);
        let played = &self.strings[self.played];
        let impedance = played[0].impedance;
        let (mut free, mut compliance) = (0.0, 0.0);
        for (p, lane) in played.iter().enumerate() {
            free += AXIS[p] * lane.free_velocity();
            compliance += AXIS[p] * AXIS[p] / (2.0 * impedance);
        }
        let c = self.settings;
        let bow_velocity = c.direction * c.speed * 0.45 * gesture * envelope;
        let normal = impedance * c.force * 3.5 * c.engagement * envelope * (0.4 + 0.6 * self.velocity);
        let (mut force, _) = self.bow.solve(bow_velocity - free, compliance, if self.pizzicato { 0.0 } else { normal }, c.rosin, self.rate);
        if self.pizzicato && self.pluck_time > 0.0 {
            // A plucked string: a short half-sine of force.
            force = impedance * self.velocity * 0.7 * (std::f32::consts::PI * self.pluck_time / 0.004).sin();
            self.pluck_time = (self.pluck_time - (1.0 / self.rate) as f32).max(0.0);
        }
        for (s, string) in self.strings.iter_mut().enumerate() {
            let active = s == self.played;
            let transformer = if active { 1.0 } else { c.sympathetic };
            for (p, lane) in string.iter_mut().enumerate() {
                lane.bow(if active { force * AXIS[p] } else { 0.0 });
                lane.bridge(transformer * bridge[p]);
            }
        }
        let f = self.body.force;
        (f[0] * 0.16 + f[1] * 0.08 + bridge[0] * 11.0 + bridge[1] * 7.0) / IMPEDANCE[self.instrument][0].sqrt()
    }
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
        let mut i = Instrument::new(rate, 0);
        i.configure(instrument);
        i.note(pitch(key as f32), key, false, 0.8);
        i.controls(Controls::default());
        (0..(rate as f32 * seconds) as usize).map(|_| i.tick(1.0, 1.0, 1.0)).collect()
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
        let mut i = Instrument::new(48000.0, 0);
        i.note(440.0, 69, false, 0.8);
        i.controls(Controls::default());
        for _ in 0..24000 {
            i.tick(1.0, 1.0, 1.0);
        }
        let tail: Vec<f32> = (0..96000).map(|_| i.tick(0.0, 1.0, 1.0)).collect();
        let late = tail[90000..].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(late < 1e-3, "rings on: {late}");
    }

    #[test]
    fn a_plucked_note_decays() {
        let mut i = Instrument::new(48000.0, 0);
        i.configure(CELLO);
        i.note(pitch(48.0), 48, true, 0.9);
        i.controls(Controls::default());
        let x: Vec<f32> = (0..96000).map(|_| i.tick(0.0, 1.0, 1.0)).collect();
        let rms = |a: &[f32]| (a.iter().map(|v| v * v).sum::<f32>() / a.len() as f32).sqrt();
        assert!(rms(&x[2400..7200]) > 4.0 * rms(&x[72000..]), "decays");
    }
}

#[cfg(test)]
mod survey {
    use super::*;

    /// Cents sharp a note sounds (with the current intonation table).
    fn error(rate: f64, instrument: usize, key: i32) -> f32 {
        let mut i = Instrument::new(rate, 0);
        i.configure(instrument);
        i.note(pitch(key as f32), key, false, 0.8);
        i.controls(Controls::default());
        let n = (rate * 0.6) as usize;
        let x: Vec<f32> = (0..n).map(|_| i.tick(1.0, 1.0, 1.0)).collect();
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
                        let mut i = Instrument::new(rate, 0);
                        i.configure(instrument);
                        // Played with this correction, measured.
                        let correction = (-table[k] / 1200.0).exp2();
                        i.note(pitch(key as f32) * correction, key, false, 0.8);
                        i.intonation = 1.0;
                        i.fundamental = pitch(key as f32) * correction;
                        i.controls(Controls::default());
                        let n = (rate * 0.6) as usize;
                        let x: Vec<f32> = (0..n).map(|_| i.tick(1.0, 1.0, 1.0)).collect();
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

    /// Prints the cost of a player-second (run by hand, in release).
    #[test]
    #[ignore]
    fn cost_per_player() {
        let rate = 48000.0;
        for (label, controls, envelope) in [("bowed", Controls::default(), 1.0), ("bow lifted (no friction)", Controls::default(), 0.0), ("no sympathetic strings", Controls { sympathetic: 0.0, ..Controls::default() }, 1.0)] {
            let mut i = Instrument::new(rate, 0);
            i.note(440.0, 69, false, 0.8);
            i.controls(controls);
            let t = std::time::Instant::now();
            let mut acc = 0.0;
            for _ in 0..(rate as usize * 10) {
                acc += i.tick(envelope, 1.0, 1.0);
            }
            let per_second = t.elapsed().as_secs_f64() / 10.0;
            println!("{label}: {:.2} ms per second of sound ({:.0} ns a sample; {acc})", per_second * 1000.0, per_second * 1e9 / rate);
        }
    }
}
