//! The notes of the scale and the genera, measured in moria.
//!
//! Corinzia follows the Patriarchal Music Commission of 1881: the octave
//! has 72 moria, and each genus is a set of tetrachords. Pitches are
//! measured from the middle Ni (Ni = 0, the next Ni up = 72).
//!
//! * Diatonic: 12, 10, 8 (Ni Pa Vou Ga), then 12 (Ga Di), then 12, 10, 8.
//! * Soft chromatic (second mode), from Di: 8, 14, 8, then 12.
//! * Hard chromatic (plagal second), from Pa: 6, 20, 4, then 12.
//!
//! Pa and Di stand where they do in the diatonic scale in every genus
//! here, so a change of genus keeps them in place.

/// One of the seven notes, in rising order from Ni.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Degree {
    Ni,
    Pa,
    Vou,
    Ga,
    Di,
    Ke,
    Zo,
}

impl Degree {
    pub const ALL: [Degree; 7] = [Degree::Ni, Degree::Pa, Degree::Vou, Degree::Ga, Degree::Di, Degree::Ke, Degree::Zo];

    /// 0 for Ni … 6 for Zo.
    pub fn index(self) -> i32 {
        self as i32
    }

    pub fn from_index(i: i32) -> Degree {
        Degree::ALL[i.rem_euclid(7) as usize]
    }

    /// "Ni", "Pa", …
    pub fn name(self) -> &'static str {
        ["Ni", "Pa", "Vou", "Ga", "Di", "Ke", "Zo"][self as usize]
    }

    /// The Greek name: "Νη", "Πα", …
    pub fn greek(self) -> &'static str {
        ["Νη", "Πα", "Βου", "Γα", "Δι", "Κε", "Ζω"][self as usize]
    }
}

/// A note in a register: `octave` 0 runs from the middle Ni up to Zo,
/// 1 from the high Ni (Ni΄) up, -1 from the low Ni down to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Note {
    pub octave: i8,
    pub degree: Degree,
}

impl Note {
    pub const fn new(degree: Degree, octave: i8) -> Note {
        Note { octave, degree }
    }

    /// Steps along the scale: 0 = the middle Ni, 7 = the high Ni.
    pub fn steps(self) -> i32 {
        self.octave as i32 * 7 + self.degree.index()
    }

    pub fn from_steps(s: i32) -> Note {
        Note { octave: s.div_euclid(7) as i8, degree: Degree::from_index(s) }
    }

    /// The note `n` steps up (down when negative).
    pub fn step(self, n: i32) -> Note {
        Note::from_steps(self.steps() + n)
    }

    /// "Pa", "Ni΄" (an octave up), "Ke͵" (an octave down).
    pub fn label(self) -> String {
        let mark = match self.octave {
            0 => String::new(),
            o if o > 0 => "΄".repeat(o as usize),
            o => "͵".repeat(o.unsigned_abs() as usize),
        };
        format!("{}{mark}", self.degree.name())
    }
}

/// A genus: the intervals the scale is sung in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Genus {
    Diatonic,
    SoftChromatic,
    HardChromatic,
}

pub const OCTAVE: i32 = 72;

impl Genus {
    /// Where each degree stands within the octave, in moria above Ni.
    pub fn positions(self) -> [i32; 7] {
        match self {
            //                       Ni  Pa  Vou Ga  Di  Ke  Zo
            Genus::Diatonic => [0, 12, 22, 30, 42, 54, 64],
            Genus::SoftChromatic => [0, 12, 20, 34, 42, 50, 64],
            Genus::HardChromatic => [8, 12, 18, 38, 42, 54, 60],
        }
    }

    /// The pitch of a note in moria above the middle Ni.
    pub fn moria(self, n: Note) -> i32 {
        n.octave as i32 * OCTAVE + self.positions()[n.degree as usize]
    }

    /// The seven intervals, in moria, from Ni up to the next Ni.
    pub fn intervals(self) -> [i32; 7] {
        let p = self.positions();
        std::array::from_fn(|i| if i == 6 { OCTAVE + p[0] - p[6] } else { p[i + 1] - p[i] })
    }
}

/// Where the middle Ni sounds, and how it maps to MIDI.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reference {
    /// The middle Ni as a MIDI key with a fraction (60.0 = C4).
    pub ni_midi: f64,
}

impl Default for Reference {
    /// Ni on D4, a common pitch for chanting with a male choir.
    fn default() -> Reference {
        Reference { ni_midi: 62.0 }
    }
}

impl Reference {
    /// A pitch in moria above the middle Ni, as a MIDI key with a fraction
    /// (one equal-tempered semitone is 6 moria).
    pub fn midi(self, moria: i32) -> f64 {
        self.ni_midi + moria as f64 / 6.0
    }

    /// The nearest MIDI key and the cents to bend it by.
    pub fn key_and_cents(self, moria: i32) -> (i32, f64) {
        let m = self.midi(moria);
        let key = m.round();
        (key as i32, (m - key) * 100.0)
    }
}
