//! The engraver: a score into a display list, on one endless line (the
//! editor's Panorama view, [`layout_line`]) or on pages ([`layout_pages`]).
//!
//! Pipeline: split events at barlines into notatable pieces (`music`) -> per
//! measure, rhythmic columns shared by all staves -> x positions (`spacing`)
//! -> the staves, notes and marks into a display list -> systems on pages
//! (`pages`).
//!
//! Here: the options, the data the modules share (measures, notes,
//! spanners) and the [`Engraver`]'s core: preparing the notes, the order
//! things are drawn in, and the skylines that keep marks apart. The drawing
//! is in the modules, one per kind of thing: `staff`, `notes`, `beams`,
//! `ties`, `scripts`, `spanners` (slurs), `tuplets`, `dynamics`, `text`.

mod music;
mod spacing;
mod staff;
mod notes;
mod beams;
mod ties;
mod scripts;
mod spanners;
mod tuplets;
mod dynamics;
mod text;
mod pages;

use music::*;
use spacing::*;
use staff::*;
use notes::*;
use text::*;
use pages::*;
pub use pages::{layout_pages, PageSpec};

use crate::display::{pt, text_width, Column, DisplayList, Item, StaffGeom};
use crate::skyline::Skyline;
use crate::font::{Font, Glyph};
use corinzia_score::score::{art, ids, span, BarKind, Key, LineKind, Nav, Part, Pitch, Score};
use corinzia_score::values::{decompose, decompose_rest, NoteValue};
use corinzia_score::beams::beam_beat;
use corinzia_score::Frac;

#[derive(Clone, Debug)]
pub struct Options {
    /// Least distance between adjacent staves (top line to top line), in
    /// staff spaces: LilyPond's `basic-distance` of `staff-staff-spacing`.
    pub staff_distance: f32,
    pub margin: f32,
    /// Instrument names left of the system.
    pub names: bool,
    /// Runs of empty bars as one multi-bar rest (parts).
    pub multirest: bool,
    /// Note spacing, 1.0 as LilyPond's.
    pub spacing: f32,
}
impl Default for Options {
    fn default() -> Self {
        Options { staff_distance: 9.0, margin: 1.0, names: true, multirest: false, spacing: 1.0 }
    }
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
    squeeze: f32,     // how far they give when compressed, rods kept
    phase: Frac,      // a pickup's missing beginning (beats count from it)
    number: usize,    // the bar number (a pickup is bar 0)
    multi: usize,     // bars of rest it stands for (a multi-bar rest when > 1)
    beams: [u8; 8],   // the meter's beam groups (zeros: its default)
}

// ------------------------------------------------------------------- pieces --
/// A grace note before a note: small heads on one stem.
#[derive(Clone, Debug)]
struct GraceGeom {
    id: u64,
    heads: Vec<Head>, // bottom to top
    base: u8,
    slash: bool,
}

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
    shape: u8,  // 0 normal; percussion: 1 x, 2 circled x, 3 diamond, 4 triangle
}

#[derive(Clone, Debug)]
struct Note {
    part: usize, // the staff it is drawn on
    home: usize, // the staff whose voice it belongs to (differs when cross-staff)
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
    tuplets: Vec<usize>, // the explicit tuplets around it (`tuplet_list`), outermost first
    graces: Vec<GraceGeom>, // grace notes before it (first piece only)
    beam: u8,               // `span::BEAMS` bits set by hand (first piece only)
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
    beam_of: Vec<usize>,                     // per note: its beam group + 1 (0: none)
}

const HEAD_W: f32 = 1.18;

/// A chord's accidentals placed as LilyPond's AccidentalPlacement places
/// them (`lily/accidental-placement.cc`; GNU LilyPond, GPL-3.0-or-later):
/// those on one note name (its octaves) together in one column; the
/// columns with most accidentals first, then from the outside in (the
/// highest, the lowest, the next highest…), each as far right as it
/// clears what is placed already, 0.2 apart and 0.15 from the heads.
/// Each glyph fitted by its outline (24 bands), as LilyPond fits them by
/// their skylines.
/// A box (x0, x1, y0, y1).
type AccBox = (f32, f32, f32, f32);

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
    /// Where a slur ends (another voice or staff, or the same).
    to_part: usize,
    to_voice: u8,
    from: Frac,
    to: Frac,
    kind: SpanKind,
    id: u64,
    /// A slur's side chosen by hand.
    dir: i8,
}

/// A chord symbol cut into text and SMuFL chord-symbol glyphs: accidentals
/// (♯ ♭ ♮, or # and b after the root/bass letter and before a degree) and
/// the qualities ° ø Δ.
#[derive(Clone, Debug, PartialEq)]
enum ChordRun {
    Text(String),
    Glyph(Glyph, f32), // centre height above the baseline, staff spaces
}

