//! One sounding note of a built-in instrument.
//!
//! Strings, woodwinds and brass are hybrids of three layers:
//!
//! - **spectral**: a bank of harmonics whose levels come from the
//!   instrument's spectral envelope ([`Spectrum`]): a slope that flattens
//!   as the player plays louder, resonance peaks at fixed frequencies (so
//!   the timbre changes across the range, and vibrato moves the harmonics
//!   through the peaks as on the real instrument), odd/even balance, and a
//!   noise part shaped by the same peaks (bow hair, breath);
//! - **physical**: a waveguide (a delay line tuned to the note with losses
//!   in the loop) driven by an exciter for bow, reed, lips or air jet. It
//!   gives the attack its build-up and the joins of slurred notes;
//! - **subtractive**: the exciter's raw waveform and noise, filtered by
//!   the dynamic.
//!
//! Plucked strings, keyboards and percussion use simpler models: a
//! Karplus-Strong string, decaying partials, struck modes and synthesized
//! drums.

use crate::dsp::{self, db, pulse, saw, Rng, Svf};
use crate::patches::{Model, Patch, MODES, PATCHES};
use std::f32::consts::TAU;

/// Playing techniques, as the technique controller (CC3) gives them.
pub mod tech {
    pub const ORD: u8 = 0;
    pub const PIZZ: u8 = 1;
    pub const TREM: u8 = 2;
    pub const CON_SORD: u8 = 3;
    pub const SUL_PONT: u8 = 4;
    pub const SUL_TASTO: u8 = 5;
    pub const COL_LEGNO: u8 = 6;
    pub const HARMONICS: u8 = 7;
    pub const FLUTTER: u8 = 8;
}

/// What drives a hybrid's waveguide.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Exciter {
    /// Stick-slip: a sawtooth with jitter and rosin noise.
    Bow,
    /// A reed beating against the mouthpiece: a pulse of this width.
    Reed(f32),
    /// Buzzing lips: a narrow pulse that sharpens with the dynamic.
    Lip,
    /// An air jet across an edge: a soft wave and breath.
    Jet,
}

/// An instrument's spectral envelope: harmonic n of a note at frequency f
/// sits at `slope(dyn) · log2(n) + Σ peaks(n·f) + even(n, n·f)` dB.
#[derive(Clone, Copy, Debug)]
pub struct Spectrum {
    /// dB per octave of harmonic number at ppp and at fff.
    pub tilt_pp: f32,
    pub tilt_ff: f32,
    /// Resonances: centre (Hz), width (octaves), level (dB).
    pub peaks: [(f32, f32, f32); 3],
    /// Even harmonics' change (dB) below a frequency (the clarinet's
    /// missing even harmonics in the chalumeau).
    pub even_db: f32,
    pub even_below: f32,
    /// How much later the high harmonics arrive in the attack (brass > 0).
    pub rise: f32,
    /// The noise part's level (dB against the harmonics) at mf.
    pub noise_db: f32,
    pub exciter: Exciter,
    /// The waveguide's share of the sustained sound (0..1); the attack
    /// always has more.
    pub physical: f32,
}

impl Spectrum {
    /// Harmonic `n`'s level (dB) for a note at `f` Hz at dynamic `d` (0..1).
    pub fn level(&self, n: usize, f: f32, d: f32, tilt_shift: f32) -> f32 {
        let hz = n as f32 * f;
        let tilt = self.tilt_pp + (self.tilt_ff - self.tilt_pp) * d.clamp(0.0, 1.0) + tilt_shift;
        let mut l = tilt * (n as f32).log2();
        for &(c, w, g) in &self.peaks {
            if g != 0.0 {
                let x = (hz / c).log2() / w.max(0.05);
                l += g * (-0.5 * x * x).exp();
            }
        }
        if n % 2 == 0 && hz < self.even_below {
            l += self.even_db;
        }
        l
    }
}

const MAX_HARMONICS: usize = 40;
const MAX_MODES: usize = 16;
const CONTROL: usize = 16; // samples between control updates

#[derive(Clone, Copy, Default)]
struct Mode {
    re: f32,
    im: f32,
    c: f32,
    s: f32,
    fast: f32,
    slow: f32,
    fast_mul: f32,
    slow_mul: f32,
}

impl Mode {
    fn set_freq(&mut self, hz: f32, sr: f32) {
        let w = TAU * hz / sr;
        self.c = w.cos();
        self.s = w.sin();
    }
    #[inline]
    fn tick(&mut self) -> f32 {
        let re = self.re * self.c - self.im * self.s;
        let im = self.re * self.s + self.im * self.c;
        self.re = re;
        self.im = im;
        let a = self.fast + self.slow;
        self.fast *= self.fast_mul;
        self.slow *= self.slow_mul;
        im * a
    }
}

#[derive(Clone, Copy, Default)]
struct KitSound {
    f0: f32,
    f1: f32,
    sweep: f32,
    tone: f32,
    tone_mul: f32,
    noise: f32,
    noise_mul: f32,
    metal: f32,
    metal_mul: f32,
    hp: f32,
    lp: f32,
}

