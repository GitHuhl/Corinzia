//! The music prepared for engraving: clefs and keys, measures, and the
//! score's events split at barlines into notatable notes.

use super::*;

// ------------------------------------------------------------- pitch & clef --
pub(super) fn diatonic(p: &Pitch) -> i32 {
    p.step as i32 + 7 * p.octave as i32
}

/// Diatonic step (step + 7 * octave) on the clef's middle line.
pub fn clef_middle(clef: &str) -> i32 {
    match clef {
        "bass" => 1 + 7 * 3,
        "bass_8" => 1 + 7 * 2,
        "alto" => 7 * 4,
        // Percussion positions read as on the treble clef (as in MusicXML).
        "tenor" => 5 + 7 * 3,
        "treble_8" => 6 + 7 * 3,
        _ => 6 + 7 * 4,
    }
}

pub(super) fn clef_glyph(clef: &str) -> (Glyph, f32) {
    // (glyph, y of its origin below the top line)
    match clef {
        "bass" => (Glyph::FClef, 1.0),
        "bass_8" => (Glyph::FClef8vb, 1.0),
        "alto" => (Glyph::CClef, 2.0),
        "tenor" => (Glyph::CClef, 1.0),
        "treble_8" => (Glyph::GClef8vb, 3.0),
        "percussion" => (Glyph::PercussionClef, 2.0),
        _ => (Glyph::GClef, 3.0),
    }
}

/// Alteration the key signature gives a step (0 = C .. 6 = B).
pub(super) fn key_alter(fifths: i32, step: u8) -> i8 {
    const SHARPS: [u8; 7] = [3, 0, 4, 1, 5, 2, 6]; // F C G D A E B
    const FLATS: [u8; 7] = [6, 2, 5, 1, 4, 0, 3]; // B E A D G C F
    if fifths > 0 && SHARPS[..fifths.min(7) as usize].contains(&step) {
        1
    } else if fifths < 0 && FLATS[..(-fifths).min(7) as usize].contains(&step) {
        -1
    } else {
        0
    }
}

/// Staff positions (half-spaces above the middle line) of key-signature
/// accidentals in this clef.
pub(super) fn key_positions(fifths: i32, clef: &str) -> Vec<(i32, Glyph)> {
    const SHARPS_TREBLE: [i32; 7] = [4, 1, 5, 2, -1, 3, 0];
    const FLATS_TREBLE: [i32; 7] = [0, 3, -1, 2, -2, 1, -3];
    let mut shift = (clef_middle("treble") - clef_middle(clef)).rem_euclid(7);
    if shift > 3 {
        shift -= 7;
    }
    let (list, g) = if fifths >= 0 { (SHARPS_TREBLE, Glyph::Sharp) } else { (FLATS_TREBLE, Glyph::Flat) };
    list.iter().take(fifths.unsigned_abs().min(7) as usize).map(|p| (p + shift, g)).collect()
}

pub(super) fn accidental_glyph(alter: i8) -> Glyph {
    match alter {
        -2 => Glyph::DoubleFlat,
        -1 => Glyph::Flat,
        1 => Glyph::Sharp,
        2 => Glyph::DoubleSharp,
        _ => Glyph::Natural,
    }
}

/// `base-shortest-duration`: the reference never exceeds a dotted eighth.
pub(super) const BASE_SHORTEST: Frac = Frac { num: 3, den: 16 };

/// The movement's common shortest duration: the shortest note of each bar,
/// the one most bars have (the shorter on a tie), capped at
/// `BASE_SHORTEST`.
pub(super) fn common_shortest(ms: &[Measure], notes: &[Note]) -> Frac {
    let mut count: Vec<(Frac, usize)> = Vec::new();
    for mi in 0..ms.len() {
        let Some(d) = notes.iter().filter(|n| n.measure == mi && !n.whole_bar).map(|n| n.dur).min() else { continue };
        match count.iter_mut().find(|c| c.0 == d) {
            Some(c) => c.1 += 1,
            None => count.push((d, 1)),
        }
    }
    let best = count.iter().max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0))).map(|c| c.0);
    best.map_or(BASE_SHORTEST, |d| d.min(BASE_SHORTEST))
}

