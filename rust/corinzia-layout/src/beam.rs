//! Beams placed as LilyPond places them.
//!
//! Ported from GNU LilyPond (`lily/beam-quanting.cc`, `Stem::calc_stem_info`
//! in `lily/stem.cc`, the Stem and Beam details in `scm/define-grobs.scm`),
//! Copyright (C) 1997–2026 Han-Wen Nienhuys, Jan Nieuwenhuizen and the
//! LilyPond authors, under the GNU General Public License, version 3 or
//! later (see `THIRD-PARTY-NOTICES.md`).
//!
//! Each stem's ideal and shortest ends (its beamed lengths, reaching the
//! middle line); a least-squares line through the ideal ends, its slope
//! damped (0.6 tanh, more for concave groups); then every pair of ends on
//! LilyPond's grid of quants (the beam straddling a line, sitting on it,
//! between lines, hanging from one) near that line, scored for slope,
//! stem lengths and the slivers of white a beam leaves against the staff
//! lines; the cheapest wins.
//!
//! Coordinates are LilyPond's: staff spaces, y up from the middle line,
//! the beam's y the centre of its outer beam (the one farthest from the
//! notes).

/// LilyPond's Stem details: ideal lengths to the outer beam's edge by
/// number of beams, the least free stem between the notes and the
/// innermost beam (ideal, and in extreme cases).
const BEAMED_LENGTHS: [f32; 3] = [3.26, 3.5, 3.6];
const BEAMED_MINIMUM_FREE_LENGTHS: [f32; 3] = [1.83, 1.5, 1.25];
const BEAMED_EXTREME_MINIMUM_FREE_LENGTHS: [f32; 2] = [2.0, 1.25];

/// LilyPond's Beam quanting parameters (`Beam_quant_parameters::fill`).
const REGION_SIZE: i32 = 2;
const SECONDARY_BEAM_DEMERIT: f32 = 10.0;
const STEM_LENGTH_DEMERIT_FACTOR: f32 = 5.0;
const HORIZONTAL_INTER_QUANT_PENALTY: f32 = 500.0;
const STEM_LENGTH_LIMIT_PENALTY: f32 = 5000.0;
const DAMPING_DIRECTION_PENALTY: f32 = 800.0;
const HINT_DIRECTION_PENALTY: f32 = 20.0;
const MUSICAL_DIRECTION_FACTOR: f32 = 400.0;
const IDEAL_SLOPE_FACTOR: f32 = 10.0;
const ROUND_TO_ZERO_SLOPE: f32 = 0.02;
const BEAM_EPS: f32 = 1e-3;
/// The Beam's `damping`.
const DAMPING: f32 = 1.0;
const STAFF_RADIUS: f32 = 2.0;
const COLLISION_PENALTY: f32 = 500.0;
const COLLISION_PADDING: f32 = 0.5;

/// A stem of the group: its x, and its heads' lowest and highest staff
/// positions (half spaces up from its staff's middle line); in a kneed
/// beam, its own direction and its staff's middle line in the beam's
/// frame (spaces up).
#[derive(Clone, Copy, Debug)]
pub struct Stem {
    pub x: f32,
    pub low: i32,
    pub high: i32,
    /// +1 up, −1 down; 0: the group's.
    pub dir: f32,
    pub middle: f32,
}

#[derive(Clone, Debug)]
pub struct Problem {
    pub stems: Vec<Stem>,
    /// +1: stems up (the beam above); −1 down.
    pub dir: f32,
    /// The group's most beams, and the beams at its first and last stem.
    pub beam_count: u32,
    pub edge_counts: [u32; 2],
    /// The beam's thickness, from one beam to the next, the staff lines'
    /// thickness (staff spaces).
    pub thickness: f32,
    pub translation: f32,
    pub line_thickness: f32,
    /// Other things under or over the beam (other voices' notes and rests,
    /// accidentals, clefs…): (x, y extent (low, high), factor), y up from
    /// the middle line; the factor scales the penalty (√width for a glyph,
    /// a tenth for a stem).
    pub collisions: Vec<(f32, (f32, f32), f32)>,
    /// Stems in both directions (a knee); `cross_staff`: across staves.
    /// The beam's y is then its stack's middle, and each stem reaches the
    /// beam farthest from its notes.
    pub knee: bool,
    pub cross_staff: bool,
}

