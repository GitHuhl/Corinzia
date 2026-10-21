//! The export document of one movement: `score` is the notation (for
//! MusicXML), `performance` the played notes with repeats, ornaments and
//! dynamics already applied (for MIDI). Built by `corinzia_perform::document`.

pub use corinzia_score::score::*;

#[derive(Debug)]
pub struct Document {
    pub title: String,
    pub composer: String,
    pub score: Score,
    pub performance: Performance,
}

#[derive(Debug)]
pub struct Performance {
    /// Tempo points over performed quarters (first at 0).
    pub tempo: Vec<PerfTempo>,
    pub meters: Vec<PerfMeter>,
    pub length_q: f64,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, Copy)]
pub struct PerfTempo {
    pub q: f64,
    pub bpm: f64,
    pub ramp: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct PerfMeter {
    pub q: f64,
    pub num: i64,
    pub den: i64,
}

#[derive(Debug)]
pub struct Track {
    pub name: String,
    pub channel: u8,
    /// The MIDI program (and bank) to select first.
    pub program: Option<(u8, Option<u16>)>,
    /// Controller changes: (performed quarter, controller, value).
    pub controls: Vec<(f64, u8, u8)>,
    /// Program changes along the way (expression maps): (performed quarter, program).
    pub programs: Vec<(f64, u8)>,
    pub notes: Vec<Note>,
}

#[derive(Debug, Clone, Copy)]
pub struct Note {
    pub on: f64,  // performed quarters
    pub off: f64,
    pub key: u8,
    pub vel: f32, // 0..1
}
