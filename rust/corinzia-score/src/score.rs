//! The notation of one movement, as the C++ side serializes it (see
//! src/export/Export.cpp): maps, structure marks and parts with voices of
//! events. Positions and durations are exact whole-note fractions.

use crate::frac::Frac;
use serde::Deserialize;

#[derive(Deserialize, Debug)]
pub struct Score {
    #[serde(default)]
    pub title: String,
    pub length: Frac,
    pub tempo: Vec<Tempo>,
    pub meter: Vec<Meter>,
    pub keys: Vec<Key>,
    #[serde(default)]
    pub barlines: Vec<Barline>,
    #[serde(default)]
    pub endings: Vec<Ending>,
    #[serde(default)]
    pub marks: Vec<Frac>,
    pub parts: Vec<Part>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Tempo {
    pub at: Frac,
    pub bpm: f64,
    #[serde(default)]
    pub text: String,
}

#[derive(Deserialize, Debug, Clone, Copy)]
pub struct Meter {
    pub at: Frac,
    pub num: i64,
    pub den: i64,
}

#[derive(Deserialize, Debug, Clone, Copy)]
pub struct Key {
    pub at: Frac,
    pub fifths: i32,
    #[serde(default)]
    pub minor: bool,
}

#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum BarKind {
    Double,
    Final,
    RepeatStart,
    RepeatEnd,
    RepeatBoth,
}

#[derive(Deserialize, Debug, Clone, Copy)]
pub struct Barline {
    pub at: Frac,
    pub kind: BarKind,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Ending {
    pub from: Frac,
    pub to: Frac,
    pub numbers: Vec<u8>,
}

#[derive(Deserialize, Debug)]
pub struct Part {
    pub name: String,
    #[serde(default)]
    pub abbr: String,
    #[serde(default = "treble")]
    pub clef: String,
    #[serde(default)]
    pub clefs: Vec<ClefChange>,
    #[serde(default)]
    pub texts: Vec<Text>,
    /// Chord symbols above the staff.
    #[serde(default)]
    pub chords: Vec<ChordSymbol>,
    pub voices: Vec<Voice>,
}

/// A chord symbol: written root, quality and bass, e.g. "F♯m7♭5/E".
#[derive(Deserialize, Debug, Clone)]
pub struct ChordSymbol {
    pub at: Frac,
    pub text: String,
    #[serde(default)]
    pub id: u64,
}

/// One syllable of a verse under a note.
#[derive(Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Lyric {
    #[serde(default = "one")]
    pub verse: u8,
    pub text: String,
    /// A hyphen joins it to the next syllable (the word goes on).
    #[serde(default)]
    pub hyphen: bool,
    /// An extender line runs through the following notes (a melisma).
    #[serde(default)]
    pub extend: bool,
}

fn one() -> u8 {
    1
}

fn treble() -> String {
    "treble".into()
}

#[derive(Deserialize, Debug, Clone)]
pub struct ClefChange {
    pub at: Frac,
    pub clef: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Text {
    pub at: Frac,
    pub text: String,
    #[serde(default)]
    pub technique: bool,
}

#[derive(Deserialize, Debug)]
pub struct Voice {
    pub index: u8,
    pub events: Vec<Event>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Event {
    #[serde(default)]
    pub id: u64, // ElementId (rests: the event; notes: see Pitch::id)
    pub start: Frac,
    pub dur: Frac,
    #[serde(default)]
    pub pitches: Vec<Pitch>, // empty = rest
    #[serde(default)]
    pub art: u16,
    #[serde(default, rename = "dyn")]
    pub dynamic: Option<String>, // "pp" .. "fff"
    #[serde(default)]
    pub spans: u8,
    #[serde(default)]
    pub lyrics: Vec<Lyric>,
}

#[derive(Deserialize, Debug, Clone, Copy)]
pub struct Pitch {
    #[serde(default)]
    pub id: u64, // notehead ElementId
    pub step: u8, // 0 = C .. 6 = B
    #[serde(default)]
    pub alter: i8,
    pub octave: i8,
    #[serde(default)]
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

