//! Dynamics and hairpins below the staff.
//!
//! Ported from GNU LilyPond (`lily/hairpin.cc`,
//! `lily/side-position-interface.cc`, the DynamicLineSpanner and Hairpin in
//! `scm/define-grobs.scm`), Copyright (C) 1997–2026 Han-Wen Nienhuys, Jan
//! Nieuwenhuizen and the LilyPond authors, under the GNU General Public
//! License, version 3 or later (see `THIRD-PARTY-NOTICES.md`).

use super::*;

/// How far above a dynamic's baseline its letters' middle (an 'm') sits:
/// hairpins are centred there (LilyPond's DynamicText Y-offset).
pub(super) const DYN_CENTRE: f32 = 0.6;

/// DynamicLineSpanner `padding`.
pub(super) const DYN_PAD: f32 = 0.6;

/// The least room between a dynamic and a barline beside it.
pub(super) const DYN_BAR_PAD: f32 = 0.4;

impl<'a> Engraver<'a> {
    pub(super) fn dynamic_glyphs(text: &str) -> Vec<Glyph> {
        text.chars()
            .filter_map(|c| match c {
                'p' => Some(Glyph::DynamicP),
                'm' => Some(Glyph::DynamicM),
                'f' => Some(Glyph::DynamicF),
                'r' => Some(Glyph::DynamicR),
                's' => Some(Glyph::DynamicS),
                'z' => Some(Glyph::DynamicZ),
                'n' => Some(Glyph::DynamicN),
                _ => None,
            })
            .collect()
    }

