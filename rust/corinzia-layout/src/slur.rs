//! Slurs placed as LilyPond places them.
//!
//! Ported from GNU LilyPond (`lily/slur-scoring.cc`,
//! `lily/slur-configuration.cc`, `lily/bezier-bow.cc`, and the defaults in
//! `scm/layout-slur.scm` and `scm/define-grobs.scm`), Copyright (C)
//! 1996–2026 Han-Wen Nienhuys, Jan Nieuwenhuizen and the LilyPond
//! authors, under the GNU General Public License, version 3 or later (see
//! `THIRD-PARTY-NOTICES.md`).
//!
//! Each end may sit anywhere from its base (half a space off its head, or
//! at its stem's tip) outwards, in half spaces; every pair of ends makes
//! a Bézier whose height and indent follow LilyPond's laws, raised just
//! enough to clear the heads and objects inside it; each is scored by
//! LilyPond's four scorers (its ends, its slope, how it encloses the
//! notes, what else it meets) and the cheapest wins.
//!
//! The engraver's coordinates have y downwards; LilyPond's go up. The
//! problem comes in the engraver's and is turned over inside, so the
//! formulas read as LilyPond's.

/// LilyPond's `default-slur-details`, with the Slur's own properties.
mod p {
    pub const CLOSE_TO_EDGE_LENGTH: f32 = 2.5;
    pub const EDGE_ATTRACTION_FACTOR: f32 = 4.0;
    pub const EDGE_SLOPE_EXPONENT: f32 = 1.7;
    pub const ENCOMPASS_OBJECT_RANGE_OVERSHOOT: f32 = 0.5;
    pub const EXTRA_ENCOMPASS_FREE_DISTANCE: f32 = 0.3;
    pub const FREE_HEAD_DISTANCE: f32 = 0.3;
    pub const FREE_SLUR_DISTANCE: f32 = 0.8;
    pub const GAP_TO_STAFFLINE_INSIDE: f32 = 0.2;
    pub const GAP_TO_STAFFLINE_OUTSIDE: f32 = 0.1;
    pub const HEAD_ENCOMPASS_PENALTY: f32 = 1000.0;
    pub const HEAD_SLUR_DISTANCE_FACTOR: f32 = 10.0;
    pub const HEAD_SLUR_DISTANCE_MAX_RATIO: f32 = 3.0;
    pub const ABSOLUTE_CLOSENESS_MEASURE: f32 = 0.3;
    pub const MAX_SLOPE: f32 = 1.1;
    pub const MAX_SLOPE_FACTOR: f32 = 10.0;
    pub const NON_HORIZONTAL_PENALTY: f32 = 15.0;
    pub const REGION_SIZE: f32 = 4.0;
    pub const SAME_SLOPE_PENALTY: f32 = 20.0;
    pub const STEEPER_SLOPE_FACTOR: f32 = 50.0;
    pub const STEM_ENCOMPASS_PENALTY: f32 = 30.0;
    /// The Slur grob: `height-limit`, `ratio`, `minimum-length`.
    pub const HEIGHT_LIMIT: f32 = 2.0;
    pub const RATIO: f32 = 0.25;
    pub const MINIMUM_LENGTH: f32 = 1.5;
    /// The staff lines' thickness (spaces).
    pub const LINE_THICKNESS: f32 = 0.13;
}

/// An end's stem.
#[derive(Clone, Copy, Debug)]
pub struct Stem {
    /// Its horizontal extent and its vertical extent (min, max), y down.
    pub x: (f32, f32),
    pub y: (f32, f32),
    /// It points the slur's way.
    pub toward: bool,
    /// Beamed on the side towards the other end (the slur may sit on it).
    pub beamed_inward: bool,
    pub beamed: bool,
}