pub(super) fn measures(s: &Score) -> Vec<Measure> {
    let mut meters = s.meter.clone();
    meters.sort_by(|a, b| a.at.cmp(&b.at));
    let mut keys = s.keys.clone();
    keys.sort_by(|a, b| a.at.cmp(&b.at));
    let mut out: Vec<Measure> = Vec::new();
    let mut at = Frac::ZERO;
    while at < s.length && out.len() < 100_000 {
        let m = meters.iter().rev().find(|m| m.at <= at).copied();
        let (mut num, mut den) = m.map(|m| (m.num.max(1), m.den.max(1))).unwrap_or((4, 4));
        let mut len = Frac::new(num, den);
        let mut phase = Frac::ZERO;
        let mut beams = m.map(|m| m.beams).unwrap_or([0; 8]);
        // A pickup is written with the following meter's signature, its
        // beats counted back from that bar's end.
        if let Some(pm) = m.filter(|m| m.pickup) {
            if let Some(next) = meters.iter().find(|x| x.at > pm.at && !x.pickup) {
                (num, den) = (next.num.max(1), next.den.max(1));
                beams = next.beams;
                let full = Frac::new(num, den);
                phase = if full > len { full - len } else { Frac::ZERO };
            }
        }
        if let Some(next) = meters.iter().find(|x| x.at > at && x.at < at + len) {
            len = next.at - at;
        }
        if at + len > s.length {
            len = s.length - at;
        }
        let prev = out.last();
        let show_time = prev.is_none_or(|p| (p.num, p.den) != (num, den));
        let key = keys.iter().rev().find(|k| k.at <= at).copied();
        // A staff whose key shift changes here (an instrument change) shows
        // the key again.
        let shift_changes = prev.is_some() && s.parts.iter().any(|p| p.key_shifts.iter().any(|(t, _)| *t == at));
        let key_changed = match (prev, key) {
            (None, k) => k.or(Some(Key { at: Frac::ZERO, fifths: 0, minor: false })),
            (Some(p), Some(k)) if k.at > p.start => Some(k),
            _ if shift_changes => Some(key.unwrap_or(Key { at: Frac::ZERO, fifths: 0, minor: false })),
            _ => None,
        };
        out.push(Measure { start: at, len, num, den, show_time, key: key_changed, x0: 0.0, x1: 0.0, last: false, shortest: BASE_SHORTEST, stretch: 0.0, squeeze: 0.0, phase, number: 0, multi: 1, beams });
        at = at + len;
    }
    if let Some(m) = out.last_mut() {
        m.last = true;
    }
    let first = if out.first().is_some_and(|m| !m.phase.is_zero()) { 0 } else { 1 };
    for (i, m) in out.iter_mut().enumerate() {
        m.number = i + first;
    }
    out
}

/// A bar's beam groups (in 1/den): the meter's own, else LilyPond's for
/// 5/8 (3+2), 7/8 (2+2+3) and 8/8 (3+3+2); None: by `beam_beat`.
/// Which beam window of its bar a position (from the bar's written start)
/// falls in: by the bar's groups, or windows of `uniform`.
pub(super) fn beam_window(m: &Measure, rel: Frac, uniform: Frac) -> i64 {
    corinzia_score::beams::window(m.num, m.den, &m.beams, rel, uniform)
}

/// The beat rests are split on: the dotted quarter in compound meters,
/// else the signature's unit.
pub(super) fn beat_of(num: i64, den: i64) -> Frac {
    if den >= 8 && num % 3 == 0 && num > 3 {
        Frac::new(3, den)
    } else {
        Frac::new(1, den.max(1))
    }
}

