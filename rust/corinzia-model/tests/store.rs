use corinzia_model::store::{load, save};
use corinzia_model::*;
use corinzia_score::Frac;
use std::sync::Arc;

fn sample() -> Project {
    let mut p = Project { title: "Serenade".into(), composer: "Anon.".into(), sample_rate: 48000.0, next_id: 100, ..Default::default() };
    p.tracks.push(Track { id: 1, name: "Flute".into(), instrument_node: None, note_port: 0, output_bus: None, midi_channel: 0, gain_db: -3.0, pan: 0.0, mute: false, solo: false, inserts: vec![], sends: vec![] });
    p.instruments.push(Instrument { id: 3, player: 2, name: "Flute".into(), engraver_name: "Fl.".into(), transposition: corinzia_model::Interval::default(), clef: "treble".into(), track: Some(1), drums: vec![], tuning: vec![] });
    p.players.push(Player { id: 2, name: "Player 1".into(), instruments: vec![3] });
    let ev = |id, s, d, step, tie| Event {
        id,
        kind: EventKind::Note,
        start: s,
        duration: d,
        heads: vec![Head { id: id + 1, pitch: Pitch { step, alter: if step == 3 { 1 } else { 0 }, octave: 5 }, tie, velocity: 0 }],
        articulations: 1,
        dynamic: Some(5),
        spans: 4,
        lyrics: vec![Lyric { verse: 1, text: "Glo".into(), hyphen: true, extend: false }, Lyric { verse: 2, text: "Ah".into(), hyphen: false, extend: true }],
        graces: vec![Grace { id: id + 100, heads: vec![Head { id: id + 101, pitch: Pitch { step: 1, alter: 0, octave: 5 }, tie: false, velocity: 0 }], base: 16, slash: true }],
        fingering: vec![3],
    };
    // Events across two storage chunks (chunk = 4 whole notes).
    let events = vec![ev(10, Frac::ZERO, Frac::new(1, 4), 0, true), ev(12, Frac::new(1, 4), Frac::new(3, 4), 3, false), ev(14, Frac::int(5), Frac::int(1), 4, false)];
    p.movements.push(Movement {
        id: 4,
        ordinal: 0,
        title: "I. Andante".into(),
        length: Frac::int(6),
        revision: 7,
        tempo: vec![Tempo { at: Frac::ZERO, bpm: 96.0, ramp: false, text: "Andante \"con moto\"".into() }],
        meter: vec![Meter { at: Frac::ZERO, num: 4, den: 4, pickup: false, beams: [0; 8] }, Meter { at: Frac::int(4), num: 7, den: 8, pickup: false, beams: [2, 2, 3, 0, 0, 0, 0, 0] }],
        keys: vec![Key { at: Frac::ZERO, fifths: 1, minor: false }],
        barlines: vec![Barline { id: 20, at: Frac::int(2), kind: BarKind::RepeatEnd }],
        rehearsal: vec![RehearsalMark { id: 21, at: Frac::int(1) }],
        endings: vec![Ending { id: 22, from: Frac::int(1), to: Frac::int(2), passes: 1 }],
        navigation: vec![NavMark { id: 30, at: Frac::int(1), kind: Nav::Segno }, NavMark { id: 31, at: Frac::int(5), kind: Nav::DalSegno(NavTo::Coda) }],
        slurs: vec![],
        lanes: vec![Lane {
            player: 2,
            changes: vec![InstrumentChange { id: 23, at: Frac::ZERO, instrument: 3 }],
            clefs: vec![ClefChange { id: 24, at: Frac::int(3), clef: "bass".into() }],
            texts: vec![TextItem { id: 25, at: Frac::ZERO, text: "dolce".into(), kind: TextKind::Expression, shown: Shown::Parts }],
            chords: vec![ChordSymbol { id: 77, at: Frac::new(1, 4), text: "F♯m7♭5/E".into() }],
            lines: vec![LineSpan { id: 40, from: Frac::ZERO, to: Frac::int(1), kind: LineKind::Octave(1) }],
            voices: vec![Voice { id: 26, index: 1, events: Arc::new(events), tuplets: vec![Tuplet { start: Frac::int(5), length: Frac::new(1, 4), actual: 3, normal: 2 }] }],
        },
        // The same player's second staff (a grand staff): its own clef, pedal and voice.
        Lane {
            player: 2,
            changes: vec![InstrumentChange { id: 41, at: Frac::ZERO, instrument: 3 }],
            clefs: vec![ClefChange { id: 42, at: Frac::ZERO, clef: "bass".into() }],
            texts: vec![],
            chords: vec![],
            lines: vec![LineSpan { id: 43, from: Frac::ZERO, to: Frac::int(2), kind: LineKind::Pedal }],
            voices: vec![Voice { id: 44, index: 1, events: Arc::new(vec![]), tuplets: vec![] }],
        }], breaks: vec![], cross: vec![],
    });
    p.instruments[0].drums = vec![DrumSound { name: "Snare drum".into(), position: Pitch { step: 0, alter: 0, octave: 5 }, key: 38, head: Notehead::X }];
    p.instruments[0].tuning = GUITAR_TUNING.to_vec();
    p.tracks[0].inserts = vec![500, 501];
    p.tracks[0].sends = vec![SendLevel { bus: 60, gain_db: -6.0 }];
    p.buses = vec![Bus { id: 60, name: "Reverb".into(), inserts: vec![502], gain_db: -3.0, pan: 0.2, mute: false }];
    p.master = Master { inserts: vec![503], gain_db: -1.5 };
    p.subtitle = "for flute".into();
    p.lyricist = "Words: N.N.".into();
    p.arranger = "arr. X".into();
    p.copyright = "© 2026".into();
    p.layout = Layout { page_width_mm: 215.9, page_height_mm: 279.4, landscape: true, staff_mm: 5.5, bars_per_system: 4, page_numbers: false, ..Layout::default() };
    p.part_layout.spacing = 1.2;
    p.movements[0].cross = vec![(12, 1)];
    p.movements[0].breaks = vec![Break { id: 90, at: Frac::int(2), page: false, part: None }, Break { id: 91, at: Frac::int(4), page: true, part: Some(2) }];
    p
}