/// One end of a slur.
#[derive(Clone, Copy, Debug)]
pub struct End {
    /// The extremal head on the slur's side: its x extent and its y (the
    /// head's centre), y down; none for a slur broken here.
    pub head: Option<((f32, f32), f32)>,
    pub stem: Option<Stem>,
    /// The note column's extent on the slur's side (y).
    pub column_edge: f32,
    /// Where a broken slur's end goes (x, y), when `head` is none.
    pub broken_at: (f32, f32),
}

/// A note column under the slur (its ends included): where the slur must
/// pass: the head's centre x (the stem's x where the stem points the
/// slur's way), the head's edge and the stem's end on the slur's side.
#[derive(Clone, Copy, Debug)]
pub struct Column {
    pub x: f32,
    pub head: f32,
    pub stem: f32,
}

/// Something else the slur meets: inside it (a staccato), or to go
/// around (an accidental, another slur), with its penalty.
#[derive(Clone, Copy, Debug)]
pub struct Extra {
    pub x: (f32, f32),
    pub y: (f32, f32),
    pub inside: bool,
    pub penalty: f32,
}

#[derive(Clone, Debug)]
pub struct Problem {
    /// +1 below the notes, −1 above (y down).
    pub dir: f32,
    /// The top line of the staff.
    pub staff_top: f32,
    pub ends: [End; 2],
    pub columns: Vec<Column>,
    pub extra: Vec<Extra>,
    /// Points the slur must also clear (other slurs' middles).
    pub avoid_slurs: Vec<(f32, f32)>,
    /// From one staff to another: each end stays within its own region
    /// (not drawn to the other end's height).
    pub cross_staff: bool,
}

// ------------------------------------------------------------- geometry --

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bezier(pub [(f32, f32); 4]);

impl Bezier {
    pub fn point(&self, t: f32) -> (f32, f32) {
        let u = 1.0 - t;
        let [a, b, c, d] = self.0;
        let f = |i: usize| {
            let v = |p: (f32, f32)| if i == 0 { p.0 } else { p.1 };
            u * u * u * v(a) + 3.0 * u * u * t * v(b) + 3.0 * u * t * t * v(c) + t * t * t * v(d)
        };
        (f(0), f(1))
    }
    /// The y where the curve passes `x` (the curve's x rising with t).
    pub fn y_at(&self, x: f32) -> f32 {
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        for _ in 0..40 {
            let m = (lo + hi) / 2.0;
            if self.point(m).0 < x {
                lo = m;
            } else {
                hi = m;
            }
        }
        self.point((lo + hi) / 2.0).1
    }
    /// The t where it runs level (dy/dt = 0), if it does.
    fn level_t(&self) -> Option<f32> {
        let [a, b, c, d] = self.0.map(|p| p.1);
        // dy/dt = 3[(b−a)(1−t)² + 2(c−b)(1−t)t + (d−c)t²]
        let (p, q, r) = (b - a, c - b, d - c);
        let (qa, qb, qc) = (p - 2.0 * q + r, 2.0 * (q - p), p);
        let roots: Vec<f32> = if qa.abs() < 1e-9 {
            if qb.abs() < 1e-9 { vec![] } else { vec![-qc / qb] }
        } else {
            let disc = qb * qb - 4.0 * qa * qc;
            if disc < 0.0 { vec![] } else { [(-qb - disc.sqrt()) / (2.0 * qa), (-qb + disc.sqrt()) / (2.0 * qa)].to_vec() }
        };
        roots.into_iter().filter(|t| (0.0..=1.0).contains(t)).reduce(f32::min)
    }
}

/// LilyPond's F(x) = 2/π · atan(πx/2): rising as x, then levelling off.
fn f0_1(x: f32) -> f32 {
    2.0 / std::f32::consts::PI * (std::f32::consts::PI * x / 2.0).atan()
}

/// A slur's height and indent for its width (`get_slur_indent_height`).
pub fn indent_height(width: f32) -> (f32, f32) {
    let max_fraction = 1.0 / 3.1;
    let h_inf = p::HEIGHT_LIMIT;
    let height = f0_1(width * p::RATIO / h_inf) * h_inf;
    let q = 2.0 * h_inf / max_fraction;
    let indent = 2.0 * h_inf - q * q * max_fraction / (width + q);
    (indent, height)
}

