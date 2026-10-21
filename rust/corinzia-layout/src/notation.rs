//! A tiny LilyPond-like notation for building scores in tests and previews:
//! `c'4 d'8 e' <e' gis'>2~ q4 r8. fes''16-. \f`.
//!
//! * pitches: `c d e f g a b`, `is`/`es` (repeatable) for ♯/♭, `'`/`,` octave
//!   marks from C3 (`c'` = middle C);
//! * durations `1 2 4 8 16 32 64` with dots, sticky as in LilyPond; `*2/3` for triplets;
//! * `<…>` chords, `q` repeats the last chord, `r` rests, `~` ties;
//! * `-.` `->` `--` `-!` `-^` articulations, `\fermata \trill \mordent \turn
//!   \arpeggio \breathe`, dynamics `\pp` … `\ff`;
//! * `(` `)` slurs, `\cr` `\decr` `\!` hairpins (LilyPond's spellings);
//! * `\ly=Glo-` lyrics (`\ly2=` for verse 2; `-` hyphen, `_` extender);
//! * fingering `-1` … `-5`, bowings `\upbow` `\downbow`;
//! * `\tuplet 5/4 { … }` tuplets (nestable; durations inside are written);
//! * `\grace c''16`, `\acciaccatura d''8`, `\grace { c''16 d'' }`: grace
//!   notes before the next note.

use corinzia_score::score::{art, span, Event, Grace, Meter, Part, Pitch, Score, Slur, Tuplet, Voice};
use corinzia_score::Frac;

fn parse_pitch(tok: &str) -> Option<(Pitch, &str)> {
    let b = tok.as_bytes();
    let step = match b.first()? {
        b'c' => 0,
        b'd' => 1,
        b'e' => 2,
        b'f' => 3,
        b'g' => 4,
        b'a' => 5,
        b'b' => 6,
        _ => return None,
    };
    let mut rest = &tok[1..];
    let mut alter = 0i8;
    loop {
        if let Some(r) = rest.strip_prefix("is") {
            alter += 1;
            rest = r;
        } else if let Some(r) = rest.strip_prefix("es").or_else(|| rest.strip_prefix("s").filter(|_| step == 2 || step == 5)) {
            alter -= 1;
            rest = r;
        } else {
            break;
        }
    }
    let mut octave = 3i8;
    while let Some(r) = rest.strip_prefix('\'') {
        octave += 1;
        rest = r;
    }
    while let Some(r) = rest.strip_prefix(',') {
        octave -= 1;
        rest = r;
    }
    Some((Pitch { id: 0, step, alter, octave, tie: false }, rest))
}

fn parse_duration(s: &str) -> Option<(Frac, &str)> {
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let base: i64 = digits.parse().ok()?;
    let mut rest = &s[digits.len()..];
    let mut d = Frac::new(1, base);
    let mut add = d;
    while let Some(r) = rest.strip_prefix('.') {
        add = add * Frac::new(1, 2);
        d = d + add;
        rest = r;
    }
    if let Some(r) = rest.strip_prefix("*2/3") {
        d = d * Frac::new(2, 3);
        rest = r;
    }
    Some((d, rest))
}

/// One voice from notation; ids are assigned from `next_id`.
pub fn voice(text: &str, index: u8, next_id: &mut u64) -> Voice {
    voice_and_slurs(text, index, next_id).0
}

