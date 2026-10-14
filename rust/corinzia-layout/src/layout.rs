//! Single-line engraving: the whole movement on one endless system (the
//! editor's Panorama view). Page breaking builds on this later.
//!
//! Pipeline: split events at barlines into notatable pieces -> per measure,
//! rhythmic columns shared by all staves -> x positions (logarithmic spacing,
//! widened where marks need room) -> marks into a display list.

use crate::display::{pt, text_width, Column, DisplayList, Item, StaffGeom};
use crate::skyline::Skyline;
use crate::font::{Font, Glyph};
use corinzia_score::score::{art, BarKind, Key, Part, Pitch, Score};
use corinzia_score::values::{decompose, NoteValue};
use corinzia_score::Frac;

#[derive(Clone, Debug)]
pub struct Options {
    /// Least distance between adjacent staves (top line to top line), in
    /// staff spaces: LilyPond's `basic-distance` of `staff-staff-spacing`.
    pub staff_distance: f32,
    pub margin: f32,
    /// Instrument names left of the system.
    pub names: bool,
}
impl Default for Options {
    fn default() -> Self {
        Options { staff_distance: 9.0, margin: 1.0, names: true }
    }
}

const NAME_SIZE: f32 = 2.0;
const TEXT_SIZE: f32 = 2.0;
const TEMPO_SIZE: f32 = 2.2;
const LYRIC_SIZE: f32 = 2.0;
const CHORD_SIZE: f32 = 2.3;
/// Space between two words of a verse; room for a hyphen inside a word.
const WORD_GAP: f32 = 0.9;
const HYPHEN_GAP: f32 = 2.0;
const CLEF_CHANGE_SCALE: f32 = 0.8;

// ------------------------------------------------------------- pitch & clef --
fn diatonic(p: &Pitch) -> i32 {
    p.step as i32 + 7 * p.octave as i32
}

/// Diatonic step (step + 7 * octave) on the clef's middle line.
pub fn clef_middle(clef: &str) -> i32 {
    match clef {
        "bass" => 1 + 7 * 3,
        "bass_8" => 1 + 7 * 2,
        "alto" | "percussion" => 7 * 4,
        "tenor" => 5 + 7 * 3,
        "treble_8" => 6 + 7 * 3,
        _ => 6 + 7 * 4,
    }
}

fn clef_glyph(clef: &str) -> (Glyph, f32) {
    // (glyph, y of its origin below the top line)
    match clef {
        "bass" => (Glyph::FClef, 1.0),
        "bass_8" => (Glyph::FClef8vb, 1.0),
        "alto" => (Glyph::CClef, 2.0),
        "tenor" => (Glyph::CClef, 1.0),
        "treble_8" => (Glyph::GClef8vb, 3.0),
        "percussion" => (Glyph::PercussionClef, 2.0),
        _ => (Glyph::GClef, 3.0),
    }
}

/// Alteration the key signature gives a step (0 = C .. 6 = B).
fn key_alter(fifths: i32, step: u8) -> i8 {
    const SHARPS: [u8; 7] = [3, 0, 4, 1, 5, 2, 6]; // F C G D A E B
    const FLATS: [u8; 7] = [6, 2, 5, 1, 4, 0, 3]; // B E A D G C F
    if fifths > 0 && SHARPS[..fifths.min(7) as usize].contains(&step) {
        1
    } else if fifths < 0 && FLATS[..(-fifths).min(7) as usize].contains(&step) {
        -1
    } else {
        0
    }
}

/// Staff positions (half-spaces above the middle line) of key-signature
/// accidentals in this clef.
fn key_positions(fifths: i32, clef: &str) -> Vec<(i32, Glyph)> {
    const SHARPS_TREBLE: [i32; 7] = [4, 1, 5, 2, -1, 3, 0];
    const FLATS_TREBLE: [i32; 7] = [0, 3, -1, 2, -2, 1, -3];
    let mut shift = (clef_middle("treble") - clef_middle(clef)).rem_euclid(7);
    if shift > 3 {
        shift -= 7;
    }
    let (list, g) = if fifths >= 0 { (SHARPS_TREBLE, Glyph::Sharp) } else { (FLATS_TREBLE, Glyph::Flat) };
    list.iter().take(fifths.unsigned_abs().min(7) as usize).map(|p| (p + shift, g)).collect()
}

fn accidental_glyph(alter: i8) -> Glyph {
    match alter {
        -2 => Glyph::DoubleFlat,
        -1 => Glyph::Flat,
        1 => Glyph::Sharp,
        2 => Glyph::DoubleSharp,
        _ => Glyph::Natural,
    }
}

// Horizontal spacing follows LilyPond's documented model and defaults
// (SpacingSpanner): the movement's common shortest note gets
// `shortest-duration-space` increments, and each doubling of duration adds
// one `spacing-increment`.
const SHORTEST_DURATION_SPACE: f64 = 2.0;
const SPACING_INCREMENT: f64 = 1.2;
/// `base-shortest-duration`: the reference never exceeds a dotted eighth.
const BASE_SHORTEST: Frac = Frac { num: 3, den: 16 };

/// Ideal space after a note of duration `d` when `shortest` is the
/// movement's common shortest duration.
fn ideal_space(d: Frac, shortest: Frac) -> f32 {
    let r = (d.num as f64 / d.den as f64) / (shortest.num as f64 / shortest.den as f64);
    (SPACING_INCREMENT * (SHORTEST_DURATION_SPACE + r.log2())) as f32
}

/// Fixed distances around barlines and between columns, measured from
/// LilyPond's output (BarLine `space-alist`, NoteSpacing, separation rods).
const BAR_TO_NOTE: f32 = 0.995; // barline centre to the next notehead
const BAR_TO_ACCIDENTAL: f32 = 0.695; // barline centre to an accidental
const BAR_HALF_WIDTH: f32 = 0.095;
/// Before a barline an up-stem gains, and a down-stem loses, this much per
/// stem length inside the staff (unless it has a flag); after a barline a
/// down-stem gains `STEM_AFTER_BAR` per stem length.
const STEM_BEFORE_BAR: f32 = 0.25;
const STEM_AFTER_BAR: f32 = 0.185;
/// Notehead widths of LilyPond's font, used for spacing so that bars come
/// out as wide as LilyPond's (the heads are drawn in Bravura).
const SPACING_HEAD_WIDTHS: [(Glyph, f32); 3] = [(Glyph::NoteheadBlack, 1.304), (Glyph::NoteheadHalf, 1.377), (Glyph::NoteheadWhole, 1.962)];
/// The stretchable half of the barline-to-note distance (LilyPond's
/// `semi-fixed-space`).
const BAR_TO_NOTE_STRETCH: f32 = 0.45;
/// Each line's fixed cost in the breaker, against the square of its force:
/// LilyPond prefers fewer, slightly compressed lines to more stretched ones.
const LINE_PENALTY: f32 = 0.2;
/// How far a line may be compressed, as a force: each spring gives up at
/// most this share of its stretchability.
const MAX_COMPRESSION: f32 = -0.9;
/// A bar holding a single note column gets this much more before it.
const LONE_NOTE_SPACE: f32 = 1.0;
/// Least room between a column's right side (heads, dots, flags) and the
/// next one's left side: an accidental, or else the heads.
const ROD_PADDING_ACCIDENTAL: f32 = 0.6;
const ROD_PADDING: f32 = 0.45;
/// NoteSpacing's optical corrections: facing stems (up, then down) get room
/// in proportion to their vertical overlap, and lose it the other way round;
/// stems in the same direction gain (rising) or lose (falling) a fixed
/// amount for steps of a third or more.
const STEM_SPACING_CORRECTION: f32 = 0.5;
const SAME_DIRECTION_CORRECTION: f32 = 0.25;
const STEM_LENGTH: f32 = 3.5;
/// How far from the head's centre a stem's overlap starts counting.
const STEM_ATTACH: f32 = 0.193;

/// The stretchability left of `x`, from the (x, stretchability) points of
/// a line, interpolated between them.
fn stretch_at(breaks: &[(f32, f32)], x: f32) -> f32 {
    let i = breaks.partition_point(|b| b.0 <= x);
    match (i.checked_sub(1).map(|j| breaks[j]), breaks.get(i)) {
        (None, _) => 0.0,
        (Some(a), None) => a.1,
        (Some(a), Some(b)) if b.0 > a.0 => a.1 + (b.1 - a.1) * (x - a.0) / (b.0 - a.0),
        (Some(a), Some(_)) => a.1,
    }
}

fn spacing_head_width(g: Glyph) -> Option<f32> {
    SPACING_HEAD_WIDTHS.iter().find(|h| h.0 == g).map(|h| h.1)
}

/// A stem for the optical spacing corrections; y in spaces up from the
/// middle line.
#[derive(Clone, Copy, Debug)]
struct StemInfo {
    part: usize,
    up: bool,
    attach: f32, // the head the stem leaves
    tip: f32,
    flag: bool,
}
impl StemInfo {
    /// The stem's extent, from just off its head to its tip.
    fn span(&self) -> (f32, f32) {
        if self.up { (self.attach + STEM_ATTACH, self.tip) } else { (self.tip, self.attach - STEM_ATTACH) }
    }
    /// Stem length inside the staff (next to a barline).
    fn in_staff(&self) -> f32 {
        overlap(self.span(), (-2.0, 2.0))
    }
}
fn overlap((l1, h1): (f32, f32), (l2, h2): (f32, f32)) -> f32 {
    (h1.min(h2) - l1.max(l2)).max(0.0)
}

/// Beams (LilyPond's Stem and Beam details): ideal beamed stem lengths (to
/// the outer beam) by number of beams, the least free stem below the
/// innermost beam, the slope damping, and the demerit weights of the
/// quanting.
const BEAMED_LENGTHS: [f32; 3] = [3.26, 3.5, 3.6];
const BEAMED_MIN_FREE: [f32; 3] = [1.83, 1.5, 1.25];
const BEAM_SLOPE_DAMPING: f32 = 0.6;
const BEAM_SLOPE_FACTOR: f32 = 10.0;
const BEAM_FLAT_PENALTY: f32 = 800.0;
const BEAM_DIRECTION_PENALTY: f32 = 400.0;
const STEM_LENGTH_DEMERIT: f32 = 5.0;
/// Stems shorter than ideal cost more than longer ones.
const BEAM_SHORT_FACTOR: f32 = 2.0;
const BEAM_CONCAVE_DAMPING: f32 = 3.0;

/// The movement's common shortest duration: the shortest note of each bar,
/// the one most bars have (the shorter on a tie), capped at
/// `BASE_SHORTEST`.
fn common_shortest(ms: &[Measure], notes: &[Note]) -> Frac {
    let mut count: Vec<(Frac, usize)> = Vec::new();
    for mi in 0..ms.len() {
        let Some(d) = notes.iter().filter(|n| n.measure == mi && !n.whole_bar).map(|n| n.dur).min() else { continue };
        match count.iter_mut().find(|c| c.0 == d) {
            Some(c) => c.1 += 1,
            None => count.push((d, 1)),
        }
    }
    let best = count.iter().max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0))).map(|c| c.0);
    best.map_or(BASE_SHORTEST, |d| d.min(BASE_SHORTEST))
}

// ----------------------------------------------------------------- measures --
#[derive(Clone, Debug)]
struct Measure {
    start: Frac,
    len: Frac,
    num: i64,
    den: i64,
    show_time: bool,
    key: Option<Key>, // key signature shown at this barline
    x0: f32,          // after the opening barline
    x1: f32,          // closing barline
    last: bool,       // the movement's last bar (final barline)
    shortest: Frac,   // the movement's common shortest duration (spacing)
    stretch: f32,     // the stretchability of the bar's springs
}

fn measures(s: &Score) -> Vec<Measure> {
    let mut meters = s.meter.clone();
    meters.sort_by(|a, b| a.at.cmp(&b.at));
    let mut keys = s.keys.clone();
    keys.sort_by(|a, b| a.at.cmp(&b.at));
    let mut out: Vec<Measure> = Vec::new();
    let mut at = Frac::ZERO;
    while at < s.length && out.len() < 100_000 {
        let m = meters.iter().rev().find(|m| m.at <= at).copied();
        let (num, den) = m.map(|m| (m.num.max(1), m.den.max(1))).unwrap_or((4, 4));
        let mut len = Frac::new(num, den);
        if let Some(next) = meters.iter().find(|x| x.at > at && x.at < at + len) {
            len = next.at - at;
        }
        if at + len > s.length {
            len = s.length - at;
        }
        let prev = out.last();
        let show_time = prev.is_none_or(|p| (p.num, p.den) != (num, den));
        let key = keys.iter().rev().find(|k| k.at <= at).copied();
        let key_changed = match (prev, key) {
            (None, k) => k.or(Some(Key { at: Frac::ZERO, fifths: 0, minor: false })),
            (Some(p), Some(k)) if k.at > p.start => Some(k),
            _ => None,
        };
        out.push(Measure { start: at, len, num, den, show_time, key: key_changed, x0: 0.0, x1: 0.0, last: false, shortest: BASE_SHORTEST, stretch: 0.0 });
        at = at + len;
    }
    if let Some(m) = out.last_mut() {
        m.last = true;
    }
    out
}

/// A staff's key: the score's, moved for a transposing instrument.
fn part_key(s: &Score, part: usize, fifths: i32) -> i32 {
    match s.parts.get(part).map(|p| p.key_shift).unwrap_or(0) {
        0 => fifths,
        by => {
            let mut f = fifths + by;
            while f > 7 {
                f -= 12;
            }
            while f < -7 {
                f += 12;
            }
            f
        }
    }
}

fn key_at(s: &Score, at: Frac) -> i32 {
    s.keys.iter().filter(|k| k.at <= at).max_by(|a, b| a.at.cmp(&b.at)).map(|k| k.fifths).unwrap_or(0)
}

