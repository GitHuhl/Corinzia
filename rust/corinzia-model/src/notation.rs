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

/// The first bar start at or after `at`.
fn bar_at_or_after(m: &Movement, at: corinzia_score::Frac) -> corinzia_score::Frac {
    let mut t = corinzia_score::Frac::ZERO;
    while t < at && t < m.length {
        let meter = m.meter.iter().rfind(|x| x.at <= t);
        let len = meter.map(|x| corinzia_score::Frac::new(x.num.max(1), x.den.max(1))).unwrap_or(corinzia_score::Frac::int(1));
        t = t + len;
    }
    t
}

/// One player's part: their staves alone, as written (transposed), named
/// in the header.
pub fn part_score(project: &Project, m: &Movement, player: Id) -> sc::Score {
    let mut s = score_for(project, m, true, true);
    let keep: Vec<bool> = m.lanes.iter().map(|l| l.player == player).collect();
    let mut k = 0;
    s.parts.retain(|_| {
        k += 1;
        keep[k - 1]
    });
    for p in s.parts.iter_mut() {
        p.group = 0;
    }
    // A part has breaks of its own.
    s.breaks = m.breaks.iter().filter(|b| b.part == Some(player)).map(|b| (b.at, b.page)).collect();
    s.instrument = s.parts.first().map(|p| p.name.clone()).or_else(|| project.player(player).map(|p| p.name.clone())).unwrap_or_default();
    s
}

/// The movement for the engraver: at concert pitch, or `transposed` with
/// each transposing instrument's notes, key and chord symbols as written.
pub fn score_as(project: &Project, m: &Movement, transposed: bool) -> sc::Score {
    score_for(project, m, transposed, false)
}

