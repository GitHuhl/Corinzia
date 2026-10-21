//! Beam grouping shared by the engraver and the MusicXML exchange: which
//! notes of a voice's bar share a beam, by the meter and the beams set by hand.

use crate::score::span;
use crate::Frac;

/// A meter's beam groups in units of 1/den: its own ("2+2+3"), or the
/// usual ones of 5/8, 7/8 and 8/8; None to beam by a uniform unit.
pub fn meter_groups(num: i64, den: i64, beams: &[u8; 8]) -> Option<Vec<u8>> {
    let own: Vec<u8> = beams.iter().copied().take_while(|b| *b > 0).collect();
    if !own.is_empty() {
        return Some(own);
    }
    match (num, den) {
        (5, 8) => Some(vec![3, 2]),
        (7, 8) => Some(vec![2, 2, 3]),
        (8, 8) => Some(vec![3, 3, 2]),
        _ => None,
    }
}

/// Which beam window of its bar a position (from the bar's written start)
/// falls in: by the meter's groups, or windows of `uniform`.
pub fn window(num: i64, den: i64, beams: &[u8; 8], rel: Frac, uniform: Frac) -> i64 {
    match meter_groups(num, den, beams) {
        Some(groups) => {
            let mut edge = Frac::ZERO;
            for (k, g) in groups.iter().enumerate() {
                edge = edge + Frac::new(*g as i64, den);
                if rel < edge {
                    return k as i64;
                }
            }
            groups.len() as i64
        }
        None => {
            let r = rel * Frac::new(uniform.den, uniform.num);
            r.num.div_euclid(r.den)
        }
    }
}

/// The beam grouping unit: dotted quarters in compound meters, halves in
/// 2/2 and 3/2, pairs of eighths in other x/8 meters, else the beat.
pub fn beam_beat(num: i64, den: i64) -> Frac {
    if den == 8 && num % 3 == 0 {
        Frac::new(3, 8)
    } else if den <= 2 {
        Frac::new(1, 2)
    } else if den >= 8 {
        Frac::new(2, den)
    } else {
        Frac::new(1, den)
    }
}

/// Eighths are beamed over half a bar of 4/4 and the whole bar of 3/4.
pub fn eighths_span(num: i64, den: i64) -> Option<Frac> {
    match (num, den) {
        (4, 4) => Some(Frac::new(1, 2)),
        (3, 4) => Some(Frac::new(3, 4)),
        _ => None,
    }
}

/// A beam's window and tuplet: notes with the same one share it.
type Key = (i64, Option<usize>);

/// A written note of one voice in one bar, for grouping.
#[derive(Clone, Copy, Debug)]
pub struct BeamNote {
    /// From the bar's written start.
    pub rel: Frac,
    /// Beams its value takes (an eighth 1, a sixteenth 2…); 0 for none.
    pub beams: u32,
    pub rest: bool,
    /// `span::BEAMS` bits set by hand.
    pub hand: u8,
    /// The innermost tuplet it is in, if any (beams stay inside one).
    pub tuplet: Option<usize>,
}

/// The beamed groups (indices, two notes or more) of a voice's bar, as the
/// engraver draws them: notes in one window share a beam (half a bar of
/// eighths in 4/4, else the beat), beams with shorter notes go back to
/// beats, and beams set by hand break, join or leave notes out.
pub fn groups(num: i64, den: i64, meter_beams: &[u8; 8], notes: &[BeamNote]) -> Vec<Vec<usize>> {
    let group = |idx: &[usize], unit: Frac| -> Vec<Vec<usize>> {
        let mut out = Vec::new();
        let mut cur: Vec<usize> = Vec::new();
        let mut cur_key: Option<Key> = None;
        let mut joined: Option<(Key, Key)> = None;
        for &i in idx {
            let n = &notes[i];
            let mut key = (window(num, den, meter_beams, n.rel, unit), n.tuplet);
            if n.hand & span::BEAM_JOIN != 0 {
                if let Some(k) = cur_key {
                    joined = Some((key, k));
                    key = k;
                }
            } else if let Some((_, to)) = joined.filter(|(from, _)| *from == key) {
                key = to;
            }
            let beamable = !n.rest && n.beams > 0 && n.hand & span::NO_BEAM == 0;
            if !beamable || cur_key != Some(key) || n.hand & span::BEAM_BREAK != 0 {
                if cur.len() >= 2 {
                    out.push(std::mem::take(&mut cur));
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
            out.push(cur);
        }
        out
    };
    let all: Vec<usize> = (0..notes.len()).collect();
    let beat = beam_beat(num, den);
    let mut out = Vec::new();
    for g in group(&all, eighths_span(num, den).unwrap_or(beat)) {
        if g.iter().any(|&i| notes[i].beams > 1) {
            out.extend(group(&g, beat));
        } else {
            out.push(g);
        }
    }
    out
}

/// MusicXML `<beam>` values of each note of `notes` (number 1 up): for
/// each level "begin", "continue", "end", "forward hook" or "backward hook".
pub fn xml_beams(notes: &[BeamNote], groups: &[Vec<usize>]) -> Vec<Vec<&'static str>> {
    let mut out = vec![Vec::new(); notes.len()];
    for g in groups {
        let levels = g.iter().map(|&i| notes[i].beams).max().unwrap_or(1);
        for level in 1..=levels {
            let mut k = 0;
            while k < g.len() {
                if notes[g[k]].beams < level {
                    k += 1;
                    continue;
                }
                let s = k;
                while k + 1 < g.len() && notes[g[k + 1]].beams >= level {
                    k += 1;
                }
                if s == k {
                    out[g[s]].push(if level == 1 { "continue" } else if s + 1 < g.len() && s == 0 { "forward hook" } else { "backward hook" });
                } else {
                    for (j, &i) in g[s..=k].iter().enumerate() {
                        out[i].push(if j == 0 { "begin" } else if s + j == k { "end" } else { "continue" });
                    }
                }
                k += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eighths(n: usize) -> Vec<BeamNote> {
        (0..n).map(|k| BeamNote { rel: Frac::new(k as i64, 8), beams: 1, rest: false, hand: 0, tuplet: None }).collect()
    }

    #[test]
    fn default_and_hand_groups() {
        assert_eq!(groups(4, 4, &[0; 8], &eighths(8)), [vec![0, 1, 2, 3], vec![4, 5, 6, 7]]);
        assert_eq!(groups(7, 8, &[0; 8], &eighths(7)), [vec![0, 1], vec![2, 3], vec![4, 5, 6]]);
        assert_eq!(groups(7, 8, &[3, 2, 2, 0, 0, 0, 0, 0], &eighths(7)), [vec![0, 1, 2], vec![3, 4], vec![5, 6]]);
        let mut n = eighths(8);
        n[2].hand = span::BEAM_BREAK;
        n[4].hand = span::BEAM_JOIN;
        n[7].hand = span::NO_BEAM;
        assert_eq!(groups(4, 4, &[0; 8], &n), [vec![0, 1], vec![2, 3, 4, 5, 6]]);
        let x = xml_beams(&n, &groups(4, 4, &[0; 8], &n));
        assert_eq!((x[0].as_slice(), x[1].as_slice(), x[3].as_slice(), x[6].as_slice(), x[7].len()), (&["begin"][..], &["end"][..], &["continue"][..], &["end"][..], 0));
    }
}
