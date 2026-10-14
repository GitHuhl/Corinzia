//! Single-line engraving: the whole movement on one endless system (the
//! editor's Panorama view). Page breaking builds on this later.
//!
//! Pipeline: split events at barlines into notatable pieces -> per measure,
//! rhythmic columns shared by all staves -> x positions (logarithmic spacing,
//! widened where marks need room) -> marks into a display list.

use crate::display::{pt, Column, DisplayList, Item, StaffGeom};
use crate::font::{Font, Glyph};
use corinzia_score::score::{art, BarKind, Key, Part, Pitch, Score};
use corinzia_score::values::{decompose, NoteValue};
use corinzia_score::Frac;

#[derive(Clone, Debug)]
pub struct Options {
    /// Distance between the top lines of adjacent staves, in staff spaces.
    pub staff_distance: f32,
    pub margin: f32,
}
impl Default for Options {
    fn default() -> Self {
        Options { staff_distance: 11.0, margin: 1.0 }
    }
}

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

/// Ideal space after a note of duration `d` (whole notes): logarithmic, so
/// each doubling adds a constant (≈ the proportions of engraved scores).
fn ideal_space(d: Frac) -> f32 {
    let w = d.num as f64 / d.den as f64;
    (1.8 + 1.6 * (1.0 + w * 16.0).log2()) as f32
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
        out.push(Measure { start: at, len, num, den, show_time, key: key_changed, x0: 0.0, x1: 0.0 });
        at = at + len;
    }
    out
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
}

const HEAD_W: f32 = 1.18;