struct StemInfo {
    ideal: f32,
    shortest: f32,
}

fn shrink_extra_weight(x: f32, fac: f32) -> f32 {
    x.abs() * if x < 0.0 { fac } else { 1.0 }
}

/// `set_minimum_dy`: a slope at least the smallest quant step.
fn minimum_dy(dy: f32, bt: f32, lt: f32) -> f32 {
    if dy == 0.0 {
        return dy;
    }
    let sit = (bt - lt) / 2.0;
    let hang = 1.0 - sit;
    dy.signum() * dy.abs().max(sit.min(0.5).min(hang))
}

impl Problem {
    fn dir_of(&self, s: &Stem) -> f32 {
        if s.dir != 0.0 { s.dir } else { self.dir }
    }

    /// The head nearest the beam and the one farthest, as staff positions.
    fn close_far(&self, s: &Stem) -> (i32, i32) {
        if self.dir_of(s) > 0.0 { (s.high, s.low) } else { (s.low, s.high) }
    }

    /// From the beam's y to where this stem's outer beam is: none, but in
    /// a knee half the stack towards the stem's far side.
    fn base_length(&self, s: &Stem) -> f32 {
        if self.knee { self.dir_of(s) * (self.beam_count.max(1) as f32 - 1.0) * self.translation / 2.0 } else { 0.0 }
    }

    /// `Stem::calc_stem_info`: ideal and shortest ends, y up (in the beam's
    /// frame, less the stem's base length).
    fn stem_info(&self, s: &Stem) -> StemInfo {
        let dir = self.dir_of(s);
        let bc = self.beam_count.max(1) as usize;
        let (bt, tr) = (self.thickness, self.translation);
        let mut ideal_length = BEAMED_LENGTHS[bc.min(3) - 1] - 0.5 * bt;
        let height_of_beams = bt + (bc as f32 - 1.0) * tr;
        let ideal_minimum_length = BEAMED_MINIMUM_FREE_LENGTHS[bc.min(3) - 1] + height_of_beams - 0.5 * bt;
        ideal_length = ideal_length.max(ideal_minimum_length);
        let note_start = self.close_far(s).0 as f32 * 0.5 * dir;
        let mut ideal = note_start + ideal_length;
        // The outer beam reaches the middle line; the inner, the second line
        // (not in a knee).
        if !self.knee {
            ideal = ideal.max(0.0);
            ideal = ideal.max(-1.0 - bt + height_of_beams);
        }
        let minimum_length = BEAMED_EXTREME_MINIMUM_FREE_LENGTHS[bc.min(2) - 1] + height_of_beams - 0.5 * bt;
        let at = s.middle - self.base_length(s);
        StemInfo { ideal: ideal * dir + at, shortest: (note_start + minimum_length) * dir + at }
    }

    /// `calc_concaveness`.
    fn concaveness(&self) -> f32 {
        if self.stems.len() <= 2 || self.knee || self.cross_staff {
            return 0.0;
        }
        let dir = self.dir as i32;
        let close: Vec<i32> = self.stems.iter().map(|s| self.close_far(s).0).collect();
        let far: Vec<i32> = self.stems.iter().map(|s| self.close_far(s).1).collect();
        if is_concave_single_notes(if dir > 0 { &close } else { &far }, dir) {
            return 10000.0;
        }
        (positions_concaveness(&far, dir) + positions_concaveness(&close, dir)) / 2.0
    }
}

