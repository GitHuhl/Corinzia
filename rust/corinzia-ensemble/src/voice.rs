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

use crate::bowed;
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

/// How a note speaks, as the articulation controller (CC9) gives it: a
/// kind, and the bowing (written, else alternating) in the high bits.
pub mod articulation {
    pub const NORMAL: u8 = 0;
    pub const STACCATO: u8 = 1;
    pub const STACCATISSIMO: u8 = 2;
    /// A short unslurred note in a quick passage: off the string, lightly
    /// tongued.
    pub const SPICCATO: u8 = 3;
    pub const TENUTO: u8 = 4;
    pub const MARCATO: u8 = 5;
    pub const KIND: u8 = 0x0f;
    pub const DOWN_BOW: u8 = 0x10;
    pub const UP_BOW: u8 = 0x20;
}

/// What drives a hybrid's waveguide.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Exciter {
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
    /// A measured body ([`crate::body`]): the strings' harmonic levels come
    /// from it instead of the slope and peaks.
    pub body: Option<u8>,
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
        if n.is_multiple_of(2) && hz < self.even_below {
            l += self.even_db;
        }
        l
    }
}

const MAX_HARMONICS: usize = 40;
/// log2 of the harmonic numbers.
#[allow(clippy::approx_constant)] // log2(10) is in the table, to four places
static LOG2: [f32; MAX_HARMONICS + 1] = {
    // log2(n) for n in 0..=40 (0 for n = 0), to four decimals.
    [0.0, 0.0, 1.0, 1.585, 2.0, 2.3219, 2.585, 2.8074, 3.0, 3.1699, 3.3219, 3.4594, 3.585, 3.7004, 3.8074, 3.9069, 4.0, 4.0875, 4.1699, 4.2479, 4.3219, 4.3923, 4.4594, 4.5236, 4.585, 4.6439, 4.7004, 4.7549, 4.8074, 4.858, 4.9069, 4.9542, 5.0, 5.0444, 5.0875, 5.1293, 5.1699, 5.2095, 5.2479, 5.2854, 5.3219]
};
/// Players of a string section, each played in full.
const PLAYERS: usize = 6;
const MAX_MODES: usize = 16;
const CONTROL: usize = 16; // samples between control updates

/// The most players one note can have (a string section's size).
pub const MAX_SECTION: usize = 32;
/// How hard a finger pulls a string aside, against the model's own pluck:
/// further than the bow drives it, so a pizzicato speaks about as loud as
/// a bowed note at its dynamic (3 dB under, over its first tenth of a
/// second), then dies away.
const PLUCK_PULL: f32 = 4.6;

/// A section's players computed together (see [`crate::lanes`]).
pub const GROUP: usize = 8;
/// The most groups one note can take.
const MAX_GROUPS: usize = MAX_SECTION.div_ceil(GROUP);
/// Soloists' instruments and sections' groups in the shared pool.
pub const SOLOS: usize = 32;
pub const GROUPS: usize = 16;

/// Where a note's players sit in the pool: a soloist's instrument, or a
/// section's groups (each up to [`GROUP`] players computed together).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Seats {
    solo: Option<u16>,
    groups: [u16; MAX_GROUPS],
    count: u8,
    players: u8,
}

/// One instrument in the pool: a soloist's, or a section's group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seat {
    Solo(u16),
    Group(u16),
}

impl Seats {
    /// How many players.
    pub fn len(&self) -> usize {
        self.players as usize
    }
    pub fn is_empty(&self) -> bool {
        self.players == 0
    }
    pub fn holds(&self, seat: Seat) -> bool {
        match seat {
            Seat::Solo(i) => self.solo == Some(i),
            Seat::Group(g) => self.groups[..self.count as usize].contains(&g),
        }
    }
    /// Each instrument and the players (from 0) it plays.
    fn each(&self) -> impl Iterator<Item = (Seat, std::ops::Range<usize>)> + '_ {
        let n = self.players as usize;
        self.solo.map(|i| (Seat::Solo(i), 0..1)).into_iter().chain(self.groups[..self.count as usize].iter().enumerate().map(move |(k, &g)| (Seat::Group(g), k * GROUP..((k + 1) * GROUP).min(n))))
    }
}

/// What a voice asks of an instrument, whatever its number of players:
/// each player's values as a slice (missing players: silent).
pub trait Players {
    fn reset(&mut self);
    fn kind(&self) -> usize;
    fn configure(&mut self, instrument: usize);
    fn note(&mut self, frequency: &[f32], key: i32, pluck: bool, velocity: f32);
    fn slur(&mut self, frequency: &[f32], key: i32, velocity: f32);
    fn pluck(&mut self, width: &[f32], strength: &[f32], wait: &[f32]);
    fn frequency(&mut self, frequency: &[f32]);
    fn controls(&mut self, c: bowed::Controls);
    /// One sample: the players' sum.
    fn tick(&mut self, envelope: &[f32], vibrato: &[f32], gesture: &[f32]) -> f32;
}

/// `x`'s values, the rest `pad`.
#[inline(always)]
fn lanes<const N: usize>(x: &[f32], pad: f32) -> crate::lanes::Fx<N> {
    crate::lanes::Fx(std::array::from_fn(|l| x.get(l).copied().unwrap_or(pad)))
}

impl<const N: usize> Players for bowed::Instrument<N> {
    fn reset(&mut self) {
        bowed::Instrument::reset(self)
    }
    fn kind(&self) -> usize {
        self.instrument()
    }
    fn configure(&mut self, instrument: usize) {
        bowed::Instrument::configure(self, instrument)
    }
    fn note(&mut self, frequency: &[f32], key: i32, pluck: bool, velocity: f32) {
        bowed::Instrument::note(self, lanes::<N>(frequency, frequency[0]), key, pluck, velocity)
    }
    fn slur(&mut self, frequency: &[f32], key: i32, velocity: f32) {
        bowed::Instrument::slur(self, lanes::<N>(frequency, frequency[0]), key, velocity)
    }
    fn pluck(&mut self, width: &[f32], strength: &[f32], wait: &[f32]) {
        self.set_pluck(lanes::<N>(width, 0.004), lanes::<N>(strength, 0.0), lanes::<N>(wait, 0.0))
    }
    fn frequency(&mut self, frequency: &[f32]) {
        self.set_frequency(lanes::<N>(frequency, frequency[0]))
    }
    fn controls(&mut self, c: bowed::Controls) {
        bowed::Instrument::controls(self, c)
    }
    #[inline(always)]
    fn tick(&mut self, envelope: &[f32], vibrato: &[f32], gesture: &[f32]) -> f32 {
        bowed::Instrument::tick(self, lanes::<N>(envelope, 0.0), lanes::<N>(vibrato, 1.0), lanes::<N>(gesture, 1.0)).sum()
    }
}

/// Physically modelled string players ([`crate::bowed`]), shared by the
/// voices: a bowed note takes as many as its part has, and gives them back
/// when it ends. Allocated with the voices, not while playing.
pub struct PlayerPool {
    pub solos: Vec<bowed::Player>,
    pub groups: Vec<bowed::Instrument<GROUP>>,
    /// The voice each is with (`u32::MAX`: none).
    solo_owner: Vec<u32>,
    group_owner: Vec<u32>,
}

impl PlayerPool {
    /// `solos` soloists' instruments and `groups` sections' groups.
    pub fn new(sample_rate: f64, solos: usize, groups: usize) -> PlayerPool {
        PlayerPool {
            solos: (0..solos).map(|i| bowed::Player::new(sample_rate, [i as u32])).collect(),
            groups: (0..groups).map(|g| bowed::Instrument::new(sample_rate, std::array::from_fn(|l| (g * GROUP + l) as u32))).collect(),
            solo_owner: vec![u32::MAX; solos],
            group_owner: vec![u32::MAX; groups],
        }
    }

