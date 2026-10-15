//! The notation of one movement, as the engraver and the exporters read it
//! (built from the model by `corinzia_model::notation::score`): maps,
//! structure marks and parts with voices of events. Positions and durations
//! are exact whole-note fractions.

use crate::frac::Frac;

#[derive(Debug)]
pub struct Score {
    /// The movement's title.
    pub title: String,
    /// The work's title and composer (the page header).
    pub work_title: String,
    pub composer: String,
    /// A part's instrument, printed top left of the first page.
    pub instrument: String,
    pub length: Frac,
    pub tempo: Vec<Tempo>,
    pub meter: Vec<Meter>,
    pub keys: Vec<Key>,
    pub barlines: Vec<Barline>,
    pub endings: Vec<Ending>,
    pub marks: Vec<Frac>,
    /// Segno, coda, Fine and the D.C./D.S. jumps.
    pub navigation: Vec<NavMark>,
    pub parts: Vec<Part>,
}

/// A navigation mark: the segno and coda signs at the start of their bar;
/// "To Coda", Fine and the jumps at the end of theirs (`at` is that
/// barline).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Segno,
    Coda,
    ToCoda,
    Fine,
    /// D.C. / D.S., then `al`: play to the end, to Fine, or to "To Coda"
    /// and on at the coda.
    DaCapo(NavTo),
    DalSegno(NavTo),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavTo {
    End,
    Fine,
    Coda,
}

impl Nav {
    /// The words written for it (the signs have none).
    pub fn text(self) -> &'static str {
        match self {
            Nav::Segno | Nav::Coda => "",
            Nav::ToCoda => "To Coda",
            Nav::Fine => "Fine",
            Nav::DaCapo(NavTo::End) => "D.C.",
            Nav::DaCapo(NavTo::Fine) => "D.C. al Fine",
            Nav::DaCapo(NavTo::Coda) => "D.C. al Coda",
            Nav::DalSegno(NavTo::End) => "D.S.",
            Nav::DalSegno(NavTo::Fine) => "D.S. al Fine",
            Nav::DalSegno(NavTo::Coda) => "D.S. al Coda",
        }
    }
    /// A short code for files: "segno", "coda", "tocoda", "fine", "dc",
    /// "dc-fine", "dc-coda", "ds", "ds-fine", "ds-coda".
    pub fn code(self) -> &'static str {
        match self {
            Nav::Segno => "segno",
            Nav::Coda => "coda",
            Nav::ToCoda => "tocoda",
            Nav::Fine => "fine",
            Nav::DaCapo(NavTo::End) => "dc",
            Nav::DaCapo(NavTo::Fine) => "dc-fine",
            Nav::DaCapo(NavTo::Coda) => "dc-coda",
            Nav::DalSegno(NavTo::End) => "ds",
            Nav::DalSegno(NavTo::Fine) => "ds-fine",
            Nav::DalSegno(NavTo::Coda) => "ds-coda",
        }
    }
    pub fn from_code(c: &str) -> Option<Nav> {
        [
            Nav::Segno,
            Nav::Coda,
            Nav::ToCoda,
            Nav::Fine,
            Nav::DaCapo(NavTo::End),
            Nav::DaCapo(NavTo::Fine),
            Nav::DaCapo(NavTo::Coda),
            Nav::DalSegno(NavTo::End),
            Nav::DalSegno(NavTo::Fine),
            Nav::DalSegno(NavTo::Coda),
        ]
        .into_iter()
        .find(|n| n.code() == c)
    }
    /// Marks at the start of a bar (the signs); the rest end one.
    pub fn at_bar_start(self) -> bool {
        matches!(self, Nav::Segno | Nav::Coda)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NavMark {
    pub id: u64,
    pub at: Frac,
    pub kind: Nav,
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
    /// An upbeat (anacrusis): this bar is num/den long, written with the
    /// next meter's signature and not counted.
    pub pickup: bool,
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
    /// Staves next to each other with the same non-zero group share a
    /// bracket (an instrument family) and barlines.
    pub group: u32,
    /// Octave lines and pedal marks on this staff.
    pub lines: Vec<LineSpan>,
    /// The instrument's transposition as (diatonic steps, semitones),
    /// written minus sounding: (1, 2) for a B♭ clarinet.
    pub transposition: (i32, i32),
}

/// A line over `[from, to)` of a staff.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineSpan {
    pub id: u64,
    pub from: Frac,
    pub to: Frac,
    pub kind: LineKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// Notes written this many octaves from where they sound: 1 = 8va
    /// (written an octave lower), -1 = 8vb, 2 = 15ma, -2 = 15mb.
    Octave(i8),
    /// The sustain pedal held.
    Pedal,
}

impl LineKind {
    pub fn code(self) -> &'static str {
        match self {
            LineKind::Octave(1) => "8va",
            LineKind::Octave(-1) => "8vb",
            LineKind::Octave(2) => "15ma",
            LineKind::Octave(-2) => "15mb",
            LineKind::Octave(_) => "8va",
            LineKind::Pedal => "ped",
        }
    }
    pub fn from_code(c: &str) -> Option<LineKind> {
        Some(match c {
            "8va" => LineKind::Octave(1),
            "8vb" => LineKind::Octave(-1),
            "15ma" => LineKind::Octave(2),
            "15mb" => LineKind::Octave(-2),
            "ped" => LineKind::Pedal,
            _ => return None,
        })
    }
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
    /// Tuplets: `actual` in the time of `normal` over `[start, start + length)`.
    pub tuplets: Vec<Tuplet>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tuplet {
    pub start: Frac,
    pub length: Frac,
    pub actual: i64,
    pub normal: i64,
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
    /// Grace notes before it.
    pub graces: Vec<Grace>,
    /// Fingers, bottom head to top (0 for none).
    pub fingering: Vec<u8>,
    /// The side of the slur starting here, if chosen by hand.
    pub slur: SlurShape,
}

/// A slur's side chosen by hand, kept on its first note (0 automatic,
/// 1 above, -1 below). Its shape is always the engraver's.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SlurShape {
    pub dir: i8,
}
impl SlurShape {
    pub fn is_default(&self) -> bool {
        *self == SlurShape::default()
    }
}

#[derive(Debug, Clone)]
pub struct Grace {
    pub id: u64,
    pub pitches: Vec<Pitch>,
    pub base: u8,
    pub slash: bool,
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
    pub const UP_BOW: u16 = 1 << 13;
    pub const DOWN_BOW: u16 = 1 << 14;
}
pub mod span {
    pub const SLUR_START: u8 = 1 << 0;
    pub const SLUR_END: u8 = 1 << 1;
    pub const CRESC_START: u8 = 1 << 2;
    pub const DIM_START: u8 = 1 << 3;
    pub const HAIRPIN_END: u8 = 1 << 4;
}

