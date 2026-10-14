//! Score structure and text: popover parsing and edits on a movement's maps
//! (key, time, tempo), barlines, endings, rehearsal marks, per-staff clefs and
//! text, plus inserting and deleting bars.

use crate::Ids;
use corinzia_model::*;
use corinzia_score::score::art;
use corinzia_score::Frac;

// ---------------------------------------------------------------- parsing --
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeySig {
    pub fifths: i32,
    pub minor: bool,
}

fn ascii_accidentals(s: &str) -> String {
    s.replace('♭', "b").replace('♯', "#").replace('♮', "")
}

/// "C", "Eb", "F#", "Bbm", "f#" (lower case = minor), "a minor", "D major", "E♭".
pub fn parse_key(text: &str) -> Option<KeySig> {
    let text = ascii_accidentals(text.trim());
    let first = text.chars().next()?;
    let mut minor = first.is_lowercase();
    let mut t = text.to_lowercase();
    for suffix in [" minor", "minor", " min", "min", "m"] {
        if t.len() > suffix.len() && t.ends_with(suffix) {
            t.truncate(t.len() - suffix.len());
            minor = true;
            break;
        }
    }
    for suffix in [" major", "major", " maj", "maj"] {
        if t.len() > suffix.len() && t.ends_with(suffix) {
            t.truncate(t.len() - suffix.len());
            minor = false;
            break;
        }
    }
    const MAJOR: [(&str, i32); 15] = [
        ("cb", -7),
        ("gb", -6),
        ("db", -5),
        ("ab", -4),
        ("eb", -3),
        ("bb", -2),
        ("f", -1),
        ("c", 0),
        ("g", 1),
        ("d", 2),
        ("a", 3),
        ("e", 4),
        ("b", 5),
        ("f#", 6),
        ("c#", 7),
    ];
    let t = t.trim();
    let major = MAJOR.iter().find(|(n, _)| *n == t)?.1;
    // A minor key has the signature of its relative major (three fifths lower).
    let fifths = if minor { major - 3 } else { major };
    (-7..=7).contains(&fifths).then_some(KeySig { fifths, minor })
}

/// "3/4", "6/8", "c" (= 4/4), "C|" or "cut" (= 2/2).
pub fn parse_time(text: &str) -> Option<(i64, i64)> {
    let t = text.trim().to_lowercase();
    match t.as_str() {
        "c" | "common" => return Some((4, 4)),
        "c|" | "cut" | "alla breve" => return Some((2, 2)),
        _ => {}
    }
    let (n, d) = t.split_once('/')?;
    let n: i64 = n.trim().parse().ok()?;
    let d: i64 = d.trim().parse().ok()?;
    let pow2 = d > 0 && d & (d - 1) == 0 && d <= 64;
    ((1..=32).contains(&n) && pow2).then_some((n, d))
}

/// Clef names and letters -> the engraver's clef name.
pub fn parse_clef(text: &str) -> Option<&'static str> {
    Some(match text.trim().to_lowercase().as_str() {
        "treble" | "g" | "violin" => "treble",
        "bass" | "f" => "bass",
        "alto" | "c" | "viola" => "alto",
        "tenor" => "tenor",
        "treble_8" | "g8" => "treble_8",
        "bass_8" | "f8" => "bass_8",
        "percussion" | "perc" => "percussion",
        _ => return None,
    })
}

