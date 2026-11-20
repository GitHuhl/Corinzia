//! Staves and what stands on them: lines, clefs, keys and times, barlines,
//! bar numbers, instrument names, braces and brackets.

use super::*;

pub(super) const NAME_SIZE: f32 = 2.0;

pub(super) const CLEF_CHANGE_SCALE: f32 = 0.8;

/// Bar numbers at system starts (LilyPond's font-size -2).
pub(super) const BAR_NUMBER_SIZE: f32 = 1.6;

/// The brace before a grand staff, and its gap to the system's barline.
pub(super) const BRACE_WIDTH: f32 = 1.3;

pub(super) const BRACE_GAP: f32 = 0.4;

/// A family's bracket: LilyPond's SystemStartBracket thickness, its gap to
/// the system's barline, and the room it takes with its serifs.
pub(super) const BRACKET_THICK: f32 = 0.45;

pub(super) const BRACKET_GAP: f32 = 0.35;

pub(super) const BRACKET_ROOM: f32 = 1.5;

/// Room left of the system's barline for braces and brackets.
pub(super) fn decoration_width(score: &Score) -> f32 {
    let brace = if score.parts.iter().any(|p| p.brace) { BRACE_WIDTH + BRACE_GAP } else { 0.0 };
    let bracket = if score.parts.iter().any(|p| p.group != 0) { BRACKET_ROOM } else { 0.0 };
    brace.max(bracket)
}

/// Barlines run on from staff `pi` to the next: a grand staff, or a family
/// under one bracket.
pub(super) fn joined_to_next(score: &Score, pi: usize) -> bool {
    let (Some(a), Some(b)) = (score.parts.get(pi), score.parts.get(pi + 1)) else { return false };
    a.brace || a.group != 0 && a.group == b.group
}

/// A brace from `top` to `bottom` whose right edge is at `right`: two
/// filled halves meeting at a point in the middle, thickest half-way along
/// each.
pub(super) fn brace(right: f32, top: f32, bottom: f32) -> [Item; 2] {
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
    /// Full name on the first system, the abbreviation (if any) after it.
    pub(super) fn part_name(&self, part: usize) -> String {
        let p = &self.score.parts[part];
        // Later systems carry only the abbreviation (none if there is none).
        if self.first_system { p.name.clone() } else { p.abbr.clone() }
    }

    // ------------------------------------------------------------- drawing --
    pub(super) fn draw_staves(&mut self) {
        let t = self.font.defaults.staff_line_thickness;
        let right = self.out.width - self.opt.margin;
        for pi in 0..self.score.parts.len() {
            let top = self.top(pi);
            let audio = self.score.parts[pi].audio;
            self.out.staves.push(StaffGeom { part: pi, top, left: self.left, right, line_width: t, system: 0, single: audio });
            // Clef at the system start (the one in force; an audio staff has
            // none); the instrument name left of the staff.
            if !audio {
                let clef = self.clef_at(pi, self.range.0);
                let (g, dy) = clef_glyph(&clef);
                self.push(Item::glyph(g, self.left + 0.8, top + dy, 0));
            }
            // A grand staff is named once, between its staves, left of its brace.
            let braced = self.score.parts[pi].brace && pi + 1 < self.score.parts.len();
            let second = pi > 0 && self.score.parts[pi - 1].brace;
            let name = self.part_name(pi);
            if self.opt.names && !name.is_empty() && !second {
                let y = if braced { (top + self.top(pi + 1) + 4.0) / 2.0 + 0.7 } else { top + 2.7 };
                let x = self.left - decoration_width(self.score) - 1.0;
                self.push(Item::Text { text: name, at: pt(x, y), size: NAME_SIZE, italic: false, bold: false, anchor: 2, id: 0 });
            }
            if braced {
                let bottom = self.top(pi + 1) + 4.0;
                for item in brace(self.left - BRACE_GAP, top, bottom) {
                    self.push(item);
                }
            }
        }
        // Family brackets: a thick line with serifs at both ends.
        let mut pi = 0;
        while pi < self.score.parts.len() {
            let g = self.score.parts[pi].group;
            let mut last = pi;
            while g != 0 && last + 1 < self.score.parts.len() && self.score.parts[last + 1].group == g {
                last += 1;
            }
            if g != 0 && last > pi {
                let (top, bottom) = (self.top(pi), self.top(last) + 4.0);
                let x = self.left - BRACKET_GAP - BRACKET_THICK;
                self.push(Item::line(x + BRACKET_THICK / 2.0, top - 0.25, x + BRACKET_THICK / 2.0, bottom + 0.25, BRACKET_THICK, 0));
                self.push(Item::glyph(Glyph::BracketTop, x, top - 0.25, 0));
                self.push(Item::glyph(Glyph::BracketBottom, x, bottom + 0.25, 0));
            }
            pi = last + 1;
        }
        // System barline at the left edge.
        if let Some(last) = self.score.parts.len().checked_sub(1) {
            let thin = self.font.defaults.thin_barline_thickness;
            self.push(Item::line(self.left, 0.0, self.left, self.top(last) + 4.0, thin, 0));
        }
    }

    pub(super) fn draw_measure_furniture(&mut self) {
        let d = self.font.defaults;
        let clef_w = self.score.parts.iter().map(|p| self.width(clef_glyph(&p.clef).0)).fold(0.0f32, f32::max);
        for mi in 0..self.ms.len() {
            let m = self.ms[mi].clone();
            let mut x = if mi == 0 { self.left + 0.8 + clef_w + 0.8 } else { self.ms[mi - 1].x1 + 1.0 };
            for pi in 0..self.score.parts.len() {
                // An audio staff: no key or time signature.
                if self.score.parts[pi].audio {
                    continue;
                }
                let mut kx = x;
                let clef = self.clef_at(pi, m.start);
                if let Some(k) = m.key.filter(|_| clef != "percussion") {
                    let k = Key { fifths: part_key(self.score, pi, k.fifths, m.start), ..k };
                    // Cancel the previous key's accidentals with naturals when it changes to fewer/other.
                    if mi > 0 {
                        let prev_at = self.ms[mi - 1].start;
                        let prev = part_key(self.score, pi, key_at(self.score, prev_at), prev_at);
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
                // Barlines run through a grand staff and a bracketed family.
                let joined = joined_to_next(self.score, pi);
                let (top, bot) = (self.top(pi), if joined { self.top(pi + 1) } else { self.top(pi) + 4.0 });
                // Across an audio staff's one line: a space each way.
                let (top, bot) = if self.score.parts[pi].audio && !joined { (top + 1.0, top + 3.0) } else { (top, bot) };
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

    pub(super) fn clef_at(&self, part: usize, at: Frac) -> String {
        let p = &self.score.parts[part];
        p.clefs.iter().filter(|c| c.at <= at).max_by(|a, b| a.at.cmp(&b.at)).map(|c| c.clef.clone()).unwrap_or(p.clef.clone())
    }

    /// x of a written position: its column, or interpolated between columns.
    pub(super) fn col_x(&self, t: Frac) -> f32 {
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
    pub(super) fn bar_x(&self, t: Frac) -> f32 {
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

    pub(super) fn draw_clef_changes(&mut self) {
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
}
