//! Model types. Music is immutable-shared per voice (`Arc<Vec<Event>>`), so
//! snapshots for undo, engraving and playback are cheap.

pub use corinzia_score::score::{LineKind, LineSpan, Lyric, Nav, NavMark, NavTo, Slur};
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

/// An interval: diatonic steps and semitones (a major second up = 1, 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Interval {
    pub steps: i32,
    pub semitones: i32,
}
impl Interval {
    pub const fn new(steps: i32, semitones: i32) -> Self {
        Interval { steps, semitones }
    }
    pub fn is_zero(self) -> bool {
        self.steps == 0 && self.semitones == 0
    }
    pub fn inverse(self) -> Self {
        Interval { steps: -self.steps, semitones: -self.semitones }
    }
    /// How far along the circle of fifths it moves a key (M2 up: +2).
    pub fn fifths(self) -> i32 {
        // C transposed by the interval, as a position on the line of fifths.
        let p = Pitch { step: 0, alter: 0, octave: 4 }.transposed(self);
        const LINE: [i32; 7] = [0, 2, 4, -1, 1, 3, 5];
        LINE[p.step as usize] + 7 * p.alter as i32
    }
}

/// A key moved along the circle of fifths, kept within seven accidentals
/// (enharmonically: C# major up a M2 is E-flat major, not D# major).
pub fn shift_key(fifths: i32, by: i32) -> i32 {
    let mut f = fifths + by;
    while f > 7 {
        f -= 12;
    }
    while f < -7 {
        f += 12;
    }
    f
}

impl Pitch {
    /// The pitch moved by an interval, spelt diatonically (C up a M2 is D,
    /// E up a M2 is F#).
    pub fn transposed(self, by: Interval) -> Pitch {
        const SEMIS: [i32; 7] = [0, 2, 4, 5, 7, 9, 11];
        let dia = self.step as i32 + 7 * self.octave as i32 + by.steps;
        let (step, octave) = (dia.rem_euclid(7), dia.div_euclid(7));
        let midi = self.midi_key() + by.semitones;
        let natural = (octave + 1) * 12 + SEMIS[step as usize];
        Pitch { step: step as i8, alter: (midi - natural).clamp(-2, 2) as i8, octave: octave as i8 }
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
    pub dynamic: Option<u8>, // an index into DYNAMICS
    pub spans: u8,           // corinzia_score::score::span bits
    pub lyrics: Vec<Lyric>,  // one syllable per verse
    /// Grace notes before it, in order; they take no time of their own.
    pub graces: Vec<Grace>,
    /// Fingers, bottom head to top (0 for none).
    pub fingering: Vec<u8>,
}

/// A grace note (or chord): written small before its event.
#[derive(Clone, Debug, PartialEq)]
pub struct Grace {
    pub id: Id,
    pub heads: Vec<Head>,
    /// Written value: 8 = eighth, 16 = sixteenth.
    pub base: u8,
    /// Acciaccatura: a slashed stem, played as short as possible.
    pub slash: bool,
}
impl Event {
    pub fn end(&self) -> Frac {
        self.start + self.duration
    }
    pub fn is_rest(&self) -> bool {
        self.kind == EventKind::Rest || self.heads.is_empty()
    }
}

/// Dynamic marks by index: the eight levels ppp..fff first, then the rest
/// of MusicXML's (and LilyPond's) set.
pub const DYNAMICS: [&str; 21] = [
    "ppp", "pp", "p", "mp", "mf", "f", "ff", "fff", "pppp", "ffff", "fp", "sf", "sfz", "sffz", "sfp", "sfpp", "fz", "rf", "rfz", "pf", "n",
];

/// The dynamic named `text` ("mf", "sfz"...).
pub fn dynamic_index(text: &str) -> Option<u8> {
    DYNAMICS.iter().position(|d| *d == text).map(|i| i as u8)
}

/// How a dynamic plays, in levels 0 = ppp .. 7 = fff, given the level
/// before it: (the note it is written on, the music after it). Sforzandi
/// accent one note; fp and its kin drop straight back.
pub fn dynamic_levels(mark: u8, before: u8) -> (u8, u8) {
    match DYNAMICS.get(mark as usize).copied().unwrap_or("mf") {
        "pppp" | "n" => (0, 0),
        "ffff" => (7, 7),
        "fp" => (5, 2),
        "sfp" => (6, 2),
        "sfpp" => (6, 1),
        "sffz" => (7, before),
        "sf" | "sfz" | "fz" | "rf" | "rfz" => ((before + 2).clamp(5, 7), before),
        "pf" => (2, 5),
        _ => (mark.min(7), mark.min(7)),
    }
}

#[derive(Clone, Debug)]
pub struct Voice {
    pub id: Id,
    pub index: u8, // 1..4
    pub events: Arc<Vec<Event>>,
    /// Tuplet brackets, sorted by start; an enclosing one comes first.
    pub tuplets: Vec<Tuplet>,
}

/// `actual` notes in the time of `normal` over `[start, start + length)`
/// (5:4, 3:2, 2:3). The events inside keep their sounding durations; they
/// are written `actual / normal` times longer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tuplet {
    pub start: Frac,
    pub length: Frac,
    pub actual: i64,
    pub normal: i64,
}
impl Tuplet {
    pub fn end(&self) -> Frac {
        self.start + self.length
    }
    /// Sounding time to written time.
    pub fn ratio(&self) -> Frac {
        Frac::new(self.actual, self.normal)
    }
    /// The written value of one of its `actual` notes.
    pub fn unit(&self) -> Frac {
        self.length * Frac::new(1, self.normal)
    }
    pub fn contains(&self, t: Frac) -> bool {
        t >= self.start && t < self.end()
    }
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
    /// Octave lines and pedal (sorted by start).
    pub lines: Vec<LineSpan>,
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
    /// An upbeat (anacrusis): this bar is num/den long, written with the
    /// next meter's signature and not counted.
    pub pickup: bool,
    /// Beam groups in units of 1/den ("2+2+3" in 7/8), zeros after; all
    /// zero: the meter's default.
    pub beams: [u8; 8],
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
    /// Segno, coda, Fine, D.C./D.S. (sorted by position).
    pub navigation: Vec<NavMark>,
    pub lanes: Vec<Lane>,
    /// Slurs between notes (event ids), in any voices or staves of a player.
    pub slurs: Vec<Slur>,
    /// System and page breaks by hand, at bar starts.
    pub breaks: Vec<Break>,
}

