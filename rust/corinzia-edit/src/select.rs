//! Filters: which elements of a passage (or a movement) a query picks.
//!
//! A query is a few words: what to pick (notes by default; rests,
//! dynamics, lyrics, chord symbols, text) and, for notes, conditions that
//! must all hold: `top`/`bottom` (of chords), `chords`, `voice 2`, a pitch
//! (`F#`, `Bb4`), `above C5`/`below G3`, articulations (`staccato`) and
//! `tied`. The result is element ids as the engraving tags them: noteheads,
//! rest events, `ids::dynamic`/`ids::lyric` ids, chord symbol and text ids.

use corinzia_model::*;
use corinzia_score::score::{art, ids};
use corinzia_score::Frac;

#[derive(Clone, Debug, PartialEq)]
pub enum Pick {
    Notes,
    Rests,
    /// Dynamics; with a mark, only that one (an index into `DYNAMICS`).
    Dynamics(Option<u8>),
    /// Lyrics; with a verse, only its syllables.
    Lyrics(Option<u8>),
    ChordSymbols,
    /// Text items; with words, only those reading so.
    Texts(Option<String>),
}

/// A note's pitch to match: step and alter, with an octave or in any.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PitchMatch {
    pub step: i8,
    pub alter: i8,
    pub octave: Option<i8>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Query {
    pub pick: Pick,
    pub top: bool,
    pub bottom: bool,
    /// Only events of two or more notes.
    pub chords: bool,
    pub voice: Option<u8>,
    pub pitch: Option<PitchMatch>,
    /// Strictly above / below this pitch.
    pub above: Option<Pitch>,
    pub below: Option<Pitch>,
    /// Articulation bits that must all be there.
    pub art: u16,
    pub tied: bool,
}

impl Query {
    pub fn notes() -> Query {
        Query { pick: Pick::Notes, top: false, bottom: false, chords: false, voice: None, pitch: None, above: None, below: None, art: 0, tied: false }
    }
    pub fn of(pick: Pick) -> Query {
        Query { pick, ..Query::notes() }
    }
}

const ARTICULATIONS: [(&str, u16); 13] = [
    ("staccato", art::STACCATO),
    ("accent", art::ACCENT),
    ("tenuto", art::TENUTO),
    ("marcato", art::MARCATO),
    ("fermata", art::FERMATA),
    ("staccatissimo", art::STACCATISSIMO),
    ("trill", art::TRILL),
    ("mordent", art::MORDENT),
    ("turn", art::TURN),
    ("tremolo", art::TREMOLO),
    ("arpeggio", art::ARPEGGIO),
    ("up bow", art::UP_BOW),
    ("down bow", art::DOWN_BOW),
];

/// "C", "f#", "Bb4", "e♭5": step, alter and maybe an octave.
pub fn parse_pitch_match(t: &str) -> Option<PitchMatch> {
    let t = t.replace('♯', "#").replace('♭', "b");
    let mut chars = t.chars();
    let letter = chars.next()?.to_ascii_uppercase();
    let step = "CDEFGAB".find(letter)? as i8;
    let rest: String = chars.collect();
    let acc: String = rest.chars().take_while(|c| *c == '#' || *c == 'b').collect();
    let alter = acc.chars().map(|c| if c == '#' { 1 } else { -1 }).sum::<i8>();
    let tail = &rest[acc.len()..];
    let octave = if tail.is_empty() { None } else { Some(tail.parse::<i8>().ok().filter(|o| (-1..=9).contains(o))?) };
    (alter.abs() <= 2).then_some(PitchMatch { step, alter, octave })
}