/// A staff's key: the score's, moved for a transposing instrument.
/// A staff's key at `at`: the score's, moved by the staff's key shift then.
pub(super) fn part_key(s: &Score, part: usize, fifths: i32, at: Frac) -> i32 {
    match s.parts.get(part).map(|p| p.key_shift_at(at)).unwrap_or(0) {
        0 => fifths,
        by => {
            let mut f = fifths + by;
            while f > 7 {
                f -= 12;
            }
            while f < -7 {
                f += 12;
            }
            f
        }
    }
}

pub(super) fn key_at(s: &Score, at: Frac) -> i32 {
    s.keys.iter().filter(|k| k.at <= at).max_by(|a, b| a.at.cmp(&b.at)).map(|k| k.fifths).unwrap_or(0)
}

/// A notehead in another shape (percussion), for the value's head.
pub(super) fn shaped_head(g: Glyph, shape: u8) -> Glyph {
    use Glyph::*;
    let k = match g {
        NoteheadWhole => 2,
        NoteheadHalf => 1,
        _ => 0,
    };
    let set = match shape {
        1 => [NoteheadXBlack, NoteheadXHalf, NoteheadXWhole],
        2 => [NoteheadCircleX, NoteheadCircleXHalf, NoteheadCircleXWhole],
        3 => [NoteheadDiamondBlack, NoteheadDiamondHalf, NoteheadDiamondWhole],
        4 => [NoteheadTriangleBlack, NoteheadTriangleHalf, NoteheadTriangleWhole],
        _ => return g,
    };
    set[k]
}

/// Every voice's explicit tuplets as (part, voice index, tuplet): a note's
/// `tuplets` index this list.
pub(super) fn tuplet_list(score: &Score) -> Vec<(usize, u8, corinzia_score::score::Tuplet)> {
    let mut out = Vec::new();
    for (pi, part) in score.parts.iter().enumerate() {
        for v in &part.voices {
            let mut ts = v.tuplets.clone();
            ts.sort_by(|a, b| a.start.cmp(&b.start).then(b.length.cmp(&a.length)));
            out.extend(ts.into_iter().filter(|t| t.actual > 0 && t.normal > 0 && t.length > Frac::ZERO).map(|t| (pi, v.index, t)));
        }
    }
    out
}