/// "||", "|.", "|:", ":|", ":|:" or words (double, final, start/end repeat).
pub fn parse_barline(text: &str) -> Option<BarKind> {
    Some(match text.trim().to_lowercase().as_str() {
        "||" | "double" => BarKind::Double,
        "|." | "final" | "end" => BarKind::Final,
        "|:" | ".|:" | "start repeat" | "start-repeat" => BarKind::RepeatStart,
        ":|" | ":|." | "end repeat" | "end-repeat" | "repeat" => BarKind::RepeatEnd,
        ":|:" | ":|.|:" | "double repeat" => BarKind::RepeatBoth,
        _ => return None,
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct TempoSpec {
    pub text: String,
    pub bpm: f64,
}

/// "Allegro 132", "q=96", "♩=96", "96", "Adagio" (a word alone: its usual tempo).
pub fn parse_tempo(text: &str) -> Option<TempoSpec> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let (mut words, number) = if let Some((w, n)) = text.split_once('=') {
        let w = w.trim();
        let w = if matches!(w, "q" | "Q" | "♩" | "crotchet" | "quarter") { "" } else { w };
        (w.to_string(), Some(n.trim()))
    } else {
        let p = text.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.').len();
        if p < text.len() {
            (text[..p].trim().to_string(), Some(&text[p..]))
        } else {
            (text.to_string(), None)
        }
    };
    let bpm = match number {
        Some(n) => {
            let b: f64 = n.parse().ok()?;
            if !(10.0..=400.0).contains(&b) {
                return None;
            }
            b
        }
        None => match words.to_lowercase().as_str() {
            "grave" => 40.0,
            "largo" => 50.0,
            "lento" => 56.0,
            "adagio" => 70.0,
            "andante" => 90.0,
            "andantino" => 96.0,
            "moderato" => 110.0,
            "allegretto" => 116.0,
            "allegro" => 132.0,
            "vivace" => 156.0,
            "presto" => 168.0,
            "prestissimo" => 200.0,
            _ => return None,
        },
    };
    words = words.trim().to_string();
    Some(TempoSpec { text: words, bpm })
}

/// Ending (volta) numbers: "1", "2.", "1,2", "1-3", "1.2." -> pass bitmask.
pub fn parse_ending(text: &str) -> Option<u8> {
    let text = text.trim();
    let mut mask = 0u32;
    let mut prev = 0u32;
    let mut range = false;
    let chars: Vec<char> = text.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if let Some(n) = c.to_digit(10) {
            if !(1..=8).contains(&n) || chars.get(i + 1).is_some_and(|c| c.is_ascii_digit()) {
                return None;
            }
            if range {
                if n < prev {
                    return None;
                }
                for k in prev..=n {
                    mask |= 1 << (k - 1);
                }
                range = false;
            }
            mask |= 1 << (n - 1);
            prev = n;
        } else if (c == '-' || c == '–') && prev != 0 {
            range = true;
        } else if !matches!(c, '.' | ',' | ' ') {
            return None;
        }
    }
    (!range && mask != 0).then_some(mask as u8)
}

/// "1." / "1., 2." / "1.–3." as engraved.
pub fn ending_label(passes: u8) -> String {
    let on = |n: u32| passes & (1 << (n - 1)) != 0;
    let mut out = String::new();
    let mut n = 1;
    while n <= 8 {
        if !on(n) {
            n += 1;
            continue;
        }
        let mut last = n;
        while last < 8 && on(last + 1) {
            last += 1;
        }
        if !out.is_empty() {
            out += ", ";
        }
        out += &format!("{n}.");
        if last - n >= 2 {
            out += &format!("–{last}.");
        } else if last > n {
            out += &format!(", {last}.");
        }
        n = last + 1;
    }
    out
}

/// Ornament / articulation popover text -> articulation bit.
pub fn parse_ornament(text: &str) -> Option<u16> {
    Some(match text.trim().to_lowercase().as_str() {
        "tr" | "trill" => art::TRILL,
        "mordent" | "mord" | "m" => art::MORDENT,
        "turn" | "~" => art::TURN,
        "trem" | "tremolo" | "///" => art::TREMOLO,
        "arp" | "arpeggio" | "roll" => art::ARPEGGIO,
        "breath" | "," => art::BREATH,
        "caesura" | "//" => art::CAESURA,
        "staccatissimo" | "!" => art::STACCATISSIMO,
        "staccato" | "." => art::STACCATO,
        "accent" | ">" => art::ACCENT,
        "tenuto" | "-" => art::TENUTO,
        "marcato" | "^" => art::MARCATO,
        "fermata" | "ferm" => art::FERMATA,
        _ => return None,
    })
}

/// A chord symbol as typed ("C", "f#m7", "Bbmaj7", "Eb7(#9)/G", "Do",
/// "C°7", "Cø") -> its engraved spelling ("F♯m7", "B♭maj7", "E♭7(♯9)/G").
/// The root (and bass) letter is A–G with ♯/♭ (#, b); the rest is a
/// quality and extensions from the usual vocabulary.
pub fn parse_chord(text: &str) -> Option<String> {
    let t = text.trim();
    let (main, bass) = match t.rsplit_once('/') {
        Some((m, b)) if !b.is_empty() && !b.chars().all(|c| c.is_ascii_digit()) => (m, Some(b)), // "6/9" is not a bass
        _ => (t, None),
    };
    fn root(s: &str) -> Option<(String, &str)> {
        let mut chars = s.chars();
        let letter = chars.next()?.to_ascii_uppercase();
        if !('A'..='G').contains(&letter) {
            return None;
        }
        let rest = chars.as_str();
        let (acc, rest) = match rest.chars().next() {
            Some('#') | Some('♯') => ("♯", &rest[rest.chars().next().unwrap().len_utf8()..]),
            Some('b') | Some('♭') if !rest.starts_with("b5") || rest.len() == 1 => ("♭", &rest[rest.chars().next().unwrap().len_utf8()..]),
            _ => ("", rest),
        };
        Some((format!("{letter}{acc}"), rest))
    }
    let (r, quality) = root(main)?;
    // Quality and extensions: a known vocabulary, accidentals spelled.
    let q = quality.replace('#', "♯").replace("dim", "°").replace("o", "°");
    let q = {
        // b before a digit is a flat: "7b9" -> "7♭9".
        let mut out = String::new();
        let cs: Vec<char> = q.chars().collect();
        for (i, &c) in cs.iter().enumerate() {
            if c == 'b' && cs.get(i + 1).is_some_and(|n| n.is_ascii_digit()) {
                out.push('♭');
            } else {
                out.push(c);
            }
        }
        out
    };
    const WORDS: [&str; 12] = ["maj", "min", "m", "M", "aug", "sus", "add", "alt", "omit", "no", "Δ", "+"];
    let mut rest = q.as_str();
    while !rest.is_empty() {
        if let Some(w) = WORDS.iter().find(|w| rest.starts_with(**w)) {
            rest = &rest[w.len()..];
            continue;
        }
        let c = rest.chars().next().unwrap();
        if c.is_ascii_digit() || "♯♭°ø()-+,/ ".contains(c) {
            rest = &rest[c.len_utf8()..];
            continue;
        }
        return None;
    }
    let mut out = format!("{r}{q}");
    if let Some(b) = bass {
        let (br, tail) = root(b)?;
        if !tail.is_empty() {
            return None;
        }
        out = format!("{out}/{br}");
    }
    Some(out)
}

// ------------------------------------------------------------ positions --
pub fn bar_length(m: &Movement, pos: Frac) -> Frac {
    m.meter.iter().filter(|x| x.at <= pos).last().map(|x| Frac::new(x.num, x.den)).unwrap_or(Frac::int(1))
}

/// Start of the bar containing `pos`.
pub fn bar_start_at_or_before(m: &Movement, pos: Frac) -> Frac {
    let start = m.meter.iter().filter(|x| x.at <= pos).last().map(|x| x.at).unwrap_or(Frac::ZERO);
    let len = bar_length(m, pos);
    let rel = pos - start;
    let bars = (rel.num * len.den) / (rel.den * len.num);
    start + len * Frac::int(bars)
}

/// The barline nearest `pos` (bar starts and the final barline).
pub fn nearest_barline(m: &Movement, pos: Frac) -> Frac {
    let a = bar_start_at_or_before(m, pos);
    let b = (a + bar_length(m, pos)).min(m.length);
    if pos - a <= b - pos {
        a
    } else {
        b
    }
}

/// Bar starts in `[0, length)`.
pub fn bar_starts(m: &Movement) -> Vec<Frac> {
    let mut out = Vec::new();
    let mut at = Frac::ZERO;
    while at < m.length {
        out.push(at);
        at = at + bar_length(m, at);
    }
    out
}

/// 1-based bar number and the position inside the bar.
pub fn bar_number(m: &Movement, pos: Frac) -> (usize, Frac) {
    let starts = bar_starts(m);
    let i = starts.iter().rposition(|&s| s <= pos).unwrap_or(0);
    let s = starts.get(i).copied().unwrap_or(Frac::ZERO);
    // A pickup is bar 0.
    let first = if pickup(m).is_some() { 0 } else { 1 };
    if pos >= m.length && !starts.is_empty() {
        // Past the end: count on in the last bar's meter.
        let len = bar_length(m, pos);
        let extra = (pos - m.length) * Frac::new(len.den, len.num);
        let n = starts.len() + (extra.num / extra.den) as usize - pickup(m).is_some() as usize;
        return (n + 1, pos - (m.length + len * Frac::int(extra.num / extra.den)));
    }
    (i + first, pos - s)
}

/// `end` rounded up to whole bars (at least `m.length`).
pub fn round_up_to_bar(m: &Movement, end: Frac) -> Frac {
    if end <= m.length {
        return m.length;
    }
    let s = bar_start_at_or_before(m, end);
    if s == end {
        s
    } else {
        s + bar_length(m, end)
    }
}

// --------------------------------------------------------- header edits --
fn replace_at<T>(v: &mut Vec<T>, at: Frac, item: T, at_of: impl Fn(&T) -> Frac) {
    v.retain(|x| at_of(x) != at);
    v.push(item);
    v.sort_by(|a, b| at_of(a).cmp(&at_of(b)));
}

pub fn set_key(m: &mut Movement, at: Frac, key: KeySig) {
    replace_at(&mut m.keys, at, Key { at, fifths: key.fifths, minor: key.minor }, |k| k.at);
}
/// A meter at the start of a pickup bar applies from the bar after it.
pub fn set_meter(m: &mut Movement, at: Frac, num: i64, den: i64) {
    let at = match m.meter.iter().find(|x| x.at == at && x.pickup) {
        Some(p) => at + Frac::new(p.num, p.den),
        None => at,
    };
    replace_at(&mut m.meter, at, Meter { at, num, den, pickup: false }, |k| k.at);
}

/// The pickup (upbeat) bar's length, if the movement starts with one.
pub fn pickup(m: &Movement) -> Option<Frac> {
    m.meter.first().filter(|x| x.at == Frac::ZERO && x.pickup).map(|x| Frac::new(x.num, x.den))
}

/// Sets, changes or removes (`None`) the pickup bar. A new pickup is an
/// empty bar of `len` before the music; removing one deletes its music.
pub fn set_pickup(m: &mut Movement, len: Option<Frac>) {
    if let Some(old) = pickup(m) {
        m.meter.remove(0);
        // The main meter moves back to 0 with everything else.
        delete_span(m, Frac::ZERO, old);
        for l in m.lanes.iter_mut() {
            for v in l.voices.iter_mut() {
                v.events = delete_span_events(&v.events, Frac::ZERO, old).into();
                v.tuplets = crate::tuplets::cut(&v.tuplets, Frac::ZERO, old);
            }
        }
    }
    let Some(len) = len.filter(|l| *l > Frac::ZERO) else { return };
    insert_span(m, Frac::ZERO, len);
    for l in m.lanes.iter_mut() {
        for v in l.voices.iter_mut() {
            v.events = insert_span_events(&v.events, Frac::ZERO, len).into();
            v.tuplets = crate::tuplets::shift(&v.tuplets, Frac::ZERO, len);
        }
    }
    // Entries at 0 (the meter, key, tempo, instruments) stay at 0: the
    // pickup is in the movement's opening meter, key and tempo. The meter
    // itself moves to the first full bar.
    for x in m.meter.iter_mut().filter(|x| x.at == Frac::ZERO) {
        x.at = len;
    }
    if m.meter.iter().all(|x| x.at != len) {
        m.meter.push(Meter { at: len, num: 4, den: 4, pickup: false });
    }
    m.meter.insert(0, Meter { at: Frac::ZERO, num: len.num, den: len.den, pickup: true });
    m.meter.sort_by(|a, b| a.at.cmp(&b.at));
}

/// Pickup lengths typed into the time popover: "pickup 1/4" (a note value
/// or fraction of a whole), "pickup 2" (beats of `beat`), "no pickup".
pub fn parse_pickup(text: &str, beat: Frac) -> Option<Option<Frac>> {
    let t = text.trim().to_lowercase();
    if matches!(t.as_str(), "no pickup" | "pickup 0" | "nopickup" | "no upbeat") {
        return Some(None);
    }
    let rest = t.strip_prefix("pickup").or_else(|| t.strip_prefix("upbeat")).or_else(|| t.strip_prefix("anacrusis"))?.trim();
    let len = match rest.split_once('/') {
        Some((n, d)) => Frac::new(n.trim().parse().ok()?, d.trim().parse().ok()?),
        None => beat * Frac::int(rest.parse().ok()?),
    };
    (len > Frac::ZERO).then_some(Some(len))
}
pub fn set_tempo(m: &mut Movement, at: Frac, t: &TempoSpec) {
    replace_at(&mut m.tempo, at, Tempo { at, bpm: t.bpm, ramp: false, text: t.text.clone() }, |k| k.at);
}
pub fn set_barline(m: &mut Movement, at: Frac, kind: BarKind, ids: &mut Ids) {
    replace_at(&mut m.barlines, at, Barline { id: ids.next(), at, kind }, |k| k.at);
}
/// Replaces any ending overlapping `[from, to)`.
pub fn set_ending(m: &mut Movement, from: Frac, to: Frac, passes: u8, ids: &mut Ids) {
    m.endings.retain(|e| !(e.from < to && from < e.to));
    m.endings.push(Ending { id: ids.next(), from, to, passes });
    m.endings.sort_by(|a, b| a.from.cmp(&b.from));
}
pub fn toggle_rehearsal_mark(m: &mut Movement, at: Frac, ids: &mut Ids) {
    let n = m.rehearsal.len();
    m.rehearsal.retain(|r| r.at != at);
    if m.rehearsal.len() == n {
        replace_at(&mut m.rehearsal, at, RehearsalMark { id: ids.next(), at }, |r| r.at);
    }
}
pub fn set_clef(m: &mut Movement, player: Id, at: Frac, clef: &str, ids: &mut Ids) {
    for l in m.lanes.iter_mut().filter(|l| l.player == player) {
        replace_at(&mut l.clefs, at, ClefChange { id: ids.next(), at, clef: clef.to_string() }, |c| c.at);
    }
}
/// Sets (replaces) the chord symbol of a staff at a position.
pub fn set_chord(m: &mut Movement, player: Id, at: Frac, text: &str, ids: &mut Ids) {
    for l in m.lanes.iter_mut().filter(|l| l.player == player) {
        replace_at(&mut l.chords, at, ChordSymbol { id: ids.next(), at, text: text.to_string() }, |c| c.at);
    }
}

pub fn add_text(m: &mut Movement, player: Id, at: Frac, text: &str, kind: TextKind, ids: &mut Ids) {
    for l in m.lanes.iter_mut().filter(|l| l.player == player) {
        l.texts.push(TextItem { id: ids.next(), at, text: text.to_string(), kind });
        l.texts.sort_by(|a, b| a.at.cmp(&b.at));
    }
}

/// Describes the barline, mark, ending, clef change or text with this id.
pub fn describe_item(m: &Movement, id: Id) -> Option<String> {
    const BAR: [&str; 5] = ["Double barline", "Final barline", "Start repeat", "End repeat", "End/start repeat"];
    if let Some(b) = m.barlines.iter().find(|b| b.id == id) {
        return Some(BAR[b.kind as usize].into());
    }
    if m.rehearsal.iter().any(|r| r.id == id) {
        return Some("Rehearsal mark".into());
    }
    if let Some(e) = m.endings.iter().find(|e| e.id == id) {
        return Some(format!("Ending {}", ending_label(e.passes)));
    }
    for l in &m.lanes {
        if let Some(c) = l.clefs.iter().find(|c| c.id == id) {
            return Some(format!("Clef {}", c.clef));
        }
        if let Some(c) = l.chords.iter().find(|c| c.id == id) {
            return Some(format!("Chord symbol {}", c.text));
        }
        if let Some(t) = l.texts.iter().find(|t| t.id == id) {
            let k = if t.kind == TextKind::Expression { "Expression" } else { "Technique" };
            return Some(format!("{k} \"{}\"", t.text));
        }
    }
    None
}

/// Removes the item with this id; returns its description.
pub fn remove_item(m: &mut Movement, id: Id) -> Option<String> {
    let what = describe_item(m, id)?;
    m.barlines.retain(|x| x.id != id);
    m.rehearsal.retain(|x| x.id != id);
    m.endings.retain(|x| x.id != id);
    for l in m.lanes.iter_mut() {
        l.clefs.retain(|x| x.id != id);
        l.texts.retain(|x| x.id != id);
        l.chords.retain(|x| x.id != id);
    }
    Some(what)
}

// ------------------------------------------------------ insert / delete --
/// Voice content: events at/after `at` move right by `length`.
pub fn insert_span_events(list: &[Event], at: Frac, length: Frac) -> Vec<Event> {
    list.iter()
        .map(|e| {
            let mut e = e.clone();
            if e.start >= at {
                e.start = e.start + length;
            }
            e
        })
        .collect()
}

/// Removes `[from, to)` (truncating an event that crosses `from`) and closes the gap.
pub fn delete_span_events(list: &[Event], from: Frac, to: Frac) -> Vec<Event> {
    let gap = to - from;
    list.iter()
        .filter_map(|e| {
            let mut m = e.clone();
            if e.start >= to {
                m.start = e.start - gap;
            } else if e.start < from {
                if e.end() > from {
                    m.duration = from - e.start;
                    m.heads.iter_mut().for_each(|h| h.tie = false);
                }
            } else {
                return None;
            }
            Some(m)
        })
        .collect()
}

trait At {
    fn at(&self) -> Frac;
    fn at_mut(&mut self) -> &mut Frac;
}
macro_rules! impl_at {
    ($($t:ty),*) => {$(impl At for $t {
        fn at(&self) -> Frac { self.at }
        fn at_mut(&mut self) -> &mut Frac { &mut self.at }
    })*};
}
impl_at!(Tempo, Meter, Key, InstrumentChange, ClefChange, TextItem, ChordSymbol, RehearsalMark, Barline);

/// Maps (tempo, meter, key, instrument changes) keep their entry at 0 and
/// hold one entry per position; marks and text are plain items.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Map,
    Items,
}

