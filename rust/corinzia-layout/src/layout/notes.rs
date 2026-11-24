//! Notes, rests, grace notes, tablature and dots: heads, stems, flags, ledger
//! lines and accidentals.
//!
//! Ported from GNU LilyPond (`lily/accidental-placement.cc`,
//! `lily/rest-collision.cc`, `lily/dot-configuration.cc`,
//! `lily/dot-column.cc`), Copyright (C) 1997–2026 Han-Wen Nienhuys, Jan
//! Nieuwenhuizen and the LilyPond authors, under the GNU General Public
//! License, version 3 or later (see `THIRD-PARTY-NOTICES.md`).

use super::*;

/// The rows of a chord's dots, in staff positions (half spaces up from the
/// middle line), as LilyPond's Dot_configuration sets them: one dot per
/// head (at most `chord-dots-limit` 3 more than the chord's span leaves
/// room for), each in a space; when two would meet, the one in the way and
/// those beyond it shift up or down, whichever moves them least (moving
/// down costs a little, moving against `dir`, a voice's side, more).
pub(super) fn dot_rows(positions: &[i32], dir: i32) -> Vec<i32> {
    use std::collections::BTreeMap;
    let mut heads: Vec<i32> = positions.to_vec();
    heads.sort();
    if let (Some(&lo), Some(&hi)) = (heads.first(), heads.last()) {
        let room = ((hi - lo) as usize + 2 + 3) / 2;
        let mut total = heads.len();
        let mut first = 0;
        while total > room {
            if (total - room).is_multiple_of(2) {
                first += 1;
            }
            total -= 1;
        }
        heads = heads[first..first + total].to_vec();
    }
    let on_line = |p: i32| p % 2 == 0;
    // Position -> the head position it belongs to.
    type Cfg = BTreeMap<i32, i32>;
    let badness = |c: &Cfg| -> i32 {
        c.iter()
            .map(|(&p, &o)| {
                let mv = (p - o).signum();
                2 * (p - o) * (p - o) + if dir != 0 && mv != dir { 2 } else if mv != 1 { 1 } else { 0 }
            })
            .sum()
    };
    let shifted = |c: &Cfg, k: i32, d: i32| -> Cfg {
        let mut out = Cfg::new();
        let mut offset = 0;
        let entries: Vec<(i32, i32)> = if d > 0 { c.iter().map(|(&a, &b)| (a, b)).collect() } else { c.iter().rev().map(|(&a, &b)| (a, b)).collect() };
        for (p, o) in entries {
            if p == k {
                let q = if on_line(p) { p + d } else { p + 2 * d };
                offset = 2 * d;
                out.insert(q, o);
            } else {
                if !out.contains_key(&p) {
                    offset = 0;
                }
                out.insert(p + offset, o);
            }
        }
        out
    };
    let remove_collision = |c: &mut Cfg, p: i32| {
        if c.contains_key(&p) {
            let (up, down) = (shifted(c, p, 1), shifted(c, p, -1));
            *c = if badness(&up) < badness(&down) { up } else { down };
        }
    };
    let mut cfg = Cfg::new();
    for p in heads {
        remove_collision(&mut cfg, p);
        cfg.insert(p, p);
        if on_line(p) {
            remove_collision(&mut cfg, p);
        }
    }
    cfg.keys().copied().collect()
}

/// Grace notes are drawn at LilyPond's grace size (font-size -3).
pub(super) const GRACE_SCALE: f32 = 0.707;

/// Cue notes: smaller than normal, larger than graces.
pub(super) const CUE_SCALE: f32 = 0.75;

/// Between grace notes, and from the last one to its main note.
pub(super) const GRACE_GAP: f32 = 0.35;

pub(super) const GRACE_TO_MAIN: f32 = 1.0;

