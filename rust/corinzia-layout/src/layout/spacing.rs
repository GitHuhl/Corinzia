//! Horizontal spacing: LilyPond's duration spacing with optical corrections,
//! rods that keep notes, accidentals and graces apart, and justification by
//! the springs' stretchability.
//!
//! Ported from GNU LilyPond (`lily/spacing-options.cc`, the spacing defaults
//! in `scm/define-grobs.scm`), Copyright (C) 1997–2026 Han-Wen Nienhuys, Jan
//! Nieuwenhuizen and the LilyPond authors, under the GNU General Public
//! License, version 3 or later (see `THIRD-PARTY-NOTICES.md`).

use super::*;

// Horizontal spacing follows LilyPond's documented model and defaults
// (SpacingSpanner): the movement's common shortest note gets
// `shortest-duration-space` increments, and each doubling of duration adds
// one `spacing-increment`.
pub(super) const SHORTEST_DURATION_SPACE: f64 = 2.0;

pub(super) const SPACING_INCREMENT: f64 = 1.2;

/// Ideal space after a note of duration `d` when `shortest` is the
/// movement's common shortest duration.
/// (LilyPond's `Spacing_options::get_duration_space`: notes shorter than
/// the shortest are spaced linearly, not by the logarithm, so that they
/// do not crowd.)
pub(super) fn ideal_space(d: Frac, shortest: Frac) -> f32 {
    let r = (d.num as f64 / d.den as f64) / (shortest.num as f64 / shortest.den as f64);
    let k = if r < 1.0 { SHORTEST_DURATION_SPACE + r - 1.0 } else { SHORTEST_DURATION_SPACE + r.log2() };
    (SPACING_INCREMENT * k) as f32
}

/// Fixed distances around barlines and between columns, measured from
/// LilyPond's output (BarLine `space-alist`, NoteSpacing, separation rods).
pub(super) const BAR_TO_NOTE: f32 = 0.995; // barline centre to the next notehead
pub(super) const BAR_TO_ACCIDENTAL: f32 = 0.695; // barline centre to an accidental
pub(super) const BAR_HALF_WIDTH: f32 = 0.095;
/// Before a barline an up-stem gains, and a down-stem loses, this much per
/// stem length inside the staff (unless it has a flag); after a barline a
/// down-stem gains `STEM_AFTER_BAR` per stem length.
pub(super) const STEM_BEFORE_BAR: f32 = 0.25;
pub(super) const STEM_AFTER_BAR: f32 = 0.185;
/// Notehead widths of LilyPond's font, used for spacing so that bars come
/// out as wide as LilyPond's (the heads are drawn in Bravura).
pub(super) const SPACING_HEAD_WIDTHS: [(Glyph, f32); 3] = [(Glyph::NoteheadBlack, 1.304), (Glyph::NoteheadHalf, 1.377), (Glyph::NoteheadWhole, 1.962)];
/// The stretchable half of the barline-to-note distance (LilyPond's
/// `semi-fixed-space`).
pub(super) const BAR_TO_NOTE_STRETCH: f32 = 0.45;
/// Each line's fixed cost in the breaker, against the square of its force:
/// LilyPond prefers fewer, slightly compressed lines to more stretched ones.
pub(super) const LINE_PENALTY: f32 = 0.2;
/// How far a line may be compressed, as a force: each spring gives up at
/// most this share of its stretchability.
pub(super) const MAX_COMPRESSION: f32 = -0.9;
/// A bar holding a single note column gets this much more before it.
pub(super) const LONE_NOTE_SPACE: f32 = 1.0;
/// Least room between a column's right side (heads, dots, flags) and the
/// next one's left side: an accidental, or else the heads.
pub(super) const ROD_PADDING_ACCIDENTAL: f32 = 0.6;
pub(super) const ROD_PADDING: f32 = 0.45;
/// NoteSpacing's optical corrections: facing stems (up, then down) get room
/// in proportion to their vertical overlap, and lose it the other way round;
/// stems in the same direction gain (rising) or lose (falling) a fixed
/// amount for steps of a third or more.
pub(super) const STEM_SPACING_CORRECTION: f32 = 0.5;
pub(super) const SAME_DIRECTION_CORRECTION: f32 = 0.25;
pub(super) const STEM_LENGTH: f32 = 3.5;
/// How far from the head's centre a stem's overlap starts counting.
pub(super) const STEM_ATTACH: f32 = 0.193;

/// The stretchability left of `x`, from the (x, stretchability) points of
/// a line, interpolated between them.
pub(super) fn stretch_at(breaks: &[(f32, f32)], x: f32) -> f32 {
    let i = breaks.partition_point(|b| b.0 <= x);
    match (i.checked_sub(1).map(|j| breaks[j]), breaks.get(i)) {
        (None, _) => 0.0,
        (Some(a), None) => a.1,
        (Some(a), Some(b)) if b.0 > a.0 => a.1 + (b.1 - a.1) * (x - a.0) / (b.0 - a.0),
        (Some(a), Some(_)) => a.1,
    }
}

pub(super) fn spacing_head_width(g: Glyph) -> Option<f32> {
    SPACING_HEAD_WIDTHS.iter().find(|h| h.0 == g).map(|h| h.1)
}