/// A system (or page) break before the bar at `at`: in the full score, or
/// in one player's part.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Break {
    pub id: Id,
    pub at: Frac,
    pub page: bool,
    /// The player whose part it is in; None: the full score.
    pub part: Option<Id>,
}

/// How a score (or the parts) is laid out on the page. Lengths in
/// millimetres, distances in staff spaces.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    /// Portrait page size; `landscape` turns it.
    pub page_width_mm: f64,
    pub page_height_mm: f64,
    pub landscape: bool,
    /// Staff height, bottom line to top line (four staff spaces).
    pub staff_mm: f64,
    /// Margins top and bottom, and at the sides.
    pub margin_y_mm: f64,
    pub margin_x_mm: f64,
    /// Note spacing, 1.0 as LilyPond's.
    pub spacing: f64,
    /// Least distance from staff to staff (top line to top line).
    pub staff_distance: f64,
    /// Space between systems (between their outermost marks).
    pub system_gap: f64,
    /// Bars on each system; 0 to let the line breaker choose.
    pub bars_per_system: u32,
    pub page_numbers: bool,
}

impl Default for Layout {
    /// A4 with an 18 pt staff, as LilyPond's default.
    fn default() -> Self {
        Layout { page_width_mm: 210.0, page_height_mm: 297.0, landscape: false, staff_mm: 6.35, margin_y_mm: 12.7, margin_x_mm: 12.7, spacing: 1.0, staff_distance: 9.0, system_gap: 4.0, bars_per_system: 0, page_numbers: true }
    }
}

impl Layout {
    /// The parts' default: a larger staff (7 mm), as parts are read from a stand.
    pub fn parts() -> Layout {
        Layout { staff_mm: 7.0, ..Layout::default() }
    }
    /// One staff space in millimetres.
    pub fn space_mm(&self) -> f64 {
        self.staff_mm / 4.0
    }
    /// The page in staff spaces (width, height), turned if landscape.
    pub fn page_spaces(&self) -> (f64, f64) {
        let (w, h) = if self.landscape { (self.page_height_mm, self.page_width_mm) } else { (self.page_width_mm, self.page_height_mm) };
        (w / self.space_mm(), h / self.space_mm())
    }
}

