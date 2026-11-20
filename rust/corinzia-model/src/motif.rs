//! The motif bank: musical material taken from the score and described, so
//! that it can be found and reused (by the Explore mode's accompaniment
//! and composition tools, and by hand).
//!
//! A [`Motif`] keeps its notes (relative to its start) with the context it
//! came from: movement, staff, bars, instrument, key, meter and tempo. The
//! user chooses the passage and says what it is: its
//! [`roles`](Motif::roles) (melody, bass, call, response...), free
//! [`tags`](Motif::tags) (character, mood, style) and notes.
//!
//! [`analyse`] adds the facts that can be read off the notes: range and
//! voice range, register, texture, contour, intervals, rhythm and density,
//! scale degrees and how it starts and ends, repetition, articulation and
//! dynamics.

use crate::model::{Event, Id, Movement, Pitch, Project};
use corinzia_score::score::art;
use corinzia_score::Frac;
use serde::{Deserialize, Serialize};

/// A time in whole notes, as numerator and denominator (saved as such).
pub type Time = (i64, i64);

fn time(f: Frac) -> Time {
    (f.num, f.den)
}
fn quarters(t: Time) -> f64 {
    t.0 as f64 * 4.0 / t.1.max(1) as f64
}

/// One note of a motif.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MotifNote {
    /// From the motif's start, and its length (whole notes).
    pub start: Time,
    pub duration: Time,
    /// The written spelling (step 0-6 from C, alter, octave) at concert pitch.
    pub step: i8,
    pub alter: i8,
    pub octave: i8,
    /// The voice it was in (1-4).
    pub voice: u8,
    /// Its articulations (`corinzia_score::score::art` bits).
    pub arts: u16,
    /// Tied to the next note of the same pitch.
    pub tie: bool,
    /// The dynamic in force (0 = ppp .. 7 = fff), if one was written on it.
    pub dynamic: Option<u8>,
}

impl MotifNote {
    pub fn pitch(&self) -> Pitch {
        Pitch { step: self.step, alter: self.alter, octave: self.octave }
    }
    pub fn key(&self) -> i32 {
        self.pitch().midi_key()
    }
    pub fn start_q(&self) -> f64 {
        quarters(self.start)
    }
    pub fn end_q(&self) -> f64 {
        quarters(self.start) + quarters(self.duration)
    }
}

/// Where a motif was taken from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MotifSource {
    pub movement: Id,
    /// The staff (lane index in the movement) and its player.
    pub lane: usize,
    pub player: Id,
    /// The instrument then, and its kind (see `catalog`).
    pub instrument: Option<Id>,
    pub kind: Option<String>,
    /// The passage (whole notes from the movement's start).
    pub from: Time,
    pub to: Time,
    /// The bars, as numbered in the score.
    pub bars: (i64, i64),
}

/// What part a motif plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    Melody,
    Countermelody,
    Bass,
    InnerVoice,
    Pad,
    Accompaniment,
    Arpeggio,
    Ostinato,
    Riff,
    Hook,
    Call,
    Response,
    Fill,
    Groove,
    Pedal,
    Intro,
    Cadence,
    Transition,
}

impl Role {
    pub const ALL: [Role; 18] = [
        Role::Melody,
        Role::Countermelody,
        Role::Bass,
        Role::InnerVoice,
        Role::Pad,
        Role::Accompaniment,
        Role::Arpeggio,
        Role::Ostinato,
        Role::Riff,
        Role::Hook,
        Role::Call,
        Role::Response,
        Role::Fill,
        Role::Groove,
        Role::Pedal,
        Role::Intro,
        Role::Cadence,
        Role::Transition,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Role::Melody => "Melody",
            Role::Countermelody => "Countermelody",
            Role::Bass => "Bass",
            Role::InnerVoice => "Inner voice",
            Role::Pad => "Pad",
            Role::Accompaniment => "Accompaniment",
            Role::Arpeggio => "Arpeggio",
            Role::Ostinato => "Ostinato",
            Role::Riff => "Riff",
            Role::Hook => "Hook",
            Role::Call => "Call",
            Role::Response => "Response",
            Role::Fill => "Fill",
            Role::Groove => "Groove",
            Role::Pedal => "Pedal point",
            Role::Intro => "Intro",
            Role::Cadence => "Cadence",
            Role::Transition => "Transition",
        }
    }
    /// From a name typed or saved ("inner voice", "Pedal point", "call").
    pub fn parse(s: &str) -> Option<Role> {
        let s = s.trim().to_lowercase().replace(['-', '_'], " ");
        Role::ALL.into_iter().find(|r| r.name().to_lowercase() == s || r.name().to_lowercase().split(' ').next() == Some(s.as_str()))
    }
}

