//! Exact musical time: whole-note fractions.

use std::cmp::Ordering;
use std::ops::{Add, Mul, Sub};

#[derive(Clone, Copy, Debug)]
pub struct Frac {
    pub num: i64,
    pub den: i64,
}

fn gcd(a: i64, b: i64) -> i64 {
    let (mut a, mut b) = (a.abs(), b.abs());
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.max(1)
}

pub fn lcm(a: i64, b: i64) -> i64 {
    a / gcd(a, b) * b
}

impl Frac {
    pub fn new(num: i64, den: i64) -> Self {
        let (num, den) = if den < 0 { (-num, -den) } else { (num, den) };
        let g = gcd(num, den);
        Frac { num: num / g, den: den / g }
    }
    pub const ZERO: Frac = Frac { num: 0, den: 1 };
    pub fn int(n: i64) -> Self {
        Frac { num: n, den: 1 }
    }
    pub fn is_zero(self) -> bool {
        self.num == 0
    }
}

impl Add for Frac {
    type Output = Frac;
    fn add(self, o: Frac) -> Frac {
        Frac::new(self.num * o.den + o.num * self.den, self.den * o.den)
    }
}
impl Sub for Frac {
    type Output = Frac;
    fn sub(self, o: Frac) -> Frac {
        Frac::new(self.num * o.den - o.num * self.den, self.den * o.den)
    }
}
impl Mul for Frac {
    type Output = Frac;
    fn mul(self, o: Frac) -> Frac {
        Frac::new(self.num * o.num, self.den * o.den)
    }
}
impl PartialEq for Frac {
    fn eq(&self, o: &Frac) -> bool {
        self.num * o.den == o.num * self.den
    }
}
impl Eq for Frac {}
impl PartialOrd for Frac {
    fn partial_cmp(&self, o: &Frac) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Frac {
    fn cmp(&self, o: &Frac) -> Ordering {
        (self.num as i128 * o.den as i128).cmp(&(o.num as i128 * self.den as i128))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arithmetic() {
        assert_eq!(Frac::new(1, 4) + Frac::new(1, 4), Frac::new(1, 2));
        assert_eq!(Frac::new(3, 8) - Frac::new(1, 8), Frac::new(1, 4));
        assert!(Frac::new(1, 3) < Frac::new(1, 2));
        assert_eq!(Frac::new(2, -4), Frac::new(-1, 2));
        assert_eq!(lcm(4, 6), 12);
    }
}