pub(super) fn place_accidentals(note: &mut Note) {
    const PADDING: f32 = 0.2;
    const RIGHT_PADDING: f32 = 0.15;
    const VERTICAL_FUDGE: f32 = 0.1;
    let font = Font::bravura();
    // Boxes (x0, x1, y0, y1), y down from the middle line, x from the note's x.
    let mut placed: Vec<(f32, f32, f32, f32)> = note
        .heads
        .iter()
        .map(|h| (h.shift - RIGHT_PADDING, h.shift + HEAD_W, -h.pos as f32 * 0.5 - 0.5, -h.pos as f32 * 0.5 + 0.5))
        .collect();
    // The groups: by note name, then split where one octave holds two.
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for (j, h) in note.heads.iter().enumerate().filter(|(_, h)| h.accidental.is_some()) {
        let name = h.dia.rem_euclid(7);
        match groups.iter_mut().find(|g| g.iter().all(|&k| note.heads[k].dia != h.dia) && note.heads[g[0]].dia.rem_euclid(7) == name) {
            Some(g) => g.push(j),
            None => groups.push(vec![j]),
        }
    }
    if groups.is_empty() {
        return;
    }
    let top = |g: &Vec<usize>, n: &Note| g.iter().map(|&k| n.heads[k].pos).max().unwrap_or(0);
    groups.sort_by(|a, b| b.len().cmp(&a.len()).then(top(a, note).cmp(&top(b, note))));
    // Within each size, from the outside in: highest, lowest, next highest…
    let mut order: Vec<Vec<usize>> = Vec::new();
    let mut i = 0;
    while i < groups.len() {
        let mut same: std::collections::VecDeque<Vec<usize>> = groups[i..].iter().take_while(|g| g.len() == groups[i].len()).cloned().collect();
        i += same.len();
        let mut from_top = true;
        while let Some(g) = if from_top { same.pop_back() } else { same.pop_front() } {
            order.push(g);
            from_top = !from_top;
        }
    }
    let mut last_offset = 0.0f32;
    for g in order {
        // The group's boxes with each glyph's right edge at 0.
        // Each glyph as its outline's bands.
        let boxes: Vec<(usize, f32, AccBox)> = g
            .iter()
            .flat_map(|&k| {
                let h = &note.heads[k];
                let gl = h.accidental.unwrap();
                let right = font.bbox(gl).ne.0;
                let y = -h.pos as f32 * 0.5;
                font.outline_bands(gl, 24).into_iter().map(move |(x0, x1, y0, y1)| (k, right, (x0 - right, x1 - right, y - y1, y - y0)))
            })
            .collect();
        let mut offset = f32::INFINITY;
        for &(_, _, (_, bx1, by0, by1)) in &boxes {
            for &(px0, _, py0, py1) in &placed {
                if by0 < py1 + VERTICAL_FUDGE && by1 > py0 - VERTICAL_FUDGE {
                    offset = offset.min(px0 - bx1);
                }
            }
        }
        let offset = if offset.is_finite() { offset - PADDING } else { last_offset };
        for &(k, right, (bx0, bx1, by0, by1)) in &boxes {
            note.heads[k].acc_x = offset - right;
            placed.push((bx0 + offset, bx1 + offset, by0, by1));
        }
        // (A glyph's bands all give it the same place.)
        last_offset = offset;
    }
}

