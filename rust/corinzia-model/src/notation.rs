//! A movement as the engraver's and exporters' input
//! (`corinzia_score::score::Score`): staff names and clefs from each lane's
//! first instrument, instrument changes written as text.

use crate::model::*;
use corinzia_score::score as sc;

/// The movement at concert pitch.
pub fn score(project: &Project, m: &Movement) -> sc::Score {
    score_as(project, m, false)
}

/// Chord-symbol text moved by an interval: its root and bass letters
/// ("Bb7/D" up a M2 is "C7/E").
pub fn transpose_chord_text(text: &str, by: Interval) -> String {
    let spell = |letter: char, rest: &str| -> (String, usize) {
        let step = "CDEFGAB".find(letter).unwrap() as i8;
        let acc: String = rest.chars().take_while(|c| matches!(c, '#' | 'b' | '♯' | '♭')).collect();
        let alter = acc.chars().map(|c| if c == '#' || c == '♯' { 1 } else { -1 }).sum::<i8>();
        let p = Pitch { step, alter, octave: 4 }.transposed(by);
        let name = format!("{}{}", "CDEFGAB".as_bytes()[p.step as usize] as char, match p.alter {
            a if a > 0 => "#".repeat(a as usize),
            a if a < 0 => "b".repeat((-a) as usize),
            _ => String::new(),
        });
        (name, acc.len())
    };
    let mut out = String::new();
    let mut chars = text.char_indices().peekable();
    let mut at_root = true;
    while let Some((i, c)) = chars.next() {
        if at_root && "CDEFGAB".contains(c) {
            let (name, skip) = spell(c, &text[i + c.len_utf8()..]);
            out.push_str(&name);
            for _ in 0..skip {
                chars.next();
            }
            at_root = false;
        } else {
            at_root = c == '/';
            out.push(c);
        }
    }
    out
}

/// The orchestral family an instrument belongs to, from its name (English,
/// with common Italian, German and French names): brackets join a family's
/// staves. Keyboards, harps and guitars belong to none.
pub fn family(name: &str) -> Option<&'static str> {
    let n = name.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| n.contains(w));
    if has(&["piano", "organ", "celesta", "harpsichord", "harp", "guitar", "keyboard", "synth", "accordion"]) {
        None
    } else if has(&["flute", "piccolo", "oboe", "cor anglais", "english horn", "clarinet", "bassoon", "sax", "recorder", "flauto", "clarinetto", "fagott", "flöte", "klarinette"]) {
        Some("woodwinds")
    } else if has(&["horn", "trumpet", "trombone", "tuba", "cornet", "euphonium", "flugel", "corno", "tromba", "trompete", "posaune"]) {
        Some("brass")
    } else if has(&["timpani", "percussion", "drum", "cymbal", "triangle", "glock", "xylo", "marimba", "vibra", "tam-tam", "gong", "bells", "pauken"]) {
        Some("percussion")
    } else if has(&["violin", "viola", "cello", "violoncell", "contrabass", "double bass", "string bass", "strings", "violon", "geige", "kontrabass"]) {
        Some("strings")
    } else if has(&["soprano", "mezzo", "alto", "tenor", "baritone", "voice", "choir", "chorus", "contralto"]) || n.trim_start().starts_with("bass") {
        // ("Bass" alone is the choir's; the orchestra's is a double bass.)
        Some("voices")
    } else {
        None
    }
}

/// Bracket groups per lane: a run of at least two lanes of one family gets
/// a number (from 1); others 0. The staves of one player stay together.
fn bracket_groups(project: &Project, m: &Movement) -> Vec<u32> {
    let fam: Vec<Option<&str>> = m
        .lanes
        .iter()
        .map(|l| l.changes.first().and_then(|c| project.instrument(c.instrument)).and_then(|i| family(&i.name)))
        .collect();
    let mut out = vec![0; fam.len()];
    let mut next = 1;
    let mut i = 0;
    while i < fam.len() {
        let mut j = i + 1;
        while j < fam.len() && fam[i].is_some() && fam[j] == fam[i] {
            j += 1;
        }
        if fam[i].is_some() && j - i >= 2 {
            out[i..j].iter_mut().for_each(|g| *g = next);
            next += 1;
        }
        i = j;
    }
    out
}

/// One player's part: their staves alone, as written (transposed), named
/// in the header.
pub fn part_score(project: &Project, m: &Movement, player: Id) -> sc::Score {
    let mut s = score_as(project, m, true);
    let keep: Vec<bool> = m.lanes.iter().map(|l| l.player == player).collect();
    let mut k = 0;
    s.parts.retain(|_| {
        k += 1;
        keep[k - 1]
    });
    for p in s.parts.iter_mut() {
        p.group = 0;
    }
    s.instrument = s.parts.first().map(|p| p.name.clone()).or_else(|| project.player(player).map(|p| p.name.clone())).unwrap_or_default();
    s
}

