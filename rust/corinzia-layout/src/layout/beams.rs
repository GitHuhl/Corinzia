//! Beam groups (LilyPond's default grouping) and drawing the beams placed by
//! `crate::beam`.

use super::*;

impl<'a> Engraver<'a> {
    /// Beat groups -> beamed runs; their stem direction is the group's.
    /// Beam groups by LilyPond's defaults: one beam per beat (dotted
    /// quarters in compound meters, halves in 2/2 and 3/2), except that
    /// beams of eighths only span half a bar in 4/4 and the bar in 3/4.
    pub(super) fn beam_groups(&mut self) -> Vec<Vec<usize>> {
        let beat_of = |m: &Measure| beam_beat(m.num, m.den);
        let eighths_span = |m: &Measure| corinzia_score::beams::eighths_span(m.num, m.den);
        // Notes in the same window of `span` (per voice and bar) share a
        // beam; beams set by hand break, join or leave notes out.
        let group = |this: &Self, idx: &[usize], span: &dyn Fn(&Measure) -> Frac| -> Vec<Vec<usize>> {
            let mut groups = Vec::new();
            let mut cur: Vec<usize> = Vec::new();
            let mut cur_key = None;
            // A join merges the note's whole window into the beam before it.
            let mut joined: Option<((usize, u8, usize, i64, Option<usize>), (usize, u8, usize, i64, Option<usize>))> = None;
            for &i in idx {
                let n = &this.notes[i];
                let m = &this.ms[n.measure];
                let window = beam_window(m, n.start - m.start + m.phase, span(m));
                let mut key = (n.home, n.voice, n.measure, window, n.tuplets.last().copied());
                if n.beam & span::BEAM_JOIN != 0 {
                    if let Some(k) = cur_key.filter(|k: &(usize, u8, usize, i64, Option<usize>)| (k.0, k.1, k.2) == (n.home, n.voice, n.measure)) {
                        joined = Some((key, k));
                        key = k;
                    }
                } else if let Some((_, to)) = joined.filter(|(from, _)| *from == key) {
                    key = to;
                }
                let beamable = !n.rest && n.value.base >= 8 && n.beam & span::NO_BEAM == 0;
                if !beamable || cur_key != Some(key) || n.beam & span::BEAM_BREAK != 0 {
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
        // In voice order, cross-staff notes with their own voice.
        let mut all: Vec<usize> = (0..self.notes.len()).collect();
        all.sort_by_key(|&i| (self.notes[i].home, self.notes[i].voice, self.notes[i].start));
        let wide = group(self, &all, &|m: &Measure| eighths_span(m).unwrap_or_else(|| beat_of(m)));
        // A wide beam holding shorter notes than eighths goes back to beats.
        let mut groups = Vec::new();
        for g in wide {
            if g.iter().any(|&i| self.notes[i].beams() > 1) {
                groups.extend(group(self, &g, &beat_of));
                continue;
            }
            // Grace notes on a beat end the beam there, as LilyPond's
            // auto-beamer ends a beam at a beat where graces come between.
            let mut cur: Vec<usize> = Vec::new();
            for &i in &g {
                let n = &self.notes[i];
                let m = &self.ms[n.measure];
                let beat = beat_of(m);
                let on_beat = (n.start - m.start + m.phase) * Frac::new(beat.den, beat.num);
                if !cur.is_empty() && !n.graces.is_empty() && on_beat.den == 1 {
                    if cur.len() >= 2 {
                        groups.push(std::mem::take(&mut cur));
                    }
                    cur.clear();
                }
                cur.push(i);
            }
            if cur.len() >= 2 {
                groups.push(cur);
            }
        }
        for g in &groups {
            // A beam across two staves goes between them: stems down from
            // the upper staff, up from the lower.
            let top = g.iter().map(|&i| self.notes[i].part).min().unwrap_or(0);
            if g.iter().any(|&i| self.notes[i].part != top) {
                for &i in g {
                    self.notes[i].stem_up = self.notes[i].part != top;
                    self.notes[i].beamed = true;
                }
                continue;
            }
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

    /// The beam of a group, placed by LilyPond's beam quanting (see
    /// `crate::beam`). Returns the outer edge at the first stem and the
    /// slope, page y down.
    pub(super) fn quant_beam(&self, g: &[usize], up: bool, bt: f32, gap: f32) -> (f32, f32) {
        let dir = if up { 1.0 } else { -1.0 };
        let part = self.notes[g[0]].part;
        let mid = self.top(part) + 2.0;
        let stems: Vec<crate::beam::Stem> = g
            .iter()
            .map(|&i| {
                let n = &self.notes[i];
                crate::beam::Stem { x: n.stem_x, low: n.heads[0].pos, high: n.heads[n.heads.len() - 1].pos, dir: 0.0, middle: 0.0 }
            })
            .collect();
        let counts: Vec<u32> = g.iter().map(|&i| self.notes[i].beams().max(1)).collect();
        // What else is under or over the beam on this staff: other notes'
        // heads, rests, accidentals and stems, clefs (LilyPond's covered grobs).
        let (xa, xb) = (stems[0].x, stems[stems.len() - 1].x);
        let own: std::collections::HashSet<u64> = g.iter().flat_map(|&i| std::iter::once(self.notes[i].id).chain(self.notes[i].heads.iter().map(|h| h.id))).collect();
        let top = self.top(part);
        let collisions: Vec<(f32, (f32, f32), f32)> = self
            .out
            .items
            .iter()
            .filter(|it| !own.contains(&it.id()) && !matches!(it, Item::Crescent { .. } | Item::Quad { .. } | Item::Text { .. }))
            .filter_map(|it| {
                let r = it.bounds(self.font);
                if r[2] < xa || r[0] > xb || r[3] < top - 8.0 || r[1] > top + 12.0 {
                    return None;
                }
                let (lo, hi) = (mid - r[3], mid - r[1]); // y up from the middle line
                match it {
                    // Stems (upright lines) count a tenth; staff and ledger lines not.
                    Item::Line { from, to, .. } if (from.x - to.x).abs() < 1e-3 && (to.y - from.y).abs() > 1.0 => Some(vec![((from.x + to.x) / 2.0, (lo, hi), 0.1)]),
                    Item::Line { .. } => None,
                    _ => {
                        let f = (r[2] - r[0]).max(0.0).sqrt();
                        Some(vec![(r[0], (lo, hi), f), (r[2], (lo, hi), f)])
                    }
                }
            })
            .flatten()
            .collect();
        let pb = crate::beam::Problem {
            collisions,
            knee: false,
            cross_staff: false,
            stems,
            dir,
            beam_count: counts.iter().copied().max().unwrap_or(1),
            edge_counts: [counts[0], counts[counts.len() - 1]],
            thickness: bt,
            translation: gap,
            line_thickness: self.font.defaults.staff_line_thickness,
        };
        let [l, r] = crate::beam::solve(&pb);
        let dx = pb.stems[pb.stems.len() - 1].x - pb.stems[0].x;
        // Back to page coordinates: the outer edge, y down.
        let edge = |c: f32| mid - (c + dir * bt / 2.0);
        let slope = if dx > 0.0 { (edge(r) - edge(l)) / dx } else { 0.0 };
        (edge(l), slope)
    }

    /// A kneed beam (stems down from the upper staff, up from the lower),
    /// placed by LilyPond's beam quanting for knees (see `crate::beam`).
    /// Returns the top beam's upper edge at the first stem and the slope,
    /// page y down.
    pub(super) fn quant_knee(&self, g: &[usize], bt: f32, gap: f32) -> (f32, f32) {
        let top_part = g.iter().map(|&i| self.notes[i].part).min().unwrap_or(0);
        let ref_mid = self.top(top_part) + 2.0;
        let stems: Vec<crate::beam::Stem> = g
            .iter()
            .map(|&i| {
                let n = &self.notes[i];
                crate::beam::Stem {
                    x: n.stem_x,
                    low: n.heads[0].pos,
                    high: n.heads[n.heads.len() - 1].pos,
                    dir: if n.stem_up { 1.0 } else { -1.0 },
                    middle: ref_mid - (self.top(n.part) + 2.0),
                }
            })
            .collect();
        let counts: Vec<u32> = g.iter().map(|&i| self.notes[i].beams().max(1)).collect();
        let levels = counts.iter().copied().max().unwrap_or(1);
        let pb = crate::beam::Problem {
            stems,
            dir: 1.0,
            beam_count: levels,
            edge_counts: [counts[0], counts[counts.len() - 1]],
            thickness: bt,
            translation: gap,
            line_thickness: self.font.defaults.staff_line_thickness,
            collisions: vec![],
            knee: true,
            cross_staff: g.iter().any(|&i| self.notes[i].part != top_part),
        };
        let [l, r] = crate::beam::solve(&pb);
        let dx = pb.stems[pb.stems.len() - 1].x - pb.stems[0].x;
        // The stack's middle -> its top edge, page y down.
        let top_edge = |c: f32| ref_mid - (c + (levels as f32 - 1.0) * gap / 2.0 + bt / 2.0);
        let slope = if dx > 0.0 { (top_edge(r) - top_edge(l)) / dx } else { 0.0 };
        (top_edge(l), slope)
    }

    pub(super) fn draw_beams(&mut self, groups: &[Vec<usize>]) {
        let d = self.font.defaults;
        for g in groups {
            let s = if self.notes[g[0]].art & art::CUE != 0 { CUE_SCALE } else { 1.0 };
            let (bt, gap) = (d.beam_thickness * s, (d.beam_thickness + d.beam_spacing) * s);
            let mut up = self.notes[g[0]].stem_up;
            let (fi, li) = (g[0], *g.last().unwrap());
            let (xf, xl) = (self.notes[fi].stem_x, self.notes[li].stem_x);
            let levels = g.iter().map(|&i| self.notes[i].beams()).max().unwrap_or(1);
            let kneed = g.iter().any(|&i| self.notes[i].part != self.notes[g[0]].part);
            let (y0, slope) = if kneed {
                // Across the staves, by LilyPond's quanting for knees; drawn
                // from the top beam's upper edge down.
                up = true;
                self.quant_knee(g, bt, gap)
            } else {
                self.quant_beam(g, up, bt, gap)
            };
            let beam_y = |x: f32| y0 + slope * (x - xf);
            let beam_id = self.notes[fi].id;
            // Stems to the beam.
            for &i in g {
                let n = self.notes[i].clone();
                let (lo, hi) = (n.heads[0].pos, n.heads[n.heads.len() - 1].pos);
                let from = if n.stem_up { self.y(n.part, lo) - 0.168 * s } else { self.y(n.part, hi) + 0.168 * s };
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
                        let item = if s + 1 < g.len() && s == 0 {
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

    /// True if the notes of `g` are exactly one beam group.
    pub(super) fn beam_spans(&self, g: &[usize]) -> bool {
        let b = self.beam_of.get(g[0]).copied().unwrap_or(0);
        b != 0 && g.iter().all(|&i| self.beam_of.get(i) == Some(&b)) && self.beam_of.iter().filter(|&&x| x == b).count() == g.len()
    }
}
