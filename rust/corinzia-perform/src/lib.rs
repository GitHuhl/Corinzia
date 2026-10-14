//! The performance of a movement: what a player would do with the written
//! music. Repeats and endings are unrolled, ties merged, ornaments written
//! out, articulations become gate times and dynamics/hairpins velocities.
//! Positions are *performed* quarters from the movement start.
//!
//! The Rust port of the C++ `compile::perform` family; it feeds MIDI export
//! (`document`) now and the Rust audio engine later.

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

/// ppp..fff -> 0..1; no dynamic = mf.
pub fn velocity_for_dynamic(mark: Option<u8>) -> f32 {
    const TABLE: [f32; 8] = [0.16, 0.26, 0.38, 0.5, 0.62, 0.75, 0.87, 1.0];
    TABLE[mark.map(|m| m.min(7) as usize).unwrap_or(4)]
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
        for e in ends.iter().filter(|e| e.from >= from && e.from < end) {
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
            return top.max(2);
        }
        2
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
    out
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EventShape {
    pub velocity: f32,
    pub legato: bool,
}

/// Velocity from dynamics with hairpins interpolated over time (towards the
/// next dynamic, or ±2 steps), and legato inside slurs (but the last note).
pub fn expressive_shape(evs: &[Event]) -> Vec<EventShape> {
    let mut out = vec![EventShape { velocity: 0.0, legato: false }; evs.len()];
    let mut dyn_ = None;
    let mut level = vec![4i32; evs.len()];
    let mut slur = false;
    for (i, e) in evs.iter().enumerate() {
        if e.dynamic.is_some() {
            dyn_ = e.dynamic;
        }
        level[i] = dyn_.map(|d| d as i32).unwrap_or(4);
        out[i].velocity = velocity_for_dynamic(dyn_);
        if e.spans & span::SLUR_START != 0 {
            slur = true;
        }
        out[i].legato = slur && e.spans & span::SLUR_END == 0;
        if e.spans & span::SLUR_END != 0 {
            slur = false;
        }
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
        let mut target = (level[i] + if cresc { 2 } else { -2 }).clamp(0, 7);
        if let Some(d) = evs[j..].iter().find_map(|e| e.dynamic) {
            target = d as i32; // the next written dynamic wins
        }
        let (v0, v1) = (velocity_for_dynamic(Some(level[i] as u8)), velocity_for_dynamic(Some(target as u8)));
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
    let mut notes = Vec::new();
    for (si, &(a, b)) in order.iter().enumerate() {
        let base = spans[si].performed_q;
        let to_q = |f: Frac| base + q(f - a);
        for lane in &m.lanes {
            for voice in &lane.voices {
                let evs = &voice.events;
                let mut consumed: HashSet<Id> = HashSet::new(); // tie continuations
                let shape = expressive_shape(evs);
                for (ei, ev) in evs.iter().enumerate() {
                    if ev.is_rest() || ev.start < a || ev.start >= b {
                        continue;
                    }
                    let Some(inst) = lane.instrument_at(ev.start) else { continue };
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
                        let figure = ornament_figure(h.pitch, end - ev.start, ev.articulations, key_at(ev.start));
                        let n = figure.len();
                        for (fi, f) in figure.into_iter().enumerate() {
                            notes.push(PerformedNote {
                                instrument: inst,
                                element: h.id,
                                key: f.pitch.midi_key(),
                                velocity: vel.min(1.0),
                                on_q: to_q(ev.start + f.offset),
                                full_off_q: to_q(ev.start + f.offset + f.length),
                                gate: if fi + 1 < n { 0.95 } else { gate },
                                roll_sec: if fi == 0 { roll } else { 0.0 },
                            });
                        }
                    }
                }
            }
        }
    }
    Performance { order, spans, tempo_points, meter_points, tempo, length_q: performed, notes }
}

/// The export document of one movement (notation + performance), as the
/// C++ `exporter::exportDocument` built it: one MIDI track per DAW track
/// that sounds, in mixer order; gate and roll applied in time.
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
            Some(doc::Track {
                name: t.name.clone(),
                channel: t.midi_channel & 15,
                notes: notes
                    .iter()
                    .map(|n| {
                        let on = perf.tempo.seconds_at(n.on_q) + n.roll_sec;
                        let off = on + (perf.tempo.seconds_at(n.full_off_q) - on) * n.gate;
                        doc::Note { on: perf.tempo.quarter_at(on), off: perf.tempo.quarter_at(off), key: n.key.clamp(0, 127) as u8, vel: n.velocity }
                    })
                    .collect(),
            })
        })
        .collect();
    doc::Document {
        version: 1,
        title: project.title.clone(),
        composer: project.composer.clone(),
        score: corinzia_model::notation::score(project, m),
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
