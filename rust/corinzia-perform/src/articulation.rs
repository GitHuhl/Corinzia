//! How each instrument articulates: the room players leave between
//! unmarked notes, how short their staccatos are, and how a slurred note
//! moves from the last. A [`Profile`] per instrument, by its kind (as the
//! timing feels are), so a bowed section, a woodwind and a piano each join
//! and separate notes their own way; the performance takes the notes'
//! lengths from it and the playback the slurs' moves.
//!
//! The values follow measured performance: the break and the transitions
//! grow with the time between notes (a slower tempo leaves more room, as
//! key overlap and detachment measured against inter-onset intervals do,
//! and transients are shorter in fast passages), and a slur's move is the
//! hand's or the keys': a string player's finger within reach changes the
//! pitch in a few milliseconds, a shift slides for tens to a couple of
//! hundred; keyed woodwinds change with their keys in a few.

use corinzia_model::catalog::{Feel, Kind};
use corinzia_model::Id;
use std::collections::BTreeMap;

/// A time that grows with the note's written length (seconds):
/// `base + per_second × length`, within `min..=max`, and never more than
/// `share` of the note.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scaled {
    pub base: f64,
    pub per_second: f64,
    pub min: f64,
    pub max: f64,
    pub share: f64,
}

impl Scaled {
    pub fn seconds(&self, length: f64) -> f64 {
        (self.base + self.per_second * length).clamp(self.min, self.max).min(self.share * length.max(0.0))
    }
}

/// A slurred note's move: its time by the note's length, times
/// `leap_base + leap_per_octave × leap / 12` (a wider leap, a longer move).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slide {
    pub time: Scaled,
    pub leap_base: f64,
    pub leap_per_octave: f64,
}

impl Slide {
    fn seconds(&self, leap: f64, length: f64) -> f64 {
        let t = self.time.seconds(length) * (self.leap_base + self.leap_per_octave * leap.abs().min(12.0) / 12.0);
        t.min(self.time.share * length.max(0.0))
    }
}

/// How an instrument articulates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Profile {
    /// The break after an unmarked, unslurred note: a bow change, a
    /// tonguing, a breath of a consonant; not a silence, but never a
    /// sustain through the next.
    pub detached: Scaled,
    /// A staccato's and a staccatissimo's sounding length (seconds): at
    /// most and at least.
    pub staccato: (f64, f64),
    pub staccatissimo: (f64, f64),
    /// A slur's move within reach, and past `reach` semitones (a shift);
    /// `None`: the sound's own.
    pub near: Option<Slide>,
    pub far: Option<(f64, Slide)>,
}

const fn scaled(base: f64, per_second: f64, min: f64, max: f64, share: f64) -> Scaled {
    Scaled { base, per_second, min, max, share }
}

const fn slide(base: f64, per_second: f64, max: f64, leap_base: f64, leap_per_octave: f64) -> Slide {
    Slide { time: scaled(base, per_second, 0.0, max, 0.3), leap_base, leap_per_octave }
}

impl Profile {
    /// Instruments of no known kind (and the performance without a
    /// project): a tongued wind's articulation.
    pub const DEFAULT: Profile = Profile::WOODWINDS;

    /// Bowed strings: the bow changes, 50 ms on a quick note to 150 ms on a
    /// long one; a finger moves in a few ms, a shift (past a fifth) slides.
    pub const STRINGS: Profile = Profile {
        detached: scaled(0.05, 0.12, 0.05, 0.15, 0.25),
        staccato: crate::STACCATO_SECONDS,
        staccatissimo: crate::STACCATISSIMO_SECONDS,
        near: Some(slide(0.003, 0.008, 0.012, 1.0, 0.0)),
        far: Some((7.0, slide(0.03, 0.12, 0.2, 0.7, 0.3))),
    };

    /// Woodwinds: tongued apart; slurred, the keys change the note at once.
    pub const WOODWINDS: Profile = Profile {
        detached: scaled(0.04, 0.1, 0.04, 0.15, 0.25),
        staccato: crate::STACCATO_SECONDS,
        staccatissimo: crate::STACCATISSIMO_SECONDS,
        near: Some(slide(0.003, 0.005, 0.008, 1.0, 0.0)),
        far: None,
    };

    /// Brass: tongued apart; slurred, the lips move a little longer, wider
    /// leaps longer still.
    pub const BRASS: Profile = Profile {
        detached: scaled(0.05, 0.1, 0.04, 0.16, 0.25),
        staccato: crate::STACCATO_SECONDS,
        staccatissimo: crate::STACCATISSIMO_SECONDS,
        near: Some(slide(0.01, 0.025, 0.04, 0.6, 1.0)),
        far: None,
    };