fn is_concave_single_notes(p: &[i32], dir: i32) -> bool {
    let (lo, hi) = (p[0].min(p[p.len() - 1]), p[0].max(p[p.len() - 1]));
    let (mut above, mut below) = (false, false);
    for &x in &p[1..p.len() - 1] {
        above |= x > hi;
        below |= x < lo;
    }
    let mut concave = above && below;
    let dy = p[p.len() - 1] - p[0];
    let closest = (dir * p[p.len() - 1]).max(dir * p[0]);
    for i in 2..p.len().saturating_sub(1) {
        if concave {
            break;
        }
        let inner = p[i] - p[i - 1];
        if inner.signum() != dy.signum() && (dir * p[i] >= closest || dir * p[i - 1] >= closest) {
            concave = true;
        }
    }
    let all_closer = p[1..p.len() - 1].iter().all(|&x| dir * x > closest);
    concave || all_closer
}

fn positions_concaveness(p: &[i32], dir: i32) -> f32 {
    let dy = (p[p.len() - 1] - p[0]) as f32;
    let slope = dy / (p.len() - 1) as f32;
    let mut c = 0.0;
    for (i, &x) in p.iter().enumerate().take(p.len() - 1).skip(1) {
        let line = slope * i as f32 + p[0] as f32;
        c += (dir as f32 * (x as f32 - line)).max(0.0);
    }
    c /= p.len() as f32;
    if dy != 0.0 { c / dy.abs() } else { c }
}

