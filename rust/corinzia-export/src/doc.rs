//! The export document: what the C++ side hands over as JSON (one movement).
//! `score` is the notation (for MusicXML), `performance` the played notes
//! with repeats, ornaments and dynamics already applied (for MIDI).

pub use corinzia_score::score::*;
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
