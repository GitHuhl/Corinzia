//! Moving music between staves: exploding chords onto several staves and
//! reducing several staves to chords on one. Both work on one passage and
//! give clips (events timed from the passage's start, ids to be given by
//! `paste_events`).

use corinzia_model::*;
use corinzia_score::Frac;

/// Spreads a passage's chords over `staves` staves, top note to the top
/// staff: a staff left without a note of its own doubles the lowest, and
/// with more notes than staves the bottom staff keeps the rest as a chord.
/// A single note is doubled on every staff; rests stay rests. Lyrics and
/// grace notes stay on the top staff.
pub fn explode(source: &[Event], from: Frac, to: Frac, staves: usize) -> Vec<Vec<Event>> {
    let clip = crate::slice_range(source, from, to);
    (0..staves)
        .map(|j| {
            clip.iter()
                .map(|e| {
                    let mut c = e.clone();
                    if !e.is_rest() {
                        let k = e.heads.len(); // lowest first
                        c.heads = if k >= staves {
                            if j + 1 < staves {
                                vec![e.heads[k - 1 - j].clone()]
                            } else {
                                e.heads[..=k - staves].to_vec()
                            }
                        } else if j < k {
                            vec![e.heads[k - 1 - j].clone()]
                        } else {
                            vec![e.heads[0].clone()]
                        };
                        c.kind = if c.heads.len() > 1 { EventKind::Chord } else { EventKind::Note };
                        c.fingering.clear();
                    }
                    if j > 0 {
                        c.lyrics.clear();
                        c.graces.clear();
                    }
                    c
                })
                .collect()
        })
        .collect()
}

/// Several voices' passages as chords in one voice: the rhythm is cut
/// wherever any of them moves, notes held across a cut are tied, and a
/// pitch in two voices sounds once. Articulations of the notes starting
/// at a point go together; the first dynamic there is kept.
pub fn reduce(voices: &[&[Event]], from: Frac, to: Frac) -> Vec<Event> {
    let clips: Vec<Vec<Event>> = voices.iter().map(|v| crate::slice_range(v, from, to)).collect();
    let length = to - from;
    let mut cuts: Vec<Frac> = vec![Frac::ZERO, length];
    for e in clips.iter().flatten() {
        cuts.push(e.start);
        cuts.push(e.end().min(length));
    }
    cuts.sort();
    cuts.dedup();
    let mut out: Vec<Event> = Vec::new();
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let mut heads: Vec<Head> = Vec::new();
        let (mut art, mut dynamic, mut spans) = (0u16, None, 0u8);
        for e in clips.iter().flatten().filter(|e| e.start <= a && a < e.end() && !e.is_rest()) {
            if e.start == a {
                art |= e.articulations;
                spans |= e.spans;
                dynamic = dynamic.or(e.dynamic);
            }
            for h in &e.heads {
                // Tied on if the note goes on past this cut (or was tied).
                let tie = b < e.end() || h.tie;
                match heads.iter_mut().find(|x| x.pitch.midi_key() == h.pitch.midi_key()) {
                    Some(x) => x.tie |= tie,
                    None => heads.push(Head { id: 0, pitch: h.pitch, tie, velocity: h.velocity }),
                }
            }
        }
        heads.sort_by_key(|h| h.pitch.midi_key());
        let kind = match heads.len() {
            0 => EventKind::Rest,
            1 => EventKind::Note,
            _ => EventKind::Chord,
        };
        // Rests run together.
        if kind == EventKind::Rest {
            if let Some(last) = out.last_mut().filter(|l| l.is_rest()) {
                last.duration = last.duration + (b - a);
                continue;
            }
        }
        out.push(Event { id: 0, kind, start: a, duration: b - a, heads, articulations: art, dynamic, spans, lyrics: vec![], graces: vec![], fingering: vec![] });
    }
    out
}