// ------------------------------------------------------------------- pieces --
#[derive(Clone, Debug)]
struct Head {
    id: u64,
    pos: i32, // half-spaces above the middle line
    step: u8,
    alter: i8,
    dia: i32,
    tie_out: bool,
    tied_in: bool, // tied from the previous note (maybe in the previous system)
    accidental: Option<Glyph>,
    acc_x: f32, // accidental origin, relative to the column
    shift: f32, // notehead offset from the column (seconds in chords)
}

#[derive(Clone, Debug)]
struct Note {
    part: usize,
    voice: u8,
    measure: usize,
    start: Frac,
    dur: Frac,
    value: NoteValue,
    rest: bool,
    whole_bar: bool,
    id: u64,
    heads: Vec<Head>, // bottom to top
    first: bool,      // first piece of its event (marks go here)
    art: u16,
    dynamic: Option<String>,
    lyrics: Vec<corinzia_score::score::Lyric>, // on the first piece only
    // layout
    stem_up: bool,
    beamed: bool,
    stem_x: f32,
    stem_end: f32,
    x: f32,
}

impl Note {
    fn head_glyph(&self) -> Glyph {
        match self.value.base {
            0 | 1 => Glyph::NoteheadWhole,
            2 => Glyph::NoteheadHalf,
            _ => Glyph::NoteheadBlack,
        }
    }
    fn has_stem(&self) -> bool {
        !self.rest && self.value.base >= 2
    }
    fn beams(&self) -> u32 {
        match self.value.base {
            8 => 1,
            16 => 2,
            32 => 3,
            64 => 4,
            128 => 5,
            _ => 0,
        }
    }
}

fn split(score: &Score, ms: &[Measure]) -> Vec<Note> {
    let clefs_at = |part: &Part, at: Frac| -> String {
        part.clefs.iter().filter(|c| c.at <= at).max_by(|a, b| a.at.cmp(&b.at)).map(|c| c.clef.clone()).unwrap_or(part.clef.clone())
    };
    let mut out = Vec::new();
    for (pi, part) in score.parts.iter().enumerate() {
        for v in &part.voices {
            for e in &v.events {
                let end = e.start + e.dur;
                let mut first = true;
                for (mi, m) in ms.iter().enumerate() {
                    let (a, b) = (e.start.max(m.start), end.min(m.start + m.len));
                    if a >= b {
                        continue;
                    }
                    let rest = e.pitches.is_empty();
                    let whole_bar = rest && a == m.start && b == m.start + m.len;
                    let values = if whole_bar { vec![NoteValue { base: 1, dots: 0, triplet: false }] } else { decompose(b - a) };
                    let mut t = a;
                    for (k, nv) in values.iter().enumerate() {
                        let last_piece = k + 1 == values.len() && b == end;
                        let middle = clef_middle(&clefs_at(part, t));
                        let mut heads: Vec<Head> = e
                            .pitches
                            .iter()
                            .map(|p| Head {
                                id: p.id,
                                pos: diatonic(p) - middle,
                                step: p.step,
                                alter: p.alter,
                                dia: diatonic(p),
                                tie_out: !last_piece || p.tie,
                                tied_in: false,
                                accidental: None,
                                acc_x: 0.0,
                                shift: 0.0,
                            })
                            .collect();
                        heads.sort_by_key(|h| h.dia);
                        let dur = if whole_bar { m.len } else { nv.length() };
                        out.push(Note {
                            part: pi,
                            voice: v.index,
                            measure: mi,
                            start: t,
                            dur,
                            value: *nv,
                            rest,
                            whole_bar,
                            id: e.id,
                            heads,
                            first,
                            art: e.art,
                            dynamic: e.dynamic.clone(),
                            lyrics: if first { e.lyrics.clone() } else { vec![] },
                            stem_up: true,
                            beamed: false,
                            stem_x: 0.0,
                            stem_end: 0.0,
                            x: 0.0,
                        });
                        first = false;
                        t = t + dur;
                    }
                }
            }
        }
    }
    // Voice 1 of a part with nothing in a bar gets a whole-bar rest.
    for pi in 0..score.parts.len() {
        for (mi, m) in ms.iter().enumerate() {
            if !out.iter().any(|n| n.part == pi && n.measure == mi && n.voice == 1) {
                out.push(Note {
                    part: pi,
                    voice: 1,
                    measure: mi,
                    start: m.start,
                    dur: m.len,
                    value: NoteValue { base: 1, dots: 0, triplet: false },
                    rest: true,
                    whole_bar: true,
                    id: 0,
                    heads: vec![],
                    first: true,
                    art: 0,
                    dynamic: None,
                    lyrics: vec![],
                    stem_up: true,
                    beamed: false,
                    stem_x: 0.0,
                    stem_end: 0.0,
                    x: 0.0,
                });
            }
        }
    }
    out.sort_by(|a, b| (a.part, a.voice, a.start).cmp(&(b.part, b.voice, b.start)));
    out
}

// ------------------------------------------------------------------- engine --
struct Engraver<'a> {
    score: &'a Score,
    font: &'static Font,
    opt: Options,
    ms: Vec<Measure>,
    notes: Vec<Note>,
    out: DisplayList,
    left: f32,                               // where the staves start (after instrument names)
    clef_changes: Vec<(usize, Frac, String)>, // (part, at, clef), at > 0
    sky: Vec<Skyline>,                       // per staff, filled after the notes
    first_system: bool,                      // full instrument names (else abbreviations)
    width: Option<f32>,                      // justify the system to this width
    range: (Frac, Frac),                     // written span of this system
    tops: Vec<f32>,                          // y of each staff's top line
}

const HEAD_W: f32 = 1.18;
/// The brace before a grand staff, and its gap to the system's barline.
const BRACE_WIDTH: f32 = 1.3;
const BRACE_GAP: f32 = 0.4;

/// A brace from `top` to `bottom` whose right edge is at `right`: two
/// filled halves meeting at a point in the middle, thickest half-way along
/// each.
fn brace(right: f32, top: f32, bottom: f32) -> [Item; 2] {
    let (h, mid) = ((bottom - top) / 2.0, (top + bottom) / 2.0);
    let w = BRACE_WIDTH.min(0.08 * (bottom - top) + 0.4);
    let xl = right - w;
    let t = (0.09 * h).clamp(0.2, 0.55); // the stroke's thickness at its widest
    // Each half is an S: it leaves its end heading left, runs down the
    // middle, and turns left again into the point.
    let half = |y0: f32, dir: f32| Item::Crescent {
        from: pt(right, y0),
        c1: pt(xl - 0.15 * w, y0 + 0.1 * h * dir),
        c2: pt(right + 0.15 * w, mid - 0.1 * h * dir),
        to: pt(xl, mid),
        d1: pt(xl - 0.15 * w + t, y0 + 0.1 * h * dir),
        d2: pt(right + 0.15 * w + t, mid - 0.1 * h * dir),
        id: 0,
    };
    [half(top, 1.0), half(bottom, -1.0)]
}

impl<'a> Engraver<'a> {
    fn top(&self, part: usize) -> f32 {
        self.tops.get(part).copied().unwrap_or(part as f32 * self.opt.staff_distance)
    }
    fn y(&self, part: usize, pos: i32) -> f32 {
        self.top(part) + 2.0 - pos as f32 * 0.5
    }
    fn width(&self, g: Glyph) -> f32 {
        self.font.bbox(g).width()
    }
    fn push(&mut self, item: Item) {
        self.out.items.push(item);
    }

