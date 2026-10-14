//! Standard MIDI File (format 1) from the performance: a conductor track
//! (title, tempo map, time signatures) and one track per DAW track.

use crate::doc::{Document, PerfTempo};

pub const PPQ: u32 = 960;
/// Tempo ramps are written as steps of this many ticks (a 16th note).
const RAMP_STEP: u32 = PPQ / 4;

fn ticks(q: f64) -> u32 {
    (q.max(0.0) * PPQ as f64).round() as u32
}

fn vlq(out: &mut Vec<u8>, mut v: u32) {
    let mut buf = [0u8; 5];
    let mut n = 0;
    loop {
        buf[n] = (v & 0x7f) as u8;
        n += 1;
        v >>= 7;
        if v == 0 {
            break;
        }
    }
    for i in (0..n).rev() {
        out.push(buf[i] | if i > 0 { 0x80 } else { 0 });
    }
}

/// (tick, order, bytes): `order` keeps note-offs before note-ons at one tick.
type Ev = (u32, u8, Vec<u8>);

fn chunk(events: &mut Vec<Ev>, end: u32) -> Vec<u8> {
    events.sort_by_key(|e| (e.0, e.1));
    let mut data = Vec::new();
    let mut last = 0;
    for (t, _, bytes) in events.iter() {
        vlq(&mut data, t - last);
        data.extend_from_slice(bytes);
        last = *t;
    }
    vlq(&mut data, end.saturating_sub(last));
    data.extend_from_slice(&[0xff, 0x2f, 0x00]); // end of track
    let mut out = b"MTrk".to_vec();
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend(data);
    out
}

fn meta_text(kind: u8, text: &str) -> Vec<u8> {
    let mut v = vec![0xff, kind];
    vlq(&mut v, text.len() as u32);
    v.extend_from_slice(text.as_bytes());
    v
}

fn tempo_event(bpm: f64) -> Vec<u8> {
    let us = (60_000_000.0 / bpm.clamp(1.0, 1000.0)).round() as u32;
    vec![0xff, 0x51, 0x03, (us >> 16) as u8, (us >> 8) as u8, us as u8]
}

/// Tempo events for the map; linear ramps (in bpm over quarters) are stepped.
fn tempo_events(tempo: &[PerfTempo], length_q: f64) -> Vec<Ev> {
    let mut out = Vec::new();
    for (i, t) in tempo.iter().enumerate() {
        let next = tempo.get(i + 1);
        out.push((ticks(t.q), 1, tempo_event(t.bpm)));
        if let (true, Some(n)) = (t.ramp, next) {
            let (a, b) = (ticks(t.q), ticks(n.q.min(length_q)));
            let mut k = a + RAMP_STEP;
            while k < b {
                let f = (k - a) as f64 / (b - a) as f64;
                out.push((k, 1, tempo_event(t.bpm + (n.bpm - t.bpm) * f)));
                k += RAMP_STEP;
            }
        }
    }
    out
}

pub fn write(doc: &Document) -> Result<Vec<u8>, String> {
    let perf = &doc.performance;
    if perf.tempo.is_empty() {
        return Err("performance has no tempo".into());
    }
    let end = ticks(perf.length_q);
    let mut file = b"MThd".to_vec();
    file.extend_from_slice(&6u32.to_be_bytes());
    file.extend_from_slice(&1u16.to_be_bytes()); // format 1
    file.extend_from_slice(&((perf.tracks.len() + 1) as u16).to_be_bytes());
    file.extend_from_slice(&(PPQ as u16).to_be_bytes());

    // Conductor track.
    let mut conductor: Vec<Ev> = vec![(0, 0, meta_text(0x03, &doc.title))];
    conductor.extend(tempo_events(&perf.tempo, perf.length_q));
    for m in &perf.meters {
        let dd = (m.den.max(1) as f64).log2().round() as u8;
        conductor.push((ticks(m.q), 1, vec![0xff, 0x58, 0x04, m.num.clamp(1, 255) as u8, dd, 24, 8]));
    }
    file.extend(chunk(&mut conductor, end));

    for tr in &perf.tracks {
        let ch = tr.channel & 0x0f;
        let mut evs: Vec<Ev> = vec![(0, 0, meta_text(0x03, &tr.name))];
        for n in &tr.notes {
            let on = ticks(n.on);
            let off = ticks(n.off).max(on + 1);
            let vel = ((n.vel.clamp(0.0, 1.0) * 127.0).round() as u8).max(1);
            evs.push((on, 3, vec![0x90 | ch, n.key & 0x7f, vel]));
            evs.push((off, 2, vec![0x80 | ch, n.key & 0x7f, 0]));
        }
        let last = evs.iter().map(|e| e.0).max().unwrap_or(0);
        file.extend(chunk(&mut evs, end.max(last)));
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variable_length_quantities() {
        let enc = |v| {
            let mut o = Vec::new();
            vlq(&mut o, v);
            o
        };
        assert_eq!(enc(0), vec![0x00]);
        assert_eq!(enc(0x7f), vec![0x7f]);
        assert_eq!(enc(0x80), vec![0x81, 0x00]);
        assert_eq!(enc(960), vec![0x87, 0x40]);
        assert_eq!(enc(0x0fff_ffff), vec![0xff, 0xff, 0xff, 0x7f]);
    }

    #[test]
    fn ramps_are_stepped() {
        let t = [PerfTempo { q: 0.0, bpm: 60.0, ramp: true }, PerfTempo { q: 1.0, bpm: 120.0, ramp: false }];
        let evs = tempo_events(&t, 4.0);
        // 60 at 0, then 75, 90, 105 at each 16th, then 120 at the quarter.
        assert_eq!(evs.len(), 5);
        assert_eq!(evs[2].0, 480);
        assert_eq!(evs[2].2, tempo_event(90.0));
    }
}