// ---------------------------------------------------------- the solver --

/// The problem turned over: y up, as LilyPond has it.
struct State {
    dir: f32,
    staff_top: f32,
    ends: [End; 2],
    columns: Vec<Column>,
    extra: Vec<Extra>,
    avoid_slurs: Vec<(f32, f32)>,
    cross_staff: bool,
    base: [(f32, f32); 2],
    musical_dy: f32,
    edge_has_beams: bool,
    broken: bool,
}

struct Config {
    ends: [(f32, f32); 2],
    curve: Bezier,
    height: f32,
    score: f32,
}

fn flip_y(y: f32) -> f32 {
    -y
}

impl State {
    fn new(pb: &Problem) -> State {
        // y up: every y negated, and the direction with it.
        let flip_stem = |s: Stem| Stem { y: (flip_y(s.y.1), flip_y(s.y.0)), ..s };
        let ends = pb.ends.map(|e| End {
            head: e.head.map(|(x, y)| (x, flip_y(y))),
            stem: e.stem.map(flip_stem),
            column_edge: flip_y(e.column_edge),
            broken_at: (e.broken_at.0, flip_y(e.broken_at.1)),
        });
        let mut st = State {
            dir: -pb.dir,
            staff_top: flip_y(pb.staff_top),
            ends,
            columns: pb.columns.iter().map(|c| Column { x: c.x, head: flip_y(c.head), stem: flip_y(c.stem) }).collect(),
            extra: pb.extra.iter().map(|e| Extra { y: (flip_y(e.y.1), flip_y(e.y.0)), ..*e }).collect(),
            avoid_slurs: pb.avoid_slurs.iter().map(|&(x, y)| (x, flip_y(y))).collect(),
            cross_staff: pb.cross_staff,
            base: [(0.0, 0.0); 2],
            musical_dy: 0.0,
            edge_has_beams: false,
            broken: ends.iter().any(|e| e.head.is_none()),
        };
        st.base = [0, 1].map(|d| st.base_attachment(d));
        if !st.broken {
            st.musical_dy = st.ends[1].head.map(|h| h.1).unwrap_or(0.0) - st.ends[0].head.map(|h| h.1).unwrap_or(0.0);
        }
        st.edge_has_beams = st.ends.iter().any(|e| e.stem.is_some_and(|s| s.beamed));
        st
    }

    /// A staff line at this y (within the five lines)?
    fn on_line(&self, y: f32) -> bool {
        // The staff's lines are at top, top−1 … top−4 (y up).
        let rel = self.staff_top - y;
        (-0.01..=4.01).contains(&rel) && (rel - rel.round()).abs() < 1e-3
    }

    /// Off a staff line by 0.15 outwards, when within 0.1 of one.
    fn move_away_from_staffline(&self, y: f32) -> f32 {
        let pos = (y - self.staff_top) * 2.0;
        let r = pos.round();
        if (pos - r).abs() < 0.2 && self.on_line(self.staff_top + r / 2.0) && (r as i32).rem_euclid(2) == 0 {
            y + 1.5 * self.dir / 10.0
        } else {
            y
        }
    }

    /// `get_base_attachments`: on the stem's tip when the stem points the
    /// slur's way and is beamed on the inner side, else half a space off
    /// the head; at the head's centre.
    fn base_attachment(&self, d: usize) -> (f32, f32) {
        let e = &self.ends[d];
        let Some((hx, hy)) = e.head else { return e.broken_at };
        let y = match e.stem {
            Some(s) if s.toward && s.beamed_inward => if self.dir > 0.0 { s.y.1 } else { s.y.0 },
            _ => hy + self.dir * 0.5,
        };
        let y = self.move_away_from_staffline(y + self.dir * 0.5);
        ((hx.0 + hx.1) / 2.0, y)
    }

