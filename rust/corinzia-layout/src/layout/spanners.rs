//! Slurs and hairpins over a system, and the slurs placed by `crate::slur`.

use super::*;

/// A slur drawn: (part, above, its midline sampled).
type DrawnSlur = (usize, bool, Vec<(f32, f32)>);

impl<'a> Engraver<'a> {
    /// Slurs and hairpins of the whole movement, so a system can draw the
    /// part of one that started or ends in another system.
    pub(super) fn spanners(&self) -> Vec<Spanner> {
        let mut out = Vec::new();
        // Where each note is: (part, voice, start).
        let mut at: std::collections::HashMap<u64, (usize, u8, Frac)> = Default::default();
        for (pi, part) in self.score.parts.iter().enumerate() {
            for v in &part.voices {
                let notes: Vec<&corinzia_score::score::Event> = v.events.iter().filter(|e| !e.pitches.is_empty()).collect();
                for e in notes.iter() {
                    at.insert(e.id, (pi, v.index, e.start));
                    // Selectable by an id of its own (not its first head's).
                    let id = ids::hairpin(e.id);
                    if e.spans & (span::CRESC_START | span::DIM_START) != 0 {
                        let kind = if e.spans & span::CRESC_START != 0 { SpanKind::Cresc } else { SpanKind::Dim };
                        // It ends at the next dynamic or end mark, on a note or a rest.
                        let to = v.events.iter().filter(|o| o.start > e.start).find(|o| o.dynamic.is_some() || o.spans & span::HAIRPIN_END != 0).map(|o| o.start);
                        // Without an end: to the end of the note's bar.
                        let to = to.unwrap_or_else(|| self.score_bar_end(e.start));
                        out.push(Spanner { part: pi, voice: v.index, to_part: pi, to_voice: v.index, from: e.start, to, kind, id, dir: 0 });
                    }
                }
            }
        }
        // Slurs: from one note to a later one, perhaps in another voice or staff.
        for sl in &self.score.slurs {
            let (Some(&(p0, v0, t0)), Some(&(p1, v1, t1))) = (at.get(&sl.from), at.get(&sl.to)) else { continue };
            if t1 <= t0 {
                continue;
            }
            out.push(Spanner { part: p0, voice: v0, to_part: p1, to_voice: v1, from: t0, to: t1, kind: SpanKind::Slur, id: sl.id, dir: sl.dir });
        }
        out
    }

    /// Whether a bar starts at `t` (whole movement).
    pub(super) fn is_bar_start(&self, t: Frac) -> bool {
        t > Frac::ZERO && self.score_bar_end(t - Frac::new(1, 4096)) == t
    }

    /// The end of the bar containing `t` (whole movement).
    pub(super) fn score_bar_end(&self, t: Frac) -> Frac {
        let mut at = Frac::ZERO;
        loop {
            let m = self.score.meter.iter().rfind(|m| m.at <= at);
            let len = m.map(|m| Frac::new(m.num, m.den)).unwrap_or(Frac::int(1));
            if at + len > t || len.is_zero() {
                return (at + len).min(self.score.length.max(t));
            }
            at = at + len;
        }
    }

    /// The first piece of the event of (part, voice) starting at `t`, in this system.
    pub(super) fn note_at(&self, part: usize, voice: u8, t: Frac) -> Option<usize> {
        self.notes.iter().position(|n| n.part == part && n.voice == voice && n.first && !n.rest && n.start == t)
    }

    /// Where a spanner cut by a system break starts and ends: just after the
    /// system's opening and at its final barline.
    pub(super) fn system_edges(&self) -> (f32, f32) {
        (self.ms.first().map(|m| m.x0 + 0.5).unwrap_or(self.left), self.ms.last().map(|m| m.x1 - 0.3).unwrap_or(self.left))
    }