    /// Stem directions (voice rule in polyphony, furthest-head rule
    /// otherwise), seconds in chords, and accidentals with their columns.
    fn prepare(&mut self) {
        let n = self.notes.len();
        for i in 0..n {
            let (part, measure, voice) = (self.notes[i].part, self.notes[i].measure, self.notes[i].voice);
            let poly = self.notes.iter().any(|o| o.part == part && o.measure == measure && o.voice != voice && !o.whole_bar);
            let note = &mut self.notes[i];
            note.stem_up = if poly {
                voice % 2 == 1
            } else if note.heads.is_empty() {
                true
            } else {
                let (lo, hi) = (note.heads[0].pos, note.heads[note.heads.len() - 1].pos);
                // The head furthest from the middle line decides; balanced or on the line: down.
                if hi.abs() == lo.abs() { hi + lo < 0 } else { lo.abs() > hi.abs() }
            };
        }
        // Accidentals: per part and bar, against the key and earlier notes.
        let mut state: std::collections::HashMap<(usize, usize, i32), i8> = Default::default();
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|&a, &b| (self.notes[a].part, self.notes[a].start).cmp(&(self.notes[b].part, self.notes[b].start)));
        let mut tied_into: std::collections::HashSet<(usize, u8, i32, i8)> = Default::default();
        let mut next_tied: Vec<(usize, u8, i32, i8)> = Vec::new();
        let mut last_start: Option<(usize, Frac)> = None;
        for &i in &order {
            let (part, measure, start) = (self.notes[i].part, self.notes[i].measure, self.notes[i].start);
            if last_start.is_some_and(|(p, s)| p != part || s != start) {
                tied_into.extend(next_tied.drain(..));
            }
            last_start = Some((part, start));
            let fifths = part_key(self.score, part, key_at(self.score, start));
            let voice = self.notes[i].voice;
            for h in self.notes[i].heads.iter_mut() {
                let key = (part, measure, h.dia);
                let current = *state.get(&key).unwrap_or(&key_alter(fifths, h.step));
                let tied = tied_into.remove(&(part, voice, h.dia, h.alter));
                h.tied_in = tied;
                if h.alter != current && !tied {
                    h.accidental = Some(accidental_glyph(h.alter));
                }
                state.insert(key, h.alter);
                if h.tie_out {
                    next_tied.push((part, voice, h.dia, h.alter));
                }
            }
        }
        // Seconds and accidental columns.
        for note in self.notes.iter_mut() {
            let k = note.heads.len();
            if k > 1 {
                if note.stem_up {
                    // From the bottom: a head a second above an unshifted one goes right.
                    for j in 1..k {
                        if note.heads[j].pos - note.heads[j - 1].pos == 1 && note.heads[j - 1].shift == 0.0 {
                            note.heads[j].shift = HEAD_W - 0.12;
                        }
                    }
                } else {
                    for j in (0..k - 1).rev() {
                        if note.heads[j + 1].pos - note.heads[j].pos == 1 && note.heads[j + 1].shift == 0.0 {
                            note.heads[j].shift = -(HEAD_W - 0.12);
                        }
                    }
                }
            }
            // Columns right to left, top accidental first; a column takes an
            // accidental if it clears the others in it by a sixth.
            let left_edge = note.heads.iter().map(|h| h.shift).fold(0.0f32, f32::min);
            let mut columns: Vec<(f32, Vec<i32>)> = Vec::new(); // (x of the column's left edge, positions)
            let mut x_cursor = left_edge - 0.23; // LilyPond: 0.35 from accidental to head
            for j in (0..k).rev() {
                let Some(g) = note.heads[j].accidental else { continue };
                let w = Font::bravura().bbox(g).width();
                let pos = note.heads[j].pos;
                let slot = columns.iter_mut().find(|(_, ps)| ps.iter().all(|p| (p - pos).abs() >= 6));
                match slot {
                    Some((x, ps)) => {
                        ps.push(pos);
                        note.heads[j].acc_x = *x;
                    }
                    None => {
                        x_cursor -= w + 0.12;
                        columns.push((x_cursor, vec![pos]));
                        note.heads[j].acc_x = x_cursor;
                    }
                }
            }
        }
    }

    /// Beat groups -> beamed runs; their stem direction is the group's.
    /// Beam groups by LilyPond's defaults: one beam per beat (dotted
    /// quarters in compound meters, halves in 2/2 and 3/2), except that
    /// beams of eighths only span half a bar in 4/4 and the bar in 3/4.
    fn beam_groups(&mut self) -> Vec<Vec<usize>> {
        let beat_of = |m: &Measure| {
            if m.den == 8 && m.num % 3 == 0 {
                Frac::new(3, 8)
            } else if m.den <= 2 {
                Frac::new(1, 2)
            } else if m.den >= 8 {
                Frac::new(2, m.den)
            } else {
                Frac::new(1, m.den)
            }
        };
        let eighths_span = |m: &Measure| match (m.num, m.den) {
            (4, 4) => Some(Frac::new(1, 2)),
            (3, 4) => Some(Frac::new(3, 4)),
            _ => None,
        };
        // Notes in the same window of `span` (per voice and bar) share a beam.
        let group = |this: &Self, idx: &[usize], span: &dyn Fn(&Measure) -> Frac| -> Vec<Vec<usize>> {
            let mut groups = Vec::new();
            let mut cur: Vec<usize> = Vec::new();
            let mut cur_key = None;
            for &i in idx {
                let n = &this.notes[i];
                let m = &this.ms[n.measure];
                let w = span(m);
                let rel = (n.start - m.start) * Frac::new(w.den, w.num);
                let key = (n.part, n.voice, n.measure, rel.num / rel.den);
                let beamable = !n.rest && n.value.base >= 8;
                if !beamable || cur_key != Some(key) {
                    if cur.len() >= 2 {
                        groups.push(std::mem::take(&mut cur));
                    }
                    cur.clear();
                }
                if beamable {
                    cur.push(i);
                    cur_key = Some(key);
                } else {
                    cur_key = None;
                }
            }
            if cur.len() >= 2 {
                groups.push(cur);
            }
            groups
        };
        let all: Vec<usize> = (0..self.notes.len()).collect();
        let wide = group(self, &all, &|m: &Measure| eighths_span(m).unwrap_or_else(|| beat_of(m)));
        // A wide beam holding shorter notes than eighths goes back to beats.
        let mut groups = Vec::new();
        for g in wide {
            if g.iter().any(|&i| self.notes[i].beams() > 1) {
                groups.extend(group(self, &g, &beat_of));
            } else {
                groups.push(g);
            }
        }
        for g in &groups {
            let poly = {
                let n = &self.notes[g[0]];
                self.notes.iter().any(|o| o.part == n.part && o.measure == n.measure && o.voice != n.voice && !o.whole_bar)
            };
            let up = if poly {
                self.notes[g[0]].voice % 2 == 1
            } else {
                let sum: i32 = g.iter().flat_map(|&i| self.notes[i].heads.iter().map(|h| h.pos)).sum();
                sum < 0
            };
            for &i in g {
                self.notes[i].stem_up = up;
                self.notes[i].beamed = true;
            }
        }
        groups
    }

    // ------------------------------------------------------------- spacing --
    fn note_extents(&self, n: &Note) -> (f32, f32) {
        let left = n.heads.iter().map(|h| if h.accidental.is_some() { -h.acc_x } else { -h.shift.min(0.0) }).fold(0.0f32, f32::max);
        let mut right = if n.rest { self.width(rest_glyph(n.value.base)) } else { spacing_head_width(n.head_glyph()).unwrap_or(HEAD_W) };
        right += n.heads.iter().map(|h| h.shift.max(0.0)).fold(0.0f32, f32::max);
        if n.value.dots > 0 {
            right += 0.4 + n.value.dots as f32 * 0.5;
        }
        if n.has_stem() && n.stem_up && !n.beamed && n.beams() > 0 {
            right += 0.9; // flag
        }
        if n.art & art::ARPEGGIO != 0 && n.heads.len() > 1 {
            return (left + 1.2, right);
        }
        (left, right)
    }

    /// Width of the widest head (or rest) in a column.
    fn column_head_width(&self, col: &[&Note]) -> f32 {
        col.iter()
            .map(|n| {
                if n.rest {
                    return self.width(rest_glyph(n.value.base));
                }
                let g = n.head_glyph();
                let w = spacing_head_width(g).unwrap_or_else(|| self.width(g));
                w + n.heads.iter().map(|h| h.shift.max(0.0)).fold(0.0f32, f32::max)
            })
            .fold(0.0f32, f32::max)
    }

    /// Per staff with a single stemmed note in the column.
    fn stems(&self, col: &[&Note]) -> Vec<StemInfo> {
        let mut out = Vec::new();
        for pi in 0..self.score.parts.len() {
            let ns: Vec<&&Note> = col.iter().filter(|n| n.part == pi && !n.rest).collect();
            let [n] = ns.as_slice() else { continue };
            if !n.has_stem() || n.heads.is_empty() {
                continue;
            }
            let (lo, hi) = (n.heads[0].pos as f32 * 0.5, n.heads[n.heads.len() - 1].pos as f32 * 0.5);
            let flag = !n.beamed && n.beams() > 0;
            out.push(if n.stem_up {
                StemInfo { part: pi, up: true, attach: lo, tip: (hi + STEM_LENGTH).max(0.0), flag }
            } else {
                StemInfo { part: pi, up: false, attach: hi, tip: (lo - STEM_LENGTH).min(0.0), flag }
            });
        }
        out
    }

    /// LilyPond's optical stem correction between two neighbouring columns
    /// (the largest over the staves).
    fn stem_correction(&self, a: &[&Note], b: &[&Note]) -> f32 {
        let sb = self.stems(b);
        self.stems(a)
            .into_iter()
            .filter_map(|x| {
                let y = sb.iter().find(|s| s.part == x.part)?;
                Some(match (x.up, y.up) {
                    (true, false) => STEM_SPACING_CORRECTION * overlap(x.span(), y.span()) / STEM_LENGTH,
                    (false, true) => -STEM_SPACING_CORRECTION * overlap(x.span(), y.span()) / STEM_LENGTH,
                    _ => {
                        let d = y.attach - x.attach;
                        if d.abs() >= 1.0 { SAME_DIRECTION_CORRECTION * d.signum() } else { 0.0 }
                    }
                })
            })
            .reduce(f32::max)
            .unwrap_or(0.0)
    }

    fn space(&mut self) {
        let mut x = self.left;
        // System start: clef, key, time.
        let clef_w = self.score.parts.iter().map(|p| self.width(clef_glyph(&p.clef).0)).fold(0.0f32, f32::max);
        x += 0.8 + clef_w + 0.8;
        let nms = self.ms.len();
        // Right edge of the last syllable per (part, verse), and whether it is hyphenated.
        let mut lyric_edge: std::collections::HashMap<(usize, u8), (f32, bool)> = Default::default();
        // Right edge of the last chord symbol per part.
        let mut chord_edge: std::collections::HashMap<usize, f32> = Default::default();
        let mut pending_chords: Vec<(usize, f32)> = Vec::new();
        // How much of the line stretches when it is justified: (x, the
        // stretchability of the springs left of x), as in LilyPond, where a
        // note's spring stretches by its duration space and fixed distances
        // (signatures, padding) do not.
        let mut breaks: Vec<(f32, f32)> = Vec::new();
        let mut stretch = 0.0f32;
        for mi in 0..nms {
            if mi > 0 {
                x += 0.0;
            }
            let m = self.ms[mi].clone();
            let bar_stretch = stretch;
            // Key and time shown at this barline.
            if let Some(k) = m.key {
                // The widest staff's signature (transposing staves differ).
                let prev_global = if mi == 0 { 0 } else { key_at(self.score, self.ms[mi - 1].start) };
                let count = (0..self.score.parts.len().max(1))
                    .map(|pi| {
                        let (now, prev) = (part_key(self.score, pi, k.fifths), part_key(self.score, pi, prev_global));
                        now.unsigned_abs().max(if mi > 0 { prev.unsigned_abs() } else { 0 })
                    })
                    .max()
                    .unwrap_or(0);
                if count > 0 || mi == 0 {
                    x += count as f32 * 1.1 + if count > 0 { 0.8 } else { 0.0 };
                }
            }
            if m.show_time {
                x += self.time_width(&m) + 1.0;
            }
            self.ms[mi].x0 = x;
            let end = m.start + m.len;
            let mut cols: Vec<Frac> = self.notes.iter().filter(|n| n.measure == mi).map(|n| n.start).collect();
            cols.sort();
            cols.dedup();
            if cols.is_empty() {
                cols.push(m.start);
            }
            let mut xs = Vec::with_capacity(cols.len());
            let mut prev_right = 0.0f32;
            let mut prev_here: Vec<&Note> = Vec::new();
            let mut cx = x;
            // The spring from the column at `t0` over `gap`: LilyPond measures
            // it from the left note's head, so a duration's space loses one
            // increment and gains the head's width. (length, stretchability)
            let ideal = |t0: Frac, gap: Frac, col: &[&Note]| -> Option<(f32, f32)> {
                let dmin = self
                    .notes
                    .iter()
                    .filter(|n| n.measure == mi && n.start <= t0 && n.start + n.dur > t0 && !n.whole_bar)
                    .map(|n| n.dur)
                    .min()
                    .unwrap_or(gap);
                if gap <= Frac::ZERO || dmin <= Frac::ZERO {
                    return None;
                }
                let space = ideal_space(dmin, m.shortest) * (gap.num as f32 / gap.den as f32) / (dmin.num as f32 / dmin.den as f32);
                let stretch = (space - SPACING_INCREMENT as f32).max(0.0);
                Some((stretch + self.column_head_width(col), stretch))
            };
            for (j, &t) in cols.iter().enumerate() {
                let here: Vec<&Note> = self.notes.iter().filter(|n| n.measure == mi && n.start == t && !n.whole_bar).collect();
                // Syllables here must clear the previous ones of their verse.
                let mut lyric_min = f32::MIN;
                for n in &here {
                    for l in &n.lyrics {
                        let w = text_width(&l.text, LYRIC_SIZE, false);
                        if let Some(&(edge, hyphen)) = lyric_edge.get(&(n.part, l.verse)) {
                            lyric_min = lyric_min.max(edge + if hyphen { HYPHEN_GAP } else { WORD_GAP } + w / 2.0 - HEAD_W / 2.0);
                        }
                    }
                }
                let (mut left, right) = here.iter().map(|n| self.note_extents(n)).fold((0.0f32, 0.0f32), |a, b| (a.0.max(b.0), a.1.max(b.1)));
                if self.clef_changes.iter().any(|c| c.1 == t) {
                    left += self.clef_change_width() + 0.5;
                }
                if j == 0 {
                    // From the barline (or the system's opening signatures).
                    cx += if left > 0.0 {
                        BAR_TO_ACCIDENTAL + left
                    } else {
                        BAR_TO_NOTE + self.stems(&here).iter().filter(|s| !s.up).map(|s| STEM_AFTER_BAR * s.in_staff() / STEM_LENGTH).fold(0.0f32, f32::max)
                    };
                    if cols.len() == 1 && !here.is_empty() {
                        cx += LONE_NOTE_SPACE;
                    }
                    if mi > 0 {
                        stretch += BAR_TO_NOTE_STRETCH;
                    }
                } else {
                    let (spring, k) = ideal(cols[j - 1], t - cols[j - 1], &prev_here).unwrap_or((0.0, 0.0));
                    stretch += k;
                    let spring = spring + self.stem_correction(&prev_here, &here);
                    cx += spring.max(prev_right + left + if left > 0.0 { ROD_PADDING_ACCIDENTAL } else { ROD_PADDING });
                }
                // A chord symbol here must clear the previous one on its staff.
                for (pi, part) in self.score.parts.iter().enumerate() {
                    if let Some(c) = part.chords.iter().find(|c| c.at == t) {
                        if let Some(&edge) = chord_edge.get(&pi) {
                            lyric_min = lyric_min.max(edge + 1.0 + 0.3);
                        }
                        let w: f32 = chord_runs(&c.text).iter().map(|r| self.chord_run_width(r)).sum();
                        pending_chords.push((pi, w));
                    }
                }
                cx = cx.max(lyric_min);
                for (pi, w) in pending_chords.drain(..) {
                    chord_edge.insert(pi, cx - 0.3 + w);
                }
                for n in &here {
                    for l in &n.lyrics {
                        let w = text_width(&l.text, LYRIC_SIZE, false);
                        lyric_edge.insert((n.part, l.verse), (cx + HEAD_W / 2.0 + w / 2.0, l.hyphen));
                    }
                }
                xs.push(cx);
                breaks.push((cx, stretch));
                prev_right = right;
                prev_here = here;
            }
            // To the closing barline (its centre): the last column's spring,
            // corrected for the side its stems are on.
            let tlast = *cols.last().unwrap();
            let bar_corr = self
                .stems(&prev_here)
                .iter()
                .filter(|s| !s.flag)
                .map(|s| STEM_BEFORE_BAR * s.in_staff() / STEM_LENGTH * if s.up { 1.0 } else { -1.0 })
                .reduce(f32::max)
                .unwrap_or(0.0);
            let (spring, k) = ideal(tlast, end - tlast, &prev_here).map(|(s, k)| (s + bar_corr + BAR_HALF_WIDTH, k)).unwrap_or((0.0, 0.0));
            stretch += k;
            let mut x1 = cx + spring.max(prev_right + 0.8);
            if self.notes.iter().filter(|n| n.measure == mi).all(|n| n.whole_bar) {
                x1 = x1.max(x + 8.0); // an empty bar keeps a sensible width
                stretch += (ideal_space(m.len, m.shortest) - SPACING_INCREMENT as f32).max(0.0);
            }
            breaks.push((x1, stretch));
            self.ms[mi].stretch = stretch - bar_stretch;
            for n in self.notes.iter_mut().filter(|n| n.measure == mi) {
                let j = cols.iter().position(|&t| t == n.start).unwrap_or(0);
                n.x = xs[j];
            }
            for (j, &t) in cols.iter().enumerate() {
                self.out.columns.push(Column { quarter: (t.num as f64 / t.den as f64) * 4.0, x: xs[j], system: 0 });
            }
            self.ms[mi].x1 = x1;
            x = x1 + 0.0;
        }
        let end = self.ms.last().map(|m| m.start + m.len).unwrap_or(Frac::ZERO);
        self.out.columns.push(Column { quarter: end.num as f64 / end.den as f64 * 4.0, x, system: 0 });
        // Justify: the springs share the extra room by their stretchability.
        if let Some(w) = self.width {
            let target = w - self.opt.margin;
            if stretch > 0.0 && (target - x).abs() > 1e-3 {
                // Compressed lines stop short of the springs' minimum.
                let k = ((target - x) / stretch).max(MAX_COMPRESSION);
                let f = |v: f32| v + k * stretch_at(&breaks, v);
                for n in self.notes.iter_mut() {
                    n.x = f(n.x);
                }
                for m in self.ms.iter_mut() {
                    m.x0 = f(m.x0);
                    m.x1 = f(m.x1);
                }
                for c in self.out.columns.iter_mut() {
                    c.x = f(c.x);
                }
                x = target;
            }
        }
        self.out.width = x + self.opt.margin;
    }

    /// Full name on the first system, the abbreviation (if any) after it.
    fn part_name(&self, part: usize) -> String {
        let p = &self.score.parts[part];
        if self.first_system || p.abbr.is_empty() { p.name.clone() } else { p.abbr.clone() }
    }

    fn clef_change_width(&self) -> f32 {
        self.width(Glyph::GClef) * CLEF_CHANGE_SCALE
    }

    fn time_width(&self, m: &Measure) -> f32 {
        let digits = |v: i64| v.to_string().len() as f32;
        digits(m.num).max(digits(m.den)) * self.width(Glyph::TimeSigDigit(4))
    }

    // ------------------------------------------------------------- drawing --
    fn draw_staves(&mut self) {
        let t = self.font.defaults.staff_line_thickness;
        let right = self.out.width - self.opt.margin;
        for pi in 0..self.score.parts.len() {
            let top = self.top(pi);
            self.out.staves.push(StaffGeom { part: pi, top, left: self.left, right, line_width: t, system: 0 });
            // Clef at the system start (the one in force); the instrument name left of the staff.
            let clef = self.clef_at(pi, self.range.0);
            let (g, dy) = clef_glyph(&clef);
            self.push(Item::glyph(g, self.left + 0.8, top + dy, 0));
            // A grand staff is named once, between its staves, left of its brace.
            let braced = self.score.parts[pi].brace && pi + 1 < self.score.parts.len();
            let second = pi > 0 && self.score.parts[pi - 1].brace;
            let name = self.part_name(pi);
            if self.opt.names && !name.is_empty() && !second {
                let (x, y) = if braced { (self.left - BRACE_WIDTH - BRACE_GAP - 1.0, (top + self.top(pi + 1) + 4.0) / 2.0 + 0.7) } else { (self.left - 1.0, top + 2.7) };
                let x = if self.score.parts.iter().any(|p| p.brace) && !braced { x - BRACE_WIDTH - BRACE_GAP } else { x };
                self.push(Item::Text { text: name, at: pt(x, y), size: NAME_SIZE, italic: false, bold: false, anchor: 2, id: 0 });
            }
            if braced {
                let bottom = self.top(pi + 1) + 4.0;
                for item in brace(self.left - BRACE_GAP, top, bottom) {
                    self.push(item);
                }
            }
        }
        // System barline at the left edge.
        if let Some(last) = self.score.parts.len().checked_sub(1) {
            let thin = self.font.defaults.thin_barline_thickness;
            self.push(Item::line(self.left, 0.0, self.left, self.top(last) + 4.0, thin, 0));
        }
    }

    fn draw_measure_furniture(&mut self) {
        let d = self.font.defaults;
        let clef_w = self.score.parts.iter().map(|p| self.width(clef_glyph(&p.clef).0)).fold(0.0f32, f32::max);
        for mi in 0..self.ms.len() {
            let m = self.ms[mi].clone();
            let mut x = if mi == 0 { self.left + 0.8 + clef_w + 0.8 } else { self.ms[mi - 1].x1 + 1.0 };
            for pi in 0..self.score.parts.len() {
                let mut kx = x;
                let clef = self.clef_at(pi, m.start);
                if let Some(k) = m.key {
                    let k = Key { fifths: part_key(self.score, pi, k.fifths), ..k };
                    // Cancel the previous key's accidentals with naturals when it changes to fewer/other.
                    if mi > 0 {
                        let prev = part_key(self.score, pi, key_at(self.score, self.ms[mi - 1].start));
                        if prev != 0 && (k.fifths == 0 || k.fifths.signum() != prev.signum()) {
                            for (pos, _) in key_positions(prev, &clef) {
                                let y = self.y(pi, pos);
                                self.push(Item::glyph(Glyph::Natural, kx, y, 0));
                                kx += 1.0;
                            }
                        }
                    }
                    for (pos, g) in key_positions(k.fifths, &clef) {
                        let y = self.y(pi, pos);
                        self.push(Item::glyph(g, kx, y, 0));
                        kx += self.width(g) + 0.1;
                    }
                    if k.fifths != 0 {
                        kx += 0.7;
                    }
                }
                if m.show_time {
                    let top = self.top(pi);
                    let w = self.time_width(&m);
                    for (v, y) in [(m.num, top + 1.0), (m.den, top + 3.0)] {
                        let s = v.to_string();
                        let dw = self.width(Glyph::TimeSigDigit(4));
                        let mut dx = kx + (w - s.len() as f32 * dw) / 2.0;
                        for ch in s.bytes() {
                            self.push(Item::glyph(Glyph::TimeSigDigit(ch - b'0'), dx, y, 0));
                            dx += dw;
                        }
                    }
                }
            }
            x = m.x0;
            let _ = x;
            // Closing barline (all staves): kind from the score.
            let end = m.start + m.len;
            let kind = self.score.barlines.iter().find(|b| b.at == end).map(|b| b.kind);
            let last = m.last;
            let start_repeat = self.score.barlines.iter().any(|b| b.at == m.start && matches!(b.kind, BarKind::RepeatStart | BarKind::RepeatBoth));
            for pi in 0..self.score.parts.len() {
                // Barlines run through a grand staff.
                let joined = self.score.parts[pi].brace && pi + 1 < self.score.parts.len();
                let (top, bot) = (self.top(pi), if joined { self.top(pi + 1) } else { self.top(pi) + 4.0 });
                let x1 = m.x1;
                match (kind, last) {
                    (Some(BarKind::Final), _) | (None, true) => {
                        self.push(Item::line(x1 - d.thick_barline_thickness - d.barline_separation, top, x1 - d.thick_barline_thickness - d.barline_separation, bot, d.thin_barline_thickness, 0));
                        self.push(Item::line(x1 - d.thick_barline_thickness / 2.0, top, x1 - d.thick_barline_thickness / 2.0, bot, d.thick_barline_thickness, 0));
                    }
                    (Some(BarKind::RepeatEnd), _) | (Some(BarKind::RepeatBoth), _) => {
                        let thick_x = x1 - d.thick_barline_thickness / 2.0;
                        let thin_x = x1 - d.thick_barline_thickness - d.barline_separation;
                        self.push(Item::line(thin_x, top, thin_x, bot, d.thin_barline_thickness, 0));
                        self.push(Item::line(thick_x, top, thick_x, bot, d.thick_barline_thickness, 0));
                        let dw = self.width(Glyph::RepeatDot);
                        for y in [top + 1.5, top + 2.5] {
                            self.push(Item::glyph(Glyph::RepeatDot, thin_x - d.repeat_barline_dot_separation - dw, y, 0));
                        }
                    }
                    (Some(BarKind::Double), _) => {
                        self.push(Item::line(x1 - d.barline_separation, top, x1 - d.barline_separation, bot, d.thin_barline_thickness, 0));
                        self.push(Item::line(x1, top, x1, bot, d.thin_barline_thickness, 0));
                    }
                    _ => self.push(Item::line(x1, top, x1, bot, d.thin_barline_thickness, 0)),
                }
                if start_repeat {
                    // Start repeat: thick-thin with dots after, at this bar's opening.
                    let x0 = if mi == 0 { m.x0 } else { self.ms[mi - 1].x1 + 0.6 };
                    self.push(Item::line(x0, top, x0, bot, d.thick_barline_thickness, 0));
                    let tx = x0 + d.thick_barline_thickness / 2.0 + d.barline_separation;
                    self.push(Item::line(tx, top, tx, bot, d.thin_barline_thickness, 0));
                    for y in [top + 1.5, top + 2.5] {
                        self.push(Item::glyph(Glyph::RepeatDot, tx + d.repeat_barline_dot_separation, y, 0));
                    }
                }
            }
        }
    }

    fn clef_at(&self, part: usize, at: Frac) -> String {
        let p = &self.score.parts[part];
        p.clefs.iter().filter(|c| c.at <= at).max_by(|a, b| a.at.cmp(&b.at)).map(|c| c.clef.clone()).unwrap_or(p.clef.clone())
    }

    /// Stems (with flags) for unbeamed notes, ledger lines, heads,
    /// accidentals, dots, rests.
    fn draw_notes(&mut self) {
        let d = self.font.defaults;
        let st = d.stem_thickness;
        for i in 0..self.notes.len() {
            let n = self.notes[i].clone();
            let top = self.top(n.part);
            if n.rest {
                let g = rest_glyph(n.value.base);
                let poly = self.notes.iter().any(|o| o.part == n.part && o.measure == n.measure && o.voice != n.voice && !o.whole_bar);
                let mut y = if n.value.base <= 1 { top + 1.0 } else { top + 2.0 };
                if poly {
                    y += if n.voice % 2 == 1 { -2.0 } else { 2.0 };
                }
                let x = if n.whole_bar {
                    let m = &self.ms[n.measure];
                    (m.x0 + m.x1) / 2.0 - self.width(g) / 2.0
                } else {
                    n.x
                };
                self.push(Item::glyph(g, x, y, n.id));
                if n.value.base <= 1 && !n.whole_bar || n.value.base > 1 {
                    self.dots(&n, x + self.width(g), &[if n.value.base <= 1 { 1 } else { 1 }]);
                }
                continue;
            }
            let hg = n.head_glyph();
            for h in &n.heads {
                let x = n.x + h.shift;
                let y = self.y(n.part, h.pos);
                // Ledger lines out to the head.
                let ext = d.leger_line_extension;
                let mut lp = 6;
                while lp <= h.pos {
                    let ly = self.y(n.part, lp);
                    self.push(Item::line(x - ext, ly, x + HEAD_W + ext, ly, d.leger_line_thickness, n.id));
                    lp += 2;
                }
                let mut lp = -6;
                while lp >= h.pos {
                    let ly = self.y(n.part, lp);
                    self.push(Item::line(x - ext, ly, x + HEAD_W + ext, ly, d.leger_line_thickness, n.id));
                    lp -= 2;
                }
                self.push(Item::glyph(hg, x, y, h.id));
                if let Some(a) = h.accidental {
                    self.push(Item::glyph(a, n.x + h.acc_x, y, h.id));
                }
            }
            let right = n.x + HEAD_W + n.heads.iter().map(|h| h.shift.max(0.0)).fold(0.0f32, f32::max);
            let positions: Vec<i32> = n.heads.iter().map(|h| h.pos).collect();
            self.dots(&n, right, &positions);
            if !n.has_stem() {
                continue;
            }
            let (lo, hi) = (n.heads[0].pos, n.heads[n.heads.len() - 1].pos);
            let mid = top + 2.0;
            let (stem_x, from, mut to) = if n.stem_up {
                let a = self.font.anchor(Glyph::NoteheadBlack, "stemUpSE").unwrap_or((HEAD_W, 0.168));
                (n.x + a.0 - st / 2.0, self.y(n.part, lo) - a.1, self.y(n.part, hi) - 3.5)
            } else {
                let a = self.font.anchor(Glyph::NoteheadBlack, "stemDownNW").unwrap_or((0.0, -0.168));
                (n.x + a.0 + st / 2.0, self.y(n.part, hi) - a.1, self.y(n.part, lo) + 3.5)
            };
            // Stems of notes far outside the staff reach the middle line.
            if n.stem_up { to = to.min(mid) } else { to = to.max(mid) }
            // Room for flags of short values.
            let extra = n.beams().saturating_sub(2) as f32 * 0.75;
            if n.stem_up { to -= extra } else { to += extra }
            self.notes[i].stem_x = stem_x;
            self.notes[i].stem_end = to;
            if n.beamed {
                continue; // drawn with the beam
            }
            self.push(Item::line(stem_x, from, stem_x, to, st, n.id));
            if n.beams() > 0 {
                let f = flag_glyph(n.value.base, n.stem_up);
                self.push(Item::glyph(f, stem_x - st / 2.0, to, n.id));
            }
        }
    }

    fn dots(&mut self, n: &Note, right: f32, positions: &[i32]) {
        if n.value.dots == 0 {
            return;
        }
        let mut done = Vec::new();
        for &p in positions {
            // On a line: the dot goes in the space above (below in voice 2).
            let dp = if p % 2 == 0 { if n.voice % 2 == 0 && !n.rest { p - 1 } else { p + 1 } } else { p };
            if done.contains(&dp) {
                continue;
            }
            done.push(dp);
            for k in 0..n.value.dots {
                let x = right + 0.25 + k as f32 * 0.5;
                let y = if n.rest { self.top(n.part) + 1.5 } else { self.y(n.part, dp) };
                self.push(Item::glyph(Glyph::Dot, x, y, n.id));
            }
        }
    }

    /// The beam of a group, placed as LilyPond does: ideal stem ends
    /// fitted by least squares, the slope damped (0.6 tanh), flat for
    /// concave groups, then the best of the allowed positions (in the staff,
    /// on a line or 0.19 off it) by stem-length and slope demerits.
    /// Returns the outer edge at the first stem and the slope, page y down.
    fn quant_beam(&self, g: &[usize], up: bool, bt: f32, gap: f32) -> (f32, f32) {
        let dir = if up { 1.0 } else { -1.0 };
        let part = self.notes[g[0]].part;
        let mid = self.top(part) + 2.0;
        // Per stem: x, the head the stem ends nearest (y up from the middle
        // line), its number of beams.
        let stems: Vec<(f32, f32, u32)> = g
            .iter()
            .map(|&i| {
                let n = &self.notes[i];
                let (lo, hi) = (n.heads[0].pos as f32 * 0.5, n.heads[n.heads.len() - 1].pos as f32 * 0.5);
                (n.stem_x, if up { hi } else { lo }, n.beams().max(1))
            })
            .collect();
        let x0 = stems[0].0;
        let outer_len = |b: u32| BEAMED_LENGTHS[(b.min(3) - 1) as usize];
        // Ideal beam centre lines over each stem, and their least-squares line.
        let ideal: Vec<(f32, f32)> = stems.iter().map(|&(x, h, b)| (x - x0, h + dir * (outer_len(b) - bt / 2.0))).collect();
        let k = ideal.len() as f32;
        let (mx, my) = (ideal.iter().map(|p| p.0).sum::<f32>() / k, ideal.iter().map(|p| p.1).sum::<f32>() / k);
        let sxx: f32 = ideal.iter().map(|p| (p.0 - mx) * (p.0 - mx)).sum();
        let sxy: f32 = ideal.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
        let ls = if sxx > 1e-6 { sxy / sxx } else { 0.0 };
        // Equal outer notes give a flat beam.
        let level = (stems[0].1 - stems[stems.len() - 1].1).abs() < 1e-4;
        // Concaveness (how far the inner notes reach towards the beam beyond
        // the line joining the outer ones) damps the slope further.
        let n = stems.len();
        let (first, last) = (stems[0].1 * dir, stems[n - 1].1 * dir);
        let excess: Vec<f32> = (1..n.saturating_sub(1)).map(|i| stems[i].1 * dir - (first + (last - first) * i as f32 / (n - 1) as f32)).collect();
        let concave = if excess.is_empty() { 0.0 } else { excess.iter().map(|e| e.max(0.0)).sum::<f32>() / excess.len() as f32 };
        let damped = if level { 0.0 } else { BEAM_SLOPE_DAMPING * ls.tanh() / (1.0 + BEAM_CONCAVE_DAMPING * concave) };
        let dx = stems[stems.len() - 1].0 - x0;
        let ideal_dy = damped * dx;
        // Candidate centre positions near the damped line through the centroid.
        let left0 = my - damped * mx;
        let quants = |c: f32| -> Vec<f32> {
            let mut q = Vec::new();
            let lo = (c - 2.0).floor() as i32;
            for k in lo..=lo + 5 {
                for off in [0.0, 0.19, 0.81] {
                    // The beam centre on a line, or sitting on / hanging from it.
                    q.push(k as f32 + off);
                }
            }
            q
        };
        let (mut best, mut best_score) = ((left0, left0 + ideal_dy), f32::INFINITY);
        for &l in &quants(left0) {
            for &r in &quants(left0 + ideal_dy) {
                let dy = r - l;
                let mut score = BEAM_SLOPE_FACTOR * (dy - ideal_dy).abs();
                if ideal_dy == 0.0 && dy != 0.0 {
                    score += BEAM_FLAT_PENALTY;
                } else if dy * ideal_dy < 0.0 {
                    score += BEAM_DIRECTION_PENALTY * dy.abs();
                }
                for &(x, h, b) in &stems {
                    let centre = l + if dx > 0.0 { dy * (x - x0) / dx } else { 0.0 };
                    let len = dir * (centre - h) + bt / 2.0;
                    let d = len - outer_len(b);
                    score += STEM_LENGTH_DEMERIT * if d < 0.0 { -d * BEAM_SHORT_FACTOR } else { d };
                    let free = len - (b - 1) as f32 * gap - bt;
                    if free < BEAMED_MIN_FREE[(b.min(3) - 1) as usize] {
                        score += 5000.0;
                    }
                }
                if score < best_score {
                    best_score = score;
                    best = (l, r);
                }
            }
        }
        let (l, r) = best;
        // Back to page coordinates: the outer edge, y down.
        let edge = |c: f32| mid - (c + dir * bt / 2.0);
        let slope = if dx > 0.0 { (edge(r) - edge(l)) / dx } else { 0.0 };
        (edge(l), slope)
    }

    fn draw_beams(&mut self, groups: &[Vec<usize>]) {
        let d = self.font.defaults;
        let (bt, gap) = (d.beam_thickness, d.beam_thickness + d.beam_spacing);
        for g in groups {
            let up = self.notes[g[0]].stem_up;
            let (fi, li) = (g[0], *g.last().unwrap());
            let (xf, xl) = (self.notes[fi].stem_x, self.notes[li].stem_x);
            let levels = g.iter().map(|&i| self.notes[i].beams()).max().unwrap_or(1);
            let (y0, slope) = self.quant_beam(g, up, bt, gap);
            let beam_y = |x: f32| y0 + slope * (x - xf);
            let beam_id = self.notes[fi].id;
            // Stems to the beam.
            for &i in g {
                let n = self.notes[i].clone();
                let (lo, hi) = (n.heads[0].pos, n.heads[n.heads.len() - 1].pos);
                let from = if n.stem_up { self.y(n.part, lo) - 0.168 } else { self.y(n.part, hi) + 0.168 };
                self.push(Item::line(n.stem_x, from, n.stem_x, beam_y(n.stem_x), d.stem_thickness, n.id));
            }
            let quad = |x0: f32, x1: f32, level: u32| {
                let off = level as f32 * gap * if up { 1.0 } else { -1.0 };
                let thick = if up { bt } else { -bt };
                let (a, b) = (beam_y(x0) + off, beam_y(x1) + off);
                Item::Quad { p: [pt(x0, a), pt(x1, b), pt(x1, b + thick), pt(x0, a + thick)], id: beam_id }
            };
            let half = d.stem_thickness / 2.0;
            self.push(quad(xf - half, xl + half, 0));
            // Secondary beams: runs of notes with at least `level + 1` beams; lone notes get a hook.
            for level in 1..levels {
                let mut k = 0;
                while k < g.len() {
                    if self.notes[g[k]].beams() <= level {
                        k += 1;
                        continue;
                    }
                    let s = k;
                    while k + 1 < g.len() && self.notes[g[k + 1]].beams() > level {
                        k += 1;
                    }
                    let (a, b) = (self.notes[g[s]].stem_x, self.notes[g[k]].stem_x);
                    if s == k {
                        let hook = 1.1;
                        let item = if s + 1 < g.len() && (s == 0 || self.notes[g[s]].value.dots == 0 && s == 0) {
                            quad(a - half, a + hook, level)
                        } else {
                            quad(a - hook, a + half, level)
                        };
                        self.push(item);
                    } else {
                        self.push(quad(a - half, b + half, level));
                    }
                    k += 1;
                }
            }
        }
    }

    fn draw_ties(&mut self) {
        let d = self.font.defaults;
        let system_end = self.ms.last().map(|m| m.x1).unwrap_or(self.left);
        let mut items = Vec::new();
        let tie = |x0: f32, x1: f32, y: f32, dir: f32, id: u64| {
            let x1 = x1.max(x0 + 0.6);
            let height = (0.35 + (x1 - x0) * 0.06).min(1.1);
            let c = |frac: f32, h: f32| pt(x0 + (x1 - x0) * frac, y + dir * h);
            Item::Crescent {
                from: pt(x0, y),
                c1: c(0.25, height + d.tie_midpoint_thickness),
                c2: c(0.75, height + d.tie_midpoint_thickness),
                to: pt(x1, y),
                d1: c(0.25, height),
                d2: c(0.75, height),
                id,
            }
        };
        for i in 0..self.notes.len() {
            let n = &self.notes[i];
            if n.rest {
                continue;
            }
            let side = |h: &Head| {
                let above = !n.stem_up || n.heads.len() > 1 && h.pos > n.heads[0].pos && h.pos >= n.heads[n.heads.len() - 1].pos;
                if above { -1.0 } else { 1.0 }
            };
            // The next piece of the same voice (in this system).
            let next = self.notes[i + 1..].iter().find(|o| o.part == n.part && o.voice == n.voice && o.start >= n.start + n.dur);
            for h in n.heads.iter().filter(|h| h.tie_out) {
                let dir = side(h);
                let y = self.y(n.part, h.pos) + dir * 0.35;
                let x0 = n.x + h.shift + HEAD_W + 0.1;
                match next {
                    Some(nx) if nx.start == n.start + n.dur => {
                        if let Some(h2) = nx.heads.iter().find(|x| x.dia == h.dia && x.alter == h.alter) {
                            items.push(tie(x0, nx.x + h2.shift - 0.1, y, dir, h.id));
                        }
                    }
                    // Continues in the next system: to the end of this one.
                    None if n.start + n.dur >= self.range.1 => items.push(tie(x0, system_end - 0.3, y, dir, h.id)),
                    _ => {}
                }
            }
            // Arriving from the previous system: from just before the note.
            let first_here = !self.notes[..i].iter().any(|o| o.part == n.part && o.voice == n.voice);
            if first_here && n.start == self.range.0 {
                for h in n.heads.iter().filter(|h| h.tied_in) {
                    let dir = side(h);
                    let y = self.y(n.part, h.pos) + dir * 0.35;
                    items.push(tie(n.x + h.shift - 2.2, n.x + h.shift - 0.1, y, dir, h.id));
                }
            }
        }
        self.out.items.extend(items);
    }

    fn draw_marks(&mut self) {
        for i in 0..self.notes.len() {
            let n = self.notes[i].clone();
            if !n.first || n.rest {
                continue;
            }
            let top = self.top(n.part);
            let center = |g: Glyph, s: &Self| n.x + HEAD_W / 2.0 - s.width(g) / 2.0;
            let poly = self.notes.iter().any(|o| o.part == n.part && o.measure == n.measure && o.voice != n.voice && !o.whole_bar);
            if !n.rest {
                // Close articulations on the notehead side away from the stem, in spaces;
                // in polyphony on the stem side, away from the other voice.
                let below = if poly { n.voice % 2 == 0 } else { n.stem_up };
                let outer = if below { n.heads[0].pos } else { n.heads[n.heads.len() - 1].pos };
                let mut pos = outer + if below { -2 } else { 2 };
                for (bit, up_g, down_g) in [
                    (art::STACCATO, Glyph::StaccatoAbove, Glyph::StaccatoBelow),
                    (art::STACCATISSIMO, Glyph::StaccatissimoAbove, Glyph::StaccatissimoBelow),
                    (art::TENUTO, Glyph::TenutoAbove, Glyph::TenutoBelow),
                    (art::ACCENT, Glyph::AccentAbove, Glyph::AccentBelow),
                    (art::MARCATO, Glyph::MarcatoAbove, Glyph::MarcatoBelow),
                ] {
                    if n.art & bit == 0 {
                        continue;
                    }
                    if poly && below == !n.stem_up && n.has_stem() {
                        // Stem side: beyond the stem end.
                        let end_pos = ((self.top(n.part) + 2.0 - self.notes[i].stem_end) * 2.0).round() as i32;
                        pos = if below { pos.min(end_pos - 1) } else { pos.max(end_pos + 1) };
                    }
                    if pos.abs() <= 4 && pos % 2 == 0 {
                        pos += if below { -1 } else { 1 }; // into the space
                    }
                    let g = if below { down_g } else { up_g };
                    let y = self.y(n.part, pos);
                    self.push(Item::glyph(g, center(g, self), y, n.id));
                    pos += if below { -2 } else { 2 };
                }
                if n.art & art::TREMOLO != 0 && n.has_stem() {
                    let g = Glyph::Tremolo3;
                    let sy = (self.notes[i].stem_end + if n.stem_up { 1.0 } else { -1.0 } + self.y(n.part, n.heads[0].pos)) / 2.0;
                    self.push(Item::glyph(g, n.stem_x - self.width(g) / 2.0, sy, n.id));
                }
                if n.art & art::ARPEGGIO != 0 && n.heads.len() > 1 {
                    let (ylo, yhi) = (self.y(n.part, n.heads[0].pos) + 0.5, self.y(n.part, n.heads[n.heads.len() - 1].pos) - 0.5);
                    let seg = self.font.bbox(Glyph::Arpeggiato).width().max(0.5);
                    // The wiggle segment is horizontal: stack it vertically via rotated segments is renderer
                    // work; draw it as a vertical column of short strokes instead.
                    let mut yy = ylo;
                    let x = n.x - 1.0 + n.heads.iter().map(|h| h.acc_x.min(0.0)).fold(0.0f32, f32::min);
                    while yy > yhi {
                        self.push(Item::line(x, yy, x + 0.25, yy - seg / 2.0, 0.14, n.id));
                        self.push(Item::line(x + 0.25, yy - seg / 2.0, x, yy - seg, 0.14, n.id));
                        yy -= seg;
                    }
                }
                if n.art & (art::BREATH | art::CAESURA) != 0 {
                    let g = if n.art & art::BREATH != 0 { Glyph::BreathMark } else { Glyph::Caesura };
                    let x = n.x + HEAD_W + 0.6 + n.value.dots as f32 * 0.5;
                    let y = if g == Glyph::BreathMark { top - 0.5 } else { top + 1.0 };
                    self.push(Item::glyph(g, x, y, n.id));
                }
            }
        }
    }
}

