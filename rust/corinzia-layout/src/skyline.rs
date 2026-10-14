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
