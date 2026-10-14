//! The notation of one movement, as the engraver and the exporters read it
//! (built from the model by `corinzia_model::notation::score`): maps,
//! structure marks and parts with voices of events. Positions and durations
//! are exact whole-note fractions.

use crate::frac::Frac;

#[derive(Debug)]
pub struct Score {
    pub title: String,
    pub length: Frac,
    pub tempo: Vec<Tempo>,
    pub meter: Vec<Meter>,
    pub keys: Vec<Key>,
    pub barlines: Vec<Barline>,
    pub endings: Vec<Ending>,
    pub marks: Vec<Frac>,
    pub parts: Vec<Part>,
}

#[derive(Debug, Clone)]
pub struct Tempo {
    pub at: Frac,
    pub bpm: f64,
    pub text: String,
}

#[derive(Debug, Clone, Copy)]
pub struct Meter {
    pub at: Frac,
    pub num: i64,
    pub den: i64,
}

#[derive(Debug, Clone, Copy)]
pub struct Key {
    pub at: Frac,
    pub fifths: i32,
    pub minor: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarKind {
    Double,
    Final,
    RepeatStart,
    RepeatEnd,
    RepeatBoth,
}

#[derive(Debug, Clone, Copy)]
pub struct Barline {
    pub at: Frac,
    pub kind: BarKind,
}

#[derive(Debug, Clone)]
pub struct Ending {
    pub from: Frac,
    pub to: Frac,
    pub numbers: Vec<u8>,
}

#[derive(Debug)]
pub struct Part {
    pub name: String,
    pub abbr: String,
    pub clef: String,
    pub clefs: Vec<ClefChange>,
    pub texts: Vec<Text>,
    /// Chord symbols above the staff.
    pub chords: Vec<ChordSymbol>,
    pub voices: Vec<Voice>,
    /// This staff and the next are one instrument's (a piano's): joined by
    /// a brace and barlines, named once.
    pub brace: bool,
    /// The staff's key differs from the score's by this many fifths (a
    /// transposing instrument in a transposed score).
    pub key_shift: i32,
}

/// A chord symbol: written root, quality and bass, e.g. "F♯m7♭5/E".
#[derive(Debug, Clone)]
pub struct ChordSymbol {
    pub at: Frac,
    pub text: String,
    pub id: u64,
}

/// One syllable of a verse under a note.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Lyric {
    pub verse: u8,
    pub text: String,
    /// A hyphen joins it to the next syllable (the word goes on).
    pub hyphen: bool,
    /// An extender line runs through the following notes (a melisma).
    pub extend: bool,
}

#[derive(Debug, Clone)]
pub struct ClefChange {
    pub at: Frac,
    pub clef: String,
}

#[derive(Debug, Clone)]
pub struct Text {
    pub at: Frac,
    pub text: String,
    pub technique: bool,
}

#[derive(Debug)]
pub struct Voice {
    pub index: u8,
    pub events: Vec<Event>,
}

#[derive(Debug, Clone)]
pub struct Event {
    pub id: u64, // ElementId (rests: the event; notes: see Pitch::id)
    pub start: Frac,
    pub dur: Frac,
    pub pitches: Vec<Pitch>, // empty = rest
    pub art: u16,
    pub dynamic: Option<String>, // "pp" .. "fff"
    pub spans: u8,
    pub lyrics: Vec<Lyric>,
}

#[derive(Debug, Clone, Copy)]
pub struct Pitch {
    pub id: u64, // notehead ElementId
    pub step: u8, // 0 = C .. 6 = B
    pub alter: i8,
    pub octave: i8,
    pub tie: bool,
}

// Bits shared with corinzia::model (Articulation, SpanMark).
pub mod art {
    pub const STACCATO: u16 = 1 << 0;
    pub const ACCENT: u16 = 1 << 1;
    pub const TENUTO: u16 = 1 << 2;
    pub const MARCATO: u16 = 1 << 3;
    pub const FERMATA: u16 = 1 << 4;
    pub const STACCATISSIMO: u16 = 1 << 5;
    pub const TRILL: u16 = 1 << 6;
    pub const MORDENT: u16 = 1 << 7;
    pub const TURN: u16 = 1 << 8;
    pub const TREMOLO: u16 = 1 << 9;
    pub const ARPEGGIO: u16 = 1 << 10;
    pub const BREATH: u16 = 1 << 11;
    pub const CAESURA: u16 = 1 << 12;
}
pub mod span {
    pub const SLUR_START: u8 = 1 << 0;
    pub const SLUR_END: u8 = 1 << 1;
    pub const CRESC_START: u8 = 1 << 2;
    pub const DIM_START: u8 = 1 << 3;
    pub const HAIRPIN_END: u8 = 1 << 4;
}