pub struct Voice {
    pub active: bool,
    pub releasing: bool,
    pub note_id: i32,
    pub port: i16,
    pub channel: i16,
    pub key: i16,
    pub age: u64,
    /// Accepts the technique controller and CC1 while held.
    pub sustained: bool,
    pub patch: usize,
    model: Model,
    tech: u8,
    sr: f32,
    /// Target and current frequency (Hz, before bend and vibrato).
    freq: f32,
    cur: f32,
    glide: f32,
    t: f32,
    env: f32,
    env_rate: f32,
    rel_mul: f32,
    /// Dynamic (0..1), its target (CC1), the attack velocity.
    pub dynamic: f32,
    pub dyn_target: f32,
    vel: f32,
    accent: f32,
    /// A legato join's renewed excitation, decaying.
    join: f32,
    scoop: f32,
    vib_phase: f32,
    vib_rate: f32,
    rng: Rng,
    lp: Svf,
    lp2: Svf,
    form: [Svf; 3],
    nf: [Svf; 3],
    hp: Svf,
    // spectral layer
    h_re: [f32; MAX_HARMONICS],
    h_im: [f32; MAX_HARMONICS],
    h_c: [f32; MAX_HARMONICS],
    h_s: [f32; MAX_HARMONICS],
    h_amp: [f32; MAX_HARMONICS],
    h_target: [f32; MAX_HARMONICS],
    h_drift: [f32; MAX_HARMONICS],
    n_h: usize,
    // oscillators of the subtractive parts and exciters
    phases: [f32; 6],
    // waveguide / Karplus-Strong
    line: Box<[f32]>,
    pos: usize,
    loop_lp: f32,
    loop_fb: f32,
    loop_bright: f32,
    // struck modes
    modes: [Mode; MAX_MODES],
    n_modes: usize,
    kit: KitSound,
    ctl: usize,
}

fn key_hz(key: f32) -> f32 {
    440.0 * 2f32.powf((key - 69.0) / 12.0)
}

impl Voice {
    pub fn new(sample_rate: f64) -> Voice {
        let len = ((sample_rate / 15.0) as usize).next_power_of_two();
        Voice {
            active: false,
            releasing: false,
            note_id: -1,
            port: 0,
            channel: 0,
            key: 0,
            age: 0,
            sustained: false,
            patch: 0,
            model: Model::Pad,
            tech: 0,
            sr: sample_rate as f32,
            freq: 440.0,
            cur: 440.0,
            glide: 1.0,
            t: 0.0,
            env: 0.0,
            env_rate: 0.0,
            rel_mul: 1.0,
            dynamic: 0.5,
            dyn_target: 0.5,
            vel: 0.5,
            accent: 0.0,
            join: 0.0,
            scoop: 0.0,
            vib_phase: 0.0,
            vib_rate: 5.5,
            rng: Rng::default(),
            lp: Svf::default(),
            lp2: Svf::default(),
            form: [Svf::default(); 3],
            nf: [Svf::default(); 3],
            hp: Svf::default(),
            h_re: [0.0; MAX_HARMONICS],
            h_im: [0.0; MAX_HARMONICS],
            h_c: [1.0; MAX_HARMONICS],
            h_s: [0.0; MAX_HARMONICS],
            h_amp: [0.0; MAX_HARMONICS],
            h_target: [0.0; MAX_HARMONICS],
            h_drift: [0.0; MAX_HARMONICS],
            n_h: 0,
            phases: [0.0; 6],
            line: vec![0.0; len].into_boxed_slice(),
            pos: 0,
            loop_lp: 0.0,
            loop_fb: 0.0,
            loop_bright: 0.5,
            modes: [Mode::default(); MAX_MODES],
            n_modes: 0,
            kit: KitSound::default(),
            ctl: 0,
        }
    }