/// One voice and its slurs (`(` … `)`, which may nest).
pub fn voice_and_slurs(text: &str, index: u8, next_id: &mut u64) -> (Voice, Vec<Slur>) {
    let mut slurs: Vec<Slur> = Vec::new();
    let mut open_slurs: Vec<u64> = Vec::new();
    let mut events: Vec<Event> = Vec::new();
    let mut at = Frac::ZERO;
    let mut dur = Frac::new(1, 4);
    let mut last_chord: Vec<Pitch> = Vec::new();
    let spaced = text.replace('\\', " \\").replace('<', " < ").replace('>', " > ").replace('{', " { ").replace('}', " } ");
    let mut tuplets: Vec<Tuplet> = Vec::new();
    let mut open: Vec<(Frac, i64, i64)> = Vec::new(); // (start, actual, normal)
    let mut pending: Option<(i64, i64)> = None;
    // Grace notes: `\grace`/`\acciaccatura` arms the next note or group.
    let mut grace_next: Option<bool> = None; // Some(slash)
    let mut grace_group: Option<bool> = None;
    let mut graces: Vec<Grace> = Vec::new();
    // Re-glue chord closings with their duration/marks: "> 2~" -> ">2~"
    let mut tokens: Vec<String> = Vec::new();
    let mut in_chord = false;
    for t in spaced.split_whitespace() {
        if t == "<" {
            in_chord = true;
            tokens.push("<".into());
        } else if t.starts_with('>') && in_chord {
            in_chord = false;
            tokens.push(t.to_string());
        } else if !in_chord && !tokens.is_empty() && tokens.last().is_some_and(|l| l.starts_with('>')) && !t.starts_with(|c: char| c.is_ascii_alphabetic() || c == '<') && !t.starts_with('\\') {
            let l = tokens.pop().unwrap();
            tokens.push(l + t);
        } else {
            tokens.push(t.to_string());
        }
    }
    let mut i = 0;
    while i < tokens.len() {
        let t = tokens[i].clone();
        i += 1;
        if t == "\\tuplet" {
            let ratio = tokens.get(i).cloned().unwrap_or_default();
            i += 1;
            if let Some((a, n)) = ratio.split_once('/') {
                pending = Some((a.parse().unwrap_or(3), n.parse().unwrap_or(2)));
            }
            continue;
        }
        if t == "\\grace" || t == "\\acciaccatura" || t == "\\appoggiatura" {
            grace_next = Some(t == "\\acciaccatura");
            continue;
        }
        if t == "{" {
            if let Some((a, n)) = pending.take() {
                open.push((at, a, n));
            } else if let Some(slash) = grace_next.take() {
                grace_group = Some(slash);
            }
            continue;
        }
        if t == "}" && grace_group.is_some() {
            grace_group = None;
            continue;
        }
        if t == "}" {
            if let Some((start, actual, normal)) = open.pop() {
                tuplets.push(Tuplet { start, length: at - start, actual, normal });
            }
            continue;
        }
        if let Some(dynamic) = t.strip_prefix('\\') {
            if let Some(e) = events.last_mut() {
                match dynamic {
                    "fermata" => e.art |= art::FERMATA,
                    "trill" => e.art |= art::TRILL,
                    "mordent" => e.art |= art::MORDENT,
                    "turn" => e.art |= art::TURN,
                    "arpeggio" => e.art |= art::ARPEGGIO,
                    "breathe" => e.art |= art::BREATH,
                    "upbow" => e.art |= art::UP_BOW,
                    "downbow" => e.art |= art::DOWN_BOW,
                    "cr" => e.spans |= span::CRESC_START,
                    "decr" => e.spans |= span::DIM_START,
                    "!" => e.spans |= span::HAIRPIN_END,
                    // \ly=syl (verse 1), \ly2=syl; a trailing - joins the next
                    // syllable, a trailing _ draws an extender.
                    l if l.starts_with("ly") && l.contains('=') => {
                        let (head, text) = l.split_once('=').unwrap();
                        let verse = head[2..].parse().unwrap_or(1);
                        let (text, hyphen, extend) = match text {
                            t if t.ends_with('-') => (&t[..t.len() - 1], true, false),
                            t if t.ends_with('_') => (&t[..t.len() - 1], false, true),
                            t => (t, false, false),
                        };
                        e.lyrics.push(corinzia_score::score::Lyric { verse, text: text.replace('~', " "), hyphen, extend });
                    }
                    d => e.dynamic = Some(d.to_string()),
                }
            }
            continue;
        }
        let (pitches, mut rest, is_rest): (Vec<Pitch>, String, bool) = if t == "<" {
            let mut ps = Vec::new();
            let mut tail = String::new();
            while i < tokens.len() {
                let u = tokens[i].clone();
                i += 1;
                if let Some(after) = u.strip_prefix('>') {
                    tail = after.to_string();
                    break;
                }
                if let Some((p, _)) = parse_pitch(&u) {
                    ps.push(p);
                }
            }
            (ps, tail, false)
        } else if let Some(r) = t.strip_prefix('q') {
            (last_chord.clone(), r.to_string(), false)
        } else if let Some(r) = t.strip_prefix('r') {
            (vec![], r.to_string(), true)
        } else if let Some((p, r)) = parse_pitch(&t) {
            (vec![p], r.to_string(), false)
        } else {
            continue;
        };
        if let Some((d, r)) = parse_duration(&rest) {
            dur = d;
            rest = r.to_string();
        }
        // A grace note waits for the next note.
        if let Some(slash) = grace_next.take().or(grace_group) {
            let base = (Frac::new(1, 1) * Frac::new(dur.den, dur.num)).num.clamp(8, 64) as u8;
            let mut ps = pitches.clone();
            for p in ps.iter_mut() {
                p.id = *next_id;
                *next_id += 1;
            }
            graces.push(Grace { id: *next_id, pitches: ps, base, slash });
            *next_id += 1;
            continue;
        }
        // Sounding time inside tuplets.
        let scale = open.iter().fold(Frac::int(1), |f, &(_, a, n)| f * Frac::new(n, a));
        let mut e = Event { id: *next_id, start: at, dur: dur * scale, pitches, art: 0, dynamic: None, spans: 0, lyrics: vec![], graces: std::mem::take(&mut graces), fingering: vec![] };
        *next_id += 1;
        for p in e.pitches.iter_mut() {
            p.id = *next_id;
            *next_id += 1;
        }
        let mut r = rest.as_str();
        while !r.is_empty() {
            if r.starts_with('(') {
                open_slurs.push(e.id);
            } else if r.starts_with(')') {
                if let Some(from) = open_slurs.pop() {
                    slurs.push(Slur { id: *next_id, from, to: e.id, dir: 0 });
                    *next_id += 1;
                }
            }
            // Fingering: -1 .. -5.
            if let Some(d) = r.strip_prefix('-').and_then(|x| x.chars().next()).and_then(|c| c.to_digit(10)) {
                e.fingering.push(d as u8);
                r = &r[2..];
                continue;
            }
            let (bit, n) = match r {
                s if s.starts_with('~') => (0, 1),
                s if s.starts_with("-.") => (art::STACCATO, 2),
                s if s.starts_with("->") => (art::ACCENT, 2),
                s if s.starts_with("--") => (art::TENUTO, 2),
                s if s.starts_with("-!") => (art::STACCATISSIMO, 2),
                s if s.starts_with("-^") => (art::MARCATO, 2),
                _ => (0, 1),
            };
            if r.starts_with('~') {
                e.pitches.iter_mut().for_each(|p| p.tie = true);
            }
            e.art |= bit;
            r = &r[n.min(r.len())..];
        }
        if !is_rest {
            last_chord = e.pitches.iter().map(|p| Pitch { tie: false, ..*p }).collect();
        }
        at = at + e.dur;
        events.push(e);
    }
    tuplets.sort_by(|a, b| a.start.cmp(&b.start).then(b.length.cmp(&a.length)));
    (Voice { index, events, tuplets }, slurs)
}