    /// `get_y_attachment_range`: how far out each end may go.
    fn end_ys(&self) -> [f32; 2] {
        let dir = self.dir;
        let mut ys = [0, 1].map(|d| {
            let e = &self.ends[d];
            if e.head.is_some() {
                let other = if self.cross_staff { f32::MIN } else { dir * self.base[1 - d].1 };
                dir * (dir * (self.base[d].1 + p::REGION_SIZE * dir)).max(dir * (dir + e.column_edge)).max(other)
            } else {
                self.base[d].1 + p::REGION_SIZE * dir
            }
        });
        // Objects inside the slur widen the range at the end they are near.
        let mut extra_ys = [0.0f32; 2];
        for x in self.extra.iter().filter(|x| x.inside) {
            let cx = (x.x.0 + x.x.1) / 2.0;
            let (bl, br) = (self.base[0].0, self.base[1].0);
            if (br - bl).abs() < 1e-6 {
                continue;
            }
            let t = (cx - bl) / (br - bl);
            let y_place = ys[0] + (ys[1] - ys[0]) * t;
            let place = if dir > 0.0 { x.y.1 } else { x.y.0 };
            if dir * place > dir * y_place {
                for (e, k) in extra_ys.iter_mut().zip([1.0 - t, t]) {
                    let add = dir * (p::ENCOMPASS_OBJECT_RANGE_OVERSHOOT + (place - y_place) * dir * k);
                    *e = if dir > 0.0 { e.max(add) } else { e.min(add) };
                }
            }
        }
        for (y, extra) in ys.iter_mut().zip(extra_ys) {
            *y += extra;
        }
        ys
    }

    /// `enumerate_attachments`: ends from the base out in half spaces,
    /// moved onto a stem pointing into the slur when they meet it.
    fn enumerate(&self, end_ys: [f32; 2]) -> Vec<[(f32, f32); 2]> {
        let dir = self.dir;
        let mut out = Vec::new();
        let mut yl = self.base[0].1;
        let mut guard = 0;
        while dir * yl <= dir * end_ys[0] + 1e-6 && guard < 64 {
            let mut yr = self.base[1].1;
            while dir * yr <= dir * end_ys[1] + 1e-6 && guard < 64 * 64 {
                guard += 1;
                let mut os = [(self.base[0].0, yl), (self.base[1].0, yr)];
                let mut on_stem = [false; 2];
                for d in 0..2 {
                    let side = if d == 0 { -1.0 } else { 1.0 };
                    if let Some(s) = self.ends[d].stem.filter(|s| s.toward) {
                        let (lo, hi) = (s.y.0 - 0.25, s.y.1 + 0.25);
                        if (lo..=hi).contains(&os[d].1) {
                            // Beside the stem, on the inner side.
                            os[d].0 = if side < 0.0 { s.x.1 } else { s.x.0 } - side * 0.3;
                            on_stem[d] = true;
                        } else if dir * (if dir > 0.0 { s.y.1 } else { s.y.0 }) < dir * os[d].1 {
                            os[d].0 = (s.x.0 + s.x.1) / 2.0;
                        }
                    }
                }
                let dz = (os[1].0 - os[0].0, os[1].1 - os[0].1);
                if dz.0 < p::MINIMUM_LENGTH || (dz.1 / dz.0).abs() > p::MAX_SLOPE {
                    for d in 0..2 {
                        if let Some((hx, _)) = self.ends[d].head {
                            os[d].0 = (hx.0 + hx.1) / 2.0;
                            on_stem[d] = false;
                        }
                    }
                }
                let dz = (os[1].0 - os[0].0, os[1].1 - os[0].1);
                let len = (dz.0 * dz.0 + dz.1 * dz.1).sqrt().max(1e-6);
                for d in 0..2 {
                    if let (Some((hx, _)), false) = (self.ends[d].head, on_stem[d]) {
                        // Tilted slurs move a little sideways, more for more tilt.
                        os[d].0 -= dir * (hx.1 - hx.0) * (dz.1 / len) / 3.0;
                    }
                }
                out.push(os);
                yr += dir * 0.5;
            }
            yl += dir * 0.5;
        }
        out
    }