    /// Voices: a consonant's break; slurred, the voice carries the pitch.
    pub const VOICES: Profile = Profile {
        detached: scaled(0.04, 0.08, 0.04, 0.12, 0.2),
        staccato: crate::STACCATO_SECONDS,
        staccatissimo: crate::STACCATISSIMO_SECONDS,
        near: Some(slide(0.02, 0.05, 0.08, 1.0, 0.0)),
        far: None,
    };

    /// Keyboards: the key lifts before the next goes down.
    pub const KEYBOARD: Profile = Profile {
        detached: scaled(0.02, 0.1, 0.02, 0.15, 0.25),
        staccato: crate::STACCATO_SECONDS,
        staccatissimo: crate::STACCATISSIMO_SECONDS,
        near: None,
        far: None,
    };

    /// Plucked and struck: they ring and are damped; a short lift.
    pub const STRUCK: Profile = Profile {
        detached: scaled(0.02, 0.05, 0.02, 0.06, 0.2),
        staccato: crate::STACCATO_SECONDS,
        staccatissimo: crate::STACCATISSIMO_SECONDS,
        near: None,
        far: None,
    };

    pub fn for_instrument(kind: Option<&Kind>) -> Profile {
        match kind.map(|k| k.feel) {
            Some(Feel::Strings) => Profile::STRINGS,
            Some(Feel::Woodwinds) => Profile::WOODWINDS,
            Some(Feel::Brass) => Profile::BRASS,
            Some(Feel::Voices) => Profile::VOICES,
            Some(Feel::Keyboard) => Profile::KEYBOARD,
            Some(Feel::Plucked | Feel::Percussion | Feel::Kit) => Profile::STRUCK,
            None => Profile::DEFAULT,
        }
    }

    /// The share of an unmarked note's written length (seconds) that sounds.
    pub fn detached_gate(&self, length: f64) -> f64 {
        if length <= 0.0 {
            return 1.0;
        }
        1.0 - self.detached.seconds(length) / length
    }

    /// How long a slurred note's move from the last takes (seconds), by the
    /// leap (semitones) and the note's written length; `None`: the sound's
    /// own.
    pub fn slide_seconds(&self, leap: f64, length: f64) -> Option<f64> {
        match self.far {
            Some((reach, far)) if leap.abs() > reach => Some(far.seconds(leap, length)),
            _ => self.near.map(|s| s.seconds(leap, length)),
        }
    }
}

/// Each instrument's profile; one missing articulates as [`Profile::DEFAULT`].
pub type Profiles = BTreeMap<Id, Profile>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_notes_are_parted_more_as_they_are_longer() {
        let s = Profile::STRINGS;
        // Four quarters at 120: each parted from the next, not a sustain.
        let quarter = s.detached_gate(0.5);
        assert!(quarter < 0.8 && quarter > 0.7, "{quarter}");
        // Slower: a longer break, but a smaller share.
        assert!(s.detached.seconds(1.0) > s.detached.seconds(0.5));
        assert!(s.detached_gate(1.0) > quarter);
        // Quick notes: a quarter of the note at most.
        assert!(s.detached_gate(0.1) >= 0.75 - 1e-9);
        // A piano's lift is shorter than a bow change.
        assert!(Profile::KEYBOARD.detached.seconds(0.5) < s.detached.seconds(0.5));
    }

    #[test]
    fn slurs_move_as_hands_and_keys_do_and_by_the_tempo() {
        let s = Profile::STRINGS;
        assert!(s.slide_seconds(2.0, 0.5).unwrap() < 0.012, "a finger");
        assert!(s.slide_seconds(12.0, 0.5).unwrap() > 0.06, "a shift slides");
        assert!(s.slide_seconds(12.0, 0.15).unwrap() < 0.6 * s.slide_seconds(12.0, 1.0).unwrap(), "quicker when quick");
        assert!(s.slide_seconds(12.0, 0.1).unwrap() <= 0.03 + 1e-9, "never more than a part of the note");
        assert!(Profile::WOODWINDS.slide_seconds(7.0, 1.0).unwrap() <= 0.008, "keys, not a slide");
        assert!(Profile::BRASS.slide_seconds(12.0, 1.0).unwrap() > Profile::BRASS.slide_seconds(2.0, 1.0).unwrap());
        assert_eq!(Profile::KEYBOARD.slide_seconds(2.0, 1.0), None);
    }
}
