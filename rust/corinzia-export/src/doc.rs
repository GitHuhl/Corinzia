//! The export document: what the C++ side hands over as JSON (one movement).
//! `score` is the notation (for MusicXML), `performance` the played notes
//! with repeats, ornaments and dynamics already applied (for MIDI).

use crate::frac::Frac;
use serde::Deserialize;

#[derive(Deserialize, Debug)]
pub struct Document {
    pub version: u32,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub composer: String,
    pub score: Score,
    pub performance: Performance,
}

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
    pub voices: Vec<Voice>,
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
}

#[derive(Deserialize, Debug, Clone, Copy)]
pub struct Pitch {
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

#[derive(Deserialize, Debug)]
pub struct Performance {
    /// Tempo points over performed quarters (first at 0).
    pub tempo: Vec<PerfTempo>,
    pub meters: Vec<PerfMeter>,
    pub length_q: f64,
    pub tracks: Vec<Track>,
}

#[derive(Deserialize, Debug, Clone, Copy)]
pub struct PerfTempo {
    pub q: f64,
    pub bpm: f64,
    #[serde(default)]
    pub ramp: bool,
}

#[derive(Deserialize, Debug, Clone, Copy)]
pub struct PerfMeter {
    pub q: f64,
    pub num: i64,
    pub den: i64,
}

#[derive(Deserialize, Debug)]
pub struct Track {
    pub name: String,
    #[serde(default)]
    pub channel: u8,
    pub notes: Vec<Note>,
}

#[derive(Deserialize, Debug, Clone, Copy)]
pub struct Note {
    pub on: f64,  // performed quarters
    pub off: f64,
    pub key: u8,
    pub vel: f32, // 0..1
}