    fn p(&self) -> &'static Patch {
        &PATCHES[self.patch]
    }

    /// Starts a note. `dynamic`: the channel's CC1, if it has one.
    #[allow(clippy::too_many_arguments)]
    pub fn start(&mut self, patch: usize, technique: u8, note_id: i32, port: i16, channel: i16, key: i16, velocity: f32, dynamic: Option<f32>, age: u64, seed: u32) {
        let p = &PATCHES[patch];
        let sr = self.sr;
        self.active = true;
        self.releasing = false;
        self.note_id = note_id;
        self.port = port;
        self.channel = channel;
        self.key = key;
        self.age = age;
        self.patch = patch;
        self.tech = technique;
        self.rng = Rng(0x9E37_79B9 ^ seed.wrapping_mul(2_654_435_761) ^ (key as u32) << 16 | 1);
        self.vel = velocity.clamp(0.0, 1.0);
        self.model = match (p.model, technique) {
            (Model::Bowed, tech::PIZZ) => Model::Pluck(0.4),
            (Model::Bowed, tech::COL_LEGNO) => Model::Pluck(0.9),
            (m, _) => m,
        };
        let percussive = matches!(self.model, Model::Pluck(_) | Model::Piano | Model::Mallet(_) | Model::Timpani | Model::Kit);
        self.sustained = !percussive;
        self.dynamic = if percussive { self.vel } else { dynamic.unwrap_or(self.vel) };
        self.dyn_target = self.dynamic;
        self.accent = if percussive { 0.0 } else { (self.vel - self.dynamic).max(0.0) * 2.5 + if self.vel > 0.8 { (self.vel - 0.8) * 1.5 } else { 0.0 } };
        self.freq = key_hz(key as f32);
        self.cur = self.freq;
        self.glide = dsp::smoothing(0.03, sr);
        self.t = 0.0;
        self.join = 0.0;
        self.env = 0.0;
        let attack = (p.attack * (1.5 - self.vel)).max(0.002) * if technique == tech::HARMONICS { 1.5 } else { 1.0 };
        self.env_rate = dsp::smoothing(attack / 3.0, sr);
        self.rel_mul = 1.0;
        self.scoop = if self.model == Model::Brass { -0.35 - 0.4 * (1.0 - self.vel) } else if matches!(self.model, Model::Bowed) { -0.06 } else { 0.0 };
        self.vib_phase = self.rng.unit();
        self.vib_rate = p.vib_rate * (0.93 + 0.14 * self.rng.unit());
        self.lp.reset();
        self.lp2.reset();
        self.hp.reset();
        for f in self.form.iter_mut().chain(self.nf.iter_mut()) {
            f.reset();
        }
        for (i, ph) in self.phases.iter_mut().enumerate() {
            *ph = if i == 0 { 0.0 } else { self.rng.unit() };
        }
        self.ctl = 0;
        // The spectral layer starts silent and rises.
        for h in 0..MAX_HARMONICS {
            // Schroeder's phases: a low crest factor, and the same every
            // time (the layers' sum would vary from note to note otherwise).
            let a = std::f32::consts::PI * (h * (h + 1)) as f32 / MAX_HARMONICS as f32;
            self.h_re[h] = a.cos();
            self.h_im[h] = a.sin();
            self.h_amp[h] = 0.0;
            self.h_target[h] = 0.0;
            self.h_drift[h] = self.rng.noise();
        }
        self.n_h = 0;
        self.line.fill(0.0);
        self.pos = 0;
        self.loop_lp = 0.0;
        match self.model {
            Model::Pluck(b) => self.start_pluck(b),
            Model::Piano => self.start_piano(),
            Model::Mallet(i) => self.start_modes(MODES[i], p.ring),
            Model::Timpani => self.start_modes(&[(1.0, 1.0, 1.0), (1.504, 0.8, 0.8), (1.742, 0.6, 0.7), (2.0, 0.5, 0.6), (2.245, 0.3, 0.5)], p.ring),
            Model::Kit => self.start_kit(),
            _ => {
                if let Some(s) = crate::patches::spectrum(patch) {
                    self.tune_waveguide(s);
                }
            }
        }
    }

    /// Moves a held note to a new one without a new attack (a slur).
    pub fn legato_to(&mut self, note_id: i32, key: i16, velocity: f32, age: u64) {
        self.note_id = note_id;
        self.key = key;
        self.age = age;
        self.freq = key_hz(key as f32);
        // Fingers are quick, bows and lips a little slower.
        let time = match self.model {
            Model::Bowed => 0.045,
            Model::Brass => 0.035,
            Model::Choir => 0.06,
            _ => 0.022,
        };
        let leap = ((self.freq / self.cur).log2().abs() * 12.0).min(12.0);
        self.glide = dsp::smoothing(time * (0.5 + leap / 12.0), self.sr);
        self.join = 0.35 + 0.3 * velocity;
        if let Some(s) = crate::patches::spectrum(self.patch) {
            self.tune_waveguide(s);
        }
    }

    pub fn release(&mut self) {
        if self.releasing {
            return;
        }
        let p = self.p();
        let time = match self.model {
            // Struck sounds ring on (the vibraphone's pedal is down).
            Model::Mallet(i) if i != 4 => return,
            Model::Kit | Model::Timpani => return,
            // A string's damper, or the top of the piano without dampers.
            Model::Piano if self.key >= 89 => return,
            Model::Pluck(_) if PATCHES[self.patch].model == Model::Bowed => 0.6,
            _ => p.release,
        };
        self.releasing = true;
        self.rel_mul = dsp::decay(time, self.sr);
    }

    pub fn set_technique(&mut self, t: u8) {
        if self.sustained {
            self.tech = t;
        }
    }

    fn tune_waveguide(&mut self, s: &Spectrum) {
        // Losses: a bright, high-Q tube for reeds and brass, a softer string.
        self.loop_fb = match s.exciter {
            Exciter::Bow => 0.985,
            Exciter::Reed(_) => 0.97,
            Exciter::Lip => 0.975,
            Exciter::Jet => 0.95,
        };
        self.loop_bright = match s.exciter {
            Exciter::Bow => 0.55,
            Exciter::Reed(_) => 0.45,
            Exciter::Lip => 0.5,
            Exciter::Jet => 0.35,
        };
    }

    fn start_pluck(&mut self, brightness: f32) {
        let period = (self.sr / self.freq).min(self.line.len() as f32 - 2.0);
        let n = period as usize;
        // A burst of noise, darker when softer.
        let a = (0.15 + 0.85 * brightness * (0.5 + 0.5 * self.vel)).clamp(0.05, 1.0);
        let mut y = 0.0;
        let mut mean = 0.0;
        let len = self.line.len();
        let n = n.max(1);
        // Where the first period is read from: just behind the write head.
        for i in 0..n {
            y += a * (self.rng.noise() - y);
            self.line[len - n + i] = y;
            mean += y;
        }
        let mean = mean / n as f32;
        for v in self.line[len - n..].iter_mut() {
            *v -= mean;
        }
        let ring = self.p().ring * (261.6 / self.freq).powf(0.5) * if self.tech == tech::COL_LEGNO { 0.15 } else { 1.0 };
        self.loop_fb = 10f32.powf(-3.0 * (period / self.sr) / ring.max(0.05));
        self.loop_bright = 0.5 + 0.48 * brightness;
        self.pos = 0;
        self.env = 1.0;
    }

    fn start_piano(&mut self) {
        let f = self.freq;
        let ring = (self.p().ring * (261.6 / f).powf(0.7)).clamp(0.4, 25.0);
        let b = 0.0004 * (f / 261.6).powf(0.5);
        let mut k = 0;
        let mut n = 1;
        while k < MAX_MODES && n <= 12 {
            let hz = f * n as f32 * (1.0 + b * (n * n) as f32).sqrt();
            if hz > self.sr * 0.45 || hz > 12000.0 {
                break;
            }
            let amp = (n as f32).powf(-(1.9 - 1.1 * self.vel)) * 0.5;
            let t60 = ring / (1.0 + 0.4 * (n - 1) as f32);
            for (twin, detune) in [(false, 1.0), (true, 1.0004)] {
                if twin && (n > 3 || k >= MAX_MODES) {
                    continue;
                }
                let m = &mut self.modes[k];
                m.set_freq(hz * detune, self.sr);
                m.re = 1.0;
                m.im = 0.0;
                let a = if n <= 3 { amp * 0.5 } else { amp };
                m.fast = a * 0.65;
                m.slow = a * 0.35;
                m.fast_mul = dsp::decay(t60 * 0.25, self.sr);
                m.slow_mul = dsp::decay(t60, self.sr);
                k += 1;
            }
            n += 1;
        }
        self.n_modes = k;
        self.env = 1.0;
    }

    fn start_modes(&mut self, modes: &[(f32, f32, f32)], ring: f32) {
        let f = self.freq;
        let t60 = ring * (261.6 / f).powf(0.6).clamp(0.2, 3.0);
        let mut k = 0;
        for &(ratio, level, dec) in modes.iter().take(MAX_MODES) {
            let hz = f * ratio;
            if hz > self.sr * 0.45 {
                continue;
            }
            let m = &mut self.modes[k];
            m.set_freq(hz, self.sr);
            m.re = 1.0;
            m.im = 0.0;
            // Harder strokes bring out the upper modes.
            let a = level * (0.35 + 0.65 * self.vel).powf(ratio.log2().max(0.0));
            m.fast = a * 0.3;
            m.slow = a * 0.7;
            m.fast_mul = dsp::decay(t60 * dec * 0.15, self.sr);
            m.slow_mul = dsp::decay(t60 * dec, self.sr);
            k += 1;
        }
        self.n_modes = k;
        self.env = 1.0;
    }

    fn start_kit(&mut self) {
        let sr = self.sr;
        let d = |s: f32| dsp::decay(s, sr);
        let key = self.key;
        let mut k = KitSound { lp: 12000.0, hp: 40.0, ..Default::default() };
        match key {
            35 | 36 => {
                k.f0 = 150.0;
                k.f1 = 48.0;
                k.sweep = d(0.04);
                k.tone = 1.0;
                k.tone_mul = d(0.45);
                k.noise = 0.15;
                k.noise_mul = d(0.01);
                k.lp = 3000.0;
            }
            37 => {
                k.f0 = 520.0;
                k.f1 = 480.0;
                k.sweep = d(0.01);
                k.tone = 0.4;
                k.tone_mul = d(0.05);
                k.noise = 0.5;
                k.noise_mul = d(0.04);
                k.hp = 1200.0;
            }
            38 | 40 => {
                k.f0 = 240.0;
                k.f1 = 185.0;
                k.sweep = d(0.02);
                k.tone = 0.5;
                k.tone_mul = d(0.12);
                k.noise = 0.8;
                k.noise_mul = d(0.22);
                k.hp = 1000.0;
            }
            39 => {
                k.noise = 1.0;
                k.noise_mul = d(0.15);
                k.hp = 900.0;
                k.lp = 4000.0;
            }
            41 | 43 | 45 | 47 | 48 | 50 => {
                let f = 70.0 * 2f32.powf((key - 41) as f32 / 12.0 * 1.4);
                k.f0 = f * 1.4;
                k.f1 = f;
                k.sweep = d(0.06);
                k.tone = 0.9;
                k.tone_mul = d(0.5);
                k.noise = 0.12;
                k.noise_mul = d(0.03);
                k.lp = 2500.0;
            }
            42 | 44 => {
                k.metal = 0.6;
                k.metal_mul = d(if key == 42 { 0.07 } else { 0.1 });
                k.noise = 0.5;
                k.noise_mul = d(0.06);
                k.hp = 7000.0;
            }
            46 => {
                k.metal = 0.6;
                k.metal_mul = d(0.6);
                k.noise = 0.4;
                k.noise_mul = d(0.5);
                k.hp = 6500.0;
            }
            49 | 52 | 55 | 57 => {
                k.metal = 0.7;
                k.metal_mul = d(if key == 55 { 0.8 } else { 2.2 });
                k.noise = 0.6;
                k.noise_mul = d(1.6);
                k.hp = 3500.0;
            }
            51 | 53 | 59 => {
                k.metal = 0.8;
                k.metal_mul = d(1.8);
                k.noise = 0.15;
                k.noise_mul = d(0.6);
                k.hp = 2500.0;
                k.f0 = 3200.0;
                k.f1 = 3200.0;
                k.sweep = 1.0;
                k.tone = 0.08;
                k.tone_mul = d(1.2);
            }
            54 => {
                k.noise = 0.8;
                k.noise_mul = d(0.25);
                k.metal = 0.4;
                k.metal_mul = d(0.25);
                k.hp = 5500.0;
            }
            56 => {
                k.f0 = 587.0;
                k.f1 = 587.0;
                k.sweep = 1.0;
                k.tone = 0.8;
                k.tone_mul = d(0.3);
                k.metal = 0.3;
                k.metal_mul = d(0.3);
                k.hp = 400.0;
            }
            80 | 81 => {
                // Triangle: high inharmonic partials, long.
                k.f0 = 4100.0;
                k.f1 = 4100.0;
                k.sweep = 1.0;
                k.tone = 0.35;
                k.tone_mul = d(if key == 80 { 0.15 } else { 3.0 });
                k.metal = 0.25;
                k.metal_mul = d(if key == 80 { 0.15 } else { 2.0 });
                k.hp = 3000.0;
            }
            _ => {
                k.noise = 0.6;
                k.noise_mul = d(0.08);
                k.f0 = 400.0;
                k.f1 = 300.0;
                k.sweep = d(0.03);
                k.tone = 0.3;
                k.tone_mul = d(0.1);
                k.hp = 300.0;
            }
        }
        let v = 0.25 + 0.75 * self.vel;
        k.tone *= v;
        k.noise *= v;
        k.metal *= v;
        self.hp.set(k.hp, 0.7, sr);
        self.lp.set(k.lp, 0.7, sr);
        self.kit = k;
        self.env = 1.0;
    }

    /// The level the patch's calibration and the dynamic give.
    fn gain(&self, trims: &[f32]) -> f32 {
        let p = self.p();
        trims[self.patch] * db(p.range_db * (self.dynamic - 0.62) / 0.84)
    }

    /// Adds this voice into `out` (mono). `bend`: the channel's pitch bend
    /// in semitones. Returns false once the voice has ended.
    pub fn render(&mut self, out: &mut [f32], bend: f32, trims: &[f32]) -> bool {
        if !self.active {
            return false;
        }
        match self.model {
            Model::Pluck(_) => self.render_pluck(out, bend, trims),
            Model::Piano | Model::Mallet(_) | Model::Timpani => self.render_modes(out, trims),
            Model::Kit => self.render_kit(out, trims),
            Model::Bowed | Model::Reed(_) | Model::Flute | Model::Brass if crate::patches::spectrum(self.patch).is_some() => self.render_hybrid(out, bend, trims),
            _ => self.render_simple(out, bend, trims),
        }
        if self.releasing && self.env < 1e-4 {
            self.active = false;
        }
        self.active
    }

    fn vibrato(&mut self) -> f32 {
        let p = self.p();
        if p.vibrato <= 0.0 || self.tech == tech::HARMONICS {
            return 0.0;
        }
        // Starts after the attack, wider when louder and on long notes.
        let fade = ((self.t - 0.15) / 0.4).clamp(0.0, 1.0);
        let depth = p.vibrato * fade * (0.6 + 0.7 * self.dynamic);
        self.vib_phase = (self.vib_phase + self.vib_rate * CONTROL as f32 / self.sr).fract();
        depth * (self.vib_phase * TAU).sin()
    }

    fn envelope_step(&mut self) {
        if self.releasing {
            self.env *= self.rel_mul;
        } else {
            self.env += (1.02 - self.env) * self.env_rate;
            self.env = self.env.min(1.0);
        }
    }

    // ------------------------------------------------ strings, winds, brass --
    fn render_hybrid(&mut self, out: &mut [f32], bend: f32, trims: &[f32]) {
        let s = *crate::patches::spectrum(self.patch).unwrap();
        let p = *self.p();
        let sr = self.sr;
        let dt = 1.0 / sr;
        let dyn_rate = dsp::smoothing(0.04, sr);
        let mut ratio = 1.0;
        let mut noise_gain = 0.0;
        let mut phys_gain = 0.0;
        let mut gain = 0.0;
        let mut tech_amp = 1.0;
        let mut exc_shape = 0.5;
        let mut period = 100.0f32;
        let len = self.line.len();
        for o in out.iter_mut() {
            if self.ctl == 0 {
                self.ctl = CONTROL;
                self.dynamic += (self.dyn_target - self.dynamic) * (1.0 - (1.0 - dyn_rate).powi(CONTROL as i32));
                self.cur += (self.freq - self.cur) * (1.0 - (1.0 - self.glide).powi(CONTROL as i32));
                self.scoop *= dsp::decay(0.08, sr / CONTROL as f32);
                let vib = self.vibrato();
                ratio = 2f32.powf((bend + vib + self.scoop) / 12.0);
                let f = self.cur * ratio;
                period = (sr / f).clamp(2.0, len as f32 - 2.0);
                let d = self.dynamic;
                // Technique changes to the spectrum.
                let (tilt_shift, peak_cut) = match self.tech {
                    tech::SUL_PONT => (5.0, 0.0),
                    tech::SUL_TASTO => (-5.0, 0.0),
                    tech::CON_SORD if p.model == Model::Brass => (-1.0, 9.0),
                    tech::CON_SORD => (-3.0, 6.0),
                    _ => (0.0, 0.0),
                };
                tech_amp = match self.tech {
                    tech::TREM => 0.45 + 0.55 * (self.t * 7.5 * std::f32::consts::PI).sin().abs(),
                    tech::FLUTTER => 0.65 + 0.35 * (self.t * 24.0 * TAU).sin(),
                    tech::CON_SORD => 0.6,
                    tech::SUL_PONT | tech::SUL_TASTO => 0.8,
                    tech::HARMONICS => 0.5,
                    _ => 1.0,
                };
                // Harmonic levels, normalised to the fundamental's region.
                let limit = (sr * 0.45).min(14000.0);
                let n_h = ((limit / f) as usize).clamp(1, MAX_HARMONICS);
                self.n_h = n_h;
                let spread = p.spread * if p.unison > 1 { 1.0 } else { 0.15 };
                for h in 0..n_h {
                    let n = h + 1;
                    let mut l = s.level(n, f, d, tilt_shift);
                    if peak_cut > 0.0 {
                        // A mute: cuts the lows, a narrow nasal peak.
                        let hz = n as f32 * f;
                        l -= peak_cut * (1.0 - (hz / 1800.0).min(1.0));
                    }
                    if self.tech == tech::HARMONICS {
                        l = if n == 1 { 0.0 } else { -40.0 - 6.0 * n as f32 };
                    }
                    self.h_target[h] = db(l);
                    // A section: each harmonic drifts on its own (the players
                    // never quite agree), with slow beating amplitudes.
                    self.h_drift[h] = (self.h_drift[h] + 0.05 * self.rng.noise()).clamp(-1.0, 1.0);
                    let cents = spread * self.h_drift[h];
                    let hz = n as f32 * f * (1.0 + cents * 0.000_577_6);
                    let w = TAU * hz / sr;
                    self.h_c[h] = w.cos();
                    self.h_s[h] = w.sin();
                    // Keep the phasors on the unit circle.
                    let m = (self.h_re[h] * self.h_re[h] + self.h_im[h] * self.h_im[h]).sqrt();
                    if m > 0.0 {
                        self.h_re[h] /= m;
                        self.h_im[h] /= m;
                    }
                }
                for h in n_h..MAX_HARMONICS {
                    self.h_target[h] = 0.0;
                }
                // Constant power: the dynamic's loudness is the calibrated
                // range; the spectrum only changes the colour.
                let power: f32 = self.h_target[..n_h].iter().map(|a| a * a).sum();
                let norm = if power > 0.0 { 1.0 / power.sqrt() } else { 0.0 };
                for h in 0..n_h {
                    // High harmonics arrive later in the attack.
                    let rise_t = p.attack * (1.0 + s.rise * ((h + 1) as f32).log2());
                    let rise = (self.t / rise_t.max(1e-3)).min(1.0);
                    self.h_target[h] *= norm * rise * rise;
                }
                // The noise part through the same resonances.
                for (i, &(c, w, g)) in s.peaks.iter().enumerate() {
                    let q = (1.0 / w.max(0.1)).clamp(0.5, 8.0);
                    self.nf[i].set(c.max(80.0), q, sr);
                    let _ = g;
                }
                let attack_noise = (1.0 - self.t / (p.attack * 2.0 + 0.02)).max(0.0);
                noise_gain = db(s.noise_db + 6.0 * (0.6 - d)) * (1.0 + 3.0 * attack_noise + 2.0 * self.join)
                    * match self.tech {
                        tech::SUL_PONT => 2.5,
                        tech::FLUTTER => 3.0,
                        tech::HARMONICS => 1.5,
                        _ => 1.0,
                    };
                // The waveguide: strongest in the attack and the joins.
                let attack_phys = (1.0 - self.t / (p.attack * 3.0 + 0.03)).max(0.0);
                phys_gain = (s.physical + 0.8 * attack_phys + 0.6 * self.join).min(1.2);
                self.join *= dsp::decay(0.12, sr / CONTROL as f32);
                exc_shape = 0.3 + 0.6 * d;
                // Subtractive brightness of the exciter, by the dynamic.
                let cutoff = f * (p.dark + (p.bright - p.dark) * d.powf(1.4)) * if self.tech == tech::SUL_PONT { 2.0 } else if self.tech == tech::SUL_TASTO { 0.5 } else { 1.0 };
                self.lp.set(cutoff, 0.7, sr);
                gain = self.gain(trims) * (1.0 + self.accent * (-self.t / 0.09).exp());
            }
            self.ctl -= 1;
            self.envelope_step();
            self.t += dt;
            // Spectral layer.
            let mut harm = 0.0;
            for h in 0..self.n_h {
                let re = self.h_re[h] * self.h_c[h] - self.h_im[h] * self.h_s[h];
                let im = self.h_re[h] * self.h_s[h] + self.h_im[h] * self.h_c[h];
                self.h_re[h] = re;
                self.h_im[h] = im;
                self.h_amp[h] += (self.h_target[h] - self.h_amp[h]) * 0.004;
                harm += im * self.h_amp[h];
            }
            // Noise part.
            let n = self.rng.noise();
            let mut noise = 0.0;
            for (i, &(_, _, g)) in s.peaks.iter().enumerate() {
                noise += self.nf[i].tick(n).band * db(g * 0.5);
            }
            noise *= noise_gain * 0.25;
            // Physical layer: exciter into a tuned waveguide.
            let f = self.cur * ratio;
            let step = f / sr;
            self.phases[0] = (self.phases[0] + step).fract();
            let ph = self.phases[0];
            let exc = match s.exciter {
                Exciter::Bow => saw(ph, step) * (1.0 + 0.08 * self.rng.noise()) + 0.15 * n,
                Exciter::Reed(w) if w <= 0.0 => saw(ph, step),
                Exciter::Reed(w) => pulse(ph, step, w),
                Exciter::Lip => pulse(ph, step, 0.5 - 0.25 * exc_shape),
                Exciter::Jet => dsp::sine(ph) + 0.3 * n,
            };
            let exc = self.lp.tick(exc).low;
            let rd = self.pos as f32 + len as f32 - period;
            let i0 = rd as usize;
            let fr = rd - i0 as f32;
            let a = self.line[i0 % len];
            let b = self.line[(i0 + 1) % len];
            let back = a + (b - a) * fr;
            self.loop_lp += (back - self.loop_lp) * self.loop_bright;
            let y = exc * 0.12 + self.loop_lp * self.loop_fb;
            self.line[self.pos] = y.clamp(-4.0, 4.0);
            self.pos = (self.pos + 1) % len;
            let phys = self.loop_lp * 0.25;
            let sum = harm * 0.5 + phys * phys_gain + noise;
            *o += sum * gain * self.env * tech_amp;
        }
    }

    // ----------------------------------- voices, organ, pads, other sustains --
    fn render_simple(&mut self, out: &mut [f32], bend: f32, trims: &[f32]) {
        let p = *self.p();
        let sr = self.sr;
        let dt = 1.0 / sr;
        let dyn_rate = dsp::smoothing(0.04, sr);
        let mut ratio = 1.0;
        let mut gain = 0.0;
        for o in out.iter_mut() {
            if self.ctl == 0 {
                self.ctl = CONTROL;
                self.dynamic += (self.dyn_target - self.dynamic) * (1.0 - (1.0 - dyn_rate).powi(CONTROL as i32));
                self.cur += (self.freq - self.cur) * (1.0 - (1.0 - self.glide).powi(CONTROL as i32));
                let vib = self.vibrato();
                ratio = 2f32.powf((bend + vib) / 12.0);
                let f = self.cur * ratio;
                let d = self.dynamic;
                self.lp.set(f * (p.dark + (p.bright - p.dark) * d), 0.7, sr);
                for (i, &(c, q, _)) in p.formants.iter().enumerate() {
                    if c > 0.0 {
                        self.form[i].set(c, q, sr);
                    }
                }
                gain = self.gain(trims) * (1.0 + self.accent * (-self.t / 0.09).exp());
            }
            self.ctl -= 1;
            self.envelope_step();
            self.t += dt;
            let f = self.cur * ratio;
            let step = f / sr;
            let mut x = 0.0;
            match self.model {
                Model::Organ => {
                    for (i, (mult, level)) in [(1.0, 1.0), (2.0, 0.6), (3.0, 0.3), (4.0, 0.35), (5.0, 0.12), (6.0, 0.1), (8.0, 0.08)].iter().enumerate() {
                        if f * mult < sr * 0.45 {
                            self.phases[i.min(5)] = if i < 6 { (self.phases[i] + step * mult).fract() } else { self.phases[5] };
                            x += level * dsp::sine(if i < 6 { self.phases[i] } else { (self.phases[0] * 8.0).fract() });
                        }
                    }
                    x *= 0.4;
                    x += 0.3 * self.rng.noise() * (-self.t / 0.01).exp();
                }
                Model::Choir => {
                    let n = p.unison.max(1) as usize;
                    for u in 0..n.min(6) {
                        let det = if n > 1 { p.spread * (u as f32 / (n - 1) as f32 - 0.5) * 2.0 } else { 0.0 };
                        let st = step * (1.0 + det * 0.000_577_6);
                        self.phases[u] = (self.phases[u] + st).fract();
                        x += saw(self.phases[u], st);
                    }
                    x /= (n as f32).sqrt();
                    x += 0.05 * self.rng.noise();
                    let src = self.lp.tick(x).low;
                    let mut y = 0.0;
                    for (i, &(c, _, g)) in p.formants.iter().enumerate() {
                        if c > 0.0 {
                            y += self.form[i].tick(src).band * g;
                        }
                    }
                    x = y * 2.0;
                }
                Model::Reed(w) => {
                    self.phases[0] = (self.phases[0] + step).fract();
                    let src = if w <= 0.0 { saw(self.phases[0], step) } else { pulse(self.phases[0], step, w) };
                    let y = self.lp.tick(src).low;
                    let mut z = 0.6 * y;
                    for (i, &(c, _, g)) in p.formants.iter().enumerate() {
                        if c > 0.0 {
                            z += self.form[i].tick(y).band * g;
                        }
                    }
                    x = z;
                }
                _ if p.bright <= 1.0 => {
                    self.phases[0] = (self.phases[0] + step).fract();
                    x = dsp::sine(self.phases[0]);
                }
                _ => {
                    for u in 0..2 {
                        let st = step * if u == 0 { 0.997 } else { 1.003 };
                        self.phases[u] = (self.phases[u] + st).fract();
                        x += saw(self.phases[u], st) * 0.7;
                    }
                    x = self.lp.tick(x).low;
                }
            }
            *o += x * gain * self.env;
        }
    }

    // ------------------------------------------------------- plucked strings --
    fn render_pluck(&mut self, out: &mut [f32], bend: f32, trims: &[f32]) {
        let len = self.line.len();
        let gain = self.gain(trims);
        let period = (self.sr / (self.freq * 2f32.powf(bend / 12.0)) - 0.5).clamp(2.0, len as f32 - 2.0);
        for o in out.iter_mut() {
            let rd = self.pos as f32 + len as f32 - period;
            let i0 = rd as usize;
            let fr = rd - i0 as f32;
            let a = self.line[i0 % len];
            let b = self.line[(i0 + 1) % len];
            let y = a + (b - a) * fr;
            let filtered = self.loop_bright * y + (1.0 - self.loop_bright) * self.loop_lp;
            self.loop_lp = y;
            self.line[self.pos] = filtered * self.loop_fb;
            self.pos = (self.pos + 1) % len;
            if self.releasing {
                self.env *= self.rel_mul;
            }
            // Ends when it has rung out.
            self.t += 1.0 / self.sr;
            let p = self.p();
            if self.t > p.ring * 4.0 + 1.0 {
                self.env *= 0.999;
                if self.env < 1e-4 {
                    self.releasing = true;
                }
            }
            *o += y * gain * self.env * 1.6;
        }
    }

    // --------------------------------------------------- partials and modes --
    fn render_modes(&mut self, out: &mut [f32], trims: &[f32]) {
        let gain = self.gain(trims);
        let vibes = self.model == Model::Mallet(2);
        let timpani = self.model == Model::Timpani;
        let piano = self.model == Model::Piano;
        if !piano {
            self.lp.set(150.0, 0.7, self.sr);
        }
        let mut level = 0.0f32;
        for o in out.iter_mut() {
            let mut x = 0.0;
            for m in &mut self.modes[..self.n_modes] {
                x += m.tick();
            }
            // The strike: a click, a thump under a timpani.
            let strike = (-self.t / 0.004).exp();
            if piano {
                x += 0.08 * self.vel * strike * self.rng.noise();
            } else if timpani {
                x += 0.3 * self.lp.tick(self.rng.noise()).low * (-self.t / 0.06).exp();
            } else {
                x += 0.1 * self.vel * strike * self.rng.noise();
            }
            if vibes {
                x *= 0.8 + 0.2 * (self.t * 5.5 * TAU).sin();
            }
            if self.releasing {
                self.env *= self.rel_mul;
            }
            self.t += 1.0 / self.sr;
            level = level.max(x.abs());
            *o += x * gain * self.env;
        }
        // Rung out.
        if self.modes[..self.n_modes].iter().all(|m| (m.fast + m.slow).abs() < 1e-5) && self.t > 0.02 {
            self.releasing = true;
            self.env = 0.0;
        }
        let _ = level;
    }

    // ------------------------------------------------------------- drums --
    fn render_kit(&mut self, out: &mut [f32], trims: &[f32]) {
        let gain = self.gain(trims);
        let sr = self.sr;
        let k = &mut self.kit;
        const METAL: [f32; 6] = [205.3, 304.4, 369.6, 522.7, 540.0, 800.0];
        for o in out.iter_mut() {
            k.f0 = k.f1 + (k.f0 - k.f1) * k.sweep;
            self.phases[0] = (self.phases[0] + k.f0 / sr).fract();
            let tone = dsp::sine(self.phases[0]) * k.tone;
            k.tone *= k.tone_mul;
            let mut metal = 0.0;
            if k.metal > 1e-6 {
                for (i, f) in METAL.iter().enumerate().take(5) {
                    self.phases[i + 1] = (self.phases[i + 1] + f * 7.3 / sr).fract();
                    metal += if self.phases[i + 1] < 0.5 { 1.0 } else { -1.0 };
                }
                metal += if (self.phases[1] * 1.37).fract() < 0.5 { 1.0 } else { -1.0 };
                metal *= k.metal / 6.0;
                k.metal *= k.metal_mul;
            }
            let noise = self.rng.noise() * k.noise;
            k.noise *= k.noise_mul;
            let bright = self.hp.tick(noise + metal).high;
            let x = self.lp.tick(tone + bright).low;
            self.t += 1.0 / sr;
            *o += x * gain * self.env;
        }
        if self.t > 0.02 && k.tone < 1e-5 && k.noise < 1e-5 && k.metal < 1e-5 {
            self.releasing = true;
            self.env = 0.0;
        }
    }
}