/// The words of a filter, e.g. "top", "voice 2 staccato", "above C5",
/// "F#", "rests", "dynamics mf", "lyrics verse 2", "chord symbols".
pub fn parse_query(text: &str) -> Result<Query, String> {
    let lower = text.trim().to_lowercase().replace(',', " ");
    if lower.is_empty() {
        return Err("Type what to select: notes, top, voice 2, above C5, F#, staccato, rests, dynamics…".into());
    }
    let words: Vec<&str> = lower.split_whitespace().collect();
    let mut q = Query::notes();
    let mut i = 0;
    let pitch = |w: Option<&&str>| -> Result<Pitch, String> {
        let p = w.and_then(|w| parse_pitch_match(w)).filter(|p| p.octave.is_some()).ok_or("Give a pitch with its octave, e.g. above C5")?;
        Ok(Pitch { step: p.step, alter: p.alter, octave: p.octave.unwrap() })
    };
    while i < words.len() {
        let w = words[i];
        let next = words.get(i + 1);
        match w {
            "notes" | "note" | "all" => {}
            "rests" | "rest" => q.pick = Pick::Rests,
            "top" | "highest" | "upper" => q.top = true,
            "bottom" | "lowest" | "lower" => q.bottom = true,
            "chords" => q.chords = true,
            "chord" if next == Some(&"symbols") || next == Some(&"symbol") => {
                q.pick = Pick::ChordSymbols;
                i += 1;
            }
            "symbols" => q.pick = Pick::ChordSymbols,
            "tied" | "ties" => q.tied = true,
            "voice" => {
                let v = next.and_then(|n| n.parse::<u8>().ok()).filter(|v| (1..=4).contains(v)).ok_or("Voice 1 to 4")?;
                q.voice = Some(v);
                i += 1;
            }
            "above" => {
                q.above = Some(pitch(next)?);
                i += 1;
            }
            "below" => {
                q.below = Some(pitch(next)?);
                i += 1;
            }
            "dynamics" | "dynamic" => {
                let mark = next.and_then(|n| dynamic_index(n));
                if mark.is_some() {
                    i += 1;
                }
                q.pick = Pick::Dynamics(mark);
            }
            "lyrics" | "lyric" => {
                let mut verse = None;
                if next == Some(&"verse") {
                    verse = Some(words.get(i + 2).and_then(|n| n.parse::<u8>().ok()).ok_or("Which verse?")?);
                    i += 2;
                }
                q.pick = Pick::Lyrics(verse);
            }
            "text" | "texts" => {
                let rest = words[i + 1..].join(" ");
                q.pick = Pick::Texts((!rest.is_empty()).then_some(rest));
                break;
            }
            "up" | "down" if next == Some(&"bow") => {
                q.art |= if w == "up" { art::UP_BOW } else { art::DOWN_BOW };
                i += 1;
            }
            _ => {
                if let Some((_, bit)) = ARTICULATIONS.iter().find(|(n, _)| *n == w || (n.len() > 4 && w.len() >= 4 && n.starts_with(w))) {
                    q.art |= bit;
                } else if let Some(d) = dynamic_index(w) {
                    q.pick = Pick::Dynamics(Some(d));
                } else if let Some(p) = parse_pitch_match(w) {
                    q.pitch = Some(p);
                } else {
                    return Err(format!("Not understood: \"{w}\""));
                }
            }
        }
        i += 1;
    }
    Ok(q)
}

/// What `q` picks in staves `lanes` over `[from, to)` (events and items
/// starting there), in score order.
pub fn find(m: &Movement, lanes: std::ops::RangeInclusive<usize>, from: Frac, to: Frac, q: &Query) -> Vec<Id> {
    let mut out = Vec::new();
    let inside = |t: Frac| t >= from && t < to;
    for li in lanes {
        let Some(lane) = m.lanes.get(li) else { continue };
        match &q.pick {
            Pick::ChordSymbols => out.extend(lane.chords.iter().filter(|c| inside(c.at)).map(|c| c.id)),
            Pick::Texts(words) => out.extend(lane.texts.iter().filter(|t| inside(t.at) && words.as_ref().is_none_or(|w| t.text.to_lowercase() == *w)).map(|t| t.id)),
            _ => {}
        }
        for v in lane.voices.iter().filter(|v| q.voice.is_none_or(|i| v.index == i)) {
            for e in v.events.iter().filter(|e| inside(e.start)) {
                match &q.pick {
                    Pick::Rests if e.is_rest() => out.push(e.id),
                    Pick::Dynamics(mark) if e.dynamic.is_some() && mark.is_none_or(|d| e.dynamic == Some(d)) => out.push(ids::dynamic(e.id)),
                    Pick::Lyrics(verse) => out.extend(e.lyrics.iter().filter(|l| verse.is_none_or(|n| l.verse == n)).map(|l| ids::lyric(e.id, l.verse))),
                    Pick::Notes if !e.is_rest() => {
                        if (q.chords && e.heads.len() < 2) || e.articulations & q.art != q.art {
                            continue;
                        }
                        let n = e.heads.len();
                        for (k, h) in e.heads.iter().enumerate() {
                            // Heads are kept lowest first.
                            let place_ok = match (q.top, q.bottom) {
                                (false, false) => true,
                                (true, false) => k + 1 == n,
                                (false, true) => k == 0,
                                (true, true) => k == 0 || k + 1 == n,
                            };
                            let key = h.pitch.midi_key();
                            let pitch_ok = q.pitch.is_none_or(|p| h.pitch.step == p.step && h.pitch.alter == p.alter && p.octave.is_none_or(|o| h.pitch.octave == o));
                            if place_ok && pitch_ok && q.above.is_none_or(|a| key > a.midi_key()) && q.below.is_none_or(|b| key < b.midi_key()) && (!q.tied || h.tie) {
                                out.push(h.id);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    out
}

/// A count for the status bar: "12 notes", "1 dynamic".
pub fn describe(pick: &Pick, n: usize) -> String {
    let (one, many) = match pick {
        Pick::Notes => ("note", "notes"),
        Pick::Rests => ("rest", "rests"),
        Pick::Dynamics(_) => ("dynamic", "dynamics"),
        Pick::Lyrics(_) => ("syllable", "syllables"),
        Pick::ChordSymbols => ("chord symbol", "chord symbols"),
        Pick::Texts(_) => ("text", "texts"),
    };
    format!("{n} {}", if n == 1 { one } else { many })
}