    /// Room for a note of `players` (tests, measurements).
    pub fn for_players(sample_rate: f64, players: usize) -> PlayerPool {
        PlayerPool::new(sample_rate, 1, players.div_ceil(GROUP))
    }

    pub fn empty() -> PlayerPool {
        PlayerPool::new(48000.0, 0, 0)
    }

    pub fn seat(&mut self, seat: Seat) -> &mut dyn Players {
        match seat {
            Seat::Solo(i) => &mut self.solos[i as usize],
            Seat::Group(g) => &mut self.groups[g as usize],
        }
    }

    fn has(&self, seat: Seat) -> bool {
        match seat {
            Seat::Solo(i) => (i as usize) < self.solos.len(),
            Seat::Group(g) => (g as usize) < self.groups.len(),
        }
    }

    /// Seats for `n` players of voice `voice`: a soloist's instrument for
    /// one, else as many groups as free (fewer players if not enough).
    /// Free: with no voice, or with one no longer holding it (`held`).
    pub fn take(&mut self, voice: u32, n: usize, held: impl Fn(u32, Seat) -> bool) -> Seats {
        let n = n.min(MAX_SECTION);
        let mut seats = Seats::default();
        let free = |owner: u32, seat: Seat| owner == u32::MAX || owner == voice || !held(owner, seat);
        if n == 1 {
            if let Some(i) = (0..self.solos.len()).find(|&i| free(self.solo_owner[i], Seat::Solo(i as u16))) {
                self.solo_owner[i] = voice;
                seats.solo = Some(i as u16);
                seats.players = 1;
                return seats;
            }
        }
        for g in 0..self.groups.len() {
            if seats.count as usize == n.div_ceil(GROUP) {
                break;
            }
            if free(self.group_owner[g], Seat::Group(g as u16)) {
                self.group_owner[g] = voice;
                seats.groups[seats.count as usize] = g as u16;
                seats.count += 1;
            }
        }
        seats.players = n.min(seats.count as usize * GROUP) as u8;
        seats
    }
}

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
    /// The slide's time (seconds) for a slur, from which each player's own.
    glide_time: f32,
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
    /// 1, or less for a note taken up from a still-ringing one (a bow
    /// change, a new breath): its attack is quicker.
    attack_scale: f32,
    /// The articulation's bite (a bright, noisy start, 0..1) and how much
    /// shorter the release is (short notes: the bow lifts, the tongue stops).
    bite: f32,
    release_scale: f32,
    /// The waveguide's and the noise's gains as they sound (smoothed).
    phys_g: f32,
    noise_g: f32,
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
    // a section's players: their harmonics, tuning, vibrato and entry
    pl_re: [[f32; MAX_HARMONICS]; PLAYERS],
    pl_im: [[f32; MAX_HARMONICS]; PLAYERS],
    pl_c: [[f32; MAX_HARMONICS]; PLAYERS],
    pl_s: [[f32; MAX_HARMONICS]; PLAYERS],
    pl_amp: [[f32; MAX_HARMONICS]; PLAYERS],
    pl_target: [[f32; MAX_HARMONICS]; PLAYERS],
    pl_n: [usize; PLAYERS],
    pl_detune: [f32; PLAYERS],
    pl_vib: [f32; PLAYERS],
    pl_rate: [f32; PLAYERS],
    pl_depth: [f32; PLAYERS],
    pl_delay: [f32; PLAYERS],
    pl_wander: [f32; PLAYERS],
    /// Each player's instrument: its own small differences per harmonic.
    pl_colour: [[f32; MAX_HARMONICS]; PLAYERS],
    phys_lp: f32,
    /// The body's gains for the current dynamic and technique.
    curve: Box<[f32; crate::body::FINE]>,
    curve_even: Box<[f32; crate::body::FINE]>,
    curve_key: (f32, u8),
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
    // physically modelled strings (see `PlayerPool`): this note's players
    // in the pool, which instrument, and each player's differences
    pub physical: bool,
    ph_n: usize,
    ph_seats: Seats,
    ph_kind: usize,
    /// The square root of the sound's own size (a section's sum is heard
    /// against it: more players, louder).
    ph_ref: f32,
    ph_detune: [f32; MAX_SECTION],
    ph_wander: [f32; MAX_SECTION],
    ph_delay: [f32; MAX_SECTION],
    ph_vib: [f32; MAX_SECTION],
    ph_vib_rate: [f32; MAX_SECTION],
    ph_vib_depth: [f32; MAX_SECTION],
    ph_vibf: [f32; MAX_SECTION],
    /// When each player's vibrato starts (seconds), and its rate's slow
    /// wander (a fraction).
    ph_vib_start: [f32; MAX_SECTION],
    ph_vib_drift: [f32; MAX_SECTION],
    /// Each player's own slur: where their finger is (Hz), how quickly it
    /// slides (a smoothing coefficient), and how long until it moves.
    ph_cur: [f32; MAX_SECTION],
    ph_glide: [f32; MAX_SECTION],
    ph_wait: [f32; MAX_SECTION],
    ph_trem: [f32; MAX_SECTION],
    ph_trem_phase: [f32; MAX_SECTION],
    /// Plucked (pizzicato), not bowed.
    ph_pluck: bool,
    ph_dir: f32,
    ph_key_changed: bool,
    /// In a slur, each group's string changed yet.
    ph_slurred: [bool; MAX_GROUPS],
    /// The players' gain, as last worked out (the control cycle runs on
    /// across the host's split blocks).
    ph_gain: f32,
}

fn key_hz(key: f32) -> f32 {
    440.0 * 2f32.powf((key - 69.0) / 12.0)
}