/// The movement for the score, or (`part`) with the texts the parts show.
fn score_for(project: &Project, m: &Movement, transposed: bool, part: bool) -> sc::Score {
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
                .filter(|t| t.shown != if part { Shown::Score } else { Shown::Parts })
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
                head_shapes: lane
                    .changes
                    .iter()
                    .filter_map(|c| project.instrument(c.instrument))
                    .flat_map(|i| i.drums.iter())
                    .filter(|d| d.head != Notehead::Normal)
                    .map(|d| {
                        let shape = match d.head {
                            Notehead::X => 1,
                            Notehead::CircleX => 2,
                            Notehead::Diamond => 3,
                            Notehead::Triangle => 4,
                            Notehead::Normal => 0,
                        };
                        (d.position.step as i32 + 7 * d.position.octave as i32, shape)
                    })
                    .collect(),
                tab: first.map(|i| i.tuning.clone()).unwrap_or_default(),
                drums: first.map(|i| i.drums.iter().map(|d| (d.position.diatonic(), d.name.clone(), d.key)).collect()).unwrap_or_default(),
                instruments: lane.changes.iter().filter_map(|c| Some((c.at, project.instrument(c.instrument)?.name.clone()))).collect(),
                cross: m.cross.iter().filter(|c| lane.voices.iter().any(|v| v.events.iter().any(|e| e.id == c.0))).copied().collect(),
                key_shift: interval_at(corinzia_score::Frac::ZERO).fifths(),
                key_shifts: {
                    // A change to an instrument transposing otherwise takes
                    // its key from the next bar on (where it is shown).
                    let mut out: Vec<(corinzia_score::Frac, i32)> = Vec::new();
                    let mut last = interval_at(corinzia_score::Frac::ZERO).fifths();
                    for c in lane.changes.iter().skip(1) {
                        let shift = interval_at(c.at).fifths();
                        if shift != last {
                            let at = bar_at_or_after(m, c.at);
                            if at < m.length {
                                out.retain(|(t, _)| *t < at);
                                out.push((at, shift));
                            }
                            last = shift;
                        }
                    }
                    out
                },
                group: groups[li],
                lines: lane.lines.clone(),
                transposition: first.map(|i| (i.transposition.steps, i.transposition.semitones)).unwrap_or((0, 0)),
            }
        })
        .collect();
    sc::Score {
        subtitle: project.subtitle.clone(),
        lyricist: project.lyricist.clone(),
        arranger: project.arranger.clone(),
        copyright: project.copyright.clone(),
        breaks: m.breaks.iter().filter(|b| b.part.is_none()).map(|b| (b.at, b.page)).collect(),
        title: m.title.clone(),
        work_title: project.title.clone(),
        composer: project.composer.clone(),
        instrument: String::new(),
        length: m.length,
        tempo: m.tempo.iter().enumerate().map(|(i, t)| sc::Tempo { id: sc::ids::tempo(i), at: t.at, bpm: t.bpm, text: t.text.clone() }).collect(),
        meter: m.meter.iter().map(|t| sc::Meter { at: t.at, num: t.num, den: t.den, pickup: t.pickup, beams: t.beams }).collect(),
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

/// A name's stem and trailing number: ("Flute", "1") for "Flute 1".
fn numbered(name: &str) -> (&str, &str) {
    let t = name.trim_end();
    let stem = t.trim_end_matches(|c: char| c.is_ascii_digit());
    if stem.len() == t.len() { (t, "") } else { (stem.trim_end(), &t[stem.len()..]) }
}

/// A note of a bar, to compare players: (start, length, pitches).
type BarNote = (corinzia_score::Frac, corinzia_score::Frac, Vec<(u8, i8, i8)>);

/// Bar starts of a score, and its end.
fn bar_starts(s: &sc::Score) -> Vec<corinzia_score::Frac> {
    let mut out = vec![corinzia_score::Frac::ZERO];
    let mut t = corinzia_score::Frac::ZERO;
    while t < s.length {
        let len = s.meter.iter().rfind(|x| x.at <= t).map_or(corinzia_score::Frac::int(1), |x| corinzia_score::Frac::new(x.num.max(1), x.den.max(1)));
        t = t + len;
        out.push(t.min(s.length));
    }
    out
}

/// Condensing for the score: two neighbouring single-staff players of one
/// instrument ("Flute 1", "Flute 2") share a staff, the first player's
/// voice up, the second's down; where they play the same bar it is written
/// once, under "a 2", and the second player's rests go. Returns, for each
/// staff now, the staff it came from (the first of a pair).
pub fn condense(s: &mut sc::Score) -> Vec<usize> {
    let bars = bar_starts(s);
    let mut map = Vec::new();
    let mut parts = Vec::new();
    let old = std::mem::take(&mut s.parts);
    let single = |i: usize| !old[i].brace && (i == 0 || !old[i - 1].brace) && old[i].voices.iter().all(|v| v.index == 1 || v.events.is_empty());
    let mut i = 0;
    while i < old.len() {
        let pair = i + 1 < old.len() && {
            let (a, b) = (&old[i], &old[i + 1]);
            let (na, nb) = (numbered(&a.name), numbered(&b.name));
            na.0 == nb.0 && !na.0.is_empty() && a.clef == b.clef && a.transposition == b.transposition && a.group == b.group && single(i) && single(i + 1)
        };
        if !pair {
            parts.push(old[i].clone());
            map.push(i);
            i += 1;
            continue;
        }
        let (a, b) = (&old[i], &old[i + 1]);
        let mut p = a.clone();
        let (na, nb) = (numbered(&a.name), numbered(&b.name));
        let nums = |x: &str, y: &str| if x.is_empty() && y.is_empty() { "1.2".to_string() } else { format!("{}.{}", if x.is_empty() { "1" } else { x }, if y.is_empty() { "2" } else { y }) };
        p.name = format!("{} {}", na.0, nums(na.1, nb.1));
        let (aa, ab) = (numbered(&a.abbr), numbered(&b.abbr));
        p.abbr = if aa.0.is_empty() { String::new() } else { format!("{} {}", aa.0, nums(aa.1, ab.1)) };
        let va: Vec<&sc::Event> = a.voices.iter().flat_map(|v| v.events.iter()).collect();
        let vb: Vec<&sc::Event> = b.voices.iter().flat_map(|v| v.events.iter()).collect();
        let in_bar = |evs: &[&sc::Event], from: corinzia_score::Frac, to: corinzia_score::Frac| -> Option<Vec<BarNote>> {
            let mut out = Vec::new();
            for e in evs.iter().filter(|e| e.start < to && e.start + e.dur > from) {
                if e.start < from || e.start + e.dur > to {
                    return None; // across the barline: never the same
                }
                out.push((e.start, e.dur, e.pitches.iter().map(|p| (p.step, p.alter, p.octave)).collect()));
            }
            Some(out)
        };
        let mut drop_b: Vec<(corinzia_score::Frac, corinzia_score::Frac)> = Vec::new();
        let mut was_unison = false;
        let mut id = u64::MAX - 1000 - i as u64 * 10000;
        for w in bars.windows(2) {
            let (from, to) = (w[0], w[1]);
            let (ea, eb) = (in_bar(&va, from, to), in_bar(&vb, from, to));
            let b_rests = eb.as_ref().is_some_and(|x| x.iter().all(|e| e.2.is_empty()));
            let unison = matches!((&ea, &eb), (Some(x), Some(y)) if x == y && x.iter().any(|e| !e.2.is_empty()));
            if unison || b_rests {
                drop_b.push((from, to));
            }
            if unison && !was_unison {
                p.texts.push(sc::Text { id, at: from, text: "a 2".into(), technique: true });
                id += 1;
            }
            was_unison = unison;
        }
        let dropped = |e: &sc::Event| drop_b.iter().any(|&(f, t)| e.start >= f && e.start + e.dur <= t);
        let mut second: Vec<sc::Event> = vb.iter().filter(|e| !dropped(e)).map(|e| (*e).clone()).collect();
        // Rests in voice 2 only where the second player rests beside the first's notes.
        second.retain(|e| !e.pitches.is_empty() || va.iter().any(|x| !x.pitches.is_empty() && x.start < e.start + e.dur && e.start < x.start + x.dur));
        let tuplets = b.voices.iter().flat_map(|v| v.tuplets.iter().copied()).collect();
        if !second.is_empty() {
            p.voices.retain(|v| !v.events.is_empty());
            p.voices.push(sc::Voice { index: 2, events: second, tuplets });
        }
        for t in &b.texts {
            if !p.texts.iter().any(|x| x.at == t.at && x.text == t.text) {
                p.texts.push(t.clone());
            }
        }
        // Audio cues go along (octave lines and pedals belong to the notes).
        p.lines.extend(b.lines.iter().filter(|l| l.kind == sc::LineKind::Cue).copied());
        parts.push(p);
        map.push(i);
        i += 2;
    }
    s.parts = parts;
    map
}