    /// `generate_avoid_offsets`: the inner notes' outermost points, other
    /// slurs' middles, objects inside.
    fn avoid(&self) -> Vec<(f32, f32)> {
        let dir = self.dir;
        let n = self.columns.len();
        let mut out: Vec<(f32, f32)> = self
            .columns
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != 0 && *i + 1 != n)
            .map(|(_, c)| (c.x, dir * (dir * c.head).max(dir * c.stem) + dir * p::FREE_HEAD_DISTANCE))
            .collect();
        out.extend(self.avoid_slurs.iter().map(|&(x, y)| (x, y + dir * p::FREE_SLUR_DISTANCE)));
        out.extend(self.extra.iter().filter(|e| e.inside).map(|e| ((e.x.0 + e.x.1) / 2.0, if dir > 0.0 { e.y.1 } else { e.y.0 })));
        out
    }

    /// `generate_curve`: the Bézier between the ends, raised to clear `avoid`.
    fn curve(&self, ends: [(f32, f32); 2], avoid: &[(f32, f32)]) -> (Bezier, f32) {
        let dz = (ends[1].0 - ends[0].0, ends[1].1 - ends[0].1);
        let len = (dz.0 * dz.0 + dz.1 * dz.1).sqrt().max(1e-4);
        let unit = (dz.0 / len, dz.1 / len);
        let perp = (-unit.1, unit.0);
        let (indent, mut height) = indent_height(len);
        let indent = indent.min(len / 3.1);
        let max_h = {
            let v = len * len / 3.0 - 0.75 * (indent + len / 3.0).powi(2);
            if v < 0.0 { len / 3.0 } else { v.sqrt() }
        };
        let make = |h: f32| {
            let off = (perp.0 * h * self.dir, perp.1 * h * self.dir);
            Bezier([ends[0], (ends[0].0 + off.0 + unit.0 * indent, ends[0].1 + off.1 + unit.1 * indent), (ends[1].0 + off.0 - unit.0 * indent, ends[1].1 + off.1 - unit.1 * indent), ends[1]])
        };
        // fit_factor: in the slur's own frame, how much higher it must be.
        let c = make(height);
        let local = |q: (f32, f32)| {
            let z = (q.0 - ends[0].0, q.1 - ends[0].1);
            (z.0 * unit.0 + z.1 * unit.1, self.dir * (z.0 * perp.0 + z.1 * perp.1))
        };
        let lc = Bezier(c.0.map(local));
        let mut ff = 0.0f32;
        for &a in avoid {
            let pz = local(a);
            let near_edge = pz.0 < p::CLOSE_TO_EDGE_LENGTH || len - pz.0 < p::CLOSE_TO_EDGE_LENGTH;
            if near_edge || pz.0 <= 0.0 || pz.0 >= len {
                continue;
            }
            let y = lc.y_at(pz.0);
            if y.abs() > 1e-6 {
                ff = ff.max(pz.1 / y);
            }
        }
        height = height.max((height * ff).min(max_h));
        (self.avoid_staff_line(make(height)), height)
    }

    /// `avoid_staff_line`: the curve's level point moved off a staff line.
    fn avoid_staff_line(&self, mut b: Bezier) -> Bezier {
        if self.broken {
            return b;
        }
        let Some(t) = b.level_t() else { return b };
        let y = b.point(t).1;
        let factor = 3.0 * t * (1.0 - t);
        let pos = 2.0 * (y - self.staff_top);
        let mut r = pos.round();
        if !self.line_at_pos(r) {
            r += if pos > r { 1.0 } else { -1.0 };
        }
        if !self.line_at_pos(r) {
            return b;
        }
        let distance = (pos - r) / 2.0;
        let min = 0.5 * 0.12 * factor + p::LINE_THICKNESS + if self.dir * distance > 0.0 { p::GAP_TO_STAFFLINE_INSIDE } else { p::GAP_TO_STAFFLINE_OUTSIDE };
        if distance.abs() < min && factor > 1e-6 {
            let dy = if distance > 0.0 { 1.0 } else { -1.0 } * (min - distance.abs());
            for i in [1, 2] {
                b.0[i].1 += dy / factor;
            }
        }
        b
    }

    /// Is half-space position `pos` (from the top line, y up) on a line?
    fn line_at_pos(&self, pos: f32) -> bool {
        let p = pos as i32;
        (-8..=0).contains(&p) && p % 2 == 0
    }

    #[allow(dead_code)]
    fn score(&self, c: &Config) -> f32 {
        self.score_edges(c) + self.score_slopes(c) + self.score_encompass(c) + self.score_extra(c)
    }

    fn score_edges(&self, c: &Config) -> f32 {
        let slope = (c.ends[1].1 - c.ends[0].1) / (c.ends[1].0 - c.ends[0].0);
        let mut total = 0.0;
        for d in 0..2 {
            let side = if d == 0 { -1.0 } else { 1.0 };
            let mut dem = p::EDGE_ATTRACTION_FACTOR * (c.ends[d].1 - self.base[d].1).abs();
            if self.ends[d].stem.is_some_and(|s| s.toward && !s.beamed_inward) {
                dem /= 5.0;
            }
            dem *= (self.dir * side * slope * p::EDGE_SLOPE_EXPONENT).exp();
            total += dem;
        }
        total
    }

    fn score_slopes(&self, c: &Config) -> f32 {
        let dy = self.musical_dy;
        let dz = (c.ends[1].0 - c.ends[0].0, c.ends[1].1 - c.ends[0].1);
        let slur_dy = dz.1;
        let steep = ((slur_dy / dz.0).abs() - p::MAX_SLOPE).max(0.0) * p::MAX_SLOPE_FACTOR;
        // (LilyPond counts the over-steep slope twice.)
        let mut dem = 2.0 * steep;
        let mut max_dy = dy.abs() + 0.2;
        if self.edge_has_beams {
            max_dy += 1.0;
        }
        if !self.broken {
            dem += p::STEEPER_SLOPE_FACTOR * (slur_dy.abs() - max_dy).max(0.0);
        }
        if dy.abs() < 0.01 && slur_dy.abs() > 0.01 && !self.broken {
            dem += p::NON_HORIZONTAL_PENALTY;
        }
        if dy.abs() > 1e-9 && slur_dy.abs() > 1e-9 && !self.broken && dy.signum() != slur_dy.signum() {
            dem += if self.edge_has_beams { p::SAME_SLOPE_PENALTY / 10.0 } else { p::SAME_SLOPE_PENALTY };
        }
        dem
    }

    fn score_encompass(&self, c: &Config) -> f32 {
        let dir = self.dir;
        let mut dem = 0.0;
        let mut convex: Vec<f32> = Vec::new();
        let n = self.columns.len();
        for (j, col) in self.columns.iter().enumerate() {
            let (l_edge, r_edge) = (j == 0, j + 1 == n);
            if !(col.x < c.ends[1].0 && col.x > c.ends[0].0) {
                continue;
            }
            let y = c.curve.y_at(col.x);
            if !(l_edge || r_edge) {
                let head_dy = y - col.head;
                if dir * head_dy < 0.0 {
                    dem += p::HEAD_ENCOMPASS_PENALTY;
                    convex.push(0.0);
                } else {
                    let hd = if head_dy != 0.0 { 1.0 / head_dy.abs() - 1.0 / p::FREE_HEAD_DISTANCE } else { p::HEAD_ENCOMPASS_PENALTY };
                    dem += hd.clamp(0.0, p::HEAD_ENCOMPASS_PENALTY);
                }
                let t = (col.x - c.ends[0].0) / (c.ends[1].0 - c.ends[0].0);
                let line_y = c.ends[0].1 + (c.ends[1].1 - c.ends[0].1) * t;
                let point = if dir > 0.0 { col.head.max(col.stem) } else { col.head.min(col.stem) };
                let closest = dir * (dir * point).max(dir * line_y);
                convex.push((closest - y).abs());
            }
            if dir * (y - col.stem) < 0.0 {
                let mut s = p::STEM_ENCOMPASS_PENALTY;
                if (l_edge && dir > 0.0) || (r_edge && dir < 0.0) {
                    s /= 5.0;
                }
                dem += s;
            }
        }
        if !convex.is_empty() {
            let mut n = convex.len();
            let min = convex.iter().copied().fold(f32::INFINITY, f32::min);
            let mut avg: f32 = convex.iter().sum();
            if n <= 2 {
                avg += c.height;
                n += 1;
            }
            avg /= n as f32;
            let mut v = p::HEAD_SLUR_DISTANCE_MAX_RATIO;
            if min > 0.0 {
                v = (avg / (min + p::ABSOLUTE_CLOSENESS_MEASURE) - 1.0).min(v);
            }
            dem += v.max(0.0) * p::HEAD_SLUR_DISTANCE_FACTOR;
        }
        dem
    }

    fn score_extra(&self, c: &Config) -> f32 {
        let mut dem = 0.0;
        for e in &self.extra {
            // Over an end's head: the end's height; else the curve's there.
            let over_end = (0..2).find(|&d| self.ends[d].head.is_some_and(|(hx, _)| e.x.0 < hx.1 && e.x.1 > hx.0));
            let y = match over_end {
                Some(d) => c.ends[d].1,
                None => {
                    let x = (e.x.0 + e.x.1) / 2.0;
                    if !(c.ends[0].0..=c.ends[1].0).contains(&x) {
                        continue;
                    }
                    c.curve.y_at(x)
                }
            };
            let dist = if e.inside {
                self.dir * (y - if self.dir > 0.0 { e.y.1 } else { e.y.0 })
            } else if y < e.y.0 {
                e.y.0 - y
            } else if y > e.y.1 {
                y - e.y.1
            } else {
                0.0
            };
            dem += e.penalty * peak_around(0.1 * p::EXTRA_ENCOMPASS_FREE_DISTANCE, p::EXTRA_ENCOMPASS_FREE_DISTANCE, dist.max(0.0));
        }
        dem
    }
}