// ------------------------------------------------------- outer marks --
impl<'a> Engraver<'a> {
    /// x of a written position: its column, or interpolated between columns.
    fn col_x(&self, t: Frac) -> f32 {
        let q = t.num as f64 / t.den as f64 * 4.0;
        let cols = &self.out.columns;
        if let Some(c) = cols.iter().find(|c| (c.quarter - q).abs() < 1e-9) {
            return c.x;
        }
        let after = cols.iter().position(|c| c.quarter > q).unwrap_or(cols.len().saturating_sub(1));
        if after == 0 || cols.is_empty() {
            return cols.first().map(|c| c.x).unwrap_or(self.left);
        }
        let (a, b) = (cols[after - 1], cols[after]);
        a.x + ((q - a.quarter) / (b.quarter - a.quarter).max(1e-9)) as f32 * (b.x - a.x)
    }

    /// x of the barline at a written position (bar starts and the end).
    fn bar_x(&self, t: Frac) -> f32 {
        if t <= self.range.0 {
            return self.left;
        }
        for (i, m) in self.ms.iter().enumerate() {
            if m.start == t {
                return if i == 0 { self.left } else { self.ms[i - 1].x1 };
            }
        }
        self.ms.last().map(|m| m.x1).unwrap_or(self.left)
    }