/// The movement for the engraver: at concert pitch, or `transposed` with
/// each transposing instrument's notes, key and chord symbols as written.
pub fn score_as(project: &Project, m: &Movement, transposed: bool) -> sc::Score {
    let groups = bracket_groups(project, m);
    let parts = m
        .lanes
        .iter()
        .enumerate()
        .map(|(li, lane)| {
            let first = lane.changes.first().and_then(|c| project.instrument(c.instrument));
            // The interval each event is written at.
            let interval_at = |at: corinzia_score::Frac| -> Interval {
                if !transposed {
                    return Interval::default();
                }
                lane.instrument_at(at).and_then(|i| project.instrument(i)).map(|i| i.transposition).unwrap_or_default()
            };
            let name = first.map(|i| i.name.clone()).or_else(|| project.player(lane.player).map(|p| p.name.clone())).unwrap_or_default();
            let mut texts: Vec<sc::Text> = lane
                .texts
                .iter()
                .map(|t| sc::Text { id: t.id, at: t.at, text: t.text.clone(), technique: t.kind == TextKind::Technique })
                .collect();
            for c in lane.changes.iter().skip(1) {
                if let Some(i) = project.instrument(c.instrument) {
                    texts.push(sc::Text { id: c.id, at: c.at, text: i.name.clone(), technique: true });
                }
            }
            sc::Part {
                name,
                abbr: first.map(|i| i.engraver_name.clone()).unwrap_or_default(),
                clef: first.map(|i| i.clef.clone()).unwrap_or_else(|| "treble".into()),
                clefs: lane.clefs.iter().map(|c| sc::ClefChange { at: c.at, clef: c.clef.clone() }).collect(),
                texts,
                chords: lane
                    .chords
                    .iter()
                    .map(|c| {
                        let by = interval_at(c.at);
                        let text = if by.is_zero() { c.text.clone() } else { transpose_chord_text(&c.text, by) };
                        sc::ChordSymbol { at: c.at, text, id: c.id }
                    })
                    .collect(),
                voices: lane
                    .voices
                    .iter()
                    .map(|v| sc::Voice {
                        index: v.index,
                        tuplets: v.tuplets.iter().map(|t| sc::Tuplet { start: t.start, length: t.length, actual: t.actual, normal: t.normal }).collect(),
                        events: v
                            .events
                            .iter()
                            .map(|e| sc::Event {
                                id: e.id,
                                start: e.start,
                                dur: e.duration,
                                pitches: if e.is_rest() {
                                    vec![]
                                } else {
                                    let by = interval_at(e.start);
                                    e.heads
                                        .iter()
                                        .map(|h| {
                                            let p = h.pitch.transposed(by);
                                            sc::Pitch { id: h.id, step: p.step as u8, alter: p.alter, octave: p.octave, tie: h.tie }
                                        })
                                        .collect()
                                },
                                art: e.articulations,
                                dynamic: e.dynamic.and_then(|d| DYNAMICS.get(d as usize)).map(|d| d.to_string()),
                                spans: e.spans,
                                lyrics: e.lyrics.clone(),
                                fingering: e.fingering.clone(),
                                graces: {
                                    let by = interval_at(e.start);
                                    e.graces
                                        .iter()
                                        .map(|g| sc::Grace {
                                            id: g.id,
                                            pitches: g
                                                .heads
                                                .iter()
                                                .map(|h| {
                                                    let p = h.pitch.transposed(by);
                                                    sc::Pitch { id: h.id, step: p.step as u8, alter: p.alter, octave: p.octave, tie: false }
                                                })
                                                .collect(),
                                            base: g.base,
                                            slash: g.slash,
                                        })
                                        .collect()
                                },
                            })
                            .collect(),
                    })
                    .collect(),
                brace: m.lanes.get(li + 1).is_some_and(|next| next.player == lane.player),
                key_shift: interval_at(corinzia_score::Frac::ZERO).fifths(),
                group: groups[li],
                lines: lane.lines.clone(),
                transposition: first.map(|i| (i.transposition.steps, i.transposition.semitones)).unwrap_or((0, 0)),
            }
        })
        .collect();
    sc::Score {
        title: m.title.clone(),
        work_title: project.title.clone(),
        composer: project.composer.clone(),
        instrument: String::new(),
        length: m.length,
        tempo: m.tempo.iter().enumerate().map(|(i, t)| sc::Tempo { id: sc::ids::tempo(i), at: t.at, bpm: t.bpm, text: t.text.clone() }).collect(),
        meter: m.meter.iter().map(|t| sc::Meter { at: t.at, num: t.num, den: t.den, pickup: t.pickup }).collect(),
        keys: m.keys.iter().map(|k| sc::Key { at: k.at, fifths: k.fifths, minor: k.minor }).collect(),
        barlines: m
            .barlines
            .iter()
            .map(|b| sc::Barline {
                at: b.at,
                kind: match b.kind {
                    BarKind::Double => sc::BarKind::Double,
                    BarKind::Final => sc::BarKind::Final,
                    BarKind::RepeatStart => sc::BarKind::RepeatStart,
                    BarKind::RepeatEnd => sc::BarKind::RepeatEnd,
                    BarKind::RepeatBoth => sc::BarKind::RepeatBoth,
                },
            })
            .collect(),
        endings: m
            .endings
            .iter()
            .map(|e| sc::Ending { from: e.from, to: e.to, numbers: (1..=8u8).filter(|n| e.passes & (1 << (n - 1)) != 0).collect() })
            .collect(),
        marks: m.rehearsal.iter().map(|r| r.at).collect(),
        navigation: m.navigation.clone(),
        parts,
        slurs: m.slurs.clone(),
    }
}
