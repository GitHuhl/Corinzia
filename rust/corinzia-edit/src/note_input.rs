//! Note input and range edits on one voice's events.

use crate::Ids;
use corinzia_model::{Event, EventKind, Grace, Head, Id, Pitch};
use corinzia_score::score::span;
use corinzia_score::Frac;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaseDuration {
    Whole = 0,
    Half,
    Quarter,
    Eighth,
    Sixteenth,
    ThirtySecond,
    SixtyFourth,
}
impl BaseDuration {
    pub const ALL: [BaseDuration; 7] = [
        BaseDuration::Whole,
        BaseDuration::Half,
        BaseDuration::Quarter,
        BaseDuration::Eighth,
        BaseDuration::Sixteenth,
        BaseDuration::ThirtySecond,
        BaseDuration::SixtyFourth,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DurationSpec {
    pub base: BaseDuration,
    pub dots: u8, // 0..2
}
impl Default for DurationSpec {
    fn default() -> Self {
        DurationSpec { base: BaseDuration::Quarter, dots: 0 }
    }
}
impl DurationSpec {
    pub fn value(&self) -> Frac {
        let mut v = Frac::new(1, 1 << (self.base as i64));
        let mut add = v;
        for _ in 0..self.dots {
            add = add * Frac::new(1, 2);
            v = v + add;
        }
        v
    }
    /// The undotted, non-tuplet value: the grid for mouse input.
    pub fn grid(&self) -> Frac {
        Frac::new(1, 1 << (self.base as i64))
    }
}

// ------------------------------------------------------------- pitch helpers --
pub fn diatonic(p: Pitch) -> i32 {
    p.step as i32 + 7 * p.octave as i32
}

pub fn from_diatonic(d: i32, alter: i32) -> Pitch {
    let step = d.rem_euclid(7);
    Pitch { step: step as i8, alter: alter as i8, octave: ((d - step) / 7) as i8 }
}

/// The alteration the key signature implies for a letter (0 = C .. 6 = B).
pub fn key_alter(fifths: i32, step: i32) -> i32 {
    // Order of sharps: F C G D A E B; flats the reverse.
    const SHARPS: [i32; 7] = [3, 0, 4, 1, 5, 2, 6];
    if fifths > 0 && SHARPS[..fifths.min(7) as usize].contains(&step) {
        return 1;
    }
    if fifths < 0 && SHARPS.iter().rev().take((-fifths).min(7) as usize).any(|&s| s == step) {
        return -1;
    }
    0
}

/// Letter `step` in the octave nearest `previous`; `alter` defaults to the key's.
pub fn nearest_pitch(step: i32, alter: Option<i32>, key_fifths: i32, previous: Pitch) -> Pitch {
    let prev = diatonic(previous);
    let mut best = 0;
    let mut best_dist = i32::MAX;
    for oct in previous.octave as i32 - 1..=previous.octave as i32 + 1 {
        let d = step + 7 * oct;
        let dist = (d - prev).abs();
        if dist < best_dist {
            best_dist = dist;
            best = d;
        }
    }
    from_diatonic(best, alter.unwrap_or_else(|| key_alter(key_fifths, step)))
}

// ------------------------------------------------------------------- results --
#[derive(Clone, Debug)]
pub struct EditResult {
    pub events: Vec<Event>,
    /// The element the caret/selection should follow.
    pub focus: Option<Id>,
    /// The musically affected range.
    pub from: Frac,
    pub to: Frac,
    /// Where the caret goes next.
    pub caret: Frac,
}

pub fn make_event(ids: &mut Ids, start: Frac, duration: Frac, pitches: &[Pitch], articulations: u16) -> Event {
    let id = ids.next();
    let heads: Vec<Head> = pitches.iter().map(|&p| Head { id: ids.next(), pitch: p, tie: false, velocity: 0 }).collect();
    let kind = match heads.len() {
        0 => EventKind::Rest,
        1 => EventKind::Note,
        _ => EventKind::Chord,
    };
    Event { id, kind, start, duration, heads, articulations, dynamic: None, spans: 0, lyrics: vec![], graces: vec![], fingering: vec![] }
}

fn rest_of(ids: &mut Ids, start: Frac, dur: Frac) -> Event {
    make_event(ids, start, dur, &[], 0)
}

fn sort_events(v: &mut [Event]) {
    v.sort_by(|a, b| a.start.cmp(&b.start)); // stable
}

fn first_focus(e: &Event) -> Id {
    e.heads.first().map(|h| h.id).unwrap_or(e.id)
}

fn untie(e: &mut Event) {
    e.heads.iter_mut().for_each(|h| h.tie = false);
}

/// Removes `[s, t)`: events crossing `s` keep their head part; remainders
/// past `t` become rests when `fill`.
fn clear_range(out: &mut Vec<Event>, input: &[Event], s: Frac, t: Frac, fill: bool, ids: &mut Ids) {
    for e in input {
        let end = e.end();
        if end <= s || e.start >= t {
            out.push(e.clone());
            continue;
        }
        if e.start < s {
            let mut kept = e.clone();
            kept.duration = s - e.start;
            untie(&mut kept);
            out.push(kept);
        }
        if end > t && fill {
            out.push(rest_of(ids, t, end - t));
        }
    }
}

/// Overwrite-insert `ev`. `fill`: the voice-1 policy.
pub fn insert_event(list: &[Event], mut ev: Event, fill: bool, ids: &mut Ids) -> EditResult {
    let (s, t) = (ev.start, ev.end());
    // Grace notes typed before the note (on the event it replaces) stay.
    if ev.graces.is_empty() {
        if let Some(old) = list.iter().find(|e| e.start == s && !e.graces.is_empty()) {
            ev.graces = old.graces.clone();
        }
    }
    let mut out = Vec::with_capacity(list.len() + 3);
    clear_range(&mut out, list, s, t, fill, ids);
    let mut from = s;
    if fill {
        let gap_start = out.iter().filter(|e| e.start < s).map(|e| e.end()).max().unwrap_or(Frac::ZERO);
        if gap_start < s {
            out.push(rest_of(ids, gap_start, s - gap_start));
            from = gap_start;
        }
    }
    let focus = first_focus(&ev);
    out.push(ev);
    sort_events(&mut out);
    EditResult { events: out, focus: Some(focus), from, to: t, caret: t }
}

/// Adds `pitch` to the note or chord starting exactly at `pos`.
pub fn add_to_chord(list: &[Event], pos: Frac, pitch: Pitch, ids: &mut Ids) -> Option<EditResult> {
    let mut out = list.to_vec();
    let e = out.iter_mut().find(|e| e.start == pos && !e.heads.is_empty())?;
    if e.heads.iter().any(|h| diatonic(h.pitch) == diatonic(pitch) && h.pitch.alter == pitch.alter) {
        return None;
    }
    let id = ids.next();
    e.heads.push(Head { id, pitch, tie: false, velocity: 0 });
    e.heads.sort_by_key(|h| h.pitch.midi_key());
    e.kind = EventKind::Chord;
    let (from, to) = (e.start, e.end());
    Some(EditResult { events: out, focus: Some(id), from, to, caret: to })
}

/// Adds a grace note before the event starting at `pos` (`None` if there
/// is none). `base`: 8 eighth, 16 sixteenth…; `slash`: acciaccatura.
pub fn add_grace(list: &[Event], pos: Frac, pitches: &[Pitch], base: u8, slash: bool, ids: &mut Ids) -> Option<EditResult> {
    let mut out = list.to_vec();
    let e = out.iter_mut().find(|e| e.start == pos)?;
    let id = ids.next();
    let heads: Vec<Head> = pitches.iter().map(|&p| Head { id: ids.next(), pitch: p, tie: false, velocity: 0 }).collect();
    let focus = heads.first().map(|h| h.id).unwrap_or(id);
    e.graces.push(Grace { id, heads, base: base.max(8), slash });
    let (from, to) = (e.start, e.end());
    Some(EditResult { events: out, focus: Some(focus), from, to, caret: pos })
}

/// The grace note (event index, grace index, head index) holding `element`.
pub fn find_grace(list: &[Event], element: Id) -> Option<(usize, usize, Option<usize>)> {
    list.iter().enumerate().find_map(|(ei, e)| {
        e.graces.iter().enumerate().find_map(|(gi, g)| {
            if g.id == element {
                Some((ei, gi, None))
            } else {
                g.heads.iter().position(|h| h.id == element).map(|hi| (ei, gi, Some(hi)))
            }
        })
    })
}

/// Applies `f` to a grace note's heads (one head if `element` is a head).
pub fn modify_grace(list: &[Event], element: Id, mut f: impl FnMut(&mut Head)) -> Option<EditResult> {
    let (ei, gi, hi) = find_grace(list, element)?;
    let mut out = list.to_vec();
    let g = &mut out[ei].graces[gi];
    match hi {
        Some(h) => f(&mut g.heads[h]),
        None => g.heads.iter_mut().for_each(f),
    }
    g.heads.sort_by_key(|h| h.pitch.midi_key());
    let (from, to) = (out[ei].start, out[ei].end());
    Some(EditResult { events: out, focus: Some(element), from, to, caret: from })
}

/// Deletes a head (from a chord) or a whole event. A single note or a
/// chord's last head becomes a rest when `fill`, otherwise a hole. A grace
/// note (or one head of a grace chord) goes alone.
pub fn delete_element(list: &[Event], element: Id, fill: bool, ids: &mut Ids) -> Option<EditResult> {
    if let Some((ei, gi, hi)) = find_grace(list, element) {
        let mut out = list.to_vec();
        let e = &mut out[ei];
        match hi {
            Some(h) if e.graces[gi].heads.len() > 1 => {
                e.graces[gi].heads.remove(h);
            }
            _ => {
                e.graces.remove(gi);
            }
        }
        let focus = e.heads.first().map(|h| h.id).unwrap_or(e.id);
        let (from, to) = (e.start, e.end());
        return Some(EditResult { events: out, focus: Some(focus), from, to, caret: from });
    }
    let mut out = Vec::with_capacity(list.len());
    let mut res: Option<EditResult> = None;
    for e in list {
        let is_event = e.id == element;
        let hit = e.heads.iter().position(|h| h.id == element);
        if !is_event && hit.is_none() {
            out.push(e.clone());
            continue;
        }
        let mut r = EditResult { events: vec![], focus: None, from: e.start, to: e.end(), caret: e.start };
        if let (false, Some(i), true) = (is_event, hit, e.heads.len() > 1) {
            let mut c = e.clone();
            c.heads.remove(i);
            c.kind = if c.heads.len() == 1 { EventKind::Note } else { EventKind::Chord };
            r.focus = Some(c.heads[0].id);
            out.push(c);
        } else if fill && e.kind != EventKind::Rest {
            let rest = rest_of(ids, e.start, e.duration);
            r.focus = Some(rest.id);
            out.push(rest);
        }
        res = Some(r);
    }
    let mut r = res?;
    r.events = out;
    Some(r)
}

/// Applies `f` to the event containing `element` (the event or one of its heads).
pub fn modify_event(list: &[Event], element: Id, f: impl FnOnce(&mut Event, Option<usize>)) -> Option<EditResult> {
    let mut out = list.to_vec();
    let e = out.iter_mut().find(|e| e.id == element || e.heads.iter().any(|h| h.id == element))?;
    let head = e.heads.iter().position(|h| h.id == element);
    f(e, head);
    let (from, to) = (e.start, e.end());
    Some(EditResult { events: out, focus: Some(element), from, to, caret: to })
}

/// The event containing `element` and, if it is a head, the head's index
/// (a grace note's element gives its event, without a head).
pub fn find_event(list: &[Event], element: Id) -> Option<(&Event, Option<usize>)> {
    list.iter()
        .find_map(|e| {
            if e.id == element {
                Some((e, None))
            } else {
                e.heads.iter().position(|h| h.id == element).map(|i| (e, Some(i)))
            }
        })
        .or_else(|| find_grace(list, element).map(|(ei, _, _)| (&list[ei], None)))
}

/// The event sounding at `pos`.
pub fn event_at(list: &[Event], pos: Frac) -> Option<&Event> {
    list.iter().find(|e| e.start <= pos && pos < e.end())
}

pub fn previous_pitch(list: &[Event], pos: Frac) -> Option<Pitch> {
    list.iter().filter(|e| e.start < pos && !e.heads.is_empty()).last().map(|e| e.heads[0].pitch)
}

pub fn voice_end(list: &[Event]) -> Frac {
    list.iter().map(|e| e.end()).max().unwrap_or(Frac::ZERO)
}

// -------------------------------------------------------------- ranges --
// [from, to) of one voice; events *starting* inside the range belong to it.

fn fill_gaps(out: &mut Vec<Event>, from: Frac, limit: Frac, ids: &mut Ids) {
    sort_events(out);
    let mut cur = from;
    let mut gaps = Vec::new();
    for e in out.iter() {
        if e.end() <= from {
            continue;
        }
        if e.start >= limit {
            break;
        }
        if e.start > cur {
            gaps.push(rest_of(ids, cur, e.start - cur));
        }
        cur = cur.max(e.end());
    }
    if cur < limit {
        gaps.push(rest_of(ids, cur, limit - cur));
    }
    out.extend(gaps);
    sort_events(out);
}

/// Removes the range (truncating events that cross `from`); voice 1 gets rests.
pub fn erase_range(list: &[Event], from: Frac, to: Frac, fill: bool, ids: &mut Ids) -> EditResult {
    let mut out = Vec::with_capacity(list.len());
    for e in list {
        let end = e.end();
        if e.start >= to || end <= from {
            out.push(e.clone());
            continue;
        }
        if e.start < from {
            let mut kept = e.clone();
            kept.duration = from - e.start;
            untie(&mut kept);
            out.push(kept);
        }
        if end > to && fill {
            out.push(rest_of(ids, to, end - to));
        }
    }
    sort_events(&mut out);
    if fill {
        let limit = to.min(voice_end(list));
        fill_gaps(&mut out, from, limit, ids);
    }
    EditResult { events: out, focus: None, from, to, caret: from }
}

/// The range shifted to start at 0 and cut at `to` (ids kept; paste re-ids).
pub fn slice_range(list: &[Event], from: Frac, to: Frac) -> Vec<Event> {
    list.iter()
        .filter(|e| e.start >= from && e.start < to)
        .map(|e| {
            let mut c = e.clone();
            c.start = e.start - from;
            if e.end() > to {
                c.duration = to - e.start;
                untie(&mut c);
            }
            c
        })
        .collect()
}

/// Overwrites `[at, at + length)` with `clip` (fresh ids): paste and repeat.
pub fn paste_events(list: &[Event], clip: &[Event], at: Frac, length: Frac, fill: bool, ids: &mut Ids) -> EditResult {
    let end = at + length;
    let erased = erase_range(list, at, end, fill, ids);
    let mut out: Vec<Event> = erased.events.into_iter().filter(|e| !(e.kind == EventKind::Rest && e.start >= at && e.start < end)).collect();
    let mut first = None;
    for c in clip {
        let mut e = c.clone();
        e.id = ids.next();
        e.start = at + c.start;
        for h in e.heads.iter_mut() {
            h.id = ids.next();
        }
        for g in e.graces.iter_mut() {
            g.id = ids.next();
            for h in g.heads.iter_mut() {
                h.id = ids.next();
            }
        }
        first.get_or_insert(first_focus(&e));
        out.push(e);
    }
    if fill {
        fill_gaps(&mut out, at, end, ids);
    }
    sort_events(&mut out);
    EditResult { events: out, focus: first, from: at, to: end, caret: end }
}

/// Applies `f` to every event starting in the range; `None` if there is none.
pub fn apply_to_range(list: &[Event], from: Frac, to: Frac, mut f: impl FnMut(&mut Event)) -> Option<EditResult> {
    let mut out = list.to_vec();
    let mut first = None;
    for e in out.iter_mut().filter(|e| e.start >= from && e.start < to) {
        f(e);
        first.get_or_insert(first_focus(e));
    }
    first.map(|f| EditResult { events: out, focus: Some(f), from, to, caret: to })
}

/// Rhythm edit: every event starting in `[from, to)` takes the duration
/// `f(old)`, the events staying in order (gaps between them kept). Longer
/// music overwrites what follows (a note it cuts into leaves its tail as a
/// rest in voice 1); shorter music leaves rests (voice 1, `fill`) or a hole.
/// A changed note's ties are dropped. `None` when nothing changes.
pub fn set_durations(list: &[Event], from: Frac, to: Frac, f: impl Fn(Frac) -> Frac, fill: bool, ids: &mut Ids) -> Option<EditResult> {
    let sel: Vec<&Event> = list.iter().filter(|e| e.start >= from && e.start < to).collect();
    let first = *sel.first()?;
    let old_end = sel.iter().map(|e| e.end()).max()?;
    let mut new = Vec::with_capacity(sel.len());
    let (mut at, mut prev_end, mut changed) = (first.start, first.start, false);
    for e in sel {
        let d = f(e.duration);
        if d <= Frac::ZERO {
            return None;
        }
        at = at + (e.start - prev_end).max(Frac::ZERO);
        prev_end = e.end();
        let mut c = e.clone();
        if d != e.duration {
            untie(&mut c);
        }
        changed |= d != e.duration || at != e.start;
        c.start = at;
        c.duration = d;
        at = at + d;
        new.push(c);
    }
    if !changed {
        return None;
    }
    let start = first.start;
    let end = at.max(old_end);
    let mut out = Vec::with_capacity(list.len() + 2);
    clear_range(&mut out, list, start, at.max(old_end), fill, ids);
    if fill && at < old_end {
        out.push(rest_of(ids, at, old_end - at));
    }
    let focus = first_focus(&new[0]);
    out.extend(new);
    sort_events(&mut out);
    Some(EditResult { events: out, focus: Some(focus), from: start, to: end, caret: at })
}

/// Joins rests that follow each other inside `[lo, hi]` into one, never
/// across a bar start in `bars` (the engraver writes a long rest as the
/// meter wants it).
pub fn merge_rests(v: &mut Vec<Event>, lo: Frac, hi: Frac, bars: &[Frac]) {
    sort_events(v);
    let mut out: Vec<Event> = Vec::with_capacity(v.len());
    for e in v.drain(..) {
        if let Some(last) = out.last_mut() {
            if last.is_rest() && e.is_rest() && last.end() == e.start && e.start >= lo && e.start <= hi && bars.binary_search(&e.start).is_err() {
                last.duration = last.duration + e.duration;
                continue;
            }
        }
        out.push(e);
    }
    *v = out;
}

/// Moves every event starting in `[from, to)` by `by` (earlier when
/// negative). The music overwrites what it lands on; voice 1 (`fill`) gets
/// rests where it was. Ties into and out of the moved music are dropped.
/// `None` if nothing moves or it would start before 0.
pub fn move_events(list: &[Event], from: Frac, to: Frac, by: Frac, fill: bool, ids: &mut Ids) -> Option<EditResult> {
    let moved: Vec<Event> = list.iter().filter(|e| e.start >= from && e.start < to).cloned().collect();
    let first = moved.first()?;
    if by == Frac::ZERO || first.start + by < Frac::ZERO {
        return None;
    }
    let (s0, s1) = (first.start, moved.iter().map(|e| e.end()).max()?);
    // Lift the music out (rests in voice 1); untie the note leading in.
    let mut rest: Vec<Event> = Vec::with_capacity(list.len() + 4);
    for e in list {
        if e.start >= from && e.start < to {
            if fill {
                rest.push(rest_of(ids, e.start, e.duration));
            }
        } else {
            let mut c = e.clone();
            if c.end() == s0 {
                untie(&mut c);
            }
            rest.push(c);
        }
    }
    let (t0, t1) = (s0 + by, s1 + by);
    let mut out = Vec::with_capacity(rest.len() + 4);
    clear_range(&mut out, &rest, t0, t1, fill, ids);
    let focus = first_focus(first);
    for mut e in moved {
        e.start = e.start + by;
        if e.end() == t1 {
            untie(&mut e);
        }
        out.push(e);
    }
    let (lo, hi) = (s0.min(t0), s1.max(t1));
    if fill {
        let limit = voice_end(&out).max(hi);
        fill_gaps(&mut out, Frac::ZERO, limit, ids);
    }
    sort_events(&mut out);
    Some(EditResult { events: out, focus: Some(focus), from: lo, to: hi, caret: t1 })
}

/// The longest undotted note value (a power of two) no longer than `d`.
pub fn plain_value(d: Frac) -> Frac {
    let mut v = Frac::int(2);
    while v > d && v > Frac::new(1, 256) {
        v = v * Frac::new(1, 2);
    }
    v
}

fn is_plain(d: Frac) -> bool {
    let pow2 = |x: i64| x > 0 && x & (x - 1) == 0;
    pow2(d.num) && pow2(d.den)
}

/// The next duration in the cycle plain → dotted → double-dotted → plain.
pub fn cycle_dots(d: Frac) -> Frac {
    if is_plain(d) {
        d * Frac::new(3, 2)
    } else if is_plain(d * Frac::new(2, 3)) {
        d * Frac::new(7, 6)
    } else if is_plain(d * Frac::new(4, 7)) {
        d * Frac::new(4, 7)
    } else {
        d
    }
}

/// Diatonic transposition of every head in the range (accidentals kept).
/// The passage moved by an interval, spelt by it (grace notes too).
pub fn transpose_by(list: &[Event], from: Frac, to: Frac, by: corinzia_model::Interval) -> Option<EditResult> {
    apply_to_range(list, from, to, |e| transpose_event(e, by))
}

/// One event moved by an interval: its heads and grace notes.
pub fn transpose_event(e: &mut Event, by: corinzia_model::Interval) {
    for h in e.heads.iter_mut() {
        h.pitch = h.pitch.transposed(by);
    }
    e.heads.sort_by_key(|h| h.pitch.midi_key());
    for g in e.graces.iter_mut() {
        for h in g.heads.iter_mut() {
            h.pitch = h.pitch.transposed(by);
        }
    }
}

pub fn transpose_range(list: &[Event], from: Frac, to: Frac, steps: i32) -> Option<EditResult> {
    apply_to_range(list, from, to, |e| {
        for h in e.heads.iter_mut() {
            h.pitch = from_diatonic(diatonic(h.pitch) + steps, h.pitch.alter as i32);
        }
        e.heads.sort_by_key(|h| h.pitch.midi_key());
    })
}

/// Marks a spanner from the first to the last *note* starting in the range
/// (two different notes needed), replacing any spanner of the same family.
pub fn mark_span(list: &[Event], from: Frac, to: Frac, start_mark: u8, end_mark: u8) -> Option<EditResult> {
    let hairpin = span::CRESC_START | span::DIM_START;
    let family = start_mark | end_mark | if start_mark & hairpin != 0 { hairpin } else { 0 };
    let mut out = list.to_vec();
    let notes: Vec<usize> = (0..out.len()).filter(|&i| out[i].start >= from && out[i].start < to && !out[i].heads.is_empty()).collect();
    let (&first, &last) = (notes.first()?, notes.last()?);
    if first == last {
        return None;
    }
    let (s, t) = (out[first].start, out[last].start);
    for e in out.iter_mut().filter(|e| e.start >= s && e.start <= t) {
        e.spans &= !family;
    }
    out[first].spans |= start_mark;
    out[last].spans |= end_mark;
    let focus = out[first].heads[0].id;
    let end = out[last].end();
    Some(EditResult { events: out, focus: Some(focus), from: s, to: end, caret: end })
}

// ------------------------------------------------------------------ lyrics --
/// Sets the syllable of a verse on the note containing `element` (an empty
/// text removes it). `hyphen`: the word continues on the next note.
pub fn set_lyric(list: &[Event], element: Id, verse: u8, text: &str, hyphen: bool, extend: bool) -> Option<EditResult> {
    if find_event(list, element).is_some_and(|(e, _)| e.is_rest()) {
        return None;
    }
    modify_event(list, element, |e, _| {
        e.lyrics.retain(|l| l.verse != verse);
        if !text.is_empty() {
            e.lyrics.push(corinzia_model::Lyric { verse, text: text.to_string(), hyphen, extend });
            e.lyrics.sort_by_key(|l| l.verse);
        }
    })
}

/// The next note (not a rest, not a tie continuation) after `element`'s event.
pub fn next_note(list: &[Event], element: Id) -> Option<&Event> {
    let (e, _) = find_event(list, element)?;
    let tied_in = |n: &Event, prev: Option<&Event>| prev.is_some_and(|p| p.end() == n.start && p.heads.iter().any(|h| h.tie));
    let mut prev = Some(e);
    for n in list.iter().filter(|n| n.start > e.start) {
        if !n.is_rest() && !tied_in(n, prev) {
            return Some(n);
        }
        prev = Some(n);
    }
    None
}