/// The best beam: its outer beam's centre (y up from the middle line) at
/// the first and the last stem.
pub fn solve(pb: &Problem) -> [f32; 2] {
    let n = pb.stems.len();
    let (bt, tr, lt, dir) = (pb.thickness, pb.translation, pb.line_thickness, pb.dir);
    let x0 = pb.stems[0].x;
    let xs: Vec<f32> = pb.stems.iter().map(|s| s.x - x0).collect();
    let x_span = xs[n - 1];
    let infos: Vec<StemInfo> = pb.stems.iter().map(|s| pb.stem_info(s)).collect();

    // least_squares_positions
    let ideal = [infos[0].ideal, infos[n - 1].ideal];
    let mut unq: [f32; 2];
    let mut musical_dy: f32;
    if ideal[1] - ideal[0] == 0.0 {
        let chord = [pb.close_far(&pb.stems[0]).1 as f32 * 0.5, pb.close_far(&pb.stems[n - 1]).1 as f32 * 0.5];
        if ideal[0] == 0.0 && chord[1] - chord[0] != 0.0 && n == 2 {
            let d = (chord[1] - chord[0]).signum();
            unq = if d > 0.0 { [-bt / 2.0, bt / 2.0] } else { [bt / 2.0, -bt / 2.0] };
        } else {
            unq = ideal;
        }
        musical_dy = unq[1] - unq[0];
    } else {
        let k = n as f32;
        let (mx, my) = (xs.iter().sum::<f32>() / k, infos.iter().map(|i| i.ideal).sum::<f32>() / k);
        let sxx: f32 = xs.iter().map(|x| (x - mx) * (x - mx)).sum();
        let sxy: f32 = xs.iter().zip(&infos).map(|(x, i)| (x - mx) * (i.ideal - my)).sum();
        let slope = if sxx > 1e-9 { sxy / sxx } else { 0.0 };
        let y = my - slope * mx;
        let dy = minimum_dy(slope * x_span, bt, lt);
        unq = [y, y + dy];
        musical_dy = dy;
    }

    // slope_damping
    if n > 1 {
        let conc = pb.concaveness();
        let mut damping = DAMPING;
        if conc >= 10000.0 {
            unq[0] = unq[1];
            musical_dy = 0.0;
            damping = 0.0;
        }
        if damping != 0.0 {
            let dy = unq[1] - unq[0];
            let slope = if dy != 0.0 && x_span != 0.0 { dy / x_span } else { 0.0 };
            let slope = 0.6 * slope.tanh() / (damping + conc);
            let damped = minimum_dy(slope * x_span, bt, lt);
            unq[0] += (dy - damped) / 2.0;
            unq[1] -= (dy - damped) / 2.0;
        }
    }

    // shift_region_to_valid: no stem shorter than its shortest.
    {
        let beam_dy = unq[1] - unq[0];
        let slope = if x_span != 0.0 { beam_dy / x_span } else { 0.0 };
        let (mut lo, mut hi) = (f32::NEG_INFINITY, f32::INFINITY);
        for ((x, info), st) in xs.iter().zip(&infos).zip(&pb.stems) {
            let left_y = info.shortest - slope * x;
            if pb.dir_of(st) > 0.0 {
                lo = lo.max(left_y);
            } else {
                hi = hi.min(left_y);
            }
        }
        if lo <= hi && !(lo..=hi).contains(&unq[0]) {
            let left = if lo.is_infinite() { hi - 2.0 } else if hi.is_infinite() { lo + 2.0 } else { (lo + hi) / 2.0 };
            unq = [left, left + beam_dy];
        }
    }
    let damped_dy = unq[1] - unq[0];

    // generate_quants, within the range the edge stems' heads allow.
    let sit = (bt - lt) / 2.0;
    let hang = 1.0 - sit;
    let base = [0.0, sit, 0.5, hang];
    // (A wider region where something is in the way.)
    let region = REGION_SIZE + if pb.collisions.is_empty() { 0 } else { 2 } + if pb.knee { 2 } else { 0 };
    let quants: Vec<f32> = (-region..region).flat_map(|i| base.map(|b| i as f32 + b)).collect();
    let range_ok = |d: usize, y: f32| {
        let s = &pb.stems[if d == 0 { 0 } else { n - 1 }];
        let widen = 0.5 + (pb.edge_counts[d].max(1) as f32 - 1.0) * tr + bt * 0.5;
        let y = y + pb.base_length(s);
        if pb.dir_of(s) > 0.0 { y >= s.middle + s.high as f32 * 0.5 + widen } else { y <= s.middle + s.low as f32 * 0.5 - widen }
    };
    let edge_dirs = [pb.dir_of(&pb.stems[0]), pb.dir_of(&pb.stems[n - 1])];
    let extra_demerit = SECONDARY_BEAM_DEMERIT / pb.edge_counts[0].max(pb.edge_counts[1]).max(1) as f32;
    let score = |y: [f32; 2], start: f32| -> f32 {
        let dy = y[1] - y[0];
        let mut dem = start;
        // score_slope_ideal
        dem += shrink_extra_weight(damped_dy.abs() - dy.abs(), 1.5) * IDEAL_SLOPE_FACTOR * if pb.cross_staff { 10.0 } else { 1.0 };
        // score_slope_direction
        if damped_dy.signum() != dy.signum() || (damped_dy == 0.0) != (dy == 0.0) {
            if dy == 0.0 {
                dem += if x_span != 0.0 && (damped_dy / x_span).abs() > ROUND_TO_ZERO_SLOPE { DAMPING_DIRECTION_PENALTY } else { HINT_DIRECTION_PENALTY };
            } else if damped_dy != 0.0 || dy != 0.0 {
                dem += DAMPING_DIRECTION_PENALTY;
            }
        }
        // score_slope_musical
        dem += MUSICAL_DIRECTION_FACTOR * (dy.abs() - musical_dy.abs()).max(0.0);
        // score_forbidden_quants: no sliver of white between a beam and a line.
        for ((&yd, &count), &dir) in y.iter().zip(&pb.edge_counts).zip(&edge_dirs) {
            for j in 1..=count.max(1) {
                let jf = j as f32;
                let g1 = yd - dir * ((jf - 1.0) * tr + bt / 2.0 - lt / 2.2);
                let g2 = yd - dir * (jf * tr - bt / 2.0 + lt / 2.2);
                let (glo, ghi) = (g1.min(g2), g1.max(g2));
                let mut k = -STAFF_RADIUS;
                while k <= STAFF_RADIUS + BEAM_EPS {
                    if (glo..=ghi).contains(&k) {
                        let dist = (ghi - k).abs().min((glo - k).abs());
                        dem += extra_demerit * (0.39 + 0.61 * (dist / (ghi - glo)) * 2.0);
                    }
                    k += 1.0;
                }
            }
        }
        if pb.edge_counts[0].max(pb.edge_counts[1]) >= 2 {
            let modf = |v: f32| v - v.floor();
            for ((&yd, &count), &dir) in y.iter().zip(&pb.edge_counts).zip(&edge_dirs) {
                if count >= 2 && (yd - dir * tr).abs() < STAFF_RADIUS + 0.5 {
                    if dir > 0.0 && dy <= BEAM_EPS && (modf(yd) - sit).abs() < BEAM_EPS {
                        dem += extra_demerit;
                    }
                    if dir < 0.0 && dy >= BEAM_EPS && (modf(yd) - hang).abs() < BEAM_EPS {
                        dem += extra_demerit;
                    }
                }
                if count >= 3 && (yd - 2.0 * dir * tr).abs() < STAFF_RADIUS + 0.5 && modf(yd).abs() < BEAM_EPS && ((dir > 0.0 && dy <= BEAM_EPS) || (dir < 0.0 && dy >= BEAM_EPS)) {
                    dem += extra_demerit;
                }
            }
        }
        // score_stem_lengths: per direction, averaged (convex in a knee).
        let (mut s, mut count) = ([0.0f32; 2], [0usize; 2]);
        for ((x, info), st) in xs.iter().zip(&infos).zip(&pb.stems) {
            let d = pb.dir_of(st);
            let k = usize::from(d > 0.0);
            let beam_y = if x_span != 0.0 { y[1] * x / x_span + y[0] * (x_span - x) / x_span } else { (y[0] + y[1]) / 2.0 };
            s[k] += STEM_LENGTH_LIMIT_PENALTY * (d * (info.shortest - beam_y)).max(0.0);
            let ideal_score = shrink_extra_weight(d * (beam_y - info.ideal), 1.5);
            s[k] += STEM_LENGTH_DEMERIT_FACTOR * if pb.knee { ideal_score.powf(1.1) } else { ideal_score };
            count[k] += 1;
        }
        dem += s[0] / count[0].max(1) as f32 + s[1] / count[1].max(1) as f32;
        // score_collisions: within half a space of something, the penalty
        // rising as the cube of how much nearer.
        for &(cx, (lo, hi), factor) in &pb.collisions {
            let x = cx - x0;
            if x < 0.0 || x > x_span {
                continue;
            }
            let centre = if x_span != 0.0 { y[1] * x / x_span + y[0] * (x_span - x) / x_span } else { y[0] };
            // The beams at x: the outer one and those towards the notes.
            let inner = if pb.knee { centre - (pb.beam_count.max(1) as f32 - 1.0) * tr / 2.0 } else { centre - dir * (pb.beam_count.max(1) as f32 - 1.0) * tr };
            let centre = if pb.knee { centre + (pb.beam_count.max(1) as f32 - 1.0) * tr / 2.0 } else { centre };
            let (blo, bhi) = (centre.min(inner) - bt / 2.0, centre.max(inner) + bt / 2.0);
            let dist = if bhi < lo { lo - bhi } else if blo > hi { blo - hi } else { 0.0 };
            let free = (COLLISION_PADDING - dist).max(0.0) / COLLISION_PADDING;
            dem += factor * free.powi(3) * COLLISION_PENALTY;
        }
        // score_horizontal_inter_quants: a level beam between two lines.
        if dy == 0.0 && y[0].abs() < STAFF_RADIUS {
            let yshift = y[0] - 0.5;
            if ((yshift + 0.5).floor() - yshift).abs() < 0.01 {
                dem += HORIZONTAL_INTER_QUANT_PENALTY;
            }
        }
        dem
    };
    let mut best = (f32::INFINITY, unq);
    for &ql in &quants {
        for &qr in &quants {
            let y = [unq[0].trunc() + ql, unq[1].trunc() + qr];
            if !range_ok(0, y[0]) || !range_ok(1, y[1]) {
                continue;
            }
            let sc = score(y, (ql.abs() + qr.abs()) / 1000.0);
            if sc < best.0 {
                best = (sc, y);
            }
        }
    }
    best.1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pb(stems: &[(f32, i32)], dir: f32, beams: u32) -> Problem {
        Problem {
            stems: stems.iter().map(|&(x, p)| Stem { x, low: p, high: p, dir: 0.0, middle: 0.0 }).collect(),
            dir,
            beam_count: beams,
            edge_counts: [beams, beams],
            thickness: 0.48,
            translation: 0.75,
            line_thickness: 0.1,
            collisions: vec![],
            knee: false,
            cross_staff: false,
        }
    }

    #[test]
    fn repeated_notes_get_a_level_beam_on_a_quant() {
        // Four g' (position −3, under the middle line), stems up.
        let y = solve(&pb(&[(0.0, -3), (2.0, -3), (4.0, -3), (6.0, -3)], 1.0, 1));
        assert_eq!(y[0], y[1]);
        // About 3.5 spaces above the heads (−1.5 + 3.26 − 0.24 ≈ 1.5).
        assert!((y[0] - 1.5).abs() < 0.6, "{y:?}");
    }

    #[test]
    fn a_rising_scale_slopes_gently_up() {
        let y = solve(&pb(&[(0.0, -4), (2.0, -3), (4.0, -2), (6.0, -1)], 1.0, 1));
        let dy = y[1] - y[0];
        assert!(dy > 0.0 && dy < 1.5, "{y:?}");
        // No stem shorter than its shortest.
        for (k, p) in [-4, -3, -2, -1].iter().enumerate() {
            let at = y[0] + dy * k as f32 / 3.0;
            assert!(at - *p as f32 * 0.5 >= 2.0 + 0.48 - 0.24 - 0.01, "stem {k}: {at}");
        }
    }

    #[test]
    fn a_concave_group_is_level() {
        // Low, high, high, low (stems up): the beam does not slope.
        let y = solve(&pb(&[(0.0, -4), (2.0, 0), (4.0, 0), (6.0, -3)], 1.0, 1));
        assert_eq!(y[0], y[1], "{y:?}");
    }

    #[test]
    fn a_beam_moves_clear_of_what_is_in_its_way() {
        // Stems up over g' (the beam ideally near 1.5); something at 1.0..2.2
        // in the middle of the group pushes it up and away.
        let mut p = pb(&[(0.0, -3), (2.0, -3), (4.0, -3)], 1.0, 1);
        let free = solve(&p);
        p.collisions = vec![(2.0, (1.0, 2.2), 1.0)];
        let y = solve(&p);
        assert!(y[0] > free[0], "{y:?} vs {free:?}");
        let centre = (y[0] + y[1]) / 2.0;
        assert!(centre - 0.24 >= 2.2 + 0.4, "the beam clears it: {y:?}");
    }

    #[test]
    fn a_kneed_beam_lies_between_its_staves() {
        // Upper staff (middle at 0): two notes stems down; lower staff
        // (middle 9 spaces lower): two notes stems up.
        let stems = vec![
            Stem { x: 0.0, low: -2, high: -2, dir: -1.0, middle: 0.0 },
            Stem { x: 2.0, low: 2, high: 2, dir: 1.0, middle: -9.0 },
            Stem { x: 4.0, low: -2, high: -2, dir: -1.0, middle: 0.0 },
            Stem { x: 6.0, low: 2, high: 2, dir: 1.0, middle: -9.0 },
        ];
        let p = Problem { stems, dir: 1.0, beam_count: 1, edge_counts: [1, 1], thickness: 0.48, translation: 0.75, line_thickness: 0.1, collisions: vec![], knee: true, cross_staff: true };
        let y = solve(&p);
        // Between the upper staff's bottom line (−2) and the lower's top (−7),
        // clear of every head, and level (the music goes nowhere).
        for v in y {
            assert!(v < -1.0 - 2.0 && v > -9.0 + 1.0 + 2.0, "{y:?}");
        }
        assert!((y[0] - y[1]).abs() < 0.6, "{y:?}");
    }

    #[test]
    fn beams_far_from_the_staff_reach_the_middle_line() {
        // Very high notes, stems down: the beam comes down to the middle line.
        let y = solve(&pb(&[(0.0, 14), (2.0, 14)], -1.0, 1));
        assert!(y[0] <= 0.0 + 1e-3, "{y:?}");
    }
}