/// Renders a patch's mf note at its reference key and measures its
/// loudness: RMS, with the bass weighed less (a first-order high-pass at
/// 200 Hz, as the ear does), over the sustain of a held sound or the first
/// 0.1 s of a struck one (the ear's short-term loudness).
pub fn measure(patch: usize, sample_rate: f64) -> f32 {
    let p = &PATCHES[patch];
    let mut v = Voice::new(sample_rate);
    let key = if p.model == Model::Kit { 38 } else { p.ref_key as i16 };
    v.start(patch, tech::ORD, 0, 0, 0, key, 0.62, Some(0.62), 0, 1);
    let sr = sample_rate as f32;
    let (from, to) = if v.sustained { (0.3, 0.8) } else { (0.0, 0.1) };
    let n = (to * sr) as usize;
    let mut buf = vec![0.0f32; n];
    let ones = [1.0f32; crate::patches::COUNT];
    for chunk in buf.chunks_mut(256) {
        v.render(chunk, 0.0, &ones);
    }
    let a = (-TAU * 200.0 / sr).exp();
    let (mut prev_x, mut prev_y) = (0.0, 0.0);
    let mut sum = 0.0f64;
    let start = (from * sr) as usize;
    for (i, &x) in buf.iter().enumerate() {
        let y = a * (prev_y + x - prev_x);
        prev_x = x;
        prev_y = y;
        if i >= start {
            sum += (y * y) as f64;
        }
    }
    ((sum / (n - start).max(1) as f64).sqrt()) as f32
}
