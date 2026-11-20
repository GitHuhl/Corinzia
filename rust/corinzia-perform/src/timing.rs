//! Timing: no player is exactly on the beat.
//!
//! Each note is moved a little ahead of or behind its written time, by an
//! amount drawn from the player's [`Feel`]: mostly close to the beat, now
//! and then further, never far. Measured performances show two parts, and
//! both are modelled:
//!
//! - **drift**: a player's deviations are correlated in time, not a fresh
//!   throw of the dice on every note. Fluctuations in drumming, tapping and
//!   ensemble playing have long-range correlations, close to 1/f noise
//!   (Hennig et al., "The nature and perception of fluctuations in human
//!   musical rhythms", PLoS ONE 6, 2011; Räsänen et al., "Fluctuations of
//!   hi-hat timing and dynamics in a virtuoso drum track of a popular music
//!   recording", PLoS ONE 10, 2015). Drift here is a sum of smooth random
//!   curves over time scales from half a second to sixteen seconds, the
//!   longer ones weighted more (a 1/f^0.8 spectrum).
//! - **jitter**: what is left, note by note (the same for every note of
//!   one player's chord, which is played as one gesture).
//!
//! Then what is particular to an instrument:
//!
//! - **melody lead** on keyboards: a chord's top note sounds a little
//!   before the rest (Palmer, "On the assignment of structure in music
//!   performance", Music Perception 14, 1996; Goebl, "Melody lead in piano
//!   performance", JASA 110, 2001), and the other notes of a chord spread
//!   by a few milliseconds;
//! - **per-sound placement** in a drum kit: time-keeping (hi-hat, ride)
//!   tightest; the backbeat snare a touch behind, the bass drum on or a
//!   touch ahead (Danielsen et al., "Effects of instructed timing and tempo
//!   on snare drum sound in drum kit performance", JASA 138, 2015; Câmara
//!   and Danielsen, "Groove", in The Oxford Handbook of Critical Concepts
//!   in Music Theory, 2019); fills and crashes looser.
//!
//! Spreads between players in an ensemble: Rasch ("Synchronization in
//! performed ensemble music", Acustica 43, 1979) measured onset
//! asynchronies of 30 to 50 ms between the parts of trios; with each
//! player off by about 15 to 20 ms on their own, two of them differ by
//! about that much. Professional drummers are far steadier (a few ms).
//!
//! Everything is deterministic: drawn from the instrument's id and the
//! note's time (and, for jitter, its notehead), so a passage sounds the
//! same each time it plays, and editing one note moves no other.

use crate::{Performance, Style};
use corinzia_model::Id;
use std::collections::BTreeMap;

/// How a player places notes around the beat. Times in milliseconds;
/// spreads are standard deviations.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Feel {
    /// The slow, correlated part (see the module notes).
    pub drift_ms: f64,
    /// The note-to-note part.
    pub jitter_ms: f64,
    /// Constant placement: + behind the beat, − ahead of it.
    pub bias_ms: f64,
    /// A chord's top note this much ahead of the others (keyboards).
    pub melody_lead_ms: f64,
    /// The notes of one chord spread by this much (keyboards, harps).
    pub chord_spread_ms: f64,
}