    /// Slurs, which may nest, overlap and go from one voice or staff to
    /// another: on the notehead side when all stems agree, else above (or
    /// the side chosen by hand); placed and curved over the notes between
    /// as LilyPond does.
    pub(super) fn draw_slurs(&mut self) {
        let d = self.font.defaults;
        let (r0, r1) = self.range;
        let (left, right) = self.system_edges();
        let mut items = Vec::new();
        // Shortest first: a slur around others clears them.
        let mut slurs: Vec<Spanner> = self.spanners().into_iter().filter(|s| s.kind == SpanKind::Slur && s.from < r1 && s.to >= r0).collect();
        slurs.sort_by_key(|a| a.to - a.from);
        // Drawn so far: (part, above, midline samples).
        let mut drawn: Vec<DrawnSlur> = Vec::new();
        for sp in slurs {
            let si = if sp.from >= r0 { self.note_at(sp.part, sp.voice, sp.from) } else { None };
            let ei = if sp.to < r1 { self.note_at(sp.to_part, sp.to_voice, sp.to) } else { None };
            let in_slur = |o: &Note| (o.part, o.voice) == (sp.part, sp.voice) || (o.part, o.voice) == (sp.to_part, sp.to_voice);
            let between: Vec<&Note> = self.notes.iter().filter(|o| in_slur(o) && !o.rest && o.start >= sp.from && o.start <= sp.to).collect();
            if between.is_empty() {
                continue;
            }
            let poly = between.iter().any(|b| self.notes.iter().any(|o| o.part == sp.part && o.measure == b.measure && o.voice != sp.voice && !o.whole_bar));
            let above = match sp.dir {
                0 if poly => sp.voice % 2 == 1,
                0 => !between.iter().all(|o| o.stem_up),
                d => d > 0,
            };
            let dir = if above { -1.0 } else { 1.0 };
            let top = self.top(sp.part);
            // The problem as LilyPond's slur scorer sees it (see `crate::slur`):
            // the ends' heads and stems, the note columns under it, what
            // else lies there, the slurs inside it.
            use crate::slur;
            let th = d.stem_thickness;
            let toward = |n: &Note| n.has_stem() && n.stem_up == above;
            let head_y = |n: &Note, s: &Self| if above { s.y(n.part, n.heads[n.heads.len() - 1].pos) } else { s.y(n.part, n.heads[0].pos) };
            let stem_of = |n: &Note, s: &Self| -> Option<slur::Stem> {
                if !n.has_stem() {
                    return None;
                }
                let ys = n.heads.iter().map(|h| s.y(n.part, h.pos)).chain(std::iter::once(n.stem_end));
                let (lo, hi) = ys.fold((f32::MAX, f32::MIN), |(a, b), y| (a.min(y), b.max(y)));
                Some(slur::Stem { x: (n.stem_x - th / 2.0, n.stem_x + th / 2.0), y: (lo, hi), toward: toward(n), beamed_inward: n.beamed, beamed: n.beamed })
            };
            let column = |n: &Note, s: &Self| -> slur::Column {
                let head = head_y(n, s) + dir * 0.5;
                if toward(n) {
                    let beam = if n.beamed { dir * 0.25 } else { 0.0 };
                    slur::Column { x: n.stem_x, head, stem: n.stem_end + beam }
                } else {
                    slur::Column { x: n.x + HEAD_W / 2.0, head, stem: head }
                }
            };
            let end = |n: &Note, s: &Self| -> slur::End {
                let c = column(n, s);
                let edge = if above { c.head.min(c.stem) } else { c.head.max(c.stem) };
                slur::End { head: Some(((n.x, n.x + HEAD_W), head_y(n, s))), stem: stem_of(n, s), column_edge: edge, broken_at: (0.0, 0.0) }
            };
            let broken = |x: f32, n: &Note, s: &Self| -> slur::End {
                let y = head_y(n, s) + dir * 1.6;
                slur::End { head: None, stem: None, column_edge: y, broken_at: (x, y) }
            };
            let ends = match (si, ei) {
                (Some(a), Some(b)) => [end(&self.notes[a], self), end(&self.notes[b], self)],
                (Some(a), None) => [end(&self.notes[a], self), broken(right, between.last().unwrap(), self)],
                (None, Some(b)) => [broken(left, between.first().unwrap(), self), end(&self.notes[b], self)],
                (None, None) => {
                    let n = between.iter().copied().max_by(|x, y| (head_y(x, self) * -dir).total_cmp(&(head_y(y, self) * -dir))).unwrap();
                    [broken(left, n, self), broken(right, n, self)]
                }
            };
            // The columns in order: the left end, the notes between by x, the right end.
            let end_ids = (si.map(|i| self.notes[i].id), ei.map(|i| self.notes[i].id));
            let mut inner: Vec<&Note> = between.iter().copied().filter(|n| Some(n.id) != end_ids.0 && Some(n.id) != end_ids.1).collect();
            inner.sort_by(|a, b| a.x.total_cmp(&b.x));
            let mut inner_cols: Vec<slur::Column> = inner.iter().map(|n| column(n, self)).collect();
            // Grace notes inside the slur are encompassed like any other
            // note (LilyPond's slur acknowledges their columns too); those
            // before its first note lie outside it.
            for n in between.iter().filter(|n| Some(n.id) != end_ids.0) {
                let up = n.voice % 2 == 1;
                let beamed = n.graces.len() > 1;
                for g in &n.graces {
                    let Some((sx, tip)) = self.out.items.iter().find_map(|it| match it {
                        Item::Line { from, to, id, .. } if *id == g.id && (from.x - to.x).abs() < 1e-3 => Some((from.x, if up { from.y.min(to.y) } else { from.y.max(to.y) })),
                        _ => None,
                    }) else {
                        continue;
                    };
                    let hy = if above { self.y(n.part, g.heads[g.heads.len() - 1].pos) } else { self.y(n.part, g.heads[0].pos) };
                    let head = hy + dir * 0.5 * GRACE_SCALE;
                    let col = if up == above {
                        slur::Column { x: sx, head, stem: tip + if beamed { dir * 0.5 * GRACE_SCALE } else { 0.0 } }
                    } else {
                        slur::Column { x: sx, head, stem: head }
                    };
                    inner_cols.push(col);
                }
            }
            inner_cols.sort_by(|a, b| a.x.total_cmp(&b.x));
            let columns: Vec<slur::Column> = si
                .map(|i| column(&self.notes[i], self))
                .into_iter()
                .chain(inner_cols)
                .chain(ei.map(|i| column(&self.notes[i], self)))
                .collect();
            // What else hangs off these notes: accidentals to go around,
            // articulations to keep inside, words around.
            let heads: std::collections::HashSet<u64> = between.iter().flat_map(|n| n.heads.iter().map(|h| h.id)).collect();
            let ids: std::collections::HashSet<u64> = between.iter().map(|n| n.id).chain(heads.iter().copied()).collect();
            let accidentals = [Glyph::Sharp, Glyph::Flat, Glyph::Natural, Glyph::DoubleSharp, Glyph::DoubleFlat].map(|g| g.codepoint());
            let skip = [Glyph::NoteheadBlack, Glyph::NoteheadHalf, Glyph::NoteheadWhole, Glyph::Dot, Glyph::Flag8thUp, Glyph::Flag8thDown, Glyph::Flag16thUp, Glyph::Flag16thDown, Glyph::Flag32ndUp, Glyph::Flag32ndDown, Glyph::Flag64thUp, Glyph::Flag64thDown].map(|g| g.codepoint());
            let mut extra: Vec<slur::Extra> = self
                .out
                .items
                .iter()
                .filter(|it| ids.contains(&it.id()))
                .filter_map(|it| {
                    let r = it.bounds(self.font);
                    let (inside, penalty) = match it {
                        Item::Glyph { glyph, .. } if skip.contains(glyph) || heads.contains(&it.id()) => return None,
                        Item::Glyph { glyph, .. } if accidentals.contains(glyph) => (false, 3.0),
                        Item::Glyph { .. } => (true, 50.0),
                        Item::Text { .. } => (false, 50.0),
                        _ => return None,
                    };
                    Some(slur::Extra { x: (r[0], r[2]), y: (r[1], r[3]), inside, penalty })
                })
                .collect();
            // Slurs inside this one on its side: their middles to clear, their
            // ends and middles to go around.
            let (x0, x1) = (ends[0].head.map(|h| h.0 .0).unwrap_or(left), ends[1].head.map(|h| h.0 .1).unwrap_or(right));
            let mut avoid_slurs = Vec::new();
            for (_, _, line) in drawn.iter().filter(|(p, a, line)| *p == sp.part && *a == above && line.first().is_some_and(|f| f.0 >= x0 - 0.1) && line.last().is_some_and(|l| l.0 <= x1 + 0.1)) {
                if let Some(&m) = line.get(line.len() / 2) {
                    avoid_slurs.push(m);
                }
                for &(x, y) in [line.first(), line.get(line.len() / 2), line.last()].into_iter().flatten() {
                    extra.push(slur::Extra { x: (x - 0.3, x + 0.3), y: (y - 0.2, y + 0.2), inside: false, penalty: 50.0 });
                }
            }
            let pb = slur::Problem { dir, staff_top: top, ends, columns, extra, avoid_slurs, cross_staff: sp.part != sp.to_part };
            let bez = slur::solve(&pb);
            let [p0, p1, p2, p3] = bez.0;
            let (x0, y0, x1, y1) = (p0.0, p0.1, p3.0, p3.1);
            // The control points are the midline; the thickness is split around it.
            let t = d.slur_midpoint_thickness / 2.0;
            let mid = |f: f32| bez.point(f);
            drawn.push((sp.part, above, (0..=12).map(|k| mid(k as f32 / 12.0)).collect()));
            items.push(Item::Crescent {
                from: pt(x0, y0),
                c1: pt(p1.0, p1.1 + dir * t),
                c2: pt(p2.0, p2.1 + dir * t),
                to: pt(x1, y1),
                d1: pt(p1.0, p1.1 - dir * t),
                d2: pt(p2.0, p2.1 - dir * t),
                id: sp.id,
            });
        }
        self.out.items.extend(items);
    }
}