/// A score from (clef, voices) per staff, in one meter and key.
pub fn score(staves: &[(&str, &[&str])], meter: (i64, i64), fifths: i32) -> Score {
    let mut id = 1;
    let mut slurs = Vec::new();
    let parts: Vec<Part> = staves
        .iter()
        .enumerate()
        .map(|(i, (clef, voices))| Part {
            name: format!("Staff {}", i + 1),
            abbr: String::new(),
            clef: clef.to_string(),
            clefs: vec![],
            texts: vec![],
            chords: vec![],
            brace: false,
            key_shift: 0,
            key_shifts: vec![],
            group: 0,
            lines: vec![],
            transposition: (0, 0),
            voices: voices
                .iter()
                .enumerate()
                .map(|(vi, v)| {
                    let (v, s) = voice_and_slurs(v, vi as u8 + 1, &mut id);
                    slurs.extend(s);
                    v
                })
                .collect(),
        })
        .collect();
    let end = parts
        .iter()
        .flat_map(|p| p.voices.iter().filter_map(|v| v.events.last().map(|e| e.start + e.dur)))
        .max()
        .unwrap_or(Frac::int(1));
    // Round up to whole bars.
    let bar = Frac::new(meter.0, meter.1);
    let q = end * Frac::new(bar.den, bar.num);
    let bars = (q.num + q.den - 1) / q.den;
    Score {
        title: String::new(),
        work_title: String::new(),
        composer: String::new(),
        instrument: String::new(),
        length: bar * Frac::int(bars.max(1)),
        tempo: vec![],
        meter: vec![Meter { at: Frac::ZERO, num: meter.0, den: meter.1, pickup: false }],
        keys: vec![corinzia_score::score::Key { at: Frac::ZERO, fifths, minor: false }],
        barlines: vec![],
        endings: vec![],
        marks: vec![],
        navigation: vec![],
        parts,
        slurs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_notation() {
        let mut id = 1;
        let v = voice("c'4 d'8 e' <e' gis'>2~ q4 r8. fes''16-.\\f", 1, &mut id);
        assert_eq!(v.events.len(), 7);
        assert_eq!(v.events[0].pitches[0].octave, 4);
        assert_eq!(v.events[2].dur, Frac::new(1, 8)); // sticky duration
        assert_eq!(v.events[3].pitches.len(), 2);
        assert_eq!(v.events[3].pitches[1].alter, 1);
        assert!(v.events[3].pitches.iter().all(|p| p.tie));
        assert_eq!(v.events[4].pitches.len(), 2); // q
        assert!(v.events[5].pitches.is_empty() && v.events[5].dur == Frac::new(3, 16));
        assert_eq!(v.events[6].pitches[0].alter, -1);
        assert_eq!(v.events[6].art, art::STACCATO);
        assert_eq!(v.events[6].dynamic.as_deref(), Some("f"));
        assert_eq!(v.events[6].start, Frac::new(1, 4) + Frac::new(1, 4) + Frac::new(1, 2) + Frac::new(1, 4) + Frac::new(3, 16));
    }
}