impl Feel {
    /// Strings: bowed attacks, the widest of the orchestra's.
    pub const STRINGS: Feel = Feel { drift_ms: 14.0, jitter_ms: 8.0, bias_ms: 0.0, melody_lead_ms: 0.0, chord_spread_ms: 0.0 };
    pub const WOODWINDS: Feel = Feel { drift_ms: 12.0, jitter_ms: 7.0, bias_ms: 0.0, melody_lead_ms: 0.0, chord_spread_ms: 0.0 };
    pub const BRASS: Feel = Feel { drift_ms: 12.0, jitter_ms: 7.0, bias_ms: 0.0, melody_lead_ms: 0.0, chord_spread_ms: 0.0 };
    pub const VOICES: Feel = Feel { drift_ms: 16.0, jitter_ms: 10.0, bias_ms: 0.0, melody_lead_ms: 0.0, chord_spread_ms: 0.0 };
    /// Piano, organ and the like: steadier, with melody lead.
    pub const KEYBOARD: Feel = Feel { drift_ms: 9.0, jitter_ms: 5.0, bias_ms: 0.0, melody_lead_ms: 15.0, chord_spread_ms: 4.0 };
    /// Harp, guitar: plucked, a little spread.
    pub const PLUCKED: Feel = Feel { drift_ms: 10.0, jitter_ms: 6.0, bias_ms: 0.0, melody_lead_ms: 0.0, chord_spread_ms: 6.0 };
    /// Timpani and orchestral percussion.
    pub const PERCUSSION: Feel = Feel { drift_ms: 8.0, jitter_ms: 5.0, bias_ms: 0.0, melody_lead_ms: 0.0, chord_spread_ms: 0.0 };
    /// A drum kit's time-keeping (hi-hat, ride): the steadiest of all.
    pub const KIT_TIME: Feel = Feel { drift_ms: 5.0, jitter_ms: 3.0, bias_ms: 0.0, melody_lead_ms: 0.0, chord_spread_ms: 0.0 };
    /// Anything else.
    pub const DEFAULT: Feel = Feel::WOODWINDS;
}

/// A player's feel; a drum kit's differs by sound.
#[derive(Clone, Debug, PartialEq)]
pub enum PlayerFeel {
    One(Feel),
    /// A drum kit: its written keys to the General MIDI sounds they play.
    Kit(BTreeMap<i32, i32>),
}

impl PlayerFeel {
    /// From what the project knows of an instrument: its kind (see
    /// `corinzia_model::catalog`), and its drum sounds (written key ->
    /// General MIDI key). A kit, or three or more sounds, play as a kit; a
    /// single drum as orchestral percussion.
    pub fn for_instrument(kind: Option<&corinzia_model::catalog::Kind>, drums: &[(i32, i32)]) -> PlayerFeel {
        use corinzia_model::catalog::Feel as K;
        let class = kind.map(|k| k.feel);
        if drums.len() >= 3 || class == Some(K::Kit) && !drums.is_empty() {
            return PlayerFeel::Kit(drums.iter().copied().collect());
        }
        PlayerFeel::One(match class {
            Some(K::Strings) => Feel::STRINGS,
            Some(K::Woodwinds) => Feel::WOODWINDS,
            Some(K::Brass) => Feel::BRASS,
            Some(K::Voices) => Feel::VOICES,
            Some(K::Keyboard) => Feel::KEYBOARD,
            Some(K::Plucked) => Feel::PLUCKED,
            Some(K::Percussion | K::Kit) => Feel::PERCUSSION,
            None => Feel::DEFAULT,
        })
    }

    /// The feel of one note (for a kit, by the sound its key plays).
    pub fn feel(&self, key: i32) -> Feel {
        match self {
            PlayerFeel::One(f) => *f,
            PlayerFeel::Kit(map) => kit_feel(map.get(&key).copied().unwrap_or(key)),
        }
    }
}

/// A drum kit by sound: time-keeping tightest and on the beat, the snare a
/// touch behind, the bass drum a touch ahead, toms and cymbal crashes
/// (fills, accents) looser.
pub fn kit_feel(key: i32) -> Feel {
    let t = Feel::KIT_TIME;
    match key {
        // Hi-hat (closed, pedal, open), ride, ride bell.
        42 | 44 | 46 | 51 | 53 | 59 => t,
        // Bass drum.
        35 | 36 => Feel { bias_ms: -2.0, jitter_ms: 3.5, ..t },
        // Snare, side stick, hand clap.
        37..=40 => Feel { bias_ms: 6.0, jitter_ms: 4.0, ..t },
        // Toms.
        41 | 43 | 45 | 47 | 48 | 50 => Feel { drift_ms: 7.0, jitter_ms: 6.0, bias_ms: 3.0, ..t },
        // Crash, splash, china.
        49 | 52 | 55 | 57 => Feel { jitter_ms: 6.0, bias_ms: 2.0, ..t },
        _ => Feel { jitter_ms: 5.0, ..t },
    }
}

