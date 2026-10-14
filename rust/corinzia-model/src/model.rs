//! Model types. Music is immutable-shared per voice (`Arc<Vec<Event>>`), so
//! snapshots for engraving and playback are cheap, as in the C++ model.

pub use corinzia_score::score::Lyric;
use corinzia_score::Frac;
use std::sync::Arc;

pub type Id = u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Pitch {
    pub step: i8,   // 0 = C .. 6 = B
    pub alter: i8,  // -2..2
    pub octave: i8, // C4 = middle C
}
impl Pitch {
    pub fn midi_key(self) -> i32 {
        const SEMIS: [i32; 7] = [0, 2, 4, 5, 7, 9, 11];
        (self.octave as i32 + 1) * 12 + SEMIS[self.step.clamp(0, 6) as usize] + self.alter as i32
    }
    pub fn diatonic(self) -> i32 {
        self.step as i32 + 7 * self.octave as i32
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Head {
    pub id: Id,
    pub pitch: Pitch,
    pub tie: bool,
    pub velocity: u8, // 0 = from dynamics
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    Note,
    Rest,
    Chord,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub id: Id,
    pub kind: EventKind,
    pub start: Frac,
    pub duration: Frac,
    pub heads: Vec<Head>,
    pub articulations: u16, // corinzia_score::score::art bits
    pub dynamic: Option<u8>, // 0 = ppp .. 7 = fff
    pub spans: u8,           // corinzia_score::score::span bits
    pub lyrics: Vec<Lyric>,  // one syllable per verse
}
impl Event {
    pub fn end(&self) -> Frac {
        self.start + self.duration
    }
    pub fn is_rest(&self) -> bool {
        self.kind == EventKind::Rest || self.heads.is_empty()
    }
}

pub const DYNAMICS: [&str; 8] = ["ppp", "pp", "p", "mp", "mf", "f", "ff", "fff"];

#[derive(Clone, Debug)]
pub struct Voice {
    pub id: Id,
    pub index: u8, // 1..4
    pub events: Arc<Vec<Event>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InstrumentChange {
    pub id: Id,
    pub at: Frac,
    pub instrument: Id,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClefChange {
    pub id: Id,
    pub at: Frac,
    pub clef: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextKind {
    Expression,
    Technique,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextItem {
    pub id: Id,
    pub at: Frac,
    pub text: String,
    pub kind: TextKind,
}

/// A chord symbol on a staff ("Cmaj7", "F♯m7♭5/E").
#[derive(Clone, Debug, PartialEq)]
pub struct ChordSymbol {
    pub id: Id,
    pub at: Frac,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct Lane {
    pub player: Id,
    pub changes: Vec<InstrumentChange>, // sorted; the first at 0
    pub clefs: Vec<ClefChange>,
    pub texts: Vec<TextItem>,
    pub chords: Vec<ChordSymbol>,
    pub voices: Vec<Voice>,
}
impl Lane {
    pub fn instrument_at(&self, at: Frac) -> Option<Id> {
        self.changes.iter().filter(|c| c.at <= at).last().map(|c| c.instrument)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tempo {
    pub at: Frac,
    pub bpm: f64,
    pub ramp: bool,
    pub text: String,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Meter {
    pub at: Frac,
    pub num: i64,
    pub den: i64,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Key {
    pub at: Frac,
    pub fifths: i32,
    pub minor: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarKind {
    Double,
    Final,
    RepeatStart,
    RepeatEnd,
    RepeatBoth,
}
impl BarKind {
    pub const CODES: [&'static str; 5] = ["||", "|.", ".|:", ":|.", ":|.|:"];
    pub fn code(self) -> &'static str {
        Self::CODES[self as usize]
    }
    pub fn from_code(s: &str) -> BarKind {
        match Self::CODES.iter().position(|c| *c == s) {
            Some(1) => BarKind::Final,
            Some(2) => BarKind::RepeatStart,
            Some(3) => BarKind::RepeatEnd,
            Some(4) => BarKind::RepeatBoth,
            _ => BarKind::Double,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Barline {
    pub id: Id,
    pub at: Frac,
    pub kind: BarKind,
}
#[derive(Clone, Debug, PartialEq)]
pub struct RehearsalMark {
    pub id: Id,
    pub at: Frac,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Ending {
    pub id: Id,
    pub from: Frac,
    pub to: Frac,
    pub passes: u8, // bit n-1 = pass n
}

#[derive(Clone, Debug)]
pub struct Movement {
    pub id: Id,
    pub ordinal: i64,
    pub title: String,
    pub length: Frac,
    pub revision: u64,
    pub tempo: Vec<Tempo>,
    pub meter: Vec<Meter>,
    pub keys: Vec<Key>,
    pub barlines: Vec<Barline>,
    pub rehearsal: Vec<RehearsalMark>,
    pub endings: Vec<Ending>,
    pub lanes: Vec<Lane>,
}

#[derive(Clone, Debug)]
pub struct Instrument {
    pub id: Id,
    pub player: Id,
    pub name: String,
    pub engraver_name: String,
    pub transposition: i32,
    pub clef: String,
    pub track: Option<Id>,
}

#[derive(Clone, Debug)]
pub struct Player {
    pub id: Id,
    pub name: String,
    pub instruments: Vec<Id>,
}

#[derive(Clone, Debug)]
pub struct Track {
    pub id: Id,
    pub name: String,
    pub instrument_node: Option<Id>, // CLAP plugin node; none = the built-in instrument
    pub note_port: u16,
    pub output_bus: Option<Id>,
    pub midi_channel: u8,
    pub gain_db: f64,
    pub pan: f64,
    pub mute: bool,
    pub solo: bool,
}

/// An audio-graph node: a CLAP plugin, a bus, or the master output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    Clap,
    Bus,
    Master,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AudioNode {
    pub id: Id,
    pub kind: NodeKind,
    pub name: String,
    pub bundle: String,    // .clap path ("builtin:corinzia-sine" for the built-in instrument)
    pub plugin_id: String, // clap_plugin_descriptor.id
    pub bypassed: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AudioEdge {
    pub from: Id,
    pub from_port: u32,
    pub to: Id,
    pub to_port: u32,
    pub gain: f64,
}

#[derive(Clone, Debug, Default)]
pub struct Project {
    pub title: String,
    pub composer: String,
    pub sample_rate: f64,
    pub next_id: u64,
    pub players: Vec<Player>,
    pub instruments: Vec<Instrument>,
    pub tracks: Vec<Track>,
    pub movements: Vec<Movement>,
    /// The audio graph and the CLAP state chunks of its plugin nodes.
    pub nodes: Vec<AudioNode>,
    pub edges: Vec<AudioEdge>,
    pub plugin_state: std::collections::BTreeMap<Id, Arc<Vec<u8>>>,
}

impl Project {
    pub fn allocate_id(&mut self) -> Id {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
    pub fn player(&self, id: Id) -> Option<&Player> {
        self.players.iter().find(|p| p.id == id)
    }
    pub fn instrument(&self, id: Id) -> Option<&Instrument> {
        self.instruments.iter().find(|i| i.id == id)
    }
    pub fn track(&self, id: Id) -> Option<&Track> {
        self.tracks.iter().find(|t| t.id == id)
    }
    pub fn node(&self, id: Id) -> Option<&AudioNode> {
        self.nodes.iter().find(|n| n.id == id)
    }
}