/// LilyPond's `peak_around`: 1 at 0, falling to 0 at `threshold`.
fn peak_around(eps: f32, threshold: f32, x: f32) -> f32 {
    if x < 0.0 {
        return 1.0;
    }
    (-eps * (x - threshold) / ((x + eps) * threshold)).max(0.0)
}

/// The best slur for the problem: its four control points, y down.
pub fn solve(pb: &Problem) -> Bezier {
    let st = State::new(pb);
    let end_ys = st.end_ys();
    let avoid = st.avoid();
    // As LilyPond does: the cheap scorers first (the ends and the slope
    // need no curve), cheapest candidates first; every score is ≥ 0, so
    // once a candidate's cheap part alone is no better than the best
    // complete score, it and all after it are out.
    let mut cands: Vec<([(f32, f32); 2], f32)> = st
        .enumerate(end_ys)
        .into_iter()
        .filter(|e| e[1].0 - e[0].0 >= 0.01)
        .map(|ends| {
            let probe = Config { ends, curve: Bezier([ends[0]; 4]), height: 0.0, score: 0.0 };
            (ends, st.score_edges(&probe) + st.score_slopes(&probe))
        })
        .collect();
    cands.sort_by(|a, b| a.1.total_cmp(&b.1));
    let mut best: Option<Config> = None;
    for (ends, cheap) in cands {
        if best.as_ref().is_some_and(|b| cheap >= b.score) {
            break;
        }
        let (curve, height) = st.curve(ends, &avoid);
        let mut c = Config { ends, curve, height, score: 0.0 };
        c.score = cheap + st.score_encompass(&c) + st.score_extra(&c);
        if best.as_ref().is_none_or(|b| c.score < b.score) {
            best = Some(c);
        }
    }
    let curve = best.map(|b| b.curve).unwrap_or_else(|| {
        // Nothing to choose from (too short): a flat arc between the bases.
        let (a, b) = (st.base[0], st.base[1]);
        let h = st.dir * 0.5;
        Bezier([a, (a.0 + (b.0 - a.0) / 3.0, a.1 + h), (a.0 + 2.0 * (b.0 - a.0) / 3.0, b.1 + h), b])
    });
    Bezier(curve.0.map(|(x, y)| (x, flip_y(y))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(x: f32, y: f32) -> (End, Column) {
        let e = End { head: Some(((x - 0.6, x + 0.6), y)), stem: None, column_edge: y + 0.5, broken_at: (0.0, 0.0) };
        (e, Column { x, head: y + 0.5, stem: y + 0.5 })
    }

    #[test]
    fn the_height_and_indent_laws_are_lilyponds() {
        // Short slurs: height ≈ width·ratio; long ones tend to the limit.
        let (i, h) = indent_height(1.0);
        assert!((h - 0.25).abs() < 0.01 && i > 0.0 && i < 1.0 / 3.0 + 0.01, "{i} {h}");
        let (_, h) = indent_height(1000.0);
        assert!(h > 1.98 && h < 2.0);
        // The indent rises towards twice the height limit.
        assert!(indent_height(6.0).0 > 1.2 && indent_height(6.0).0 < 1.4);
    }

    #[test]
    fn a_slur_under_four_notes_on_a_line_hugs_them() {
        // Four heads on the middle line (y = 2 below a top at 0), slur below.
        let xs = [0.0, 3.0, 6.0, 9.0];
        let parts: Vec<(End, Column)> = xs.iter().map(|&x| note(x, 2.0)).collect();
        let pb = Problem { dir: 1.0, staff_top: 0.0, ends: [parts[0].0, parts[3].0], columns: parts.iter().map(|p| p.1).collect(), extra: vec![], avoid_slurs: vec![], cross_staff: false };
        let b = solve(&pb);
        let (start, end) = (b.0[0], b.0[3]);
        // Ends about a space under the heads' centres, at their middles.
        assert!((start.1 - 3.0).abs() < 0.6 && (end.1 - 3.0).abs() < 0.6, "{b:?}");
        assert!((start.0 - 0.0).abs() < 0.5 && (end.0 - 9.0).abs() < 0.5);
        // Below every inner head, bulging downwards.
        for &x in &xs[1..3] {
            assert!(b.y_at(x) > 2.5 + 0.2, "{x}: {}", b.y_at(x));
        }
        assert!(b.point(0.5).1 > start.1);
    }

    #[test]
    fn a_high_note_in_the_middle_lifts_the_slur() {
        let mut parts: Vec<(End, Column)> = [0.0, 3.0, 6.0].iter().map(|&x| note(x, 2.0)).collect();
        parts[1] = note(3.0, -2.0); // two spaces above the staff
        let pb = Problem { dir: -1.0, staff_top: 0.0, ends: [parts[0].0, parts[2].0], columns: parts.iter().map(|p| { let mut c = p.1; c.head -= 1.0; c.stem -= 1.0; c }).collect(), extra: vec![], avoid_slurs: vec![], cross_staff: false };
        let pb = Problem { ends: pb.ends.map(|mut e| { e.column_edge -= 1.0; e }), ..pb };
        let b = solve(&pb);
        assert!(b.y_at(3.0) < -2.5, "over the high note: {}", b.y_at(3.0));
    }
}