fn shift_items<T: At>(v: &mut [T], at: Frac, delta: Frac, kind: Kind) {
    for x in v.iter_mut() {
        let a = x.at_mut();
        if *a >= at && (kind == Kind::Items || *a > Frac::ZERO) {
            *a = *a + delta;
        }
    }
}

fn delete_items<T: At>(v: &mut Vec<T>, from: Frac, to: Frac, kind: Kind) {
    v.retain(|x| !(x.at() >= from && x.at() < to && (kind == Kind::Items || x.at() > Frac::ZERO)));
    for x in v.iter_mut() {
        let a = x.at_mut();
        if *a >= to {
            *a = *a - (to - from);
        }
    }
    v.sort_by_key(|x| x.at());
    if kind == Kind::Items {
        return;
    }
    // An entry shifted onto `from` supersedes the one kept there.
    let mut out: Vec<T> = Vec::with_capacity(v.len());
    for x in v.drain(..) {
        if let Some(last) = out.last_mut() {
            if last.at() == x.at() && x.at() == from {
                *last = x;
                continue;
            }
        }
        out.push(x);
    }
    *v = out;
}

/// Header: items at/after `at` move and the movement grows by `length`. Map
/// entries at 0 stay; a barline at `at` ends the previous bar and stays
/// (unless it is a start repeat).
pub fn insert_span(m: &mut Movement, at: Frac, length: Frac) {
    shift_items(&mut m.tempo, at, length, Kind::Map);
    shift_items(&mut m.meter, at, length, Kind::Map);
    shift_items(&mut m.keys, at, length, Kind::Map);
    for b in m.barlines.iter_mut() {
        if b.at > at || (b.at == at && b.kind == BarKind::RepeatStart) {
            b.at = b.at + length;
        }
    }
    shift_items(&mut m.rehearsal, at, length, Kind::Items);
    for e in m.endings.iter_mut() {
        if e.from >= at {
            e.from = e.from + length;
        }
        if e.to > at {
            e.to = e.to + length;
        }
    }
    for l in m.lanes.iter_mut() {
        shift_items(&mut l.changes, at, length, Kind::Map);
        shift_items(&mut l.clefs, at, length, Kind::Items);
        shift_items(&mut l.texts, at, length, Kind::Items);
        shift_items(&mut l.chords, at, length, Kind::Items);
    }
    m.length = m.length + length;
}