pub(super) fn overlap((l1, h1): (f32, f32), (l2, h2): (f32, f32)) -> f32 {
    (h1.min(h2) - l1.max(l2)).max(0.0)
}

impl<'a> Engraver<'a> {
    // ------------------------------------------------------------- spacing --
    /// The width the grace notes before `n` take, right to left: each
    /// grace's accidentals, head and gap.
    pub(super) fn grace_widths(&self, n: &Note) -> Vec<f32> {
        // A lone grace's flag sticks out to the right of its stem.
        let flag = if n.graces.len() == 1 && n.voice % 2 == 1 { 0.55 } else { 0.0 };
        n.graces
            .iter()
            .map(|g| {
                let acc = g.heads.iter().filter_map(|h| h.accidental).map(|a| self.width(a) * GRACE_SCALE + 0.1).fold(0.0f32, f32::max);
                acc + HEAD_W * GRACE_SCALE + GRACE_GAP + flag
            })
            .collect()
    }

    pub(super) fn note_extents(&self, n: &Note) -> (f32, f32) {
        let mut left = n.heads.iter().map(|h| if h.accidental.is_some() { -h.acc_x } else { -h.shift.min(0.0) }).fold(0.0f32, f32::max);
        if !n.graces.is_empty() {
            left += GRACE_TO_MAIN - GRACE_GAP + self.grace_widths(n).iter().sum::<f32>();
        }
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
    pub(super) fn column_head_width(&self, col: &[&Note]) -> f32 {
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
    pub(super) fn stems(&self, col: &[&Note]) -> Vec<StemInfo> {
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
    pub(super) fn stem_correction(&self, a: &[&Note], b: &[&Note]) -> f32 {
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

    pub(super) fn space(&mut self) {
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
        // Compression's own weights: a gap held open by grace notes gives
        // no more than down to its rod.
        let mut squeezes: Vec<(f32, f32)> = Vec::new();
        let mut squeeze = 0.0f32;
        let mut stretch = 0.0f32;
        for mi in 0..nms {
            if mi > 0 {
                x += 0.0;
            }
            let m = self.ms[mi].clone();
            let bar_stretch = stretch;
            let bar_squeeze = squeeze;
            // Key and time shown at this barline.
            if let Some(k) = m.key {
                // The widest staff's signature (transposing staves differ).
                let prev_at = if mi == 0 { Frac::ZERO } else { self.ms[mi - 1].start };
                let prev_global = if mi == 0 { 0 } else { key_at(self.score, prev_at) };
                let count = (0..self.score.parts.len().max(1))
                    .map(|pi| {
                        let (now, prev) = (part_key(self.score, pi, k.fifths, m.start), part_key(self.score, pi, prev_global, prev_at));
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
                let space = ideal_space(dmin, m.shortest) * self.opt.spacing * (gap.num as f32 / gap.den as f32) / (dmin.num as f32 / dmin.den as f32);
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
                        squeeze += BAR_TO_NOTE_STRETCH;
                    }
                } else {
                    let (spring, k) = ideal(cols[j - 1], t - cols[j - 1], &prev_here).unwrap_or((0.0, 0.0));
                    stretch += k;
                    let spring = spring + self.stem_correction(&prev_here, &here);
                    let rod = prev_right + left + if left > 0.0 { ROD_PADDING_ACCIDENTAL } else { ROD_PADDING };
                    // Grace notes squeezed into the note before them would
                    // collide: there the rod holds under compression.
                    squeeze += if here.iter().any(|n| !n.graces.is_empty()) { k.min((spring - rod).max(0.0) / -MAX_COMPRESSION) } else { k };
                    cx += spring.max(rod);
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
                squeezes.push((cx, squeeze));
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
            squeeze += k;
            let mut x1 = cx + spring.max(prev_right + 0.8);
            if m.multi > 1 {
                // A multi-bar rest: a little wider for longer ones.
                x1 = x1.max(x + 10.0 + 2.0 * (m.multi as f32).log2());
            }
            if self.notes.iter().filter(|n| n.measure == mi).all(|n| n.whole_bar) {
                x1 = x1.max(x + 8.0); // an empty bar keeps a sensible width
                let k = (ideal_space(m.len, m.shortest) - SPACING_INCREMENT as f32).max(0.1) * self.opt.spacing;
                stretch += k;
                squeeze += k;
            }
            breaks.push((x1, stretch));
            squeezes.push((x1, squeeze));
            self.ms[mi].stretch = stretch - bar_stretch;
            self.ms[mi].squeeze = squeeze - bar_squeeze;
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
            let (total, weights) = if target < x { (squeeze, &squeezes) } else { (stretch, &breaks) };
            if total > 0.0 && (target - x).abs() > 1e-3 {
                // Compressed lines stop short of the springs' minimum.
                let k = ((target - x) / total).max(MAX_COMPRESSION);
                let f = |v: f32| v + k * stretch_at(weights, v);
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

    pub(super) fn clef_change_width(&self) -> f32 {
        self.width(Glyph::GClef) * CLEF_CHANGE_SCALE
    }

    pub(super) fn time_width(&self, m: &Measure) -> f32 {
        let digits = |v: i64| v.to_string().len() as f32;
        digits(m.num).max(digits(m.den)) * self.width(Glyph::TimeSigDigit(4))
    }
}
