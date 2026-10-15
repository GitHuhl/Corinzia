//! A small built-in project, so the app opens with music (`corinzia`
//! without a file): two players over eight bars, written in the engraver's
//! test notation and converted into the model.

use corinzia_layout::notation;
use corinzia_model::*;
use corinzia_score::Frac;
use std::sync::Arc;

pub fn project() -> Project {
    let mut p = Project { title: "Serenade (demo)".into(), composer: "Corinzia".into(), sample_rate: 48000.0, next_id: 1, ..Default::default() };
    let staves = [
        ("Flute", "Fl.", "treble", "e''4(\\mp d''8 c'' b'4 c''8 d'' e''2 g''4 e'') d''4.( c''8 b'4 a' g'2.) r4 c'''4(\\f b''8 a'' g''4 e'' f''2 d''4 b') c''1\\fermata"),
        ("Cello", "Vc.", "bass", "c2\\p g, a, e, f, c f, g, c4( e g c') a,( c e a) f,2 g, c1\\fermata"),
    ];
    let mut lanes = Vec::new();
    let mut id = 1000u64;
    for (k, (name, abbr, clef, music)) in staves.iter().enumerate() {
        let track = 10 + k as u64;
        let player = 20 + k as u64;
        let inst = 30 + k as u64;
        p.tracks.push(Track { id: track, name: name.to_string(), instrument_node: None, note_port: 0, output_bus: None, midi_channel: k as u8, gain_db: 0.0, pan: 0.0, mute: false, solo: false });
        p.instruments.push(Instrument { id: inst, player, name: name.to_string(), engraver_name: abbr.to_string(), transposition: corinzia_model::Interval::default(), clef: clef.to_string(), track: Some(track) });
        p.players.push(Player { id: player, name: format!("Player {}", k + 1), instruments: vec![inst] });
        let v = notation::voice(music, 1, &mut id);
        let events = v
            .events
            .into_iter()
            .map(|e| Event {
                id: e.id,
                kind: if e.pitches.is_empty() { EventKind::Rest } else if e.pitches.len() > 1 { EventKind::Chord } else { EventKind::Note },
                start: e.start,
                duration: e.dur,
                heads: e
                    .pitches
                    .iter()
                    .map(|h| Head { id: h.id, pitch: Pitch { step: h.step as i8, alter: h.alter, octave: h.octave }, tie: h.tie, velocity: 0 })
                    .collect(),
                articulations: e.art,
                dynamic: e.dynamic.as_deref().and_then(|d| DYNAMICS.iter().position(|x| *x == d)).map(|i| i as u8),
                spans: e.spans,
                lyrics: e.lyrics,
                fingering: e.fingering,
                slur: e.slur,
                graces: e
                    .graces
                    .iter()
                    .map(|g| Grace {
                        id: g.id,
                        heads: g.pitches.iter().map(|h| Head { id: h.id, pitch: Pitch { step: h.step as i8, alter: h.alter, octave: h.octave }, tie: false, velocity: 0 }).collect(),
                        base: g.base,
                        slash: g.slash,
                    })
                    .collect(),
            })
            .collect();
        lanes.push(Lane {
            player,
            changes: vec![InstrumentChange { id: 40 + k as u64, at: Frac::ZERO, instrument: inst }],
            clefs: vec![],
            texts: if k == 0 { vec![TextItem { id: 50, at: Frac::ZERO, text: "dolce".into(), kind: TextKind::Expression }] } else { vec![] },
            chords: vec![],
            lines: vec![],
            voices: vec![Voice { id: 60 + k as u64, index: 1, events: Arc::new(events), tuplets: vec![] }],
        });
    }
    p.movements.push(Movement {
        id: 2,
        ordinal: 0,
        title: "I. Andante".into(),
        length: Frac::int(8),
        revision: 0,
        tempo: vec![Tempo { at: Frac::ZERO, bpm: 84.0, ramp: false, text: "Andante".into() }],
        meter: vec![Meter { at: Frac::ZERO, num: 4, den: 4, pickup: false }],
        keys: vec![Key { at: Frac::ZERO, fifths: 0, minor: false }],
        barlines: vec![Barline { id: 70, at: Frac::int(4), kind: BarKind::Double }],
        rehearsal: vec![RehearsalMark { id: 71, at: Frac::int(4) }],
        navigation: vec![],
        endings: vec![],
        lanes,
    });
    p.next_id = id + 1;
    p
}