    /// Hairpins and dynamics below the staff: a hairpin and the dynamics at
    /// its ends share one baseline; simultaneous dynamics stack.
    pub(super) fn draw_dynamics_and_hairpins(&mut self) {
        let d = self.font.defaults;
        let dyn_width = |s: &Self, text: &str| Self::dynamic_glyphs(text).iter().map(|g| s.width(*g) - 0.05).sum::<f32>();
        // A dynamic's left edge: centred on its note's head (LilyPond's
        // DynamicText), but kept inside its bar. LilyPond leaves dynamics out
        // of the horizontal spacing and relies on the gap after a barline;
        // a squeezed system, or a wide dynamic, would run over the barline
        // (or the clef and key at a system's start), so it moves right to
        // clear it, or left to clear the next barline.
        let dyn_left = |s: &Self, n: &Note, text: &str| -> f32 {
            let w = dyn_width(s, text);
            let x = n.x + HEAD_W / 2.0 - w / 2.0;
            let Some(m) = s.ms.get(n.measure) else { return x };
            let lo = m.x0 + DYN_BAR_PAD;
            let hi = m.x1 - DYN_BAR_PAD - w;
            if x < lo {
                lo
            } else if x > hi {
                hi.max(lo)
            } else {
                x
            }
        };
        let mut baseline_for: std::collections::HashMap<usize, f32> = Default::default();
        let (r0, r1) = self.range;
        let (left, right) = self.system_edges();
        let pins: Vec<Spanner> = self.spanners().into_iter().filter(|s| s.kind != SpanKind::Slur && s.from < r1 && s.to > r0).collect();
        // Where each hairpin starts and ends, as LilyPond places them: just
        // after a dynamic or its first head; at a dynamic, at the end head's
        // right side, or (on the first note of a bar) at the barline; where
        // one hands over to the next on a note, the first stops inside the
        // head and the next starts just after it.
        struct Pin {
            sp: Spanner,
            si: Option<usize>,
            ei: Option<usize>,
            x0: f32,
            x1: f32,
        }
        // Hairpin bound-padding.
        const PIN_PAD: f32 = 1.0;
        let mut placed: Vec<Pin> = Vec::new();
        for sp in pins.iter().cloned() {
            let si = if sp.from >= r0 { self.note_at(sp.part, sp.voice, sp.from) } else { None };
            let ei = if sp.to < r1 { self.note_at(sp.part, sp.voice, sp.to) } else { None };
            // A hand-over on a bar's first note happens at the barline instead.
            let handed_from = !self.is_bar_start(sp.from) && pins.iter().any(|o| o.part == sp.part && o.voice == sp.voice && o.to == sp.from && o.from < sp.from);
            let hands_on = pins.iter().any(|o| o.part == sp.part && o.voice == sp.voice && o.from == sp.to && o.to > sp.to);
            let x0 = match si {
                Some(i) => {
                    let n = &self.notes[i];
                    match &n.dynamic {
                        Some(t) => dyn_left(self, n, t) + dyn_width(self, t) + PIN_PAD,
                        None if handed_from => n.x + HEAD_W / 2.0 + PIN_PAD / 3.0,
                        None => n.x,
                    }
                }
                None => left,
            };
            let x1 = match ei {
                Some(k) => {
                    let o = &self.notes[k];
                    let at_dynamic = o.dynamic.as_ref().map(|t| dyn_left(self, o, t) - PIN_PAD);
                    if self.is_bar_start(sp.to) && sp.to > sp.from {
                        // LilyPond's to-barline: a hairpin ending on a bar's
                        // first note is bound to the barline before it, and
                        // stops its bound-padding short of the line's right side.
                        self.bar_x(sp.to) + d.thin_barline_thickness / 2.0 - PIN_PAD
                    } else if let Some(x) = at_dynamic {
                        x
                    } else if hands_on {
                        o.x + HEAD_W / 2.0 - PIN_PAD / 3.0
                    } else {
                        o.x + HEAD_W
                    }
                }
                // Continues in the next system (or ends at its bar's end).
                None if sp.to >= r1 => right,
                // Ending on a bar that opens with a rest: to-barline still.
                None if self.is_bar_start(sp.to) && sp.to > sp.from => self.bar_x(sp.to) + d.thin_barline_thickness / 2.0 - PIN_PAD,
                // On a rest: its left edge.
                None => self.col_x(sp.to),
            };
            if x1 <= x0 + 0.5 {
                continue;
            }
            placed.push(Pin { sp, si, ei, x0, x1 });
        }
        // Hairpins that touch (one ends where the next starts) share one
        // line, below everything under any of them.
        placed.sort_by(|a, b| (a.sp.part, a.sp.voice, a.sp.from).cmp(&(b.sp.part, b.sp.voice, b.sp.from)));
        let mut k = 0;
        while k < placed.len() {
            let mut end = k + 1;
            while end < placed.len() && placed[end].sp.part == placed[k].sp.part && placed[end].sp.voice == placed[k].sp.voice && placed[end].sp.from <= placed[end - 1].sp.to {
                end += 1;
            }
            let part = placed[k].sp.part;
            // LilyPond's DynamicLineSpanner: the hairpin's open end clear of
            // the staff by 0.1 and of what hangs below it by 0.6, on the
            // line its dynamics are centred on.
            // The skyline is the wedge's own outline, so the narrow end may
            // tuck in under a stem the wide end must clear.
            let base = placed[k..end]
                .iter()
                .map(|p| {
                    let cresc = p.sp.kind == SpanKind::Cresc;
                    let half = |x: f32| {
                        let f = ((x - p.x0) / (p.x1 - p.x0)).clamp(0.0, 1.0);
                        2.0 / 3.0 * if cresc { f } else { 1.0 - f }
                    };
                    let edge = d.hairpin_thickness / 2.0;
                    let staff = self.top(part) + 4.0 + DYN_PAD + 2.0 / 3.0 + edge;
                    self.sky[part]
                        .objects_over(p.x0, p.x1)
                        .map(|b| b[3] + DYN_PAD + half(if cresc { b[2] } else { b[0] }) + edge)
                        .fold(staff, f32::max)
                        + DYN_CENTRE
                })
                .fold(f32::MIN, f32::max);
            // The dynamics at the hairpins' ends are on the same line
            // (LilyPond's DynamicLineSpanner holds them all): the line goes
            // as low as the lowest of them needs.
            let base = placed[k..end]
                .iter()
                .flat_map(|p| [p.si, p.ei])
                .flatten()
                .filter_map(|i| {
                    let n = &self.notes[i];
                    let t = n.dynamic.as_ref().filter(|_| n.first)?;
                    let (x, w) = (dyn_left(self, n, t), dyn_width(self, t));
                    let ascent = Self::dynamic_glyphs(t).iter().map(|g| self.font.bbox(*g).ne.1).fold(0.0f32, f32::max);
                    Some(self.dynamic_clearance(part, x - 0.2, x + w + 0.2) + ascent)
                })
                .fold(base, f32::max);
            for p in &placed[k..end] {
                let sp = &p.sp;
                let c = base - DYN_CENTRE;
                // The opening grows (or shrinks) with time, so the parts of a
                // hairpin cut by a system break continue each other; at its
                // widest, LilyPond's 1⅓ spaces.
                let total = sp.to - sp.from;
                let frac = |t: Frac| -> f32 {
                    if total.is_zero() {
                        return 0.0;
                    }
                    let f = (t.max(sp.from).min(sp.to) - sp.from) * Frac::new(total.den, total.num);
                    (f.num as f32 / f.den as f32).clamp(0.0, 1.0)
                };
                const HALF: f32 = 2.0 / 3.0;
                let open = |f: f32| if sp.kind == SpanKind::Cresc { HALF * f } else { HALF * (1.0 - f) };
                let (fa, fb) = (if p.si.is_some() { 0.0 } else { frac(r0) }, if p.ei.is_some() || sp.to < r1 { 1.0 } else { frac(r1) });
                let (a, b) = (open(fa), open(fb));
                let w = d.hairpin_thickness;
                self.place(part, Item::line(p.x0, c - a, p.x1, c - b, w, sp.id));
                self.place(part, Item::line(p.x0, c + a, p.x1, c + b, w, sp.id));
                if let Some(i) = p.si {
                    baseline_for.insert(i, base);
                }
                if let Some(e) = p.ei {
                    baseline_for.insert(e, base);
                }
            }
            k = end;
        }
        for i in 0..self.notes.len() {
            let n = self.notes[i].clone();
            let Some(text) = n.dynamic.clone() else { continue };
            if !n.first {
                continue;
            }
            let w = dyn_width(self, &text);
            let mut x = dyn_left(self, &n, &text);
            // Its top clear of the staff and the notes by padding 0.6.
            let ascent = Self::dynamic_glyphs(&text).iter().map(|g| self.font.bbox(*g).ne.1).fold(0.0f32, f32::max);
            let own = self.dynamic_clearance(n.part, x - 0.2, x + w + 0.2) + ascent;
            let y = baseline_for.get(&i).map_or(own, |&b| b.max(own));
            for g in Self::dynamic_glyphs(&text) {
                // Bravura's letters reach left of their origin (p by 0.36,
                // f by 0.56): set the ink, not the origin, at `x`.
                let sw = self.font.bbox(g).sw.0;
                self.place(n.part, Item::glyph(g, x - sw, y, ids::dynamic(n.id)));
                x += self.width(g) - 0.05;
            }
        }
    }

    /// Where something under staff `part` over [x0, x1] may start (its
    /// top edge): LilyPond's DynamicLineSpanner `padding`, 0.6 under the
    /// staff and under the notes, slurs and marks there.
    pub(super) fn dynamic_clearance(&self, part: usize, x0: f32, x1: f32) -> f32 {
        // The staff joins the support as its minimum height, and `padding`
        // applies to it as to the rest: staff-padding never comes into play.
        let staff = self.top(part) + 4.0;
        self.sky[part].lowest_object(x0, x1).unwrap_or(staff).max(staff) + DYN_PAD
    }
}