/// A motif in the bank.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Motif {
    pub id: Id,
    pub name: String,
    pub notes: Vec<MotifNote>,
    /// Its length (whole notes): the passage it was taken from.
    pub length: Time,
    /// The meter, key (fifths, minor) and tempo it was in.
    pub meter: (i64, i64),
    pub fifths: i32,
    pub minor: bool,
    pub bpm: f64,
    pub source: Option<MotifSource>,
    /// What part it plays, as the user says.
    pub roles: Vec<Role>,
    /// Free words: character, mood, style ("lyrical", "dark", "march").
    pub tags: Vec<String>,
    /// What the user says about it, in their own words.
    #[serde(default)]
    pub comment: String,
}

impl Motif {
    pub fn length_q(&self) -> f64 {
        quarters(self.length)
    }
}

// --------------------------------------------------------------- analysis --

/// Where a motif sits among the voice types' ranges.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceRange {
    Soprano,
    MezzoSoprano,
    Alto,
    Tenor,
    Baritone,
    Bass,
}

impl VoiceRange {
    /// The usual choral ranges (MIDI keys): soprano C4-A5, mezzo A3-F5, alto
    /// F3-D5, tenor C3-A4, baritone A2-F4, bass E2-E4.
    pub const RANGES: [(VoiceRange, i32, i32); 6] = [
        (VoiceRange::Soprano, 60, 81),
        (VoiceRange::MezzoSoprano, 57, 77),
        (VoiceRange::Alto, 53, 74),
        (VoiceRange::Tenor, 48, 69),
        (VoiceRange::Baritone, 45, 65),
        (VoiceRange::Bass, 40, 64),
    ];
    pub fn name(self) -> &'static str {
        match self {
            VoiceRange::Soprano => "Soprano",
            VoiceRange::MezzoSoprano => "Mezzo-soprano",
            VoiceRange::Alto => "Alto",
            VoiceRange::Tenor => "Tenor",
            VoiceRange::Baritone => "Baritone",
            VoiceRange::Bass => "Bass",
        }
    }
}

/// The overall shape of a line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Static,
    Ascending,
    Descending,
    Arch,
    InvertedArch,
    Wave,
}

impl Shape {
    pub fn name(self) -> &'static str {
        match self {
            Shape::Static => "static",
            Shape::Ascending => "ascending",
            Shape::Descending => "descending",
            Shape::Arch => "arch",
            Shape::InvertedArch => "inverted arch",
            Shape::Wave => "wave",
        }
    }
}

/// How the notes sound together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Texture {
    /// One note at a time.
    Line,
    /// Chords throughout.
    Chordal,
    /// Lines and chords, or several voices.
    Mixed,
}

/// What [`analyse`] finds.
#[derive(Clone, Debug, PartialEq)]
pub struct Analysis {
    pub notes: usize,
    pub length_q: f64,
    /// Range (MIDI keys), its span in semitones, the median.
    pub lowest: i32,
    pub highest: i32,
    pub ambitus: i32,
    pub median: f64,
    /// The voice type whose range holds it most comfortably, if one does.
    pub voice_range: Option<VoiceRange>,
    /// "low", "middle", "high", "very low", "very high".
    pub register: &'static str,
    pub texture: Texture,
    /// Parsons code of the top line ("*udrd": up, down, repeat).
    pub contour: String,
    pub shape: Shape,
    /// Of the moves between successive top-line notes: steps (1-2
    /// semitones), leaps (3+), repeats; the largest leap.
    pub steps: f64,
    pub leaps: f64,
    pub repeats: f64,
    pub largest_leap: i32,
    /// Notes per quarter, the shortest and longest note (quarters), the
    /// share of time that sounds, syncopation (share of onsets off the
    /// beat that are held across it), dotted rhythms.
    pub density: f64,
    pub shortest: f64,
    pub longest: f64,
    pub sounding: f64,
    pub syncopation: f64,
    pub dotted: bool,
    /// The onsets on a sixteenth grid ("x..x..x.x..."), at most four bars.
    pub rhythm: String,
    /// Pitch classes used (0 = C), and the scale degrees of the top line in
    /// the key ("1", "♭3", "♯4"...), the first and last.
    pub pitch_classes: Vec<i32>,
    pub degrees: Vec<String>,
    pub first_degree: String,
    pub last_degree: String,
    /// Notes outside the key.
    pub chromatic: usize,
    /// Ends on the tonic (closed) or elsewhere (open: a call, a half cadence).
    pub closed: bool,
    /// The second half repeats the first (transposed or not): an ostinato or
    /// sequence.
    pub repeats_itself: bool,
    /// Articulation: share of notes staccato (or staccatissimo), accented,
    /// slurred is not stored; the loudest and softest dynamic written.
    pub staccato: f64,
    pub accents: f64,
    pub dynamics: Option<(u8, u8)>,
}

