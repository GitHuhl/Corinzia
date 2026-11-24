//! Tuplet brackets and numbers.
//!
//! Ported from GNU LilyPond (`lily/tuplet-bracket.cc`), Copyright (C)
//! 1997–2026 Han-Wen Nienhuys, Jan Nieuwenhuizen and the LilyPond authors,
//! under the GNU General Public License, version 3 or later (see
//! `THIRD-PARTY-NOTICES.md`).

use super::*;

impl<'a> Engraver<'a> {
    /// Triplet groups: a number on the stem side (beamed) or a bracket with
    /// the number in a gap.
    pub(super) fn draw_tuplets(&mut self) {
        let d = self.font.defaults;
        // Explicit tuplets, innermost first so outer brackets clear them.
        let list = tuplet_list(self.score);
        let mut groups: Vec<(usize, i64, Vec<usize>)> = Vec::new(); // (depth, number, notes)
        for (k, t) in list.iter().enumerate() {
            let notes: Vec<usize> = (0..self.notes.len()).filter(|&i| self.notes[i].tuplets.contains(&k)).collect();
            if let Some(&i) = notes.first() {
                let depth = self.notes[i].tuplets.iter().position(|&x| x == k).unwrap_or(0);
                groups.push((depth, t.2.actual, notes));
            }
        }
        groups.sort_by_key(|g| std::cmp::Reverse(g.0));
        // Triplet values outside any explicit tuplet: grouped by whole
        // undotted spans (imported or typed as durations alone).
        let mut cur: Vec<usize> = Vec::new();
        let mut sum = Frac::ZERO;
        let mut min_plain = Frac::int(4);
        for i in 0..self.notes.len() {
            let n = &self.notes[i];
            let implicit = n.value.triplet && n.tuplets.is_empty();
            let contiguous = cur.last().is_some_and(|&p| {
                let o = &self.notes[p];
                o.part == n.part && o.voice == n.voice && o.start + o.dur == n.start
            });
            if !implicit || !cur.is_empty() && !contiguous {
                cur.clear();
                sum = Frac::ZERO;
                min_plain = Frac::int(4);
                if !implicit {
                    continue;
                }
            }
            cur.push(i);
            sum = sum + n.dur;
            min_plain = min_plain.min(n.dur * Frac::new(3, 2));
            let pow2 = sum.num == 1 && sum.den & (sum.den - 1) == 0;
            if pow2 && sum >= min_plain * Frac::int(2) {
                groups.push((0, 3, std::mem::take(&mut cur)));
                sum = Frac::ZERO;
                min_plain = Frac::int(4);
            }
        }
        // What each note column covers: its heads, stem, flag, ledger
        // lines and articulations (LilyPond's note column with its scripts).
        let mut owner: std::collections::HashMap<u64, usize> = Default::default();
        for (i, n) in self.notes.iter().enumerate() {
            owner.insert(n.id, i);
            for h in &n.heads {
                owner.insert(h.id, i);
            }
        }
        let mut ext: Vec<Option<[f32; 4]>> = vec![None; self.notes.len()];
        for it in &self.out.items {
            if let Some(&i) = owner.get(&it.id()) {
                let r = it.bounds(self.font);
                ext[i] = Some(match ext[i] {
                    Some(e) => [e[0].min(r[0]), e[1].min(r[1]), e[2].max(r[2]), e[3].max(r[3])],
                    None => r,
                });
            }
        }
        let th = d.stem_thickness;
        // Brackets already drawn (inner ones first): (x0, x1, top, bottom).
        let mut drawn: Vec<(usize, [f32; 4])> = Vec::new();
        for (_, number, g) in groups {
            let notes: Vec<&Note> = g.iter().map(|&i| &self.notes[i]).collect();
            let (first, last) = (notes[0], notes[notes.len() - 1]);
            let part = first.part;
            let top = self.top(part);
            let id = first.id;
            // LilyPond's get_default_dir: the side most stems point to; on a
            // tie, the side whose outermost head lies further out. A voice
            // in polyphony keeps its own side.
            let poly = self.notes.iter().any(|o| o.part == part && o.measure == first.measure && o.voice != first.voice && !o.whole_bar);
            let sounding: Vec<&Note> = notes.iter().copied().filter(|n| !n.rest).collect();
            let up = sounding.iter().filter(|n| n.stem_up).count();
            let down = sounding.len() - up;
            let above = if poly {
                first.voice % 2 == 1
            } else if up != down {
                up > down
            } else if up == 0 {
                true
            } else {
                let hi = sounding.iter().filter(|n| n.stem_up).map(|n| n.heads[n.heads.len() - 1].pos).max().unwrap_or(0) as f32;
                let lo = sounding.iter().filter(|n| !n.stem_up).map(|n| n.heads[0].pos).min().unwrap_or(0) as f32;
                hi - 2.0 <= -2.0 - lo
            };
            let s = if above { -1.0 } else { 1.0 }; // outwards, y down
            let out = |r: [f32; 4]| if above { r[1] } else { r[3] };
            let further = |a: f32, b: f32| if above { a.min(b) } else { a.max(b) };
            // The bounds: the stem when it points the bracket's way, else
            // the column; shorten-pair -0.2 lengthens both ends.
            let stem_side = |n: &Note| n.has_stem() && !n.rest && n.stem_up == above;
            let col = |i: usize| ext[i].unwrap_or([self.notes[i].x, top + 2.0, self.notes[i].x + HEAD_W, top + 2.0]);
            let x0 = if stem_side(first) { first.stem_x - th / 2.0 } else { col(g[0])[0] } - 0.2;
            let x1 = if stem_side(last) { last.stem_x + th / 2.0 } else { col(*g.last().unwrap())[2] } + 0.2;
            let staff = (top - 0.25, top + 4.25); // staff-padding
            let staff_out = if above { staff.0 } else { staff.1 };
            // The number alone over one beam that spans exactly the tuplet
            // (bracket-visibility if-no-beam); the line then follows the beam.
            let beamed = g.iter().all(|&i| self.notes[i].beamed) && self.beam_spans(&g);
            let follow_beam = beamed && sounding.iter().all(|n| n.stem_up == above) && !sounding.is_empty();
            let mut points: Vec<(f32, f32)> = Vec::new();
            let mut dy = 0.0;
            if follow_beam {
                let (f, l) = (sounding[0], sounding[sounding.len() - 1]);
                let (yl, yr) = (f.stem_end + s * 0.25, l.stem_end + s * 0.25);
                dy = yr - yl;
                points.push((f.stem_x - x0, yl));
                points.push((l.stem_x - x0, yr));
            } else {
                // The slope of the outer sounding columns, if their heads
                // agree on it, at most max-slope-factor (0.5) per space.
                if let (Some(l), Some(r)) = (g.iter().position(|&i| !self.notes[i].rest), g.iter().rposition(|&i| !self.notes[i].rest)) {
                    let (ln, rn) = (&self.notes[g[l]], &self.notes[g[r]]);
                    let unite = |e: [f32; 4]| [e[0], e[1].min(staff.0), e[2], e[3].max(staff.1)];
                    let graphical = out(unite(col(g[r]))) - out(unite(col(g[l])));
                    let sg = |v: f32| if v > 0.0 { 1 } else if v < 0.0 { -1 } else { 0 };
                    let mu = rn.heads[rn.heads.len() - 1].pos - ln.heads[ln.heads.len() - 1].pos;
                    let md = rn.heads[0].pos - ln.heads[0].pos;
                    // Head positions count up, y down.
                    dy = if mu.signum() != md.signum() || sg(-graphical) != md.signum() { 0.0 } else { graphical };
                }
                let mut last_x = 0.0;
                for &i in &g {
                    let n = &self.notes[i];
                    last_x = n.x - x0;
                    points.push((last_x, out(col(i))));
                }
                let max_dy = 0.5 * last_x;
                if dy.abs() > max_dy {
                    dy = max_dy * dy.signum();
                }
                points.push((0.0, staff_out));
                points.push((x1 - x0, staff_out));
            }
            // Inner tuplets' brackets and numbers.
            for (p, r) in &drawn {
                if *p == part && r[0] >= x0 - 0.1 && r[1] <= x1 + 0.1 {
                    let y = if above { r[2] } else { r[3] };
                    points.push((r[0] - x0, y));
                    points.push((r[1] - x0, y));
                }
            }
            let factor = if g.len() > 1 { 1.0 / (x1 - x0) } else { 1.0 };
            let mut offset = points.iter().map(|&(x, y)| y - dy * x * factor).fold(if above { f32::MAX } else { f32::MIN }, further);
            offset += s * 1.1; // padding
            if dy.abs() < 0.01 {
                // Level brackets keep off the staff lines.
                let mut p = (top + 2.0 - offset) * 2.0;
                if p.abs() <= 6.0 {
                    p = p.round();
                    if (p as i32) % 2 == 0 && p.abs() <= 4.0 {
                        p -= s;
                    }
                    offset = top + 2.0 - p / 2.0;
                }
            }
            let (yl, yr) = (offset, offset + dy);
            let y_at = |x: f32| yl + (yr - yl) * (x - x0) / (x1 - x0).max(1e-3);
            // The number, centred on the bracket (font-size -2 in LilyPond:
            // SMuFL's tuplet digits here).
            let digits: Vec<Glyph> = number.to_string().bytes().map(|c| Glyph::TupletDigit(c - b'0')).collect();
            let boxes: Vec<_> = digits.iter().map(|&g| self.font.bbox(g)).collect();
            let nw: f32 = boxes.iter().map(|b| b.width()).sum();
            let (sw, ne) = (boxes.iter().map(|b| b.sw.1).fold(0.0f32, f32::min), boxes.iter().map(|b| b.ne.1).fold(0.0f32, f32::max));
            let xm = (x0 + x1) / 2.0;
            let ym = (yl + yr) / 2.0;
            let gap = nw + 1.0;
            let bracket = !beamed && g.len() > 1 && gap <= x1 - x0;
            let mut x = xm - nw / 2.0;
            for (gl, b) in digits.iter().zip(&boxes) {
                self.place(part, Item::glyph(*gl, x, ym + (ne + sw) / 2.0, id));
                x += b.width();
            }
            let mut bb = [xm - nw / 2.0, ym - (ne - sw) / 2.0, xm + nw / 2.0, ym + (ne - sw) / 2.0];
            if bracket {
                let w = d.tuplet_bracket_thickness;
                let hook = -s * 0.7; // edge-height, towards the notes
                let (ga, gb) = (xm - gap / 2.0 + 0.1, xm + gap / 2.0 + 0.1);
                self.place(part, Item::line(x0, yl + hook, x0, yl, w, id));
                self.place(part, Item::line(x0, yl, ga, y_at(ga), w, id));
                self.place(part, Item::line(gb, y_at(gb), x1, yr, w, id));
                self.place(part, Item::line(x1, yr, x1, yr + hook, w, id));
                bb = [x0, bb[1].min(yl.min(yr)), x1, bb[3].max(yl.max(yr))];
            }
            drawn.push((part, bb));
        }
    }
}