impl<'a> Engraver<'a> {
    fn top(&self, part: usize) -> f32 {
        part as f32 * self.opt.staff_distance
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
            let fifths = key_at(self.score, start);
            let voice = self.notes[i].voice;
            for h in self.notes[i].heads.iter_mut() {
                let key = (part, measure, h.dia);
                let current = *state.get(&key).unwrap_or(&key_alter(fifths, h.step));
                let tied = tied_into.remove(&(part, voice, h.dia, h.alter));
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
            let mut x_cursor = left_edge - 0.2;
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
    fn beam_groups(&mut self) -> Vec<Vec<usize>> {
        let mut groups = Vec::new();
        let mut cur: Vec<usize> = Vec::new();
        let mut cur_key = None;
        for i in 0..self.notes.len() {
            let n = &self.notes[i];
            let m = &self.ms[n.measure];
            let beat = if m.den == 8 && m.num % 3 == 0 { Frac::new(3, 8) } else if m.den >= 8 { Frac::new(1, m.den) * Frac::int(2) } else { Frac::new(1, 4) };
            let rel = (n.start - m.start) * Frac::new(beat.den, beat.num);
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
        let mut right = if n.rest { self.width(rest_glyph(n.value.base)) } else { HEAD_W };
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

    fn space(&mut self) {
        let x_start = self.opt.margin;
        let mut x = x_start;
        // System start: clef, key, time.
        let clef_w = self.score.parts.iter().map(|p| self.width(clef_glyph(&p.clef).0)).fold(0.0f32, f32::max);
        x += 0.8 + clef_w + 0.8;
        let nms = self.ms.len();
        for mi in 0..nms {
            if mi > 0 {
                x += 0.0;
            }
            let m = self.ms[mi].clone();
            // Key and time shown at this barline.
            if let Some(k) = m.key {
                let prev = if mi == 0 { 0 } else { key_at(self.score, self.ms[mi - 1].start) };
                let count = k.fifths.unsigned_abs().max(if mi > 0 { prev.unsigned_abs() } else { 0 });
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
            let mut cx = x + 1.0;
            for (j, &t) in cols.iter().enumerate() {
                let here: Vec<&Note> = self.notes.iter().filter(|n| n.measure == mi && n.start == t && !n.whole_bar).collect();
                let (left, right) = here.iter().map(|n| self.note_extents(n)).fold((0.0f32, 0.0f32), |a, b| (a.0.max(b.0), a.1.max(b.1)));
                if j == 0 {
                    cx += left;
                } else {
                    let tprev = cols[j - 1];
                    let gap = t - tprev;
                    let dmin = self
                        .notes
                        .iter()
                        .filter(|n| n.measure == mi && n.start <= tprev && n.start + n.dur > tprev && !n.whole_bar)
                        .map(|n| n.dur)
                        .min()
                        .unwrap_or(gap);
                    let ideal = ideal_space(dmin) * (gap.num as f32 / gap.den as f32) / (dmin.num as f32 / dmin.den as f32);
                    cx += ideal.max(prev_right + left + 0.4);
                }
                xs.push(cx);
                prev_right = right;
            }
            let tlast = *cols.last().unwrap();
            let gap = end - tlast;
            let dmin = self
                .notes
                .iter()
                .filter(|n| n.measure == mi && n.start <= tlast && n.start + n.dur > tlast && !n.whole_bar)
                .map(|n| n.dur)
                .min()
                .unwrap_or(gap);
            let mut x1 = cx
                + if gap > Frac::ZERO && dmin > Frac::ZERO {
                    (ideal_space(dmin) * (gap.num as f32 / gap.den as f32) / (dmin.num as f32 / dmin.den as f32)).max(prev_right + 0.8)
                } else {
                    prev_right + 0.8
                };
            if self.notes.iter().filter(|n| n.measure == mi).all(|n| n.whole_bar) {
                x1 = x1.max(x + 8.0); // an empty bar keeps a sensible width
            }
            for n in self.notes.iter_mut().filter(|n| n.measure == mi) {
                let j = cols.iter().position(|&t| t == n.start).unwrap_or(0);
                n.x = xs[j];
            }
            for (j, &t) in cols.iter().enumerate() {
                self.out.columns.push(Column { quarter: (t.num as f64 / t.den as f64) * 4.0, x: xs[j] });
            }
            self.ms[mi].x1 = x1;
            x = x1 + 0.0;
        }
        self.out.columns.push(Column { quarter: self.score.length.num as f64 / self.score.length.den as f64 * 4.0, x });
        self.out.width = x + self.opt.margin;
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
            self.out.staves.push(StaffGeom { part: pi, top, left: self.opt.margin, right, line_width: t });
            // Clef at the system start.
            let (g, dy) = clef_glyph(&self.score.parts[pi].clef);
            self.push(Item::glyph(g, self.opt.margin + 0.8, top + dy, 0));
        }
        // System barline at the left edge.
        if let Some(last) = self.score.parts.len().checked_sub(1) {
            let thin = self.font.defaults.thin_barline_thickness;
            self.push(Item::line(self.opt.margin, 0.0, self.opt.margin, self.top(last) + 4.0, thin, 0));
        }
    }

    fn draw_measure_furniture(&mut self) {
        let d = self.font.defaults;
        let clef_w = self.score.parts.iter().map(|p| self.width(clef_glyph(&p.clef).0)).fold(0.0f32, f32::max);
        for mi in 0..self.ms.len() {
            let m = self.ms[mi].clone();
            let mut x = if mi == 0 { self.opt.margin + 0.8 + clef_w + 0.8 } else { self.ms[mi - 1].x1 + 1.0 };
            for pi in 0..self.score.parts.len() {
                let mut kx = x;
                let clef = self.clef_at(pi, m.start);
                if let Some(k) = m.key {
                    // Cancel the previous key's accidentals with naturals when it changes to fewer/other.
                    if mi > 0 {
                        let prev = key_at(self.score, self.ms[mi - 1].start);
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
            let last = mi + 1 == self.ms.len();
            let start_repeat = self.score.barlines.iter().any(|b| b.at == m.start && matches!(b.kind, BarKind::RepeatStart | BarKind::RepeatBoth));
            for pi in 0..self.score.parts.len() {
                let (top, bot) = (self.top(pi), self.top(pi) + 4.0);
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
                    self.push(Item::line(x - ext, ly, x + HEAD_W + ext, ly, d.leger_line_thickness, 0));
                    lp += 2;
                }
                let mut lp = -6;
                while lp >= h.pos {
                    let ly = self.y(n.part, lp);
                    self.push(Item::line(x - ext, ly, x + HEAD_W + ext, ly, d.leger_line_thickness, 0));
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

    fn draw_beams(&mut self, groups: &[Vec<usize>]) {
        let d = self.font.defaults;
        let (bt, gap) = (d.beam_thickness, d.beam_thickness + d.beam_spacing);
        for g in groups {
            let up = self.notes[g[0]].stem_up;
            let (fi, li) = (g[0], *g.last().unwrap());
            let (xf, xl) = (self.notes[fi].stem_x, self.notes[li].stem_x);
            let (yf, yl) = (self.notes[fi].stem_end, self.notes[li].stem_end);
            // Slope from the outer notes, gentle; then shifted so no stem is shorter than 3 spaces.
            let slope = if xl > xf { ((yl - yf) / (xl - xf)).clamp(-0.2, 0.2) } else { 0.0 };
            let levels = g.iter().map(|&i| self.notes[i].beams()).max().unwrap_or(1);
            let min_len = 2.75 + (levels.saturating_sub(1)) as f32 * gap;
            let mut y0 = yf;
            for &i in g {
                let n = &self.notes[i];
                let yb = y0 + slope * (n.stem_x - xf);
                let (lo, hi) = (self.y(n.part, n.heads[0].pos), self.y(n.part, n.heads[n.heads.len() - 1].pos));
                if up {
                    let need = hi - min_len;
                    if yb > need { y0 -= yb - need }
                } else {
                    let need = lo + min_len;
                    if yb < need { y0 += need - yb }
                }
            }
            let beam_y = |x: f32| y0 + slope * (x - xf);
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
                Item::Quad { p: [pt(x0, a), pt(x1, b), pt(x1, b + thick), pt(x0, a + thick)], id: 0 }
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
        for i in 0..self.notes.len() {
            let n = &self.notes[i];
            if n.rest {
                continue;
            }
            // The next piece of the same voice.
            let Some(next) = self.notes[i + 1..].iter().find(|o| o.part == n.part && o.voice == n.voice && o.start >= n.start + n.dur) else {
                continue;
            };
            if next.start != n.start + n.dur {
                continue;
            }
            let mut items = Vec::new();
            for h in n.heads.iter().filter(|h| h.tie_out) {
                let Some(h2) = next.heads.iter().find(|x| x.dia == h.dia && x.alter == h.alter) else { continue };
                let above = !n.stem_up || n.heads.len() > 1 && h.pos > n.heads[0].pos && h.pos >= n.heads[n.heads.len() - 1].pos;
                let dir = if above { -1.0 } else { 1.0 };
                let y = self.y(n.part, h.pos) + dir * 0.35;
                let x0 = n.x + h.shift + HEAD_W + 0.1;
                let x1 = (next.x + h2.shift - 0.1).max(x0 + 0.6);
                let height = (0.35 + (x1 - x0) * 0.06).min(1.1);
                let (thick_mid, thin) = (d.tie_midpoint_thickness, d.tie_endpoint_thickness);
                let c = |frac: f32, h: f32| pt(x0 + (x1 - x0) * frac, y + dir * h);
                items.push(Item::Crescent {
                    from: pt(x0, y + dir * thin * 0.0),
                    c1: c(0.25, height + thick_mid),
                    c2: c(0.75, height + thick_mid),
                    to: pt(x1, y),
                    d1: c(0.25, height),
                    d2: c(0.75, height),
                    id: h.id,
                });
            }
            self.out.items.extend(items);
        }
    }

    fn draw_marks(&mut self) {
        for i in 0..self.notes.len() {
            let n = self.notes[i].clone();
            if !n.first || n.rest && n.dynamic.is_none() {
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
                // Fermata and ornaments above the staff (or the note).
                let mut y = (self.y(n.part, n.heads[n.heads.len() - 1].pos) - 1.5).min(top - 1.0);
                if n.stem_up && n.has_stem() {
                    y = y.min(n.stem_end - 1.0);
                }
                for (bit, g) in [(art::FERMATA, Glyph::FermataAbove), (art::TRILL, Glyph::Trill), (art::MORDENT, Glyph::Mordent), (art::TURN, Glyph::Turn)] {
                    if n.art & bit != 0 {
                        self.push(Item::glyph(g, center(g, self), y, n.id));
                        y -= 1.8;
                    }
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
            if let Some(dynamic) = &n.dynamic {
                let mut x = n.x - 0.3;
                // Below the staff, and below anything of this column on it.
                let mut y = top + 4.0 + 3.0;
                for o in self.notes.iter().filter(|o| o.part == n.part && o.start == n.start && !o.rest) {
                    y = y.max(self.y(o.part, o.heads[0].pos) + 2.6);
                    if !o.stem_up && o.has_stem() {
                        let marks = o.art & (art::STACCATO | art::STACCATISSIMO | art::TENUTO | art::ACCENT | art::MARCATO) != 0;
                        y = y.max(o.stem_end + if marks { 3.2 } else { 2.0 });
                    }
                }
                // Several voices with a dynamic at one moment: stacked, voice 1 on top.
                let earlier = self.notes[..i]
                    .iter()
                    .filter(|o| o.part == n.part && o.start == n.start && o.first && o.dynamic.is_some())
                    .count();
                y += earlier as f32 * 2.2;
                for ch in dynamic.chars() {
                    let g = match ch {
                        'p' => Glyph::DynamicP,
                        'm' => Glyph::DynamicM,
                        _ => Glyph::DynamicF,
                    };
                    self.push(Item::glyph(g, x, y, n.id));
                    x += self.width(g) - 0.05;
                }
            }
        }
    }
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

/// Lays the whole movement out on one system.
pub fn layout_line(score: &Score, opt: &Options) -> DisplayList {
    let ms = measures(score);
    let notes = split(score, &ms);
    let mut e = Engraver { score, font: Font::bravura(), opt: opt.clone(), ms, notes, out: DisplayList::default() };
    e.prepare();
    let groups = e.beam_groups();
    e.space();
    e.draw_staves();
    e.draw_measure_furniture();
    e.draw_notes();
    e.draw_beams(&groups);
    e.draw_ties();
    e.draw_marks();
    let staves = score.parts.len().max(1);
    e.out.height = (staves - 1) as f32 * e.opt.staff_distance + 4.0;
    e.out
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
    fn spacing_grows_logarithmically() {
        let q = ideal_space(Frac::new(1, 4));
        let h = ideal_space(Frac::new(1, 2));
        let e = ideal_space(Frac::new(1, 8));
        assert!(e < q && q < h);
        assert!(h / q < 1.5 && q / e < 1.5);
    }
}
