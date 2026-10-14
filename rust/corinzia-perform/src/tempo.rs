//! Musical time <-> seconds, with constant tempo segments and linear ramps
//! (bpm linear in quarters):
//!   t(q) = 60/k · ln(bpm(q)/bpm0)            (k ≠ 0)
//!   q(t) = bpm0 · (exp(k t / 60) − 1) / k

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TempoPoint {
    pub quarter: f64,
    pub bpm: f64,
    pub ramp_to_next: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeterPoint {
    pub quarter: f64,
    pub num: i64,
    pub den: i64,
}

#[derive(Clone, Copy, Debug)]
struct Segment {
    start_quarter: f64,
    start_second: f64,
    bpm0: f64,
    slope: f64, // d(bpm)/d(quarter)
}

impl Segment {
    fn seconds(&self, dq: f64) -> f64 {
        if self.slope.abs() < 1e-12 {
            60.0 * dq / self.bpm0
        } else {
            60.0 / self.slope * ((self.bpm0 + self.slope * dq) / self.bpm0).ln()
        }
    }
    fn quarters(&self, dt: f64) -> f64 {
        if self.slope.abs() < 1e-12 {
            dt * self.bpm0 / 60.0
        } else {
            self.bpm0 * ((self.slope * dt / 60.0).exp() - 1.0) / self.slope
        }
    }
}

#[derive(Clone, Debug)]
pub struct TempoMap {
    segs: Vec<Segment>,
    meters: Vec<(f64, i64, i64, i64)>, // (quarter, first bar number, num, den)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BarInfo {
    pub bar: i64,
    pub start_quarter: f64,
    pub num: i64,
    pub den: i64,
}

impl TempoMap {
    /// `points` sorted by quarter, the first at 0.
    pub fn new(points: &[TempoPoint], meters: &[MeterPoint]) -> TempoMap {
        let mut segs = Vec::with_capacity(points.len());
        let mut sec = 0.0;
        for (i, p) in points.iter().enumerate() {
            let mut s = Segment { start_quarter: p.quarter, start_second: sec, bpm0: p.bpm, slope: 0.0 };
            if let Some(next) = points.get(i + 1) {
                let dq = next.quarter - p.quarter;
                if p.ramp_to_next && dq > 0.0 {
                    s.slope = (next.bpm - p.bpm) / dq;
                }
                sec += s.seconds(dq);
            }
            segs.push(s);
        }
        if segs.is_empty() {
            segs.push(Segment { start_quarter: 0.0, start_second: 0.0, bpm0: 120.0, slope: 0.0 });
        }
        let mut ms = Vec::new();
        let mut bar = 1;
        for (i, m) in meters.iter().enumerate() {
            ms.push((m.quarter, bar, m.num, m.den));
            if let Some(next) = meters.get(i + 1) {
                let len = 4.0 * m.num as f64 / m.den as f64;
                bar += ((next.quarter - m.quarter) / len).round() as i64;
            }
        }
        if ms.is_empty() {
            ms.push((0.0, 1, 4, 4));
        }
        TempoMap { segs, meters: ms }
    }

    fn seg_by_quarter(&self, q: f64) -> &Segment {
        let i = self.segs.partition_point(|s| s.start_quarter <= q);
        &self.segs[i.saturating_sub(1)]
    }

    pub fn seconds_at(&self, q: f64) -> f64 {
        let s = self.seg_by_quarter(q);
        s.start_second + s.seconds(q - s.start_quarter)
    }

    pub fn quarter_at(&self, t: f64) -> f64 {
        let i = self.segs.partition_point(|s| s.start_second <= t);
        let s = &self.segs[i.saturating_sub(1)];
        s.start_quarter + s.quarters(t - s.start_second)
    }

    pub fn tempo_at(&self, q: f64) -> f64 {
        let s = self.seg_by_quarter(q);
        s.bpm0 + s.slope * (q - s.start_quarter)
    }

    pub fn bar_at(&self, q: f64) -> BarInfo {
        let i = self.meters.partition_point(|m| m.0 <= q);
        let (start, bar, num, den) = self.meters[i.saturating_sub(1)];
        let len = 4.0 * num as f64 / den as f64;
        let n = ((q - start) / len + 1e-9).floor() as i64;
        BarInfo { bar: bar + n, start_quarter: start + n as f64 * len, num, den }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn constant_and_ramped_tempo() {
        let pts = [
            TempoPoint { quarter: 0.0, bpm: 120.0, ramp_to_next: false },
            TempoPoint { quarter: 4.0, bpm: 60.0, ramp_to_next: true },
            TempoPoint { quarter: 8.0, bpm: 120.0, ramp_to_next: false },
        ];
        let m = TempoMap::new(&pts, &[MeterPoint { quarter: 0.0, num: 3, den: 4 }]);
        assert!((m.seconds_at(4.0) - 2.0).abs() < 1e-9);
        // Ramp 60 -> 120 over 4 quarters: 60/15 · ln 2 seconds.
        assert!((m.seconds_at(8.0) - (2.0 + 4.0 * std::f64::consts::LN_2)).abs() < 1e-9);
        for q in [0.5, 3.0, 5.5, 7.9, 9.0] {
            assert!((m.quarter_at(m.seconds_at(q)) - q).abs() < 1e-9);
        }
        assert!((m.tempo_at(6.0) - 90.0).abs() < 1e-9);
        assert_eq!(m.bar_at(7.0), BarInfo { bar: 3, start_quarter: 6.0, num: 3, den: 4 });
    }
}
