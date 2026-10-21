//! The performance of a movement: what a player would do with the written
//! music. Repeats and endings are unrolled, ties merged, ornaments written
//! out, articulations become gate times and dynamics/hairpins velocities.
//! Positions are *performed* quarters from the movement start.
//!
//! It feeds both MIDI export (`document`) and the audio engine
//! (`corinzia-audio`'s snapshots).

pub mod tempo;

use corinzia_edit as edit;
use corinzia_model::*;
use corinzia_score::score::{art, span};
use corinzia_score::Frac;
use std::collections::{BTreeMap, HashSet};
use tempo::{MeterPoint, TempoMap, TempoPoint};

fn q(f: Frac) -> f64 {
    f.num as f64 * 4.0 / f.den as f64
}

/// Fraction of the written length that sounds.
pub fn gate_factor(a: u16) -> f64 {
    if a & art::STACCATISSIMO != 0 {
        0.25
    } else if a & art::STACCATO != 0 {
        0.5
    } else if a & (art::BREATH | art::CAESURA) != 0 {
        0.7 // leave room to breathe
    } else if a & art::TENUTO != 0 {
        1.0
    } else {
        0.9
    }
}

/// A level (0 = ppp .. 7 = fff) -> 0..1; none = mf.
pub fn velocity_for_dynamic(level: Option<u8>) -> f32 {
    const TABLE: [f32; 8] = [0.16, 0.26, 0.38, 0.5, 0.62, 0.75, 0.87, 1.0];
    TABLE[level.map(|m| m.min(7) as usize).unwrap_or(4)]
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FigureNote {
    pub offset: Frac,
    pub length: Frac,
    pub pitch: Pitch,
}

/// An ornament written out: trills alternate with the upper neighbour (in
/// the key) in 32nds and end on the main note; turns, mordents, tremolos.
pub fn ornament_figure(main: Pitch, length: Frac, a: u16, key_fifths: i32) -> Vec<FigureNote> {
    let neighbour = |dir: i32| {
        let d = edit::diatonic(main) + dir;
        edit::from_diatonic(d, edit::key_alter(key_fifths, d.rem_euclid(7)))
    };
    let unit = Frac::new(1, 32).min(length * Frac::new(1, 4));
    let mut out: Vec<FigureNote> = Vec::new();
    let mut at = Frac::ZERO;
    let add = |out: &mut Vec<FigureNote>, at: &mut Frac, p: Pitch, len: Frac| {
        let len = len.min(length - *at);
        if len > Frac::ZERO {
            out.push(FigureNote { offset: *at, length: len, pitch: p });
        }
        *at = *at + len;
    };
    if a & art::TRILL != 0 {
        let upper = neighbour(1);
        let mut on_main = true;
        while length - at > unit {
            add(&mut out, &mut at, if on_main { main } else { upper }, unit);
            on_main = !on_main;
        }
        if !on_main && !out.is_empty() {
            let last = out.last_mut().unwrap();
            last.length = last.length + (length - at); // end on the main note
        } else {
            add(&mut out, &mut at, main, length); // the rest (clamped)
        }
    } else if a & art::TURN != 0 {
        add(&mut out, &mut at, neighbour(1), unit);
        add(&mut out, &mut at, main, unit);
        add(&mut out, &mut at, neighbour(-1), unit);
        add(&mut out, &mut at, main, length); // the rest (clamped)
    } else if a & art::MORDENT != 0 {
        add(&mut out, &mut at, main, unit);
        add(&mut out, &mut at, neighbour(-1), unit);
        add(&mut out, &mut at, main, length); // the rest (clamped)
    } else if a & art::TREMOLO != 0 {
        while length - at > unit {
            add(&mut out, &mut at, main, unit);
        }
        add(&mut out, &mut at, main, length); // the rest (clamped)
    } else {
        add(&mut out, &mut at, main, length);
    }
    out
}

/// Plays the movement like a performer: an end repeat jumps back to the last
/// start repeat (or the start) until its passes are done; an ending not
/// marked for the current pass is skipped, with the end repeat at its end. A
/// repeat with endings takes as many passes as its highest ending number;
/// without endings, two. Returns written `[from, to)` spans in order.
pub fn playback_order(m: &Movement) -> Vec<(Frac, Frac)> {
    let mut bars = m.barlines.clone();
    bars.sort_by_key(|b| b.at);
    let mut ends = m.endings.clone();
    ends.sort_by_key(|e| e.from);
    let len = m.length;
    let bar_at = |at: Frac| bars.iter().find(|b| b.at == at);
    let ending_at = |at: Frac| ends.iter().find(|e| e.from == at && e.from < e.to);
    let passes_for = |from: Frac, end: Frac| {
        // The first ending in the section and those chained after it.
        let Some(e) = ends.iter().find(|e| e.from >= from && e.from < end) else { return 2 };
        let mut top = 0;
        let mut c = Some(e);
        while let Some(x) = c {
            for n in 1..=8 {
                if x.passes & (1 << (n - 1)) != 0 {
                    top = top.max(n);
                }
            }
            c = ending_at(x.to);
        }
        top.max(2)
    };
    let is_end = |k: BarKind| matches!(k, BarKind::RepeatEnd | BarKind::RepeatBoth);
    let is_start = |k: BarKind| matches!(k, BarKind::RepeatStart | BarKind::RepeatBoth);

    let mut out: Vec<(Frac, Frac)> = Vec::new();
    let flush = |out: &mut Vec<(Frac, Frac)>, seg_start: Frac, to: Frac| {
        if to > seg_start {
            match out.last_mut() {
                Some(last) if last.1 == seg_start => last.1 = to,
                _ => out.push((seg_start, to)),
            }
        }
    };
    let none = Frac::int(-1);
    let (mut cur, mut seg_start, mut repeat_from, mut reset_at) = (Frac::ZERO, Frac::ZERO, Frac::ZERO, none);
    let mut pass = 1;
    let (mut jumped, mut skipped) = (false, false);
    let mut guard = 0;
    while cur <= len && guard < 100_000 {
        guard += 1;
        let bar = bar_at(cur);
        if cur == reset_at && !skipped {
            pass = 1;
            repeat_from = cur;
            reset_at = none;
        }
        if let Some(b) = bar {
            if !jumped && !skipped && is_end(b.kind) && cur > repeat_from {
                if pass < passes_for(repeat_from, cur) {
                    flush(&mut out, seg_start, cur);
                    pass += 1;
                    cur = repeat_from;
                    seg_start = repeat_from;
                    jumped = true;
                    continue;
                }
                pass = 1;
                repeat_from = cur;
            }
            if !jumped && is_start(b.kind) {
                repeat_from = cur;
                pass = 1;
            }
        }
        if let Some(e) = ending_at(cur) {
            if e.passes & (1 << (pass - 1).min(7)) == 0 {
                flush(&mut out, seg_start, cur);
                cur = e.to.min(len);
                seg_start = cur;
                skipped = true;
                jumped = false;
                if ending_at(cur).is_none() {
                    reset_at = cur;
                    pass = 1;
                    repeat_from = cur;
                }
                continue;
            }
            if !bar_at(e.to).is_some_and(|b| is_end(b.kind)) {
                reset_at = e.to; // an ending that does not repeat is the last
            }
        }
        if cur >= len {
            break;
        }
        let mut next = len;
        if let Some(b) = bars.iter().find(|b| b.at > cur) {
            next = next.min(b.at);
        }
        for e in &ends {
            if e.from > cur {
                next = next.min(e.from);
            }
            if e.to > cur {
                next = next.min(e.to);
            }
        }
        if reset_at > cur {
            next = next.min(reset_at);
        }
        cur = next;
        jumped = false;
        skipped = false;
    }
    flush(&mut out, seg_start, len);
    if out.is_empty() {
        out.push((Frac::ZERO, len));
    }
    navigate(m, out)
}

/// `[from, to)` played straight through, as after a D.C. or D.S.: repeats
/// are not taken, and of each run of endings only the last is played.
fn straight(m: &Movement, from: Frac, to: Frac) -> Vec<(Frac, Frac)> {
    let mut cut: Vec<(Frac, Frac)> = m
        .endings
        .iter()
        .filter(|e| e.from >= from && e.from < to && m.endings.iter().any(|n| n.from == e.to))
        .map(|e| (e.from, e.to.min(to)))
        .collect();
    cut.sort();
    let mut out: Vec<(Frac, Frac)> = Vec::new();
    let mut at = from;
    for (a, b) in cut {
        if a > at {
            out.push((at, a));
        }
        at = at.max(b);
    }
    if at < to {
        out.push((at, to));
    }
    out
}

/// D.C. and D.S.: the first time the jump is reached, play on from the
/// start (or the segno) straight through, to Fine, or to "To Coda" and on
/// from the coda sign to the end.
fn navigate(m: &Movement, first: Vec<(Frac, Frac)>) -> Vec<(Frac, Frac)> {
    let Some(jump) = m.navigation.iter().find(|n| matches!(n.kind, Nav::DaCapo(_) | Nav::DalSegno(_))) else { return first };
    let mut out = Vec::new();
    let mut reached = false;
    for &(a, b) in &first {
        if a < jump.at && jump.at <= b {
            out.push((a, jump.at));
            reached = true;
            break;
        }
        out.push((a, b));
    }
    if !reached {
        return first;
    }
    let (target, to) = match jump.kind {
        Nav::DaCapo(to) => (Frac::ZERO, to),
        Nav::DalSegno(to) => (m.navigation.iter().filter(|n| n.kind == Nav::Segno && n.at < jump.at).map(|n| n.at).last().unwrap_or(Frac::ZERO), to),
        _ => unreachable!(),
    };
    let find = |kind: Nav| m.navigation.iter().find(|n| n.kind == kind && n.at > target).map(|n| n.at);
    match to {
        NavTo::End => out.extend(straight(m, target, m.length)),
        NavTo::Fine => out.extend(straight(m, target, find(Nav::Fine).unwrap_or(m.length))),
        NavTo::Coda => match (find(Nav::ToCoda), m.navigation.iter().find(|n| n.kind == Nav::Coda).map(|n| n.at)) {
            (Some(t), Some(c)) => {
                out.extend(straight(m, target, t));
                out.extend(straight(m, c, m.length));
            }
            _ => out.extend(straight(m, target, m.length)),
        },
    }
    // Join what runs on.
    let mut joined: Vec<(Frac, Frac)> = Vec::new();
    for (a, b) in out {
        match joined.last_mut() {
            Some(l) if l.1 == a => l.1 = b,
            _ if b > a => joined.push((a, b)),
            _ => {}
        }
    }
    joined
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EventShape {
    pub velocity: f32,
    pub legato: bool,
}

/// Velocity from dynamics with hairpins interpolated over time (towards the
/// next dynamic, or ±2 steps), and legato inside slurs (but the last note).
/// `slurs`: the [first note, last note) times of the slurs in the voice.
pub fn expressive_shape(evs: &[Event], slurs: &[(Frac, Frac)]) -> Vec<EventShape> {
    let mut out = vec![EventShape { velocity: 0.0, legato: false }; evs.len()];
    // The level each event is played at, and the level the music is at after it.
    let mut cur = 4u8;
    let mut attack = vec![4u8; evs.len()];
    let mut after = vec![4u8; evs.len()];
    for (i, e) in evs.iter().enumerate() {
        let a = match e.dynamic {
            Some(d) => {
                let (a, next) = dynamic_levels(d, cur);
                cur = next;
                a
            }
            None => cur,
        };
        attack[i] = a;
        after[i] = cur;
        out[i].velocity = velocity_for_dynamic(Some(a));
        out[i].legato = slurs.iter().any(|&(from, to)| from <= e.start && e.start < to);
    }
    for i in 0..evs.len() {
        let (cresc, dim) = (evs[i].spans & span::CRESC_START != 0, evs[i].spans & span::DIM_START != 0);
        if !cresc && !dim {
            continue;
        }
        let mut j = i + 1;
        while j < evs.len() && evs[j].spans & span::HAIRPIN_END == 0 && evs[j].dynamic.is_none() {
            j += 1;
        }
        let j = j.min(evs.len() - 1);
        let mut target = (after[i] as i32 + if cresc { 2 } else { -2 }).clamp(0, 7) as u8;
        if let Some((k, d)) = evs.iter().enumerate().skip(j).find_map(|(k, e)| e.dynamic.map(|d| (k, d))) {
            target = dynamic_levels(d, after[k.saturating_sub(1)]).0; // the next written dynamic wins
        }
        let (v0, v1) = (velocity_for_dynamic(Some(attack[i])), velocity_for_dynamic(Some(target)));
        let (t0, t1) = (q(evs[i].start), q(evs[j].start));
        for k in i..=j {
            let f = if t1 > t0 { (q(evs[k].start) - t0) / (t1 - t0) } else { 1.0 };
            out[k].velocity = v0 + (v1 - v0) * f as f32;
        }
        for k in j + 1..evs.len() {
            if evs[k].dynamic.is_some() {
                break;
            }
            out[k].velocity = v1;
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
pub struct PerformedNote {
    pub instrument: Id, // sounding instrument at the onset
    pub element: Id,    // the notehead
    pub key: i32,
    pub velocity: f32, // 0..1
    pub on_q: f64,
    pub full_off_q: f64, // the written end, before the gate
    pub gate: f64,
    pub roll_sec: f64, // arpeggio delay
    /// The playing technique in force (the last technique text, canonical:
    /// "pizz.", "arco", "con sord."…), and the note's own articulation
    /// ("staccato", "legato"…), for expression maps.
    pub technique: Option<&'static str>,
    pub articulation: Option<&'static str>,
}

/// Technique texts Corinzia understands, by how they begin: (prefix, canonical).
pub const TECHNIQUES: [(&str, &str); 16] = [
    ("pizz", "pizz."),
    ("arco", "arco"),
    ("con sord", "con sord."),
    ("senza sord", "senza sord."),
    ("mute", "con sord."),
    ("open", "senza sord."),
    ("sul pont", "sul pont."),
    ("sul tasto", "sul tasto"),
    ("col legno", "col legno"),
    ("ord", "ord."),
    ("nat", "ord."),
    ("norm", "ord."),
    ("trem", "trem."),
    ("flz", "flutter"),
    ("flutter", "flutter"),
    ("harm", "harmonics"),
];

/// Note articulations an expression map can answer, by priority.
pub const ARTICULATIONS: [&str; 8] = ["staccatissimo", "staccato", "marcato", "accent", "tenuto", "tremolo", "trill", "legato"];

/// A technique text's canonical name ("Pizzicato" → "pizz.").
pub fn technique_of(text: &str) -> Option<&'static str> {
    let t = text.trim().to_lowercase();
    TECHNIQUES.iter().find(|(p, _)| t.starts_with(p)).map(|x| x.1)
}

/// A note's articulation for expression maps (the strongest it has).
fn articulation_of(arts: u16, legato: bool) -> Option<&'static str> {
    [(art::STACCATISSIMO, 0), (art::STACCATO, 1), (art::MARCATO, 2), (art::ACCENT, 3), (art::TENUTO, 4), (art::TREMOLO, 5), (art::TRILL, 6)]
        .iter()
        .find(|(bit, _)| arts & bit != 0)
        .map(|(_, i)| ARTICULATIONS[*i])
        .or(legato.then_some("legato"))
}

/// A written span placed on the performed timeline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    pub performed_q: f64,
    pub written_q: f64,
    pub length_q: f64,
}

#[derive(Clone, Debug)]
pub struct Performance {
    pub order: Vec<(Frac, Frac)>,
    pub spans: Vec<Span>,
    pub tempo_points: Vec<TempoPoint>,
    pub meter_points: Vec<MeterPoint>,
    pub tempo: TempoMap,
    pub length_q: f64,
    pub notes: Vec<PerformedNote>,
}

impl Performance {
    /// Performed -> written quarters (for the playhead).
    pub fn written_quarter(&self, performed: f64) -> f64 {
        let s = self.spans.iter().filter(|s| s.performed_q <= performed + 1e-12).last().copied().unwrap_or(Span { performed_q: 0.0, written_q: 0.0, length_q: 0.0 });
        s.written_q + (performed - s.performed_q)
    }
}

pub fn perform(m: &Movement) -> Performance {
    let order = playback_order(m);
    // Tempo and meter maps over performed time.
    let (mut tempo_points, mut meter_points, mut spans) = (Vec::new(), Vec::new(), Vec::new());
    let mut performed = 0.0;
    for &(a, b) in &order {
        if let Some(t0) = m.tempo.iter().filter(|t| t.at <= a).last().or(m.tempo.first()) {
            tempo_points.push(TempoPoint { quarter: performed, bpm: t0.bpm, ramp_to_next: t0.ramp && t0.at == a });
        }
        for t in m.tempo.iter().filter(|t| t.at > a && t.at < b) {
            tempo_points.push(TempoPoint { quarter: performed + q(t.at - a), bpm: t.bpm, ramp_to_next: t.ramp });
        }
        if let Some(m0) = m.meter.iter().filter(|x| x.at <= a).last().or(m.meter.first()) {
            meter_points.push(MeterPoint { quarter: performed, num: m0.num, den: m0.den });
        }
        for x in m.meter.iter().filter(|x| x.at > a && x.at < b) {
            meter_points.push(MeterPoint { quarter: performed + q(x.at - a), num: x.num, den: x.den });
        }
        spans.push(Span { performed_q: performed, written_q: q(a), length_q: q(b - a) });
        performed += q(b - a);
    }
    let tempo = TempoMap::new(&tempo_points, &meter_points);
    let key_at = |at: Frac| m.keys.iter().filter(|k| k.at <= at).last().map(|k| k.fifths).unwrap_or(0);
    // Each voice's slurs, as times (a slur between voices is in both).
    let places = m.note_places();
    let mut voice_slurs: std::collections::HashMap<(usize, usize), Vec<(Frac, Frac)>> = Default::default();
    for sl in &m.slurs {
        if let (Some(a), Some(b)) = (places.get(&sl.from), places.get(&sl.to)) {
            voice_slurs.entry((a.0, a.1)).or_default().push((a.2, b.2));
            if (a.0, a.1) != (b.0, b.1) {
                voice_slurs.entry((b.0, b.1)).or_default().push((a.2, b.2));
            }
        }
    }
    let mut notes = Vec::new();
    for (si, &(a, b)) in order.iter().enumerate() {
        let base = spans[si].performed_q;
        let to_q = |f: Frac| base + q(f - a);
        for (li, lane) in m.lanes.iter().enumerate() {
            for (vi, voice) in lane.voices.iter().enumerate() {
                let evs = &voice.events;
                let mut consumed: HashSet<Id> = HashSet::new(); // tie continuations
                let shape = expressive_shape(evs, voice_slurs.get(&(li, vi)).map(Vec::as_slice).unwrap_or(&[]));
                for (ei, ev) in evs.iter().enumerate() {
                    if ev.is_rest() || ev.articulations & art::CUE != 0 || ev.start < a || ev.start >= b {
                        continue;
                    }
                    let Some(inst) = lane.instrument_at(ev.start) else { continue };
                    // The technique in force: the player's last technique text.
                    let technique = m
                        .lanes
                        .iter()
                        .filter(|o| o.player == lane.player)
                        .flat_map(|o| o.texts.iter())
                        .filter(|t| t.kind == TextKind::Technique && t.at <= ev.start)
                        .filter_map(|t| Some((t.at, technique_of(&t.text)?)))
                        .max_by_key(|x| x.0)
                        .map(|x| x.1);
                    let articulation = articulation_of(ev.articulations, shape[ei].legato);
                    // Grace notes play on the beat and take their time from
                    // the note: a 32nd each (a 64th for a slashed one), never
                    // more than half the note.
                    let vel_main = shape[ei].velocity;
                    let mut steal = Frac::ZERO;
                    if !ev.graces.is_empty() {
                        let each = if ev.graces.iter().all(|g| g.slash) { Frac::new(1, 64) } else { Frac::new(1, 32) };
                        let total = (each * Frac::int(ev.graces.len() as i64)).min(ev.duration * Frac::new(1, 2));
                        let one = total * Frac::new(1, ev.graces.len() as i64);
                        for (gi, g) in ev.graces.iter().enumerate() {
                            let on = ev.start + one * Frac::int(gi as i64);
                            for h in &g.heads {
                                notes.push(PerformedNote {
                                    instrument: inst,
                                    element: h.id,
                                    key: h.pitch.midi_key(),
                                    velocity: (vel_main * 0.9).min(1.0),
                                    on_q: to_q(on),
                                    full_off_q: to_q(on + one),
                                    gate: 0.95,
                                    roll_sec: 0.0,
                                    technique,
                                    articulation: None,
                                });
                            }
                        }
                        steal = total;
                    }
                    for h in &ev.heads {
                        if consumed.contains(&h.id) {
                            continue;
                        }
                        // Follow the tie chain (cut at the end of this pass).
                        let mut end = ev.end();
                        let mut last_art = ev.articulations;
                        let mut tied = h.tie;
                        let mut k = ei + 1;
                        while tied && k < evs.len() && evs[k].start < b {
                            tied = false;
                            if let Some(h2) = evs[k].heads.iter().find(|h2| h2.pitch.midi_key() == h.pitch.midi_key()) {
                                consumed.insert(h2.id);
                                end = evs[k].end();
                                last_art = evs[k].articulations;
                                tied = h2.tie;
                            }
                            k += 1;
                        }
                        let end = end.min(b);
                        let gate = if shape[ei].legato && last_art & (art::STACCATO | art::STACCATISSIMO) == 0 { 1.0 } else { gate_factor(last_art) };
                        let vel = if h.velocity > 0 {
                            h.velocity as f32 / 127.0
                        } else {
                            shape[ei].velocity * if ev.articulations & art::ACCENT != 0 { 1.15 } else { 1.0 }
                        };
                        // A rolled chord starts each head a little later, bottom up.
                        let mut roll = 0.0;
                        if ev.articulations & art::ARPEGGIO != 0 && ev.heads.len() > 1 {
                            let rank = ev.heads.iter().filter(|o| o.pitch.midi_key() < h.pitch.midi_key()).count();
                            roll = (rank as f64 * 0.03).min((tempo.seconds_at(to_q(end)) - tempo.seconds_at(to_q(ev.start))) / 2.0);
                        }
                        let figure = ornament_figure(h.pitch, end - ev.start - steal, ev.articulations, key_at(ev.start));
                        let n = figure.len();
                        // The sustain pedal (on any of the player's staves) held
                        // when the note is let go keeps it sounding until it lifts.
                        let held = m
                            .lanes
                            .iter()
                            .filter(|o| o.player == lane.player)
                            .flat_map(|o| o.lines.iter())
                            .find(|p| p.kind == LineKind::Pedal && p.from < end && end < p.to)
                            .map(|p| p.to.min(b));
                        for (fi, f) in figure.into_iter().enumerate() {
                            notes.push(PerformedNote {
                                instrument: inst,
                                element: h.id,
                                key: f.pitch.midi_key(),
                                velocity: vel.min(1.0),
                                on_q: to_q(ev.start + steal + f.offset),
                                full_off_q: match held {
                                    Some(t) if fi + 1 == n => to_q(t),
                                    _ => to_q(ev.start + steal + f.offset + f.length),
                                },
                                gate: if fi + 1 < n { 0.95 } else if held.is_some() { 1.0 } else { gate },
                                roll_sec: if fi == 0 { roll } else { 0.0 },
                                technique,
                                articulation,
                            });
                        }
                    }
                }
            }
        }
    }
    Performance { order, spans, tempo_points, meter_points, tempo, length_q: performed, notes }
}

/// The export document of one movement (notation + performance): one MIDI
/// track per DAW track that sounds, in mixer order; gate and roll applied in
/// time, as in playback.
pub fn document(project: &Project, m: &Movement) -> corinzia_export::doc::Document {
    use corinzia_export::doc;
    let perf = perform(m);
    let mut by_track: BTreeMap<Id, Vec<&PerformedNote>> = BTreeMap::new();
    for n in &perf.notes {
        if let Some(t) = project.instrument(n.instrument).and_then(|i| i.track) {
            by_track.entry(t).or_default().push(n);
        }
    }
    let tracks = project
        .tracks
        .iter()
        .filter_map(|t| {
            let notes = by_track.get(&t.id)?;
            // The expression map's switches, just before the notes that need them.
            let mut sorted: Vec<&PerformedNote> = notes.to_vec();
            sorted.sort_by(|a, b| a.on_q.total_cmp(&b.on_q));
            let (mut programs, mut switch_ccs, mut keyswitches) = (Vec::new(), Vec::new(), Vec::new());
            for (n, e) in sorted.iter().zip(expression(t, &sorted)) {
                let before = perf.tempo.quarter_at((perf.tempo.seconds_at(n.on_q) - 0.003).max(0.0));
                for a in e.before {
                    match a {
                        corinzia_model::SwitchAction::Program(p) => programs.push((before, p)),
                        corinzia_model::SwitchAction::Cc(cc, v) => switch_ccs.push((before, cc, v)),
                        corinzia_model::SwitchAction::Keyswitch(k) => keyswitches.push(doc::Note { on: before, off: perf.tempo.quarter_at((perf.tempo.seconds_at(n.on_q) - 0.001).max(0.0)), key: k, vel: 0.8 }),
                        corinzia_model::SwitchAction::Channel(_) => {}
                    }
                }
            }
            Some(doc::Track {
                name: t.name.clone(),
                channel: t.midi_channel & 15,
                program: t.program.map(|p| (p, t.bank)),
                // Dynamics as a controller: its value at each onset where it changes.
                controls: match t.dynamics_cc {
                    Some(cc) => {
                        let mut last = None;
                        notes
                            .iter()
                            .filter_map(|n| {
                                let v = (n.velocity.clamp(0.0, 1.0) * 127.0).round() as u8;
                                (last.replace(v) != Some(v)).then(|| (perf.tempo.quarter_at((perf.tempo.seconds_at(n.on_q) - 0.002).max(0.0)), cc, v))
                            })
                            .collect()
                    }
                    None => vec![],
                }
                .into_iter()
                .chain(switch_ccs)
                .collect(),
                programs,
                notes: notes
                    .iter()
                    .map(|n| {
                        let on = perf.tempo.seconds_at(n.on_q) + n.roll_sec;
                        let off = on + (perf.tempo.seconds_at(n.full_off_q) - on) * n.gate;
                        doc::Note { on: perf.tempo.quarter_at(on), off: perf.tempo.quarter_at(off), key: project.sounding_key(n.instrument, n.key).clamp(0, 127) as u8, vel: n.velocity }
                    })
                    .chain(keyswitches)
                    .collect(),
            })
        })
        .collect();
    doc::Document {
        title: project.title.clone(),
        composer: project.composer.clone(),
        // Written pitch with <transpose>, as MusicXML expects.
        score: corinzia_model::notation::score_as(project, m, true),
        performance: doc::Performance {
            tempo: perf.tempo_points.iter().map(|t| doc::PerfTempo { q: t.quarter, bpm: t.bpm, ramp: t.ramp_to_next }).collect(),
            meters: perf.meter_points.iter().map(|x| doc::PerfMeter { q: x.quarter, num: x.num, den: x.den }).collect(),
            length_q: perf.length_q,
            tracks,
        },
    }
}

pub fn midi(project: &Project, m: &Movement) -> Result<Vec<u8>, String> {
    corinzia_export::midi::write(&document(project, m))
}

pub fn musicxml(project: &Project, m: &Movement) -> Result<String, String> {
    corinzia_export::musicxml::write(&document(project, m))
}

/// What a track's expression map asks for a note: switches to send just
/// before it (only when the sound changes), and the channel it plays on.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Expression {
    pub before: Vec<corinzia_model::SwitchAction>,
    pub channel: Option<u8>,
}

/// The expression map applied to a track's notes (in time order): each
/// note's switch is its articulation's if mapped, else its technique's,
/// else the map's "ord." (the normal sound), else nothing changes.
pub fn expression(track: &corinzia_model::Track, notes: &[&PerformedNote]) -> Vec<Expression> {
    use corinzia_model::SwitchAction;
    let map = |name: &str| track.techniques.iter().find(|t| t.technique.trim().eq_ignore_ascii_case(name) || technique_of(&t.technique) == Some(name)).map(|t| t.action);
    let mut current: Option<SwitchAction> = None;
    notes
        .iter()
        .map(|n| {
            let wanted = n.articulation.and_then(map).or_else(|| n.technique.and_then(map)).or_else(|| map("ord."));
            let mut e = Expression::default();
            match wanted {
                Some(SwitchAction::Channel(c)) => e.channel = Some(c),
                Some(a) if current != Some(a) => {
                    e.before.push(a);
                    current = Some(a);
                }
                _ => {}
            }
            e
        })
        .collect()
}