/// A new, empty project: these instruments (name, abbreviation, clef), each
/// on its own player, over `bars` bars of 4/4 filled with whole-bar rests.
#[cfg_attr(not(test), allow(dead_code))]
pub fn blank(title: &str, instruments: &[(&str, &str, &str)], bars: i64) -> Project {
    let with: Vec<(&str, &str, &str, Interval)> = instruments.iter().map(|(a, b, c)| (*a, *b, *c, Interval::default())).collect();
    blank_with(title, &with, bars)
}

/// As [`blank`], with each instrument's transposition.
pub fn blank_with(title: &str, instruments: &[(&str, &str, &str, Interval)], bars: i64) -> Project {
    let mut p = Project { title: title.into(), sample_rate: 48000.0, next_id: 1, ..Default::default() };
    let mut lanes = Vec::new();
    for (k, (name, abbr, clefs, transposition)) in instruments.iter().enumerate() {
        // "treble+bass": a grand staff, one staff per clef.
        let clefs: Vec<&str> = clefs.split('+').collect();
        let (track, player, inst) = (p.allocate_id(), p.allocate_id(), p.allocate_id());
        p.tracks.push(Track { id: track, name: name.to_string(), instrument_node: None, note_port: 0, output_bus: None, midi_channel: (k % 16) as u8, gain_db: 0.0, pan: 0.0, mute: false, solo: false });
        p.instruments.push(Instrument { id: inst, player, name: name.to_string(), engraver_name: abbr.to_string(), transposition: *transposition, clef: clefs[0].to_string(), track: Some(track) });
        p.players.push(Player { id: player, name: name.to_string(), instruments: vec![inst] });
        for (si, clef) in clefs.iter().enumerate() {
            let rests: Vec<Event> = (0..bars)
                .map(|b| Event { id: p.allocate_id(), kind: EventKind::Rest, start: Frac::int(b), duration: Frac::int(1), heads: vec![], articulations: 0, dynamic: None, spans: 0, lyrics: vec![], graces: vec![], fingering: vec![], slur: Default::default() })
                .collect();
            let (change, voice) = (p.allocate_id(), p.allocate_id());
            let clef_changes = if si == 0 { vec![] } else { vec![ClefChange { id: p.allocate_id(), at: Frac::ZERO, clef: clef.to_string() }] };
            lanes.push(Lane {
                player,
                changes: vec![InstrumentChange { id: change, at: Frac::ZERO, instrument: inst }],
                clefs: clef_changes,
                texts: vec![],
                chords: vec![],
                lines: vec![],
                voices: vec![Voice { id: voice, index: 1, events: Arc::new(rests), tuplets: vec![] }],
            });
        }
    }
    let id = p.allocate_id();
    p.movements.push(Movement {
        id,
        ordinal: 0,
        title: String::new(),
        length: Frac::int(bars.max(1)),
        revision: 0,
        tempo: vec![Tempo { at: Frac::ZERO, bpm: 120.0, ramp: false, text: String::new() }],
        meter: vec![Meter { at: Frac::ZERO, num: 4, den: 4, pickup: false }],
        keys: vec![Key { at: Frac::ZERO, fifths: 0, minor: false }],
        barlines: vec![],
        rehearsal: vec![],
        navigation: vec![],
        endings: vec![],
        lanes,
    });
    p
}