    fn draw_clef_changes(&mut self) {
        for (part, at, clef) in self.clef_changes.clone() {
            let here: Vec<&Note> = self.notes.iter().filter(|n| n.part == part && n.start == at && !n.whole_bar).collect();
            if here.is_empty() {
                continue;
            }
            let left = here.iter().map(|n| self.note_extents(n).0).fold(0.0f32, f32::max);
            let x = here[0].x - left - 0.3 - self.clef_change_width();
            let (g, dy) = clef_glyph(&clef);
            let top = self.top(part);
            self.push(Item::glyph_scaled(g, x, top + dy, CLEF_CHANGE_SCALE, 0));
        }
    }

    /// Slurs from SLUR_START to the next SLUR_END in the voice: on the
    /// notehead side when all stems agree, else above; lifted over the notes
    /// in between.
    /// Slurs and hairpins of the whole movement, so a system can draw the
    /// part of one that started or ends in another system.
    fn spanners(&self) -> Vec<Spanner> {
        use corinzia_score::score::span;
        let mut out = Vec::new();
        for (pi, part) in self.score.parts.iter().enumerate() {
            for v in &part.voices {
                let notes: Vec<&corinzia_score::score::Event> = v.events.iter().filter(|e| !e.pitches.is_empty()).collect();
                for (k, e) in notes.iter().enumerate() {
                    let id = e.pitches.first().map(|p| p.id).unwrap_or(e.id);
                    if e.spans & span::SLUR_START != 0 {
                        if let Some(end) = notes[k + 1..].iter().find(|o| o.spans & span::SLUR_END != 0) {
                            out.push(Spanner { part: pi, voice: v.index, from: e.start, to: end.start, kind: SpanKind::Slur, id });
                        }
                    }
                    if e.spans & (span::CRESC_START | span::DIM_START) != 0 {
                        let kind = if e.spans & span::CRESC_START != 0 { SpanKind::Cresc } else { SpanKind::Dim };
                        let to = notes[k + 1..].iter().find(|o| o.dynamic.is_some() || o.spans & span::HAIRPIN_END != 0).map(|o| o.start);
                        // Without an end: to the end of the note's bar.
                        let to = to.unwrap_or_else(|| self.score_bar_end(e.start));
                        out.push(Spanner { part: pi, voice: v.index, from: e.start, to, kind, id });
                    }
                }
            }
        }
        out
    }