/// The key's tonic pitch class from its fifths (and mode).
pub fn tonic(fifths: i32, minor: bool) -> i32 {
    ((fifths * 7).rem_euclid(12) + if minor { 9 } else { 0 }) % 12
}

/// A pitch class as a scale degree of the key ("1", "♭3", "♯4").
pub fn degree(pc: i32, fifths: i32, minor: bool) -> String {
    let t = tonic(fifths, minor);
    let iv = (pc - t).rem_euclid(12);
    let major = ["1", "♭2", "2", "♭3", "3", "4", "♯4", "5", "♭6", "6", "♭7", "7"];
    let minor_names = ["1", "♭2", "2", "3", "♮3", "4", "♯4", "5", "6", "♮6", "7", "♮7"];
    if minor { minor_names[iv as usize] } else { major[iv as usize] }.to_string()
}

fn in_key(pc: i32, fifths: i32, minor: bool) -> bool {
    let t = tonic(fifths, minor);
    let iv = (pc - t).rem_euclid(12);
    // Natural minor plus the raised 6th and 7th (melodic and harmonic minor).
    if minor { [0, 2, 3, 5, 7, 8, 9, 10, 11].contains(&iv) } else { [0, 2, 4, 5, 7, 9, 11].contains(&iv) }
}

/// Describes a motif's notes (see [`Analysis`]).
pub fn analyse(m: &Motif) -> Analysis {
    let mut notes: Vec<&MotifNote> = m.notes.iter().collect();
    notes.sort_by(|a, b| a.start_q().total_cmp(&b.start_q()).then(b.key().cmp(&a.key())));
    let len_q = m.length_q().max(notes.iter().map(|n| n.end_q()).fold(0.0, f64::max)).max(1e-9);
    let keys: Vec<i32> = notes.iter().map(|n| n.key()).collect();
    let (lowest, highest) = (keys.iter().copied().min().unwrap_or(60), keys.iter().copied().max().unwrap_or(60));
    let median = {
        let mut k = keys.clone();
        k.sort();
        if k.is_empty() { 60.0 } else if k.len() % 2 == 1 { k[k.len() / 2] as f64 } else { (k[k.len() / 2 - 1] + k[k.len() / 2]) as f64 / 2.0 }
    };
    // The voice type whose range holds it, the nearest centre first.
    let voice_range = VoiceRange::RANGES
        .iter()
        .filter(|(_, lo, hi)| lowest >= *lo && highest <= *hi)
        .min_by(|a, b| ((a.1 + a.2) as f64 / 2.0 - median).abs().total_cmp(&((b.1 + b.2) as f64 / 2.0 - median).abs()))
        .map(|r| r.0);
    let register = match median {
        x if x < 43.0 => "very low",
        x if x < 55.0 => "low",
        x if x < 70.0 => "middle",
        x if x < 84.0 => "high",
        _ => "very high",
    };
    // Onsets: chords are notes starting together.
    let mut onsets: Vec<f64> = notes.iter().map(|n| n.start_q()).collect();
    onsets.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    let chordal = onsets.iter().filter(|&&t| notes.iter().filter(|n| (n.start_q() - t).abs() < 1e-6).count() > 1).count();
    let overlapping = notes.windows(2).any(|w| w[1].start_q() < w[0].end_q() - 1e-6 && (w[1].start_q() - w[0].start_q()).abs() > 1e-6);
    let texture = if chordal == 0 && !overlapping {
        Texture::Line
    } else if chordal * 4 >= onsets.len() * 3 {
        Texture::Chordal
    } else {
        Texture::Mixed
    };
    // The top line: the highest note at each onset.
    let top: Vec<&MotifNote> = onsets.iter().filter_map(|&t| notes.iter().filter(|n| (n.start_q() - t).abs() < 1e-6).max_by_key(|n| n.key()).copied()).collect();
    let moves: Vec<i32> = top.windows(2).map(|w| w[1].key() - w[0].key()).collect();
    let contour: String = std::iter::once('*')
        .chain(moves.iter().map(|d| match d.signum() {
            1 => 'u',
            -1 => 'd',
            _ => 'r',
        }))
        .collect();
    let n_moves = moves.len().max(1) as f64;
    let steps = moves.iter().filter(|d| (1..=2).contains(&d.abs())).count() as f64 / n_moves;
    let leaps = moves.iter().filter(|d| d.abs() >= 3).count() as f64 / n_moves;
    let repeats = moves.iter().filter(|d| **d == 0).count() as f64 / n_moves;
    let largest_leap = moves.iter().map(|d| d.abs()).max().unwrap_or(0);
    let shape = {
        let tk: Vec<i32> = top.iter().map(|n| n.key()).collect();
        if tk.len() < 2 || tk.iter().max() == tk.iter().min() {
            Shape::Static
        } else {
            let (first, last) = (tk[0], tk[tk.len() - 1]);
            let (hi, lo) = (*tk.iter().max().unwrap(), *tk.iter().min().unwrap());
            let hi_at = tk.iter().position(|k| *k == hi).unwrap() as f64 / (tk.len() - 1) as f64;
            let lo_at = tk.iter().position(|k| *k == lo).unwrap() as f64 / (tk.len() - 1) as f64;
            let turns = moves.iter().filter(|d| **d != 0).collect::<Vec<_>>().windows(2).filter(|w| w[0].signum() != w[1].signum()).count();
            if hi > first.max(last) && (0.15..=0.85).contains(&hi_at) && turns <= 3 {
                Shape::Arch
            } else if lo < first.min(last) && (0.15..=0.85).contains(&lo_at) && turns <= 3 {
                Shape::InvertedArch
            } else if turns >= 4 {
                Shape::Wave
            } else if last > first {
                Shape::Ascending
            } else if last < first {
                Shape::Descending
            } else {
                Shape::Wave
            }
        }
    };
    // Rhythm.
    let beat_q = 4.0 / m.meter.1.max(1) as f64 * if m.meter.1 == 8 && m.meter.0 % 3 == 0 { 3.0 } else { 1.0 };
    let durations: Vec<f64> = notes.iter().map(|n| n.end_q() - n.start_q()).collect();
    let shortest = durations.iter().copied().fold(f64::INFINITY, f64::min);
    let longest = durations.iter().copied().fold(0.0, f64::max);
    let sounding = {
        // The union of the notes' spans.
        let mut spans: Vec<(f64, f64)> = notes.iter().map(|n| (n.start_q(), n.end_q())).collect();
        spans.sort_by(|a, b| a.0.total_cmp(&b.0));
        let (mut total, mut cur): (f64, Option<(f64, f64)>) = (0.0, None);
        for (a, b) in spans {
            cur = match cur {
                Some((x, y)) if a <= y => Some((x, y.max(b))),
                Some((x, y)) => {
                    total += y - x;
                    Some((a, b))
                }
                None => Some((a, b)),
            };
        }
        if let Some((x, y)) = cur {
            total += y - x;
        }
        (total / len_q).min(1.0)
    };
    let off_beat: Vec<&f64> = onsets.iter().filter(|t| ((*t / beat_q) - (*t / beat_q).round()).abs() > 1e-6).collect();
    let syncopated = off_beat.iter().filter(|&&&t| notes.iter().any(|n| (n.start_q() - t).abs() < 1e-6 && n.end_q() > ((t / beat_q).floor() + 1.0) * beat_q + 1e-6)).count();
    let syncopation = syncopated as f64 / onsets.len().max(1) as f64;
    let dotted = durations.iter().any(|d| {
        let x = d / 1.5;
        x > 0.0 && (x.log2() - x.log2().round()).abs() < 1e-6
    });
    let rhythm: String = (0..((len_q * 4.0).round() as usize).min(64)).map(|i| if onsets.iter().any(|t| (t * 4.0 - i as f64).abs() < 1e-6) { 'x' } else { '.' }).collect();
    // Pitch.
    let mut pcs: Vec<i32> = keys.iter().map(|k| k.rem_euclid(12)).collect();
    pcs.sort();
    pcs.dedup();
    let degrees: Vec<String> = top.iter().map(|n| degree(n.key().rem_euclid(12), m.fifths, m.minor)).collect();
    let chromatic = keys.iter().filter(|k| !in_key(k.rem_euclid(12), m.fifths, m.minor)).count();
    let last_pc = notes.iter().filter(|n| (n.end_q() - notes.iter().map(|x| x.end_q()).fold(0.0, f64::max)).abs() < 1e-6).map(|n| n.key()).min().map(|k| k.rem_euclid(12));
    let closed = last_pc == Some(tonic(m.fifths, m.minor));
    // Internal repetition: the second half's intervals and rhythm repeat the first's.
    let repeats_itself = {
        let half = len_q / 2.0;
        let a: Vec<(f64, f64, i32)> = notes.iter().filter(|n| n.start_q() < half - 1e-6).map(|n| (n.start_q(), n.end_q() - n.start_q(), n.key())).collect();
        let b: Vec<(f64, f64, i32)> = notes.iter().filter(|n| n.start_q() >= half - 1e-6).map(|n| (n.start_q() - half, n.end_q() - n.start_q(), n.key())).collect();
        a.len() >= 2
            && a.len() == b.len()
            && a.iter().zip(&b).all(|(x, y)| (x.0 - y.0).abs() < 1e-6 && (x.1 - y.1).abs() < 1e-6)
            && a.iter().zip(&b).map(|(x, y)| y.2 - x.2).collect::<std::collections::BTreeSet<_>>().len() == 1
    };
    let count = notes.len().max(1) as f64;
    let staccato = notes.iter().filter(|n| n.arts & (art::STACCATO | art::STACCATISSIMO) != 0).count() as f64 / count;
    let accents = notes.iter().filter(|n| n.arts & (art::ACCENT | art::MARCATO) != 0).count() as f64 / count;
    let dyn_levels: Vec<u8> = notes.iter().filter_map(|n| n.dynamic).collect();
    let dynamics = (!dyn_levels.is_empty()).then(|| (*dyn_levels.iter().min().unwrap(), *dyn_levels.iter().max().unwrap()));
    Analysis {
        notes: notes.len(),
        length_q: len_q,
        lowest,
        highest,
        ambitus: highest - lowest,
        median,
        voice_range,
        register,
        texture,
        contour,
        shape,
        steps,
        leaps,
        repeats,
        largest_leap,
        density: onsets.len() as f64 / len_q,
        shortest: if shortest.is_finite() { shortest } else { 0.0 },
        longest,
        sounding,
        syncopation,
        dotted,
        rhythm,
        pitch_classes: pcs,
        first_degree: degrees.first().cloned().unwrap_or_default(),
        last_degree: degrees.last().cloned().unwrap_or_default(),
        degrees,
        chromatic,
        closed,
        repeats_itself,
        staccato,
        accents,
        dynamics,
    }
}