/// Header: items inside `[from, to)` go (map entries at 0 stay; one shifted
/// onto `from` replaces the one there), later ones move left. Barlines
/// strictly inside go; the one at `to` replaces the one at `from`.
pub fn delete_span(m: &mut Movement, from: Frac, to: Frac) {
    let gap = to - from;
    delete_items(&mut m.tempo, from, to, Kind::Map);
    delete_items(&mut m.meter, from, to, Kind::Map);
    delete_items(&mut m.keys, from, to, Kind::Map);
    m.barlines.retain(|b| !(b.at > from && b.at < to));
    for b in m.barlines.iter_mut() {
        if b.at >= to {
            b.at = b.at - gap;
        }
    }
    m.barlines.sort_by(|a, b| a.at.cmp(&b.at));
    if let Some(i) = (0..m.barlines.len().saturating_sub(1)).find(|&i| m.barlines[i].at == from && m.barlines[i + 1].at == from) {
        m.barlines.remove(i); // the moved one comes second
    }
    delete_items(&mut m.rehearsal, from, to, Kind::Items);
    let cut = |x: Frac| {
        if x <= from {
            x
        } else if x >= to {
            x - gap
        } else {
            from
        }
    };
    for e in m.endings.iter_mut() {
        e.from = cut(e.from);
        e.to = cut(e.to);
    }
    m.endings.retain(|e| e.from < e.to);
    for l in m.lanes.iter_mut() {
        delete_items(&mut l.changes, from, to, Kind::Map);
        delete_items(&mut l.clefs, from, to, Kind::Items);
        delete_items(&mut l.texts, from, to, Kind::Items);
        delete_items(&mut l.chords, from, to, Kind::Items);
    }
    m.length = m.length - gap;
}
