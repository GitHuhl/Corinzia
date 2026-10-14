//! Tuplets: `actual` notes in the time of `normal` over a span of a voice.
//! The events inside keep their sounding durations; these functions keep a
//! voice's tuplet list in step with its events.

use crate::note_input::{insert_event, make_event, EditResult};
use crate::Ids;
use corinzia_model::{Event, Tuplet};
use corinzia_score::Frac;

/// Sounding time per written time at `t`: the product of normal/actual of
/// every tuplet around it (1 outside tuplets).
pub fn sounding_scale(tuplets: &[Tuplet], t: Frac) -> Frac {
    tuplets.iter().filter(|x| x.contains(t)).fold(Frac::int(1), |f, x| f * Frac::new(x.normal, x.actual))
}

/// The innermost tuplet around `t`.
pub fn tuplet_at(tuplets: &[Tuplet], t: Frac) -> Option<Tuplet> {
    tuplets.iter().filter(|x| x.contains(t)).min_by(|a, b| a.length.cmp(&b.length)).copied()
}

fn sorted(mut v: Vec<Tuplet>) -> Vec<Tuplet> {
    v.sort_by(|a, b| a.start.cmp(&b.start).then(b.length.cmp(&a.length)));
    v.dedup();
    v
}

/// The tuplets still in force after an edit: one goes when an event
/// crosses its start or end, or when nothing starts inside it.
pub fn sanitize(events: &[Event], tuplets: &[Tuplet]) -> Vec<Tuplet> {
    let ok = |t: &Tuplet| {
        let crosses = |b: Frac| events.iter().any(|e| e.start < b && b < e.end());
        t.length > Frac::ZERO && !crosses(t.start) && !crosses(t.end()) && events.iter().any(|e| t.contains(e.start))
    };
    sorted(tuplets.iter().copied().filter(ok).collect())
}

/// Bars inserted at `at`: tuplets from there on move by `length`.
pub fn shift(tuplets: &[Tuplet], at: Frac, length: Frac) -> Vec<Tuplet> {
    tuplets.iter().map(|t| if t.start >= at { Tuplet { start: t.start + length, ..*t } } else { *t }).collect()
}

/// `[from, to)` deleted: tuplets touching it go, later ones move left.
pub fn cut(tuplets: &[Tuplet], from: Frac, to: Frac) -> Vec<Tuplet> {
    tuplets
        .iter()
        .filter(|t| t.end() <= from || t.start >= to)
        .map(|t| if t.start >= to { Tuplet { start: t.start - (to - from), ..*t } } else { *t })
        .collect()
}

/// The tuplets not lying inside `[from, to)` (a passage erased or
/// overwritten).
pub fn remove_within(tuplets: &[Tuplet], from: Frac, to: Frac) -> Vec<Tuplet> {
    tuplets.iter().copied().filter(|t| !(t.start >= from && t.end() <= to)).collect()
}

/// The tuplets inside `[from, to)`, moved to start from 0 (copy).
pub fn slice(tuplets: &[Tuplet], from: Frac, to: Frac) -> Vec<Tuplet> {
    tuplets.iter().filter(|t| t.start >= from && t.end() <= to).map(|t| Tuplet { start: t.start - from, ..*t }).collect()
}

/// Copied tuplets placed at `at` over `[at, at + length)`: what was
/// there goes.
pub fn paste(tuplets: &[Tuplet], clip: &[Tuplet], at: Frac, length: Frac) -> Vec<Tuplet> {
    let mut out = remove_within(tuplets, at, at + length);
    out.extend(clip.iter().map(|t| Tuplet { start: t.start + at, ..*t }));
    sorted(out)
}

/// The tuplets inside `[from, to)` moved by `by` (a passage nudged);
/// any the music lands on go.
pub fn moved(tuplets: &[Tuplet], from: Frac, to: Frac, by: Frac) -> Vec<Tuplet> {
    let (inside, outside): (Vec<Tuplet>, Vec<Tuplet>) = tuplets.iter().partition(|t| t.start >= from && t.start < to);
    let (t0, t1) = (from + by, to + by);
    let mut out: Vec<Tuplet> = outside.into_iter().filter(|t| t.end() <= t0 || t.start >= t1).collect();
    out.extend(inside.into_iter().map(|t| Tuplet { start: t.start + by, ..t }));
    sorted(out)
}

/// The usual ratio for a tuplet of `n`: in the time of the next lower
/// power of two (5:4, 7:4, 9:8), duplets and quadruplets in the time of 3.
pub fn default_normal(n: i64) -> i64 {
    match n {
        2 => 3,
        4 => 3,
        n if n > 2 => {
            let mut p = 1;
            while p * 2 < n {
                p *= 2;
            }
            p
        }
        _ => 1,
    }
}

/// The tuplet popover: "3", "5", "5:4", "7:8"; `None` if not a tuplet.
pub fn parse_tuplet(text: &str) -> Option<(i64, i64)> {
    let t = text.trim();
    let (a, n) = match t.split_once(':') {
        Some((a, n)) => (a.trim().parse::<i64>().ok()?, n.trim().parse::<i64>().ok()?),
        None => {
            let a = t.parse::<i64>().ok()?;
            (a, default_normal(a))
        }
    };
    (a >= 2 && n >= 1 && a <= 64 && n <= 64 && a != n).then_some((a, n))
}

/// A new tuplet of `actual:normal` at `at` whose notes are written as
/// `unit` (so it lasts `normal` units, written). It overwrites what is
/// there: `first`, if given, keeps its pitches as the first tuplet note;
/// the rest is a rest. `fill`: voice 1 (rests around it). Returns the
/// events and the voice's new tuplets.
#[allow(clippy::too_many_arguments)]
pub fn create(events: &[Event], tuplets: &[Tuplet], at: Frac, unit: Frac, actual: i64, normal: i64, first: Option<&Event>, fill: bool, ids: &mut Ids) -> Option<(EditResult, Vec<Tuplet>)> {
    if actual < 1 || normal < 1 || unit <= Frac::ZERO {
        return None;
    }
    let scale = sounding_scale(tuplets, at);
    let length = unit * Frac::int(normal) * scale;
    // Inside an enclosing tuplet the new one must fit.
    if let Some(outer) = tuplet_at(tuplets, at) {
        if at + length > outer.end() {
            return None;
        }
    }
    let t = Tuplet { start: at, length, actual, normal };
    let note = length * Frac::new(1, actual);
    let rest = make_event(ids, at, length, &[], 0);
    let mut r = insert_event(events, rest, fill, ids);
    let mut focus = r.focus;
    if let Some(f) = first.filter(|f| !f.heads.is_empty()) {
        let mut e = f.clone();
        e.start = at;
        e.duration = note;
        e.heads.iter_mut().for_each(|h| h.tie = false);
        let tail = make_event(ids, at + note, length - note, &[], 0);
        let r2 = insert_event(&r.events, tail, fill, ids);
        let r3 = insert_event(&r2.events, e, fill, ids);
        focus = r3.focus;
        r = r3;
    }
    let mut ts = remove_within(tuplets, at, at + length);
    // Partly overlapped tuplets go too.
    ts.retain(|x| x.end() <= at || x.start >= at + length || (x.start <= at && x.end() >= at + length));
    ts.push(t);
    let ts = sanitize(&r.events, &ts);
    Some((EditResult { focus, from: at, to: at + length, caret: if first.is_some() { at + note } else { at }, ..r }, ts))
}