pub(super) fn split(score: &Score, ms: &[Measure]) -> Vec<Note> {
    let tuplets = tuplet_list(score);
    let clefs_at = |part: &Part, at: Frac| -> String {
        part.clefs.iter().filter(|c| c.at <= at).max_by(|a, b| a.at.cmp(&b.at)).map(|c| c.clef.clone()).unwrap_or(part.clef.clone())
    };
    let mut out = Vec::new();
    for (pi, part) in score.parts.iter().enumerate() {
        for v in &part.voices {
            for e in &v.events {
                let end = e.start + e.dur;
                let mut first = true;
                for (mi, m) in ms.iter().enumerate() {
                    let (a, b) = (e.start.max(m.start), end.min(m.start + m.len));
                    if a >= b {
                        continue;
                    }
                    let rest = e.pitches.is_empty();
                    if rest && m.multi > 1 {
                        continue; // the bar's own rest stands for them
                    }
                    // Under an octave line notes are written octaves from where they sound.
                    let ottava = part.lines.iter().find_map(|l| match l.kind {
                        LineKind::Octave(k) if a >= l.from && a < l.to => Some(k as i32 * 7),
                        _ => None,
                    }).unwrap_or(0);
                    // Inside tuplets the written value is the sounding one
                    // times each tuplet's actual/normal.
                    let around: Vec<usize> = (0..tuplets.len())
                        .filter(|&k| tuplets[k].0 == pi && tuplets[k].1 == v.index && a >= tuplets[k].2.start && a < tuplets[k].2.start + tuplets[k].2.length)
                        .collect();
                    let ratio = around.iter().fold(Frac::int(1), |r, &k| r * Frac::new(tuplets[k].2.actual, tuplets[k].2.normal));
                    let whole_bar = rest && around.is_empty() && a == m.start && b == m.start + m.len && m.phase.is_zero();
                    let values = if whole_bar {
                        vec![NoteValue { base: 1, dots: 0, triplet: false }]
                    } else if !around.is_empty() {
                        decompose((b - a) * ratio)
                    } else if rest {
                        decompose_rest(a - m.start + m.phase, b - a, beat_of(m.num, m.den))
                    } else {
                        decompose(b - a)
                    };
                    // Cross-staff: drawn on another staff of the player, in its clef.
                    let staff = part.cross.iter().find(|c| c.0 == e.id && !rest).map_or(pi, |c| (pi as i64 + c.1 as i64).clamp(0, score.parts.len() as i64 - 1) as usize);
                    let mut t = a;
                    for (k, nv) in values.iter().enumerate() {
                        let last_piece = k + 1 == values.len() && b == end;
                        let middle = clef_middle(&clefs_at(&score.parts[staff], t));
                        let mut heads: Vec<Head> = e
                            .pitches
                            .iter()
                            .map(|p| Head {
                                id: p.id,
                                pos: diatonic(p) - ottava - middle,
                                step: p.step,
                                alter: p.alter,
                                dia: diatonic(p) - ottava,
                                tie_out: !last_piece || p.tie,
                                tied_in: false,
                                accidental: None,
                                acc_x: 0.0,
                                shift: 0.0,
                                shape: part.head_shapes.iter().find(|s| s.0 == diatonic(p)).map_or(0, |s| s.1),
                            })
                            .collect();
                        heads.sort_by_key(|h| h.dia);
                        let graces: Vec<GraceGeom> = if first && k == 0 {
                            e.graces
                                .iter()
                                .map(|g| {
                                    let mut hs: Vec<Head> = g
                                        .pitches
                                        .iter()
                                        .map(|p| Head {
                                            id: p.id,
                                            pos: diatonic(p) - ottava - middle,
                                            step: p.step,
                                            alter: p.alter,
                                            dia: diatonic(p) - ottava,
                                            tie_out: false,
                                            tied_in: false,
                                            accidental: None,
                                            acc_x: 0.0,
                                            shift: 0.0,
                                            shape: part.head_shapes.iter().find(|s| s.0 == diatonic(p)).map_or(0, |s| s.1),
                                        })
                                        .collect();
                                    hs.sort_by_key(|h| h.dia);
                                    GraceGeom { id: g.id, heads: hs, base: g.base.max(8), slash: g.slash }
                                })
                                .collect()
                        } else {
                            vec![]
                        };
                        let dur = if whole_bar { m.len } else { nv.length() * Frac::new(ratio.den, ratio.num) };
                        out.push(Note {
                            part: staff,
                            home: pi,
                            voice: v.index,
                            measure: mi,
                            start: t,
                            dur,
                            value: *nv,
                            rest,
                            whole_bar,
                            id: e.id,
                            heads,
                            first,
                            art: e.art,
                            dynamic: e.dynamic.clone(),
                            lyrics: if first { e.lyrics.clone() } else { vec![] },
                            tuplets: around.clone(),
                            graces,
                            beam: if first && k == 0 { e.spans & span::BEAMS } else { 0 },
                            stem_up: true,
                            beamed: false,
                            stem_x: 0.0,
                            stem_end: 0.0,
                            x: 0.0,
                        });
                        first = false;
                        t = t + dur;
                    }
                }
            }
        }
    }
    // Voice 1 of a part with nothing in a bar gets a whole-bar rest (not
    // an audio track's staff: nothing is written there).
    for pi in (0..score.parts.len()).filter(|&pi| !score.parts[pi].audio) {
        for (mi, m) in ms.iter().enumerate() {
            if !out.iter().any(|n| (n.part == pi || n.home == pi) && n.measure == mi && n.voice == 1) {
                out.push(Note {
                    part: pi,
                    home: pi,
                    voice: 1,
                    measure: mi,
                    start: m.start,
                    dur: m.len,
                    value: NoteValue { base: 1, dots: 0, triplet: false },
                    rest: true,
                    whole_bar: true,
                    id: 0,
                    heads: vec![],
                    first: true,
                    art: 0,
                    dynamic: None,
                    lyrics: vec![],
                    tuplets: vec![],
                    graces: vec![],
                    beam: 0,
                    stem_up: true,
                    beamed: false,
                    stem_x: 0.0,
                    stem_end: 0.0,
                    x: 0.0,
                });
            }
        }
    }
    out.sort_by(|a, b| (a.part, a.voice, a.start).cmp(&(b.part, b.voice, b.start)));
    out
}

