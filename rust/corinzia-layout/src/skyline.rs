//! Collision avoidance by skylines: per staff, the boxes of everything drawn
//! so far. A mark is placed above (below) the highest (lowest) box it would
//! overlap horizontally, then added itself, so marks stack instead of colliding.

/// A box in staff spaces: x0, y0 (top), x1, y1 (bottom), y down.
pub type Rect = [f32; 4];

#[derive(Clone, Debug, Default)]
pub struct Skyline {
    pub staff_top: f32,
    boxes: Vec<Rect>,
}

impl Skyline {
    pub fn new(staff_top: f32) -> Self {
        Skyline { staff_top, boxes: Vec::new() }
    }
    pub fn add(&mut self, r: Rect) {
        if r.iter().all(|v| v.is_finite()) {
            self.boxes.push(r);
        }
    }
    fn overlapping(&self, x0: f32, x1: f32) -> impl Iterator<Item = &Rect> {
        self.boxes.iter().filter(move |b| b[0] < x1 && b[2] > x0)
    }
    /// The highest y (smallest value) occupied over [x0, x1]; the staff's top line at least.
    pub fn top_over(&self, x0: f32, x1: f32) -> f32 {
        self.overlapping(x0, x1).map(|b| b[1]).fold(self.staff_top, f32::min)
    }
    /// How far below this staff's top line the top line of `below` must be
    /// so that nothing of either comes closer than `padding` where they
    /// overlap horizontally (staff lines included).
    pub fn clearance_above(&self, below: &Skyline, padding: f32) -> f32 {
        let mut need = 4.0 + padding;
        for a in &self.boxes {
            for b in &below.boxes {
                if a[0] < b[2] && b[0] < a[2] {
                    need = need.max((a[3] - self.staff_top) - (b[1] - below.staff_top) + padding);
                }
            }
            // Against the staff lines below.
            need = need.max((a[3] - self.staff_top) + padding);
        }
        for b in &below.boxes {
            // The staff lines above against what reaches up from below.
            need = need.max(4.0 - (b[1] - below.staff_top) + padding);
        }
        need
    }
    /// The lowest y (largest value) occupied over [x0, x1]; the bottom line at least.
    pub fn bottom_over(&self, x0: f32, x1: f32) -> f32 {
        self.overlapping(x0, x1).map(|b| b[3]).fold(self.staff_top + 4.0, f32::max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stacks_marks() {
        let mut s = Skyline::new(0.0);
        assert_eq!(s.top_over(0.0, 1.0), 0.0);
        s.add([0.0, -2.0, 1.0, 0.0]);
        assert_eq!(s.top_over(0.5, 2.0), -2.0);
        assert_eq!(s.top_over(1.5, 2.0), 0.0); // no overlap
        s.add([0.0, 4.0, 1.0, 6.5]);
        assert_eq!(s.bottom_over(0.0, 3.0), 6.5);
    }
}