/// How a physically modelled player bows at a dynamic `d` (0..1), with a
/// technique, and a bite (an accent, a marked attack, a slur's join):
/// louder, a faster bow pressed harder and a brighter tone; near the
/// bridge or over the fingerboard; a mute damps the body.
pub fn bowing(d: f32, technique: u8, bite: f32, direction: f32) -> bowed::Controls {
    bowed::Controls {
        // Pressed enough at every dynamic for the string to speak (a
        // too light bow on a slow stroke only scrapes): see the
        // `playability` test.
        // A bite (an accent, a staccato's or marcato's start) is a faster
        // stroke pressed harder: the string catches sooner, and louder.
        force: (0.55 + 0.4 * d).min(1.0) * (1.0 + 0.5 * bite),
        speed: (0.25 + 0.65 * d) * (1.0 + 0.4 * bite),
        direction,
        engagement: 1.0,
        position: match technique {
            tech::SUL_PONT => 0.0,
            tech::SUL_TASTO | tech::HARMONICS => 1.0,
            _ => 0.35,
        },
        brightness: (match technique {
            // A harmonic: the light touch leaves a pure, glassy tone.
            tech::HARMONICS => 0.15,
            tech::SUL_PONT => 0.65 + 0.3 * d,
            _ => 0.4 + 0.3 * d,
        } + 0.5 * bite)
            .min(1.0),
        coupling: if technique == tech::CON_SORD { 0.2 } else { 0.6 },
        sympathetic: 0.45,
        rosin: 0.5,
    }
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
            glide_time: 0.03,
            t: 0.0,
            env: 0.0,
            env_rate: 0.0,
            rel_mul: 1.0,
            dynamic: 0.5,
            dyn_target: 0.5,
            vel: 0.5,
            accent: 0.0,
            join: 0.0,
            attack_scale: 1.0,
            bite: 0.0,
            release_scale: 1.0,
            phys_g: 0.0,
            noise_g: 0.0,
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
            pl_re: [[0.0; MAX_HARMONICS]; PLAYERS],
            pl_im: [[0.0; MAX_HARMONICS]; PLAYERS],
            pl_c: [[1.0; MAX_HARMONICS]; PLAYERS],
            pl_s: [[0.0; MAX_HARMONICS]; PLAYERS],
            pl_amp: [[0.0; MAX_HARMONICS]; PLAYERS],
            pl_target: [[0.0; MAX_HARMONICS]; PLAYERS],
            pl_n: [0; PLAYERS],
            pl_detune: [0.0; PLAYERS],
            pl_vib: [0.0; PLAYERS],
            pl_rate: [5.5; PLAYERS],
            pl_depth: [1.0; PLAYERS],
            pl_delay: [0.0; PLAYERS],
            pl_wander: [0.0; PLAYERS],
            pl_colour: [[1.0; MAX_HARMONICS]; PLAYERS],
            phys_lp: 0.0,
            curve: Box::new([0.0; crate::body::FINE]),
            curve_even: Box::new([0.0; crate::body::FINE]),
            curve_key: (-1.0, 255),
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
            physical: false,
            ph_n: 0,
            ph_seats: Seats::default(),
            ph_kind: 0,
            ph_ref: 1.0,
            ph_detune: [0.0; MAX_SECTION],
            ph_wander: [0.0; MAX_SECTION],
            ph_delay: [0.0; MAX_SECTION],
            ph_vib: [0.0; MAX_SECTION],
            ph_vib_rate: [0.0; MAX_SECTION],
            ph_vib_depth: [1.0; MAX_SECTION],
            ph_vibf: [1.0; MAX_SECTION],
            ph_vib_start: [0.15; MAX_SECTION],
            ph_vib_drift: [0.0; MAX_SECTION],
            ph_cur: [440.0; MAX_SECTION],
            ph_glide: [0.0; MAX_SECTION],
            ph_wait: [0.0; MAX_SECTION],
            ph_trem: [6.0; MAX_SECTION],
            ph_trem_phase: [0.25; MAX_SECTION],
            ph_pluck: false,
            ph_dir: 1.0,
            ph_key_changed: false,
            ph_slurred: [true; MAX_GROUPS],
            ph_gain: 0.0,
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
        self.physical = false;
        self.ph_n = 0;
        self.rng = Rng(0x9E37_79B9 ^ seed.wrapping_mul(2_654_435_761) ^ (key as u32) << 16 | 1);
        self.vel = velocity.clamp(0.0, 1.0);
        self.model = match (p.model, technique) {
            (Model::Bowed, tech::PIZZ) => Model::Pluck(0.4),
            (Model::Bowed, tech::COL_LEGNO) => Model::Pluck(0.9),
            (m, _) => m,
        };
        let percussive = matches!(self.model, Model::Pluck(_) | Model::Piano | Model::Mallet(_) | Model::Timpani | Model::Kit);
        self.sustained = !percussive;
        // A held sound starts at its attack level (an accent, an sfz) and
        // settles to the prevailing dynamic.
        let settled = if percussive { self.vel } else { dynamic.unwrap_or(self.vel) };
        self.dynamic = if percussive { self.vel } else { settled.max(self.vel.min(settled + 0.4)) };
        self.dyn_target = settled;
        self.accent = if percussive { 0.0 } else { (self.vel - self.dynamic).max(0.0) * 2.5 + if self.vel > 0.8 { (self.vel - 0.8) * 1.5 } else { 0.0 } };
        self.freq = key_hz(key as f32);
        self.cur = self.freq;
        self.glide = dsp::smoothing(0.03, sr);
        self.glide_time = 0.03;
        self.t = 0.0;
        self.join = 0.0;
        self.attack_scale = 1.0;
        self.bite = 0.0;
        self.release_scale = 1.0;
        self.phys_g = 0.0;
        self.noise_g = 0.0;
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
        // A section's players: none quite in tune with the others, each
        // with a vibrato of their own, coming in a little apart.
        const DETUNE: [f32; PLAYERS] = [0.0, 0.8, -0.7, 0.35, -0.3, 0.55];
        for (q, detune) in DETUNE.iter().enumerate() {
            let section = p.unison > 1;
            self.pl_detune[q] = if section { p.spread * (detune + 0.3 * self.rng.noise()) } else { 0.0 };
            self.pl_rate[q] = p.vib_rate * (0.92 + 0.16 * self.rng.unit());
            self.pl_vib[q] = self.rng.unit();
            self.pl_depth[q] = 0.8 + 0.4 * self.rng.unit();
            // No two instruments alike: up to ±3 dB per harmonic, so the
            // players never cancel all at once.
            for h in 0..MAX_HARMONICS {
                self.pl_colour[q][h] = if section { db(3.0 * self.rng.noise()) } else { 1.0 };
            }
            self.pl_delay[q] = if section && q > 0 { 0.03 * self.rng.unit() * (1.2 - self.vel) } else { 0.0 };
            self.pl_wander[q] = 0.0;
            self.pl_n[q] = 0;
            for h in 0..MAX_HARMONICS {
                let a = std::f32::consts::PI * (h * (h + 1)) as f32 / MAX_HARMONICS as f32 + 1.37 * (q * (h + 1)) as f32;
                self.pl_re[q][h] = a.cos();
                self.pl_im[q][h] = a.sin();
                self.pl_amp[q][h] = 0.0;
                self.pl_target[q][h] = 0.0;
            }
        }
        self.phys_lp = 0.0;
        self.curve_key = (-1.0, 255);
        self.line.fill(0.0);
        self.pos = 0;
        self.loop_lp = 0.0;
        match self.model {
            // Bowed strings' plucks are their players'.
            Model::Pluck(_) if p.model == Model::Bowed => {}
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
        // A string player's finger slides on a wide leap, audibly.
        let time = if self.model == Model::Bowed && leap >= 3.0 { 0.035 + 0.075 * leap / 12.0 } else { time * (0.5 + leap / 12.0) };
        self.glide = dsp::smoothing(time, self.sr);
        self.glide_time = time;
        self.join = 0.35 + 0.3 * velocity;
        if let Some(s) = crate::patches::spectrum(self.patch) {
            self.tune_waveguide(s);
        }
        // The players' fingers move too (on the next block: the players
        // are in the pool).
        self.ph_key_changed = true;
    }

    /// How the note speaks ([`articulation`]): its attack, its bite and its
    /// release; `down_bow`: a bowed note's stroke (a little heavier).
    pub fn articulate(&mut self, code: u8, down_bow: bool) {
        use articulation as a;
        let (attack, bite, release): (f32, f32, f32) = match code & a::KIND {
            a::STACCATO => (0.45, 0.5, 0.5),
            a::STACCATISSIMO => (0.3, 0.7, 0.35),
            a::SPICCATO => (0.4, 0.35, 0.55),
            a::TENUTO => (1.25, 0.0, 1.0),
            a::MARCATO => (0.5, 1.0, 0.8),
            _ => (1.0, 0.0, 1.0),
        };
        let bow: f32 = if self.model == Model::Bowed && down_bow { 0.2 } else { 0.0 };
        self.attack_scale *= attack;
        self.bite = (bite + bow).min(1.0);
        self.release_scale = release;
        let p = self.p();
        let time = (p.attack * (1.5 - self.vel) * self.attack_scale).max(0.002);
        self.env_rate = dsp::smoothing(time / 3.0, self.sr);
    }

    /// Still sounding after its note ended (a bow change can take it up).
    pub fn is_ringing(&self) -> bool {
        self.active && self.releasing && self.sustained && self.env > 0.02
    }

    /// The note just started follows one still ringing on its instrument:
    /// the bow or breath changes rather than starting from silence.
    pub fn connect(&mut self) {
        let p = self.p();
        self.attack_scale = 0.2;
        let attack = (p.attack * (1.5 - self.vel) * self.attack_scale).max(0.002);
        self.env_rate = dsp::smoothing(attack / 3.0, self.sr);
        self.env = 0.5;
        self.scoop *= 0.3;
        // A section changes bow nearly together.
        for d in self.pl_delay.iter_mut() {
            *d *= 0.25;
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
            // A short stroke: the bow lifts, the tongue stops (a string
            // still rings a little).
            Model::Bowed => (p.release * self.release_scale).max(0.12),
            _ => p.release * self.release_scale,
        };
        self.releasing = true;
        self.rel_mul = dsp::decay(time, self.sr);
    }

    /// A new technique for a held note (not one already ending: its tail
    /// is not played anew).
    pub fn set_technique(&mut self, t: u8) {
        if self.sustained && !self.releasing {
            self.tech = t;
        }
    }

    /// Whether a note just started is played by physically modelled
    /// players: every bowed string's, in every technique.
    pub fn wants_players(&self) -> bool {
        PATCHES[self.patch].model == Model::Bowed
    }

    /// Plays the note just started (and articulated) with these players
    /// from the pool: each its own instrument, a little out of tune with
    /// the others, with a vibrato of their own, coming in a little apart.
    /// `down_bow`: the stroke.
    pub fn attach_players(&mut self, seats: Seats, pool: &mut PlayerPool, down_bow: bool) {
        let p = self.p();
        let n = seats.len();
        if n == 0 {
            return;
        }
        self.physical = true;
        self.ctl = 0;
        self.ph_n = n;
        self.ph_seats = seats;
        // The instrument: by the sound's name, else (the string orchestra) by register.
        self.ph_kind = crate::patches::bowed_instrument(self.patch).unwrap_or(match self.key {
            55.. => bowed::VIOLIN,
            48..=54 => bowed::VIOLA,
            36..=47 => bowed::CELLO,
            _ => bowed::BASS,
        });
        self.ph_ref = (p.unison.max(1) as f32).sqrt();
        self.ph_dir = if down_bow { 1.0 } else { -1.0 };
        // Plucked, or struck with the bow's wood (col legno).
        let pluck = matches!(self.tech, tech::PIZZ | tech::COL_LEGNO);
        self.ph_pluck = pluck;
        if pluck {
            // A pluck sounds at once: no bow to bring in.
            self.env = 1.0;
        }
        let section = n > 1;
        let (mut width, mut strength, mut wait, mut freq) = ([0.0f32; MAX_SECTION], [0.0f32; MAX_SECTION], [0.0f32; MAX_SECTION], [0.0f32; MAX_SECTION]);
        for q in 0..n {
            self.ph_detune[q] = if section { 7.0 * self.rng.noise() } else { 0.0 };
            self.ph_wander[q] = 0.0;
            // A section comes in a little apart (plucks closer together).
            // An accent or a marked attack: they come in together.
            let together = 1.0 - 0.75 * self.bite.min(1.0);
            self.ph_delay[q] = if section && q > 0 { (if pluck { 0.025 } else { 0.03 * (1.2 - self.vel) }) * together * self.rng.unit() } else { 0.0 };
            self.ph_vib[q] = self.rng.unit();
            // Each player's own vibrato: in a section, rates from about 4.8
            // to 6.8 a second (at a sound's 5.8), starting after the attack
            // at their own moment.
            self.ph_vib_rate[q] = p.vib_rate * if section { 0.83 + 0.34 * self.rng.unit() } else { 0.95 + 0.1 * self.rng.unit() };
            self.ph_vib_depth[q] = if section { 0.75 + 0.5 * self.rng.unit() } else { 1.0 };
            self.ph_vib_start[q] = if section { 0.1 + 0.2 * self.rng.unit() } else { 0.15 };
            self.ph_vib_drift[q] = 0.0;
            self.ph_vibf[q] = 1.0;
            self.ph_cur[q] = self.freq;
            self.ph_glide[q] = self.glide;
            self.ph_wait[q] = 0.0;
            // Tremolo: the bow's back and forth, 5 to 6.5 times a second (10
            // to 13 strokes), each player at their own pace, starting
            // mid-stroke (the bow already moving).
            self.ph_trem[q] = 5.0 + if section { 1.5 * self.rng.unit() } else { 0.75 };
            self.ph_trem_phase[q] = 0.25 + if section { 0.1 * self.rng.noise() } else { 0.0 };
            // No two fingers pluck alike: the one plucking alone as the
            // table measured; in a section, each softer or firmer, each at
            // their own moment. (The same pull in all, so the width colours
            // rather than louder: a wider pluck is the gentler.)
            let (w, st) = if section { (0.003 + 0.004 * self.rng.unit(), 0.8 + 0.4 * self.rng.unit()) } else { (0.004, 1.0) };
            (width[q], strength[q]) = if self.tech == tech::COL_LEGNO {
                // The wood: a short, hard tap, lighter than a finger's pull.
                (0.0012, 0.5 * PLUCK_PULL * st)
            } else {
                (w, PLUCK_PULL * st * 0.004 / w)
            };
            wait[q] = if pluck { self.ph_delay[q] } else { 0.0 };
            freq[q] = self.freq * (self.ph_detune[q] / 1200.0).exp2();
        }
        for (seat, range) in seats.each() {
            let inst = pool.seat(seat);
            inst.reset();
            if inst.kind() != self.ph_kind {
                inst.configure(self.ph_kind);
            }
            inst.pluck(&width[range.clone()], &strength[range.clone()], &wait[range.clone()]);
            inst.note(&freq[range], self.key as i32, pluck, self.vel);
        }
        self.ph_key_changed = false;
    }

    /// Plucked by physically modelled players (their pizzicato).
    pub fn plucked(&self) -> bool {
        self.physical && self.ph_pluck
    }

    pub fn technique(&self) -> u8 {
        self.tech
    }

    /// How many players this voice has in the pool (none once it has
    /// ended). (See `bowing` for how they play.)
    pub fn players(&self) -> usize {
        if self.active && self.physical { self.ph_n } else { 0 }
    }

    /// Whether this voice holds an instrument of the pool.
    pub fn holds(&self, seat: Seat) -> bool {
        self.players() > 0 && self.ph_seats.holds(seat)
    }

    fn render_physical(&mut self, out: &mut [f32], bend: f32, trims: &[f32], pool: &mut PlayerPool) {
        let mut p = *self.p();
        p.attack *= self.attack_scale;
        let sr = self.sr;
        let dt = 1.0 / sr;
        let dyn_rate = dsp::smoothing(0.04, sr);
        let blocks = CONTROL as f32 / sr;
        let n = self.ph_n;
        if self.ph_key_changed {
            // A slur: in a section, each player's finger moves at their own
            // moment (within 20 ms) and slides at their own pace (0.6 to 1.6
            // times the slide); a soloist's at once.
            // (The string a group plays changes with its first player's
            // finger: until each moves, the others play on at their pitch.)
            self.ph_key_changed = false;
            let section = n > 1;
            for q in 0..n {
                let pace = if section { 0.6 + self.rng.unit() } else { 1.0 };
                self.ph_glide[q] = dsp::smoothing(self.glide_time * pace, sr);
                self.ph_wait[q] = if section { 0.02 * self.rng.unit() } else { 0.0 };
            }
            self.ph_slurred = [false; MAX_GROUPS];
        }
        for o in out.iter_mut() {
            if self.ctl == 0 {
                self.ctl = CONTROL;
                // An accent or an fp holds its attack level until the note has
                // spoken, then settles.
                if self.t > (p.attack * (1.5 - self.vel) * 1.5).max(0.06) || self.dyn_target > self.dynamic {
                    self.dynamic += (self.dyn_target - self.dynamic) * (1.0 - (1.0 - dyn_rate).powi(CONTROL as i32));
                }
                self.cur += (self.freq - self.cur) * (1.0 - (1.0 - self.glide).powi(CONTROL as i32));
                self.scoop *= dsp::decay(0.08, sr / CONTROL as f32);
                let d = self.dynamic;
                let bite = (1.0 - self.t / (p.attack * 3.0 + 0.03)).max(0.0) * self.bite + self.accent * (-self.t / 0.09).exp() + self.join;
                let controls = bowing(d, self.tech, bite, self.ph_dir);
                self.join *= dsp::decay(0.12, sr / CONTROL as f32);
                // Vibrato: after the attack, wider when louder.
                let depth = if self.ph_pluck || self.tech == tech::HARMONICS { 0.0 } else { p.vibrato * (0.6 + 0.7 * d) };
                let scoop = 2f32.powf((bend + self.scoop) / 12.0);
                let mut freq = [0.0f32; MAX_SECTION];
                for (q, freq) in freq.iter_mut().enumerate().take(n) {
                    // The finger: waiting to move, then sliding to the note.
                    if self.ph_wait[q] > 0.0 {
                        self.ph_wait[q] -= blocks;
                    } else {
                        self.ph_cur[q] += (self.freq - self.ph_cur[q]) * (1.0 - (1.0 - self.ph_glide[q]).powi(CONTROL as i32));
                    }
                    // The rate wanders a little (a few per cent), slowly.
                    if n > 1 {
                        self.ph_vib_drift[q] = (self.ph_vib_drift[q] + 0.02 * self.rng.noise() * blocks.sqrt()).clamp(-0.04, 0.04);
                    }
                    self.ph_vib[q] = (self.ph_vib[q] + self.ph_vib_rate[q] * (1.0 + self.ph_vib_drift[q]) * blocks).fract();
                    let fade = ((self.t - self.ph_vib_start[q]) / 0.4).clamp(0.0, 1.0);
                    // Intonation wanders a little, slowly.
                    if n > 1 {
                        self.ph_wander[q] = (self.ph_wander[q] + 6.0 * self.rng.noise() * blocks.sqrt()).clamp(-4.0, 4.0) * 0.999;
                    }
                    let vib = 100.0 * depth * fade * self.ph_vib_depth[q] * (self.ph_vib[q] * TAU).sin();
                    // The period's factor: the vibrato moves the finger.
                    self.ph_vibf[q] = (-vib / 1200.0).exp2();
                    *freq = self.ph_cur[q] * scoop * ((self.ph_detune[q] + self.ph_wander[q]) / 1200.0).exp2();
                }
                for (k, (seat, range)) in self.ph_seats.each().enumerate() {
                    let inst = pool.seat(seat);
                    // A slur: the group's string changes as its first finger moves.
                    if !self.ph_slurred[k] && range.clone().any(|q| self.ph_wait[q] <= 0.0) {
                        self.ph_slurred[k] = true;
                        inst.slur(&freq[range.clone()], self.key as i32, self.vel);
                    }
                    inst.frequency(&freq[range]);
                    inst.controls(controls);
                }
                // The loudness the sound is calibrated to at this dynamic
                // (what `trims` gives the earlier engine), over what the
                // players give here (see `bowed_levels`); a section's
                // players add up against the sound's own size.
                let _ = trims;
                // (A pluck goes through the same gain: how loud it is
                // against a bowed note at its dynamic is the instrument's.)
                let target = crate::patches::REFERENCE_RMS * db(p.level_db + p.range_db * (d - 0.62) / 0.84);
                self.ph_gain = target / (crate::bowed_levels::level(self.ph_kind, self.key as i32, d) * self.ph_ref);
            }
            self.ctl -= 1;
            self.envelope_step();
            self.t += dt;
            let (mut e, mut gesture) = ([0.0f32; MAX_SECTION], [1.0f32; MAX_SECTION]);
            for q in 0..n {
                e[q] = if self.t >= self.ph_delay[q] { self.env } else { 0.0 };
                if self.tech == tech::TREM {
                    gesture[q] = (TAU * (self.ph_trem[q] * self.t + self.ph_trem_phase[q])).sin();
                }
            }
            let mut x = 0.0;
            for (seat, range) in self.ph_seats.each() {
                x += pool.seat(seat).tick(&e[range.clone()], &self.ph_vibf[range.clone()], &gesture[range]);
            }
            *o += x * self.ph_gain * self.env;
        }
    }

    fn tune_waveguide(&mut self, s: &Spectrum) {
        // Losses: a bright, high-Q tube for reeds and brass, a softer string.
        self.loop_fb = match s.exciter {
            Exciter::Reed(_) => 0.97,
            Exciter::Lip => 0.975,
            Exciter::Jet => 0.95,
        };
        self.loop_bright = match s.exciter {
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
        self.render_with(out, bend, trims, &mut PlayerPool::empty())
    }

    /// As [`Voice::render`], its bowed strings played by players from the pool.
    pub fn render_with(&mut self, out: &mut [f32], bend: f32, trims: &[f32], pool: &mut PlayerPool) -> bool {
        if !self.active {
            return false;
        }
        if self.physical && self.ph_seats.each().all(|(seat, _)| pool.has(seat)) {
            self.render_physical(out, bend, trims, pool);
            if self.releasing && self.env < 1e-4 {
                self.active = false;
            }
            return self.active;
        }
        // The host splits its blocks at every event: the control values are
        // the renderers' locals, so they are worked out afresh at the start
        // of every call (else the first samples of a split play silent).
        self.ctl = 0;
        // Bowed strings sound only through their players.
        if PATCHES[self.patch].model == Model::Bowed {
            self.active = false;
            return false;
        }
        match self.model {
            Model::Pluck(_) => self.render_pluck(out, bend, trims),
            Model::Piano | Model::Mallet(_) | Model::Timpani => self.render_modes(out, trims),
            Model::Kit => self.render_kit(out, trims),
            _ if crate::patches::spectrum(self.patch).is_some_and(|s| s.body.is_some()) => self.render_measured(out, bend, trims),
            Model::Reed(_) | Model::Flute | Model::Brass if crate::patches::spectrum(self.patch).is_some() => self.render_hybrid(out, bend, trims),
            Model::Bowed => {}
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
        let mut p = *self.p();
        p.attack *= self.attack_scale;
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
        let glide_k = dsp::smoothing(0.005, sr);
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
            self.noise_g += (noise_gain - self.noise_g) * glide_k;
            self.phys_g += (phys_gain - self.phys_g) * glide_k;
            noise *= self.noise_g * 0.25;
            // Physical layer: exciter into a tuned waveguide.
            let f = self.cur * ratio;
            let step = f / sr;
            self.phases[0] = (self.phases[0] + step).fract();
            let ph = self.phases[0];
            let exc = match s.exciter {
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
            let sum = harm * 0.5 + phys * self.phys_g + noise;
            *o += sum * gain * self.env * tech_amp;
        }
    }

    // ------------------------------------ strings, woodwinds, brass, measured --
    /// An instrument on its measured spectrum: each player's harmonics at
    /// the level the recordings give for their frequency and parity (so
    /// vibrato moves them through the resonances), bow or breath noise, and
    /// the waveguide with its exciter (bow, reed, lips, air jet) for the
    /// attack and the joins.
    fn render_measured(&mut self, out: &mut [f32], bend: f32, trims: &[f32]) {
        let s = *crate::patches::spectrum(self.patch).unwrap();
        let mut p = *self.p();
        p.attack *= self.attack_scale;
        let body = s.body.unwrap();
        let sr = self.sr;
        let dt = 1.0 / sr;
        let players = (p.unison as usize).clamp(1, PLAYERS);
        let solo = players == 1;
        let dyn_rate = dsp::smoothing(0.04, sr);
        let amp_rate = dsp::smoothing(0.006, sr);
        let (mut noise_gain, mut phys_gain, mut gain, mut tech_amp, mut period, mut f_main, mut exc_shape) = (0.0, 0.0, 0.0, 1.0, 100.0f32, self.freq, 0.5f32);
        let len = self.line.len();
        let phys_k = 1.0 - (-TAU * 2500.0 / sr).exp();
        let glide_k = dsp::smoothing(0.005, sr);
        for o in out.iter_mut() {
            if self.ctl == 0 {
                self.ctl = CONTROL;
                let blocks = CONTROL as f32 / sr;
                // An accent or an fp holds its attack level until the note has
                // spoken, then settles.
                if self.t > (p.attack * (1.5 - self.vel) * 1.5).max(0.06) || self.dyn_target > self.dynamic {
                    self.dynamic += (self.dyn_target - self.dynamic) * (1.0 - (1.0 - dyn_rate).powi(CONTROL as i32));
                }
                self.cur += (self.freq - self.cur) * (1.0 - (1.0 - self.glide).powi(CONTROL as i32));
                self.scoop *= dsp::decay(0.08, sr / CONTROL as f32);
                let d = self.dynamic;
                let base = self.cur * 2f32.powf((bend + self.scoop) / 12.0);
                f_main = base;
                period = (sr / base).clamp(2.0, len as f32 - 2.0);
                // The body's curve for this dynamic and technique (made
                // again only when they change).
                // (An accent settles quickly: steps of 0.02 are enough, the
                // harmonics glide between them.)
                if (d - self.curve_key.0).abs() > 0.02 || self.tech != self.curve_key.1 {
                    self.curve_key = (d, self.tech);
                    let (t, brass) = (self.tech, self.model == Model::Brass);
                    let shape = move |hz: f32| match t {
                        // A straight mute in the bell: the lows gone, a
                        // nasal peak near 2 kHz, the top cut.
                        tech::CON_SORD if brass => {
                            let o = (hz / 2000.0).log2();
                            -12.0 * (-o).clamp(0.0, 1.5) + 6.0 * (-o * o * 4.0).exp() - 6.0 * (o - 1.3).max(0.0)
                        }
                        _ => 0.0,
                    };
                    let shaped = t == tech::CON_SORD && brass;
                    if shaped {
                        crate::body::fine_curves(body, d, !solo, shape, &mut self.curve, &mut self.curve_even);
                    } else {
                        // The usual case: blended from the curves made ready.
                        crate::body::blend(body, d, !solo, &mut self.curve, &mut self.curve_even);
                    }
                }
                tech_amp = match self.tech {
                    tech::TREM => 0.45 + 0.55 * (self.t * 7.5 * std::f32::consts::PI).sin().abs(),
                    tech::FLUTTER => 0.65 + 0.35 * (self.t * 24.0 * TAU).sin(),
                    tech::CON_SORD if self.model == Model::Brass => 0.5,
                    tech::CON_SORD => 0.6,
                    tech::SUL_PONT | tech::SUL_TASTO => 0.8,
                    tech::HARMONICS => 0.5,
                    _ => 1.0,
                };
                // Vibrato: after the attack, wider when louder.
                let fade = ((self.t - 0.15) / 0.4).clamp(0.0, 1.0);
                let depth = if self.tech == tech::HARMONICS { 0.0 } else { p.vibrato * fade * (0.6 + 0.7 * d) };
                // The measured strings lie 35 dB and more under their 1 kHz region
                // past 10 kHz: harmonics up to there (a soloist to 12 kHz).
                let limit = (sr * 0.45).min(14000.0);
                for q in 0..players {
                    self.pl_vib[q] = (self.pl_vib[q] + self.pl_rate[q] * blocks).fract();
                    // Intonation wanders a little, slowly.
                    self.pl_wander[q] = (self.pl_wander[q] + 6.0 * self.rng.noise() * blocks.sqrt()).clamp(-4.0, 4.0) * 0.999;
                    let cents = self.pl_detune[q] + self.pl_wander[q] + 100.0 * depth * self.pl_depth[q] * (self.pl_vib[q] * TAU).sin();
                    let fq = base * 2f32.powf(cents / 1200.0);
                    let n_h = ((limit / fq) as usize).clamp(1, MAX_HARMONICS);
                    self.pl_n[q] = n_h;
                    let entry = ((self.t - self.pl_delay[q]) / (p.attack * (1.5 - self.vel)).max(0.005)).clamp(0.0, 1.0);
                    let log2_f = fq.log2();
                    let w = TAU * fq / sr;
                    let (c1, s1) = (w.cos(), w.sin());
                    let (mut cn, mut sn) = (c1, s1);
                    let mut power = 0.0;
                    for h in 0..n_h {
                        let a = if self.tech == tech::HARMONICS {
                            if h == 0 { 1.0 } else { 0.0 }
                        } else {
                            let curve = if h % 2 == 1 { &self.curve_even } else { &self.curve };
                            crate::body::at(curve, log2_f + LOG2[h + 1]) * self.pl_colour[q][h]
                        };
                        power += a * a;
                        self.pl_target[q][h] = a;
                        self.pl_c[q][h] = cn;
                        self.pl_s[q][h] = sn;
                        (cn, sn) = (cn * c1 - sn * s1, sn * c1 + cn * s1);
                        let m = self.pl_re[q][h] * self.pl_re[q][h] + self.pl_im[q][h] * self.pl_im[q][h];
                        // Back to the unit circle (one Newton step suffices).
                        let k = 1.5 - 0.5 * m;
                        self.pl_re[q][h] *= k;
                        self.pl_im[q][h] *= k;
                    }
                    let norm = if power > 0.0 { entry / (power.sqrt() * (players as f32).sqrt()) } else { 0.0 };
                    // An articulation's bite: the start brighter (up to 6 dB
                    // an octave of harmonics), fading in 60 ms.
                    let bite = self.bite * (-self.t / 0.06).exp();
                    for h in 0..n_h {
                        // The bow's start: the upper harmonics settle a little
                        // after the fundamental.
                        // (A bitten start sets them going sooner.)
                        let rise_t = p.attack * (1.0 + s.rise * LOG2[h + 1]) / (1.0 + 2.0 * self.bite);
                        let rise = (self.t / rise_t.max(1e-3)).min(1.0);
                        let bright = if bite > 0.01 { (bite * LOG2[h + 1]).exp2() } else { 1.0 };
                        self.pl_target[q][h] *= norm * rise * bright;
                    }
                    for h in n_h..MAX_HARMONICS {
                        self.pl_target[q][h] = 0.0;
                    }
                }
                // A section's players beat against each other; where a
                // harmonic's players happen to cancel (or pile up), it is
                // mostly evened out: the sum's power against the sum of
                // the powers, from the phasors themselves, so without lag.
                if !solo {
                    let n_max = self.pl_n[..players].iter().copied().max().unwrap_or(0);
                    for h in 0..n_max {
                        let (mut re, mut im, mut expect) = (0.0f32, 0.0f32, 0.0f32);
                        for q in 0..players {
                            if h < self.pl_n[q] {
                                let a = self.pl_target[q][h];
                                re += a * self.pl_re[q][h];
                                im += a * self.pl_im[q][h];
                                expect += a * a;
                            }
                        }
                        let actual = re * re + im * im;
                        if actual > 1e-20 && expect > 1e-20 {
                            let g = (expect / actual).powf(0.4).clamp(0.4, 2.5);
                            for q in 0..players {
                                self.pl_target[q][h] *= g;
                            }
                        }
                    }
                }
                for (i, &(c, w, _)) in s.peaks.iter().enumerate() {
                    self.nf[i].set(c.max(80.0), (1.0 / w.max(0.1)).clamp(0.5, 8.0), sr);
                }
                let attack_noise = (1.0 - self.t / (p.attack * 2.0 + 0.02)).max(0.0);
                noise_gain = db(s.noise_db + 4.0 * (0.62 - d))
                    * (1.0 + 3.0 * attack_noise * (1.0 + 2.0 * self.bite) + 2.0 * self.join)
                    * match self.tech {
                        tech::SUL_PONT => 2.5,
                        tech::HARMONICS => 1.5,
                        tech::FLUTTER => 3.0,
                        _ => 1.0,
                    };
                let attack_phys = (1.0 - self.t / (p.attack * 3.0 + 0.03)).max(0.0);
                phys_gain = s.physical + 0.5 * attack_phys * (1.0 + self.bite) + 0.5 * self.join;
                exc_shape = 0.3 + 0.6 * d;
                self.join *= dsp::decay(0.12, sr / CONTROL as f32);
                self.lp.set(base * (p.dark + (p.bright - p.dark) * d.powf(1.4)), 0.7, sr);
                gain = self.gain(trims) * (1.0 + self.accent * (-self.t / 0.09).exp());
            }
            self.ctl -= 1;
            self.envelope_step();
            self.t += dt;
            let mut harm = 0.0;
            for q in 0..players {
                let (re, im, c, sn, amp, target) = (&mut self.pl_re[q], &mut self.pl_im[q], &self.pl_c[q], &self.pl_s[q], &mut self.pl_amp[q], &self.pl_target[q]);
                for h in 0..self.pl_n[q] {
                    let r = re[h] * c[h] - im[h] * sn[h];
                    let i = re[h] * sn[h] + im[h] * c[h];
                    re[h] = r;
                    im[h] = i;
                    amp[h] += (target[h] - amp[h]) * amp_rate;
                    harm += i * amp[h];
                }
            }
            // Bow noise through the instrument's resonances.
            let n = self.rng.noise();
            let mut noise = 0.0;
            for (i, &(_, _, g)) in s.peaks.iter().enumerate() {
                noise += self.nf[i].tick(n).band * db(g * 0.5);
            }
            // Gains glide (a join's lift is no step).
            self.noise_g += (noise_gain - self.noise_g) * glide_k;
            self.phys_g += (phys_gain - self.phys_g) * glide_k;
            noise *= self.noise_g * 0.25;
            // The waveguide: the exciter into the tuned string or tube.
            let step = f_main / sr;
            self.phases[0] = (self.phases[0] + step).fract();
            let ph = self.phases[0];
            let raw = match s.exciter {
                Exciter::Reed(w) if w <= 0.0 => saw(ph, step),
                Exciter::Reed(w) => pulse(ph, step, w),
                Exciter::Lip => pulse(ph, step, 0.5 - 0.25 * exc_shape),
                Exciter::Jet => dsp::sine(ph) + 0.3 * n,
            };
            let exc = self.lp.tick(raw).low;
            let rd = self.pos as f32 + len as f32 - period;
            let i0 = rd as usize;
            let fr = rd - i0 as f32;
            let back = self.line[i0 % len] + (self.line[(i0 + 1) % len] - self.line[i0 % len]) * fr;
            self.loop_lp += (back - self.loop_lp) * self.loop_bright;
            self.line[self.pos] = (exc * 0.12 + self.loop_lp * self.loop_fb).clamp(-4.0, 4.0);
            self.pos = (self.pos + 1) % len;
            self.phys_lp += (self.loop_lp - self.phys_lp) * phys_k;
            let sum = harm * 0.5 + self.phys_lp * 0.25 * self.phys_g + noise;
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
    measure_with(patch, sample_rate, None, tech::ORD)
}

/// As [`measure`]; bowed strings played by physically modelled players
/// from `pool` (as many as the sound's own) when given.
pub fn measure_with(patch: usize, sample_rate: f64, pool: Option<&mut PlayerPool>, technique: u8) -> f32 {
    let p = &PATCHES[patch];
    measure_note(patch, sample_rate, pool, technique, if p.model == Model::Kit { 38 } else { p.ref_key as i16 })
}

/// As [`measure_with`], at another key.
pub fn measure_note(patch: usize, sample_rate: f64, pool: Option<&mut PlayerPool>, technique: u8, key: i16) -> f32 {
    measure_at(patch, sample_rate, pool, technique, key, 0.62)
}

/// As [`measure_note`], at a dynamic (0..1).
pub fn measure_at(patch: usize, sample_rate: f64, pool: Option<&mut PlayerPool>, technique: u8, key: i16, d: f32) -> f32 {
    let p = &PATCHES[patch];
    let mut v = Voice::new(sample_rate);
    v.start(patch, technique, 0, 0, 0, key, d, Some(d), 0, 1);
    let mut empty = PlayerPool::empty();
    let pool = match pool {
        Some(pool) => {
            if v.wants_players() {
                let seats = pool.take(0, p.unison.max(1) as usize, |_, _| false);
                v.attach_players(seats, pool, true);
            }
            pool
        }
        None => &mut empty,
    };
    let sr = sample_rate as f32;
    let (from, to) = if v.sustained { (0.3, 0.8) } else { (0.0, 0.1) };
    let n = (to * sr) as usize;
    let mut buf = vec![0.0f32; n];
    let ones = [1.0f32; crate::patches::COUNT];
    for chunk in buf.chunks_mut(256) {
        v.render_with(chunk, 0.0, &ones, pool);
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

#[cfg(test)]
mod levels {
    use super::*;
    use crate::bowed_levels::{DYNAMICS, HIGHEST, LOWEST};

    /// A soloist's level (dB, as `measure` hears it) for a note at a
    /// dynamic, as a voice plays it (the table's levels set to 1).
    fn raw(instrument: usize, key: i32, d: f32) -> f32 {
        let patch = (0..PATCHES.len()).find(|&i| PATCHES[i].unison == 1 && crate::patches::bowed_instrument(i) == Some(instrument)).unwrap();
        let p = &PATCHES[patch];
        let mut pool = PlayerPool::for_players(48000.0, 1);
        let measured = measure_at(patch, 48000.0, Some(&mut pool), tech::ORD, key as i16, d);
        // What the voice's gain was (`render_physical`), taken out.
        let gain = crate::patches::REFERENCE_RMS * db(p.level_db + p.range_db * (d - 0.62) / 0.84);
        20.0 * (measured / gain).max(1e-10).log10()
    }

    /// Writes `src/bowed_levels.rs`'s table: run by hand when the model or
    /// `bowing` changes (`cargo test --release -p corinzia-ensemble
    /// make_level_table -- --ignored`).
    #[test]
    #[ignore]
    fn make_level_table() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/bowed_levels.rs");
        let src = std::fs::read_to_string(path).unwrap();
        // (Alone: other tests in this process would hear raw levels.)
        crate::bowed_levels::RAW.store(true, std::sync::atomic::Ordering::Relaxed);
        let mut table = String::from("pub static DB: [&[[f32; 5]]; 4] = [\n");
        for instrument in bowed::VIOLIN..=bowed::BASS {
            table += "    &[\n";
            for key in LOWEST[instrument]..=HIGHEST[instrument] {
                let row: Vec<String> = DYNAMICS.iter().map(|&d| format!("{:.1}", raw(instrument, key, d))).collect();
                table += &format!("        [{}],\n", row.join(", "));
            }
            table += "    ],\n";
        }
        table += "];";
        // From the declaration to the end of its initialiser (past the type's own `];`).
        let start = src.find("pub static DB:").unwrap();
        let init = start + src[start..].find("= [").unwrap();
        let end = init + src[init..].find("];").unwrap() + 2;
        crate::bowed_levels::RAW.store(false, std::sync::atomic::Ordering::Relaxed);
        std::fs::write(path, format!("{}{}{}", &src[..start], table, &src[end..])).unwrap();
    }
}

#[cfg(test)]
mod playability {
    use super::*;

    /// Whether a note speaks evenly: its level (dB) and its pitch's
    /// steadiness under a mapping (force, speed) at a dynamic.
    fn speaks(instrument: usize, key: i32, force: f32, speed: f32) -> (f32, bool) {
        let sr = 48000.0;
        let mut inst = bowed::Player::new(sr, [0]);
        inst.configure(instrument);
        inst.note(key_hz(key as f32), key, false, 0.5);
        inst.controls(bowed::Controls { force, speed, ..bowed::Controls::default() });
        let n = (0.6 * sr) as usize;
        let x: Vec<f32> = (0..n).map(|_| inst.tick(1.0, 1.0, 1.0).sum()).collect();
        let rms = |a: &[f32]| (a.iter().map(|v| v * v).sum::<f32>() / a.len() as f32).sqrt();
        // Steady: the last two tenths alike in level (a Helmholtz motion holds).
        let (a, b) = (rms(&x[n * 6 / 10..n * 8 / 10]), rms(&x[n * 8 / 10..]));
        (20.0 * rms(&x[n / 2..]).max(1e-10).log10(), (a / b.max(1e-12)).log10().abs() < 0.05 && a > 1e-5)
    }

    /// Prints, for some mappings of the dynamic to the bow, how many notes
    /// fail to speak steadily or get quieter as the dynamic rises (run by hand).
    #[test]
    #[ignore]
    fn mappings() {
        type Map = fn(f32) -> (f32, f32);
        let maps: [(&str, Map); 4] = [
            ("heavier: f .55+.40d, v .25+.65d", |d| (0.55 + 0.4 * d, 0.25 + 0.65 * d)),
            ("heavier still: f .65+.30d, v .25+.65d", |d| (0.65 + 0.3 * d, 0.25 + 0.65 * d)),
            ("heavy, faster: f .60+.35d, v .35+.55d", |d| (0.6 + 0.35 * d, 0.35 + 0.55 * d)),
            ("heavy, slower: f .60+.35d, v .18+.72d", |d| (0.6 + 0.35 * d, 0.18 + 0.72 * d)),
        ];
        for (label, map) in maps {
            let (mut unsteady, mut falling, mut total) = (0, 0, 0);
            for (instrument, lo, hi) in [(bowed::VIOLIN, 55, 103), (bowed::VIOLA, 48, 91), (bowed::CELLO, 36, 81), (bowed::BASS, 28, 67)] {
                for key in (lo..=hi).step_by(2) {
                    let mut last = f32::NEG_INFINITY;
                    for d in [0.0f32, 0.25, 0.5, 0.75, 1.0] {
                        let (f, v) = map(d);
                        let (level, steady) = speaks(instrument, key, f, v);
                        total += 1;
                        if !steady {
                            unsteady += 1;
                        }
                        if level < last - 1.0 {
                            falling += 1;
                        }
                        last = level;
                    }
                }
            }
            println!("{label}: {unsteady} of {total} unsteady, {falling} quieter than the dynamic below");
        }
    }
}

#[cfg(test)]
mod section_players {
    use super::*;

    fn violins(players: usize) -> (Voice, PlayerPool) {
        let patch = (0..PATCHES.len()).find(|&i| PATCHES[i].name == "Violins").unwrap();
        let mut v = Voice::new(48000.0);
        v.start(patch, tech::ORD, 0, 0, 0, 69, 0.6, Some(0.6), 0, 5);
        let mut pool = PlayerPool::for_players(48000.0, players);
        let seats = pool.take(0, players, |_, _| false);
        v.attach_players(seats, &mut pool, true);
        (v, pool)
    }

    fn spread(x: &[f32]) -> f32 {
        x.iter().cloned().fold(f32::MIN, f32::max) - x.iter().cloned().fold(f32::MAX, f32::min)
    }

    #[test]
    fn a_sections_players_vibrate_and_slur_each_in_their_own_time() {
        let (mut v, mut pool) = violins(12);
        let n = 12;
        let rate = PATCHES[v.patch].vib_rate;
        // Vibrato rates over more than a cycle a second; their starts apart.
        assert!(spread(&v.ph_vib_rate[..n]) > 0.2 * rate, "rates {:?}", &v.ph_vib_rate[..n]);
        assert!(spread(&v.ph_vib_start[..n]) > 0.08, "starts {:?}", &v.ph_vib_start[..n]);
        let mut out = vec![0.0f32; 4800];
        v.render_with(&mut out, 0.0, crate::patches::trims(), &mut pool);
        // A slur: the fingers move at their own moments, slide at their own pace.
        v.legato_to(1, 72, 0.6, 1);
        v.render_with(&mut out[..16], 0.0, crate::patches::trims(), &mut pool);
        assert!(spread(&v.ph_wait[..n]) > 0.005, "moments {:?}", &v.ph_wait[..n]);
        assert!(spread(&v.ph_glide[..n]) > 0.25 * v.ph_glide[..n].iter().sum::<f32>() / n as f32, "paces {:?}", &v.ph_glide[..n]);
        // Midway, some have arrived and some not; after, all have.
        let target = key_hz(72.0);
        let mut mid = vec![0.0f32; 5760];
        v.render_with(&mut mid, 0.0, crate::patches::trims(), &mut pool);
        let near = v.ph_cur[..n].iter().filter(|&&f| (f / target - 1.0).abs() < 0.01).count();
        assert!(near > 0 && near < n, "{near} of {n} there after 120 ms");
        let mut long = vec![0.0f32; 24000];
        v.render_with(&mut long, 0.0, crate::patches::trims(), &mut pool);
        assert!(v.ph_cur[..n].iter().all(|&f| (f / target - 1.0).abs() < 0.002), "all there");
        // A soloist: no waiting.
        let patch = (0..PATCHES.len()).find(|&i| PATCHES[i].name == "Violin").unwrap();
        let mut s = Voice::new(48000.0);
        s.start(patch, tech::ORD, 0, 0, 0, 69, 0.6, Some(0.6), 0, 5);
        let mut pool1 = PlayerPool::for_players(48000.0, 1);
        let seats = pool1.take(0, 1, |_, _| false);
        s.attach_players(seats, &mut pool1, true);
        s.legato_to(1, 72, 0.6, 1);
        s.render_with(&mut out[..16], 0.0, crate::patches::trims(), &mut pool1);
        assert_eq!(s.ph_wait[0], 0.0);
    }
}
