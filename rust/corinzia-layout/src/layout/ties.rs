//! Ties: their shape, placement and directions.
//!
//! Ported from GNU LilyPond (`lily/tie-formatting-problem.cc`,
//! `lily/tie-configuration.cc`, `lily/tie.cc`), Copyright (C) 1997–2026
//! Han-Wen Nienhuys, Jan Nieuwenhuizen and the LilyPond authors, under the
//! GNU General Public License, version 3 or later (see
//! `THIRD-PARTY-NOTICES.md`).

use super::*;

/// A tie's indent and height for its width: LilyPond's slur shape law
/// with the Tie's details (height limit 1, ratio 1/3).
pub(super) fn tie_shape(width: f32) -> (f32, f32) {
    const H_INF: f32 = 1.0;
    const RATIO: f32 = 0.333;
    let max_fraction = 1.0 / 3.1;
    let height = 2.0 / std::f32::consts::PI * (std::f32::consts::PI / 2.0 * width * RATIO / H_INF).atan() * H_INF;
    let q = 2.0 * H_INF / max_fraction;
    (2.0 * H_INF - q * q * max_fraction / (width + q), height)
}

impl<'a> Engraver<'a> {
    /// Ties, shaped and placed as LilyPond places them
    /// (`lily/tie-formatting-problem.cc`, `lily/tie-configuration.cc`,
    /// `lily/tie.cc`, the Tie details in `scm/define-grobs.scm`; GNU
    /// LilyPond, GPL-3.0-or-later): LilyPond's slur shape with the tie's
    /// height limit (1) and ratio (1/3); ends 0.2 from the heads (0.35 past
    /// a stem in the way); each tie half a space out from its head (in the
    /// space beside a head on a line), a short tie centred in its space or
    /// kept off the line, a tall one with its peak clear of the lines; a
    /// chord's ties on one side moved apart where they would touch.
    pub(super) fn draw_ties(&mut self) {
        const NOTE_HEAD_GAP: f32 = 0.2;
        const INTRA_SPACE_THRESHOLD: f32 = 1.25;
        const TIP_STAFF_LINE_CLEARANCE: f32 = 0.45;
        const CENTER_STAFF_LINE_CLEARANCE: f32 = 0.6;
        const OUTER_TIE_VERTICAL_GAP: f32 = 0.25;
        const TIE_TIE_COLLISION_DISTANCE: f32 = 0.45;
        let d = self.font.defaults;
        let system_end = self.ms.last().map(|m| m.x1).unwrap_or(self.left);
        let mut items = Vec::new();
        // `generate_configuration`: a tie from x0 to x1 for the head at
        // `pos` (half spaces up from the middle line), `extra` half spaces
        // further out, on side `dir` (+1 down); `chord`: the chord's head
        // positions. Gives the ends' page y and the height.
        // `get_configuration` at tie position `p` (half spaces up).
        // `dots`: the left note's dots (x, staff position): a tie at a dot's
        // position moves a quarter space out, untuned.
        let configure_at = |x0: f32, x1: f32, pos: i32, p: i32, chord: (i32, i32), top: f32, dir: f32, dots: &[(f32, i32)]| -> (f32, f32) {
            let (_, h) = tie_shape((x1 - x0).max(0.6));
            let up = -dir; // LilyPond's direction, y up
            let y = p as f32 * 0.5;
            if dots.iter().any(|d| d.1 == p) {
                return (top + 2.0 - (y + up * 0.25), h);
            }
            let on_line = |q: i32| q % 2 == 0 && (-4..=4).contains(&q);
            let head_edge = pos as f32 * 0.5 + up * 0.5;
            let mut delta = 0.0;
            if (head_edge - y).abs() < 0.25 && !on_line(p) {
                delta = head_edge - y + up * OUTER_TIE_VERTICAL_GAP;
            }
            let within_staff = (-3..=3).contains(&p);
            if (chord.0..=chord.1).contains(&p) || within_staff {
                if h < INTRA_SPACE_THRESHOLD * 0.5 {
                    if on_line(p) {
                        delta += up * TIP_STAFF_LINE_CLEARANCE * 0.5;
                    } else if within_staff {
                        // Its ends and its peak either side of the space's middle.
                        delta = -up * 0.375 * h;
                    }
                } else {
                    let top_y = y + delta + up * h;
                    let r = (top_y * 2.0).round() as i32;
                    if (top_y * 2.0 - r as f32).abs() < CENTER_STAFF_LINE_CLEARANCE && on_line(r) {
                        delta = (r as f32 + CENTER_STAFF_LINE_CLEARANCE * up) * 0.5 - top_y;
                    }
                }
            }
            (top + 2.0 - (y + delta), h)
        };
        let configure = |x0: f32, x1: f32, pos: i32, extra: i32, chord: (i32, i32), top: f32, dir: f32, dots: &[(f32, i32)]| -> (f32, f32) {
            configure_at(x0, x1, pos, pos - dir as i32 * (1 + extra), chord, top, dir, dots)
        };
        // A single tie: LilyPond's variations (the other direction, and up
        // to three half spaces either way) scored by `score_configuration`
        // and `score_aptitude` (too short, tips or peak on a line, far from
        // its head, the wrong way, the stem's way); the best one.
        let single = |x0: f32, x1: f32, pos: i32, base_dir: f32, stem: Option<f32>, chord: (i32, i32), top: f32, dots: &[(f32, i32)]| -> (f32, f32, f32) {
            const DOT_COLLISION_PENALTY: f32 = 0.25;
            const DOT_COLLISION_CLEARANCE: f32 = 0.25;
            const MIN_LENGTH: f32 = 1.0;
            const MIN_LENGTH_PENALTY_FACTOR: f32 = 26.0;
            const STAFF_LINE_COLLISION_PENALTY: f32 = 5.0;
            const WRONG_DIRECTION_OFFSET_PENALTY: f32 = 10.0;
            const VERTICAL_DISTANCE_PENALTY_FACTOR: f32 = 7.0;
            const SAME_DIR_AS_STEM_PENALTY: f32 = 8.0;
            let peak = |eps: f32, threshold: f32, x: f32| if x < 0.0 { 1.0 } else { (-eps * (x - threshold) / ((x + eps) * threshold)).max(0.0) };
            let convex = |standard: f32, factor: f32, x: f32| ((factor * x / standard).exp() - 1.0) / (factor.exp() - 1.0);
            let on_line = |q: i32| q % 2 == 0 && (-4..=4).contains(&q);
            let p0 = pos - base_dir as i32;
            let mut cands: Vec<(i32, f32)> = vec![(p0, base_dir), (p0, -base_dir)];
            for i in 1..4 {
                for d in [1.0f32, -1.0] {
                    cands.push((p0 - d as i32 * i, d));
                }
            }
            let mut best = (f32::INFINITY, 0.0, 0.0, base_dir);
            for (p, dir) in cands {
                let (y, h) = configure_at(x0, x1, pos, p, chord, top, dir, dots);
                let up = -dir;
                let tip_y = top + 2.0 - y; // y up from the middle line
                let mut sc = MIN_LENGTH_PENALTY_FACTOR * peak(0.33 * MIN_LENGTH, MIN_LENGTH, x1 - x0);
                let top_pos = 2.0 * (tip_y + up * h);
                if on_line(top_pos.round() as i32) {
                    sc += STAFF_LINE_COLLISION_PENALTY * peak(0.06, 0.6, (top_pos - top_pos.round()).abs());
                }
                let tip_pos = 2.0 * tip_y;
                let rt = tip_pos.round() as i32;
                if on_line(rt) && ((chord.0..=chord.1).contains(&rt) || (-3..=3).contains(&rt)) {
                    sc += STAFF_LINE_COLLISION_PENALTY * peak(0.045, 0.45, (tip_pos - tip_pos.round()).abs());
                }
                let head_y = pos as f32 * 0.5;
                if (tip_y - head_y).signum() != up {
                    sc += WRONG_DIRECTION_OFFSET_PENALTY;
                }
                sc += VERTICAL_DISTANCE_PENALTY_FACTOR * convex(1.0, 0.9, ((tip_y - head_y).abs() - 0.5).max(0.0));
                // A dot under the curve: penalised within a quarter space.
                let indent = tie_shape(x1 - x0).0.min((x1 - x0) / 3.1);
                for &(dx, dp) in dots {
                    if dx < x0 || dx > x1 {
                        continue;
                    }
                    let curve = crate::slur::Bezier([(x0, y), (x0 + indent, y + dir * h), (x1 - indent, y + dir * h), (x1, y)]);
                    let dist = ((top + 2.0 - dp as f32 * 0.5) - curve.y_at(dx)).abs();
                    sc += DOT_COLLISION_PENALTY * peak(0.1 * DOT_COLLISION_CLEARANCE, DOT_COLLISION_CLEARANCE, dist);
                }
                match stem {
                    Some(st) if st == up => sc += SAME_DIR_AS_STEM_PENALTY,
                    None if pos != 0 && (pos as f32).signum() != up => sc += SAME_DIR_AS_STEM_PENALTY,
                    _ => {}
                }
                if sc < best.0 {
                    best = (sc, y, h, dir);
                }
            }
            (best.1, best.2, best.3)
        };
        let shape = |x0: f32, x1: f32, y: f32, h: f32, dir: f32, id: u64| {
            let x1 = x1.max(x0 + 0.6);
            let w = x1 - x0;
            let indent = tie_shape(w).0.min(w / 3.1);
            let c = |x: f32, hh: f32| pt(x, y + dir * hh);
            Item::Crescent {
                from: pt(x0, y),
                c1: c(x0 + indent, h + d.tie_midpoint_thickness / 2.0),
                c2: c(x1 - indent, h + d.tie_midpoint_thickness / 2.0),
                to: pt(x1, y),
                d1: c(x0 + indent, h - d.tie_midpoint_thickness / 2.0),
                d2: c(x1 - indent, h - d.tie_midpoint_thickness / 2.0),
                id,
            }
        };
        // A chord's ties (x0, x1, pos, dir, id): placed, those on one side
        // moved out a half space at a time while their middles are nearer
        // than LilyPond's tie-tie collision distance.
        let place = |ties: &[(f32, f32, i32, f32, u64)], chord: (i32, i32), top: f32, stem: Option<f32>, dots: &[(f32, i32)]| -> Vec<Item> {
            if let [(x0, x1, pos, dir, id)] = *ties {
                let (y, h, dir) = single(x0, x1, pos, dir, stem, chord, top, dots);
                return vec![shape(x0, x1, y, h, dir, id)];
            }
            let mut extra = vec![0i32; ties.len()];
            let at = |k: usize, extra: &[i32]| {
                let (x0, x1, pos, dir, _) = ties[k];
                let (y, h) = configure(x0, x1, pos, extra[k], chord, top, dir, dots);
                (y, h, y + dir * 0.75 * h)
            };
            for _ in 0..3 {
                let mut moved = false;
                for side in [1.0f32, -1.0] {
                    // Inner to outer on this side.
                    let mut idx: Vec<usize> = (0..ties.len()).filter(|&k| ties[k].3 == side).collect();
                    idx.sort_by_key(|&k| ties[k].2 * -(side as i32));
                    for w in idx.windows(2) {
                        let (a, b) = (at(w[0], &extra).2, at(w[1], &extra).2);
                        if (a - b).abs() < TIE_TIE_COLLISION_DISTANCE + d.tie_midpoint_thickness {
                            extra[w[1]] += 1;
                            moved = true;
                        }
                    }
                }
                if !moved {
                    break;
                }
            }
            (0..ties.len())
                .map(|k| {
                    let (x0, x1, _, dir, id) = ties[k];
                    let (y, h, _) = at(k, &extra);
                    shape(x0, x1, y, h, dir, id)
                })
                .collect()
        };
        for i in 0..self.notes.len() {
            let n = &self.notes[i];
            if n.rest {
                continue;
            }
            // LilyPond's tie directions (`Tie::get_default_dir`,
            // `set_ties_config_standard_directions`): a single tie away from
            // the stems (by its place on the staff without one); in a chord
            // the lowest down, the highest up, a second split, the rest by
            // their side of the middle line. (+1 down, −1 up.)
            let tied: Vec<i32> = n.heads.iter().filter(|h| h.tie_out || h.tied_in).map(|h| h.pos).collect();
            let side = |h: &Head| -> f32 {
                if tied.len() <= 1 {
                    return if n.has_stem() {
                        if n.stem_up { 1.0 } else { -1.0 }
                    } else if h.pos < 0 {
                        1.0
                    } else {
                        -1.0
                    };
                }
                let i = tied.iter().position(|&p| p == h.pos).unwrap_or(0);
                if i == 0 {
                    return 1.0;
                }
                if i + 1 == tied.len() {
                    return -1.0;
                }
                // A second: its lower tie down, its upper up (the lower pair first).
                if tied[i] - tied[i - 1] <= 1 {
                    return -1.0;
                }
                if tied[i + 1] - tied[i] <= 1 {
                    return 1.0;
                }
                if h.pos > 0 { -1.0 } else { 1.0 }
            };
            // The next piece of the same voice (in this system).
            let next = self.notes[i + 1..].iter().find(|o| o.part == n.part && o.voice == n.voice && o.start >= n.start + n.dur);
            let top = self.top(n.part);
            let chord = (n.heads[0].pos, n.heads[n.heads.len() - 1].pos);
            let mut out: Vec<(f32, f32, i32, f32, u64)> = Vec::new();
            for h in n.heads.iter().filter(|h| h.tie_out) {
                let dir = side(h);
                let y = self.y(n.part, h.pos);
                // Past the left note's stem when the stem is on the tie's right
                // and reaches its height.
                let stem_in_way = n.has_stem() && n.stem_up && (n.stem_end.min(y)..=n.stem_end.max(y)).contains(&(y - dir * 0.1));
                let x0 = if stem_in_way { n.stem_x + 0.35 } else { n.x + h.shift + HEAD_W + NOTE_HEAD_GAP };
                match next {
                    Some(nx) if nx.start == n.start + n.dur => {
                        if let Some(h2) = nx.heads.iter().find(|x| x.dia == h.dia && x.alter == h.alter) {
                            out.push((x0, nx.x + h2.shift - NOTE_HEAD_GAP, h.pos, dir, h.id));
                        }
                    }
                    // Continues in the next system: to the end of this one.
                    None if n.start + n.dur >= self.range.1 => out.push((x0, system_end - 0.3, h.pos, dir, h.id)),
                    _ => {}
                }
            }
            // The stems' direction (LilyPond's, +1 up), for a single tie.
            let stem = n.has_stem().then_some(if n.stem_up { 1.0 } else { -1.0 });
            // The note's dots, as drawn: (x, staff position).
            let dots: Vec<(f32, i32)> = self
                .out
                .items
                .iter()
                .filter_map(|it| match it {
                    Item::Glyph { glyph, at, id, .. } if *id == n.id && *glyph == Glyph::Dot.codepoint() => Some((at.x, ((top + 2.0 - at.y) * 2.0).round() as i32)),
                    _ => None,
                })
                .collect();
            items.extend(place(&out, chord, top, stem, &dots));
            // Arriving from the previous system: from just before the note.
            let first_here = !self.notes[..i].iter().any(|o| o.part == n.part && o.voice == n.voice);
            if first_here && n.start == self.range.0 {
                let arriving: Vec<(f32, f32, i32, f32, u64)> = n.heads.iter().filter(|h| h.tied_in).map(|h| (n.x + h.shift - 2.2, n.x + h.shift - NOTE_HEAD_GAP, h.pos, side(h), h.id)).collect();
                items.extend(place(&arriving, chord, top, n.has_stem().then_some(if n.stem_up { 1.0 } else { -1.0 }), &[]));
            }
        }
        self.out.items.extend(items);
    }
}