/// Named page sizes in millimetres (portrait).
pub const PAGE_SIZES: [(&str, f64, f64); 7] = [("A4", 210.0, 297.0), ("Letter", 215.9, 279.4), ("A3", 297.0, 420.0), ("B4", 250.0, 353.0), ("Legal", 215.9, 355.6), ("Tabloid", 279.4, 431.8), ("A5", 148.0, 210.0)];

impl Movement {
    /// Each note's (lane, voice index into `lane.voices`, start), by event id.
    pub fn note_places(&self) -> std::collections::HashMap<Id, (usize, usize, Frac)> {
        let mut out = std::collections::HashMap::new();
        for (li, l) in self.lanes.iter().enumerate() {
            for (vi, v) in l.voices.iter().enumerate() {
                for e in v.events.iter().filter(|e| !e.is_rest()) {
                    out.insert(e.id, (li, vi, e.start));
                }
            }
        }
        out
    }

    /// Drops the slurs whose notes are gone, that no longer go forwards,
    /// or that join two players. True if any went.
    pub fn tidy_slurs(&mut self) -> bool {
        if self.slurs.is_empty() {
            return false;
        }
        let places = self.note_places();
        let n = self.slurs.len();
        let lanes = &self.lanes;
        self.slurs.retain(|s| match (places.get(&s.from), places.get(&s.to)) {
            (Some(a), Some(b)) => a.2 < b.2 && lanes[a.0].player == lanes[b.0].player,
            _ => false,
        });
        self.slurs.len() != n
    }
}

#[derive(Clone, Debug)]
pub struct Instrument {
    pub id: Id,
    pub player: Id,
    pub name: String,
    pub engraver_name: String,
    /// Written minus sounding (a B-flat clarinet: a major second up).
    pub transposition: Interval,
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
    pub instrument_node: Option<Id>, // plugin node (CLAP or VST3); none = the built-in instrument
    pub note_port: u16,
    pub output_bus: Option<Id>,
    pub midi_channel: u8,
    pub gain_db: f64,
    pub pan: f64,
    pub mute: bool,
    pub solo: bool,
}

/// An audio-graph node: a plugin, a bus, or the master output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    /// A CLAP or VST3 plugin (the bundle's extension says which); stored
    /// as "clap".
    Plugin,
    Bus,
    Master,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AudioNode {
    pub id: Id,
    pub kind: NodeKind,
    pub name: String,
    pub bundle: String,    // .clap or .vst3 path ("builtin:corinzia-sine" for the built-in instrument)
    pub plugin_id: String, // clap_plugin_descriptor.id, or the VST3 class id in hex
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

#[derive(Clone, Debug)]
pub struct Project {
    pub title: String,
    pub composer: String,
    /// Under the title; lyricist top left, arranger under the composer;
    /// the copyright at the foot of the first page.
    pub subtitle: String,
    pub lyricist: String,
    pub arranger: String,
    pub copyright: String,
    /// The full score's layout and the parts'.
    pub layout: Layout,
    pub part_layout: Layout,
    pub sample_rate: f64,
    pub next_id: u64,
    pub players: Vec<Player>,
    pub instruments: Vec<Instrument>,
    pub tracks: Vec<Track>,
    pub movements: Vec<Movement>,
    /// The audio graph and the state chunks of its plugin nodes.
    pub nodes: Vec<AudioNode>,
    pub edges: Vec<AudioEdge>,
    pub plugin_state: std::collections::BTreeMap<Id, Arc<Vec<u8>>>,
}

impl Default for Project {
    fn default() -> Self {
        Project {
            title: String::new(),
            composer: String::new(),
            subtitle: String::new(),
            lyricist: String::new(),
            arranger: String::new(),
            copyright: String::new(),
            layout: Layout::default(),
            part_layout: Layout::parts(),
            sample_rate: 48000.0,
            next_id: 1,
            players: vec![],
            instruments: vec![],
            tracks: vec![],
            movements: vec![],
            nodes: vec![],
            edges: vec![],
            plugin_state: Default::default(),
        }
    }
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
