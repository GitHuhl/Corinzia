//! Note values: which written durations can be shown as one note (plain or
//! dotted powers of two, triplets), and how to split any duration into them.

use crate::frac::Frac;

/// A notatable value: plain or dotted power of two, optionally a triplet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoteValue {
    pub base: i64, // 1 = whole, 2 = half ... (0 = breve)
    pub dots: u8,
    pub triplet: bool,
}

impl NoteValue {
    pub fn length(self) -> Frac {
        let plain = if self.base == 0 { Frac::int(2) } else { Frac::new(1, self.base) };
        let dotted = plain * Frac::new((1 << (self.dots + 1)) - 1, 1 << self.dots);
        if self.triplet { dotted * Frac::new(2, 3) } else { dotted }
    }
    /// MusicXML `<type>` name.
    pub fn type_name(self) -> &'static str {
        match self.base {
            0 => "breve",
            1 => "whole",
            2 => "half",
            4 => "quarter",
            8 => "eighth",
            16 => "16th",
            32 => "32nd",
            64 => "64th",
            _ => "128th",
        }
    }
}

pub fn candidates() -> Vec<NoteValue> {
    let mut v = Vec::new();
    for triplet in [false, true] {
        for base in [0, 1, 2, 4, 8, 16, 32, 64, 128] {
            for dots in 0..=2u8 {
                if base == 128 && dots > 0 {
                    continue;
                }
                v.push(NoteValue { base, dots, triplet });
            }
        }
    }
    v
}

/// Splits a duration into notatable values, longest first; plain values are
/// preferred, triplet values cover what plain ones cannot.
pub fn decompose(mut d: Frac) -> Vec<NoteValue> {
    let all = candidates();
    let mut out = Vec::new();
    while d > Frac::ZERO && out.len() < 32 {
        let triplety = d.den % 3 == 0;
        let best = all
            .iter()
            .filter(|v| v.triplet == triplety && v.length() <= d)
            .max_by(|a, b| a.length().cmp(&b.length()))
            .or_else(|| all.iter().filter(|v| v.length() <= d).max_by(|a, b| a.length().cmp(&b.length())));
        match best {
            Some(v) => {
                out.push(*v);
                d = d - v.length();
            }
            None => break, // below a 128th triplet: drop the sliver
        }
    }
    out
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decomposition() {
        let q = |n, d| Frac::new(n, d);
        assert_eq!(decompose(q(1, 4)), vec![NoteValue { base: 4, dots: 0, triplet: false }]);
        assert_eq!(decompose(q(3, 8)), vec![NoteValue { base: 4, dots: 1, triplet: false }]);
        assert_eq!(decompose(q(7, 8)), vec![NoteValue { base: 2, dots: 2, triplet: false }]);
        // 5/8 = half + eighth (tied)
        assert_eq!(decompose(q(5, 8)), vec![NoteValue { base: 2, dots: 0, triplet: false }, NoteValue { base: 8, dots: 0, triplet: false }]);
        assert_eq!(decompose(q(1, 12)), vec![NoteValue { base: 8, dots: 0, triplet: true }]);
        let sum = |v: Vec<NoteValue>| v.into_iter().fold(Frac::ZERO, |a, n| a + n.length());
        for d in [q(11, 16), q(13, 32), q(5, 12), q(2, 1), q(3, 1)] {
            assert_eq!(sum(decompose(d)), d, "{d:?}");
        }
    }

}