impl<'a> Engraver<'a> {
    fn new(score: &'a Score, opt: &Options, ms: Vec<Measure>, notes: Vec<Note>, first_system: bool, width: Option<f32>) -> Self {
        let range = (
            ms.first().map(|m| m.start).unwrap_or(Frac::ZERO),
            ms.last().map(|m| m.start + m.len).unwrap_or(Frac::ZERO),
        );
        let name = |p: &Part| if first_system || p.abbr.is_empty() { p.name.clone() } else { p.abbr.clone() };
        let names_w = if opt.names { score.parts.iter().map(|p| text_width(&name(p), NAME_SIZE, false)).fold(0.0f32, f32::max) } else { 0.0 };
        let left = opt.margin + if names_w > 0.0 { names_w + 1.5 } else { 0.0 } + decoration_width(score);
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
            beam_of: vec![],
        }
    }

    /// Everything after `prepare`: spacing and drawing.
    /// The display list and each staff's skyline.
    fn engrave(mut self) -> (DisplayList, Vec<Skyline>) {
        let groups = self.beam_groups();
        self.beam_of = vec![0; self.notes.len()];
        for (k, g) in groups.iter().enumerate() {
            for &i in g {
                self.beam_of[i] = k + 1;
            }
        }
        self.space();
        self.draw_staves();
        self.draw_measure_furniture();
        self.draw_clef_changes();
        self.draw_notes();
        self.draw_graces();
        self.draw_beams(&groups);
        self.draw_ties();
        self.draw_marks();
        self.draw_slurs();
        self.draw_tabs();
        self.build_skylines();
        self.draw_tuplets();
        self.draw_fingering();
        self.draw_ornaments();
        self.draw_lines(false);
        self.draw_texts(true);
        self.draw_chord_symbols();
        self.draw_dynamics_and_hairpins();
        self.draw_lines(true);
        self.draw_texts(false);
        self.draw_lyrics();
        self.draw_tempo_marks_endings();
        let staves = self.score.parts.len().max(1);
        self.out.height = self.top(staves - 1) + 4.0;
        (self.out, self.sky)
    }

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
        // Percussion staves have neither key nor accidentals.
        let percussion: Vec<bool> = self.score.parts.iter().map(|p| p.clef == "percussion").collect();
        for &i in &order {
            let (part, measure, start) = (self.notes[i].part, self.notes[i].measure, self.notes[i].start);
            if last_start.is_some_and(|(p, s)| p != part || s != start) {
                tied_into.extend(next_tied.drain(..));
            }
            last_start = Some((part, start));
            let fifths = part_key(self.score, part, key_at(self.score, start), start);
            let voice = self.notes[i].voice;
            // Grace notes come first and count for the bar.
            for g in self.notes[i].graces.iter_mut() {
                for h in g.heads.iter_mut() {
                    let key = (part, measure, h.dia);
                    let current = *state.get(&key).unwrap_or(&key_alter(fifths, h.step));
                    if h.alter != current {
                        h.accidental = Some(accidental_glyph(h.alter));
                    }
                    state.insert(key, h.alter);
                }
            }
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
        for note in self.notes.iter_mut().filter(|n| percussion.get(n.part).copied().unwrap_or(false)) {
            note.heads.iter_mut().for_each(|h| h.accidental = None);
            note.graces.iter_mut().flat_map(|g| g.heads.iter_mut()).for_each(|h| h.accidental = None);
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
            place_accidentals(note);
        }
    }

    /// Skylines from everything drawn so far; each item goes to the nearest staff.
    fn build_skylines(&mut self) {
        let mut sky: Vec<Skyline> = (0..self.score.parts.len()).map(|p| Skyline::new(self.top(p))).collect();
        for st in &self.out.staves {
            sky[st.part].add_staff([st.left, st.top, st.right, st.top + 4.0]);
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
                // Barlines and clefs within the staff are the staff's own:
                // dynamics keep the staff's 0.1 from them, not the notes' 0.6.
                let inside = r[1] >= sky[k].staff_top - 0.2 && r[3] <= sky[k].staff_top + 4.2;
                if item.id() == 0 && inside {
                    sky[k].add_staff(r);
                } else {
                    sky[k].add(r);
                }
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

#[cfg(test)]
mod dot_tests {
    #[test]
    fn chord_dots_as_lilypond_sets_them() {
        use super::dot_rows;
        // A line note's dot goes up; a voice 2 note's down.
        assert_eq!(dot_rows(&[-2], 0), vec![-1]);
        assert_eq!(dot_rows(&[-2], -1), vec![-3]);
        // c' e' g' (lines -6 -4 -2): a dot each, all in spaces.
        assert_eq!(dot_rows(&[-6, -4, -2], 0), vec![-5, -3, -1]);
        // A second: e' f' a' share no space, so one moves.
        let r = dot_rows(&[-4, -3, -1], 0);
        assert_eq!(r.len(), 3);
        assert!(r.iter().all(|p| p % 2 != 0), "{r:?}");
    }
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
        // Shorter than the shortest: linear (half the ratio below, not a whole increment).
        let sixteenth = ideal_space(Frac::new(1, 16), s);
        assert!((sixteenth - 1.2 * 1.5).abs() < 1e-4, "{sixteenth}");
    }
}