/// A motif from the notes of one staff over `[from, to)` in a movement
/// (bars `bars` as numbered in the score), with its context; the user then
/// names it and says what it is. None when the passage holds no notes.
pub fn capture(project: &Project, movement: &Movement, lane: usize, from: Frac, to: Frac, bars: (i64, i64), id: Id) -> Option<Motif> {
    let l = movement.lanes.get(lane)?;
    let mut notes = Vec::new();
    for v in &l.voices {
        for e in v.events.iter().filter(|e: &&Event| e.start >= from && e.start < to && !e.heads.is_empty()) {
            let end = (e.start + e.duration).min(to);
            for h in &e.heads {
                notes.push(MotifNote {
                    start: time(e.start - from),
                    duration: time(end - e.start),
                    step: h.pitch.step,
                    alter: h.pitch.alter,
                    octave: h.pitch.octave,
                    voice: v.index,
                    arts: e.articulations,
                    tie: h.tie && e.start + e.duration <= to,
                    dynamic: e.dynamic,
                });
            }
        }
    }
    if notes.is_empty() {
        return None;
    }
    notes.sort_by(|a, b| quarters(a.start).total_cmp(&quarters(b.start)).then(b.key().cmp(&a.key())));
    let meter = movement.meter.iter().rfind(|x| x.at <= from).or(movement.meter.first()).map_or((4, 4), |x| (x.num, x.den));
    let key = movement.keys.iter().rfind(|k| k.at <= from).or(movement.keys.first());
    let bpm = movement.tempo.iter().rfind(|t| t.at <= from).or(movement.tempo.first()).map_or(120.0, |t| t.bpm);
    let instrument = l.instrument_at(from);
    let inst = instrument.and_then(|i| project.instrument(i));
    let name = format!("{} {}", inst.map_or("Staff", |i| i.name.as_str()), if bars.0 == bars.1 { format!("bar {}", bars.0) } else { format!("bars {}–{}", bars.0, bars.1) });
    let m = Motif {
        id,
        name,
        notes,
        length: time(to - from),
        meter,
        fifths: key.map_or(0, |k| k.fifths),
        minor: key.is_some_and(|k| k.minor),
        bpm,
        source: Some(MotifSource { movement: movement.id, lane, player: l.player, instrument, kind: inst.and_then(|i| i.kind).map(|k| k.id.to_string()), from: time(from), to: time(to), bars }),
        roles: vec![],
        tags: vec![],
        comment: String::new(),
    };
    Some(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(start: (i64, i64), dur: (i64, i64), step: i8, octave: i8) -> MotifNote {
        MotifNote { start, duration: dur, step, alter: 0, octave, voice: 1, arts: 0, tie: false, dynamic: None }
    }
    fn motif(notes: Vec<MotifNote>, length: Time) -> Motif {
        Motif { id: 1, name: "m".into(), notes, length, meter: (4, 4), fifths: 0, minor: false, bpm: 120.0, source: None, roles: vec![], tags: vec![], comment: String::new() }
    }

    #[test]
    fn a_rising_and_falling_tune_is_described() {
        // C D E F G F E D | C1 in C major: an arch, stepwise, closed, soprano-ish.
        let steps = [0, 1, 2, 3, 4, 3, 2, 1];
        let mut ns: Vec<MotifNote> = steps.iter().enumerate().map(|(i, s)| note((i as i64, 8), (1, 8), *s, 5)).collect();
        ns.push(note((1, 1), (1, 1), 0, 5));
        let a = analyse(&motif(ns, (2, 1)));
        assert_eq!((a.lowest, a.highest, a.ambitus), (72, 79, 7));
        assert_eq!(a.texture, Texture::Line);
        assert_eq!(a.shape, Shape::Arch);
        assert_eq!(a.contour, "*uuuudddd");
        assert!(a.steps > 0.99 && a.leaps == 0.0);
        assert_eq!((a.first_degree.as_str(), a.last_degree.as_str(), a.closed), ("1", "1", true));
        assert_eq!(a.voice_range, Some(VoiceRange::Soprano));
        assert_eq!(a.chromatic, 0);
        assert_eq!(&a.rhythm[..8], "x.x.x.x.");
    }

    #[test]
    fn a_low_repeated_figure_repeats_itself() {
        // A2 E3 A2 E3 | A2 E3 A2 E3: low, a bass voice's, repeating.
        let ns: Vec<MotifNote> = (0..8).map(|i| if i % 2 == 0 { note((i, 8), (1, 8), 5, 2) } else { note((i, 8), (1, 8), 2, 3) }).collect();
        let mut m = motif(ns, (1, 1));
        m.minor = true;
        let a = analyse(&m);
        assert!(a.repeats_itself && a.register == "low", "{a:?}");
        assert_eq!(a.voice_range, Some(VoiceRange::Bass));
        assert_eq!((a.first_degree.as_str(), a.last_degree.as_str(), a.closed), ("1", "5", false));
    }

    #[test]
    fn open_endings_and_chords() {
        // C D E G (ends on the 5th): open.
        let ns = vec![note((0, 4), (1, 4), 0, 5), note((1, 4), (1, 4), 1, 5), note((2, 4), (1, 4), 2, 5), note((3, 4), (1, 4), 4, 5)];
        let a = analyse(&motif(ns, (1, 1)));
        assert!(!a.closed);
        // Two bars of held triads: chordal, sustained.
        let mut ch = Vec::new();
        for bar in 0..2 {
            for (s, o) in [(0, 4), (2, 4), (4, 4)] {
                ch.push(note((bar, 1), (1, 1), s, o));
            }
        }
        let a = analyse(&motif(ch, (2, 1)));
        assert_eq!(a.texture, Texture::Chordal);
        assert_eq!((a.longest, a.sounding), (4.0, 1.0));
    }

    #[test]
    fn roles_read_and_round_trip() {
        assert_eq!(Role::parse("inner voice"), Some(Role::InnerVoice));
        assert_eq!(Role::parse("Pedal"), Some(Role::Pedal));
        assert_eq!(Role::parse("call"), Some(Role::Call));
        let m = motif(vec![note((0, 1), (1, 4), 0, 4)], (1, 4));
        let back: Motif = serde_json::from_value(serde_json::to_value(&m).unwrap()).unwrap();
        assert_eq!(back, m);
    }
}
