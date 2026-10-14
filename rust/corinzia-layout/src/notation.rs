//! A tiny LilyPond-like notation for building scores in tests and previews:
//! `c'4 d'8 e' <e' gis'>2~ q4 r8. fes''16-. \f`.
//!
//! * pitches: `c d e f g a b`, `is`/`es` (repeatable) for ♯/♭, `'`/`,` octave
//!   marks from C3 (`c'` = middle C);
//! * durations `1 2 4 8 16 32 64` with dots, sticky as in LilyPond; `*2/3` for triplets;
//! * `<…>` chords, `q` repeats the last chord, `r` rests, `~` ties;
//! * `-.` `->` `--` `-!` `-^` articulations, `\fermata \trill \mordent \turn
//!   \arpeggio \breathe`, dynamics `\pp` … `\ff`;
//! * `(` `)` slurs, `\cr` `\decr` `\!` hairpins (LilyPond's spellings).

use corinzia_score::score::{art, span, Event, Meter, Part, Pitch, Score, Voice};
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
    let mut events: Vec<Event> = Vec::new();
    let mut at = Frac::ZERO;
    let mut dur = Frac::new(1, 4);
    let mut last_chord: Vec<Pitch> = Vec::new();
    let spaced = text.replace('\\', " \\").replace('<', " < ").replace('>', " > ");
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
        if let Some(dynamic) = t.strip_prefix('\\') {
            if let Some(e) = events.last_mut() {
                match dynamic {
                    "fermata" => e.art |= art::FERMATA,
                    "trill" => e.art |= art::TRILL,
                    "mordent" => e.art |= art::MORDENT,
                    "turn" => e.art |= art::TURN,
                    "arpeggio" => e.art |= art::ARPEGGIO,
                    "breathe" => e.art |= art::BREATH,
                    "cr" => e.spans |= span::CRESC_START,
                    "decr" => e.spans |= span::DIM_START,
                    "!" => e.spans |= span::HAIRPIN_END,
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
        let mut e = Event { id: *next_id, start: at, dur, pitches, art: 0, dynamic: None, spans: 0 };
        *next_id += 1;
        for p in e.pitches.iter_mut() {
            p.id = *next_id;
            *next_id += 1;
        }
        let mut r = rest.as_str();
        while !r.is_empty() {
            if r.starts_with('(') {
                e.spans |= span::SLUR_START;
            } else if r.starts_with(')') {
                e.spans |= span::SLUR_END;
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
        at = at + dur;
        events.push(e);
    }
    Voice { index, events }
}

/// A score from (clef, voices) per staff, in one meter and key.
pub fn score(staves: &[(&str, &[&str])], meter: (i64, i64), fifths: i32) -> Score {
    let mut id = 1;
    let parts: Vec<Part> = staves
        .iter()
        .enumerate()
        .map(|(i, (clef, voices))| Part {
            name: format!("Staff {}", i + 1),
            abbr: String::new(),
            clef: clef.to_string(),
            clefs: vec![],
            texts: vec![],
            voices: voices.iter().enumerate().map(|(vi, v)| voice(v, vi as u8 + 1, &mut id)).collect(),
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
        length: bar * Frac::int(bars.max(1)),
        tempo: vec![],
        meter: vec![Meter { at: Frac::ZERO, num: meter.0, den: meter.1 }],
        keys: vec![corinzia_score::score::Key { at: Frac::ZERO, fifths, minor: false }],
        barlines: vec![],
        endings: vec![],
        marks: vec![],
        parts,
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