    /// The end of the bar containing `t` (whole movement).
    fn score_bar_end(&self, t: Frac) -> Frac {
        let mut at = Frac::ZERO;
        loop {
            let m = self.score.meter.iter().filter(|m| m.at <= at).last();
            let len = m.map(|m| Frac::new(m.num, m.den)).unwrap_or(Frac::int(1));
            if at + len > t || len.is_zero() {
                return (at + len).min(self.score.length.max(t));
            }
            at = at + len;
        }
    }

    /// The first piece of the event of (part, voice) starting at `t`, in this system.
    fn note_at(&self, part: usize, voice: u8, t: Frac) -> Option<usize> {
        self.notes.iter().position(|n| n.part == part && n.voice == voice && n.first && !n.rest && n.start == t)
    }

    /// Where a spanner cut by a system break starts and ends: just after the
    /// system's opening and at its final barline.
    fn system_edges(&self) -> (f32, f32) {
        (self.ms.first().map(|m| m.x0 + 0.5).unwrap_or(self.left), self.ms.last().map(|m| m.x1 - 0.3).unwrap_or(self.left))
    }

    fn draw_slurs(&mut self) {
        let d = self.font.defaults;
        let (r0, r1) = self.range;
        let (left, right) = self.system_edges();
        let mut items = Vec::new();
        for sp in self.spanners().into_iter().filter(|s| s.kind == SpanKind::Slur && s.from < r1 && s.to >= r0) {
            let si = if sp.from >= r0 { self.note_at(sp.part, sp.voice, sp.from) } else { None };
            let ei = if sp.to < r1 { self.note_at(sp.part, sp.voice, sp.to) } else { None };
            let between: Vec<&Note> = self
                .notes
                .iter()
                .filter(|o| o.part == sp.part && o.voice == sp.voice && !o.rest && o.start >= sp.from && o.start <= sp.to)
                .collect();
            if between.is_empty() {
                continue;
            }
            let poly = between.iter().any(|b| self.notes.iter().any(|o| o.part == sp.part && o.measure == b.measure && o.voice != sp.voice && !o.whole_bar));
            let above = if poly { sp.voice % 2 == 1 } else { !between.iter().all(|o| o.stem_up) };
            let dir = if above { -1.0 } else { 1.0 };
            // Endpoint: at the head on the head side, at the stem tip on the stem side.
            let end_y = |n: &Note, s: &Self| -> f32 {
                let (lo, hi) = (s.y(n.part, n.heads[0].pos), s.y(n.part, n.heads[n.heads.len() - 1].pos));
                if above {
                    if n.stem_up && n.has_stem() { n.stem_end - 0.5 } else { hi - 0.7 }
                } else if !n.stem_up && n.has_stem() {
                    n.stem_end + 0.5
                } else {
                    lo + 0.7
                }
            };
            // A cut end continues at the height of the music it leaves.
            let edge_y = |n: &Note, s: &Self| end_y(n, s) + dir * 0.6;
            let (x0, y0, x1, y1) = match (si, ei) {
                (Some(a), Some(b)) => {
                    let (a, b) = (&self.notes[a], &self.notes[b]);
                    (a.x + HEAD_W / 2.0 + 0.2, end_y(a, self), b.x + HEAD_W / 2.0 - 0.2, end_y(b, self))
                }
                (Some(a), None) => {
                    let a = &self.notes[a];
                    let last = between.last().unwrap();
                    (a.x + HEAD_W / 2.0 + 0.2, end_y(a, self), right, edge_y(last, self))
                }
                (None, Some(b)) => {
                    let b = &self.notes[b];
                    let first = between.first().unwrap();
                    (left, edge_y(first, self), b.x + HEAD_W / 2.0 - 0.2, end_y(b, self))
                }
                (None, None) => {
                    let y = between.iter().map(|n| edge_y(n, self)).fold(if above { f32::MAX } else { f32::MIN }, |a, b| if above { a.min(b) } else { a.max(b) });
                    (left, y, right, y)
                }
            };
            let dx = (x1 - x0).max(0.5);
            let mut h = (0.6 + 0.07 * dx).min(2.2);
            for o in &between {
                let t = ((o.x + HEAD_W / 2.0 - x0) / dx).clamp(0.05, 0.95);
                if t <= 0.06 || t >= 0.94 {
                    continue;
                }
                let line = y0 + (y1 - y0) * t;
                let extreme = end_y(o, self);
                let poke = (extreme - line) * dir + 0.4; // > 0: the note reaches past the chord
                if poke > 0.0 {
                    h = h.max(poke / (3.0 * t * (1.0 - t)));
                }
            }
            h = h.min(5.0);
            let c = |f: f32, hh: f32| pt(x0 + dx * f, y0 + (y1 - y0) * f + dir * hh);
            items.push(Item::Crescent {
                from: pt(x0, y0),
                c1: c(1.0 / 3.0, h + d.slur_midpoint_thickness),
                c2: c(2.0 / 3.0, h + d.slur_midpoint_thickness),
                to: pt(x1, y1),
                d1: c(1.0 / 3.0, h),
                d2: c(2.0 / 3.0, h),
                id: sp.id,
            });
        }
        self.out.items.extend(items);
    }

    /// Skylines from everything drawn so far; each item goes to the nearest staff.
    fn build_skylines(&mut self) {
        let mut sky: Vec<Skyline> = (0..self.score.parts.len()).map(|p| Skyline::new(self.top(p))).collect();
        for st in &self.out.staves {
            sky[st.part].add([st.left, st.top, st.right, st.top + 4.0]);
        }
        // Notes and what hangs off them belong to their part.
        let mut owner: std::collections::HashMap<u64, usize> = Default::default();
        for n in &self.notes {
            owner.insert(n.id, n.part);
            for h in &n.heads {
                owner.insert(h.id, n.part);
            }
        }
        for item in &self.out.items {
            if let Item::Text { anchor: 2, .. } = item {
                continue; // instrument names live left of the system
            }
            let r = item.bounds(self.font);
            if let Some(&p) = owner.get(&item.id()).filter(|_| item.id() != 0) {
                sky[p].add(r);
                continue;
            }
            // Barlines and brackets that join staves belong to none of them.
            if (0..sky.len()).filter(|&p| r[1] < self.top(p) + 4.0 && r[3] > self.top(p)).count() >= 2 {
                continue;
            }
            let cy = (r[1] + r[3]) / 2.0;
            if let Some(k) = (0..sky.len()).min_by(|&a, &b| {
                (cy - (sky[a].staff_top + 2.0)).abs().partial_cmp(&(cy - (sky[b].staff_top + 2.0)).abs()).unwrap()
            }) {
                sky[k].add(r);
            }
        }
        self.sky = sky;
    }

    /// Places `item` by its box against the part's skyline and records it.
    fn place(&mut self, part: usize, item: Item) {
        let r = item.bounds(self.font);
        self.sky[part].add(r);
        self.out.items.push(item);
    }

    /// Triplet groups: a number on the stem side (beamed) or a bracket with
    /// the number in a gap.
    fn draw_tuplets(&mut self) {
        let d = self.font.defaults;
        let mut groups: Vec<Vec<usize>> = Vec::new();
        let mut cur: Vec<usize> = Vec::new();
        let mut sum = Frac::ZERO;
        let mut min_plain = Frac::int(4);
        for i in 0..self.notes.len() {
            let n = &self.notes[i];
            let contiguous = cur.last().is_some_and(|&p| {
                let o = &self.notes[p];
                o.part == n.part && o.voice == n.voice && o.start + o.dur == n.start
            });
            if !n.value.triplet || !cur.is_empty() && !contiguous {
                cur.clear();
                sum = Frac::ZERO;
                min_plain = Frac::int(4);
                if !n.value.triplet {
                    continue;
                }
            }
            cur.push(i);
            sum = sum + n.dur;
            min_plain = min_plain.min(n.dur * Frac::new(3, 2));
            let pow2 = sum.num == 1 && sum.den & (sum.den - 1) == 0;
            if pow2 && sum >= min_plain * Frac::int(2) {
                groups.push(std::mem::take(&mut cur));
                sum = Frac::ZERO;
                min_plain = Frac::int(4);
            }
        }
        for g in groups {
            let (first, last) = (&self.notes[g[0]], &self.notes[*g.last().unwrap()]);
            let part = first.part;
            let above = g.iter().all(|&i| self.notes[i].stem_up || self.notes[i].rest);
            let beamed = g.iter().all(|&i| self.notes[i].beamed);
            let (x0, x1) = (first.x - 0.2, last.x + HEAD_W + 0.2);
            let num = Glyph::TupletDigit(3);
            let nb = self.font.bbox(num);
            let xm = (x0 + x1) / 2.0;
            let id = first.id;
            if beamed {
                let y = if above { self.sky[part].top_over(x0, x1) - 0.4 + nb.sw.1 } else { self.sky[part].bottom_over(x0, x1) + 0.4 + nb.ne.1 };
                self.place(part, Item::glyph(num, xm - nb.width() / 2.0, y, id));
            } else {
                let (line, hook) = if above {
                    (self.sky[part].top_over(x0, x1) - 1.0, 0.6)
                } else {
                    (self.sky[part].bottom_over(x0, x1) + 1.0, -0.6)
                };
                let gap = nb.width() / 2.0 + 0.3;
                let w = d.tuplet_bracket_thickness;
                self.place(part, Item::line(x0, line + hook, x0, line, w, id));
                self.place(part, Item::line(x0, line, xm - gap, line, w, id));
                self.place(part, Item::line(xm + gap, line, x1, line, w, id));
                self.place(part, Item::line(x1, line, x1, line + hook, w, id));
                self.place(part, Item::glyph(num, xm - nb.width() / 2.0, line + 0.75, id));
            }
        }
    }

    /// Fermatas and ornaments above the note, clear of everything there.
    fn draw_ornaments(&mut self) {
        for i in 0..self.notes.len() {
            let n = self.notes[i].clone();
            if !n.first || n.rest {
                continue;
            }
            for (bit, g) in [(art::FERMATA, Glyph::FermataAbove), (art::TRILL, Glyph::Trill), (art::MORDENT, Glyph::Mordent), (art::TURN, Glyph::Turn)] {
                if n.art & bit == 0 {
                    continue;
                }
                let b = self.font.bbox(g);
                let x = n.x + HEAD_W / 2.0 - b.width() / 2.0;
                let y = self.sky[n.part].top_over(x, x + b.width()) - 0.4 + b.sw.1;
                self.place(n.part, Item::glyph(g, x, y, n.id));
            }
        }
    }

    /// Technique text above (`above`), expression text below, in italics.
    fn draw_texts(&mut self, above: bool) {
        for (pi, part) in self.score.parts.iter().enumerate() {
            for t in part.texts.iter().filter(|t| t.technique == above && t.at >= self.range.0 && t.at < self.range.1) {
                let x = self.col_x(t.at);
                let w = text_width(&t.text, TEXT_SIZE, false);
                let y = if above {
                    self.sky[pi].top_over(x, x + w) - 0.4 - 0.22 * TEXT_SIZE
                } else {
                    self.sky[pi].bottom_over(x, x + w) + 0.4 + 0.72 * TEXT_SIZE
                };
                let item = Item::Text { text: t.text.clone(), at: pt(x, y), size: TEXT_SIZE, italic: !above, bold: false, anchor: 0, id: 0 };
                let r = item.bounds(self.font);
                self.sky[pi].add(r);
                self.out.items.push(item);
            }
        }
    }

    /// Chord symbols: text with Bravura's chord-symbol accidentals and
    /// qualities, on a common baseline above each staff of the system.
    fn draw_chord_symbols(&mut self) {
        for pi in 0..self.score.parts.len() {
            let chords: Vec<_> = self.score.parts[pi].chords.iter().filter(|c| c.at >= self.range.0 && c.at < self.range.1).cloned().collect();
            if chords.is_empty() {
                continue;
            }
            let runs: Vec<(f32, Vec<ChordRun>, u64)> = chords.iter().map(|c| (self.col_x(c.at), chord_runs(&c.text), c.id)).collect();
            let width = |r: &[ChordRun]| r.iter().map(|x| self.chord_run_width(x)).sum::<f32>();
            let top = runs.iter().map(|(x, r, _)| self.sky[pi].top_over(*x, x + width(r))).fold(self.top(pi), f32::min);
            let y = top - 0.8;
            for (x0, r, id) in runs {
                let mut x = x0;
                for run in r {
                    let w = self.chord_run_width(&run);
                    let item = match run {
                        ChordRun::Text(t) => Item::Text { text: t, at: pt(x, y), size: CHORD_SIZE, italic: false, bold: false, anchor: 0, id },
                        ChordRun::Glyph(g, centre) => {
                            // The glyph's box centred `centre` above the baseline.
                            let b = self.font.bbox(g);
                            let cy = (b.sw.1 + b.ne.1) / 2.0 * CHORD_GLYPH_SCALE;
                            Item::Glyph { glyph: g.codepoint(), at: pt(x + 0.05 - b.sw.0 * CHORD_GLYPH_SCALE, y - centre + cy), scale: CHORD_GLYPH_SCALE, id }
                        }
                    };
                    self.place(pi, item);
                    x += w;
                }
            }
        }
    }