/// The feels of a project's instruments.
pub type Feels = BTreeMap<Id, PlayerFeel>;

/// A deterministic hash to a number in [-1, 1).
fn unit(a: u64, b: u64, c: u64) -> f64 {
    // SplitMix64 over the three, folded.
    let mut x = a.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ b.wrapping_add(0xBF58_476D_1CE4_E5B9).rotate_left(17) ^ c.wrapping_mul(0x94D0_49BB_1331_11EB).rotate_left(31);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    (x >> 11) as f64 / (1u64 << 52) as f64 - 1.0
}

/// A normal-ish deviate (the sum of three uniforms: unit variance, and no
/// tails beyond three standard deviations).
fn normal(a: u64, b: u64, c: u64) -> f64 {
    // Each uniform has variance 1/3.
    unit(a, b, c) + unit(a, b ^ 0x51ED, c) + unit(a, b ^ 0xA3F1, c)
}

/// The drift curve: smooth value noise over several time scales, unit
/// variance, at `t` seconds, for the player `seed`.
fn drift(seed: u64, t: f64) -> f64 {
    const SCALES: [f64; 6] = [0.5, 1.0, 2.0, 4.0, 8.0, 16.0];
    let mut sum = 0.0;
    let mut var = 0.0;
    for (k, &s) in SCALES.iter().enumerate() {
        // 1/f^0.8: amplitude grows with the time scale as s^0.4.
        let w = s.powf(0.4);
        let x = t / s;
        let i = x.floor();
        let f = x - i;
        let ease = f * f * (3.0 - 2.0 * f);
        let v0 = unit(seed, k as u64, i as i64 as u64);
        let v1 = unit(seed, k as u64, (i as i64 + 1) as u64);
        // A uniform in [-1, 1) has variance 1/3; the blend less, ~ 2/3 of it.
        sum += w * (v0 + (v1 - v0) * ease);
        var += w * w * (1.0 / 3.0) * (2.0 / 3.0);
    }
    sum / var.sqrt()
}

/// Moves each note off the beat by its player's feel (see the module
/// notes). Only with the phrasing rules on.
pub fn apply(perf: &mut Performance, style: &Style) {
    if !style.phrasing {
        return;
    }
    let tempo = perf.tempo.clone();
    // Each player's chords: notes at one onset, highest first.
    let mut chords: BTreeMap<(u64, i64), Vec<usize>> = BTreeMap::new();
    for (i, n) in perf.notes.iter().enumerate() {
        chords.entry((n.instrument, (n.on_q * 1e6).round() as i64)).or_default().push(i);
    }
    for ((inst, onset), mut idx) in chords {
        idx.sort_by(|a, b| perf.notes[*b].key.cmp(&perf.notes[*a].key));
        let player = style.feels.get(&inst).cloned().unwrap_or(PlayerFeel::One(Feel::DEFAULT));
        let on_sec = tempo.seconds_at(perf.notes[idx[0]].on_q);
        let shared_drift = drift(inst, on_sec);
        let shared_jitter = normal(inst, 1, onset as u64);
        for (k, &i) in idx.iter().enumerate() {
            let n = &perf.notes[i];
            let f = player.feel(n.key);
            let mut ms = f.bias_ms + f.drift_ms * shared_drift + f.jitter_ms * shared_jitter;
            if let PlayerFeel::Kit(_) = player {
                // Each limb on its own: the kit's sounds are separate strokes.
                ms = f.bias_ms + f.drift_ms * shared_drift + f.jitter_ms * normal(inst, 2 + n.key as u64, onset as u64);
            } else if idx.len() > 1 {
                if k == 0 {
                    ms -= f.melody_lead_ms;
                }
                ms += f.chord_spread_ms * normal(inst, 3, n.element);
            }
            // Never before the start.
            let at = tempo.seconds_at(n.on_q) + n.roll_sec;
            perf.notes[i].timing_sec = (ms / 1000.0).max(-at);
        }
    }
}