pub(super) fn rest_glyph(base: i64) -> Glyph {
    match base {
        0 | 1 => Glyph::RestWhole,
        2 => Glyph::RestHalf,
        4 => Glyph::RestQuarter,
        8 => Glyph::Rest8th,
        16 => Glyph::Rest16th,
        32 => Glyph::Rest32nd,
        _ => Glyph::Rest64th,
    }
}

pub(super) fn flag_glyph(base: i64, up: bool) -> Glyph {
    match (base, up) {
        (8, true) => Glyph::Flag8thUp,
        (8, false) => Glyph::Flag8thDown,
        (16, true) => Glyph::Flag16thUp,
        (16, false) => Glyph::Flag16thDown,
        (32, true) => Glyph::Flag32ndUp,
        (32, false) => Glyph::Flag32ndDown,
        (_, true) => Glyph::Flag64thUp,
        (_, false) => Glyph::Flag64thDown,
    }
}

/// Notes split and prepared (stems, accidentals) for the whole movement.
/// Runs of bars where nothing sounds and nothing changes become one bar
/// standing for them all (LilyPond's compressed multi-measure rests): a
/// run breaks at a key, time, tempo, rehearsal mark, barline, ending,
/// jump, clef change, text or line.
pub(super) fn compress_empty(score: &Score, ms: Vec<Measure>) -> Vec<Measure> {
    let empty = |m: &Measure| {
        let (a, b) = (m.start, m.start + m.len);
        let inside = |t: Frac| t >= a && t < b;
        m.phase.is_zero()
            && score.parts.iter().all(|p| {
                p.voices.iter().all(|v| v.events.iter().all(|e| e.pitches.is_empty() || e.start + e.dur <= a || e.start >= b))
                    && !p.texts.iter().any(|t| inside(t.at))
                    && !p.chords.iter().any(|c| inside(c.at))
                    && !p.clefs.iter().any(|c| inside(c.at) && c.at > Frac::ZERO)
                    && !p.lines.iter().any(|l| l.from < b && l.to > a)
            })
    };
    // Nothing may happen where bar `m` joins the one before.
    let joins = |m: &Measure| {
        let t = m.start;
        m.key.is_none()
            && !m.show_time
            && !score.barlines.iter().any(|x| x.at == t)
            && !score.marks.contains(&t)
            && !score.tempo.iter().any(|x| x.at == t)
            && !score.navigation.iter().any(|x| x.at == t)
            && !score.endings.iter().any(|e| e.from <= t && e.to >= t)
            && !score.breaks.iter().any(|b| b.0 == t)
    };
    let mut out: Vec<Measure> = Vec::with_capacity(ms.len());
    for m in ms {
        if let Some(last) = out.last_mut() {
            let last_empty = last.multi > 1 || empty(last);
            if last_empty && empty(&m) && joins(&m) && !score.endings.iter().any(|e| e.from < m.start + m.len && e.to > last.start) {
                last.len = last.len + m.len;
                last.multi += 1;
                last.last = m.last;
                continue;
            }
        }
        out.push(m);
    }
    out
}

pub(super) fn prepared(score: &Score, opt: &Options) -> (Vec<Measure>, Vec<Note>) {
    let mut ms = measures(score);
    if opt.multirest {
        ms = compress_empty(score, ms);
    }
    let notes = split(score, &ms);
    let shortest = common_shortest(&ms, &notes);
    for m in ms.iter_mut() {
        m.shortest = shortest;
    }
    let mut e = Engraver::new(score, opt, ms, notes, true, None);
    e.prepare();
    (e.ms, e.notes)
}