    fn chord_run_width(&self, r: &ChordRun) -> f32 {
        match r {
            ChordRun::Text(t) => text_width(t, CHORD_SIZE, false),
            ChordRun::Glyph(g, _) => self.width(*g) * CHORD_GLYPH_SCALE + 0.15,
        }
    }

    /// Lyrics: one line per verse under each staff, below everything drawn
    /// so far in the system; syllables centred under their notes, hyphens
    /// centred between the syllables of a word, extenders through melismas.
    fn draw_lyrics(&mut self) {
        for pi in 0..self.score.parts.len() {
            let idx: Vec<usize> = (0..self.notes.len()).filter(|&i| self.notes[i].part == pi && !self.notes[i].lyrics.is_empty()).collect();
            let Some(verses) = idx.iter().flat_map(|&i| self.notes[i].lyrics.iter().map(|l| l.verse)).max() else { continue };
            let (x0, x1) = (self.left, self.out.width);
            let base = (self.sky[pi].bottom_over(x0, x1) + 0.8 + 0.75 * LYRIC_SIZE).max(self.top(pi) + 4.0 + 2.6);
            let mut boxes = Vec::new();
            for v in 1..=verses {
                let y = base + (v - 1) as f32 * LYRIC_SIZE * 1.2;
                let mut syl: Vec<(usize, corinzia_score::score::Lyric)> = idx.iter().filter_map(|&i| self.notes[i].lyrics.iter().find(|l| l.verse == v).map(|l| (i, l.clone()))).collect();
                syl.sort_by(|a, b| (self.notes[a.0].voice, self.notes[a.0].start).cmp(&(self.notes[b.0].voice, self.notes[b.0].start)));
                for (k, (i, l)) in syl.iter().enumerate() {
                    let n = &self.notes[*i];
                    let cx = n.x + HEAD_W / 2.0;
                    let w = text_width(&l.text, LYRIC_SIZE, false);
                    let right = cx + w / 2.0;
                    let item = Item::Text { text: l.text.clone(), at: pt(cx, y), size: LYRIC_SIZE, italic: false, bold: false, anchor: 1, id: n.heads.first().map(|h| h.id).unwrap_or(n.id) };
                    boxes.push(item.bounds(self.font));
                    self.out.items.push(item);
                    let next = syl.get(k + 1).filter(|(j, _)| self.notes[*j].voice == n.voice);
                    let next_left = next.map(|(j, nl)| self.notes[*j].x + HEAD_W / 2.0 - text_width(&nl.text, LYRIC_SIZE, false) / 2.0);
                    if l.hyphen {
                        // Centred in the gap; at the end of the system when the word goes on in the next one.
                        let hx = match next_left {
                            Some(nl) if nl - right > 0.7 => (right + nl) / 2.0,
                            Some(_) => continue,
                            None => right + 0.9,
                        };
                        self.out.items.push(Item::Text { text: "-".into(), at: pt(hx, y), size: LYRIC_SIZE, italic: false, bold: false, anchor: 1, id: 0 });
                    } else if l.extend {
                        // Through the following notes of the voice up to the next syllable.
                        let until = next.map(|(j, _)| self.notes[*j].start).unwrap_or(self.range.1);
                        let last = self.notes.iter().filter(|o| o.part == pi && o.voice == n.voice && !o.rest && o.start > n.start && o.start < until).map(|o| o.x + HEAD_W).fold(n.x + HEAD_W, f32::max);
                        if last > right + 0.5 {
                            self.out.items.push(Item::line(right + 0.25, y + 0.15, last, y + 0.15, 0.12, 0));
                        }
                    }
                }
            }
            for b in boxes {
                self.sky[pi].add(b);
            }
        }
    }

    fn dynamic_glyphs(text: &str) -> Vec<Glyph> {
        text.chars()
            .filter_map(|c| match c {
                'p' => Some(Glyph::DynamicP),
                'm' => Some(Glyph::DynamicM),
                'f' => Some(Glyph::DynamicF),
                _ => None,
            })
            .collect()
    }

    /// Hairpins and dynamics below the staff: a hairpin and the dynamics at
    /// its ends share one baseline; simultaneous dynamics stack.
    fn draw_dynamics_and_hairpins(&mut self) {
        let d = self.font.defaults;
        let dyn_width = |s: &Self, text: &str| Self::dynamic_glyphs(text).iter().map(|g| s.width(*g) - 0.05).sum::<f32>();
        let mut baseline_for: std::collections::HashMap<usize, f32> = Default::default();
        let (r0, r1) = self.range;
        let (left, right) = self.system_edges();
        for sp in self.spanners().into_iter().filter(|s| s.kind != SpanKind::Slur && s.from < r1 && s.to > r0) {
            let si = if sp.from >= r0 { self.note_at(sp.part, sp.voice, sp.from) } else { None };
            let ei = if sp.to < r1 { self.note_at(sp.part, sp.voice, sp.to) } else { None };
            let start_x = match si {
                Some(i) => {
                    let n = &self.notes[i];
                    n.x + n.dynamic.as_ref().map(|t| HEAD_W / 2.0 + dyn_width(self, t) / 2.0 + 0.5).unwrap_or(0.0)
                }
                None => left,
            };
            let end_x = match ei {
                Some(k) => {
                    let o = &self.notes[k];
                    match &o.dynamic {
                        Some(t) => o.x + HEAD_W / 2.0 - dyn_width(self, t) / 2.0 - 0.5,
                        None => o.x + HEAD_W,
                    }
                }
                // Continues in the next system (or ends at its bar's end).
                None if sp.to >= r1 => right,
                None => self.col_x(sp.to) - 0.5,
            };
            if end_x <= start_x + 0.5 {
                continue;
            }
            let hi = ei.map(|k| self.notes[k].x + 2.5).unwrap_or(end_x);
            let base = (self.sky[sp.part].bottom_over(start_x - 0.5, hi) + 2.0).max(self.top(sp.part) + 6.5);
            let c = base - 0.55;
            // The opening grows (or shrinks) with time, so the parts of a
            // hairpin cut by a system break continue each other.
            let total = sp.to - sp.from;
            let frac = |t: Frac| -> f32 {
                if total.is_zero() {
                    return 0.0;
                }
                let f = (t.max(sp.from).min(sp.to) - sp.from) * Frac::new(total.den, total.num);
                (f.num as f32 / f.den as f32).clamp(0.0, 1.0)
            };
            let open = |f: f32| if sp.kind == SpanKind::Cresc { 0.5 * f } else { 0.5 * (1.0 - f) };
            let (fa, fb) = (if si.is_some() { 0.0 } else { frac(r0) }, if ei.is_some() || sp.to < r1 { 1.0 } else { frac(r1) });
            let (a, b) = (open(fa), open(fb));
            let w = d.hairpin_thickness;
            self.place(sp.part, Item::line(start_x, c - a, end_x, c - b, w, sp.id));
            self.place(sp.part, Item::line(start_x, c + a, end_x, c + b, w, sp.id));
            if let Some(i) = si {
                baseline_for.insert(i, base);
            }
            if let Some(k) = ei {
                baseline_for.insert(k, base);
            }
        }
        for i in 0..self.notes.len() {
            let n = self.notes[i].clone();
            let Some(text) = n.dynamic.clone() else { continue };
            if !n.first {
                continue;
            }
            let w = dyn_width(self, &text);
            let mut x = n.x + HEAD_W / 2.0 - w / 2.0;
            let own = (self.sky[n.part].bottom_over(x, x + w) + 2.0).max(self.top(n.part) + 6.5);
            let y = baseline_for.get(&i).map_or(own, |&b| b.max(own));
            for g in Self::dynamic_glyphs(&text) {
                self.place(n.part, Item::glyph(g, x, y, n.id));
                x += self.width(g) - 0.05;
            }
        }
    }

    /// Above the top staff: volta brackets, rehearsal marks, tempo marks.
    fn draw_tempo_marks_endings(&mut self) {
        if self.sky.is_empty() {
            return;
        }
        let d = self.font.defaults;
        // Endings.
        let (r0, r1) = self.range;
        for e in self.score.endings.clone() {
            if e.to <= r0 || e.from >= r1 {
                continue;
            }
            let starts_here = e.from >= r0;
            let (x0, x1) = (self.bar_x(e.from.max(r0)) + if starts_here { 0.3 } else { 0.0 }, self.bar_x(e.to.min(r1)) - 0.3);
            if x1 <= x0 {
                continue;
            }
            let label: Vec<String> = e.numbers.iter().map(|n| format!("{n}.")).collect();
            let label = label.join(", ");
            let lw = text_width(&label, 1.6, false);
            // The line clears everything under it; the label (hanging below the
            // line at the start) clears what is under the label.
            let y = (self.sky[0].top_over(x0, x1) - 0.8)
                .min(self.sky[0].top_over(x0, x0 + lw + 0.8) - 2.2)
                .min(self.top(0) - 2.5);
            let w = d.repeat_ending_line_thickness;
            let closed = e.to <= r1 && self.score.barlines.iter().any(|b| b.at == e.to && matches!(b.kind, BarKind::RepeatEnd | BarKind::RepeatBoth));
            if starts_here {
                self.place(0, Item::line(x0, y + 2.0, x0, y, w, 0));
            }
            self.place(0, Item::line(x0, y, x1, y, w, 0));
            if closed {
                self.place(0, Item::line(x1, y, x1, y + 2.0, w, 0));
            }
            if starts_here {
                self.place(0, Item::Text { text: label, at: pt(x0 + 0.4, y + 1.6), size: 1.6, italic: false, bold: false, anchor: 0, id: 0 });
            }
        }
        // Rehearsal marks: boxed letters at their barline.
        let mut marks = self.score.marks.clone();
        marks.sort();
        for (k, at) in marks.iter().enumerate().filter(|(_, at)| **at >= r0 && **at < r1) {
            let letter: String = std::iter::repeat_n((b'A' + (k % 26) as u8) as char, k / 26 + 1).collect();
            let size = 2.4;
            let w = text_width(&letter, size, true);
            let x = self.bar_x(*at);
            let (bx0, bx1) = (x - w / 2.0 - 0.4, x + w / 2.0 + 0.4);
            let bottom = self.sky[0].top_over(bx0, bx1) - 0.6;
            let top = bottom - 0.72 * size - 0.8;
            let t = 0.16;
            for (a, b) in [((bx0, top), (bx1, top)), ((bx1, top), (bx1, bottom)), ((bx1, bottom), (bx0, bottom)), ((bx0, bottom), (bx0, top))] {
                self.place(0, Item::line(a.0, a.1, b.0, b.1, t, 0));
            }
            self.place(0, Item::Text { text: letter, at: pt(x, bottom - 0.4), size, italic: false, bold: true, anchor: 1, id: 0 });
        }
        // Tempo marks: words (bold) and a metronome mark; repeated only when they change.
        let mut tempo = self.score.tempo.clone();
        tempo.sort_by(|a, b| a.at.cmp(&b.at));
        let mut printed = -1i64;
        for t in tempo {
            if t.at < r0 || t.at >= r1 {
                if t.at < r0 {
                    printed = t.bpm.round() as i64;
                }
                continue;
            }
            let bpm = t.bpm.round() as i64;
            if t.text.is_empty() && bpm == printed {
                continue;
            }
            printed = bpm;
            let mut x = if t.at == Frac::ZERO { self.left + 1.0 } else { self.col_x(t.at) - 0.5 };
            let words = if t.text.is_empty() { String::new() } else { format!("{} (", t.text) };
            let tail = format!(" = {bpm}{}", if t.text.is_empty() { "" } else { ")" });
            let total = text_width(&words, TEMPO_SIZE, true) + 1.0 + text_width(&tail, TEMPO_SIZE, false);
            let y = (self.sky[0].top_over(x, x + total) - 0.6).min(self.top(0) - 2.0);
            if !words.is_empty() {
                let item = Item::Text { text: words.clone(), at: pt(x, y), size: TEMPO_SIZE, italic: false, bold: true, anchor: 0, id: 0 };
                x += text_width(&words, TEMPO_SIZE, true) + 0.35;
                self.place(0, item);
            }
            self.place(0, Item::glyph_scaled(Glyph::MetNoteQuarterUp, x, y - 0.1, 0.6, 0));
            x += 0.9;
            self.place(0, Item::Text { text: tail, at: pt(x, y), size: TEMPO_SIZE, italic: false, bold: false, anchor: 0, id: 0 });
        }
    }
}

const CHORD_GLYPH_SCALE: f32 = 0.55;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SpanKind {
    Slur,
    Cresc,
    Dim,
}

/// A slur or hairpin: from its start event to the event that ends it.
#[derive(Clone, Copy, Debug)]
struct Spanner {
    part: usize,
    voice: u8,
    from: Frac,
    to: Frac,
    kind: SpanKind,
    id: u64,
}

/// A chord symbol cut into text and SMuFL chord-symbol glyphs: accidentals
/// (♯ ♭ ♮, or # and b after the root/bass letter and before a degree) and
/// the qualities ° ø Δ.
#[derive(Clone, Debug, PartialEq)]
enum ChordRun {
    Text(String),
    Glyph(Glyph, f32), // centre height above the baseline, staff spaces
}