impl<'a> Engraver<'a> {
    /// Stems (with flags) for unbeamed notes, ledger lines, heads,
    /// accidentals, dots, rests.
    pub(super) fn draw_notes(&mut self) {
        let d = self.font.defaults;
        let st = d.stem_thickness;
        for i in 0..self.notes.len() {
            let n = self.notes[i].clone();
            let top = self.top(n.part);
            let s = if n.art & art::CUE != 0 { CUE_SCALE } else { 1.0 };
            if n.rest {
                let g = rest_glyph(n.value.base);
                let poly = self.notes.iter().any(|o| o.part == n.part && o.measure == n.measure && o.voice != n.voice && !o.whole_bar);
                let mut y = if n.value.base <= 1 { top + 1.0 } else { top + 2.0 };
                if poly {
                    let up = n.voice % 2 == 1;
                    y += if up { -2.0 } else { 2.0 };
                    // LilyPond's RestCollision: clear of the other voices'
                    // notes at this moment by 0.75, moved in half spaces
                    // (whole spaces within the staff, so it stays on its line).
                    let b = self.font.bbox(g);
                    let (r_top, r_bottom) = (y - b.ne.1 * s, y - b.sw.1 * s);
                    let heads = self.notes.iter().filter(|o| o.part == n.part && o.voice != n.voice && !o.rest && o.start == n.start).flat_map(|o| o.heads.iter().map(move |h| (o.part, h.pos)));
                    let (mut n_top, mut n_bottom) = (f32::MAX, f32::MIN);
                    for (p, pos) in heads {
                        let hy = self.y(p, pos);
                        (n_top, n_bottom) = (n_top.min(hy - 0.5), n_bottom.max(hy + 0.5));
                    }
                    if n_top < f32::MAX {
                        let need = if up { (r_bottom - n_top + 0.75).max(0.0) } else { (n_bottom - r_top + 0.75).max(0.0) };
                        let mut steps = (need / 0.5).ceil() as i32;
                        let pos_after = -((y - top - 2.0) * 2.0) as i32 + if up { steps } else { -steps };
                        if (-5..=5).contains(&pos_after) {
                            steps = (steps as f32 / 2.0).ceil() as i32 * 2;
                        }
                        y += if up { -0.5 } else { 0.5 } * steps as f32;
                    } else if let Some(o) = self.notes.iter().find(|o| o.part == n.part && o.voice != n.voice && o.rest && o.start == n.start && (o.voice % 2 == 1) != up) {
                        // Only rests here (LilyPond's rest-against-rest case):
                        // where the upper voice's rest and the lower's overlap,
                        // the lower moves down by half the overlap, the upper
                        // up by the rest, in whole spaces.
                        let og = rest_glyph(o.value.base);
                        let ob = self.font.bbox(og);
                        let oy = (if o.value.base <= 1 { top + 1.0 } else { top + 2.0 }) + if up { 2.0 } else { -2.0 };
                        let (up_bottom, down_top) = if up { (r_bottom, oy - ob.ne.1) } else { (oy - ob.sw.1, r_top) };
                        let mut diff = up_bottom - down_top;
                        if diff > 0.0 {
                            let down = (diff / 2.0).ceil();
                            diff -= down;
                            if !up {
                                y += down;
                            } else if diff > 0.0 {
                                y -= diff.ceil();
                            }
                        }
                    }
                }
                if n.whole_bar && self.ms[n.measure].multi > 1 {
                    // An H-bar on the middle line between serifs, the count above.
                    let m = self.ms[n.measure].clone();
                    let (a, b) = (m.x0 + 1.2, m.x1 - 1.2);
                    let mid = top + 2.0;
                    let h = 0.5;
                    self.push(Item::Quad { p: [pt(a, mid - h), pt(b, mid - h), pt(b, mid + h), pt(a, mid + h)], id: n.id });
                    let t = self.font.defaults.thin_barline_thickness * 1.5;
                    for xs in [a, b] {
                        self.push(Item::line(xs, top + 1.0, xs, top + 3.0, t, n.id));
                    }
                    let digits: Vec<Glyph> = m.multi.to_string().bytes().map(|c| Glyph::TimeSigDigit(c - b'0')).collect();
                    let w: f32 = digits.iter().map(|&g| self.width(g)).sum();
                    let mut dx = (a + b) / 2.0 - w / 2.0;
                    for g in digits {
                        // Digits stand on their middle: one above the top line.
                        self.push(Item::glyph(g, dx, top - 1.6, n.id));
                        dx += self.width(g);
                    }
                    continue;
                }
                let x = if n.whole_bar {
                    let m = &self.ms[n.measure];
                    (m.x0 + m.x1) / 2.0 - self.width(g) / 2.0
                } else {
                    n.x
                };
                self.push(Item::glyph_scaled(g, x, y, s, n.id));
                if n.value.base > 1 || !n.whole_bar {
                    self.dots(&n, x + self.width(g), &[1]);
                }
                continue;
            }
            let hg = n.head_glyph();
            for h in &n.heads {
                let x = n.x + h.shift * s;
                let y = self.y(n.part, h.pos);
                // Ledger lines out to the head.
                let ext = d.leger_line_extension * s;
                let mut lp = 6;
                while lp <= h.pos {
                    let ly = self.y(n.part, lp);
                    self.push(Item::line(x - ext, ly, x + HEAD_W * s + ext, ly, d.leger_line_thickness, n.id));
                    lp += 2;
                }
                let mut lp = -6;
                while lp >= h.pos {
                    let ly = self.y(n.part, lp);
                    self.push(Item::line(x - ext, ly, x + HEAD_W * s + ext, ly, d.leger_line_thickness, n.id));
                    lp -= 2;
                }
                self.push(Item::glyph_scaled(shaped_head(hg, h.shape), x, y, s, h.id));
                if let Some(a) = h.accidental {
                    self.push(Item::glyph_scaled(a, n.x + h.acc_x + self.width(a) * (1.0 - s), y, s, h.id));
                }
            }
            let right = n.x + HEAD_W * s + n.heads.iter().map(|h| h.shift.max(0.0)).fold(0.0f32, f32::max);
            let positions: Vec<i32> = n.heads.iter().map(|h| h.pos).collect();
            self.dots(&n, right, &positions);
            if !n.has_stem() {
                continue;
            }
            let (lo, hi) = (n.heads[0].pos, n.heads[n.heads.len() - 1].pos);
            let mid = top + 2.0;
            let (stem_x, from, mut to) = if n.stem_up {
                let a = self.font.anchor(Glyph::NoteheadBlack, "stemUpSE").unwrap_or((HEAD_W, 0.168));
                (n.x + a.0 * s - st / 2.0, self.y(n.part, lo) - a.1 * s, self.y(n.part, hi) - 3.5 * s)
            } else {
                let a = self.font.anchor(Glyph::NoteheadBlack, "stemDownNW").unwrap_or((0.0, -0.168));
                (n.x + a.0 * s + st / 2.0, self.y(n.part, hi) - a.1 * s, self.y(n.part, lo) + 3.5 * s)
            };
            // Stems of notes far outside the staff reach the middle line (not cues).
            if s == 1.0 {
                if n.stem_up { to = to.min(mid) } else { to = to.max(mid) }
            }
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
                self.push(Item::glyph_scaled(f, stem_x - st / 2.0, to, s, n.id));
            }
        }
    }

    /// Grace notes: small heads left of their note (and its accidentals),
    /// stems up (down in voices 2 and 4), a flag each or one beam over a
    /// group, a slash through the first stem of an acciaccatura.
    pub(super) fn draw_graces(&mut self) {
        let d = self.font.defaults;
        let s = GRACE_SCALE;
        for i in 0..self.notes.len() {
            let n = self.notes[i].clone();
            if n.graces.is_empty() {
                continue;
            }
            let up = n.voice % 2 == 1;
            let main_left = n.heads.iter().map(|h| if h.accidental.is_some() { -h.acc_x } else { -h.shift.min(0.0) }).fold(0.0f32, f32::max);
            let widths = self.grace_widths(&n);
            let mut x = n.x - main_left - (GRACE_TO_MAIN - GRACE_GAP) - widths.iter().sum::<f32>();
            let st = d.stem_thickness * s.max(0.8);
            let head_w = HEAD_W * s;
            // (stem x, attach y, natural tip y) per grace.
            let mut stems: Vec<(f32, f32, f32)> = Vec::new();
            let flag = if n.graces.len() == 1 && up { 0.55 } else { 0.0 };
            for (g, w) in n.graces.iter().zip(&widths) {
                let acc_w = w - head_w - GRACE_GAP - flag;
                let hx = x + acc_w;
                for h in &g.heads {
                    let y = self.y(n.part, h.pos);
                    let ext = d.leger_line_extension * s;
                    let mut lp = 6;
                    while lp <= h.pos {
                        let ly = self.y(n.part, lp);
                        self.push(Item::line(hx - ext, ly, hx + head_w + ext, ly, d.leger_line_thickness, h.id));
                        lp += 2;
                    }
                    let mut lp = -6;
                    while lp >= h.pos {
                        let ly = self.y(n.part, lp);
                        self.push(Item::line(hx - ext, ly, hx + head_w + ext, ly, d.leger_line_thickness, h.id));
                        lp -= 2;
                    }
                    self.push(Item::glyph_scaled(Glyph::NoteheadBlack, hx, y, s, h.id));
                    if let Some(a) = h.accidental {
                        self.push(Item::glyph_scaled(a, hx - self.width(a) * s - 0.1, y, s, h.id));
                    }
                }
                let (lo, hi) = (g.heads[0].pos, g.heads[g.heads.len() - 1].pos);
                let stem = 3.5 * s;
                let (sx, attach, tip) = if up {
                    (hx + head_w - st / 2.0, self.y(n.part, lo) - 0.12, self.y(n.part, hi) - stem)
                } else {
                    (hx + st / 2.0, self.y(n.part, hi) + 0.12, self.y(n.part, lo) + stem)
                };
                stems.push((sx, attach, tip));
                x += w;
            }
            let beamed = n.graces.len() > 1;
            let beams = |base: u8| match base {
                0..=8 => 1,
                16 => 2,
                32 => 3,
                _ => 4,
            };
            if beamed {
                // A straight beam from the first tip to the last, moved out
                // until every stem is long enough.
                let (x0, x1) = (stems[0].0, stems[stems.len() - 1].0);
                let (mut y0, mut y1) = (stems[0].2, stems[stems.len() - 1].2);
                let slope = ((y1 - y0) / (x1 - x0).max(0.1)).clamp(-0.25, 0.25);
                y1 = y0 + slope * (x1 - x0);
                let need = stems
                    .iter()
                    .map(|&(sx, _, tip)| {
                        let at = y0 + slope * (sx - x0);
                        if up { at - tip } else { tip - at }
                    })
                    .fold(0.0f32, f32::max);
                if up {
                    y0 -= need;
                    y1 -= need;
                } else {
                    y0 += need;
                    y1 += need;
                }
                let at = |sx: f32| y0 + (y1 - y0) * (sx - x0) / (x1 - x0).max(0.1);
                for (k, &(sx, attach, _)) in stems.iter().enumerate() {
                    self.push(Item::line(sx, attach, sx, at(sx), st, n.graces[k].id));
                }
                let bt = d.beam_thickness * s;
                let gap = (d.beam_spacing + d.beam_thickness) * s;
                let id = n.graces[0].id;
                for b in 0..beams(n.graces.iter().map(|g| g.base).min().unwrap_or(8)) {
                    let off = b as f32 * gap * if up { 1.0 } else { -1.0 };
                    let (a, c) = (at(x0 - st / 2.0) + off, at(x1 + st / 2.0) + off);
                    let th = if up { bt } else { -bt };
                    self.push(Item::Quad { p: [pt(x0 - st / 2.0, a), pt(x1 + st / 2.0, c), pt(x1 + st / 2.0, c + th), pt(x0 - st / 2.0, a + th)], id });
                }
                if n.graces[0].slash {
                    let (sx, _, _) = stems[0];
                    let y = at(sx) + if up { 1.1 } else { -1.1 };
                    self.push(Item::line(sx - 0.55, y + if up { 0.45 } else { -0.45 }, sx + 0.75, y - if up { 0.45 } else { -0.45 }, st * 1.1, id));
                }
            } else {
                let (sx, attach, tip) = stems[0];
                let g = &n.graces[0];
                self.push(Item::line(sx, attach, sx, tip, st, g.id));
                let f = flag_glyph(g.base as i64, up);
                self.push(Item::glyph_scaled(f, sx - st / 2.0, tip, s, g.id));
                if g.slash {
                    let y = tip + if up { 1.0 } else { -1.0 };
                    self.push(Item::line(sx - 0.5, y + if up { 0.45 } else { -0.45 }, sx + 0.8, y - if up { 0.5 } else { -0.5 }, st * 1.1, g.id));
                }
            }
        }
    }

    /// Tablature under the staves of fretted instruments: a line per string
    /// (1.5 spaces apart, the highest on top), "TAB" at the start, and each
    /// note's fret on its string, the line broken around the number.
    pub(super) fn draw_tabs(&mut self) {
        const LINE_GAP: f32 = 1.5;
        const FRET_SIZE: f32 = 1.7;
        for part in 0..self.score.parts.len() {
            let tuning = self.score.parts[part].tab.clone();
            if tuning.is_empty() {
                continue;
            }
            let strings = tuning.len();
            let top = self.top(part);
            // Below the staff's lowest note or stem.
            let deep = self
                .notes
                .iter()
                .filter(|n| n.part == part && !n.rest)
                .map(|n| {
                    let low = self.y(part, n.heads[0].pos) + 0.5;
                    if n.has_stem() && !n.stem_up { low.max(n.stem_end) } else { low }
                })
                .fold(top + 4.0, f32::max);
            let tab_top = (top + 4.0 + 3.0).max(deep + 2.0);
            let line_y = |k: usize| tab_top + (strings - 1 - k) as f32 * LINE_GAP; // k: string, lowest 0
            // Frets: each head on the highest free string it can be played on.
            let mut gaps: Vec<Vec<(f32, f32)>> = vec![Vec::new(); strings];
            let mut frets = Vec::new();
            for n in self.notes.iter().filter(|n| n.part == part && !n.rest && n.first) {
                let mut free: Vec<bool> = vec![true; strings];
                for h in n.heads.iter().rev() {
                    if h.tied_in {
                        continue;
                    }
                    const SEMIS: [i32; 7] = [0, 2, 4, 5, 7, 9, 11];
                    let key = (h.dia.div_euclid(7) + 1) * 12 + SEMIS[h.dia.rem_euclid(7) as usize] + h.alter as i32;
                    let Some(k) = (0..strings).rev().find(|&k| free[k] && (0..=24).contains(&(key - tuning[k] as i32))) else { continue };
                    free[k] = false;
                    let fret = key - tuning[k] as i32;
                    let x = n.x + HEAD_W / 2.0;
                    let half = 0.45 * fret.to_string().len() as f32 * FRET_SIZE / 1.7 + 0.15;
                    gaps[k].push((x - half, x + half));
                    frets.push((fret, x, line_y(k), h.id));
                }
            }
            let staves: Vec<(f32, f32)> = self.out.staves.iter().filter(|s| s.part == part).map(|s| (s.left, s.right)).collect();
            let lw = self.font.defaults.staff_line_thickness;
            for (left, right) in staves {
                for (k, g) in gaps.iter_mut().enumerate() {
                    g.sort_by(|a, b| a.0.total_cmp(&b.0));
                    let y = line_y(k);
                    let mut x = left;
                    for &(a, b) in g.iter() {
                        if a > x {
                            self.push(Item::line(x, y, a.min(right), y, lw, 0));
                        }
                        x = x.max(b);
                    }
                    if x < right {
                        self.push(Item::line(x, y, right, y, lw, 0));
                    }
                }
                self.push(Item::line(left, tab_top, left, line_y(0), self.font.defaults.thin_barline_thickness, 0));
                let bars: Vec<f32> = self.ms.iter().map(|m| m.x1).filter(|&x| x > left + 1.0 && x <= right + 0.01).collect();
                for x in bars {
                    self.push(Item::line(x, tab_top, x, line_y(0), self.font.defaults.thin_barline_thickness, 0));
                }
                let clef = if strings <= 4 { Glyph::TabClef4 } else { Glyph::TabClef6 };
                self.push(Item::glyph(clef, left + 1.0, (tab_top + line_y(0)) / 2.0, 0));
            }
            for (fret, x, y, id) in frets {
                self.push(Item::Text { text: fret.to_string(), at: pt(x, y + 0.55), size: FRET_SIZE, italic: false, bold: false, anchor: 1, id });
            }
        }
    }

    pub(super) fn dots(&mut self, n: &Note, right: f32, positions: &[i32]) {
        if n.value.dots == 0 {
            return;
        }
        let rows = if n.rest {
            positions.first().map(|&p| vec![p]).unwrap_or_default()
        } else {
            // A voice in polyphony prefers its own side (Dots.direction).
            let poly = self.notes.iter().any(|o| o.part == n.part && o.measure == n.measure && o.voice != n.voice && !o.whole_bar);
            let dir = if !poly { 0 } else if n.voice.is_multiple_of(2) { -1 } else { 1 };
            dot_rows(positions, dir)
        };
        for dp in rows {
            for k in 0..n.value.dots {
                let x = right + 0.25 + k as f32 * 0.5;
                let y = if n.rest { self.top(n.part) + 1.5 } else { self.y(n.part, dp) };
                self.push(Item::glyph(Glyph::Dot, x, y, n.id));
            }
        }
    }
}