#[test]
fn save_and_load_round_trip() {
    let dir = std::env::temp_dir().join(format!("corinzia-model-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.project");
    let _ = std::fs::remove_file(&path);
    let a = sample();
    save(&a, &path).unwrap();
    let b = load(&path).unwrap();
    assert_eq!((b.title.as_str(), b.composer.as_str(), b.next_id), ("Serenade", "Anon.", 100));
    assert_eq!(b.tracks[0].gain_db, -3.0);
    assert_eq!(b.instruments[0].track, Some(1));
    assert_eq!((b.subtitle.as_str(), b.lyricist.as_str(), b.arranger.as_str(), b.copyright.as_str()), ("for flute", "Words: N.N.", "arr. X", "© 2026"));
    assert_eq!(b.layout, a.layout);
    assert_eq!(b.part_layout, a.part_layout);
    assert_eq!(b.part_layout.staff_mm, 7.0);
    assert_eq!(b.movements[0].breaks, a.movements[0].breaks);
    assert_eq!(b.movements[0].cross, [(12, 1)]);
    assert_eq!(b.movements[0].lanes[0].texts[0].shown, Shown::Parts);
    assert_eq!(b.instruments[0].drums, a.instruments[0].drums);
    assert_eq!(b.instruments[0].tuning, GUITAR_TUNING);
    assert_eq!((&b.tracks[0].inserts, &b.tracks[0].sends), (&a.tracks[0].inserts, &a.tracks[0].sends));
    assert_eq!((&b.buses, &b.master), (&a.buses, &a.master));
    let (ma, mb) = (&a.movements[0], &b.movements[0]);
    assert_eq!(mb.lanes[0].chords, ma.lanes[0].chords);
    assert_eq!(mb.lanes[0].voices[0].events[0].lyrics, ma.lanes[0].voices[0].events[0].lyrics);
    assert_eq!(mb.tempo, ma.tempo);
    assert_eq!(mb.meter, ma.meter);
    assert_eq!(mb.keys, ma.keys);
    assert_eq!(mb.barlines, ma.barlines);
    assert_eq!(mb.rehearsal, ma.rehearsal);
    assert_eq!(mb.endings, ma.endings);
    assert_eq!(mb.lanes[0].clefs, ma.lanes[0].clefs);
    assert_eq!(mb.lanes[0].texts, ma.lanes[0].texts);
    assert_eq!(mb.lanes[0].changes, ma.lanes[0].changes);
    assert_eq!(*mb.lanes[0].voices[0].events, *ma.lanes[0].voices[0].events);
    assert_eq!(mb.navigation, ma.navigation);
    assert_eq!(mb.lanes[0].voices[0].tuplets, ma.lanes[0].voices[0].tuplets);
    assert_eq!(*mb.lanes[0].voices[0].events[0].graces, *ma.lanes[0].voices[0].events[0].graces);
    assert_eq!(mb.lanes[0].voices[0].events[0].fingering, [3]);
    // A player's two staves stay two, each with its own clef and lines.
    assert_eq!(mb.lanes.len(), 2);
    assert_eq!((mb.lanes[0].clefs.len(), mb.lanes[1].clefs[0].clef.as_str()), (1, "bass"));
    assert_eq!((mb.lanes[0].lines.clone(), mb.lanes[1].lines.clone()), (ma.lanes[0].lines.clone(), ma.lanes[1].lines.clone()));
    assert_eq!(mb.lanes[1].voices[0].id, 44);
    // Saving again after a change keeps everything consistent (no stale rows).
    let mut c = b.clone();
    c.movements[0].lanes[0].voices[0].events = Arc::new(vec![]);
    save(&c, &path).unwrap();
    assert!(load(&path).unwrap().movements[0].lanes[0].voices[0].events.is_empty());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn notation_matches_the_model() {
    let p = sample();
    let s = corinzia_model::notation::score(&p, &p.movements[0]);
    assert_eq!(s.parts.len(), 2);
    assert!(s.parts[0].brace, "one player's two staves");
    assert_eq!(s.parts[0].name, "Flute");
    assert_eq!(s.parts[0].abbr, "Fl.");
    let e = &s.parts[0].voices[0].events[1];
    assert_eq!((e.pitches[0].step, e.pitches[0].alter, e.dynamic.as_deref()), (3, 1, Some("f")));
    assert_eq!(s.endings[0].numbers, vec![1]);
}

#[test]
fn audio_graph_round_trip() {
    let dir = std::env::temp_dir().join(format!("corinzia-model-graph-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("g.project");
    let _ = std::fs::remove_file(&path);
    let mut a = sample();
    a.nodes = vec![
        AudioNode { id: 500, kind: NodeKind::Plugin, name: "Sine".into(), bundle: "builtin:corinzia-sine".into(), plugin_id: "org.corinzia.sine".into(), bypassed: false },
        AudioNode { id: 501, kind: NodeKind::Master, name: "Master".into(), bundle: String::new(), plugin_id: String::new(), bypassed: false },
    ];
    a.edges = vec![AudioEdge { from: 500, from_port: 0, to: 501, to_port: 0, gain: 0.5 }];
    a.plugin_state.insert(500, std::sync::Arc::new(b"CZS1\0\0\0?".to_vec()));
    a.tracks[0].instrument_node = Some(500);
    save(&a, &path).unwrap();
    let b = load(&path).unwrap();
    assert_eq!(b.nodes, a.nodes);
    assert_eq!(b.edges, a.edges);
    assert_eq!(b.plugin_state.get(&500).map(|s| s.as_slice()), Some(&b"CZS1\0\0\0?"[..]));
    assert_eq!(b.tracks[0].instrument_node, Some(500));
    // Removing the plugin: node, edge and state go; the track falls back to the built-in sound.
    let mut c = b.clone();
    c.nodes.retain(|n| n.id != 500);
    c.edges.clear();
    c.plugin_state.clear();
    c.tracks[0].instrument_node = None;
    save(&c, &path).unwrap();
    let d = load(&path).unwrap();
    assert_eq!(d.nodes.len(), 1);
    assert!(d.edges.is_empty() && d.plugin_state.is_empty());
    std::fs::remove_dir_all(&dir).ok();
}