fn chord_runs(text: &str) -> Vec<ChordRun> {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<ChordRun> = Vec::new();
    let mut buf = String::new();
    let flush = |buf: &mut String, out: &mut Vec<ChordRun>| {
        if !buf.is_empty() {
            out.push(ChordRun::Text(std::mem::take(buf)));
        }
    };
    for (i, &c) in chars.iter().enumerate() {
        let prev = if i > 0 { Some(chars[i - 1]) } else { None };
        let next = chars.get(i + 1).copied();
        let after_letter = prev.is_some_and(|p| ('A'..='G').contains(&p));
        let before_digit = next.is_some_and(|n| n.is_ascii_digit());
        let glyph = match c {
            // Accidentals centred on the capitals; qualities raised like exponents.
            '♯' => Some((Glyph::CsymSharp, 1.0)),
            '♭' => Some((Glyph::CsymFlat, 0.9)),
            '♮' => Some((Glyph::CsymNatural, 1.0)),
            '#' if after_letter || before_digit => Some((Glyph::CsymSharp, 1.0)),
            'b' if after_letter || before_digit => Some((Glyph::CsymFlat, 0.9)),
            '°' => Some((Glyph::CsymDiminished, 1.35)),
            'ø' => Some((Glyph::CsymHalfDiminished, 1.35)),
            'Δ' => Some((Glyph::CsymMajorSeventh, 1.2)),
            _ => None,
        };
        match glyph {
            Some((g, raise)) => {
                flush(&mut buf, &mut out);
                out.push(ChordRun::Glyph(g, raise));
            }
            None => buf.push(c),
        }
    }
    flush(&mut buf, &mut out);
    out
}

fn rest_glyph(base: i64) -> Glyph {
    match base {
        0 | 1 => Glyph::RestWhole,
        2 => Glyph::RestHalf,
        4 => Glyph::RestQuarter,
        8 => Glyph::Rest8th,
        16 => Glyph::Rest16th,
        32 => Glyph::Rest32nd,
        _ => Glyph::Rest64th,
    }
}

fn flag_glyph(base: i64, up: bool) -> Glyph {
    match (base, up) {
        (8, true) => Glyph::Flag8thUp,
        (8, false) => Glyph::Flag8thDown,
        (16, true) => Glyph::Flag16thUp,
        (16, false) => Glyph::Flag16thDown,
        (32, true) => Glyph::Flag32ndUp,
        (32, false) => Glyph::Flag32ndDown,
        (_, true) => Glyph::Flag64thUp,
        (_, false) => Glyph::Flag64thDown,
    }
}

impl<'a> Engraver<'a> {
    fn new(score: &'a Score, opt: &Options, ms: Vec<Measure>, notes: Vec<Note>, first_system: bool, width: Option<f32>) -> Self {
        let range = (
            ms.first().map(|m| m.start).unwrap_or(Frac::ZERO),
            ms.last().map(|m| m.start + m.len).unwrap_or(Frac::ZERO),
        );
        let name = |p: &Part| if first_system || p.abbr.is_empty() { p.name.clone() } else { p.abbr.clone() };
        let names_w = if opt.names { score.parts.iter().map(|p| text_width(&name(p), NAME_SIZE, false)).fold(0.0f32, f32::max) } else { 0.0 };
        let brace_w = if score.parts.iter().any(|p| p.brace) { BRACE_WIDTH + BRACE_GAP } else { 0.0 };
        let left = opt.margin + if names_w > 0.0 { names_w + 1.5 } else { 0.0 } + brace_w;
        let mut clef_changes: Vec<(usize, Frac, String)> = score
            .parts
            .iter()
            .enumerate()
            .flat_map(|(pi, p)| p.clefs.iter().filter(|c| c.at > range.0 && c.at < range.1).map(move |c| (pi, c.at, c.clef.clone())))
            .collect();
        clef_changes.sort_by(|a, b| a.1.cmp(&b.1));
        Engraver {
            score,
            font: Font::bravura(),
            opt: opt.clone(),
            ms,
            notes,
            out: DisplayList::default(),
            left,
            clef_changes,
            sky: Vec::new(),
            first_system,
            width,
            range,
            tops: (0..score.parts.len()).map(|p| p as f32 * opt.staff_distance).collect(),
        }
    }

    /// Everything after `prepare`: spacing and drawing.
    /// The display list and each staff's skyline.
    fn engrave(mut self) -> (DisplayList, Vec<Skyline>) {
        let groups = self.beam_groups();
        self.space();
        self.draw_staves();
        self.draw_measure_furniture();
        self.draw_clef_changes();
        self.draw_notes();
        self.draw_beams(&groups);
        self.draw_ties();
        self.draw_marks();
        self.draw_slurs();
        self.build_skylines();
        self.draw_tuplets();
        self.draw_ornaments();
        self.draw_texts(true);
        self.draw_chord_symbols();
        self.draw_dynamics_and_hairpins();
        self.draw_texts(false);
        self.draw_lyrics();
        self.draw_tempo_marks_endings();
        let staves = self.score.parts.len().max(1);
        self.out.height = self.top(staves - 1) + 4.0;
        (self.out, self.sky)
    }
}

/// Notes split and prepared (stems, accidentals) for the whole movement.
fn prepared(score: &Score, opt: &Options) -> (Vec<Measure>, Vec<Note>) {
    let mut ms = measures(score);
    let notes = split(score, &ms);
    let shortest = common_shortest(&ms, &notes);
    for m in ms.iter_mut() {
        m.shortest = shortest;
    }
    let mut e = Engraver::new(score, opt, ms, notes, true, None);
    e.prepare();
    (e.ms, e.notes)
}

/// Lays the whole movement out on one system.
pub fn layout_line(score: &Score, opt: &Options) -> DisplayList {
    let (ms, notes) = prepared(score, opt);
    engrave_spaced(score, opt, ms, notes, true, None)
}

/// Lays the whole movement out on one system justified to `width` (staff
/// spaces, margins included).
pub fn layout_line_at(score: &Score, opt: &Options, width: f32) -> DisplayList {
    let (ms, notes) = prepared(score, opt);
    engrave_spaced(score, opt, ms, notes, true, Some(width))
}

/// LilyPond's `staff-staff-spacing` padding: the least room between one
/// staff's music and the next staff's, where they overlap horizontally.
const STAFF_PADDING: f32 = 1.0;

/// Engraves, then, where the music of neighbouring staves would come closer
/// than `STAFF_PADDING` (lyrics, chord symbols, high or low notes), moves
/// the staves apart and engraves again. As in LilyPond, the skylines are
/// compared column by column, and staves never come closer than
/// `staff_distance` (`basic-distance`).
fn engrave_spaced(score: &Score, opt: &Options, ms: Vec<Measure>, notes: Vec<Note>, first: bool, width: Option<f32>) -> DisplayList {
    let (list, sky) = Engraver::new(score, opt, ms.clone(), notes.clone(), first, width).engrave();
    let mut tops = vec![0.0f32; sky.len()];
    for p in 1..sky.len() {
        let need = sky[p - 1].clearance_above(&sky[p], STAFF_PADDING);
        tops[p] = tops[p - 1] + need.max(opt.staff_distance);
    }
    if tops.iter().enumerate().all(|(p, t)| (t - p as f32 * opt.staff_distance).abs() < 0.01) {
        return list;
    }
    let mut e = Engraver::new(score, opt, ms, notes, first, width);
    e.tops = tops;
    e.engrave().0
}

/// One system of bars [a, b) from prepared notes, optionally justified.
fn system(score: &Score, opt: &Options, ms: &[Measure], notes: &[Note], a: usize, b: usize, width: Option<f32>) -> DisplayList {
    let mut sm: Vec<Measure> = ms[a..b].to_vec();
    for m in sm.iter_mut() {
        m.x0 = 0.0;
        m.x1 = 0.0;
    }
    if a > 0 {
        // Each system restates the key; the time only where it changes.
        let fifths = key_at(score, sm[0].start);
        sm[0].key = Some(sm[0].key.unwrap_or(Key { at: sm[0].start, fifths, minor: false }));
    }
    let sn: Vec<Note> = notes
        .iter()
        .filter(|n| n.measure >= a && n.measure < b)
        .cloned()
        .map(|mut n| {
            n.measure -= a;
            n
        })
        .collect();
    engrave_spaced(score, opt, sm, sn, a == 0, width)
}

/// Page geometry in staff spaces (A4 at an 18 pt staff: 132.3 × 187.1).
#[derive(Clone, Debug)]
pub struct PageSpec {
    pub width: f32,
    pub height: f32,
    pub margin_x: f32,
    pub margin_y: f32,
    /// Space between systems (between their outermost marks).
    pub system_gap: f32,
}
impl Default for PageSpec {
    fn default() -> Self {
        PageSpec { width: 132.3, height: 187.1, margin_x: 8.0, margin_y: 8.0, system_gap: 4.0 }
    }
}

/// System breaks by dynamic programming over bar boundaries, as LilyPond's
/// optimal breaker does: each line costs the square of its force (the room
/// it must gain or give up, over its stretchability). Lines may be
/// compressed down to `MAX_COMPRESSION`; a bar wider than the line gets a
/// system to itself. Like LilyPond (`ragged-last = ##f`), the last line is
/// justified too.
fn break_systems(natural: &[f32], stretch: &[f32], prefix: impl Fn(usize) -> f32, line: f32) -> Vec<(usize, usize)> {
    let n = natural.len();
    let mut best = vec![f32::INFINITY; n + 1];
    let mut from = vec![0usize; n + 1];
    best[0] = 0.0;
    for b in 1..=n {
        let (mut sum, mut k) = (0.0, 0.0);
        for a in (0..b).rev() {
            sum += natural[a];
            k += stretch[a];
            let width = prefix(a) + sum;
            let force = (line - width) / k.max(1e-3);
            if force < MAX_COMPRESSION && b - a > 1 {
                break;
            }
            let cost = force * force + LINE_PENALTY;
            if best[a] + cost < best[b] {
                best[b] = best[a] + cost;
                from[b] = a;
            }
        }
    }
    let mut out = Vec::new();
    let mut b = n;
    while b > 0 {
        out.push((from[b], b));
        b = from[b];
    }
    out.reverse();
    out
}

/// The movement on pages: each page a display list in page coordinates.
pub fn layout_pages(score: &Score, opt: &Options, page: &PageSpec) -> Vec<DisplayList> {
    let (ms, notes) = prepared(score, opt);
    if ms.is_empty() {
        return vec![];
    }
    // System coordinates start at the engraver's own margin, which lands on
    // the page margin: staves may run to x = line (+ that margin).
    let line = page.width - 2.0 * page.margin_x + opt.margin;
    // Natural bar widths from one unjustified line.
    let natural_line = {
        let mut e = Engraver::new(score, opt, ms.clone(), notes.clone(), true, None);
        e.beam_groups();
        e.space();
        e
    };
    let natural: Vec<f32> = (0..ms.len())
        .map(|i| natural_line.ms[i].x1 - if i == 0 { natural_line.ms[0].x0 } else { natural_line.ms[i - 1].x1 })
        .collect();
    let first_prefix = natural_line.ms[0].x0;
    let later_prefix = {
        let e = Engraver::new(score, opt, vec![], vec![], false, None);
        let clef_w = score.parts.iter().map(|p| e.width(clef_glyph(&p.clef).0)).fold(0.0f32, f32::max);
        e.left + 0.8 + clef_w + 0.8 + 3.0
    };
    let stretch: Vec<f32> = natural_line.ms.iter().map(|m| m.stretch).collect();
    let systems = break_systems(&natural, &stretch, |a| if a == 0 { first_prefix } else { later_prefix }, line);

    // Lay out each system, then stack them on pages by their real extents.
    let mut pages: Vec<DisplayList> = Vec::new();
    let new_page = || DisplayList { width: page.width, height: page.height, ..Default::default() };
    let mut cur = new_page();
    let mut y = page.margin_y;
    if !score.title.is_empty() {
        cur.items.push(Item::Text { text: score.title.clone(), at: pt(page.width / 2.0, y + 3.0), size: 3.2, italic: false, bold: true, anchor: 1, id: 0 });
        y += 7.0;
    }
    for &(a, b) in &systems {
        let sys = system(score, opt, &ms, &notes, a, b, Some(line + opt.margin));
        let font = Font::bravura();
        let (mut y0, mut y1) = (0.0f32, sys.height);
        for it in &sys.items {
            let r = it.bounds(font);
            y0 = y0.min(r[1]);
            y1 = y1.max(r[3]);
        }
        let mut offset = y - y0;
        if offset + y1 > page.height - page.margin_y && !cur.staves.is_empty() {
            pages.push(std::mem::replace(&mut cur, new_page()));
            y = page.margin_y;
            offset = y - y0;
        }
        let (dx, dy) = (page.margin_x - opt.margin, offset);
        let sys_index = cur.staves.last().map(|s| s.system + 1).unwrap_or(0);
        cur.items.extend(sys.items.into_iter().map(|i| i.translated(dx, dy)));
        cur.staves.extend(sys.staves.into_iter().map(|mut st| {
            st.top += dy;
            st.left += dx;
            st.right += dx;
            st.system = sys_index;
            st
        }));
        cur.columns.extend(sys.columns.into_iter().map(|mut c| {
            c.x += dx;
            c.system = sys_index;
            c
        }));
        y = offset + y1 + page.system_gap;
    }
    pages.push(cur);
    pages
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_signature_tables() {
        assert_eq!(key_alter(1, 3), 1); // G major: F#
        assert_eq!(key_alter(-3, 5), -1); // E-flat major: A-flat
        assert_eq!(key_alter(-3, 1), 0);
        // Bass clef sharps sit a third... one step lower than treble (F3 on the 4th line: +2).
        assert_eq!(key_positions(1, "bass")[0].0, 2);
        assert_eq!(key_positions(1, "treble")[0].0, 4);
        assert_eq!(key_positions(-1, "alto")[0].0, -1);
    }

    #[test]
    fn spacing_follows_lilypond_defaults() {
        let s = Frac::new(1, 8);
        let (e, q, h) = (ideal_space(Frac::new(1, 8), s), ideal_space(Frac::new(1, 4), s), ideal_space(Frac::new(1, 2), s));
        assert!((e - 2.4).abs() < 1e-4 && (q - e - 1.2).abs() < 1e-4 && (h - q - 1.2).abs() < 1e-4);
    }
}
